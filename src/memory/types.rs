use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
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

impl From<ModelRole> for MessageRole {
    fn from(role: ModelRole) -> Self {
        match role {
            ModelRole::User => MessageRole::User,
            ModelRole::Assistant => MessageRole::Assistant,
            ModelRole::System => MessageRole::System,
            ModelRole::Tool => MessageRole::Tool,
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
    pub role: ModelRole,
    pub content: String,
    pub tool_calls: Option<Vec<ModelToolCall>>,
    pub tool_call_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum ModelRole {
    User,
    Assistant,
    System,
    Tool,
}

impl std::fmt::Display for ModelRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ModelRole::User => write!(f, "user"),
            ModelRole::Assistant => write!(f, "assistant"),
            ModelRole::System => write!(f, "system"),
            ModelRole::Tool => write!(f, "tool"),
        }
    }
}

impl std::str::FromStr for ModelRole {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "user" => Ok(ModelRole::User),
            "assistant" => Ok(ModelRole::Assistant),
            "system" => Ok(ModelRole::System),
            "tool" => Ok(ModelRole::Tool),
            _ => Err(anyhow::anyhow!("Invalid model role: {}", s)),
        }
    }
}

impl From<MessageRole> for ModelRole {
    fn from(role: MessageRole) -> Self {
        match role {
            MessageRole::User => ModelRole::User,
            MessageRole::Assistant => ModelRole::Assistant,
            MessageRole::System => ModelRole::System,
            MessageRole::Tool => ModelRole::Tool,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelToolCall {
    pub id: String,
    pub ty: String,
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
