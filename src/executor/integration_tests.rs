//! Integration tests for the executor module
//!
//! Tests for Skills script discovery + execution flow, multiple script types,
//! executor auto-selection, and error handling.

use std::{collections::HashMap, path::PathBuf, sync::Arc};

use super::{
    deno::{DenoConfig, DenoExecutor},
    error::ExecutionError,
    manager::ScriptExecutorManager,
    traits::{Executor, IsolationLevel},
    types::{ExecuteRequest, FilesystemPolicy, ResourceLimits, ScriptOutput},
};
use crate::skills::types::ScriptInfo;

// ============================================================================
// Mock Executor for Integration Testing
// ============================================================================

/// Mock executor for testing executor routing and management
struct MockExecutor {
    name: &'static str,
    extensions: Vec<&'static str>,
    isolation_level: IsolationLevel,
    /// If true, execute returns a permission denied error
    should_fail: bool,
    /// Recorded execution count
    execution_count: std::sync::atomic::AtomicUsize,
}

impl MockExecutor {
    fn new(name: &'static str, extensions: Vec<&'static str>) -> Self {
        Self {
            name,
            extensions,
            isolation_level: IsolationLevel::None,
            should_fail: false,
            execution_count: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    fn with_isolation(mut self, level: IsolationLevel) -> Self {
        self.isolation_level = level;
        self
    }

    fn failing(mut self) -> Self {
        self.should_fail = true;
        self
    }

    fn execution_count(&self) -> usize {
        self.execution_count
            .load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl Executor for MockExecutor {
    fn name(&self) -> &str {
        self.name
    }

    fn supported_extensions(&self) -> Vec<&str> {
        self.extensions.clone()
    }

    async fn execute(&self, request: ExecuteRequest) -> Result<ScriptOutput, ExecutionError> {
        self.execution_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        if self.should_fail {
            return Err(ExecutionError::PermissionDenied("Test error".to_string()));
        }

        Ok(ScriptOutput {
            stdout: format!("Executed {} with {}", request.script.name, self.name()),
            stderr: String::new(),
            exit_code: 0,
            duration: std::time::Duration::from_millis(10),
            resource_usage: super::types::ResourceUsage::default(),
            timed_out: false,
        })
    }

    fn isolation_level(&self) -> IsolationLevel {
        self.isolation_level.clone()
    }
}

// ============================================================================
// TEST-001: Integration Tests
// ============================================================================

/// Test executor auto-selection based on file extension
#[tokio::test]
async fn test_executor_auto_selection() {
    let mut manager = ScriptExecutorManager::default();

    // Register multiple executors
    let python_executor = Arc::new(MockExecutor::new("python", vec!["py"]));
    let js_executor = Arc::new(MockExecutor::new("js", vec!["js", "ts", "mjs"]));
    let shell_executor = Arc::new(MockExecutor::new("shell", vec!["sh", "bash"]));

    manager.register(python_executor.clone());
    manager.register(js_executor.clone());
    manager.register(shell_executor.clone());

    // Test Python script routing
    let py_script = ScriptInfo {
        name: "test.py".to_string(),
        path: PathBuf::from("/scripts/test.py"),
        executable: true,
    };
    let result = manager
        .execute(&py_script, vec![], HashMap::new(), None)
        .await;
    assert!(result.is_ok());
    assert!(result.unwrap().stdout.contains("python"));

    // Test JavaScript script routing
    let js_script = ScriptInfo {
        name: "test.js".to_string(),
        path: PathBuf::from("/scripts/test.js"),
        executable: true,
    };
    let result = manager
        .execute(&js_script, vec![], HashMap::new(), None)
        .await;
    assert!(result.is_ok());
    assert!(result.unwrap().stdout.contains("js"));

    // Test TypeScript script routing
    let ts_script = ScriptInfo {
        name: "test.ts".to_string(),
        path: PathBuf::from("/scripts/test.ts"),
        executable: true,
    };
    let result = manager
        .execute(&ts_script, vec![], HashMap::new(), None)
        .await;
    assert!(result.is_ok());
    assert!(result.unwrap().stdout.contains("js"));

    // Test Shell script routing
    let sh_script = ScriptInfo {
        name: "test.sh".to_string(),
        path: PathBuf::from("/scripts/test.sh"),
        executable: true,
    };
    let result = manager
        .execute(&sh_script, vec![], HashMap::new(), None)
        .await;
    assert!(result.is_ok());
    assert!(result.unwrap().stdout.contains("shell"));
}

/// Test mixed script type execution
#[tokio::test]
async fn test_multiple_script_types_execution() {
    let mut manager = ScriptExecutorManager::default();

    let multi_executor = Arc::new(MockExecutor::new("multi", vec!["py", "js", "sh"]));
    manager.register(multi_executor.clone());

    let scripts = vec![
        ScriptInfo {
            name: "script1.py".to_string(),
            path: PathBuf::from("/scripts/script1.py"),
            executable: true,
        },
        ScriptInfo {
            name: "script2.js".to_string(),
            path: PathBuf::from("/scripts/script2.js"),
            executable: true,
        },
        ScriptInfo {
            name: "script3.sh".to_string(),
            path: PathBuf::from("/scripts/script3.sh"),
            executable: true,
        },
    ];

    // Execute all scripts
    for script in &scripts {
        let result = manager.execute(script, vec![], HashMap::new(), None).await;
        assert!(result.is_ok(), "Failed to execute {}", script.name);
    }

    // Verify all scripts were executed
    assert_eq!(multi_executor.execution_count(), 3);
}

/// Test error handling for unsupported script types
#[tokio::test]
async fn test_unsupported_script_type() {
    let mut manager = ScriptExecutorManager::default();

    // Only register Python executor
    manager.register(Arc::new(MockExecutor::new("python", vec!["py"])));

    // Try to execute an unsupported script type
    let ruby_script = ScriptInfo {
        name: "test.rb".to_string(),
        path: PathBuf::from("/scripts/test.rb"),
        executable: true,
    };

    let result = manager
        .execute(&ruby_script, vec![], HashMap::new(), None)
        .await;
    assert!(result.is_err());
    assert!(matches!(result, Err(ExecutionError::NoExecutorFound(_))));
}

/// Test error handling for scripts without extension
#[tokio::test]
async fn test_script_without_extension() {
    let manager = ScriptExecutorManager::default();

    let no_ext_script = ScriptInfo {
        name: "script".to_string(),
        path: PathBuf::from("/scripts/script"),
        executable: true,
    };

    let result = manager
        .execute(&no_ext_script, vec![], HashMap::new(), None)
        .await;
    assert!(result.is_err());
    assert!(matches!(result, Err(ExecutionError::UnsupportedScript(_))));
}

/// Test error propagation from executor
#[tokio::test]
async fn test_executor_error_propagation() {
    let mut manager = ScriptExecutorManager::default();

    let failing_executor = Arc::new(MockExecutor::new("failing", vec!["fail"]).failing());
    manager.register(failing_executor);

    let script = ScriptInfo {
        name: "test.fail".to_string(),
        path: PathBuf::from("/scripts/test.fail"),
        executable: true,
    };

    let result = manager.execute(&script, vec![], HashMap::new(), None).await;
    assert!(result.is_err());
    assert!(matches!(result, Err(ExecutionError::PermissionDenied(_))));
}

/// Test custom resource limits are passed to executor
#[tokio::test]
async fn test_custom_resource_limits() {
    let mut manager = ScriptExecutorManager::new(ResourceLimits::default());

    // Custom executor that verifies limits
    struct LimitsCheckExecutor;

    #[async_trait::async_trait]
    impl Executor for LimitsCheckExecutor {
        fn name(&self) -> &str {
            "limits-check"
        }

        fn supported_extensions(&self) -> Vec<&str> {
            vec!["test"]
        }

        async fn execute(&self, request: ExecuteRequest) -> Result<ScriptOutput, ExecutionError> {
            // Verify custom limits were passed
            assert_eq!(request.limits.max_memory_bytes, 128 * 1024 * 1024);
            assert_eq!(request.limits.timeout, std::time::Duration::from_secs(10));
            assert!(request.limits.network_access);

            Ok(ScriptOutput {
                stdout: "Limits verified".to_string(),
                ..Default::default()
            })
        }

        fn isolation_level(&self) -> IsolationLevel {
            IsolationLevel::None
        }
    }

    manager.register(Arc::new(LimitsCheckExecutor));

    let script = ScriptInfo {
        name: "test.test".to_string(),
        path: PathBuf::from("/scripts/test.test"),
        executable: true,
    };

    let custom_limits = ResourceLimits {
        max_memory_bytes: 128 * 1024 * 1024,
        timeout: std::time::Duration::from_secs(10),
        max_output_bytes: 1024 * 1024,
        network_access: true,
        filesystem_access: FilesystemPolicy::None,
    };

    let result = manager
        .execute(&script, vec![], HashMap::new(), Some(custom_limits))
        .await;
    assert!(result.is_ok());
}

/// Test executor override when same extension is registered twice
#[tokio::test]
async fn test_executor_override() {
    let mut manager = ScriptExecutorManager::default();

    let first_executor = Arc::new(MockExecutor::new("first", vec!["py"]));
    let second_executor = Arc::new(MockExecutor::new("second", vec!["py"]));

    manager.register(first_executor.clone());
    manager.register(second_executor.clone());

    let script = ScriptInfo {
        name: "test.py".to_string(),
        path: PathBuf::from("/scripts/test.py"),
        executable: true,
    };

    let result = manager.execute(&script, vec![], HashMap::new(), None).await;
    assert!(result.is_ok());

    // Second executor should be used (override)
    assert!(result.unwrap().stdout.contains("second"));
    assert_eq!(first_executor.execution_count(), 0);
    assert_eq!(second_executor.execution_count(), 1);
}

/// Test case-insensitive extension matching
#[tokio::test]
async fn test_case_insensitive_extension() {
    let mut manager = ScriptExecutorManager::default();

    manager.register(Arc::new(MockExecutor::new("python", vec!["py"])));

    // Test uppercase extension
    let script = ScriptInfo {
        name: "test.PY".to_string(),
        path: PathBuf::from("/scripts/test.PY"),
        executable: true,
    };

    let result = manager.execute(&script, vec![], HashMap::new(), None).await;
    assert!(result.is_ok());
}

/// Test isolation levels are correctly reported
#[tokio::test]
async fn test_isolation_levels() {
    let mut manager = ScriptExecutorManager::default();

    let runtime_executor =
        Arc::new(MockExecutor::new("deno", vec!["js"]).with_isolation(IsolationLevel::Runtime));
    let container_executor =
        Arc::new(MockExecutor::new("docker", vec!["py"]).with_isolation(IsolationLevel::Container));

    manager.register(runtime_executor);
    manager.register(container_executor);

    // Verify isolation levels through get_executor
    let js_executor = manager.get_executor("js").unwrap();
    assert!(matches!(
        js_executor.isolation_level(),
        IsolationLevel::Runtime
    ));

    let py_executor = manager.get_executor("py").unwrap();
    assert!(matches!(
        py_executor.isolation_level(),
        IsolationLevel::Container
    ));
}

/// Test health check for all registered executors
#[tokio::test]
async fn test_health_check_all() {
    let mut manager = ScriptExecutorManager::default();

    manager.register(Arc::new(MockExecutor::new("exec1", vec!["a"])));
    manager.register(Arc::new(MockExecutor::new("exec2", vec!["b"])));

    let results = manager.health_check_all().await;

    assert_eq!(results.len(), 2);
    assert!(results.get("exec1").unwrap().is_ok());
    assert!(results.get("exec2").unwrap().is_ok());
}

/// Test arguments and environment variables are passed correctly
#[tokio::test]
async fn test_args_and_env_passing() {
    let mut manager = ScriptExecutorManager::default();

    struct ArgsEnvCheckExecutor;

    #[async_trait::async_trait]
    impl Executor for ArgsEnvCheckExecutor {
        fn name(&self) -> &str {
            "args-env-check"
        }

        fn supported_extensions(&self) -> Vec<&str> {
            vec!["test"]
        }

        async fn execute(&self, request: ExecuteRequest) -> Result<ScriptOutput, ExecutionError> {
            // Verify args
            assert_eq!(request.args, vec!["--verbose", "--output", "result.txt"]);

            // Verify env
            assert_eq!(request.env.get("API_KEY"), Some(&"secret".to_string()));
            assert_eq!(request.env.get("DEBUG"), Some(&"true".to_string()));

            Ok(ScriptOutput::default())
        }

        fn isolation_level(&self) -> IsolationLevel {
            IsolationLevel::None
        }
    }

    manager.register(Arc::new(ArgsEnvCheckExecutor));

    let script = ScriptInfo {
        name: "test.test".to_string(),
        path: PathBuf::from("/scripts/test.test"),
        executable: true,
    };

    let args = vec![
        "--verbose".to_string(),
        "--output".to_string(),
        "result.txt".to_string(),
    ];

    let mut env = HashMap::new();
    env.insert("API_KEY".to_string(), "secret".to_string());
    env.insert("DEBUG".to_string(), "true".to_string());

    let result = manager.execute(&script, args, env, None).await;
    assert!(result.is_ok());
}

/// Test default resource limits are applied when none specified
#[tokio::test]
async fn test_default_resource_limits() {
    let custom_default = ResourceLimits {
        max_memory_bytes: 512 * 1024 * 1024, // 512MB
        timeout: std::time::Duration::from_secs(60),
        max_output_bytes: 5 * 1024 * 1024,
        network_access: false,
        filesystem_access: FilesystemPolicy::None,
    };

    let mut manager = ScriptExecutorManager::new(custom_default.clone());

    struct DefaultLimitsCheckExecutor {
        expected_memory: u64,
    }

    #[async_trait::async_trait]
    impl Executor for DefaultLimitsCheckExecutor {
        fn name(&self) -> &str {
            "default-check"
        }

        fn supported_extensions(&self) -> Vec<&str> {
            vec!["test"]
        }

        async fn execute(&self, request: ExecuteRequest) -> Result<ScriptOutput, ExecutionError> {
            assert_eq!(request.limits.max_memory_bytes, self.expected_memory);
            Ok(ScriptOutput::default())
        }

        fn isolation_level(&self) -> IsolationLevel {
            IsolationLevel::None
        }
    }

    manager.register(Arc::new(DefaultLimitsCheckExecutor {
        expected_memory: 512 * 1024 * 1024,
    }));

    let script = ScriptInfo {
        name: "test.test".to_string(),
        path: PathBuf::from("/scripts/test.test"),
        executable: true,
    };

    // Execute without specifying limits - should use manager's default
    let result = manager.execute(&script, vec![], HashMap::new(), None).await;
    assert!(result.is_ok());
}

// ============================================================================
// Deno Executor Integration Tests (if Deno is available)
// ============================================================================

/// Test Deno executor health check
#[tokio::test]
async fn test_deno_executor_health_check() {
    let executor = DenoExecutor::with_defaults();

    match executor {
        Ok(exec) => {
            let health = exec.health_check().await;
            // Health check may pass or fail depending on Deno availability
            // Just verify it doesn't panic
            let _ = health;
        }
        Err(_) => {
            // Deno not available, skip test
        }
    }
}

/// Test Deno executor supported extensions
#[test]
fn test_deno_executor_extensions() {
    if let Ok(executor) = DenoExecutor::with_defaults() {
        let extensions = executor.supported_extensions();
        assert!(extensions.contains(&"js"));
        assert!(extensions.contains(&"ts"));
        assert!(extensions.contains(&"mjs"));
        assert!(extensions.contains(&"mts"));
        assert!(extensions.contains(&"jsx"));
        assert!(extensions.contains(&"tsx"));
    }
}
