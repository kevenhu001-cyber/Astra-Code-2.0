use serde::Serialize;
use serde_json::Value;

/// OpenAI Chat Completions request payload (`/chat/completions`).
#[derive(Debug, Serialize, Clone, PartialEq)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ChatTool>>,
    pub stream: bool,
}

#[derive(Debug, Serialize, Clone, PartialEq)]
pub struct ChatMessage {
    /// One of `system`, `user`, `assistant`, or `tool`.
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<ChatMessageContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ChatToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// Optional display name for the participant.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Serialize, Clone, PartialEq)]
#[serde(untagged)]
pub enum ChatMessageContent {
    Text(String),
    Parts(Vec<ChatContentPart>),
}

#[derive(Debug, Serialize, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChatContentPart {
    Text { text: String },
    ImageUrl { image_url: ChatImageUrl },
}

#[derive(Debug, Serialize, Clone, PartialEq)]
pub struct ChatImageUrl {
    pub url: String,
}

#[derive(Debug, Serialize, Clone, PartialEq)]
pub struct ChatTool {
    #[serde(rename = "type")]
    pub r#type: String,
    pub function: ChatToolFunction,
}

#[derive(Debug, Serialize, Clone, PartialEq)]
pub struct ChatToolFunction {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Debug, Serialize, Clone, PartialEq)]
pub struct ChatToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub r#type: String,
    pub function: ChatToolCallFunction,
}

#[derive(Debug, Serialize, Clone, PartialEq)]
pub struct ChatToolCallFunction {
    pub name: String,
    /// JSON-encoded arguments string.
    pub arguments: String,
}
