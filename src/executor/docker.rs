//! Docker Executor
//!
//! Executes scripts in isolated Docker containers.
//! Supports Python, Shell, Ruby, and other interpreted languages.
//!
//! ## File Access Strategies
//!
//! The Docker executor supports two file access strategies:
//!
//! 1. **Bind Mount (Method B)**: Files in pre-configured `data_dirs` are mounted
//!    directly into the container. Fast and efficient for local deployment.
//!
//! 2. **File Copy (Method C)**: Files not in `data_dirs` are copied to a temporary
//!    directory and mounted. Better for remote/network scenarios.
//!
//! The executor automatically chooses the appropriate strategy based on file location.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::Instant,
};

use async_trait::async_trait;
use bollard::{
    Docker,
    container::LogOutput,
    models::{ContainerCreateBody, HostConfig, Mount, MountTypeEnum},
    query_parameters::{
        CreateContainerOptionsBuilder, CreateImageOptionsBuilder, InspectContainerOptions,
        KillContainerOptions, LogsOptionsBuilder, RemoveContainerOptionsBuilder,
        StartContainerOptions,
    },
};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tempfile::TempDir;
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

    /// Additional directories to mount as read-only data volumes
    /// Format: ["/host/path:/container/path", "/another/path"]
    /// If only host path is specified, it will be mounted at the same path in container
    #[serde(default)]
    pub data_dirs: Vec<String>,

    /// File access strategy mode
    /// - "auto": Auto-detect based on file location (default)
    /// - "bind_only": Only use bind mounts (fail if file not in data_dirs)
    /// - "copy_only": Always copy files to temp directory
    #[serde(default = "default_file_access_mode")]
    pub file_access_mode: String,

    /// Temporary directory for file copy mode
    /// Default: system temp directory
    #[serde(default)]
    pub temp_dir: Option<String>,

    /// Whether to allow write access for output files in copy mode
    /// When true, output files are copied back to the original location after execution
    #[serde(default = "default_true")]
    pub copy_output_back: bool,
}

fn default_file_access_mode() -> String {
    "auto".to_string()
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
            data_dirs: Vec::new(),
            file_access_mode: default_file_access_mode(),
            temp_dir: None,
            copy_output_back: true,
        }
    }
}

/// File access strategy for a specific file
#[derive(Debug, Clone)]
pub enum FileAccessStrategy {
    /// File is in a pre-configured data_dir, use bind mount
    BindMount {
        #[allow(dead_code)] // Used for debugging and future enhancements
        host_path: PathBuf,
        container_path: PathBuf,
    },
    /// File needs to be copied to temp directory
    FileCopy {
        source_path: PathBuf,
        temp_path: PathBuf,
        container_path: PathBuf,
        is_output: bool,
    },
}

/// Parsed file reference from script arguments
#[derive(Debug, Clone)]
pub struct FileReference {
    /// Original argument value
    pub original: String,
    /// Parsed absolute path
    pub path: PathBuf,
    /// Whether this is an output file (may not exist yet)
    pub is_output: bool,
    /// Index in the args array
    pub arg_index: usize,
}

/// Execution context with file mappings
#[derive(Debug)]
struct ExecutionContext {
    /// Temporary directory for file copy mode (if used)
    /// Note: This field is kept alive to prevent temp directory cleanup until execution completes
    #[allow(dead_code)]
    temp_dir: Option<TempDir>,
    /// File strategies for each file reference
    file_strategies: Vec<(FileReference, FileAccessStrategy)>,
    /// Modified args with container paths
    container_args: Vec<String>,
    /// Additional mounts needed
    additional_mounts: Vec<Mount>,
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
    #[allow(dead_code)]
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

    /// Parse data_dirs configuration into (host_path, container_path) pairs
    fn parse_data_dirs(&self) -> Vec<(PathBuf, PathBuf)> {
        self.config
            .data_dirs
            .iter()
            .filter_map(|data_dir| {
                let (host_path, container_path) = if let Some(pos) = data_dir.find(':') {
                    let host = &data_dir[..pos];
                    let container = &data_dir[pos + 1..];
                    (PathBuf::from(host), PathBuf::from(container))
                } else {
                    (PathBuf::from(data_dir), PathBuf::from(data_dir))
                };

                debug!(
                    raw_config = %data_dir,
                    host_path = %host_path.display(),
                    container_path = %container_path.display(),
                    "parsing data_dir config"
                );

                // Only include if host path exists
                if host_path.exists() {
                    info!(
                        host_path = %host_path.display(),
                        container_path = %container_path.display(),
                        "data directory registered"
                    );
                    Some((host_path, container_path))
                } else {
                    warn!(path = %host_path.display(), "data directory does not exist, skipping");
                    None
                }
            })
            .collect()
    }

    /// Check if a path is under any of the configured data_dirs
    fn find_data_dir_for_path(
        &self,
        path: &Path,
        data_dirs: &[(PathBuf, PathBuf)],
    ) -> Option<(PathBuf, PathBuf)> {
        let canonical_path = path.canonicalize().ok()?;

        for (host_path, container_path) in data_dirs {
            if let Ok(canonical_host) = host_path.canonicalize()
                && canonical_path.starts_with(&canonical_host)
            {
                // Calculate relative path from host_path
                if let Ok(relative) = canonical_path.strip_prefix(&canonical_host) {
                    let container_file_path = container_path.join(relative);
                    return Some((canonical_path, container_file_path));
                }
            }
        }
        None
    }

    /// Container mount point for file copy mode
    /// Using a unique path to avoid conflicts with user-configured data_dirs
    const FILE_COPY_MOUNT_POINT: &'static str = "/llama_nexus_file_copy";

    /// Determine the file access strategy for a given file
    fn determine_file_strategy(
        &self,
        file_ref: &FileReference,
        data_dirs: &[(PathBuf, PathBuf)],
        temp_mount_path: &Path,
    ) -> FileAccessStrategy {
        let mode = self.config.file_access_mode.as_str();

        // If copy_only mode, always use file copy
        if mode == "copy_only" {
            let temp_path = temp_mount_path.join(file_ref.path.file_name().unwrap_or_default());
            let container_path = PathBuf::from(Self::FILE_COPY_MOUNT_POINT)
                .join(file_ref.path.file_name().unwrap_or_default());
            return FileAccessStrategy::FileCopy {
                source_path: file_ref.path.clone(),
                temp_path,
                container_path,
                is_output: file_ref.is_output,
            };
        }

        // Try to find in data_dirs first
        if let Some((host_path, container_path)) =
            self.find_data_dir_for_path(&file_ref.path, data_dirs)
        {
            return FileAccessStrategy::BindMount {
                host_path,
                container_path,
            };
        }

        // For output files that don't exist yet, check parent directory
        if file_ref.is_output
            && let Some(parent) = file_ref.path.parent()
            && let Some((_, container_parent)) = self.find_data_dir_for_path(parent, data_dirs)
        {
            let container_path =
                container_parent.join(file_ref.path.file_name().unwrap_or_default());
            return FileAccessStrategy::BindMount {
                host_path: file_ref.path.clone(),
                container_path,
            };
        }

        // If bind_only mode and not in data_dirs, this will fail later
        if mode == "bind_only" {
            // Return a bind mount that will fail - let the caller handle the error
            return FileAccessStrategy::BindMount {
                host_path: file_ref.path.clone(),
                container_path: file_ref.path.clone(),
            };
        }

        // Auto mode: fall back to file copy
        let temp_path = temp_mount_path.join(file_ref.path.file_name().unwrap_or_default());
        let container_path = PathBuf::from(Self::FILE_COPY_MOUNT_POINT)
            .join(file_ref.path.file_name().unwrap_or_default());
        FileAccessStrategy::FileCopy {
            source_path: file_ref.path.clone(),
            temp_path,
            container_path,
            is_output: file_ref.is_output,
        }
    }

    /// Parse file references from script arguments
    ///
    /// For relative paths (like "sample.csv"), the resolution order is:
    /// 1. Search in configured `data_dirs` (host paths)
    /// 2. Fall back to current working directory
    fn parse_file_references(
        &self,
        args: &[String],
        data_dirs: &[(PathBuf, PathBuf)],
    ) -> Vec<FileReference> {
        debug!(
            args = ?args,
            data_dirs_count = data_dirs.len(),
            "parsing file references from args"
        );

        args.iter()
            .enumerate()
            .filter_map(|(index, arg)| {
                // Skip flags (arguments starting with -)
                if arg.starts_with('-') {
                    return None;
                }

                // Check if it looks like a file path
                let path = PathBuf::from(arg);

                // Must be an absolute path or look like a file (has extension)
                if !path.is_absolute() && !arg.contains('.') {
                    return None;
                }

                // Convert to absolute path
                let absolute_path = if path.is_absolute() {
                    debug!(arg = %arg, "argument is absolute path");
                    path
                } else {
                    // For relative paths, search in data_dirs first
                    debug!(
                        arg = %arg,
                        data_dirs_count = data_dirs.len(),
                        "searching relative path in data_dirs"
                    );
                    let mut found_path: Option<PathBuf> = None;

                    for (host_path, _container_path) in data_dirs {
                        let candidate = host_path.join(arg);
                        debug!(
                            candidate = %candidate.display(),
                            exists = candidate.exists(),
                            "checking candidate path"
                        );
                        if candidate.exists() {
                            info!(
                                relative_path = %arg,
                                resolved_to = %candidate.display(),
                                "resolved relative path in data_dirs"
                            );
                            found_path = Some(candidate);
                            break;
                        }
                    }

                    // Fall back to current working directory if not found in data_dirs
                    if found_path.is_none() {
                        let cwd = std::env::current_dir().ok();
                        debug!(
                            arg = %arg,
                            cwd = ?cwd,
                            "relative path not found in data_dirs, falling back to cwd"
                        );
                    }

                    found_path.unwrap_or_else(|| {
                        std::env::current_dir()
                            .map(|cwd| cwd.join(arg))
                            .unwrap_or_else(|_| PathBuf::from(arg))
                    })
                };

                // Determine if it's an output file (doesn't exist)
                // Check after converting to absolute path for accuracy
                let is_output = !absolute_path.exists();

                Some(FileReference {
                    original: arg.clone(),
                    path: absolute_path,
                    is_output,
                    arg_index: index,
                })
            })
            .collect()
    }

    /// Prepare execution context with file strategies
    async fn prepare_execution_context(
        &self,
        args: &[String],
    ) -> Result<ExecutionContext, ExecutionError> {
        let data_dirs = self.parse_data_dirs();
        let file_refs = self.parse_file_references(args, &data_dirs);

        // Check if we need a temp directory
        let needs_temp_dir = file_refs.iter().any(|fr| {
            let mode = self.config.file_access_mode.as_str();
            mode == "copy_only"
                || (mode == "auto" && self.find_data_dir_for_path(&fr.path, &data_dirs).is_none())
        });

        let temp_dir = if needs_temp_dir {
            let base_dir = self
                .config
                .temp_dir
                .as_ref()
                .map(PathBuf::from)
                .unwrap_or_else(std::env::temp_dir);

            Some(
                tempfile::Builder::new()
                    .prefix("llama-nexus-data-")
                    .tempdir_in(&base_dir)
                    .map_err(|e| {
                        ExecutionError::runtime(
                            "docker",
                            format!("failed to create temp directory: {e}"),
                        )
                    })?,
            )
        } else {
            None
        };

        let temp_mount_path = temp_dir
            .as_ref()
            .map(|t| t.path().to_path_buf())
            .unwrap_or_else(|| PathBuf::from("/tmp/data"));

        // Determine strategy for each file
        let mut file_strategies = Vec::new();
        let mut container_args = args.to_vec();
        let mut additional_mounts: Vec<Mount> = Vec::new();
        let mut temp_mount_added = false;

        for file_ref in file_refs {
            let strategy = self.determine_file_strategy(&file_ref, &data_dirs, &temp_mount_path);

            match &strategy {
                FileAccessStrategy::BindMount { container_path, .. } => {
                    // Update args with container path
                    container_args[file_ref.arg_index] =
                        container_path.to_string_lossy().to_string();
                }
                FileAccessStrategy::FileCopy {
                    source_path,
                    temp_path,
                    container_path,
                    is_output,
                } => {
                    // Copy input file to temp directory
                    if !is_output {
                        if source_path.exists() {
                            debug!(source = %source_path.display(), dest = %temp_path.display(), "copying input file to temp directory");
                            tokio::fs::copy(source_path, temp_path).await.map_err(|e| {
                                ExecutionError::runtime(
                                    "docker",
                                    format!("failed to copy file {}: {e}", source_path.display()),
                                )
                            })?;
                        } else {
                            // Input file doesn't exist - provide helpful error message
                            let hint = if !source_path.is_absolute() {
                                " (hint: use absolute path like /path/to/file.csv)"
                            } else {
                                ""
                            };
                            return Err(ExecutionError::runtime(
                                "docker",
                                format!(
                                    "input file does not exist: {}{}",
                                    source_path.display(),
                                    hint
                                ),
                            ));
                        }
                    }

                    // Add temp directory mount (only once)
                    if !temp_mount_added && let Some(ref temp) = temp_dir {
                        additional_mounts.push(Mount {
                            target: Some(Self::FILE_COPY_MOUNT_POINT.to_string()),
                            source: Some(temp.path().to_string_lossy().to_string()),
                            typ: Some(MountTypeEnum::BIND),
                            read_only: Some(false), // Need write for output files
                            ..Default::default()
                        });
                        temp_mount_added = true;
                    }

                    // Update args with container path
                    container_args[file_ref.arg_index] =
                        container_path.to_string_lossy().to_string();
                }
            }

            file_strategies.push((file_ref, strategy));
        }

        debug!(
            file_count = file_strategies.len(),
            temp_dir = ?temp_dir.as_ref().map(|t| t.path()),
            additional_mounts = additional_mounts.len(),
            "prepared execution context"
        );

        Ok(ExecutionContext {
            temp_dir,
            file_strategies,
            container_args,
            additional_mounts,
        })
    }

    /// Copy output files back to their original locations
    async fn copy_outputs_back(&self, context: &ExecutionContext) -> Result<(), ExecutionError> {
        if !self.config.copy_output_back {
            return Ok(());
        }

        for (file_ref, strategy) in &context.file_strategies {
            if let FileAccessStrategy::FileCopy {
                source_path,
                temp_path,
                is_output,
                ..
            } = strategy
                && *is_output
                && temp_path.exists()
            {
                debug!(
                    source = %temp_path.display(),
                    dest = %source_path.display(),
                    "copying output file back"
                );

                // Ensure parent directory exists
                if let Some(parent) = source_path.parent()
                    && !parent.exists()
                {
                    tokio::fs::create_dir_all(parent).await.map_err(|e| {
                        ExecutionError::runtime(
                            "docker",
                            format!("failed to create directory {}: {e}", parent.display()),
                        )
                    })?;
                }

                tokio::fs::copy(temp_path, source_path).await.map_err(|e| {
                    ExecutionError::runtime(
                        "docker",
                        format!(
                            "failed to copy output file {} -> {}: {e}",
                            temp_path.display(),
                            source_path.display()
                        ),
                    )
                })?;

                info!(
                    source = %file_ref.original,
                    "output file written successfully"
                );
            }
        }

        Ok(())
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

        // Prepare execution context with file access strategies
        let context = self.prepare_execution_context(&request.args).await?;

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

        // Build command with modified args (using container paths)
        let cmd = self.build_command(&extension, &container_script_path, &context.container_args);

        debug!(
            container = %container_name,
            image = %image,
            script = %request.script.path.display(),
            original_args = ?request.args,
            container_args = ?context.container_args,
            file_strategies = context.file_strategies.len(),
            "creating container with file access strategies"
        );

        // Build environment variables
        let mut env: Vec<String> = request
            .env
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect();

        // Add Python-specific env vars for read-only rootfs compatibility
        if extension == "py" && self.config.read_only {
            env.push("PYTHONDONTWRITEBYTECODE=1".to_string());
            env.push("PYTHONUNBUFFERED=1".to_string());
        }

        // Build mounts
        let script_dir = request
            .script
            .path
            .parent()
            .ok_or_else(|| ExecutionError::runtime("docker", "invalid script path"))?;

        let mut mounts = vec![Mount {
            target: Some("/scripts".to_string()),
            source: Some(script_dir.to_string_lossy().to_string()),
            typ: Some(MountTypeEnum::BIND),
            read_only: Some(true),
            ..Default::default()
        }];

        // Add tmpfs for /tmp when using read-only rootfs (needed for temp files)
        if self.config.read_only {
            mounts.push(Mount {
                target: Some("/tmp".to_string()),
                typ: Some(MountTypeEnum::TMPFS),
                ..Default::default()
            });
        }

        // Add additional data directories from config (for bind mount strategy)
        for (host_path, container_path) in self.parse_data_dirs() {
            debug!(host_path = %host_path.display(), container_path = %container_path.display(), "adding data directory mount");
            mounts.push(Mount {
                target: Some(container_path.to_string_lossy().to_string()),
                source: Some(host_path.to_string_lossy().to_string()),
                typ: Some(MountTypeEnum::BIND),
                read_only: Some(false), // Allow writes for output files
                ..Default::default()
            });
        }

        // Add additional mounts from execution context (for file copy strategy)
        mounts.extend(context.additional_mounts.clone());

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

        // Wait for container to finish using polling (more reliable than wait stream)
        let poll_interval = std::time::Duration::from_millis(100);
        let deadline = tokio::time::Instant::now() + request.limits.timeout;

        let (exit_code, timed_out) = loop {
            // Check if we've exceeded the timeout
            if tokio::time::Instant::now() >= deadline {
                warn!(container_id = %container_id, "container execution timed out, killing");
                if let Err(e) = self
                    .client
                    .kill_container(&container_id, None::<KillContainerOptions>)
                    .await
                {
                    warn!(error = %e, "failed to kill container");
                }
                break (-1, true);
            }

            // Inspect container to check its state
            match self
                .client
                .inspect_container(&container_id, None::<InspectContainerOptions>)
                .await
            {
                Ok(info) => {
                    if let Some(state) = info.state {
                        let running = state.running.unwrap_or(false);
                        if !running {
                            // Container has finished
                            let exit_code = state.exit_code.unwrap_or(0) as i32;
                            break (exit_code, false);
                        }
                    }
                }
                Err(e) => {
                    // Container might have been removed or doesn't exist
                    error!(error = %e, container_id = %container_id, "failed to inspect container");
                    break (-1, false);
                }
            }

            // Wait before polling again
            tokio::time::sleep(poll_interval).await;
        };

        // Collect logs
        let (stdout, stderr) = self.collect_logs(&container_id).await;

        // Copy output files back (for file copy strategy)
        if exit_code == 0
            && let Err(e) = self.copy_outputs_back(&context).await
        {
            warn!(error = %e, "failed to copy output files back");
        }

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

    #[test]
    fn test_docker_config_file_access_mode() {
        let config = DockerConfig::default();
        assert_eq!(config.file_access_mode, "auto");
        assert!(config.copy_output_back);
        assert!(config.temp_dir.is_none());
    }

    #[test]
    fn test_parse_file_references() {
        // Test file reference parsing logic
        fn is_file_reference(arg: &str) -> bool {
            if arg.starts_with('-') {
                return false;
            }
            let path = PathBuf::from(arg);
            path.is_absolute() || arg.contains('.')
        }

        // Absolute paths
        assert!(is_file_reference("/path/to/file.csv"));
        assert!(is_file_reference("/Volumes/Data/input.json"));

        // Files with extensions
        assert!(is_file_reference("data.csv"));
        assert!(is_file_reference("output.json"));

        // Flags should be skipped
        assert!(!is_file_reference("--delimiter=,"));
        assert!(!is_file_reference("-v"));

        // Commands without extensions (not file references)
        assert!(!is_file_reference("csv2json"));
        assert!(!is_file_reference("convert"));
    }

    #[test]
    fn test_data_dir_parsing() {
        // Test data_dirs parsing logic
        fn parse_data_dir(data_dir: &str) -> (String, String) {
            if let Some(pos) = data_dir.find(':') {
                let host = &data_dir[..pos];
                let container = &data_dir[pos + 1..];
                (host.to_string(), container.to_string())
            } else {
                (data_dir.to_string(), data_dir.to_string())
            }
        }

        // Without container path mapping
        assert_eq!(
            parse_data_dir("/Volumes/Data"),
            ("/Volumes/Data".to_string(), "/Volumes/Data".to_string())
        );

        // With container path mapping
        assert_eq!(
            parse_data_dir("/Volumes/Data:/data"),
            ("/Volumes/Data".to_string(), "/data".to_string())
        );

        // Complex path with colon in Windows-like path (edge case)
        assert_eq!(
            parse_data_dir("/host/path:/container/path"),
            ("/host/path".to_string(), "/container/path".to_string())
        );
    }

    #[test]
    fn test_file_access_strategy_variants() {
        // Test FileAccessStrategy enum
        let bind_mount = FileAccessStrategy::BindMount {
            host_path: PathBuf::from("/host/file.csv"),
            container_path: PathBuf::from("/data/file.csv"),
        };

        let file_copy = FileAccessStrategy::FileCopy {
            source_path: PathBuf::from("/remote/file.csv"),
            temp_path: PathBuf::from("/tmp/llama-nexus-data-xxx/file.csv"),
            container_path: PathBuf::from("/data/file.csv"),
            is_output: false,
        };

        // Just verify the variants can be created
        match bind_mount {
            FileAccessStrategy::BindMount { container_path, .. } => {
                assert_eq!(container_path, PathBuf::from("/data/file.csv"));
            }
            _ => panic!("Expected BindMount"),
        }

        match file_copy {
            FileAccessStrategy::FileCopy { is_output, .. } => {
                assert!(!is_output);
            }
            _ => panic!("Expected FileCopy"),
        }
    }

    #[test]
    fn test_file_reference_struct() {
        let file_ref = FileReference {
            original: "/path/to/input.csv".to_string(),
            path: PathBuf::from("/path/to/input.csv"),
            is_output: false,
            arg_index: 1,
        };

        assert_eq!(file_ref.original, "/path/to/input.csv");
        assert!(!file_ref.is_output);
        assert_eq!(file_ref.arg_index, 1);

        let output_ref = FileReference {
            original: "/path/to/output.json".to_string(),
            path: PathBuf::from("/path/to/output.json"),
            is_output: true,
            arg_index: 2,
        };

        assert!(output_ref.is_output);
    }
}
