use std::collections::HashMap;

use once_cell::sync::OnceCell;
use rmcp::{
    RoleClient,
    service::{DynService, RunningService},
};
use tokio::sync::RwLock as TokioRwLock;

// Global MCP clients
pub static MCP_SERVICES: OnceCell<TokioRwLock<HashMap<ServiceName, TokioRwLock<McpService>>>> =
    OnceCell::new();

pub(crate) const MCP_SEPARATOR: &str = "---";

pub(crate) const SEARCH_MCP_SERVER_NAMES: [&str; 6] = [
    "cardea-agentic-search",
    "cardea-agentic-search-mcp-server",
    "cardea-tidb-mcp-server",
    "cardea-qdrant-mcp-server",
    "cardea-elastic-mcp-server",
    "cardea-kwsearch-mcp-server",
];
pub(crate) const DEFAULT_SEARCH_FALLBACK_MESSAGE: &str = "I’m unable to retrieve the necessary information to answer your question right now. Please try rephrasing or asking about something else.";

pub type RawMcpService = RunningService<RoleClient, Box<dyn DynService<RoleClient>>>;
pub type ServiceName = String;
pub type McpToolName = String;

#[allow(dead_code)]
pub struct McpService {
    pub name: ServiceName,
    pub raw: RawMcpService,
    pub tools: Vec<McpToolName>,
    pub fallback_message: Option<String>,
}
impl McpService {
    pub fn new(name: impl AsRef<str>, raw: RawMcpService) -> Self {
        Self {
            name: name.as_ref().to_string(),
            raw,
            tools: Vec::new(),
            fallback_message: None,
        }
    }

    pub fn has_fallback_message(&self) -> bool {
        if let Some(fallback_message) = &self.fallback_message {
            !fallback_message.is_empty()
        } else {
            false
        }
    }
}
