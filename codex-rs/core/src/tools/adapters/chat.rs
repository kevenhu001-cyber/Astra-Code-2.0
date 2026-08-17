//! Adapter building OpenAI Chat Completions requests from a [`Prompt`].

use super::WireToolEntry;
use super::WireToolIndex;
use super::freeform_arguments_from_input;
use crate::client_common::Prompt;
use codex_api::ChatContentPart;
use codex_api::ChatImageUrl;
use codex_api::ChatMessage;
use codex_api::ChatMessageContent;
use codex_api::ChatRequest;
use codex_api::ChatTool;
use codex_api::ChatToolCall;
use codex_api::ChatToolCallFunction;
use codex_api::ChatToolFunction;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseItem;
use codex_tools::ResponsesApiNamespaceTool;
use codex_tools::ToolSpec;

const FREEFORM_PARAMETERS: &str =
    r#"{"type":"object","properties":{"input":{"type":"string"}},"required":["input"]}"#;

/// Builds a Chat Completions request for the given prompt.
pub(crate) fn build_chat_request(
    prompt: &Prompt,
    model: &str,
) -> Result<ChatRequest, serde_json::Error> {
    let tool_index = WireToolIndex::new(&prompt.tools);
    let mut messages = Vec::new();
    if !prompt.base_instructions.text.trim().is_empty() {
        messages.push(ChatMessage {
            role: "system".to_string(),
            content: Some(ChatMessageContent::Text(prompt.base_instructions.text.clone())),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        });
    }
    for item in &prompt.input {
        match item {
            ResponseItem::Message {
                role, content, ..
            } => {
                let Some(content) = chat_content_from_content_items(content) else {
                    continue;
                };
                messages.push(ChatMessage {
                    role: chat_role(role),
                    content: Some(content),
                    tool_calls: None,
                    tool_call_id: None,
                    name: None,
                });
            }
            ResponseItem::FunctionCall {
                call_id,
                name,
                namespace,
                arguments,
                ..
            } => {
                let entry = tool_index.lookup(name).cloned().unwrap_or(WireToolEntry {
                    namespace: namespace.clone(),
                    name: name.clone(),
                    freeform: false,
                });
                messages.push(assistant_tool_call_message(
                    call_id.clone(),
                    &entry,
                    arguments.clone(),
                ));
            }
            ResponseItem::CustomToolCall {
                call_id,
                name,
                namespace,
                input,
                ..
            } => {
                let entry = tool_index.lookup(name).cloned().unwrap_or(WireToolEntry {
                    namespace: namespace.clone(),
                    name: name.clone(),
                    freeform: true,
                });
                messages.push(assistant_tool_call_message(
                    call_id.clone(),
                    &entry,
                    freeform_arguments_from_input(input)?,
                ));
            }
            ResponseItem::FunctionCallOutput { call_id, output, .. }
            | ResponseItem::CustomToolCallOutput { call_id, output, .. } => {
                let content = output.body.to_text().unwrap_or_default();
                messages.push(ChatMessage {
                    role: "tool".to_string(),
                    content: Some(ChatMessageContent::Text(content)),
                    tool_calls: None,
                    tool_call_id: Some(call_id.clone()),
                    name: None,
                });
            }
            _ => {}
        }
    }
    let tools = chat_tools(&prompt.tools)?;
    Ok(ChatRequest {
        model: model.to_string(),
        messages,
        tools: if tools.is_empty() { None } else { Some(tools) },
        stream: true,
    })
}

fn chat_role(role: &str) -> String {
    match role {
        // "developer" is Responses-API-only; Chat Completions expects "system".
        "developer" => "system".to_string(),
        other => other.to_string(),
    }
}

fn assistant_tool_call_message(
    call_id: String,
    entry: &WireToolEntry,
    arguments: String,
) -> ChatMessage {
    ChatMessage {
        role: "assistant".to_string(),
        content: None,
        tool_calls: Some(vec![ChatToolCall {
            id: call_id,
            r#type: "function".to_string(),
            function: ChatToolCallFunction {
                name: entry.flat_name(),
                arguments,
            },
        }]),
        tool_call_id: None,
        name: None,
    }
}

fn chat_content_from_content_items(content: &[ContentItem]) -> Option<ChatMessageContent> {
    let mut parts = Vec::new();
    let mut plain_text: Option<String> = None;
    for item in content {
        match item {
            ContentItem::InputText { text } | ContentItem::OutputText { text } => {
                plain_text = Some(match plain_text {
                    Some(existing) => format!("{existing}{text}"),
                    None => text.clone(),
                });
            }
            ContentItem::InputImage { image_url, .. } => {
                if let Some(text) = plain_text.take() {
                    parts.push(ChatContentPart::Text { text });
                }
                parts.push(ChatContentPart::ImageUrl {
                    image_url: ChatImageUrl {
                        url: image_url.clone(),
                    },
                });
            }
            ContentItem::InputAudio { .. } => {}
        }
    }
    if let Some(text) = plain_text.take() {
        if parts.is_empty() {
            return Some(ChatMessageContent::Text(text));
        }
        parts.push(ChatContentPart::Text { text });
    }
    if parts.is_empty() {
        return None;
    }
    Some(ChatMessageContent::Parts(parts))
}

fn chat_tools(tools: &[ToolSpec]) -> Result<Vec<ChatTool>, serde_json::Error> {
    let mut chat_tools = Vec::new();
    for tool in tools {
        match tool {
            ToolSpec::Function(tool) => {
                chat_tools.push(ChatTool {
                    r#type: "function".to_string(),
                    function: ChatToolFunction {
                        name: tool.name.clone(),
                        description: tool.description.clone(),
                        parameters: serde_json::to_value(&tool.parameters)?,
                    },
                });
            }
            ToolSpec::Freeform(tool) => {
                chat_tools.push(ChatTool {
                    r#type: "function".to_string(),
                    function: ChatToolFunction {
                        name: tool.name.clone(),
                        description: tool.description.clone(),
                        parameters: serde_json::from_str(FREEFORM_PARAMETERS)?,
                    },
                });
            }
            ToolSpec::Namespace(namespace) => {
                let namespace_name = namespace.name.clone();
                for tool in &namespace.tools {
                    let (name, description, parameters, freeform) = match tool {
                        ResponsesApiNamespaceTool::Function(tool) => (
                            tool.name.clone(),
                            tool.description.clone(),
                            serde_json::to_value(&tool.parameters)?,
                            false,
                        ),
                        ResponsesApiNamespaceTool::Custom(tool) => (
                            tool.name.clone(),
                            tool.description.clone(),
                            serde_json::from_str(FREEFORM_PARAMETERS)?,
                            true,
                        ),
                    };
                    let entry = WireToolEntry {
                        namespace: (namespace_name != codex_protocol::tool_name::DEFAULT_FUNCTION_NAMESPACE)
                            .then(|| namespace_name.clone()),
                        name,
                        freeform,
                    };
                    chat_tools.push(ChatTool {
                        r#type: "function".to_string(),
                        function: ChatToolFunction {
                            name: entry.flat_name(),
                            description,
                            parameters,
                        },
                    });
                }
            }
            ToolSpec::ToolSearch { .. } | ToolSpec::WebSearch { .. } => {}
        }
    }
    Ok(chat_tools)
}
