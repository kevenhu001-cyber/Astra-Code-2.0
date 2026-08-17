//! Request/response adapters for non-Responses wire APIs (OpenAI Chat
//! Completions and Anthropic Messages).
//!
//! The Responses API carries tools as namespaced items and freeform tools as
//! raw-string `custom_tool_call` items. Chat Completions and Anthropic
//! Messages both use flat function names and JSON-schema arguments, so tools
//! are flattened here and freeform tools are wrapped in a single `input`
//! JSON-schema property. On the way back, emitted function calls are mapped
//! back to namespaced [`ResponseItem`]s so the tool router sees the same
//! shapes it sees from the Responses API.

pub(crate) mod anthropic;
pub(crate) mod chat;

use codex_api::ResponseEvent;
use codex_protocol::models::ResponseItem;
use codex_protocol::DEFAULT_FUNCTION_NAMESPACE;
use codex_tools::ResponsesApiNamespaceTool;
use codex_tools::ToolSpec;
use std::collections::HashMap;

/// A flattened tool known to the wire API, indexed by its flat callable name.
#[derive(Debug, Clone)]
pub(crate) struct WireToolEntry {
    pub(crate) namespace: Option<String>,
    pub(crate) name: String,
    pub(crate) freeform: bool,
}

impl WireToolEntry {
    /// The flat name a Chat Completions / Anthropic model should call this tool by.
    pub(crate) fn flat_name(&self) -> String {
        match &self.namespace {
            Some(namespace) if !namespace.is_empty() && namespace != DEFAULT_FUNCTION_NAMESPACE => {
                format!("{namespace}{}", self.name)
            }
            _ => self.name.clone(),
        }
    }
}

/// Index of all tools in a prompt, keyed by their flat wire names.
#[derive(Debug, Default)]
pub(crate) struct WireToolIndex {
    entries: HashMap<String, WireToolEntry>,
}

impl WireToolIndex {
    pub(crate) fn new(tools: &[ToolSpec]) -> Self {
        let mut index = Self::default();
        for tool in tools {
            match tool {
                ToolSpec::Function(tool) => {
                    let entry = WireToolEntry {
                        namespace: None,
                        name: tool.name.clone(),
                        freeform: false,
                    };
                    index.entries.insert(entry.flat_name(), entry);
                }
                ToolSpec::Freeform(tool) => {
                    let entry = WireToolEntry {
                        namespace: None,
                        name: tool.name.clone(),
                        freeform: true,
                    };
                    index.entries.insert(entry.flat_name(), entry);
                }
                ToolSpec::Namespace(namespace) => {
                    let namespace_name = namespace.name.clone();
                    for tool in &namespace.tools {
                        let (name, freeform) = match tool {
                            ResponsesApiNamespaceTool::Function(tool) => (tool.name.clone(), false),
                            ResponsesApiNamespaceTool::Custom(tool) => (tool.name.clone(), true),
                        };
                        let namespace = if namespace_name == DEFAULT_FUNCTION_NAMESPACE {
                            None
                        } else {
                            Some(namespace_name.clone())
                        };
                        let entry = WireToolEntry {
                            namespace,
                            name,
                            freeform,
                        };
                        index.entries.insert(entry.flat_name(), entry);
                    }
                }
                ToolSpec::ToolSearch { .. } | ToolSpec::WebSearch { .. } => {}
            }
        }
        index
    }

    pub(crate) fn lookup(&self, flat_name: &str) -> Option<&WireToolEntry> {
        self.entries.get(flat_name)
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Maps a wire function-call item back to the namespaced shape the tool router
/// expects, converting freeform tool calls into [`ResponseItem::CustomToolCall`].
pub(crate) fn normalize_function_call_item(
    item: ResponseItem,
    index: &WireToolIndex,
) -> ResponseItem {
    let ResponseItem::FunctionCall {
        id,
        name,
        namespace,
        arguments,
        encrypted_function_args,
        call_id,
        internal_chat_message_metadata_passthrough,
    } = item
    else {
        return item;
    };

    let Some(entry) = index.lookup(&name) else {
        return ResponseItem::FunctionCall {
            id,
            name,
            namespace,
            arguments,
            encrypted_function_args,
            call_id,
            internal_chat_message_metadata_passthrough,
        };
    };

    if entry.freeform {
        ResponseItem::CustomToolCall {
            id,
            status: None,
            call_id,
            name: entry.name.clone(),
            namespace: entry.namespace.clone(),
            input: freeform_input_from_arguments(&arguments),
            internal_chat_message_metadata_passthrough,
        }
    } else {
        ResponseItem::FunctionCall {
            id,
            name: entry.name.clone(),
            namespace: entry.namespace.clone(),
            arguments,
            encrypted_function_args,
            call_id,
            internal_chat_message_metadata_passthrough,
        }
    }
}

/// Normalizes wire events: namespaced function calls are restored and freeform
/// tool calls become [`ResponseItem::CustomToolCall`].
pub(crate) fn normalize_response_event(event: ResponseEvent, index: &WireToolIndex) -> ResponseEvent {
    match event {
        ResponseEvent::OutputItemDone(item) => {
            ResponseEvent::OutputItemDone(normalize_function_call_item(item, index))
        }
        event => event,
    }
}

/// Extracts the raw freeform input from chat-style `{"input": "..."}` arguments.
pub(crate) fn freeform_input_from_arguments(arguments: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(arguments) {
        Ok(serde_json::Value::Object(map)) => map
            .get("input")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| arguments.to_string()),
        _ => arguments.to_string(),
    }
}

/// Serializes raw freeform input into chat-style `{"input": "..."}` arguments.
pub(crate) fn freeform_arguments_from_input(input: &str) -> Result<String, serde_json::Error> {
    let value = serde_json::json!({ "input": input });
    serde_json::to_string(&value)
}
