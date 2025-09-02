use std::{collections::HashMap, env, net::SocketAddr, sync::Arc};

use axum::{
    Router,
    extract::{Query, State},
    response::Html,
    routing::get,
};
use chat_prompts::MergeRagContextPolicy;
use clap::ValueEnum;
use endpoints::chat::McpTransport;
use rmcp::{
    model::{ClientCapabilities, ClientInfo, Implementation, Tool as RmcpTool},
    service::ServiceExt,
    transport::{
        SseClientTransport, StreamableHttpClientTransport,
        auth::{AuthClient, OAuthState},
        sse_client::SseClientConfig,
        streamable_http_client::StreamableHttpClientTransportConfig,
    },
};
use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncWriteExt, BufWriter},
    sync::{Mutex, RwLock as TokioRwLock, oneshot},
};

use crate::{
    dual_debug, dual_error, dual_info,
    error::{ServerError, ServerResult},
    mcp::{MCP_SERVICES, MCP_TOOLS, McpService},
};

const MCP_REDIRECT_URI: &str = "http://localhost:8080/callback";
const CALLBACK_PORT: u16 = 8080;
const CALLBACK_HTML: &str = include_str!("auth/callback.html");

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Config {
    pub server: ServerConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory: Option<MemoryConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rag: Option<RagConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_info_push_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_health_push_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcp: Option<McpConfig>,
}
impl Config {
    pub async fn load(path: impl AsRef<std::path::Path>) -> ServerResult<Self> {
        let config = config::Config::builder()
            .add_source(config::File::with_name(path.as_ref().to_str().unwrap()))
            .build()
            .map_err(|e| {
                let err_msg = format!("Failed to load config file: {e}");
                dual_error!("{}", &err_msg);
                ServerError::FailedToLoadConfig(err_msg)
            })?;

        let mut config = config.try_deserialize::<Self>().map_err(|e| {
            let err_msg = format!("Failed to deserialize config: {e}");
            dual_error!("{}", &err_msg);
            ServerError::FailedToLoadConfig(err_msg)
        })?;

        if let Some(mcp_config) = config.mcp.as_mut()
            && !mcp_config.server.tool_servers.is_empty()
        {
            for server_config in mcp_config.server.tool_servers.iter_mut() {
                server_config.connect_mcp_server().await?;
            }
        }

        dual_debug!("config:\n{:#?}", config);

        Ok(config)
    }
}

// Add Default implementation for Config
impl Default for Config {
    fn default() -> Self {
        Self {
            server: ServerConfig {
                host: "127.0.0.1".to_string(),
                port: 3389,
                chat_mode: ChatMode::default(),
            },
            memory: None,
            rag: None,
            server_info_push_url: None,
            server_health_push_url: None,
            mcp: None,
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone, Copy, Default)]
pub enum ChatMode {
    #[default]
    #[serde(rename = "normal")]
    Normal,
    #[serde(rename = "react")]
    React,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    #[serde(default)]
    pub chat_mode: ChatMode,
}

/// Summarization strategy for handling conversation history
#[derive(Debug, Default, Copy, Deserialize, Serialize, Clone, PartialEq)]
pub enum SummarizationStrategy {
    /// Incremental summarization: Use existing summary + new messages
    /// This is more efficient but may lose some context over time
    #[default]
    Incremental,
    /// Full history summarization: Re-summarize all historical messages
    /// This provides better context but is more computationally expensive
    FullHistory,
}

impl std::fmt::Display for SummarizationStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            SummarizationStrategy::Incremental => write!(f, "Incremental"),
            SummarizationStrategy::FullHistory => write!(f, "FullHistory"),
        }
    }
}

/// Memory system configuration
///
/// Controls the behavior of conversation memory management including
/// automatic summarization and message retention policies.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct MemoryConfig {
    /// Enable or disable memory functionality
    pub enable: bool,
    /// Path to SQLite database file for storing conversation history
    pub database_path: String,
    /// Maximum context window size in tokens
    pub context_window: u64,
    /// Enable automatic message summarization when limits are reached
    pub auto_summarize: bool,

    /// Summarization strategy to use when generating summaries
    /// - Incremental: Use existing summary + new messages (default, more efficient)
    /// - FullHistory: Re-summarize all historical messages (better context, more expensive)
    pub summarization_strategy: SummarizationStrategy,

    /// Base number for calculating minimum messages to keep after summarization.
    /// Actual kept messages = summarize_threshold / 2
    /// This should be LESS than max_stored_messages to allow effective summarization.
    ///
    /// Note: The name "summarize_threshold" is somewhat misleading - it's not the threshold
    /// for triggering summarization, but rather the base for calculating retention count.
    /// A more descriptive name would be "min_keep_messages_base" or similar.
    pub summarize_threshold: u32,

    /// Maximum number of messages to store before triggering summarization.
    /// When message count reaches this limit, summarization is automatically triggered.
    /// This should be GREATER than summarize_threshold for proper operation.
    ///
    /// This name accurately reflects its purpose as the trigger point for summarization.
    pub max_stored_messages: u32,

    /// Base URL for the summary service used to generate conversation summaries.
    /// This service is called when automatic summarization is triggered.
    pub summary_service_base_url: String,

    /// API key for authenticating with the summary service.
    /// Leave empty if the summary service doesn't require authentication.
    pub summary_service_api_key: String,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            enable: false,
            database_path: "data/memory.db".to_string(),
            context_window: 8192,
            auto_summarize: true,
            summarization_strategy: SummarizationStrategy::default(),
            // Default configuration follows the principle: max_stored_messages > summarize_threshold
            // This allows for effective summarization: 20 messages trigger → keep 6 → summarize 14
            summarize_threshold: 12, // Keep 6 messages minimum (12/2)
            max_stored_messages: 20, // Trigger summarization at 20 messages
            summary_service_base_url: "http://localhost:10086/v1".to_string(),
            summary_service_api_key: String::new(),
        }
    }
}

#[derive(Debug, Serialize, Clone)]
pub struct RagConfig {
    pub enable: bool,
    pub prompt: Option<String>,
    pub policy: MergeRagContextPolicy,
    pub context_window: u64,
}
impl<'de> Deserialize<'de> for RagConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct RagConfigHelper {
            enable: bool,
            policy: String,
            context_window: u64,
        }

        let helper = RagConfigHelper::deserialize(deserializer)?;

        let policy = MergeRagContextPolicy::from_str(&helper.policy, true)
            .map_err(|e| serde::de::Error::custom(e.to_string()))?;

        Ok(RagConfig {
            enable: helper.enable,
            prompt: None,
            policy,
            context_window: helper.context_window,
        })
    }
}

// #[derive(Debug, Deserialize, Serialize, Clone)]
// pub struct RagVectorSearchConfig {
//     pub url: String,
//     pub collection_name: Vec<String>,
//     pub limit: u64,
//     pub score_threshold: f32,
// }

// #[derive(Debug, Default, Deserialize, Serialize, Clone)]
// pub struct KwSearchConfig {
//     pub enable: bool,
//     pub url: String,
//     pub index_name: String,
// }

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct McpConfig {
    #[serde(rename = "server")]
    pub server: McpServerConfig,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct McpServerConfig {
    #[serde(rename = "tool")]
    pub tool_servers: Vec<McpToolServerConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct McpToolServerConfig {
    pub name: String,
    pub transport: McpTransport,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oauth_url: Option<String>,
    pub enable: bool,
    #[serde(skip_deserializing)]
    pub tools: Option<Vec<RmcpTool>>,
    pub fallback_message: Option<String>,
}
impl McpToolServerConfig {
    /// Connect the mcp server if it is enabled
    pub async fn connect_mcp_server(&mut self) -> ServerResult<()> {
        if self.enable {
            // Validate URL configuration: exactly one must be non-empty
            let mut use_oauth = false;
            let server_url = match (&self.url, &self.oauth_url) {
                (Some(url), None) => url,
                (None, Some(oauth_url)) => {
                    use_oauth = true;
                    oauth_url
                }
                (Some(_), Some(_)) => {
                    let err_msg = format!(
                        "Invalid configuration for mcp server '{}': Both url and oauth_url cannot be set at the same time",
                        self.name
                    );
                    dual_error!("{}", err_msg);
                    return Err(ServerError::Operation(err_msg));
                }
                (None, None) => {
                    let err_msg = format!(
                        "Invalid configuration for mcp server '{}': Either url or oauth_url must be set",
                        self.name
                    );
                    dual_error!("{}", err_msg);
                    return Err(ServerError::Operation(err_msg));
                }
            };

            match self.transport {
                McpTransport::Sse => {
                    let url = server_url.trim_end_matches('/');

                    let service = match use_oauth {
                        false => {
                            if !url.ends_with("/sse") {
                                let err_msg = format!(
                                    "Invalid mcp tools sse URL: {url}. The correct format should end with `/sse`",
                                );
                                dual_error!("{}", err_msg);
                                return Err(ServerError::Operation(err_msg.to_string()));
                            }
                            dual_debug!("Sync mcp tools from mcp server: {}", url);

                            // create a sse transport
                            let transport = SseClientTransport::start(url).await.map_err(|e| {
                                let err_msg = format!("Failed to create sse transport: {e}");
                                dual_error!("{}", &err_msg);
                                ServerError::McpOperation(err_msg)
                            })?;

                            // create a mcp client
                            let client_info = ClientInfo {
                                protocol_version: Default::default(),
                                capabilities: ClientCapabilities::default(),
                                client_info: Implementation {
                                    name: env!("CARGO_PKG_NAME").to_string(),
                                    version: env!("CARGO_PKG_VERSION").to_string(),
                                },
                            };
                            client_info.into_dyn().serve(transport).await.map_err(|e| {
                                let err_msg = format!(
                                    "Failed to connect to mcp server (name: {}, url: {}, transport: {}). {e}. Please check if the mcp server is running.",
                                    self.name, url, self.transport
                                );
                                dual_error!("{}", &err_msg);
                                ServerError::McpOperation(err_msg)
                            })?
                        }
                        true => {
                            // it is a http server for handling callback
                            // Create channel for receiving authorization code
                            let (code_sender, code_receiver) = oneshot::channel::<String>();

                            // Create app state
                            let app_state = AppState {
                                code_receiver: Arc::new(Mutex::new(Some(code_sender))),
                            };

                            // Start HTTP server for handling callbacks
                            let app = Router::new()
                                .route("/callback", get(callback_handler))
                                .with_state(app_state);

                            let addr = SocketAddr::from(([127, 0, 0, 1], CALLBACK_PORT));
                            tracing::info!("Starting callback server at: http://{}", addr);

                            // Start server in a separate task
                            tokio::spawn(async move {
                                let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
                                let result = axum::serve(listener, app).await;

                                if let Err(e) = result {
                                    tracing::error!("Callback server error: {}", e);
                                }
                            });

                            // Get server URL
                            tracing::info!("Using MCP server OAuth URL: {}", url);

                            // Initialize oauth state machine
                            let mut oauth_state =
                                OAuthState::new(url, None).await.map_err(|e| {
                                    let err_msg =
                                        format!("Failed to initialize oauth state machine: {e}");
                                    dual_error!("{}", err_msg);
                                    ServerError::McpOperation(err_msg)
                                })?;

                            // Get metadata to view supported scopes
                            if let OAuthState::Unauthorized(manager) = &mut oauth_state {
                                let metadata = manager.discover_metadata().await.map_err(|e| {
                                    let err_msg = format!("Failed to discover metadata: {e}");
                                    dual_error!("{}", err_msg);
                                    ServerError::McpOperation(err_msg.to_string())
                                })?;
                                if let Some(supported_scopes) = metadata.scopes_supported {
                                    dual_debug!("Server supported scopes: {:?}", supported_scopes);
                                    // Use server supported scopes
                                    oauth_state
                                        .start_authorization(
                                            &supported_scopes
                                                .iter()
                                                .map(|s| s.as_str())
                                                .collect::<Vec<_>>(),
                                            MCP_REDIRECT_URI,
                                        )
                                        .await
                                        .map_err(|e| {
                                            let err_msg =
                                                format!("Failed to start authorization: {e}");
                                            dual_error!("{}", err_msg);
                                            ServerError::McpOperation(err_msg)
                                        })?;
                                } else {
                                    let err_msg = "Failed to get supported scopes from mcp server";
                                    dual_error!("{}", err_msg);
                                    return Err(ServerError::McpOperation(err_msg.to_string()));
                                }
                            }

                            // Output authorization URL to user
                            let mut output = BufWriter::new(tokio::io::stdout());
                            output
                                .write_all(b"\n=== MCP OAuth Client ===\n\n")
                                .await
                                .map_err(|e| {
                                    let err_msg = format!("Failed to write to stdout: {e}");
                                    dual_error!("{}", err_msg);
                                    ServerError::McpOperation(err_msg)
                                })?;
                            output.write_all(b"Please open the following URL in your browser to authorize:\n\n")
                            .await.map_err(|e| {
                                let err_msg = format!("Failed to write to stdout: {e}");
                                dual_error!("{}", err_msg);
                                ServerError::McpOperation(err_msg)
                            })?;

                            output
                                .write_all(
                                    oauth_state
                                        .get_authorization_url()
                                        .await
                                        .map_err(|e| {
                                            let err_msg =
                                                format!("Failed to get authorization url: {e}");
                                            dual_error!("{}", err_msg);
                                            ServerError::McpOperation(err_msg)
                                        })?
                                        .as_bytes(),
                                )
                                .await
                                .map_err(|e| {
                                    let err_msg = format!("Failed to write to stdout: {e}");
                                    dual_error!("{}", err_msg);
                                    ServerError::McpOperation(err_msg)
                                })?;
                            output
                                .write_all(b"\n\nWaiting for browser callback, please do not close this window...\n")
                                .await.map_err(|e| {
                                    let err_msg = format!("Failed to write to stdout: {e}");
                                    dual_error!("{}", err_msg);
                                    ServerError::McpOperation(err_msg)
                                })?;
                            output.flush().await.map_err(|e| {
                                let err_msg = format!("Failed to flush stdout: {e}");
                                dual_error!("{}", err_msg);
                                ServerError::McpOperation(err_msg)
                            })?;

                            // Wait for authorization code
                            tracing::info!("Waiting for authorization code...");
                            let auth_code = code_receiver.await.map_err(|e| {
                                let err_msg = format!("Failed to get authorization code: {e}");
                                dual_error!("{}", err_msg);
                                ServerError::McpOperation(err_msg)
                            })?;
                            tracing::info!("Received authorization code: {}", auth_code);
                            // Exchange code for access token
                            tracing::info!("Exchanging authorization code for access token...");
                            oauth_state.handle_callback(&auth_code).await.map_err(|e| {
                                let err_msg = format!("Failed to handle callback: {e}");
                                dual_error!("{}", err_msg);
                                ServerError::McpOperation(err_msg)
                            })?;
                            tracing::info!("Successfully obtained access token");

                            output
                                .write_all(
                                    b"\nAuthorization successful! Access token obtained.\n\n",
                                )
                                .await
                                .map_err(|e| {
                                    let err_msg = format!("Failed to write to stdout: {e}");
                                    dual_error!("{}", err_msg);
                                    ServerError::McpOperation(err_msg)
                                })?;
                            output.flush().await.map_err(|e| {
                                let err_msg = format!("Failed to flush stdout: {e}");
                                dual_error!("{}", err_msg);
                                ServerError::McpOperation(err_msg)
                            })?;

                            // Create authorized transport, this transport is authorized by the oauth state machine
                            tracing::info!("Establishing authorized connection to MCP server...");
                            let am = oauth_state.into_authorization_manager().ok_or_else(|| {
                                let err_msg = "Failed to get authorization manager";
                                dual_error!("{}", err_msg);
                                ServerError::McpOperation(err_msg.to_string())
                            })?;
                            let client = AuthClient::new(reqwest::Client::default(), am);
                            let transport = SseClientTransport::start_with_client(
                                client,
                                SseClientConfig {
                                    sse_endpoint: url.into(),
                                    ..Default::default()
                                },
                            )
                            .await
                            .map_err(|e| {
                                let err_msg = format!("Failed to create authorized transport: {e}");
                                dual_error!("{}", err_msg);
                                ServerError::McpOperation(err_msg)
                            })?;

                            // Create client and connect to MCP server
                            let client_info = ClientInfo {
                                protocol_version: Default::default(),
                                capabilities: ClientCapabilities::default(),
                                client_info: Implementation {
                                    name: env!("CARGO_PKG_NAME").to_string(),
                                    version: env!("CARGO_PKG_VERSION").to_string(),
                                },
                            };
                            let service =
                                client_info.into_dyn().serve(transport).await.map_err(|e| {
                                    let err_msg = format!(
                                        "Failed to connect to mcp server (name: {}, url: {}, transport: {}). {e}. Please check if the mcp server is running.",
                                        self.name, url, self.transport
                                    );
                                    dual_error!("{}", &err_msg);
                                    ServerError::McpOperation(err_msg)
                                })?;
                            tracing::info!("Successfully connected to MCP server");

                            service
                        }
                    };

                    // list tools
                    let tools = service.list_all_tools().await.map_err(|e| {
                        let err_msg = format!("Failed to list tools: {e}");
                        dual_error!("{}", &err_msg);
                        ServerError::McpOperation(err_msg)
                    })?;
                    dual_info!("Found {} tools from {} mcp server", tools.len(), self.name,);

                    dual_debug!(
                        "Retrieved mcp tools: {}",
                        serde_json::to_string_pretty(&tools).unwrap()
                    );

                    // update tools
                    self.tools = Some(tools.clone());

                    let mut client = McpService::new(self.name.clone(), service);
                    client.tools = tools.iter().map(|tool| tool.name.to_string()).collect();
                    client.fallback_message = self.fallback_message.clone();

                    // print name of all tools
                    for (idx, tool) in tools.iter().enumerate() {
                        dual_debug!(
                            "Tool {} - name: {}, description: {}",
                            idx,
                            tool.name,
                            tool.description.as_deref().unwrap_or("No description"),
                        );

                        match MCP_TOOLS.get() {
                            Some(mcp_tools) => {
                                let mut tools = mcp_tools.write().await;
                                tools.insert(tool.name.to_string(), self.name.clone());
                            }
                            None => {
                                let tools =
                                    HashMap::from([(tool.name.to_string(), self.name.clone())]);

                                MCP_TOOLS.set(TokioRwLock::new(tools)).map_err(|_| {
                                    let err_msg = "Failed to set MCP_TOOLS";
                                    dual_error!("{}", err_msg);
                                    ServerError::Operation(err_msg.to_string())
                                })?;
                            }
                        }
                    }

                    // add mcp client to MCP_CLIENTS
                    match MCP_SERVICES.get() {
                        Some(clients) => {
                            let mut clients = clients.write().await;
                            clients.insert(self.name.clone(), TokioRwLock::new(client));
                        }
                        None => {
                            MCP_SERVICES
                                .set(TokioRwLock::new(HashMap::from([(
                                    self.name.clone(),
                                    TokioRwLock::new(client),
                                )])))
                                .map_err(|_| {
                                    let err_msg = "Failed to set MCP_CLIENTS";
                                    dual_error!("{}", err_msg);
                                    ServerError::Operation(err_msg.to_string())
                                })?;
                        }
                    }
                }
                McpTransport::StreamHttp => {
                    let url = server_url.trim_end_matches('/');

                    let service = match use_oauth {
                        false => {
                            if !url.ends_with("/mcp") {
                                let err_msg = format!(
                                    "Invalid mcp tools stream-http URL: {url}. The correct format should end with `/mcp`",
                                );
                                dual_error!("{}", err_msg);
                                return Err(ServerError::Operation(err_msg.to_string()));
                            }
                            dual_debug!("Sync mcp tools from mcp server: {}", url);

                            // create a stream-http transport
                            let transport = StreamableHttpClientTransport::from_uri(url);

                            // create a mcp client
                            let client_info = ClientInfo {
                                protocol_version: Default::default(),
                                capabilities: ClientCapabilities::default(),
                                client_info: Implementation {
                                    name: env!("CARGO_PKG_NAME").to_string(),
                                    version: env!("CARGO_PKG_VERSION").to_string(),
                                },
                            };
                            client_info.into_dyn().serve(transport).await.map_err(|e| {
                                let err_msg = format!(
                                    "Failed to connect to mcp server (name: {}, url: {}, transport: {}). {e}. Please check if the mcp server is running.",
                                    self.name, server_url, self.transport
                                );
                                dual_error!("{}", &err_msg);
                                ServerError::McpOperation(err_msg)
                            })?
                        }
                        true => {
                            // it is a http server for handling callback
                            // Create channel for receiving authorization code
                            let (code_sender, code_receiver) = oneshot::channel::<String>();

                            // Create app state
                            let app_state = AppState {
                                code_receiver: Arc::new(Mutex::new(Some(code_sender))),
                            };

                            // Start HTTP server for handling callbacks
                            let app = Router::new()
                                .route("/callback", get(callback_handler))
                                .with_state(app_state);

                            let addr = SocketAddr::from(([127, 0, 0, 1], CALLBACK_PORT));
                            tracing::info!("Starting callback server at: http://{}", addr);

                            // Start server in a separate task
                            tokio::spawn(async move {
                                let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
                                let result = axum::serve(listener, app).await;

                                if let Err(e) = result {
                                    tracing::error!("Callback server error: {}", e);
                                }
                            });

                            // Get server URL
                            tracing::info!("Using MCP server OAuth URL: {}", url);

                            // Initialize oauth state machine
                            let mut oauth_state =
                                OAuthState::new(url, None).await.map_err(|e| {
                                    let err_msg =
                                        format!("Failed to initialize oauth state machine: {e}");
                                    dual_error!("{}", err_msg);
                                    ServerError::McpOperation(err_msg)
                                })?;

                            // Get metadata to view supported scopes
                            if let OAuthState::Unauthorized(manager) = &mut oauth_state {
                                let metadata = manager.discover_metadata().await.map_err(|e| {
                                    let err_msg = format!("Failed to discover metadata: {e}");
                                    dual_error!("{}", err_msg);
                                    ServerError::McpOperation(err_msg.to_string())
                                })?;
                                if let Some(supported_scopes) = metadata.scopes_supported {
                                    dual_debug!("Server supported scopes: {:?}", supported_scopes);
                                    // Use server supported scopes
                                    oauth_state
                                        .start_authorization(
                                            &supported_scopes
                                                .iter()
                                                .map(|s| s.as_str())
                                                .collect::<Vec<_>>(),
                                            MCP_REDIRECT_URI,
                                        )
                                        .await
                                        .map_err(|e| {
                                            let err_msg =
                                                format!("Failed to start authorization: {e}");
                                            dual_error!("{}", err_msg);
                                            ServerError::McpOperation(err_msg)
                                        })?;
                                } else {
                                    let err_msg = "Failed to get supported scopes from mcp server";
                                    dual_error!("{}", err_msg);
                                    return Err(ServerError::McpOperation(err_msg.to_string()));
                                }
                            }

                            // Output authorization URL to user
                            let mut output = BufWriter::new(tokio::io::stdout());
                            output
                                .write_all(b"\n=== MCP OAuth Client ===\n\n")
                                .await
                                .map_err(|e| {
                                    let err_msg = format!("Failed to write to stdout: {e}");
                                    dual_error!("{}", err_msg);
                                    ServerError::McpOperation(err_msg)
                                })?;
                            output.write_all(b"Please open the following URL in your browser to authorize:\n\n")
                            .await.map_err(|e| {
                                let err_msg = format!("Failed to write to stdout: {e}");
                                dual_error!("{}", err_msg);
                                ServerError::McpOperation(err_msg)
                            })?;

                            output
                                .write_all(
                                    oauth_state
                                        .get_authorization_url()
                                        .await
                                        .map_err(|e| {
                                            let err_msg =
                                                format!("Failed to get authorization url: {e}");
                                            dual_error!("{}", err_msg);
                                            ServerError::McpOperation(err_msg)
                                        })?
                                        .as_bytes(),
                                )
                                .await
                                .map_err(|e| {
                                    let err_msg = format!("Failed to write to stdout: {e}");
                                    dual_error!("{}", err_msg);
                                    ServerError::McpOperation(err_msg)
                                })?;
                            output
                                .write_all(b"\n\nWaiting for browser callback, please do not close this window...\n")
                                .await.map_err(|e| {
                                    let err_msg = format!("Failed to write to stdout: {e}");
                                    dual_error!("{}", err_msg);
                                    ServerError::McpOperation(err_msg)
                                })?;
                            output.flush().await.map_err(|e| {
                                let err_msg = format!("Failed to flush stdout: {e}");
                                dual_error!("{}", err_msg);
                                ServerError::McpOperation(err_msg)
                            })?;

                            // Wait for authorization code
                            tracing::info!("Waiting for authorization code...");
                            let auth_code = code_receiver.await.map_err(|e| {
                                let err_msg = format!("Failed to get authorization code: {e}");
                                dual_error!("{}", err_msg);
                                ServerError::McpOperation(err_msg)
                            })?;
                            tracing::info!("Received authorization code: {}", auth_code);
                            // Exchange code for access token
                            tracing::info!("Exchanging authorization code for access token...");
                            oauth_state.handle_callback(&auth_code).await.map_err(|e| {
                                let err_msg = format!("Failed to handle callback: {e}");
                                dual_error!("{}", err_msg);
                                ServerError::McpOperation(err_msg)
                            })?;
                            tracing::info!("Successfully obtained access token");

                            output
                                .write_all(
                                    b"\nAuthorization successful! Access token obtained.\n\n",
                                )
                                .await
                                .map_err(|e| {
                                    let err_msg = format!("Failed to write to stdout: {e}");
                                    dual_error!("{}", err_msg);
                                    ServerError::McpOperation(err_msg)
                                })?;
                            output.flush().await.map_err(|e| {
                                let err_msg = format!("Failed to flush stdout: {e}");
                                dual_error!("{}", err_msg);
                                ServerError::McpOperation(err_msg)
                            })?;

                            // Create authorized transport, this transport is authorized by the oauth state machine
                            tracing::info!("Establishing authorized connection to MCP server...");
                            let am = oauth_state.into_authorization_manager().ok_or_else(|| {
                                let err_msg = "Failed to get authorization manager";
                                dual_error!("{}", err_msg);
                                ServerError::McpOperation(err_msg.to_string())
                            })?;
                            let client = AuthClient::new(reqwest::Client::default(), am);

                            // Use StreamableHttpClientTransport
                            let transport = StreamableHttpClientTransport::with_client(
                                client,
                                StreamableHttpClientTransportConfig {
                                    uri: url.into(),
                                    ..Default::default()
                                },
                            );

                            // Create client and connect to MCP server
                            let client_info = ClientInfo {
                                protocol_version: Default::default(),
                                capabilities: ClientCapabilities::default(),
                                client_info: Implementation {
                                    name: env!("CARGO_PKG_NAME").to_string(),
                                    version: env!("CARGO_PKG_VERSION").to_string(),
                                },
                            };
                            client_info.into_dyn().serve(transport).await.map_err(|e| {
                                let err_msg = format!(
                                    "Failed to connect to mcp server (name: {}, url: {}, transport: {}). {e}. Please check if the mcp server is running.",
                                    self.name, url, self.transport
                                );
                                dual_error!("{}", &err_msg);
                                ServerError::McpOperation(err_msg)
                            })?
                        }
                    };

                    // list tools
                    let tools = service.list_all_tools().await.map_err(|e| {
                        let err_msg = format!("Failed to list tools: {e}");
                        dual_error!("{}", &err_msg);
                        ServerError::McpOperation(err_msg)
                    })?;
                    dual_info!("Found {} tools from {} mcp server", tools.len(), self.name,);

                    dual_debug!(
                        "Retrieved mcp tools: {}",
                        serde_json::to_string_pretty(&tools).unwrap()
                    );

                    // update tools
                    self.tools = Some(tools.clone());

                    let mut client = McpService::new(self.name.clone(), service);
                    client.tools = tools.iter().map(|tool| tool.name.to_string()).collect();
                    client.fallback_message = self.fallback_message.clone();

                    // print name of all tools
                    for (idx, tool) in tools.iter().enumerate() {
                        dual_debug!(
                            "Tool {} - name: {}, description: {}",
                            idx,
                            tool.name,
                            tool.description.as_deref().unwrap_or("No description"),
                        );

                        match MCP_TOOLS.get() {
                            Some(mcp_tools) => {
                                let mut tools = mcp_tools.write().await;
                                tools.insert(tool.name.to_string(), self.name.clone());
                            }
                            None => {
                                let tools =
                                    HashMap::from([(tool.name.to_string(), self.name.clone())]);

                                MCP_TOOLS.set(TokioRwLock::new(tools)).map_err(|_| {
                                    let err_msg = "Failed to set MCP_TOOLS";
                                    dual_error!("{}", err_msg);
                                    ServerError::Operation(err_msg.to_string())
                                })?;
                            }
                        }
                    }

                    // add mcp client to MCP_CLIENTS
                    match MCP_SERVICES.get() {
                        Some(clients) => {
                            let mut clients = clients.write().await;
                            clients.insert(self.name.clone(), TokioRwLock::new(client));
                        }
                        None => {
                            MCP_SERVICES
                                .set(TokioRwLock::new(HashMap::from([(
                                    self.name.clone(),
                                    TokioRwLock::new(client),
                                )])))
                                .map_err(|_| {
                                    let err_msg = "Failed to set MCP_CLIENTS";
                                    dual_error!("{}", err_msg);
                                    ServerError::Operation(err_msg.to_string())
                                })?;
                        }
                    }
                }
                _ => {
                    let err_msg = format!("Unsupported transport: {}", self.transport);
                    dual_error!("{}", err_msg);
                    return Err(ServerError::Operation(err_msg.to_string()));
                }
            }
        }

        Ok(())
    }
}

// #[derive(Debug, Deserialize, Serialize, Clone)]
// pub enum Transport {
//     #[serde(rename = "sse")]
//     Sse,
//     #[serde(rename = "stdio")]
//     Stdio,
//     #[serde(rename = "stream-http")]
//     StreamHttp,
// }
// impl std::fmt::Display for Transport {
//     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
//         match self {
//             Transport::Sse => write!(f, "sse"),
//             Transport::Stdio => write!(f, "stdio"),
//             Transport::StreamHttp => write!(f, "streamable-http"),
//         }
//     }
// }

// #[derive(Debug, Deserialize, Serialize, Clone)]
// pub enum Transport {
//     #[serde(rename = "sse")]
//     Sse,
//     #[serde(rename = "stdio")]
//     Stdio,
//     #[serde(rename = "stream-http")]
//     StreamHttp,
// }
// impl std::fmt::Display for Transport {
//     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
//         match self {
//             Transport::Sse => write!(f, "sse"),
//             Transport::Stdio => write!(f, "stdio"),
//             Transport::StreamHttp => write!(f, "streamable-http"),
//         }
//     }
// }

#[derive(Debug, Clone)]
struct AppState {
    code_receiver: Arc<Mutex<Option<oneshot::Sender<String>>>>,
}

#[derive(Debug, Deserialize)]
struct CallbackParams {
    code: String,
    #[allow(dead_code)]
    state: Option<String>,
}

async fn callback_handler(
    Query(params): Query<CallbackParams>,
    State(state): State<AppState>,
) -> Html<String> {
    tracing::info!("Received callback with code: {}", params.code);

    // Send the code to the main thread
    if let Some(sender) = state.code_receiver.lock().await.take() {
        let _ = sender.send(params.code);
    }
    // Return success page
    Html(CALLBACK_HTML.to_string())
}
