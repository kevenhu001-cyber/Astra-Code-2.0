use crate::common::ResponseEvent;
use crate::common::ResponseStream;
use crate::error::ApiError;
use crate::rate_limits::parse_all_rate_limits;
use crate::telemetry::SseTelemetry;
use codex_client::ByteStream;
use codex_client::StreamResponse;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::TokenUsage;
use eventsource_stream::Eventsource;
use futures::StreamExt;
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::Instant;
use tokio::time::timeout;
use tracing::debug;
use tracing::trace;

/// Spawns a background task that parses Anthropic Messages SSE events into
/// domain [`ResponseEvent`]s.
pub fn spawn_anthropic_stream(
    stream_response: StreamResponse,
    idle_timeout: Duration,
    telemetry: Option<Arc<dyn SseTelemetry>>,
    _turn_state: Option<Arc<OnceLock<String>>>,
) -> ResponseStream {
    let rate_limit_snapshots = parse_all_rate_limits(&stream_response.headers);
    let (tx_event, rx_event) = mpsc::channel::<Result<ResponseEvent, ApiError>>(1600);
    tokio::spawn(async move {
        for snapshot in rate_limit_snapshots {
            let _ = tx_event.send(Ok(ResponseEvent::RateLimits(snapshot))).await;
        }
        process_anthropic_stream(stream_response.bytes, tx_event, idle_timeout, telemetry).await;
    });

    ResponseStream {
        rx_event,
        upstream_request_id: None,
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AnthropicStreamEvent {
    MessageStart {
        message: AnthropicMessageInfo,
    },
    ContentBlockStart {
        index: i64,
        content_block: AnthropicContentBlockStart,
    },
    ContentBlockDelta {
        index: i64,
        delta: AnthropicContentDelta,
    },
    ContentBlockStop {
        index: i64,
    },
    MessageDelta {
        delta: AnthropicMessageDelta,
        #[serde(default)]
        usage: Option<AnthropicMessageUsage>,
    },
    MessageStop,
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
struct AnthropicMessageInfo {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    usage: Option<AnthropicMessageUsage>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AnthropicContentBlockStart {
    Text {
        #[serde(default)]
        text: String,
    },
    ToolUse {
        #[serde(default)]
        id: Option<String>,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        input: Option<Value>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AnthropicContentDelta {
    TextDelta {
        text: String,
    },
    InputJsonDelta {
        #[serde(default)]
        partial_json: String,
    },
}

#[derive(Debug, Deserialize)]
struct AnthropicMessageDelta {
    #[serde(default)]
    stop_reason: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct AnthropicMessageUsage {
    #[serde(default)]
    input_tokens: i64,
    #[serde(default)]
    output_tokens: i64,
}

enum PendingBlock {
    Text { text: String },
    ToolUse { id: Option<String>, name: Option<String>, arguments: String },
}

async fn process_anthropic_stream(
    stream: ByteStream,
    tx_event: mpsc::Sender<Result<ResponseEvent, ApiError>>,
    idle_timeout: Duration,
    telemetry: Option<Arc<dyn SseTelemetry>>,
) {
    let mut stream = stream.eventsource();
    let mut response_id: Option<String> = None;
    let mut input_tokens: i64 = 0;
    let mut output_tokens: i64 = 0;
    let mut pending_blocks: Vec<(i64, PendingBlock)> = Vec::new();
    let mut message_added = false;
    let mut completed = false;

    loop {
        let start = Instant::now();
        let response = timeout(idle_timeout, stream.next()).await;
        if let Some(t) = telemetry.as_ref() {
            t.on_sse_poll(&response, start.elapsed());
        }
        let sse = match response {
            Ok(Some(Ok(sse))) => sse,
            Ok(Some(Err(e))) => {
                debug!("SSE Error: {e:#}");
                let _ = tx_event.send(Err(ApiError::Stream(e.to_string()))).await;
                return;
            }
            Ok(None) => {
                if !completed {
                    let _ = tx_event
                        .send(Err(ApiError::Stream(
                            "anthropic stream closed before message_stop".into(),
                        )))
                        .await;
                }
                return;
            }
            Err(_) => {
                let _ = tx_event
                    .send(Err(ApiError::Stream("idle timeout waiting for SSE".into())))
                    .await;
                return;
            }
        };

        trace!("SSE event: {}", &sse.data);

        let event: AnthropicStreamEvent = match serde_json::from_str(&sse.data) {
            Ok(event) => event,
            Err(e) => {
                debug!(
                    error_category = ?e.classify(),
                    error_line = e.line(),
                    error_column = e.column(),
                    payload_bytes = sse.data.len(),
                    "Failed to parse anthropic SSE event"
                );
                continue;
            }
        };

        match event {
            AnthropicStreamEvent::MessageStart { message } => {
                if response_id.is_none()
                    && let Some(id) = message.id
                {
                    response_id = Some(id);
                }
                if let Some(usage) = message.usage {
                    input_tokens = usage.input_tokens;
                }
            }
            AnthropicStreamEvent::ContentBlockStart { index, content_block } => {
                match content_block {
                    AnthropicContentBlockStart::Text { text } => {
                        if !message_added {
                            let item = ResponseItem::Message {
                                id: None,
                                role: "assistant".to_string(),
                                content: Vec::new(),
                                phase: None,
                                internal_chat_message_metadata_passthrough: None,
                            };
                            if tx_event
                                .send(Ok(ResponseEvent::OutputItemAdded(item)))
                                .await
                                .is_err()
                            {
                                return;
                            }
                            message_added = true;
                        }
                        if !text.is_empty() {
                            if tx_event
                                .send(Ok(ResponseEvent::OutputTextDelta(text.clone())))
                                .await
                                .is_err()
                            {
                                return;
                            }
                        }
                        pending_blocks.push((index, PendingBlock::Text { text }));
                    }
                    AnthropicContentBlockStart::ToolUse { id, name, input } => {
                        let mut arguments = String::new();
                        if let Some(input) = input
                            && !input.is_null()
                        {
                            arguments = input.to_string();
                        }
                        pending_blocks
                            .push((index, PendingBlock::ToolUse { id, name, arguments }));
                    }
                }
            }
            AnthropicStreamEvent::ContentBlockDelta { index, delta } => {
                match delta {
                    AnthropicContentDelta::TextDelta { text } => {
                        if !text.is_empty() {
                            if tx_event
                                .send(Ok(ResponseEvent::OutputTextDelta(text.clone())))
                                .await
                                .is_err()
                            {
                                return;
                            }
                        }
                        if let Some((_, PendingBlock::Text { text: pending })) =
                            pending_blocks.iter_mut().find(|(i, _)| *i == index)
                        {
                            pending.push_str(&text);
                        }
                    }
                    AnthropicContentDelta::InputJsonDelta { partial_json } => {
                        if let Some((_, PendingBlock::ToolUse { arguments, .. })) =
                            pending_blocks.iter_mut().find(|(i, _)| *i == index)
                        {
                            arguments.push_str(&partial_json);
                        }
                    }
                }
            }
            AnthropicStreamEvent::ContentBlockStop { index } => {
                let block = pending_blocks
                    .iter()
                    .position(|(i, _)| *i == index)
                    .map(|position| pending_blocks.remove(position));
                if let Some((_, PendingBlock::ToolUse { id, name, arguments })) = block {
                    let call_id = id.unwrap_or_else(|| format!("toolu-block-{index}"));
                    let name = name.unwrap_or_else(|| "unknown_tool".to_string());
                    let item = ResponseItem::FunctionCall {
                        id: None,
                        name,
                        namespace: None,
                        arguments,
                        encrypted_function_args: None,
                        call_id,
                        internal_chat_message_metadata_passthrough: None,
                    };
                    if tx_event
                        .send(Ok(ResponseEvent::OutputItemDone(item)))
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
            }
            AnthropicStreamEvent::MessageDelta { delta, usage } => {
                if let Some(usage) = usage {
                    output_tokens = usage.output_tokens;
                }
                if delta.stop_reason.as_deref() == Some("max_tokens") {
                    debug!("anthropic stream stopped early: max_tokens");
                }
            }
            AnthropicStreamEvent::MessageStop => {
                if completed {
                    continue;
                }
                completed = true;
                let text = pending_blocks
                    .iter()
                    .filter_map(|(_, block)| match block {
                        PendingBlock::Text { text } => Some(text.as_str()),
                        PendingBlock::ToolUse { .. } => None,
                    })
                    .collect::<String>();
                if !text.is_empty() {
                    let item = ResponseItem::Message {
                        id: None,
                        role: "assistant".to_string(),
                        content: vec![ContentItem::InputText { text }],
                        phase: None,
                        internal_chat_message_metadata_passthrough: None,
                    };
                    if tx_event
                        .send(Ok(ResponseEvent::OutputItemDone(item)))
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
                if tx_event
                    .send(Ok(ResponseEvent::Completed {
                        response_id: response_id
                            .clone()
                            .unwrap_or_else(|| "msg-unknown".to_string()),
                        token_usage: Some(TokenUsage {
                            input_tokens,
                            cached_input_tokens: 0,
                            cache_write_input_tokens: 0,
                            output_tokens,
                            reasoning_output_tokens: 0,
                            total_tokens: input_tokens + output_tokens,
                            codex_rollout_budget_units: None,
                        }),
                        end_turn: Some(true),
                    }))
                    .await
                    .is_err()
                {
                    return;
                }
            }
            AnthropicStreamEvent::Other => {
                debug!("unhandled anthropic stream event type");
            }
        }
    }
}
