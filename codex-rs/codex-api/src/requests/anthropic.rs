use serde::Serialize;
use serde_json::Value;

/// Anthropic Messages API request payload (`/v1/messages`).
#[derive(Debug, Serialize, Clone, PartialEq)]
pub struct AnthropicRequest {
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    pub messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<AnthropicTool>>,
    pub max_tokens: u32,
    pub stream: bool,
}

#[derive(Debug, Serialize, Clone, PartialEq)]
pub struct AnthropicMessage {
    /// One of `user` or `assistant`.
    pub role: String,
    pub content: Vec<AnthropicContentBlock>,
}

#[derive(Debug, Serialize, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AnthropicContentBlock {
    Text { text: String },
    ToolUse { id: String, name: String, input: Value },
    ToolResult { tool_use_id: String, content: String },
}

#[derive(Debug, Serialize, Clone, PartialEq)]
pub struct AnthropicTool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}
