mod chat;
mod config;
mod error;
mod handlers;
mod info;
mod mcp;
mod memory;
mod responses;
mod server;
mod utils;

use std::{
    collections::{HashMap, HashSet},
    net::{IpAddr, SocketAddr},
    path::PathBuf,
    str::FromStr,
    sync::Arc,
};

use axum::{
    body::Body,
    http::{self, HeaderValue, Request},
    routing::{Router, get, post},
};
use clap::Parser;
use config::Config;
use error::{ServerError, ServerResult};
use futures_util::stream::{self, StreamExt};
use once_cell::sync::OnceCell;
use tokio::{signal, sync::RwLock};
use tokio_util::sync::CancellationToken;
use tower_http::{
    cors::{Any, CorsLayer},
    services::ServeDir,
    trace::TraceLayer,
};
use tracing::Level;
use uuid::Uuid;

use crate::{
    info::ServerInfo,
    server::{Server, ServerGroup, ServerId, ServerKind},
};

// Global health check interval for downstream servers in seconds
pub(crate) static HEALTH_CHECK_INTERVAL: OnceCell<u64> = OnceCell::new();

#[derive(Debug, Parser)]
#[command(version = env!("CARGO_PKG_VERSION"), about = "LlamaEdge Nexus - A gateway service for LLM backends")]
struct Cli {
    /// Path to the config file
    #[arg(long, default_value = "config.toml", value_parser = clap::value_parser!(PathBuf))]
    config: PathBuf,
    /// Enable health check for downstream servers
    #[arg(long, default_value = "false")]
    check_health: bool,
    /// Health check interval for downstream servers in seconds
    #[arg(long, default_value = "60")]
    check_health_interval: u64,
    /// Root path for the Web UI files
    #[arg(long, default_value = "chatbot-ui")]
    web_ui: PathBuf,
    /// Log destination: "stdout", "file", or "both"
    #[arg(long, default_value = "stdout")]
    log_destination: String,
    /// Log file path (required when log_destination is "file" or "both")
    #[arg(long)]
    log_file: Option<String>,
}

#[tokio::main]
async fn main() -> ServerResult<()> {
    #[cfg(debug_assertions)]
    dotenv::dotenv().ok();

    #[cfg(not(debug_assertions))]
    {
        if std::path::Path::new(".env").exists() {
            panic!("Production should not contain `.env` file");
        }
    }

    // parse the command line arguments
    let cli = Cli::parse();

    // Validate log configuration
    if (cli.log_destination == "file" || cli.log_destination == "both") && cli.log_file.is_none() {
        eprintln!("Error: --log-file is required when --log-destination is 'file' or 'both'");
        return Err(ServerError::Operation("Missing log file path".to_string()));
    }

    // Initialize logging based on destination
    init_logging(&cli.log_destination, cli.log_file.as_deref())?;

    // log the version of the server
    dual_info!("Version: {}", env!("CARGO_PKG_VERSION"));

    // Load the config based on the command
    let config = Config::load(&cli.config).await?;

    // set the health check interval
    HEALTH_CHECK_INTERVAL
        .set(cli.check_health_interval)
        .map_err(|e| {
            let err_msg = format!("Failed to set health check interval: {e}");
            dual_error!("{err_msg}");
            ServerError::Operation(err_msg)
        })?;

    // socket address
    let addr = SocketAddr::from((
        config.server.host.parse::<IpAddr>().unwrap(),
        config.server.port,
    ));

    // Initialize memory system if enabled
    let memory = if let Some(memory_config) = &config.memory {
        if memory_config.enable {
            dual_info!("Memory system is enabled");

            // Ensure data directory exists
            if let Some(parent) = std::path::Path::new(&memory_config.database_path).parent() {
                tokio::fs::create_dir_all(parent).await.map_err(|e| {
                    let err_msg = format!("Failed to create memory data directory: {e}");
                    dual_error!("{err_msg}");
                    ServerError::Operation(err_msg)
                })?;
            }

            match crate::memory::CompleteChatMemory::new(memory_config.clone()).await {
                Ok(memory_system) => {
                    dual_info!("Memory system initialized successfully");
                    Some(Arc::new(memory_system))
                }
                Err(e) => {
                    dual_error!("Failed to initialize memory system: {}", e);
                    return Err(ServerError::Operation(format!(
                        "Memory initialization failed: {e}"
                    )));
                }
            }
        } else {
            dual_info!("Memory system is disabled");
            None
        }
    } else {
        dual_info!("Memory system is not configured");
        None
    };

    // Initialize application state
    let mut state = AppState::new(config, ServerInfo::default());

    // Attach memory system to state
    if let Some(memory_system) = memory {
        state = state.with_memory(memory_system);
    }

    let state = Arc::new(state);

    // Initialize responses state with lazy database initialization
    let db_path =
        std::env::var("NEXUS_RESPONSES_DB_PATH").unwrap_or_else(|_| "sessions.db".to_string());
    let responses_state = Arc::new(responses::ResponsesAppState::new(db_path, state.clone()));

    // Register servers defined in configuration file
    state.register_config_servers().await?;

    // Start the health check task if enabled
    if cli.check_health {
        dual_info!("Health check is enabled");
        Arc::clone(&state).start_health_check_task().await;
    }

    // Set up CORS
    let cors = CorsLayer::new()
        .allow_methods([http::Method::GET, http::Method::POST])
        .allow_headers(Any)
        .allow_origin(Any);

    // Set up the main router
    let mut main_router = Router::new()
        .route("/v1/chat/completions", post(handlers::chat_handler))
        .route("/v1/embeddings", post(handlers::embeddings_handler))
        .route(
            "/v1/audio/transcriptions",
            post(handlers::audio_transcriptions_handler),
        )
        .route(
            "/v1/audio/translations",
            post(handlers::audio_translations_handler),
        )
        .route("/v1/audio/speech", post(handlers::audio_tts_handler))
        .route("/v1/images/generations", post(handlers::image_handler))
        .route("/v1/images/edits", post(handlers::image_handler))
        .route("/v1/models", get(handlers::models_handler))
        .route("/v1/info", get(handlers::info_handler))
        .route(
            "/admin/servers/register",
            post(handlers::admin::register_downstream_server_handler),
        )
        .route(
            "/admin/servers/unregister",
            post(handlers::admin::remove_downstream_server_handler),
        )
        .route(
            "/admin/servers",
            get(handlers::admin::list_downstream_servers_handler),
        );

    // Add memory endpoints only if memory is enabled
    if state.memory.is_some() {
        dual_info!("Memory endpoints are enabled");
        main_router = main_router
            .route(
                "/v1/memory/conversations/{conv_id}/history",
                get(handlers::get_conversation_history_handler),
            )
            .route(
                "/v1/memory/users/{user_id}/history",
                get(handlers::get_user_history_handler),
            )
            .route(
                "/v1/memory/users/{user_id}/conversations",
                get(handlers::list_user_conversations_handler),
            );
    } else {
        dual_info!("Memory endpoints are disabled");
    }

    // Add state to main router
    let main_router = main_router.with_state(state.clone());

    // Create responses router
    let responses_router = Router::new()
        .route("/v1/responses", post(responses::responses_handler))
        .route("/health", get(responses::health_handler))
        .with_state(responses_state);

    let app =
        Router::new()
            .merge(main_router)
            .merge(responses_router)
            .layer(cors)
            .layer(TraceLayer::new_for_http())
            .layer(axum::middleware::from_fn(
                |mut req: Request<Body>, next: axum::middleware::Next| async move {
                    // Generate request ID
                    let request_id = Uuid::new_v4().to_string();

                    // Add request ID to headers
                    req.headers_mut()
                        .insert("x-request-id", HeaderValue::from_str(&request_id).unwrap());

                    // Add cancellation token
                    let cancel_token = CancellationToken::new();
                    req.extensions_mut().insert(cancel_token);

                    // Log request start
                    dual_info!("Request started - ID: {}", request_id);

                    let response = next.run(req).await;

                    // Log request completion
                    dual_info!("Request completed - ID: {}", request_id);

                    response
                },
            ))
            .fallback_service(ServeDir::new(&cli.web_ui).not_found_service(
                ServeDir::new(&cli.web_ui).append_index_html_on_directories(true),
            ));

    // Create the listener
    let listener = tokio::net::TcpListener::bind(&addr).await.map_err(|e| {
        let err_msg = format!("Failed to bind to address: {e}");

        dual_error!("{err_msg}");

        ServerError::Operation(err_msg)
    })?;
    dual_info!("Listening on {}", addr);

    // Set up graceful shutdown
    let server =
        axum::serve(listener, app.into_make_service()).with_graceful_shutdown(shutdown_signal());

    // Start the server
    match server.await {
        Ok(_) => {
            dual_info!("Server shutdown completed");
            Ok(())
        }
        Err(e) => {
            let err_msg = format!("Server failed: {e}");
            dual_error!("{err_msg}");
            Err(ServerError::Operation(err_msg))
        }
    }
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("Failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {
            dual_info!("Received Ctrl+C, starting graceful shutdown");
        },
        _ = terminate => {
            dual_info!("Received SIGTERM, starting graceful shutdown");
        },
    }
}

/// Initialize logging based on the specified destination
fn init_logging(destination: &str, file_path: Option<&str>) -> ServerResult<()> {
    // Store the log destination for later use
    utils::LOG_DESTINATION
        .set(destination.to_string())
        .map_err(|_| {
            let err_msg = "Failed to set log destination".to_string();
            eprintln!("{err_msg}");
            ServerError::Operation(err_msg)
        })?;

    let log_level = get_log_level_from_env();

    match destination {
        "stdout" => {
            // Terminal output preserves colors
            tracing_subscriber::fmt()
                .with_target(false)
                .with_level(true)
                .with_file(true)
                .with_line_number(true)
                .with_thread_ids(true)
                .with_max_level(log_level)
                .init();
            Ok(())
        }
        "file" => {
            if let Some(path) = file_path {
                let file = std::fs::File::create(path).map_err(|e| {
                    let err_msg = format!("Failed to create log file: {e}");
                    eprintln!("{err_msg}");
                    ServerError::Operation(err_msg)
                })?;

                // File output disables ANSI colors
                tracing_subscriber::fmt()
                    .with_target(false)
                    .with_level(true)
                    .with_file(true)
                    .with_line_number(true)
                    .with_thread_ids(true)
                    .with_max_level(log_level)
                    .with_writer(file)
                    .with_ansi(false) // Disable ANSI colors
                    .init();
                Ok(())
            } else {
                Err(ServerError::Operation("Missing log file path".to_string()))
            }
        }
        "both" => {
            if let Some(path) = file_path {
                // Create directory if it doesn't exist
                if let Some(parent) = std::path::Path::new(path).parent()
                    && !parent.exists()
                {
                    std::fs::create_dir_all(parent).map_err(|e| {
                        let err_msg = format!("Failed to create directory for log file: {e}");
                        eprintln!("{err_msg}");
                        ServerError::Operation(err_msg)
                    })?;
                }

                // Create file appender and disable colors
                let file_appender = tracing_appender::rolling::never(
                    std::path::Path::new(path)
                        .parent()
                        .unwrap_or_else(|| std::path::Path::new(".")),
                    std::path::Path::new(path).file_name().unwrap_or_default(),
                );
                let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

                // Configure subscriber, disable ANSI colors
                tracing_subscriber::fmt()
                    .with_target(false)
                    .with_level(true)
                    .with_file(true)
                    .with_line_number(true)
                    .with_thread_ids(true)
                    .with_max_level(log_level)
                    .with_writer(non_blocking)
                    .with_ansi(false) // Disable ANSI colors
                    .init();

                println!("Logging to both stdout and file: {path}");

                Ok(())
            } else {
                Err(ServerError::Operation("Missing log file path".to_string()))
            }
        }
        _ => {
            let err_msg = format!(
                "Invalid log destination: {destination}. Valid values are 'stdout', 'file', or 'both'",
            );
            eprintln!("{err_msg}");
            Err(ServerError::Operation(err_msg))
        }
    }
}

fn get_log_level_from_env() -> Level {
    match std::env::var("LLAMA_LOG").ok().as_deref() {
        Some("trace") => Level::TRACE,
        Some("debug") => Level::DEBUG,
        Some("info") => Level::INFO,
        Some("warn") => Level::WARN,
        Some("error") => Level::ERROR,
        _ => Level::INFO,
    }
}

/// Application state
pub(crate) struct AppState {
    server_group: Arc<RwLock<HashMap<ServerKind, ServerGroup>>>,
    config: Arc<RwLock<Config>>,
    server_info: Arc<RwLock<ServerInfo>>,
    models: Arc<RwLock<HashMap<ServerId, Vec<endpoints::models::Model>>>>,
    memory: Option<Arc<crate::memory::CompleteChatMemory>>,
}
impl AppState {
    pub(crate) fn new(config: Config, server_info: ServerInfo) -> Self {
        Self {
            server_group: Arc::new(RwLock::new(HashMap::new())),
            config: Arc::new(RwLock::new(config)),
            server_info: Arc::new(RwLock::new(server_info)),
            models: Arc::new(RwLock::new(HashMap::new())),
            memory: None,
        }
    }

    pub(crate) fn with_memory(mut self, memory: Arc<crate::memory::CompleteChatMemory>) -> Self {
        self.memory = Some(memory);
        self
    }

    pub(crate) async fn register_downstream_server(&self, server: Server) -> ServerResult<()> {
        if server.kind.contains(ServerKind::chat) {
            self.server_group
                .write()
                .await
                .entry(ServerKind::chat)
                .or_insert(ServerGroup::new(ServerKind::chat))
                .register(server.clone())
                .await?;
        }
        if server.kind.contains(ServerKind::embeddings) {
            self.server_group
                .write()
                .await
                .entry(ServerKind::embeddings)
                .or_insert(ServerGroup::new(ServerKind::embeddings))
                .register(server.clone())
                .await?;
        }
        if server.kind.contains(ServerKind::image) {
            self.server_group
                .write()
                .await
                .entry(ServerKind::image)
                .or_insert(ServerGroup::new(ServerKind::image))
                .register(server.clone())
                .await?;
        }
        if server.kind.contains(ServerKind::tts) {
            self.server_group
                .write()
                .await
                .entry(ServerKind::tts)
                .or_insert(ServerGroup::new(ServerKind::tts))
                .register(server.clone())
                .await?;
        }
        if server.kind.contains(ServerKind::translate) {
            self.server_group
                .write()
                .await
                .entry(ServerKind::translate)
                .or_insert(ServerGroup::new(ServerKind::translate))
                .register(server.clone())
                .await?;
        }
        if server.kind.contains(ServerKind::transcribe) {
            self.server_group
                .write()
                .await
                .entry(ServerKind::transcribe)
                .or_insert(ServerGroup::new(ServerKind::transcribe))
                .register(server.clone())
                .await?;
        }

        Ok(())
    }

    pub(crate) async fn unregister_downstream_server(
        &self,
        server_id: impl AsRef<str>,
    ) -> ServerResult<()> {
        let mut found = false;

        // unregister the server from the servers
        {
            // parse server kind from server id
            let kinds = server_id
                .as_ref()
                .split("-server-")
                .next()
                .unwrap()
                .split("-")
                .collect::<Vec<&str>>();

            let group_map = self.server_group.read().await;

            for kind in kinds {
                let kind = ServerKind::from_str(kind).unwrap();
                if let Some(group) = group_map.get(&kind) {
                    group.unregister(server_id.as_ref()).await?;
                    dual_info!("Unregistered {} server: {}", &kind, server_id.as_ref());

                    if !found {
                        found = true;
                    }
                }
            }
        }

        if found {
            // remove the server info from the server_info
            let mut server_info = self.server_info.write().await;
            server_info.servers.remove(server_id.as_ref());

            // remove the server from the models
            let mut models = self.models.write().await;
            models.remove(server_id.as_ref());
        }

        if !found {
            return Err(ServerError::Operation(format!(
                "Server {} not found",
                server_id.as_ref()
            )));
        }

        Ok(())
    }

    pub(crate) async fn list_downstream_servers(
        &self,
    ) -> ServerResult<HashMap<ServerKind, Vec<Server>>> {
        let servers = self.server_group.read().await;

        let mut server_groups = HashMap::new();
        for (kind, group) in servers.iter() {
            if !group.is_empty().await {
                let servers = group.servers.read().await;

                // Create a new Vec with cloned Server instances using async stream
                let server_vec = stream::iter(servers.iter())
                    .then(|server_lock| async move {
                        let server = server_lock.read().await;
                        server.clone()
                    })
                    .collect::<Vec<_>>()
                    .await;

                server_groups.insert(*kind, server_vec);
            }
        }

        Ok(server_groups)
    }

    pub(crate) async fn check_server_health(&self) -> ServerResult<()> {
        if !self.server_group.read().await.is_empty() {
            let mut unhealthy_servers = Vec::new();

            // Check health status of downstream servers
            // 1. Get all registered downstream servers
            // 2. Check health status of downstream servers
            //   2.1 If a downstream server has multiple types, only perform one health check
            //   2.2 If there are multiple downstream servers of the same type, health checks are needed for all
            //   2.3 If two or more downstream servers have different types but the same URL, only perform one health check
            // 3. Remove unhealthy downstream servers
            {
                let group_map = self.server_group.read().await;

                // check health of unique servers
                let mut unique_server_ids = HashSet::new();
                for (kind, group) in group_map.iter() {
                    if !group.is_empty().await {
                        let servers = group.servers.read().await;
                        for server_lock in servers.iter() {
                            let mut server = server_lock.write().await;

                            if !unique_server_ids.contains(&server.id)
                                && unique_server_ids.contains(&server.url)
                            {
                                dual_info!("Checking health of {}", &server.id);

                                unique_server_ids.insert(server.id.clone());
                                unique_server_ids.insert(server.url.clone());

                                let is_healthy = server.check_health().await;
                                if !is_healthy {
                                    dual_warn!("{} server {} is unhealthy", kind, &server.id);
                                    unhealthy_servers.push(server.id.clone());
                                }
                            }
                        }
                    }
                }
            }

            // Unregister unhealthy servers
            if !unhealthy_servers.is_empty() {
                for server_id in unhealthy_servers {
                    self.unregister_downstream_server(&server_id).await?;
                }
            }

            // Push the healthy servers to the external service if configured
            if let Some(push_url) = &self.config.read().await.server_health_push_url {
                // collect the healthy servers by kind
                let mut healthy_servers: HashMap<ServerKind, Vec<String>> = HashMap::new();
                {
                    let group_map = self.server_group.read().await;
                    for (kind, group) in group_map.iter() {
                        if group.is_empty().await {
                            dual_warn!("No {} servers available after health check", kind);
                        }

                        healthy_servers.insert(
                            *kind,
                            group.healthy_servers.read().await.iter().cloned().collect(),
                        );
                    }
                }

                let health_status = serde_json::json!({
                    "rag": self.config.read().await.rag.as_ref().unwrap().enable,
                    "servers": healthy_servers,
                });

                dual_debug!(
                    "Healthy servers:\n{}",
                    serde_json::to_string_pretty(&health_status).unwrap()
                );

                // Send the healthy servers to the external service
                reqwest::Client::new()
                    .post(push_url)
                    .json(&health_status)
                    .send()
                    .await
                    .map_err(|e| {
                        let err_msg = format!("Failed to send health check result: {e}");

                        dual_error!("{}", err_msg);

                        ServerError::Operation(err_msg)
                    })?;
            }
        } else {
            dual_warn!("No servers registered, skipping health check");
        }

        Ok(())
    }

    pub(crate) async fn start_health_check_task(self: Arc<Self>) {
        let check_interval = HEALTH_CHECK_INTERVAL.get().unwrap_or(&60);
        let check_interval = tokio::time::Duration::from_secs(*check_interval);

        tokio::spawn(async move {
            loop {
                dual_debug!("Starting health check");

                if let Err(e) = self.check_server_health().await {
                    dual_error!("Health check error: {}", e);
                }

                tokio::time::sleep(check_interval).await;
            }
        });
    }

    pub(crate) async fn register_config_servers(self: &Arc<Self>) -> ServerResult<()> {
        let config = self.config.read().await;

        // Register chat service from configuration file
        if let Some(chat_config) = &config.chat {
            dual_info!("Registering chat service from config: {}", chat_config.url);
            let server = Server::from_chat_config(chat_config)?;

            // Update model list for the server
            let headers = axum::http::HeaderMap::new();
            if let Err(e) = crate::handlers::update_model_list(
                axum::extract::State(Arc::clone(self)),
                &headers,
                "config-registration",
                &server,
            )
            .await
            {
                dual_warn!(
                    "Failed to update model list for chat server {}: {}",
                    server.id,
                    e
                );
                // Continue with registration even if model list update fails
            }

            self.register_downstream_server(server).await?;
            dual_info!("Chat service registered successfully");
        }

        // Register embedding service from configuration file
        if let Some(embedding_config) = &config.embedding {
            dual_info!(
                "Registering embedding service from config: {}",
                embedding_config.url
            );
            let server = Server::from_embedding_config(embedding_config)?;

            // Update model list for the server
            let headers = axum::http::HeaderMap::new();
            if let Err(e) = crate::handlers::update_model_list(
                axum::extract::State(Arc::clone(self)),
                &headers,
                "config-registration",
                &server,
            )
            .await
            {
                dual_warn!(
                    "Failed to update model list for embedding server {}: {}",
                    server.id,
                    e
                );
                // Continue with registration even if model list update fails
            }

            self.register_downstream_server(server).await?;
            dual_info!("Embedding service registered successfully");
        }

        Ok(())
    }
}
