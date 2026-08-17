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
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::Instant;
use tokio::time::timeout;
use tracing::debug;
use tracing::trace;

/// Spawns a background task that parses OpenAI Chat Completions SSE chunks
/// into domain [`ResponseEvent`]s.
pub fn spawn_chat_stream(
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
        process_chat_stream(stream_response.bytes, tx_event, idle_timeout, telemetry).await;
    });

    ResponseStream {
        rx_event,
        upstream_request_id: None,
    }
}

#[derive(Debug, Deserialize)]
struct ChatStreamChunk {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    choices: Vec<ChatStreamChoice>,
    #[serde(default)]
    usage: Option<ChatStreamUsage>,
}

#[derive(Debug, Deserialize)]
struct ChatStreamChoice {
    #[serde(default)]
    delta: ChatStreamDelta,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct ChatStreamDelta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<ChatStreamToolCall>,
}

#[derive(Debug, Deserialize, Default)]
struct ChatStreamToolCall {
    index: i64,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<ChatStreamFunction>,
}

#[derive(Debug, Deserialize, Default)]
struct ChatStreamFunction {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct ChatStreamUsage {
    #[serde(default)]
    prompt_tokens: i64,
    #[serde(default)]
    completion_tokens: i64,
    #[serde(default)]
    total_tokens: i64,
}

struct PendingToolCall {
    index: i64,
    id: Option<String>,
    name: Option<String>,
    arguments: String,
}

async fn process_chat_stream(
    stream: ByteStream,
    tx_event: mpsc::Sender<Result<ResponseEvent, ApiError>>,
    idle_timeout: Duration,
    telemetry: Option<Arc<dyn SseTelemetry>>,
) {
    let mut stream = stream.eventsource();
    let mut pending_tool_calls: Vec<PendingToolCall> = Vec::new();
    let mut text_parts: Vec<String> = Vec::new();
    let mut response_id: Option<String> = None;
    let mut usage: Option<TokenUsage> = None;
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
                            "chat stream closed before completion".into(),
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

        let chunk: ChatStreamChunk = match serde_json::from_str(&sse.data) {
            Ok(chunk) => chunk,
            Err(e) => {
                debug!(
                    error_category = ?e.classify(),
                    error_line = e.line(),
                    error_column = e.column(),
                    payload_bytes = sse.data.len(),
                    "Failed to parse chat SSE event"
                );
                continue;
            }
        };

        if response_id.is_none()
            && let Some(id) = chunk.id.as_deref()
        {
            response_id = Some(id.to_string());
        }
        if let Some(u) = chunk.usage
            && usage.is_none()
        {
            usage = Some(TokenUsage {
                input_tokens: u.prompt_tokens,
                cached_input_tokens: 0,
                cache_write_input_tokens: 0,
                output_tokens: u.completion_tokens,
                reasoning_output_tokens: 0,
                total_tokens: u.total_tokens,
                codex_rollout_budget_units: None,
            });
        }

        for choice in chunk.choices {
            if let Some(content) = choice.delta.content.as_deref() {
                if !content.is_empty() {
                    if !message_added {
                        if !emit_message_added(&tx_event).await {
                            return;
                        }
                        message_added = true;
                    }
                    text_parts.push(content.to_string());
                    if tx_event
                        .send(Ok(ResponseEvent::OutputTextDelta(content.to_string())))
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
            }

            for tool_call in choice.delta.tool_calls {
                if let Some(existing) = pending_tool_calls
                    .iter_mut()
                    .find(|pending| pending.index == tool_call.index)
                {
                    if let Some(id) = tool_call.id {
                        existing.id = Some(id);
                    }
                    if let Some(name) = tool_call.function.as_ref().and_then(|f| f.name.clone()) {
                        existing.name = Some(name);
                    }
                    if let Some(arguments) =
                        tool_call.function.as_ref().and_then(|f| f.arguments.clone())
                    {
                        if !arguments.is_empty() {
                            existing.arguments.push_str(&arguments);
                        }
                    }
                } else {
                    let arguments = tool_call
                        .function
                        .as_ref()
                        .and_then(|f| f.arguments.clone())
                        .unwrap_or_default();
                    pending_tool_calls.push(PendingToolCall {
                        index: tool_call.index,
                        id: tool_call.id,
                        name: tool_call.function.as_ref().and_then(|f| f.name.clone()),
                        arguments,
                    });
                }
            }

            if let Some(finish_reason) = choice.finish_reason.as_deref() {
                if !finish_reason.is_empty() && !completed {
                    completed = true;
                    if !pending_tool_calls.is_empty() {
                        for pending in pending_tool_calls.drain(..) {
                            let call_id = pending
                                .id
                                .clone()
                                .unwrap_or_else(|| format!("chatcmpl-tool-{}", pending.index));
                            let name = pending
                                .name
                                .unwrap_or_else(|| "unknown_tool".to_string());
                            let item = ResponseItem::FunctionCall {
                                id: None,
                                name,
                                namespace: None,
                                arguments: pending.arguments,
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
                    if !text_parts.is_empty() {
                        let text = text_parts.join("");
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
                                .unwrap_or_else(|| "chatcmpl-unknown".to_string()),
                            token_usage: usage.clone(),
                            end_turn: Some(true),
                        }))
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
            }
        }
    }
}

async fn emit_message_added(
    tx_event: &mpsc::Sender<Result<ResponseEvent, ApiError>>,
) -> bool {
    let item = ResponseItem::Message {
        id: None,
        role: "assistant".to_string(),
        content: Vec::new(),
        phase: None,
        internal_chat_message_metadata_passthrough: None,
    };
    tx_event.send(Ok(ResponseEvent::OutputItemAdded(item))).await.is_ok()
}
