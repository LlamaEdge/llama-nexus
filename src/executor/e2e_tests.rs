//! End-to-end tests for the executor module
//!
//! These tests verify the complete script execution workflow from
//! skill script discovery to execution and output handling.

use std::{collections::HashMap, path::PathBuf, sync::Arc};

use tempfile::TempDir;

use super::{
    deno::DenoExecutor,
    error::ExecutionError,
    manager::ScriptExecutorManager,
    traits::{Executor, IsolationLevel},
    types::{ExecuteRequest, ResourceLimits, ScriptOutput},
};
use crate::skills::types::ScriptInfo;

// ============================================================================
// Mock Executor for E2E Testing
// ============================================================================

/// A mock executor that simulates real execution behavior
struct E2EMockExecutor {
    name: &'static str,
    extensions: Vec<&'static str>,
    /// Simulated execution results based on script content
    results: HashMap<String, ScriptOutput>,
}

impl E2EMockExecutor {
    fn new(name: &'static str, extensions: Vec<&'static str>) -> Self {
        Self {
            name,
            extensions,
            results: HashMap::new(),
        }
    }

    fn with_result(mut self, script_name: &str, output: ScriptOutput) -> Self {
        self.results.insert(script_name.to_string(), output);
        self
    }
}

#[async_trait::async_trait]
impl Executor for E2EMockExecutor {
    fn name(&self) -> &str {
        self.name
    }

    fn supported_extensions(&self) -> Vec<&str> {
        self.extensions.clone()
    }

    async fn execute(&self, request: ExecuteRequest) -> Result<ScriptOutput, ExecutionError> {
        // Return pre-configured result if available
        if let Some(output) = self.results.get(&request.script.name) {
            return Ok(output.clone());
        }

        // Default behavior: return script info in output
        Ok(ScriptOutput {
            stdout: format!(
                "Executed: {} with args: {:?}",
                request.script.name, request.args
            ),
            stderr: String::new(),
            exit_code: 0,
            duration: std::time::Duration::from_millis(50),
            resource_usage: super::types::ResourceUsage::default(),
            timed_out: false,
        })
    }

    fn isolation_level(&self) -> IsolationLevel {
        IsolationLevel::None
    }
}

// ============================================================================
// Test Fixtures
// ============================================================================

/// Create a test skill directory with scripts
fn create_test_skill_with_scripts(base_dir: &std::path::Path, skill_name: &str) -> PathBuf {
    let skill_dir = base_dir.join(skill_name);
    let scripts_dir = skill_dir.join("scripts");
    std::fs::create_dir_all(&scripts_dir).unwrap();

    // Create various script types
    std::fs::write(
        scripts_dir.join("process.js"),
        r#"
// JavaScript processing script
const args = Deno.args;
console.log("Processing:", args.join(" "));
"#,
    )
    .unwrap();

    std::fs::write(
        scripts_dir.join("analyze.ts"),
        r#"
// TypeScript analysis script
const input = Deno.args[0] || "default";
console.log(`Analyzing: ${input}`);
"#,
    )
    .unwrap();

    std::fs::write(
        scripts_dir.join("helper.sh"),
        r#"#!/bin/bash
echo "Helper script executed"
"#,
    )
    .unwrap();

    std::fs::write(
        scripts_dir.join("utility.py"),
        r#"#!/usr/bin/env python3
import sys
print(f"Python utility: {sys.argv[1:]}")
"#,
    )
    .unwrap();

    skill_dir
}

// ============================================================================
// E2E Test: Complete Script Execution Flow
// ============================================================================

#[tokio::test]
async fn test_e2e_complete_script_execution_flow() {
    let temp_dir = TempDir::new().unwrap();
    let skill_dir = create_test_skill_with_scripts(temp_dir.path(), "test-skill");

    let mut manager = ScriptExecutorManager::default();

    // Register mock executors for different script types
    let js_executor = Arc::new(
        E2EMockExecutor::new("deno", vec!["js", "ts", "mjs", "mts"])
            .with_result(
                "process.js",
                ScriptOutput {
                    stdout: "Processing: arg1 arg2".to_string(),
                    exit_code: 0,
                    ..Default::default()
                },
            )
            .with_result(
                "analyze.ts",
                ScriptOutput {
                    stdout: "Analyzing: test-input".to_string(),
                    exit_code: 0,
                    ..Default::default()
                },
            ),
    );

    let shell_executor = Arc::new(
        E2EMockExecutor::new("shell", vec!["sh", "bash"]).with_result(
            "helper.sh",
            ScriptOutput {
                stdout: "Helper script executed".to_string(),
                exit_code: 0,
                ..Default::default()
            },
        ),
    );

    let python_executor = Arc::new(E2EMockExecutor::new("python", vec!["py"]).with_result(
        "utility.py",
        ScriptOutput {
            stdout: "Python utility: ['test']".to_string(),
            exit_code: 0,
            ..Default::default()
        },
    ));

    manager.register(js_executor);
    manager.register(shell_executor);
    manager.register(python_executor);

    // Discover scripts in skill directory
    let scripts_dir = skill_dir.join("scripts");
    let scripts: Vec<ScriptInfo> = std::fs::read_dir(&scripts_dir)
        .unwrap()
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let path = entry.path();
            if path.is_file() {
                Some(ScriptInfo {
                    name: path.file_name()?.to_str()?.to_string(),
                    path,
                    executable: true,
                })
            } else {
                None
            }
        })
        .collect();

    assert_eq!(scripts.len(), 4);

    // Execute each script and verify output
    for script in &scripts {
        let result = manager
            .execute(script, vec!["test".to_string()], HashMap::new(), None)
            .await;
        assert!(result.is_ok(), "Failed to execute {}", script.name);
        assert_eq!(result.unwrap().exit_code, 0);
    }
}

// ============================================================================
// E2E Test: Script Discovery and Categorization
// ============================================================================

#[tokio::test]
async fn test_e2e_script_discovery_and_categorization() {
    let temp_dir = TempDir::new().unwrap();
    let _skill_dir = create_test_skill_with_scripts(temp_dir.path(), "categorize-skill");

    let mut manager = ScriptExecutorManager::default();

    // Register executors
    manager.register(Arc::new(E2EMockExecutor::new(
        "deno",
        vec!["js", "ts", "mjs", "mts"],
    )));
    manager.register(Arc::new(E2EMockExecutor::new("shell", vec!["sh", "bash"])));
    manager.register(Arc::new(E2EMockExecutor::new("python", vec!["py"])));

    // Verify extension support
    assert!(manager.supports("js"));
    assert!(manager.supports("ts"));
    assert!(manager.supports("sh"));
    assert!(manager.supports("py"));
    assert!(!manager.supports("rb")); // Ruby not supported

    // Verify executor count
    assert_eq!(manager.executor_count(), 3);

    // Verify supported extensions list
    let extensions = manager.supported_extensions();
    assert!(extensions.contains(&"js"));
    assert!(extensions.contains(&"ts"));
    assert!(extensions.contains(&"sh"));
    assert!(extensions.contains(&"py"));
}

// ============================================================================
// E2E Test: Error Handling in Script Execution
// ============================================================================

#[tokio::test]
async fn test_e2e_error_handling_in_script_execution() {
    let temp_dir = TempDir::new().unwrap();
    let skill_dir = temp_dir.path().join("error-skill");
    let scripts_dir = skill_dir.join("scripts");
    std::fs::create_dir_all(&scripts_dir).unwrap();

    // Create script that will fail
    std::fs::write(scripts_dir.join("failing.js"), "throw new Error('test');").unwrap();

    let mut manager = ScriptExecutorManager::default();

    // Create executor that returns failure
    struct FailingExecutor;

    #[async_trait::async_trait]
    impl Executor for FailingExecutor {
        fn name(&self) -> &str {
            "failing"
        }

        fn supported_extensions(&self) -> Vec<&str> {
            vec!["js"]
        }

        async fn execute(&self, _request: ExecuteRequest) -> Result<ScriptOutput, ExecutionError> {
            Err(ExecutionError::RuntimeError {
                runtime: "test".to_string(),
                message: "Script threw an error".to_string(),
            })
        }

        fn isolation_level(&self) -> IsolationLevel {
            IsolationLevel::None
        }
    }

    manager.register(Arc::new(FailingExecutor));

    let script = ScriptInfo {
        name: "failing.js".to_string(),
        path: scripts_dir.join("failing.js"),
        executable: true,
    };

    let result = manager.execute(&script, vec![], HashMap::new(), None).await;
    assert!(result.is_err());
    assert!(matches!(result, Err(ExecutionError::RuntimeError { .. })));
}

// ============================================================================
// E2E Test: Resource Limits Enforcement
// ============================================================================

#[tokio::test]
async fn test_e2e_resource_limits_enforcement() {
    let mut manager = ScriptExecutorManager::new(ResourceLimits::default());

    /// Executor that verifies resource limits
    struct LimitsVerifyingExecutor {
        expected_timeout_secs: u64,
        expected_memory_mb: u64,
    }

    #[async_trait::async_trait]
    impl Executor for LimitsVerifyingExecutor {
        fn name(&self) -> &str {
            "limits-verifier"
        }

        fn supported_extensions(&self) -> Vec<&str> {
            vec!["test"]
        }

        async fn execute(&self, request: ExecuteRequest) -> Result<ScriptOutput, ExecutionError> {
            // Verify limits match expected values
            assert_eq!(request.limits.timeout.as_secs(), self.expected_timeout_secs);
            assert_eq!(
                request.limits.max_memory_bytes,
                self.expected_memory_mb * 1024 * 1024
            );

            Ok(ScriptOutput {
                stdout: "Limits verified".to_string(),
                ..Default::default()
            })
        }

        fn isolation_level(&self) -> IsolationLevel {
            IsolationLevel::Runtime
        }
    }

    // Register executor expecting specific limits
    manager.register(Arc::new(LimitsVerifyingExecutor {
        expected_timeout_secs: 30,
        expected_memory_mb: 256,
    }));

    let script = ScriptInfo {
        name: "test.test".to_string(),
        path: PathBuf::from("/tmp/test.test"),
        executable: true,
    };

    // Execute with custom limits
    let custom_limits = ResourceLimits {
        timeout: std::time::Duration::from_secs(30),
        max_memory_bytes: 256 * 1024 * 1024,
        ..Default::default()
    };

    let result = manager
        .execute(&script, vec![], HashMap::new(), Some(custom_limits))
        .await;
    assert!(result.is_ok());
}

// ============================================================================
// E2E Test: Environment Variable Passing
// ============================================================================

#[tokio::test]
async fn test_e2e_environment_variable_passing() {
    let mut manager = ScriptExecutorManager::default();

    /// Executor that captures environment variables
    struct EnvCaptureExecutor;

    #[async_trait::async_trait]
    impl Executor for EnvCaptureExecutor {
        fn name(&self) -> &str {
            "env-capture"
        }

        fn supported_extensions(&self) -> Vec<&str> {
            vec!["env"]
        }

        async fn execute(&self, request: ExecuteRequest) -> Result<ScriptOutput, ExecutionError> {
            // Return env vars in stdout for verification
            let env_str: String = request
                .env
                .iter()
                .map(|(k, v)| format!("{}={}", k, v))
                .collect::<Vec<_>>()
                .join("\n");

            Ok(ScriptOutput {
                stdout: env_str,
                ..Default::default()
            })
        }

        fn isolation_level(&self) -> IsolationLevel {
            IsolationLevel::None
        }
    }

    manager.register(Arc::new(EnvCaptureExecutor));

    let script = ScriptInfo {
        name: "test.env".to_string(),
        path: PathBuf::from("/tmp/test.env"),
        executable: true,
    };

    let mut env = HashMap::new();
    env.insert("API_KEY".to_string(), "secret123".to_string());
    env.insert("DEBUG".to_string(), "true".to_string());
    env.insert("CONFIG_PATH".to_string(), "/etc/config".to_string());

    let result = manager.execute(&script, vec![], env, None).await.unwrap();

    assert!(result.stdout.contains("API_KEY=secret123"));
    assert!(result.stdout.contains("DEBUG=true"));
    assert!(result.stdout.contains("CONFIG_PATH=/etc/config"));
}

// ============================================================================
// E2E Test: Working Directory Configuration
// ============================================================================

#[tokio::test]
async fn test_e2e_working_directory_configuration() {
    let temp_dir = TempDir::new().unwrap();
    let script_dir = temp_dir.path().join("scripts");
    std::fs::create_dir_all(&script_dir).unwrap();
    std::fs::write(script_dir.join("script.wdir"), "test content").unwrap();

    let mut manager = ScriptExecutorManager::default();

    /// Executor that verifies working directory
    struct WorkdirVerifyingExecutor {
        expected_dir: PathBuf,
    }

    #[async_trait::async_trait]
    impl Executor for WorkdirVerifyingExecutor {
        fn name(&self) -> &str {
            "workdir-verify"
        }

        fn supported_extensions(&self) -> Vec<&str> {
            vec!["wdir"]
        }

        async fn execute(&self, request: ExecuteRequest) -> Result<ScriptOutput, ExecutionError> {
            let working_dir = request
                .working_dir
                .as_ref()
                .expect("Working dir should be set");
            assert_eq!(working_dir, &self.expected_dir);

            Ok(ScriptOutput {
                stdout: format!("Working dir: {}", working_dir.display()),
                ..Default::default()
            })
        }

        fn isolation_level(&self) -> IsolationLevel {
            IsolationLevel::None
        }
    }

    manager.register(Arc::new(WorkdirVerifyingExecutor {
        expected_dir: script_dir.clone(),
    }));

    let script = ScriptInfo {
        name: "script.wdir".to_string(),
        path: script_dir.join("script.wdir"),
        executable: true,
    };

    let result = manager.execute(&script, vec![], HashMap::new(), None).await;
    assert!(result.is_ok());
    assert!(
        result
            .unwrap()
            .stdout
            .contains(&script_dir.display().to_string())
    );
}

// ============================================================================
// E2E Test: Health Check Integration
// ============================================================================

#[tokio::test]
async fn test_e2e_health_check_integration() {
    let mut manager = ScriptExecutorManager::default();

    /// Healthy executor
    struct HealthyExecutor;

    #[async_trait::async_trait]
    impl Executor for HealthyExecutor {
        fn name(&self) -> &str {
            "healthy"
        }
        fn supported_extensions(&self) -> Vec<&str> {
            vec!["healthy"]
        }
        async fn execute(&self, _: ExecuteRequest) -> Result<ScriptOutput, ExecutionError> {
            Ok(ScriptOutput::default())
        }
        fn isolation_level(&self) -> IsolationLevel {
            IsolationLevel::None
        }
        async fn health_check(&self) -> Result<(), ExecutionError> {
            Ok(())
        }
    }

    /// Unhealthy executor
    struct UnhealthyExecutor;

    #[async_trait::async_trait]
    impl Executor for UnhealthyExecutor {
        fn name(&self) -> &str {
            "unhealthy"
        }
        fn supported_extensions(&self) -> Vec<&str> {
            vec!["unhealthy"]
        }
        async fn execute(&self, _: ExecuteRequest) -> Result<ScriptOutput, ExecutionError> {
            Ok(ScriptOutput::default())
        }
        fn isolation_level(&self) -> IsolationLevel {
            IsolationLevel::None
        }
        async fn health_check(&self) -> Result<(), ExecutionError> {
            Err(ExecutionError::ExecutorUnavailable(
                "unhealthy".to_string(),
                "Service unavailable".to_string(),
            ))
        }
    }

    manager.register(Arc::new(HealthyExecutor));
    manager.register(Arc::new(UnhealthyExecutor));

    let results = manager.health_check_all().await;

    assert_eq!(results.len(), 2);
    assert!(results.get("healthy").unwrap().is_ok());
    assert!(results.get("unhealthy").unwrap().is_err());
}

// ============================================================================
// E2E Test: Deno Executor Real Execution (if available)
// ============================================================================

#[tokio::test]
async fn test_e2e_deno_real_execution() {
    // Skip if Deno is not available
    let executor = match DenoExecutor::new() {
        Ok(e) => e,
        Err(_) => {
            eprintln!("Skipping Deno real execution test: Deno not available");
            return;
        }
    };

    // Check health before proceeding
    if executor.health_check().await.is_err() {
        eprintln!("Skipping Deno real execution test: Deno health check failed");
        return;
    }

    // Create a simple JavaScript file
    let temp_dir = TempDir::new().unwrap();
    let script_path = temp_dir.path().join("hello.js");
    std::fs::write(&script_path, "console.log('Hello from Deno!');").unwrap();

    let script = ScriptInfo {
        name: "hello.js".to_string(),
        path: script_path,
        executable: true,
    };

    let request = ExecuteRequest {
        script,
        args: vec![],
        env: HashMap::new(),
        limits: ResourceLimits {
            timeout: std::time::Duration::from_secs(10),
            ..Default::default()
        },
        working_dir: Some(temp_dir.path().to_path_buf()),
        stdin: None,
    };

    let result = executor.execute(request).await;
    assert!(result.is_ok(), "Deno execution failed: {:?}", result);

    let output = result.unwrap();
    assert_eq!(output.exit_code, 0);
    assert!(output.stdout.contains("Hello from Deno!"));
}

// ============================================================================
// E2E Test: Script Output Capture
// ============================================================================

#[tokio::test]
async fn test_e2e_script_output_capture() {
    let mut manager = ScriptExecutorManager::default();

    /// Executor that produces mixed output
    struct MixedOutputExecutor;

    #[async_trait::async_trait]
    impl Executor for MixedOutputExecutor {
        fn name(&self) -> &str {
            "mixed-output"
        }
        fn supported_extensions(&self) -> Vec<&str> {
            vec!["mixed"]
        }
        async fn execute(&self, _: ExecuteRequest) -> Result<ScriptOutput, ExecutionError> {
            Ok(ScriptOutput {
                stdout: "Standard output line 1\nStandard output line 2\n".to_string(),
                stderr: "Warning: something happened\nError: but we recovered\n".to_string(),
                exit_code: 0,
                duration: std::time::Duration::from_millis(100),
                ..Default::default()
            })
        }
        fn isolation_level(&self) -> IsolationLevel {
            IsolationLevel::None
        }
    }

    manager.register(Arc::new(MixedOutputExecutor));

    let script = ScriptInfo {
        name: "test.mixed".to_string(),
        path: PathBuf::from("/tmp/test.mixed"),
        executable: true,
    };

    let result = manager
        .execute(&script, vec![], HashMap::new(), None)
        .await
        .unwrap();

    // Verify stdout
    assert!(result.stdout.contains("Standard output line 1"));
    assert!(result.stdout.contains("Standard output line 2"));

    // Verify stderr
    assert!(result.stderr.contains("Warning:"));
    assert!(result.stderr.contains("Error:"));

    // Verify exit code
    assert_eq!(result.exit_code, 0);

    // Verify duration was captured
    assert!(result.duration.as_millis() >= 100);
}

// ============================================================================
// E2E Test: Concurrent Script Execution
// ============================================================================

#[tokio::test]
async fn test_e2e_concurrent_script_execution() {
    let manager = Arc::new({
        let mut m = ScriptExecutorManager::default();

        /// Executor that simulates work
        struct SlowExecutor;

        #[async_trait::async_trait]
        impl Executor for SlowExecutor {
            fn name(&self) -> &str {
                "slow"
            }
            fn supported_extensions(&self) -> Vec<&str> {
                vec!["slow"]
            }
            async fn execute(
                &self,
                request: ExecuteRequest,
            ) -> Result<ScriptOutput, ExecutionError> {
                // Simulate some work
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                Ok(ScriptOutput {
                    stdout: format!("Completed: {}", request.script.name),
                    ..Default::default()
                })
            }
            fn isolation_level(&self) -> IsolationLevel {
                IsolationLevel::None
            }
        }

        m.register(Arc::new(SlowExecutor));
        m
    });

    // Create multiple scripts
    let scripts: Vec<ScriptInfo> = (0..5)
        .map(|i| ScriptInfo {
            name: format!("script{}.slow", i),
            path: PathBuf::from(format!("/tmp/script{}.slow", i)),
            executable: true,
        })
        .collect();

    // Execute all scripts concurrently
    let start = std::time::Instant::now();

    let handles: Vec<_> = scripts
        .into_iter()
        .map(|script| {
            let mgr = manager.clone();
            tokio::spawn(async move { mgr.execute(&script, vec![], HashMap::new(), None).await })
        })
        .collect();

    // Wait for all to complete
    let mut results = vec![];
    for handle in handles {
        results.push(handle.await.unwrap());
    }

    let duration = start.elapsed();

    // All should succeed
    for result in &results {
        assert!(result.is_ok());
    }

    // Concurrent execution should be faster than sequential (5 * 50ms = 250ms)
    // Allow some margin for overhead
    assert!(
        duration.as_millis() < 200,
        "Concurrent execution took too long: {:?}",
        duration
    );
}

// ============================================================================
// E2E Test: Script Execution with Arguments
// ============================================================================

#[tokio::test]
async fn test_e2e_script_execution_with_arguments() {
    let mut manager = ScriptExecutorManager::default();

    /// Executor that echoes arguments
    struct ArgEchoExecutor;

    #[async_trait::async_trait]
    impl Executor for ArgEchoExecutor {
        fn name(&self) -> &str {
            "arg-echo"
        }
        fn supported_extensions(&self) -> Vec<&str> {
            vec!["args"]
        }
        async fn execute(&self, request: ExecuteRequest) -> Result<ScriptOutput, ExecutionError> {
            Ok(ScriptOutput {
                stdout: format!("Args: [{}]", request.args.join(", ")),
                ..Default::default()
            })
        }
        fn isolation_level(&self) -> IsolationLevel {
            IsolationLevel::None
        }
    }

    manager.register(Arc::new(ArgEchoExecutor));

    let script = ScriptInfo {
        name: "test.args".to_string(),
        path: PathBuf::from("/tmp/test.args"),
        executable: true,
    };

    // Test with various argument patterns
    let args = vec![
        "--verbose".to_string(),
        "-o".to_string(),
        "output.txt".to_string(),
        "input file.txt".to_string(),         // With space
        "--config=/etc/app.conf".to_string(), // With equals
    ];

    let result = manager
        .execute(&script, args.clone(), HashMap::new(), None)
        .await
        .unwrap();

    assert!(result.stdout.contains("--verbose"));
    assert!(result.stdout.contains("output.txt"));
    assert!(result.stdout.contains("input file.txt"));
    assert!(result.stdout.contains("--config=/etc/app.conf"));
}

// ============================================================================
// Performance Test: Executor Routing
// ============================================================================

#[tokio::test]
async fn test_performance_executor_routing() {
    let mut manager = ScriptExecutorManager::default();

    // Register many executors
    for i in 0..10 {
        struct PerfExecutor {
            name: String,
            ext: String,
        }

        #[async_trait::async_trait]
        impl Executor for PerfExecutor {
            fn name(&self) -> &str {
                &self.name
            }
            fn supported_extensions(&self) -> Vec<&str> {
                vec![Box::leak(self.ext.clone().into_boxed_str())]
            }
            async fn execute(&self, _: ExecuteRequest) -> Result<ScriptOutput, ExecutionError> {
                Ok(ScriptOutput::default())
            }
            fn isolation_level(&self) -> IsolationLevel {
                IsolationLevel::None
            }
        }

        manager.register(Arc::new(PerfExecutor {
            name: format!("executor{}", i),
            ext: format!("ext{}", i),
        }));
    }

    // Measure routing time
    let script = ScriptInfo {
        name: "test.ext5".to_string(),
        path: PathBuf::from("/tmp/test.ext5"),
        executable: true,
    };

    let start = std::time::Instant::now();
    for _ in 0..1000 {
        let _ = manager.execute(&script, vec![], HashMap::new(), None).await;
    }
    let duration = start.elapsed();

    // 1000 routing operations should complete quickly (< 500ms)
    assert!(
        duration.as_millis() < 500,
        "Executor routing took too long: {:?}",
        duration
    );
}
