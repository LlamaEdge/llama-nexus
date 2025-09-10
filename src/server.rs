use std::{
    collections::HashSet,
    sync::atomic::{AtomicUsize, Ordering},
    time::{Duration, SystemTime},
};

use async_trait::async_trait;
use bitflags::bitflags;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::{
    HEALTH_CHECK_INTERVAL, dual_error, dual_warn,
    error::{ServerError, ServerResult},
};

/// Timeout duration for health checks (in seconds)
const TIMEOUT: u64 = 10;

pub(crate) type ServerId = String;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ServerIdToRemove {
    pub server_id: ServerId,
}

/// Represents the health status of a server
#[derive(Debug, Clone)]
pub struct HealthStatus {
    pub is_healthy: bool,
    pub last_check: SystemTime,
}

impl Default for HealthStatus {
    fn default() -> Self {
        Self {
            is_healthy: true,
            last_check: SystemTime::now(),
        }
    }
}

/// Represents a LlamaEdge API server
#[derive(Debug, Serialize)]
pub struct Server {
    pub id: ServerId,
    pub url: String,
    pub kind: ServerKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(skip)]
    connections: AtomicUsize,
    #[serde(skip)]
    pub health_status: HealthStatus,
}
impl<'de> Deserialize<'de> for Server {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        // Create a helper struct to deserialize into
        #[derive(Deserialize)]
        struct ServerHelper {
            url: String,
            kind: ServerKind,
            api_key: Option<String>,
        }

        // Deserialize into the helper struct
        let helper = ServerHelper::deserialize(deserializer)?;

        let kind = helper.kind.to_string().trim().replace(',', "-");
        let id = format!("{}-server-{}", kind, uuid::Uuid::new_v4());

        // Create the actual Server instance
        Ok(Server {
            id,
            url: helper.url,
            kind: helper.kind,
            api_key: helper.api_key,
            connections: AtomicUsize::new(0),
            health_status: HealthStatus::default(),
        })
    }
}
impl Clone for Server {
    fn clone(&self) -> Self {
        Self {
            id: self.id.clone(),
            url: self.url.clone(),
            kind: self.kind,
            api_key: self.api_key.clone(),
            connections: AtomicUsize::new(self.connections.load(Ordering::Relaxed)),
            health_status: self.health_status.clone(),
        }
    }
}
impl Server {
    pub(crate) async fn check_health(&mut self) -> bool {
        // If the server is currently healthy, check if a new health check is needed
        if self.health_status.is_healthy {
            let now = SystemTime::now();
            if let Ok(duration) = now.duration_since(self.health_status.last_check) {
                let check_interval =
                    Duration::from_secs(*HEALTH_CHECK_INTERVAL.get().unwrap_or(&60));
                if duration < check_interval {
                    // If the time since last check is less than the interval, return current status
                    return true;
                }
            }
        }

        // Perform new health check
        let client = reqwest::Client::new();
        let health_url = format!("{}/info", self.url);

        // Use configured timeout duration
        let timeout = Duration::from_secs(TIMEOUT);
        let is_healthy = match client.get(&health_url).timeout(timeout).send().await {
            Ok(response) => {
                // Consider server healthy if response is timeout (408)
                if response.status() == reqwest::StatusCode::REQUEST_TIMEOUT {
                    dual_warn!("Health check: {} server {} is in use", self.kind, self.id);
                    true
                } else {
                    response.status().is_success()
                }
            }
            Err(e) => {
                // Consider server healthy if error is timeout
                dual_warn!("Health check: {} server {} is in use", self.kind, self.id);
                e.is_timeout()
            }
        };

        self.health_status = HealthStatus {
            is_healthy,
            last_check: SystemTime::now(),
        };

        is_healthy
    }

    pub(crate) fn from_chat_config(chat_config: &crate::config::ChatConfig) -> ServerResult<Self> {
        // Validate URL format
        if chat_config.url.is_empty() {
            return Err(ServerError::Operation(
                "Chat service URL cannot be empty".to_string(),
            ));
        }

        // Check if API key is available
        let api_key = chat_config.get_api_key();

        let id = format!("config-chat-{}", Uuid::new_v4());

        Ok(Server {
            id,
            url: chat_config.url.clone(),
            kind: ServerKind::chat,
            api_key,
            connections: AtomicUsize::new(0),
            health_status: HealthStatus::default(),
        })
    }

    pub(crate) fn from_embedding_config(
        embedding_config: &crate::config::EmbeddingConfig,
    ) -> ServerResult<Self> {
        // Validate URL format
        if embedding_config.url.is_empty() {
            return Err(ServerError::Operation(
                "Embedding service URL cannot be empty".to_string(),
            ));
        }

        // Check if API key is available
        let api_key = embedding_config.get_api_key();

        let id = format!("config-embedding-{}", Uuid::new_v4());

        Ok(Server {
            id,
            url: embedding_config.url.clone(),
            kind: ServerKind::embeddings,
            api_key,
            connections: AtomicUsize::new(0),
            health_status: HealthStatus::default(),
        })
    }
}

#[test]
fn test_deserialize_server() {
    let serialized = r#"{"url": "http://localhost:8000", "kind": "chat,tts"}"#;
    let server: Server = serde_json::from_str(serialized).unwrap();
    println!("id: {}", server.id);
    assert_eq!(server.url, "http://localhost:8000");
    assert_eq!(server.kind, ServerKind::chat | ServerKind::tts);

    let serialized = r#"{"url": "http://localhost:8000", "kind": "chat"}"#;
    let server: Server = serde_json::from_str(serialized).unwrap();
    println!("id: {}", server.id);
    assert_eq!(server.url, "http://localhost:8000");
    assert_eq!(server.kind, ServerKind::chat);
}

#[test]
fn test_serialize_server() {
    let id = "chat-tts-29b6c973-d45a-4487-a3da-2e9b1f704fd9".to_string();
    let server = Server {
        id,
        url: "http://localhost:8000".to_string(),
        kind: ServerKind::chat | ServerKind::tts,
        api_key: None,
        connections: AtomicUsize::new(0),
        health_status: HealthStatus::default(),
    };
    let serialized = serde_json::to_string(&server).unwrap();
    assert_eq!(
        serialized,
        r#"{"id":"chat-tts-29b6c973-d45a-4487-a3da-2e9b1f704fd9","url":"http://localhost:8000","kind":"chat,tts"}"#
    );

    let id = "chat-2424f42e-fcfb-458e-9a6a-ad419e24b5f5".to_string();
    let server: Server = Server {
        id,
        url: "http://localhost:8000".to_string(),
        kind: ServerKind::chat,
        api_key: Some("test-api-key".to_string()),
        connections: AtomicUsize::new(0),
        health_status: HealthStatus::default(),
    };
    let serialized = serde_json::to_string(&server).unwrap();
    assert_eq!(
        serialized,
        r#"{"id":"chat-2424f42e-fcfb-458e-9a6a-ad419e24b5f5","url":"http://localhost:8000","kind":"chat","api_key":"test-api-key"}"#
    );
}

bitflags! {
    /// Represents the kind of server
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct ServerKind: u8{
        const chat = 1;
        const embeddings = 1 << 1;
        const image = 1 << 2;
        const tts = 1 << 3;
        const translate = 1 << 4;
        const transcribe = 1 << 5;
    }
}
impl std::fmt::Display for ServerKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut kind_str = String::new();
        if self.contains(ServerKind::chat) {
            kind_str.push_str("chat,");
        }
        if self.contains(ServerKind::embeddings) {
            kind_str.push_str("embeddings,");
        }
        if self.contains(ServerKind::image) {
            kind_str.push_str("image,");
        }
        if self.contains(ServerKind::tts) {
            kind_str.push_str("tts,");
        }
        if self.contains(ServerKind::translate) {
            kind_str.push_str("translate,");
        }
        if self.contains(ServerKind::transcribe) {
            kind_str.push_str("transcribe,");
        }

        if !kind_str.is_empty() {
            kind_str = kind_str.trim_end_matches(',').to_string();
        }

        write!(f, "{kind_str}")
    }
}
impl std::str::FromStr for ServerKind {
    type Err = ServerError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let ss = s.to_lowercase();
        let values = ss.split(',').collect::<Vec<&str>>();
        let mut kind = Self::empty();
        for val in values {
            match val.trim() {
                "chat" => kind.set(Self::chat, true),
                "embeddings" => kind.set(Self::embeddings, true),
                "image" => kind.set(Self::image, true),
                "tts" => kind.set(Self::tts, true),
                "translate" => kind.set(Self::translate, true),
                "transcribe" => kind.set(Self::transcribe, true),
                _ => return Err(ServerError::InvalidServerKind(s.to_string())),
            }
        }
        Ok(kind)
    }
}
impl Serialize for ServerKind {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        // Convert the flags to a string representation
        let mut kind_str = String::new();
        if self.contains(ServerKind::chat) {
            kind_str.push_str("chat,");
        }
        if self.contains(ServerKind::embeddings) {
            kind_str.push_str("embeddings,");
        }
        if self.contains(ServerKind::image) {
            kind_str.push_str("image,");
        }
        if self.contains(ServerKind::tts) {
            kind_str.push_str("tts,");
        }
        if self.contains(ServerKind::translate) {
            kind_str.push_str("translate,");
        }
        if self.contains(ServerKind::transcribe) {
            kind_str.push_str("transcribe,");
        }

        // Remove trailing comma if present
        if !kind_str.is_empty() {
            kind_str.pop();
        }

        // Serialize as a string
        serializer.serialize_str(&kind_str)
    }
}
impl<'de> Deserialize<'de> for ServerKind {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        // First deserialize into a String
        let s = String::deserialize(deserializer)?;

        // Parse the string using from_str
        s.parse::<ServerKind>()
            .map_err(|e| serde::de::Error::custom(format!("Failed to parse ServerKindNew: {e}")))
    }
}
impl std::hash::Hash for ServerKind {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.bits().hash(state);
    }
}

#[test]
fn test_serialize_server_kind() {
    let kind = ServerKind::chat | ServerKind::tts;
    let serialized = serde_json::to_string(&kind).unwrap();
    assert_eq!(serialized, "\"chat,tts\"");

    let kind = ServerKind::chat;
    let serialized = serde_json::to_string(&kind).unwrap();
    assert_eq!(serialized, "\"chat\"");

    // let kind = ServerKind::vdb;
    // let serialized = serde_json::to_string(&kind).unwrap();
    // assert_eq!(serialized, "\"vdb\"");
}

#[test]
fn test_deserialize_server_kind() {
    let serialized = "\"chat,tts\"";
    let kind: ServerKind = serde_json::from_str(serialized).unwrap();
    assert_eq!(kind, ServerKind::chat | ServerKind::tts);

    let serialized = "\"chat\"";
    let kind: ServerKind = serde_json::from_str(serialized).unwrap();
    assert_eq!(kind, ServerKind::chat);

    // let serialized = "\"vdb\"";
    // let kind: ServerKind = serde_json::from_str(serialized).unwrap();
    // assert_eq!(kind, ServerKind::vdb);
}

#[derive(Debug)]
pub(crate) struct ServerGroup {
    pub(crate) servers: RwLock<Vec<RwLock<Server>>>,
    pub(crate) healthy_servers: RwLock<HashSet<ServerId>>,
    ty: ServerKind,
}
impl ServerGroup {
    pub(crate) fn new(ty: ServerKind) -> Self {
        Self {
            servers: RwLock::new(Vec::new()),
            healthy_servers: RwLock::new(HashSet::new()),
            ty,
        }
    }

    pub(crate) async fn register(&self, server: Server) -> ServerResult<()> {
        // check if the server is already registered
        if self.healthy_servers.read().await.contains(&server.id) {
            let err_msg = format!("Server already registered: {}", server.url);
            dual_warn!("{}", &err_msg);
            return Err(ServerError::Operation(err_msg));
        }

        self.healthy_servers.write().await.insert(server.id.clone());
        self.servers.write().await.push(RwLock::new(server));

        Ok(())
    }

    pub(crate) async fn unregister(&self, server_id: impl AsRef<str>) -> ServerResult<()> {
        let id_to_remove = server_id.as_ref();

        // locate the server to remove
        let idx_to_remove = {
            let servers = self.servers.read().await;

            // Find the index of the server to remove
            let mut idx_to_remove = None;
            for (idx, server_lock) in servers.iter().enumerate() {
                let server = server_lock.read().await;
                if server.id == id_to_remove {
                    idx_to_remove = Some(idx);
                    break;
                }
            }

            idx_to_remove
        };

        // Remove the server from server list if found
        if let Some(idx) = idx_to_remove {
            let mut servers = self.servers.write().await;
            servers.swap_remove(idx);
        }

        // Remove the server from the healthy server set if found
        if !self.healthy_servers.write().await.remove(id_to_remove) {
            let err_msg = format!("Server not found: {id_to_remove}");
            dual_warn!("{err_msg}");
            return Err(ServerError::Operation(err_msg));
        }

        Ok(())
    }

    #[allow(dead_code)]
    pub(crate) async fn ty(&self) -> ServerKind {
        self.ty
    }

    pub(crate) async fn is_empty(&self) -> bool {
        self.healthy_servers.read().await.is_empty()
    }
}
#[async_trait]
impl RoutingPolicy for ServerGroup {
    async fn next(&self) -> Result<TargetServerInfo, ServerError> {
        let servers = self.servers.read().await;
        if servers.is_empty() {
            let err_msg = format!("No {} server found", self.ty);
            dual_error!("{}", &err_msg);
            return Err(ServerError::NotFoundServer(self.ty.to_string()));
        }

        let server_lock = if servers.len() == 1 {
            servers.first().unwrap()
        } else {
            // Find server with minimum connections - need to read each server
            let mut min_connections = usize::MAX;
            let mut min_server = &servers[0];

            for server in servers.iter() {
                let guard = server.read().await;
                let connections = guard.connections.load(Ordering::Relaxed);
                if connections < min_connections {
                    min_connections = connections;
                    min_server = server;
                }
            }
            min_server
        };

        // Access the chosen server
        let target_server_info = {
            let server = server_lock.write().await;
            server.connections.fetch_add(1, Ordering::Relaxed);
            TargetServerInfo {
                id: server.id.clone(),
                url: server.url.clone(),
                api_key: server.api_key.clone(),
            }
        };

        Ok(target_server_info)
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct TargetServerInfo {
    pub id: ServerId,
    pub url: String,
    pub api_key: Option<String>,
}

#[async_trait]
pub(crate) trait RoutingPolicy: Sync + Send {
    async fn next(&self) -> Result<TargetServerInfo, ServerError>;
}
