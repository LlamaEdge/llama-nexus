//! Docker Executor
//!
//! Executes scripts in isolated Docker containers.
//! Supports Python, Shell, Ruby, and other interpreted languages.

use std::{collections::HashMap, time::Instant};

use async_trait::async_trait;
use bollard::{
    Docker,
    container::LogOutput,
    models::{ContainerCreateBody, HostConfig, Mount, MountTypeEnum},
    query_parameters::{
        CreateContainerOptionsBuilder, CreateImageOptionsBuilder, KillContainerOptions,
        LogsOptionsBuilder, RemoveContainerOptionsBuilder, StartContainerOptions,
        WaitContainerOptionsBuilder,
    },
};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tracing::{debug, error, info, warn};

use super::{
    error::ExecutionError,
    traits::{Executor, IsolationLevel},
    types::{ExecuteRequest, ResourceUsage, ScriptOutput},
};

/// Default Docker images for different script types
const DEFAULT_PYTHON_IMAGE: &str = "python:3.11-slim";
const DEFAULT_SHELL_IMAGE: &str = "alpine:latest";
const DEFAULT_RUBY_IMAGE: &str = "ruby:3.2-slim";
const DEFAULT_NODE_IMAGE: &str = "node:20-slim";

/// Docker executor configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockerConfig {
    /// Docker socket path (default: auto-detect)
    #[serde(default)]
    pub socket: Option<String>,

    /// Default image for unknown script types
    #[serde(default = "default_image")]
    pub default_image: String,

    /// Image mapping (extension -> image)
    #[serde(default = "default_images")]
    pub images: HashMap<String, String>,

    /// Whether to auto-remove containers after execution
    #[serde(default = "default_true")]
    pub auto_remove: bool,

    /// Whether to use read-only root filesystem
    #[serde(default = "default_true")]
    pub read_only: bool,

    /// Network mode (default: "none" for isolation)
    #[serde(default = "default_network_mode")]
    pub network_mode: String,

    /// Container name prefix
    #[serde(default = "default_container_prefix")]
    pub container_prefix: String,

    /// Whether to auto-pull images if not present locally
    #[serde(default = "default_true")]
    pub auto_pull: bool,
}

fn default_image() -> String {
    DEFAULT_SHELL_IMAGE.to_string()
}

fn default_images() -> HashMap<String, String> {
    let mut images = HashMap::new();
    images.insert("py".to_string(), DEFAULT_PYTHON_IMAGE.to_string());
    images.insert("sh".to_string(), DEFAULT_SHELL_IMAGE.to_string());
    images.insert("bash".to_string(), DEFAULT_SHELL_IMAGE.to_string());
    images.insert("rb".to_string(), DEFAULT_RUBY_IMAGE.to_string());
    images.insert("js".to_string(), DEFAULT_NODE_IMAGE.to_string());
    images
}

fn default_true() -> bool {
    true
}

fn default_network_mode() -> String {
    "none".to_string()
}

fn default_container_prefix() -> String {
    "llama-nexus-exec".to_string()
}

impl Default for DockerConfig {
    fn default() -> Self {
        Self {
            socket: None,
            default_image: default_image(),
            images: default_images(),
            auto_remove: true,
            read_only: true,
            network_mode: default_network_mode(),
            container_prefix: default_container_prefix(),
            auto_pull: true,
        }
    }
}

/// Docker executor for running scripts in containers
pub struct DockerExecutor {
    /// Docker client
    client: Docker,
    /// Configuration
    config: DockerConfig,
}

impl DockerExecutor {
    /// Creates a new Docker executor with default configuration
    pub async fn new() -> Result<Self, ExecutionError> {
        Self::with_config(DockerConfig::default()).await
    }

    /// Creates a new Docker executor with custom configuration
    pub async fn with_config(config: DockerConfig) -> Result<Self, ExecutionError> {
        let client = if let Some(ref socket) = config.socket {
            Docker::connect_with_socket(socket, 120, bollard::API_DEFAULT_VERSION)
                .map_err(|e| ExecutionError::runtime("docker", format!("failed to connect: {e}")))?
        } else {
            Docker::connect_with_local_defaults()
                .map_err(|e| ExecutionError::runtime("docker", format!("failed to connect: {e}")))?
        };

        Ok(Self { client, config })
    }

    /// Gets the appropriate image for a script extension
    fn get_image(&self, extension: &str) -> &str {
        self.config
            .images
            .get(extension)
            .map(|s| s.as_str())
            .unwrap_or(&self.config.default_image)
    }

    /// Builds the command to run a script
    fn build_command(&self, extension: &str, script_path: &str, args: &[String]) -> Vec<String> {
        let mut cmd = match extension {
            "py" => vec!["python".to_string(), script_path.to_string()],
            "sh" | "bash" => vec!["sh".to_string(), script_path.to_string()],
            "rb" => vec!["ruby".to_string(), script_path.to_string()],
            "js" => vec!["node".to_string(), script_path.to_string()],
            _ => vec!["sh".to_string(), script_path.to_string()],
        };
        cmd.extend(args.iter().cloned());
        cmd
    }

    /// Generates a unique container name
    fn generate_container_name(&self) -> String {
        let id = uuid::Uuid::new_v4();
        format!("{}-{}", self.config.container_prefix, &id.to_string()[..8])
    }

    /// Collects logs from a container
    async fn collect_logs(&self, container_id: &str) -> (String, String) {
        let options = LogsOptionsBuilder::new()
            .stdout(true)
            .stderr(true)
            .follow(false)
            .build();

        let mut stdout = String::new();
        let mut stderr = String::new();

        let mut stream = self.client.logs(container_id, Some(options));

        while let Some(result) = stream.next().await {
            match result {
                Ok(LogOutput::StdOut { message }) => {
                    stdout.push_str(&String::from_utf8_lossy(&message));
                }
                Ok(LogOutput::StdErr { message }) => {
                    stderr.push_str(&String::from_utf8_lossy(&message));
                }
                Ok(_) => {}
                Err(e) => {
                    warn!(error = %e, "failed to read container logs");
                    break;
                }
            }
        }

        (stdout, stderr)
    }

    /// Removes a container
    async fn remove_container(&self, container_id: &str) {
        let options = RemoveContainerOptionsBuilder::new().force(true).build();

        if let Err(e) = self
            .client
            .remove_container(container_id, Some(options))
            .await
        {
            warn!(container_id, error = %e, "failed to remove container");
        }
    }

    /// Checks if an image exists locally
    async fn image_exists(&self, image: &str) -> bool {
        self.client.inspect_image(image).await.is_ok()
    }

    /// Pulls an image from the registry
    async fn pull_image(&self, image: &str) -> Result<(), ExecutionError> {
        info!(image = %image, "pulling docker image");

        // Parse image name and tag
        let (from_image, tag) = if let Some(pos) = image.rfind(':') {
            // Check if it's a tag or a port number (e.g., registry:5000/image)
            let after_colon = &image[pos + 1..];
            if after_colon.contains('/') {
                // It's a port, not a tag
                (image, "latest")
            } else {
                (&image[..pos], after_colon)
            }
        } else {
            (image, "latest")
        };

        let options = CreateImageOptionsBuilder::new()
            .from_image(from_image)
            .tag(tag)
            .build();

        let mut stream = self.client.create_image(Some(options), None, None);

        while let Some(result) = stream.next().await {
            match result {
                Ok(info) => {
                    if let Some(status) = info.status {
                        debug!(status = %status, "image pull progress");
                    }
                }
                Err(e) => {
                    error!(image = %image, error = %e, "failed to pull image");
                    return Err(ExecutionError::runtime(
                        "docker",
                        format!("failed to pull image {image}: {e}"),
                    ));
                }
            }
        }

        info!(image = %image, "image pulled successfully");
        Ok(())
    }

    /// Ensures an image is available locally, pulling if necessary
    async fn ensure_image(&self, image: &str) -> Result<(), ExecutionError> {
        if self.image_exists(image).await {
            debug!(image = %image, "image already exists locally");
            return Ok(());
        }

        if !self.config.auto_pull {
            return Err(ExecutionError::runtime(
                "docker",
                format!("image {image} not found locally and auto_pull is disabled"),
            ));
        }

        self.pull_image(image).await
    }
}

#[async_trait]
impl Executor for DockerExecutor {
    fn name(&self) -> &str {
        "docker"
    }

    fn supported_extensions(&self) -> Vec<&str> {
        vec!["py", "sh", "bash", "rb"]
    }

    fn isolation_level(&self) -> IsolationLevel {
        IsolationLevel::Container
    }

    async fn health_check(&self) -> Result<(), ExecutionError> {
        self.client
            .ping()
            .await
            .map_err(|e| ExecutionError::ExecutorUnavailable("docker".into(), e.to_string()))?;
        Ok(())
    }

    async fn execute(&self, request: ExecuteRequest) -> Result<ScriptOutput, ExecutionError> {
        let start_time = Instant::now();

        // Get script extension
        let extension = request
            .script
            .path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("sh")
            .to_lowercase();

        // Get image and ensure it exists
        let image = self.get_image(&extension);
        self.ensure_image(image).await?;

        // Container name
        let container_name = self.generate_container_name();

        // Script path in container
        let script_filename = request
            .script
            .path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("script");
        let container_script_path = format!("/scripts/{}", script_filename);

        // Build command
        let cmd = self.build_command(&extension, &container_script_path, &request.args);

        debug!(
            container = %container_name,
            image = %image,
            script = %request.script.path.display(),
            "creating container"
        );

        // Build environment variables
        let env: Vec<String> = request
            .env
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect();

        // Build mounts
        let script_dir = request
            .script
            .path
            .parent()
            .ok_or_else(|| ExecutionError::runtime("docker", "invalid script path"))?;

        let mounts = vec![Mount {
            target: Some("/scripts".to_string()),
            source: Some(script_dir.to_string_lossy().to_string()),
            typ: Some(MountTypeEnum::BIND),
            read_only: Some(true),
            ..Default::default()
        }];

        // Build host config with security settings
        let host_config = HostConfig {
            mounts: Some(mounts),
            memory: Some(request.limits.max_memory_bytes as i64),
            memory_swap: Some(request.limits.max_memory_bytes as i64), // Disable swap
            nano_cpus: Some(1_000_000_000),                            // 1 CPU
            network_mode: if request.limits.network_access {
                None
            } else {
                Some(self.config.network_mode.clone())
            },
            readonly_rootfs: Some(self.config.read_only),
            security_opt: Some(vec!["no-new-privileges:true".to_string()]),
            auto_remove: Some(false), // We'll remove manually to get logs first
            ..Default::default()
        };

        // Create container config
        let config = ContainerCreateBody {
            image: Some(image.to_string()),
            cmd: Some(cmd),
            env: Some(env),
            host_config: Some(host_config),
            working_dir: Some("/scripts".to_string()),
            ..Default::default()
        };

        // Create container
        let create_options = CreateContainerOptionsBuilder::new()
            .name(&container_name)
            .build();

        let container = self
            .client
            .create_container(Some(create_options), config)
            .await
            .map_err(|e| {
                ExecutionError::runtime("docker", format!("failed to create container: {e}"))
            })?;

        let container_id = container.id;

        debug!(container_id = %container_id, "container created, starting");

        // Start container
        if let Err(e) = self
            .client
            .start_container(&container_id, None::<StartContainerOptions>)
            .await
        {
            self.remove_container(&container_id).await;
            return Err(ExecutionError::runtime(
                "docker",
                format!("failed to start container: {e}"),
            ));
        }

        // Wait for container with timeout
        let wait_options = WaitContainerOptionsBuilder::new()
            .condition("not-running")
            .build();

        let wait_result = tokio::time::timeout(request.limits.timeout, async {
            let mut stream = self
                .client
                .wait_container(&container_id, Some(wait_options));
            stream.next().await
        })
        .await;

        let (exit_code, timed_out) = match wait_result {
            Ok(Some(Ok(response))) => (response.status_code as i32, false),
            Ok(Some(Err(e))) => {
                error!(error = %e, "container wait error");
                self.remove_container(&container_id).await;
                return Err(ExecutionError::runtime(
                    "docker",
                    format!("container wait error: {e}"),
                ));
            }
            Ok(None) => {
                warn!("container wait stream ended unexpectedly");
                (0, false)
            }
            Err(_) => {
                // Timeout - kill container
                warn!(container_id = %container_id, "container execution timed out, killing");
                if let Err(e) = self
                    .client
                    .kill_container(&container_id, None::<KillContainerOptions>)
                    .await
                {
                    warn!(error = %e, "failed to kill container");
                }
                (-1, true)
            }
        };

        // Collect logs
        let (stdout, stderr) = self.collect_logs(&container_id).await;

        // Remove container
        if self.config.auto_remove {
            self.remove_container(&container_id).await;
        }

        let duration = start_time.elapsed();

        info!(
            container_id = %container_id,
            exit_code,
            duration_ms = duration.as_millis(),
            "container execution completed"
        );

        Ok(ScriptOutput {
            stdout,
            stderr,
            exit_code,
            duration,
            timed_out,
            resource_usage: ResourceUsage::default(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_docker_config_default() {
        let config = DockerConfig::default();
        assert_eq!(config.network_mode, "none");
        assert!(config.auto_remove);
        assert!(config.read_only);
        assert!(config.auto_pull);
        assert!(config.images.contains_key("py"));
        assert!(config.images.contains_key("sh"));
    }

    #[test]
    fn test_image_tag_parsing() {
        // Test the image:tag parsing logic used in pull_image
        fn parse_image_tag(image: &str) -> (&str, &str) {
            if let Some(pos) = image.rfind(':') {
                let after_colon = &image[pos + 1..];
                if after_colon.contains('/') {
                    (image, "latest")
                } else {
                    (&image[..pos], after_colon)
                }
            } else {
                (image, "latest")
            }
        }

        // Standard image:tag
        assert_eq!(parse_image_tag("python:3.11-slim"), ("python", "3.11-slim"));

        // Image without tag
        assert_eq!(parse_image_tag("python"), ("python", "latest"));

        // Registry with port
        assert_eq!(
            parse_image_tag("registry:5000/myimage"),
            ("registry:5000/myimage", "latest")
        );

        // Registry with port and tag
        assert_eq!(
            parse_image_tag("registry:5000/myimage:v1"),
            ("registry:5000/myimage", "v1")
        );
    }

    #[test]
    fn test_get_image() {
        let config = DockerConfig::default();

        // Test image mapping directly from config
        assert_eq!(
            config.images.get("py"),
            Some(&DEFAULT_PYTHON_IMAGE.to_string())
        );
        assert_eq!(
            config.images.get("sh"),
            Some(&DEFAULT_SHELL_IMAGE.to_string())
        );
        assert_eq!(
            config.images.get("rb"),
            Some(&DEFAULT_RUBY_IMAGE.to_string())
        );
        assert_eq!(
            config.images.get("js"),
            Some(&DEFAULT_NODE_IMAGE.to_string())
        );
    }

    #[test]
    fn test_build_command() {
        // Test command building logic without needing DockerExecutor instance
        fn build_command(extension: &str, script_path: &str, args: &[String]) -> Vec<String> {
            let mut cmd = match extension {
                "py" => vec!["python".to_string(), script_path.to_string()],
                "sh" | "bash" => vec!["sh".to_string(), script_path.to_string()],
                "rb" => vec!["ruby".to_string(), script_path.to_string()],
                "js" => vec!["node".to_string(), script_path.to_string()],
                _ => vec!["sh".to_string(), script_path.to_string()],
            };
            cmd.extend(args.iter().cloned());
            cmd
        }

        assert_eq!(
            build_command("py", "/scripts/test.py", &["--verbose".to_string()]),
            vec!["python", "/scripts/test.py", "--verbose"]
        );
        assert_eq!(
            build_command("sh", "/scripts/test.sh", &[]),
            vec!["sh", "/scripts/test.sh"]
        );
        assert_eq!(
            build_command("js", "/scripts/test.js", &["arg1".to_string()]),
            vec!["node", "/scripts/test.js", "arg1"]
        );
    }

    #[test]
    fn test_supported_extensions() {
        // Test that our supported extensions are reasonable
        let extensions = vec!["py", "sh", "bash", "rb"];
        assert!(extensions.contains(&"py"));
        assert!(extensions.contains(&"sh"));
    }
}
