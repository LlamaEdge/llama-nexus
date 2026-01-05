//! Script Executor Manager
//!
//! Manages multiple executor instances and routes script execution
//! to the appropriate executor based on file extension.

use std::{collections::HashMap, sync::Arc};

use tracing::{debug, info, warn};

use super::{
    error::ExecutionError,
    traits::Executor,
    types::{ExecuteRequest, ResourceLimits, ScriptOutput},
};
use crate::skills::types::ScriptInfo;

/// Script Executor Manager
///
/// Manages registered executors and routes script execution to the
/// appropriate executor based on file extension.
///
/// # Thread Safety
///
/// The manager is thread-safe and can be shared across threads using `Arc`.
/// Each executor is stored as `Arc<dyn Executor>` for shared ownership.
///
/// # Example
///
/// ```rust,ignore
/// let mut manager = ScriptExecutorManager::new(ResourceLimits::default());
///
/// // Register executors
/// manager.register(Arc::new(WasmEdgeExecutor::new()?));
/// manager.register(Arc::new(DockerExecutor::new()?));
///
/// // Execute a script
/// let script = ScriptInfo { ... };
/// let output = manager.execute(&script, vec![], HashMap::new(), None).await?;
/// ```
pub struct ScriptExecutorManager {
    /// Registered executors (extension -> executor)
    executors: HashMap<String, Arc<dyn Executor>>,
    /// Default resource limits
    default_limits: ResourceLimits,
}

impl ScriptExecutorManager {
    /// Creates a new executor manager with default resource limits
    pub fn new(default_limits: ResourceLimits) -> Self {
        Self {
            executors: HashMap::new(),
            default_limits,
        }
    }

    /// Registers an executor
    ///
    /// The executor will handle all file extensions it supports.
    /// If multiple executors support the same extension, the last
    /// registered one takes precedence.
    pub fn register(&mut self, executor: Arc<dyn Executor>) {
        let name = executor.name();
        let extensions = executor.supported_extensions();

        info!(
            executor = name,
            extensions = ?extensions,
            "registering executor"
        );

        for ext in extensions {
            let ext_lower = ext.to_lowercase();
            if self.executors.contains_key(&ext_lower) {
                warn!(
                    extension = ext_lower,
                    executor = name,
                    "overwriting existing executor for extension"
                );
            }
            self.executors.insert(ext_lower, Arc::clone(&executor));
        }
    }

    /// Unregisters all executors for a given extension
    pub fn unregister(&mut self, extension: &str) -> Option<Arc<dyn Executor>> {
        self.executors.remove(&extension.to_lowercase())
    }

    /// Gets the executor for a given extension
    pub fn get_executor(&self, extension: &str) -> Option<&Arc<dyn Executor>> {
        self.executors.get(&extension.to_lowercase())
    }

    /// Checks if an executor is registered for the given extension
    pub fn supports(&self, extension: &str) -> bool {
        self.executors.contains_key(&extension.to_lowercase())
    }

    /// Returns all supported extensions
    pub fn supported_extensions(&self) -> Vec<&str> {
        self.executors.keys().map(|s| s.as_str()).collect()
    }

    /// Returns the number of registered executors
    pub fn executor_count(&self) -> usize {
        // Count unique executors (same executor may handle multiple extensions)
        let unique: std::collections::HashSet<_> =
            self.executors.values().map(|e| e.name()).collect();
        unique.len()
    }

    /// Executes a script using the appropriate executor
    ///
    /// # Arguments
    ///
    /// * `script` - The script to execute
    /// * `args` - Command line arguments
    /// * `env` - Environment variables
    /// * `limits` - Optional resource limits (uses default if None)
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - No executor is registered for the script's extension
    /// - The script file doesn't exist
    /// - Execution fails
    pub async fn execute(
        &self,
        script: &ScriptInfo,
        args: Vec<String>,
        env: HashMap<String, String>,
        limits: Option<ResourceLimits>,
    ) -> Result<ScriptOutput, ExecutionError> {
        // Get extension
        let ext = script
            .path
            .extension()
            .and_then(|e| e.to_str())
            .ok_or_else(|| {
                ExecutionError::UnsupportedScript(format!(
                    "no extension: {}",
                    script.path.display()
                ))
            })?;

        // Find executor
        let executor = self
            .executors
            .get(&ext.to_lowercase())
            .ok_or_else(|| ExecutionError::NoExecutorFound(ext.to_string()))?;

        debug!(
            script = %script.path.display(),
            executor = executor.name(),
            "executing script"
        );

        // Build request
        let request = ExecuteRequest {
            script: script.clone(),
            args,
            env,
            limits: limits.unwrap_or_else(|| self.default_limits.clone()),
            working_dir: script.path.parent().map(|p| p.to_path_buf()),
            stdin: None,
        };

        // Execute
        let output = executor.execute(request).await?;

        debug!(
            script = %script.path.display(),
            exit_code = output.exit_code,
            duration = ?output.duration,
            "script execution completed"
        );

        Ok(output)
    }

    /// Performs health checks on all registered executors
    ///
    /// Returns a map of executor name -> health check result.
    pub async fn health_check_all(&self) -> HashMap<String, Result<(), ExecutionError>> {
        let mut results = HashMap::new();

        // Collect unique executors
        let mut seen = std::collections::HashSet::new();
        for executor in self.executors.values() {
            let name = executor.name().to_string();
            if seen.insert(name.clone()) {
                let result = executor.health_check().await;
                results.insert(name, result);
            }
        }

        results
    }
}

impl Default for ScriptExecutorManager {
    fn default() -> Self {
        Self::new(ResourceLimits::default())
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::executor::traits::IsolationLevel;

    // Mock executor for testing
    struct MockExecutor {
        name: &'static str,
        extensions: Vec<&'static str>,
    }

    #[async_trait::async_trait]
    impl Executor for MockExecutor {
        fn name(&self) -> &str {
            self.name
        }

        fn supported_extensions(&self) -> Vec<&str> {
            self.extensions.clone()
        }

        async fn execute(&self, _request: ExecuteRequest) -> Result<ScriptOutput, ExecutionError> {
            Ok(ScriptOutput {
                stdout: format!("executed by {}", self.name),
                exit_code: 0,
                ..Default::default()
            })
        }

        fn isolation_level(&self) -> IsolationLevel {
            IsolationLevel::None
        }
    }

    #[test]
    fn test_register_executor() {
        let mut manager = ScriptExecutorManager::default();

        let executor = Arc::new(MockExecutor {
            name: "test",
            extensions: vec!["py", "sh"],
        });

        manager.register(executor);

        assert!(manager.supports("py"));
        assert!(manager.supports("sh"));
        assert!(manager.supports("PY")); // Case insensitive
        assert!(!manager.supports("js"));
    }

    #[test]
    fn test_get_executor() {
        let mut manager = ScriptExecutorManager::default();

        let executor = Arc::new(MockExecutor {
            name: "test",
            extensions: vec!["py"],
        });

        manager.register(executor);

        let exec = manager.get_executor("py");
        assert!(exec.is_some());
        assert_eq!(exec.unwrap().name(), "test");
    }

    #[test]
    fn test_executor_count() {
        let mut manager = ScriptExecutorManager::default();

        // Register one executor handling multiple extensions
        manager.register(Arc::new(MockExecutor {
            name: "multi",
            extensions: vec!["py", "sh", "rb"],
        }));

        // Register another executor
        manager.register(Arc::new(MockExecutor {
            name: "js",
            extensions: vec!["js", "ts"],
        }));

        assert_eq!(manager.executor_count(), 2);
        assert_eq!(manager.supported_extensions().len(), 5);
    }

    #[tokio::test]
    async fn test_execute() {
        let mut manager = ScriptExecutorManager::default();

        manager.register(Arc::new(MockExecutor {
            name: "python",
            extensions: vec!["py"],
        }));

        let script = ScriptInfo {
            name: "test.py".to_string(),
            path: PathBuf::from("/test/script.py"),
            executable: true,
        };

        let output = manager
            .execute(&script, vec![], HashMap::new(), None)
            .await
            .unwrap();

        assert_eq!(output.exit_code, 0);
        assert!(output.stdout.contains("python"));
    }

    #[tokio::test]
    async fn test_execute_no_executor() {
        let manager = ScriptExecutorManager::default();

        let script = ScriptInfo {
            name: "test.py".to_string(),
            path: PathBuf::from("/test/script.py"),
            executable: true,
        };

        let result = manager.execute(&script, vec![], HashMap::new(), None).await;

        assert!(matches!(result, Err(ExecutionError::NoExecutorFound(_))));
    }
}
