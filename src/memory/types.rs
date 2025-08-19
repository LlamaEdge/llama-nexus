use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MessageRole {
    User,
    Assistant,
    System,
    Tool,
}

impl std::fmt::Display for MessageRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MessageRole::User => write!(f, "user"),
            MessageRole::Assistant => write!(f, "assistant"),
            MessageRole::System => write!(f, "system"),
            MessageRole::Tool => write!(f, "tool"),
        }
    }
}

impl std::str::FromStr for MessageRole {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "user" => Ok(MessageRole::User),
            "assistant" => Ok(MessageRole::Assistant),
            "system" => Ok(MessageRole::System),
            "tool" => Ok(MessageRole::Tool),
            _ => Err(anyhow::anyhow!("Invalid message role: {}", s)),
        }
    }
}

// 完整存储的消息记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredMessage {
    pub id: String,
    pub conversation_id: String,
    pub role: MessageRole,
    pub content: String,
    pub timestamp: DateTime<Utc>,
    pub sequence: i64,
    pub tokens: Option<usize>,
    pub tool_calls: Vec<StoredToolCall>,
}

// 完整存储的工具调用
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
    pub result: Option<StoredToolResult>,
    pub sequence: i32,
}

// 完整存储的工具执行结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredToolResult {
    pub content: serde_json::Value,
    pub success: bool,
    pub error: Option<String>,
    pub execution_time_ms: Option<i64>,
    pub timestamp: DateTime<Utc>,
}

// 会话记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredConversation {
    pub id: String,
    pub user_id: Option<String>,
    pub title: Option<String>,
    pub model_name: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub message_count: i64,
    pub total_tokens: i64,
    pub summary: Option<String>,
    pub last_summary_sequence: Option<i64>,
}

// 运行时的上下文管理
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ContextMemory {
    pub conversation_id: String,
    pub working_messages: Vec<StoredMessage>,
    pub summary: Option<String>,
    pub total_tokens: usize,
    pub max_context_tokens: usize,
}

// 用于与模型交互的消息格式
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct ModelMessage {
    pub role: String,
    pub content: String,
    pub tool_calls: Option<Vec<ModelToolCall>>,
    pub tool_call_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelToolCall {
    pub id: String,
    pub r#type: String,
    pub function: ModelToolFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelToolFunction {
    pub name: String,
    pub arguments: String,
}

// 统计信息
#[derive(Debug, Serialize)]
pub struct MemoryStats {
    pub total_conversations: i64,
    pub total_messages: i64,
    pub total_tool_calls: i64,
    pub total_tokens: i64,
    pub database_size_mb: f64,
    pub most_used_tools: Vec<(String, i64)>,
    pub conversations_by_model: Vec<(String, i64)>,
}

// 会话摘要信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationSummary {
    pub id: String,
    pub user_id: Option<String>,
    pub title: Option<String>,
    pub model_name: String,
    pub message_count: i64,
    pub last_message_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

// 导出格式
#[derive(Debug, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct ConversationExport {
    pub conversation: StoredConversation,
    pub messages: Vec<StoredMessage>,
    pub export_timestamp: DateTime<Utc>,
    pub version: String,
}

// 错误类型
#[derive(Debug, thiserror::Error)]
#[allow(dead_code)]
pub enum MemoryError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Conversation not found: {0}")]
    ConversationNotFound(String),

    #[error("Message not found: {0}")]
    MessageNotFound(String),

    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),

    #[error("Summarization failed: {0}")]
    SummarizationFailed(String),

    #[error("Invalid data: {0}")]
    InvalidData(String),
}

pub type MemoryResult<T> = Result<T, MemoryError>;
