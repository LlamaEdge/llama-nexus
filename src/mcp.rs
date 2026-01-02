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

// MCP tool naming constants
/// Prefix for MCP tool names
pub(crate) const MCP_PREFIX: &str = "mcp";
/// Separator for MCP tool name components
pub(crate) const MCP_SEPARATOR: &str = "__";

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

// ============================================================================
// MCP Tool Naming Utilities
// ============================================================================

/// Generates a full MCP tool name in the new format.
///
/// Format: `mcp__{server_name}__{tool_name}`
///
/// # Example
/// ```ignore
/// let name = format_mcp_tool_name("cardea-calculator", "sum");
/// assert_eq!(name, "mcp__cardea-calculator__sum");
/// ```
pub fn format_mcp_tool_name(server_name: &str, tool_name: &str) -> String {
    format!("{MCP_PREFIX}{MCP_SEPARATOR}{server_name}{MCP_SEPARATOR}{tool_name}")
}

/// Parses an MCP tool name in the new format.
///
/// Returns `Some((server_name, tool_name))` if the name matches the new format,
/// otherwise returns `None`.
///
/// # Example
/// ```ignore
/// let result = parse_mcp_tool_name("mcp__cardea-calculator__sum");
/// assert_eq!(result, Some(("cardea-calculator", "sum")));
/// ```
pub fn parse_mcp_tool_name(full_name: &str) -> Option<(&str, &str)> {
    let parts: Vec<&str> = full_name.split(MCP_SEPARATOR).collect();
    if parts.len() == 3 && parts[0] == MCP_PREFIX {
        Some((parts[1], parts[2]))
    } else {
        None
    }
}

/// Extracts the tool name part from a full MCP tool name.
///
/// If the name doesn't match the MCP format, returns the original name unchanged.
///
/// # Example
/// ```ignore
/// assert_eq!(extract_tool_name("mcp__server__search"), "search");
/// assert_eq!(extract_tool_name("local_tool"), "local_tool");
/// ```
pub fn extract_tool_name(full_name: &str) -> &str {
    if let Some((_, tool_name)) = parse_mcp_tool_name(full_name) {
        tool_name
    } else {
        full_name
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_mcp_tool_name() {
        let result = format_mcp_tool_name("cardea-calculator", "sum");
        assert_eq!(result, "mcp__cardea-calculator__sum");
    }

    #[test]
    fn test_format_mcp_tool_name_with_special_chars() {
        let result = format_mcp_tool_name("my-server", "get_data");
        assert_eq!(result, "mcp__my-server__get_data");
    }

    #[test]
    fn test_parse_mcp_tool_name_new_format() {
        let result = parse_mcp_tool_name("mcp__cardea-calculator__sum");
        assert_eq!(result, Some(("cardea-calculator", "sum")));
    }

    #[test]
    fn test_parse_mcp_tool_name_invalid() {
        assert_eq!(parse_mcp_tool_name("invalid_name"), None);
        assert_eq!(parse_mcp_tool_name("search---serverA"), None);
        assert_eq!(parse_mcp_tool_name("mcp__only_two_parts"), None);
        assert_eq!(parse_mcp_tool_name("wrong__server__tool"), None);
    }

    #[test]
    fn test_extract_tool_name() {
        // MCP format
        assert_eq!(extract_tool_name("mcp__server__search"), "search");
        assert_eq!(extract_tool_name("mcp__cardea-calculator__sum"), "sum");

        // Not MCP tool names - returns original
        assert_eq!(extract_tool_name("local_tool"), "local_tool");
        assert_eq!(extract_tool_name("some_function"), "some_function");
    }

    #[test]
    fn test_roundtrip_new_format() {
        let server = "my-server";
        let tool = "my_tool";
        let full_name = format_mcp_tool_name(server, tool);
        let (parsed_server, parsed_tool) = parse_mcp_tool_name(&full_name).unwrap();
        assert_eq!(parsed_server, server);
        assert_eq!(parsed_tool, tool);
    }
}
