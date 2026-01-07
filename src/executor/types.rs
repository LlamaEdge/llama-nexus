//! Type definitions for the executor module

use std::{collections::HashMap, path::PathBuf, time::Duration};

use serde::{Deserialize, Serialize};

use crate::skills::types::ScriptInfo;

/// Script execution request
#[derive(Debug, Clone)]
pub struct ExecuteRequest {
    /// Script information
    pub script: ScriptInfo,
    /// Command line arguments
    pub args: Vec<String>,
    /// Environment variables
    pub env: HashMap<String, String>,
    /// Resource limits
    pub limits: ResourceLimits,
    /// Working directory (optional, defaults to script directory)
    pub working_dir: Option<PathBuf>,
    /// Standard input content (optional)
    pub stdin: Option<String>,
}

#[allow(dead_code)]
impl ExecuteRequest {
    /// Creates a new execution request with default limits
    pub fn new(script: ScriptInfo) -> Self {
        Self {
            script,
            args: Vec::new(),
            env: HashMap::new(),
            limits: ResourceLimits::default(),
            working_dir: None,
            stdin: None,
        }
    }

    /// Sets command line arguments
    pub fn with_args(mut self, args: Vec<String>) -> Self {
        self.args = args;
        self
    }

    /// Sets environment variables
    pub fn with_env(mut self, env: HashMap<String, String>) -> Self {
        self.env = env;
        self
    }

    /// Sets resource limits
    pub fn with_limits(mut self, limits: ResourceLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Sets working directory
    pub fn with_working_dir(mut self, dir: PathBuf) -> Self {
        self.working_dir = Some(dir);
        self
    }

    /// Sets standard input
    pub fn with_stdin(mut self, stdin: String) -> Self {
        self.stdin = Some(stdin);
        self
    }
}

/// Script execution output
#[derive(Debug, Clone, Default)]
pub struct ScriptOutput {
    /// Standard output content
    pub stdout: String,
    /// Standard error content
    pub stderr: String,
    /// Exit code (0 = success)
    pub exit_code: i32,
    /// Execution duration
    pub duration: Duration,
    /// Resource usage statistics (reserved for future use)
    #[allow(dead_code)]
    pub resource_usage: ResourceUsage,
    /// Whether the script was killed due to timeout
    pub timed_out: bool,
}

#[allow(dead_code)]
impl ScriptOutput {
    /// Returns true if the script exited successfully (exit code 0)
    pub fn success(&self) -> bool {
        self.exit_code == 0 && !self.timed_out
    }

    /// Returns combined stdout and stderr
    pub fn combined_output(&self) -> String {
        if self.stderr.is_empty() {
            self.stdout.clone()
        } else if self.stdout.is_empty() {
            self.stderr.clone()
        } else {
            format!("{}\n{}", self.stdout, self.stderr)
        }
    }
}

/// Resource usage statistics
#[derive(Debug, Clone, Default)]
pub struct ResourceUsage {
    /// Peak memory usage in bytes (reserved for future use)
    #[allow(dead_code)]
    pub peak_memory_bytes: u64,
    /// CPU time in milliseconds (reserved for future use)
    #[allow(dead_code)]
    pub cpu_time_ms: u64,
}

/// Resource limits configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceLimits {
    /// Maximum memory in bytes (default: 256MB)
    #[serde(default = "default_max_memory")]
    pub max_memory_bytes: u64,

    /// Maximum execution time (default: 30 seconds)
    #[serde(
        default = "default_timeout",
        with = "humantime_serde",
        rename = "timeout"
    )]
    pub timeout: Duration,

    /// Maximum output size in bytes (default: 1MB)
    #[serde(default = "default_max_output")]
    pub max_output_bytes: u64,

    /// Whether network access is allowed (default: false)
    #[serde(default)]
    pub network_access: bool,

    /// Filesystem access policy (default: None)
    #[serde(default)]
    pub filesystem_access: FilesystemPolicy,
}

fn default_max_memory() -> u64 {
    256 * 1024 * 1024 // 256MB
}

fn default_timeout() -> Duration {
    Duration::from_secs(30)
}

fn default_max_output() -> u64 {
    1024 * 1024 // 1MB
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            max_memory_bytes: default_max_memory(),
            timeout: default_timeout(),
            max_output_bytes: default_max_output(),
            network_access: false,
            filesystem_access: FilesystemPolicy::None,
        }
    }
}

#[allow(dead_code)]
impl ResourceLimits {
    /// Creates limits suitable for untrusted scripts
    pub fn strict() -> Self {
        Self {
            max_memory_bytes: 64 * 1024 * 1024, // 64MB
            timeout: Duration::from_secs(10),
            max_output_bytes: 256 * 1024, // 256KB
            network_access: false,
            filesystem_access: FilesystemPolicy::None,
        }
    }

    /// Creates limits suitable for trusted scripts
    pub fn permissive() -> Self {
        Self {
            max_memory_bytes: 1024 * 1024 * 1024, // 1GB
            timeout: Duration::from_secs(300),    // 5 minutes
            max_output_bytes: 10 * 1024 * 1024,   // 10MB
            network_access: true,
            filesystem_access: FilesystemPolicy::ReadWrite(vec![]),
        }
    }
}

/// Filesystem access policy
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FilesystemPolicy {
    /// No filesystem access allowed
    #[default]
    None,
    /// Read-only access to specified paths
    ReadOnly(Vec<PathBuf>),
    /// Read-write access to specified paths
    ReadWrite(Vec<PathBuf>),
}

#[allow(dead_code)]
impl FilesystemPolicy {
    /// Returns true if any filesystem access is allowed
    pub fn allows_access(&self) -> bool {
        !matches!(self, FilesystemPolicy::None)
    }

    /// Returns true if write access is allowed
    pub fn allows_write(&self) -> bool {
        matches!(self, FilesystemPolicy::ReadWrite(_))
    }

    /// Returns the allowed paths (empty if no access)
    pub fn allowed_paths(&self) -> &[PathBuf] {
        match self {
            FilesystemPolicy::None => &[],
            FilesystemPolicy::ReadOnly(paths) | FilesystemPolicy::ReadWrite(paths) => paths,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resource_limits_default() {
        let limits = ResourceLimits::default();
        assert_eq!(limits.max_memory_bytes, 256 * 1024 * 1024);
        assert_eq!(limits.timeout, Duration::from_secs(30));
        assert!(!limits.network_access);
    }

    #[test]
    fn test_resource_limits_strict() {
        let limits = ResourceLimits::strict();
        assert_eq!(limits.max_memory_bytes, 64 * 1024 * 1024);
        assert_eq!(limits.timeout, Duration::from_secs(10));
    }

    #[test]
    fn test_script_output_success() {
        let output = ScriptOutput {
            exit_code: 0,
            timed_out: false,
            ..Default::default()
        };
        assert!(output.success());

        let output = ScriptOutput {
            exit_code: 1,
            timed_out: false,
            ..Default::default()
        };
        assert!(!output.success());

        let output = ScriptOutput {
            exit_code: 0,
            timed_out: true,
            ..Default::default()
        };
        assert!(!output.success());
    }

    #[test]
    fn test_filesystem_policy() {
        assert!(!FilesystemPolicy::None.allows_access());
        assert!(FilesystemPolicy::ReadOnly(vec![]).allows_access());
        assert!(FilesystemPolicy::ReadWrite(vec![]).allows_access());

        assert!(!FilesystemPolicy::None.allows_write());
        assert!(!FilesystemPolicy::ReadOnly(vec![]).allows_write());
        assert!(FilesystemPolicy::ReadWrite(vec![]).allows_write());
    }

    #[test]
    fn test_execute_request_builder() {
        use std::path::PathBuf;

        let script = ScriptInfo {
            name: "test.py".to_string(),
            path: PathBuf::from("/scripts/test.py"),
            executable: true,
        };

        let request = ExecuteRequest::new(script)
            .with_args(vec!["--verbose".to_string()])
            .with_limits(ResourceLimits::strict());

        assert_eq!(request.args, vec!["--verbose"]);
        assert_eq!(request.limits.max_memory_bytes, 64 * 1024 * 1024);
    }
}
