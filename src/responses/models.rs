use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Specifies the format of the model's output
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(tag = "type")]
pub enum ResponseFormat {
    /// Plain text response (default)
    #[serde(rename = "text")]
    #[default]
    Text,
    /// Valid JSON object
    #[serde(rename = "json_object")]
    JsonObject,
    /// JSON conforming to a specific schema
    #[serde(rename = "json_schema")]
    JsonSchema { json_schema: JsonSchemaDefinition },
}

/// JSON schema definition for structured output
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonSchemaDefinition {
    /// Name of the schema
    pub name: String,
    /// Optional description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// JSON schema specification
    pub schema: serde_json::Value,
    /// Whether to enforce strict adherence
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strict: Option<bool>,
}

/// Settings for model reasoning enhancements
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasoningSettings {
    /// Reasoning effort level (e.g., "medium")
    pub effort: String,
    /// Additional reasoning parameters
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

/// Definition of a tool that can be called by the model
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    /// Type of tool (e.g., "function")
    #[serde(rename = "type")]
    pub tool_type: String,
    /// Function definition if type is "function"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<ToolFunction>,
}

/// Function tool specification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolFunction {
    /// Function name
    pub name: String,
    /// Function description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// JSON schema for function parameters
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameters: Option<serde_json::Value>,
}

/// How the model should use tools
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ToolChoice {
    /// String choice ("auto", "none")
    String(String),
    /// Specific tool selection
    Object(ToolChoiceObject),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolChoiceObject {
    #[serde(rename = "type")]
    pub choice_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<ToolChoiceFunction>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolChoiceFunction {
    pub name: String,
}

/// Resources available to tools during execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResources {
    /// Code interpreter sandbox resources
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code_interpreter: Option<CodeInterpreterResources>,
    /// File search resources
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_search: Option<FileSearchResources>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeInterpreterResources {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sandbox: Option<CodeInterpreterSandbox>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeInterpreterSandbox {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileSearchResources {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vector_store_ids: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files: Option<Vec<String>>,
}

/// File attachment for the request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    /// File identifier
    pub file_id: String,
    /// Tools that can access this file
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<String>>,
}

/// Error information when request fails
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorObject {
    /// Error type classification
    #[serde(rename = "type")]
    pub error_type: String,
    /// Human-readable error message
    pub message: String,
    /// Parameter that caused the error
    #[serde(skip_serializing_if = "Option::is_none")]
    pub param: Option<String>,
    /// Error code
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

/// Request to generate a response from a model
///
/// Implements the OpenAI Responses API specification
#[derive(Debug, Deserialize)]
pub struct ResponseRequest {
    /// Model identifier
    pub model: String,
    /// Input text to process
    pub input: String,
    /// System instructions for the model
    pub instructions: Option<String>,
    /// ID of previous response for conversation continuity
    pub previous_response_id: Option<String>,

    /// Output modalities (e.g., ["text", "audio"])
    pub modalities: Option<Vec<String>>,
    /// Format specification for the response
    pub response_format: Option<ResponseFormat>,
    /// Reasoning enhancement settings
    pub reasoning: Option<ReasoningSettings>,
    /// Sampling temperature (0.0-2.0)
    pub temperature: Option<f32>,
    /// Nucleus sampling parameter (0.0-1.0)
    pub top_p: Option<f32>,
    /// Maximum tokens to generate
    pub max_output_tokens: Option<u32>,
    /// Enable streaming response
    pub stream: Option<bool>,
    /// Additional response sections to include
    #[allow(dead_code)]
    pub include: Option<Vec<String>>,
    /// Tool definitions available to the model
    pub tools: Option<Vec<ToolDefinition>>,
    /// Tool selection strategy
    pub tool_choice: Option<ToolChoice>,
    /// Resources available to tools
    pub tool_resources: Option<ToolResources>,
    /// File attachments
    pub attachments: Option<Vec<Attachment>>,
    /// User-defined metadata
    pub metadata: Option<HashMap<String, String>>,
    /// End-user identifier
    pub user: Option<String>,
}

/// Response object from a model generation request
///
/// Implements the OpenAI Responses API specification at
/// https://platform.openai.com/docs/api-reference/responses/object
#[derive(Debug, Serialize)]
pub struct ResponseReply {
    /// Unique identifier for this response
    pub id: String,
    /// Object type ("response")
    pub object: String,
    /// Unix timestamp of creation
    pub created_at: i64,
    /// Response status ("completed", "failed", etc.)
    pub status: String,
    /// Model that generated the response
    pub model: String,
    /// Generated output items
    pub output: Vec<OutputItem>,
    /// Token usage information
    pub usage: Usage,
    /// ID of the previous response in conversation
    pub previous_response_id: Option<String>,

    /// Echoed metadata from request
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, String>>,
    /// System instructions used
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    /// Output modalities produced
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modalities: Option<Vec<String>>,
    /// Response format used
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<ResponseFormat>,
    /// Reasoning traces and metadata
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<serde_json::Value>,
    /// Tool calls made by the model
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<serde_json::Value>,
    /// Results from tool executions
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_outputs: Option<serde_json::Value>,
    /// Resources used by tools
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_resources: Option<ToolResources>,
    /// Concatenated text output for convenience
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_text: Option<String>,
    /// Normalized input data
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<serde_json::Value>,
    /// Error details if status is "failed"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorObject>,
    /// Non-fatal warnings and notices
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warnings: Option<Vec<String>>,
    /// Detailed usage breakdown
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage_details: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct OutputItem {
    #[serde(rename = "type")]
    pub item_type: String,
    pub id: String,
    pub status: String,
    pub role: String,
    pub content: Vec<ContentItem>,
}

#[derive(Debug, Serialize)]
pub struct ContentItem {
    #[serde(rename = "type")]
    pub content_type: String,
    pub text: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: i32,
    pub output_tokens: i32,
    pub total_tokens: i32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Session {
    pub response_id: String,
    pub created: i64,
    pub model_used: String,
    pub messages: HashMap<String, SessionMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extended_data: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SessionMessage {
    pub role: String,
    pub content: String,
    pub tokens: i32,
    pub created_at: i64,
    pub response_time: Option<i64>,
    pub response_id: Option<String>,
}

#[derive(Debug)]
#[allow(dead_code)]
pub struct SessionRow {
    pub id: String,
    pub session_data: String,
    pub created_at: i64,
    pub last_updated: i64,
}

impl Session {
    pub fn new(response_id: String, model: String, instructions: Option<String>) -> Self {
        let now = chrono::Utc::now().timestamp();
        let mut messages = HashMap::new();

        if let Some(inst) = instructions {
            messages.insert(
                "0".to_string(),
                SessionMessage {
                    role: "system".to_string(),
                    content: inst,
                    tokens: 0,
                    created_at: now,
                    response_time: None,
                    response_id: None,
                },
            );
        }

        Session {
            response_id,
            created: now,
            model_used: model,
            messages,
            extended_data: None,
        }
    }

    pub fn add_message(
        &mut self,
        role: String,
        content: String,
        tokens: i32,
        response_time: Option<i64>,
        response_id: Option<String>,
    ) {
        let now = chrono::Utc::now().timestamp();
        let index = self.messages.len().to_string();

        self.messages.insert(
            index,
            SessionMessage {
                role,
                content,
                tokens,
                created_at: now,
                response_time,
                response_id,
            },
        );
    }

    pub fn get_conversation_history(&self) -> Vec<(String, String)> {
        let mut history = Vec::new();

        for i in 0..self.messages.len() {
            let key = i.to_string();
            if let Some(msg) = self.messages.get(&key) {
                history.push((msg.role.clone(), msg.content.clone()));
            }
        }

        history
    }

    #[allow(dead_code)]
    pub fn total_tokens(&self) -> i32 {
        self.messages.values().map(|msg| msg.tokens).sum()
    }
}

impl ResponseReply {
    pub fn new(
        response_id: String,
        model: String,
        content: String,
        input_tokens: i32,
        output_tokens: i32,
        previous_id: Option<String>,
    ) -> Self {
        let now = chrono::Utc::now().timestamp();
        let message_id = format!("msg_{}", uuid::Uuid::new_v4().simple());

        ResponseReply {
            id: response_id,
            object: "response".to_string(),
            created_at: now,
            status: "completed".to_string(),
            model,
            output: vec![OutputItem {
                item_type: "message".to_string(),
                id: message_id,
                status: "completed".to_string(),
                role: "assistant".to_string(),
                content: vec![ContentItem {
                    content_type: "output_text".to_string(),
                    text: content.clone(),
                }],
            }],
            usage: Usage {
                input_tokens,
                output_tokens,
                total_tokens: input_tokens + output_tokens,
            },
            previous_response_id: previous_id,

            metadata: None,
            instructions: None,
            modalities: None,
            response_format: None,
            reasoning: None,
            tool_calls: None,
            tool_outputs: None,
            tool_resources: None,
            output_text: Some(content),
            input: None,
            error: None,
            warnings: None,
            usage_details: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_add_message() {
        let mut session = Session::new("test_id".to_string(), "test_model".to_string(), None);

        session.add_message("user".to_string(), "Hello!".to_string(), 5, None, None);

        assert_eq!(session.messages.len(), 1);
        let message = session.messages.get("0").unwrap();
        assert_eq!(message.role, "user");
        assert_eq!(message.content, "Hello!");
        assert_eq!(message.tokens, 5);
        assert!(message.response_id.is_none());

        session.add_message(
            "assistant".to_string(),
            "Hi there!".to_string(),
            10,
            Some(150),
            Some("resp_123".to_string()),
        );

        assert_eq!(session.messages.len(), 2);
        let assistant_msg = session.messages.get("1").unwrap();
        assert_eq!(assistant_msg.role, "assistant");
        assert_eq!(assistant_msg.content, "Hi there!");
        assert_eq!(assistant_msg.tokens, 10);
        assert_eq!(assistant_msg.response_time, Some(150));
        assert_eq!(assistant_msg.response_id, Some("resp_123".to_string()));
    }

    #[test]
    fn test_session_get_conversation_history() {
        let mut session = Session::new(
            "test_id".to_string(),
            "test_model".to_string(),
            Some("System prompt".to_string()),
        );

        session.add_message(
            "user".to_string(),
            "First message".to_string(),
            5,
            None,
            None,
        );
        session.add_message(
            "assistant".to_string(),
            "First response".to_string(),
            8,
            None,
            None,
        );
        session.add_message(
            "user".to_string(),
            "Second message".to_string(),
            6,
            None,
            None,
        );

        let history = session.get_conversation_history();

        assert_eq!(history.len(), 4);
        assert_eq!(
            history[0],
            ("system".to_string(), "System prompt".to_string())
        );
        assert_eq!(
            history[1],
            ("user".to_string(), "First message".to_string())
        );
        assert_eq!(
            history[2],
            ("assistant".to_string(), "First response".to_string())
        );
        assert_eq!(
            history[3],
            ("user".to_string(), "Second message".to_string())
        );
    }

    #[test]
    fn test_response_reply_new() {
        let response = ResponseReply::new(
            "resp_123".to_string(),
            "test_model".to_string(),
            "Hello, world!".to_string(),
            10,
            15,
            Some("prev_resp_456".to_string()),
        );

        assert_eq!(response.id, "resp_123");
        assert_eq!(response.object, "response");
        assert_eq!(response.status, "completed");
        assert_eq!(response.model, "test_model");
        assert_eq!(
            response.previous_response_id,
            Some("prev_resp_456".to_string())
        );

        assert_eq!(response.usage.input_tokens, 10);
        assert_eq!(response.usage.output_tokens, 15);
        assert_eq!(response.usage.total_tokens, 25);

        assert_eq!(response.output.len(), 1);
        let output_item = &response.output[0];
        assert_eq!(output_item.item_type, "message");
        assert_eq!(output_item.status, "completed");
        assert_eq!(output_item.role, "assistant");

        assert_eq!(output_item.content.len(), 1);
        let content_item = &output_item.content[0];
        assert_eq!(content_item.content_type, "output_text");
        assert_eq!(content_item.text, "Hello, world!");
    }
}
