use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ResponseFormat {
    #[serde(rename = "text")]
    Text,
    #[serde(rename = "json_object")]
    JsonObject,
    #[serde(rename = "json_schema")]
    JsonSchema { json_schema: JsonSchemaDefinition },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonSchemaDefinition {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub schema: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strict: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasoningSettings {
    pub effort: ReasoningEffort,
    #[serde(flatten)]
    pub extra: HashMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningEffort {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResources {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code_interpreter: Option<CodeInterpreterResources>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    pub file_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ResponseRequest {
    #[serde(flatten)]
    pub inner: endpoints::responses::response_object::RequestOfModelResponse,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modalities: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<ResponseFormat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ReasoningSettings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_resources: Option<ToolResources>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachments: Option<Vec<Attachment>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
}

impl ResponseRequest {
    pub fn upstream(&self) -> &endpoints::responses::response_object::RequestOfModelResponse {
        &self.inner
    }
}

pub type ResponseReply = endpoints::responses::response_object::ResponseObject;

pub use endpoints::responses::{
    items::{
        ResponseItemInputMessageContent, ResponseOutputItem, ResponseOutputItemOutputMessageContent,
    },
    response_object::{Input, InputItem, InputMessageContent, ToolChoice},
};

#[derive(Debug, Serialize, Deserialize)]
pub struct Session {
    pub response_id: String,
    pub created: i64,
    pub model_used: String,
    pub messages: Vec<SessionMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extended_data: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SessionMessage {
    pub role: String,
    pub content: String,
    pub tokens: u64,
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
        let mut messages = Vec::new();

        if let Some(inst) = instructions {
            messages.push(SessionMessage {
                role: "system".to_string(),
                content: inst,
                tokens: 0,
                created_at: now,
                response_time: None,
                response_id: None,
            });
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
        tokens: u64,
        response_time: Option<i64>,
        response_id: Option<String>,
    ) {
        let now = chrono::Utc::now().timestamp();
        self.messages.push(SessionMessage {
            role,
            content,
            tokens,
            created_at: now,
            response_time,
            response_id,
        });
    }

    #[allow(dead_code)]
    pub fn get_conversation_history(&self) -> Vec<(String, String)> {
        let mut history = Vec::new();

        for msg in &self.messages {
            history.push((msg.role.clone(), msg.content.clone()));
        }

        history
    }

    #[allow(dead_code)]
    pub fn total_tokens(&self) -> u64 {
        self.messages.iter().map(|msg| msg.tokens).sum()
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
        let message = &session.messages[0];
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
        let assistant_msg = &session.messages[1];
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
}
