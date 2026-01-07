//! Error types for the executor module

use std::path::PathBuf;

use thiserror::Error;

/// Errors that can occur during script execution
#[derive(Debug, Error)]
#[allow(dead_code)]
pub enum ExecutionError {
    /// Script file not found
    #[error("script not found: {0}")]
    ScriptNotFound(PathBuf),

    /// Unsupported script type (no executor registered for extension)
    #[error("unsupported script type: {0}")]
    UnsupportedScript(String),

    /// No executor found for the given extension
    #[error("no executor registered for extension: {0}")]
    NoExecutorFound(String),

    /// Executor is not available (e.g., Docker daemon not running)
    #[error("executor '{0}' is not available: {1}")]
    ExecutorUnavailable(String, String),

    /// Script execution timed out
    #[error("execution timed out after {0:?}")]
    Timeout(std::time::Duration),

    /// Script exceeded memory limit
    #[error("memory limit exceeded: {used} bytes (limit: {limit} bytes)")]
    MemoryLimitExceeded { used: u64, limit: u64 },

    /// Script output exceeded size limit
    #[error("output size limit exceeded: {size} bytes (limit: {limit} bytes)")]
    OutputSizeLimitExceeded { size: u64, limit: u64 },

    /// Script execution failed with non-zero exit code
    #[error("script exited with code {code}: {message}")]
    ExecutionFailed { code: i32, message: String },

    /// Permission denied (e.g., trying to access restricted path)
    #[error("permission denied: {0}")]
    PermissionDenied(String),

    /// Runtime-specific error (WasmEdge, Docker, Deno, etc.)
    #[error("{runtime} error: {message}")]
    RuntimeError { runtime: String, message: String },

    /// Configuration error
    #[error("configuration error: {0}")]
    ConfigError(String),

    /// I/O error
    #[error("I/O error: {0}")]
    IoError(#[from] std::io::Error),

    /// Internal error
    #[error("internal error: {0}")]
    Internal(String),
}

#[allow(dead_code)]
impl ExecutionError {
    /// Creates a runtime-specific error
    pub fn runtime(runtime: impl Into<String>, message: impl Into<String>) -> Self {
        Self::RuntimeError {
            runtime: runtime.into(),
            message: message.into(),
        }
    }

    /// Creates an execution failed error
    pub fn failed(code: i32, message: impl Into<String>) -> Self {
        Self::ExecutionFailed {
            code,
            message: message.into(),
        }
    }

    /// Returns true if this error is retryable
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            ExecutionError::Timeout(_)
                | ExecutionError::ExecutorUnavailable(_, _)
                | ExecutionError::IoError(_)
        )
    }

    /// Returns true if this error indicates a resource limit was exceeded
    pub fn is_resource_limit(&self) -> bool {
        matches!(
            self,
            ExecutionError::Timeout(_)
                | ExecutionError::MemoryLimitExceeded { .. }
                | ExecutionError::OutputSizeLimitExceeded { .. }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = ExecutionError::Timeout(std::time::Duration::from_secs(30));
        assert!(err.to_string().contains("30"));

        let err = ExecutionError::runtime("docker", "container failed to start");
        assert!(err.to_string().contains("docker"));
        assert!(err.to_string().contains("container failed to start"));
    }

    #[test]
    fn test_is_retryable() {
        assert!(ExecutionError::Timeout(std::time::Duration::from_secs(1)).is_retryable());
        assert!(
            ExecutionError::ExecutorUnavailable("docker".into(), "daemon not running".into())
                .is_retryable()
        );
        assert!(!ExecutionError::PermissionDenied("test".into()).is_retryable());
    }

    #[test]
    fn test_is_resource_limit() {
        assert!(ExecutionError::Timeout(std::time::Duration::from_secs(1)).is_resource_limit());
        assert!(
            ExecutionError::MemoryLimitExceeded {
                used: 1000,
                limit: 100
            }
            .is_resource_limit()
        );
        assert!(!ExecutionError::PermissionDenied("test".into()).is_resource_limit());
    }
}
