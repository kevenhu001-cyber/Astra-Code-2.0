//! Adapter building Anthropic Messages requests from a [`Prompt`].

use super::WireToolEntry;
use super::WireToolIndex;
use super::freeform_arguments_from_input;
use crate::client_common::Prompt;
use codex_api::AnthropicContentBlock;
use codex_api::AnthropicMessage;
use codex_api::AnthropicRequest;
use codex_api::AnthropicTool;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseItem;
use codex_tools::ResponsesApiNamespaceTool;
use codex_tools::ToolSpec;
use serde_json::Value;

/// Anthropic requires `max_tokens`; pick a generous default for coding turns.
pub(crate) const DEFAULT_ANTHROPIC_MAX_TOKENS: u32 = 8192;

const FREEFORM_PARAMETERS: &str =
    r#"{"type":"object","properties":{"input":{"type":"string"}},"required":["input"]}"#;

/// Builds an Anthropic Messages request for the given prompt.
pub(crate) fn build_anthropic_request(
    prompt: &Prompt,
    model: &str,
) -> Result<AnthropicRequest, serde_json::Error> {
    let tool_index = WireToolIndex::new(&prompt.tools);
    let system = if prompt.base_instructions.text.trim().is_empty() {
        None
    } else {
        Some(prompt.base_instructions.text.clone())
    };

    let mut messages: Vec<AnthropicMessage> = Vec::new();
    for item in &prompt.input {
        match item {
            ResponseItem::Message {
                role, content, ..
            } if matches!(role.as_str(), "user" | "assistant") => {
                let text = content
                    .iter()
                    .filter_map(|item| match item {
                        ContentItem::InputText { text } | ContentItem::OutputText { text } => {
                            Some(text.as_str())
                        }
                        ContentItem::InputImage { .. } | ContentItem::InputAudio { .. } => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                if text.trim().is_empty() {
                    continue;
                }
                push_message(
                    &mut messages,
                    role.clone(),
                    vec![AnthropicContentBlock::Text { text }],
                );
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
                push_message(
                    &mut messages,
                    "assistant".to_string(),
                    vec![AnthropicContentBlock::ToolUse {
                        id: call_id.clone(),
                        name: entry.flat_name(),
                        input: tool_input_from_arguments(arguments),
                    }],
                );
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
                let arguments = freeform_arguments_from_input(input)?;
                push_message(
                    &mut messages,
                    "assistant".to_string(),
                    vec![AnthropicContentBlock::ToolUse {
                        id: call_id.clone(),
                        name: entry.flat_name(),
                        input: tool_input_from_arguments(&arguments),
                    }],
                );
            }
            ResponseItem::FunctionCallOutput { call_id, output, .. }
            | ResponseItem::CustomToolCallOutput { call_id, output, .. } => {
                let content = output.body.to_text().unwrap_or_default();
                push_message(
                    &mut messages,
                    "user".to_string(),
                    vec![AnthropicContentBlock::ToolResult {
                        tool_use_id: call_id.clone(),
                        content,
                    }],
                );
            }
            _ => {}
        }
    }

    let tools = anthropic_tools(&prompt.tools)?;
    Ok(AnthropicRequest {
        model: model.to_string(),
        system,
        messages,
        tools: if tools.is_empty() { None } else { Some(tools) },
        max_tokens: DEFAULT_ANTHROPIC_MAX_TOKENS,
        stream: true,
    })
}

/// Appends a content block to the last message if it has the same role,
/// otherwise starts a new message (Anthropic requires alternating roles).
fn push_message(messages: &mut Vec<AnthropicMessage>, role: String, mut blocks: Vec<AnthropicContentBlock>) {
    if let Some(last) = messages.last_mut()
        && last.role == role
    {
        last.content.append(&mut blocks);
        return;
    }
    messages.push(AnthropicMessage { role, content: blocks });
}

fn tool_input_from_arguments(arguments: &str) -> Value {
    match serde_json::from_str::<Value>(arguments) {
        Ok(value @ Value::Object(_)) => value,
        // Anthropic requires tool inputs to be objects; wrap bare strings.
        Ok(other) => serde_json::json!({ "input": other }),
        Err(_) => serde_json::json!({ "input": arguments }),
    }
}

fn anthropic_tools(tools: &[ToolSpec]) -> Result<Vec<AnthropicTool>, serde_json::Error> {
    let mut anthropic_tools = Vec::new();
    for tool in tools {
        match tool {
            ToolSpec::Function(tool) => {
                anthropic_tools.push(AnthropicTool {
                    name: tool.name.clone(),
                    description: tool.description.clone(),
                    input_schema: serde_json::to_value(&tool.parameters)?,
                });
            }
            ToolSpec::Freeform(tool) => {
                anthropic_tools.push(AnthropicTool {
                    name: tool.name.clone(),
                    description: tool.description.clone(),
                    input_schema: serde_json::from_str(FREEFORM_PARAMETERS)?,
                });
            }
            ToolSpec::Namespace(namespace) => {
                let namespace_name = namespace.name.clone();
                for tool in &namespace.tools {
                    let (name, description, input_schema, freeform) = match tool {
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
                        namespace: (namespace_name != codex_protocol::DEFAULT_FUNCTION_NAMESPACE)
                            .then(|| namespace_name.clone()),
                        name,
                        freeform,
                    };
                    anthropic_tools.push(AnthropicTool {
                        name: entry.flat_name(),
                        description,
                        input_schema,
                    });
                }
            }
            ToolSpec::ToolSearch { .. } | ToolSpec::WebSearch { .. } => {}
        }
    }
    Ok(anthropic_tools)
}
