//! Deno Executor
//!
//! Executes JavaScript and TypeScript scripts using Deno runtime.
//! Leverages Deno's built-in permission system for sandboxing.

use std::{collections::HashMap, path::PathBuf, process::Stdio, time::Instant};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncReadExt;
use tracing::{debug, info, warn};

use super::{
    error::ExecutionError,
    traits::{Executor, IsolationLevel},
    types::{ExecuteRequest, FilesystemPolicy, ResourceUsage, ScriptOutput},
};

/// Default Deno binary name
const DEFAULT_DENO_BINARY: &str = "deno";

/// Deno executor configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DenoConfig {
    /// Path to Deno binary (default: "deno" from PATH)
    #[serde(default = "default_deno_path")]
    pub deno_path: PathBuf,

    /// Whether to allow network access by default
    #[serde(default)]
    pub allow_net: bool,

    /// Whether to allow environment variable access by default
    #[serde(default)]
    pub allow_env: bool,

    /// Whether to allow high-resolution time measurement
    #[serde(default)]
    pub allow_hrtime: bool,

    /// Whether to run in unstable mode
    #[serde(default)]
    pub unstable: bool,

    /// Additional V8 flags
    #[serde(default)]
    pub v8_flags: Vec<String>,

    /// Whether to cache compiled scripts
    #[serde(default = "default_true")]
    pub cache: bool,

    /// Custom cache directory (default: system temp)
    pub cache_dir: Option<PathBuf>,
}

fn default_deno_path() -> PathBuf {
    PathBuf::from(DEFAULT_DENO_BINARY)
}

fn default_true() -> bool {
    true
}

impl Default for DenoConfig {
    fn default() -> Self {
        Self {
            deno_path: default_deno_path(),
            allow_net: false,
            allow_env: false,
            allow_hrtime: false,
            unstable: false,
            v8_flags: Vec::new(),
            cache: true,
            cache_dir: None,
        }
    }
}

/// Deno executor for running JavaScript/TypeScript scripts
pub struct DenoExecutor {
    /// Configuration
    config: DenoConfig,
}

impl DenoExecutor {
    /// Creates a new Deno executor with the given configuration
    pub fn new(config: DenoConfig) -> Result<Self, ExecutionError> {
        Ok(Self { config })
    }

    /// Creates a new Deno executor with default configuration
    pub fn with_defaults() -> Result<Self, ExecutionError> {
        Self::new(DenoConfig::default())
    }

    /// Builds the permission flags based on request limits
    fn build_permission_flags(&self, request: &ExecuteRequest) -> Vec<String> {
        let mut flags = Vec::new();

        // File system permissions
        match &request.limits.filesystem_access {
            FilesystemPolicy::None => {
                // No file access - this is the default in Deno
            }
            FilesystemPolicy::ReadOnly(paths) => {
                if paths.is_empty() {
                    // Allow read to script directory only
                    if let Some(parent) = request.script.path.parent() {
                        flags.push(format!("--allow-read={}", parent.display()));
                    }
                } else {
                    let paths_str: Vec<String> =
                        paths.iter().map(|p| p.display().to_string()).collect();
                    flags.push(format!("--allow-read={}", paths_str.join(",")));
                }
            }
            FilesystemPolicy::ReadWrite(paths) => {
                if paths.is_empty() {
                    // Allow read/write to script directory only
                    if let Some(parent) = request.script.path.parent() {
                        let parent_str = parent.display().to_string();
                        flags.push(format!("--allow-read={}", parent_str));
                        flags.push(format!("--allow-write={}", parent_str));
                    }
                } else {
                    let paths_str: Vec<String> =
                        paths.iter().map(|p| p.display().to_string()).collect();
                    let joined = paths_str.join(",");
                    flags.push(format!("--allow-read={}", joined));
                    flags.push(format!("--allow-write={}", joined));
                }
            }
        }

        // Network permissions
        if request.limits.network_access || self.config.allow_net {
            flags.push("--allow-net".to_string());
        }

        // Environment variable permissions
        if self.config.allow_env {
            flags.push("--allow-env".to_string());
        } else if !request.env.is_empty() {
            // Allow access to specific environment variables
            let env_keys: Vec<&str> = request.env.keys().map(|s| s.as_str()).collect();
            flags.push(format!("--allow-env={}", env_keys.join(",")));
        }

        // High-resolution time
        if self.config.allow_hrtime {
            flags.push("--allow-hrtime".to_string());
        }

        flags
    }

    /// Builds the complete command arguments
    fn build_command_args(&self, request: &ExecuteRequest) -> Vec<String> {
        let mut args = Vec::new();

        // Subcommand
        args.push("run".to_string());

        // V8 flags
        for flag in &self.config.v8_flags {
            args.push(format!("--v8-flags={}", flag));
        }

        // Unstable mode
        if self.config.unstable {
            args.push("--unstable".to_string());
        }

        // Cache settings
        if !self.config.cache {
            args.push("--no-cache".to_string());
        }

        // Permission flags
        args.extend(self.build_permission_flags(request));

        // Script path
        args.push(request.script.path.display().to_string());

        // Script arguments
        args.extend(request.args.clone());

        args
    }

    /// Builds environment variables for the process
    fn build_env(&self, request: &ExecuteRequest) -> HashMap<String, String> {
        let mut env = request.env.clone();

        // Set cache directory if configured
        if let Some(cache_dir) = &self.config.cache_dir {
            env.insert("DENO_DIR".to_string(), cache_dir.display().to_string());
        }

        // Disable color output for consistent parsing
        env.insert("NO_COLOR".to_string(), "1".to_string());

        env
    }
}

#[async_trait]
impl Executor for DenoExecutor {
    fn name(&self) -> &str {
        "deno"
    }

    fn supported_extensions(&self) -> Vec<&str> {
        vec!["js", "ts", "mjs", "mts", "jsx", "tsx"]
    }

    fn isolation_level(&self) -> IsolationLevel {
        IsolationLevel::Runtime
    }

    async fn execute(&self, request: ExecuteRequest) -> Result<ScriptOutput, ExecutionError> {
        let start_time = Instant::now();

        // Build command
        let args = self.build_command_args(&request);
        let env = self.build_env(&request);

        debug!(
            script = %request.script.path.display(),
            args = ?args,
            "executing deno script"
        );

        // Create command
        let mut cmd = tokio::process::Command::new(&self.config.deno_path);
        cmd.args(&args)
            .envs(&env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        // Set working directory if specified
        if let Some(working_dir) = &request.working_dir {
            cmd.current_dir(working_dir);
        } else if let Some(parent) = request.script.path.parent() {
            cmd.current_dir(parent);
        }

        // Spawn process
        let mut child = cmd.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                ExecutionError::runtime(
                    "deno",
                    format!(
                        "Deno binary not found at '{}'. Please install Deno or configure the correct path.",
                        self.config.deno_path.display()
                    ),
                )
            } else {
                ExecutionError::runtime("deno", format!("failed to spawn Deno process: {e}"))
            }
        })?;

        // Handle stdin if provided
        if let Some(stdin_data) = &request.stdin {
            if let Some(mut stdin) = child.stdin.take() {
                use tokio::io::AsyncWriteExt;
                if let Err(e) = stdin.write_all(stdin_data.as_bytes()).await {
                    warn!(error = %e, "failed to write to stdin");
                }
            }
        }

        // Wait for completion with timeout
        let output_result = tokio::time::timeout(request.limits.timeout, async {
            let mut stdout_handle = child.stdout.take().unwrap();
            let mut stderr_handle = child.stderr.take().unwrap();

            let mut stdout = Vec::new();
            let mut stderr = Vec::new();

            // Read stdout and stderr with size limits
            let max_output = request.limits.max_output_bytes as usize;

            let (stdout_result, stderr_result, wait_result) = tokio::join!(
                async {
                    let mut buf = vec![0u8; 8192];
                    loop {
                        match stdout_handle.read(&mut buf).await {
                            Ok(0) => break,
                            Ok(n) => {
                                let remaining = max_output.saturating_sub(stdout.len());
                                if remaining > 0 {
                                    stdout.extend_from_slice(&buf[..n.min(remaining)]);
                                }
                            }
                            Err(e) => {
                                warn!(error = %e, "error reading stdout");
                                break;
                            }
                        }
                    }
                },
                async {
                    let mut buf = vec![0u8; 8192];
                    loop {
                        match stderr_handle.read(&mut buf).await {
                            Ok(0) => break,
                            Ok(n) => {
                                let remaining = max_output.saturating_sub(stderr.len());
                                if remaining > 0 {
                                    stderr.extend_from_slice(&buf[..n.min(remaining)]);
                                }
                            }
                            Err(e) => {
                                warn!(error = %e, "error reading stderr");
                                break;
                            }
                        }
                    }
                },
                child.wait()
            );

            let status = wait_result.map_err(|e| {
                ExecutionError::runtime("deno", format!("failed to wait for process: {e}"))
            })?;

            // Suppress unused variable warnings
            let _ = stdout_result;
            let _ = stderr_result;

            Ok::<_, ExecutionError>((
                String::from_utf8_lossy(&stdout).to_string(),
                String::from_utf8_lossy(&stderr).to_string(),
                status.code().unwrap_or(-1),
            ))
        })
        .await;

        let duration = start_time.elapsed();

        match output_result {
            Ok(Ok((stdout, stderr, exit_code))) => {
                debug!(
                    script = %request.script.path.display(),
                    exit_code,
                    duration = ?duration,
                    "deno script completed"
                );

                Ok(ScriptOutput {
                    stdout,
                    stderr,
                    exit_code,
                    duration,
                    resource_usage: ResourceUsage {
                        peak_memory_bytes: 0, // Not easily measurable for external processes
                        cpu_time_ms: duration.as_millis() as u64,
                    },
                    timed_out: false,
                })
            }
            Ok(Err(e)) => Err(e),
            Err(_) => {
                // Timeout occurred
                warn!(
                    script = %request.script.path.display(),
                    timeout = ?request.limits.timeout,
                    "deno script timed out"
                );

                // Process will be killed by kill_on_drop(true)
                Ok(ScriptOutput {
                    stdout: String::new(),
                    stderr: format!(
                        "Script execution timed out after {:?}",
                        request.limits.timeout
                    ),
                    exit_code: -1,
                    duration,
                    resource_usage: ResourceUsage::default(),
                    timed_out: true,
                })
            }
        }
    }

    async fn health_check(&self) -> Result<(), ExecutionError> {
        // Check if Deno binary exists and is executable
        let output = tokio::process::Command::new(&self.config.deno_path)
            .arg("--version")
            .output()
            .await
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    ExecutionError::runtime(
                        "deno",
                        format!(
                            "Deno binary not found at '{}'",
                            self.config.deno_path.display()
                        ),
                    )
                } else {
                    ExecutionError::runtime("deno", format!("failed to check Deno version: {e}"))
                }
            })?;

        if output.status.success() {
            let version = String::from_utf8_lossy(&output.stdout);
            info!(version = %version.trim(), "Deno health check passed");
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(ExecutionError::runtime(
                "deno",
                format!("Deno version check failed: {}", stderr),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::skills::types::ScriptInfo;

    #[test]
    fn test_deno_config_default() {
        let config = DenoConfig::default();
        assert_eq!(config.deno_path, PathBuf::from("deno"));
        assert!(!config.allow_net);
        assert!(!config.allow_env);
        assert!(!config.allow_hrtime);
        assert!(!config.unstable);
        assert!(config.cache);
        assert!(config.v8_flags.is_empty());
    }

    #[test]
    fn test_supported_extensions() {
        let executor = DenoExecutor::with_defaults().unwrap();
        let extensions = executor.supported_extensions();

        assert!(extensions.contains(&"js"));
        assert!(extensions.contains(&"ts"));
        assert!(extensions.contains(&"mjs"));
        assert!(extensions.contains(&"mts"));
        assert!(extensions.contains(&"jsx"));
        assert!(extensions.contains(&"tsx"));
    }

    #[test]
    fn test_isolation_level() {
        let executor = DenoExecutor::with_defaults().unwrap();
        assert_eq!(executor.isolation_level(), IsolationLevel::Runtime);
    }

    #[test]
    fn test_build_permission_flags_no_access() {
        let executor = DenoExecutor::with_defaults().unwrap();
        let request = ExecuteRequest {
            script: ScriptInfo {
                name: "test.js".to_string(),
                path: PathBuf::from("/scripts/test.js"),
                executable: true,
            },
            args: vec![],
            env: HashMap::new(),
            limits: crate::executor::ResourceLimits::strict(),
            working_dir: None,
            stdin: None,
        };

        let flags = executor.build_permission_flags(&request);
        // With FilesystemPolicy::None, no read/write flags
        assert!(!flags.iter().any(|f| f.starts_with("--allow-read")));
        assert!(!flags.iter().any(|f| f.starts_with("--allow-write")));
        assert!(!flags.iter().any(|f| f == "--allow-net"));
    }

    #[test]
    fn test_build_permission_flags_with_network() {
        let mut config = DenoConfig::default();
        config.allow_net = true;

        let executor = DenoExecutor::new(config).unwrap();
        let request = ExecuteRequest {
            script: ScriptInfo {
                name: "test.js".to_string(),
                path: PathBuf::from("/scripts/test.js"),
                executable: true,
            },
            args: vec![],
            env: HashMap::new(),
            limits: crate::executor::ResourceLimits::default(),
            working_dir: None,
            stdin: None,
        };

        let flags = executor.build_permission_flags(&request);
        assert!(flags.contains(&"--allow-net".to_string()));
    }

    #[test]
    fn test_build_permission_flags_readonly() {
        let executor = DenoExecutor::with_defaults().unwrap();
        let mut limits = crate::executor::ResourceLimits::default();
        limits.filesystem_access = FilesystemPolicy::ReadOnly(vec![PathBuf::from("/data")]);

        let request = ExecuteRequest {
            script: ScriptInfo {
                name: "test.js".to_string(),
                path: PathBuf::from("/scripts/test.js"),
                executable: true,
            },
            args: vec![],
            env: HashMap::new(),
            limits,
            working_dir: None,
            stdin: None,
        };

        let flags = executor.build_permission_flags(&request);
        assert!(flags.iter().any(|f| f == "--allow-read=/data"));
        assert!(!flags.iter().any(|f| f.starts_with("--allow-write")));
    }

    #[test]
    fn test_build_permission_flags_readwrite() {
        let executor = DenoExecutor::with_defaults().unwrap();
        let mut limits = crate::executor::ResourceLimits::default();
        limits.filesystem_access =
            FilesystemPolicy::ReadWrite(vec![PathBuf::from("/data"), PathBuf::from("/tmp")]);

        let request = ExecuteRequest {
            script: ScriptInfo {
                name: "test.js".to_string(),
                path: PathBuf::from("/scripts/test.js"),
                executable: true,
            },
            args: vec![],
            env: HashMap::new(),
            limits,
            working_dir: None,
            stdin: None,
        };

        let flags = executor.build_permission_flags(&request);
        assert!(flags.iter().any(|f| f == "--allow-read=/data,/tmp"));
        assert!(flags.iter().any(|f| f == "--allow-write=/data,/tmp"));
    }

    #[test]
    fn test_build_command_args() {
        let executor = DenoExecutor::with_defaults().unwrap();
        let request = ExecuteRequest {
            script: ScriptInfo {
                name: "test.ts".to_string(),
                path: PathBuf::from("/scripts/test.ts"),
                executable: true,
            },
            args: vec!["--arg1".to_string(), "value".to_string()],
            env: HashMap::new(),
            limits: crate::executor::ResourceLimits::default(),
            working_dir: None,
            stdin: None,
        };

        let args = executor.build_command_args(&request);

        assert_eq!(args[0], "run");
        assert!(args.contains(&"/scripts/test.ts".to_string()));
        assert!(args.contains(&"--arg1".to_string()));
        assert!(args.contains(&"value".to_string()));
    }

    #[test]
    fn test_build_command_args_with_unstable() {
        let mut config = DenoConfig::default();
        config.unstable = true;
        config.v8_flags = vec!["--max-old-space-size=512".to_string()];

        let executor = DenoExecutor::new(config).unwrap();
        let request = ExecuteRequest {
            script: ScriptInfo {
                name: "test.ts".to_string(),
                path: PathBuf::from("/scripts/test.ts"),
                executable: true,
            },
            args: vec![],
            env: HashMap::new(),
            limits: crate::executor::ResourceLimits::default(),
            working_dir: None,
            stdin: None,
        };

        let args = executor.build_command_args(&request);

        assert!(args.contains(&"--unstable".to_string()));
        assert!(
            args.iter()
                .any(|a| a == "--v8-flags=--max-old-space-size=512")
        );
    }

    #[test]
    fn test_build_env() {
        let mut config = DenoConfig::default();
        config.cache_dir = Some(PathBuf::from("/tmp/deno-cache"));

        let executor = DenoExecutor::new(config).unwrap();

        let mut request_env = HashMap::new();
        request_env.insert("MY_VAR".to_string(), "value".to_string());

        let request = ExecuteRequest {
            script: ScriptInfo {
                name: "test.js".to_string(),
                path: PathBuf::from("/scripts/test.js"),
                executable: true,
            },
            args: vec![],
            env: request_env,
            limits: crate::executor::ResourceLimits::default(),
            working_dir: None,
            stdin: None,
        };

        let env = executor.build_env(&request);

        assert_eq!(env.get("MY_VAR"), Some(&"value".to_string()));
        assert_eq!(env.get("DENO_DIR"), Some(&"/tmp/deno-cache".to_string()));
        assert_eq!(env.get("NO_COLOR"), Some(&"1".to_string()));
    }

    #[test]
    fn test_build_permission_flags_with_env_vars() {
        let executor = DenoExecutor::with_defaults().unwrap();

        let mut request_env = HashMap::new();
        request_env.insert("API_KEY".to_string(), "secret".to_string());
        request_env.insert("DB_URL".to_string(), "localhost".to_string());

        let request = ExecuteRequest {
            script: ScriptInfo {
                name: "test.js".to_string(),
                path: PathBuf::from("/scripts/test.js"),
                executable: true,
            },
            args: vec![],
            env: request_env,
            limits: crate::executor::ResourceLimits::default(),
            working_dir: None,
            stdin: None,
        };

        let flags = executor.build_permission_flags(&request);

        // Should have --allow-env with specific variables
        let env_flag = flags.iter().find(|f| f.starts_with("--allow-env="));
        assert!(env_flag.is_some());
        let env_flag = env_flag.unwrap();
        assert!(env_flag.contains("API_KEY") || env_flag.contains("DB_URL"));
    }
}
