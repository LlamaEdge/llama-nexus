//! Performance benchmark tests for the executor module
//!
//! Tests measure:
//! - Executor startup time
//! - Script execution latency
//! - Resource usage
//! - Concurrent execution performance

use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Instant};

use super::{
    deno::DenoExecutor,
    error::ExecutionError,
    manager::ScriptExecutorManager,
    traits::{Executor, IsolationLevel},
    types::{ExecuteRequest, ResourceLimits, ScriptOutput},
};
use crate::skills::types::ScriptInfo;

// ============================================================================
// Benchmark Configuration
// ============================================================================

/// Performance thresholds for benchmarks
struct BenchmarkThresholds {
    /// Maximum acceptable executor registration time (ms)
    max_registration_time_ms: u64,
    /// Maximum acceptable single execution time (ms)
    max_single_execution_ms: u64,
    /// Maximum acceptable routing time for 1000 operations (ms)
    max_routing_1000_ms: u64,
    /// Maximum acceptable time for 100 concurrent executions (ms)
    max_concurrent_100_ms: u64,
}

impl Default for BenchmarkThresholds {
    fn default() -> Self {
        Self {
            max_registration_time_ms: 10,
            max_single_execution_ms: 50,
            max_routing_1000_ms: 100,
            max_concurrent_100_ms: 500,
        }
    }
}

// ============================================================================
// Mock Executor for Benchmarking
// ============================================================================

/// Lightweight mock executor for performance testing
struct BenchmarkExecutor {
    name: String,
    extensions: Vec<String>,
    simulated_latency_us: u64,
}

impl BenchmarkExecutor {
    fn new(name: &str, extensions: &[&str]) -> Self {
        Self {
            name: name.to_string(),
            extensions: extensions.iter().map(|s| s.to_string()).collect(),
            simulated_latency_us: 0,
        }
    }

    fn with_latency(mut self, latency_us: u64) -> Self {
        self.simulated_latency_us = latency_us;
        self
    }
}

#[async_trait::async_trait]
impl Executor for BenchmarkExecutor {
    fn name(&self) -> &str {
        &self.name
    }

    fn supported_extensions(&self) -> Vec<&str> {
        self.extensions.iter().map(|s| s.as_str()).collect()
    }

    async fn execute(&self, _request: ExecuteRequest) -> Result<ScriptOutput, ExecutionError> {
        if self.simulated_latency_us > 0 {
            tokio::time::sleep(std::time::Duration::from_micros(self.simulated_latency_us)).await;
        }
        Ok(ScriptOutput::default())
    }

    fn isolation_level(&self) -> IsolationLevel {
        IsolationLevel::None
    }
}

// ============================================================================
// BENCH-001: Executor Registration Performance
// ============================================================================

#[tokio::test]
async fn bench_executor_registration() {
    let thresholds = BenchmarkThresholds::default();
    let mut total_time_us = 0u64;
    let iterations = 100;

    for _ in 0..iterations {
        let mut manager = ScriptExecutorManager::default();
        let start = Instant::now();

        // Register multiple executors
        manager.register(Arc::new(BenchmarkExecutor::new("js", &["js", "ts", "mjs"])));
        manager.register(Arc::new(BenchmarkExecutor::new("py", &["py"])));
        manager.register(Arc::new(BenchmarkExecutor::new("sh", &["sh", "bash"])));
        manager.register(Arc::new(BenchmarkExecutor::new("rb", &["rb"])));
        manager.register(Arc::new(BenchmarkExecutor::new("go", &["go"])));

        total_time_us += start.elapsed().as_micros() as u64;
    }

    let avg_time_us = total_time_us / iterations;
    let avg_time_ms = avg_time_us as f64 / 1000.0;

    println!(
        "Executor registration (5 executors): avg {:.3}ms over {} iterations",
        avg_time_ms, iterations
    );

    assert!(
        avg_time_us / 1000 < thresholds.max_registration_time_ms,
        "Registration too slow: {:.3}ms (threshold: {}ms)",
        avg_time_ms,
        thresholds.max_registration_time_ms
    );
}

// ============================================================================
// BENCH-002: Extension Lookup Performance
// ============================================================================

#[tokio::test]
async fn bench_extension_lookup() {
    let mut manager = ScriptExecutorManager::default();

    // Register executors with many extensions
    for i in 0..20 {
        manager.register(Arc::new(BenchmarkExecutor::new(
            &format!("exec{}", i),
            &[Box::leak(format!("ext{}", i).into_boxed_str())],
        )));
    }

    let test_extensions = ["ext0", "ext5", "ext10", "ext15", "ext19"];
    let iterations = 10000;
    let start = Instant::now();

    for _ in 0..iterations {
        for ext in &test_extensions {
            let _ = manager.get_executor(ext);
        }
    }

    let duration = start.elapsed();
    let ops_per_sec = (iterations * test_extensions.len()) as f64 / duration.as_secs_f64();

    println!(
        "Extension lookup: {} ops in {:?} ({:.0} ops/sec)",
        iterations * test_extensions.len(),
        duration,
        ops_per_sec
    );

    // Should handle at least 100,000 lookups per second
    assert!(
        ops_per_sec > 100_000.0,
        "Extension lookup too slow: {:.0} ops/sec (threshold: 100,000)",
        ops_per_sec
    );
}

// ============================================================================
// BENCH-003: Script Execution Routing Performance
// ============================================================================

#[tokio::test]
async fn bench_script_execution_routing() {
    let thresholds = BenchmarkThresholds::default();
    let mut manager = ScriptExecutorManager::default();

    // Register fast mock executors
    manager.register(Arc::new(BenchmarkExecutor::new(
        "fast",
        &["js", "ts", "py", "sh"],
    )));

    let scripts: Vec<ScriptInfo> = ["test.js", "test.ts", "test.py", "test.sh"]
        .iter()
        .map(|name| ScriptInfo {
            name: name.to_string(),
            path: PathBuf::from(format!("/tmp/{}", name)),
            executable: true,
        })
        .collect();

    let iterations = 1000;
    let start = Instant::now();

    for _ in 0..iterations {
        for script in &scripts {
            let _ = manager.execute(script, vec![], HashMap::new(), None).await;
        }
    }

    let duration = start.elapsed();
    let total_ops = iterations * scripts.len();
    let ops_per_sec = total_ops as f64 / duration.as_secs_f64();

    println!(
        "Script routing: {} ops in {:?} ({:.0} ops/sec)",
        total_ops, duration, ops_per_sec
    );

    assert!(
        duration.as_millis() < thresholds.max_routing_1000_ms as u128 * 4,
        "Script routing too slow: {:?} (threshold: {}ms for 4000 ops)",
        duration,
        thresholds.max_routing_1000_ms * 4
    );
}

// ============================================================================
// BENCH-004: Concurrent Execution Performance
// ============================================================================

#[tokio::test]
async fn bench_concurrent_execution() {
    let thresholds = BenchmarkThresholds::default();
    let manager = Arc::new({
        let mut m = ScriptExecutorManager::default();
        // Executor with 1ms simulated latency
        m.register(Arc::new(
            BenchmarkExecutor::new("slow", &["slow"]).with_latency(1000),
        ));
        m
    });

    let num_tasks = 100;
    let scripts: Vec<ScriptInfo> = (0..num_tasks)
        .map(|i| ScriptInfo {
            name: format!("script{}.slow", i),
            path: PathBuf::from(format!("/tmp/script{}.slow", i)),
            executable: true,
        })
        .collect();

    let start = Instant::now();

    let handles: Vec<_> = scripts
        .into_iter()
        .map(|script| {
            let mgr = manager.clone();
            tokio::spawn(async move { mgr.execute(&script, vec![], HashMap::new(), None).await })
        })
        .collect();

    let mut success_count = 0;
    for handle in handles {
        if handle.await.unwrap().is_ok() {
            success_count += 1;
        }
    }

    let duration = start.elapsed();
    let concurrent_speedup = (num_tasks as f64 * 1.0) / duration.as_secs_f64(); // Expected 1ms each

    println!(
        "Concurrent execution: {} tasks in {:?} ({:.2}x speedup)",
        num_tasks, duration, concurrent_speedup
    );

    assert_eq!(success_count, num_tasks);
    assert!(
        duration.as_millis() < thresholds.max_concurrent_100_ms as u128,
        "Concurrent execution too slow: {:?} (threshold: {}ms)",
        duration,
        thresholds.max_concurrent_100_ms
    );
}

// ============================================================================
// BENCH-005: Memory Allocation Performance
// ============================================================================

#[tokio::test]
async fn bench_memory_allocation() {
    let iterations = 1000;
    let start = Instant::now();

    for _ in 0..iterations {
        // Simulate request creation and result handling
        let script = ScriptInfo {
            name: "test.js".to_string(),
            path: PathBuf::from("/tmp/test.js"),
            executable: true,
        };

        let _request = ExecuteRequest {
            script,
            args: vec!["--verbose".to_string(), "--output".to_string()],
            env: {
                let mut env = HashMap::new();
                env.insert("KEY1".to_string(), "value1".to_string());
                env.insert("KEY2".to_string(), "value2".to_string());
                env
            },
            limits: ResourceLimits::default(),
            working_dir: Some(PathBuf::from("/tmp")),
            stdin: None,
        };

        let _output = ScriptOutput {
            stdout: "Output content".repeat(10),
            stderr: "Error content".to_string(),
            exit_code: 0,
            duration: std::time::Duration::from_millis(100),
            ..Default::default()
        };
    }

    let duration = start.elapsed();
    let ops_per_sec = iterations as f64 / duration.as_secs_f64();

    println!(
        "Memory allocation: {} iterations in {:?} ({:.0} ops/sec)",
        iterations, duration, ops_per_sec
    );

    // Should handle at least 10,000 allocations per second
    assert!(
        ops_per_sec > 10_000.0,
        "Memory allocation too slow: {:.0} ops/sec",
        ops_per_sec
    );
}

// ============================================================================
// BENCH-006: Large Output Handling
// ============================================================================

#[tokio::test]
async fn bench_large_output_handling() {
    let mut manager = ScriptExecutorManager::default();

    /// Executor that produces large output
    struct LargeOutputExecutor {
        output_size: usize,
    }

    #[async_trait::async_trait]
    impl Executor for LargeOutputExecutor {
        fn name(&self) -> &str {
            "large-output"
        }
        fn supported_extensions(&self) -> Vec<&str> {
            vec!["large"]
        }
        async fn execute(&self, _: ExecuteRequest) -> Result<ScriptOutput, ExecutionError> {
            Ok(ScriptOutput {
                stdout: "x".repeat(self.output_size),
                ..Default::default()
            })
        }
        fn isolation_level(&self) -> IsolationLevel {
            IsolationLevel::None
        }
    }

    // Test with 1MB output
    manager.register(Arc::new(LargeOutputExecutor {
        output_size: 1024 * 1024,
    }));

    let script = ScriptInfo {
        name: "test.large".to_string(),
        path: PathBuf::from("/tmp/test.large"),
        executable: true,
    };

    let iterations = 10;
    let start = Instant::now();

    for _ in 0..iterations {
        let result = manager.execute(&script, vec![], HashMap::new(), None).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().stdout.len(), 1024 * 1024);
    }

    let duration = start.elapsed();
    let throughput_mb = (iterations * 1) as f64 / duration.as_secs_f64();

    println!(
        "Large output: {}MB in {:?} ({:.2} MB/sec)",
        iterations, duration, throughput_mb
    );

    // Should handle at least 10 MB/sec
    assert!(
        throughput_mb > 10.0,
        "Large output handling too slow: {:.2} MB/sec",
        throughput_mb
    );
}

// ============================================================================
// BENCH-007: Health Check Performance
// ============================================================================

#[tokio::test]
async fn bench_health_check() {
    let mut manager = ScriptExecutorManager::default();

    // Register multiple executors
    for i in 0..10 {
        manager.register(Arc::new(BenchmarkExecutor::new(
            &format!("exec{}", i),
            &[Box::leak(format!("ext{}", i).into_boxed_str())],
        )));
    }

    let iterations = 100;
    let start = Instant::now();

    for _ in 0..iterations {
        let _ = manager.health_check_all().await;
    }

    let duration = start.elapsed();
    let avg_time_ms = duration.as_millis() as f64 / iterations as f64;

    println!(
        "Health check (10 executors): avg {:.3}ms over {} iterations",
        avg_time_ms, iterations
    );

    // Health check should complete in under 10ms on average
    assert!(
        avg_time_ms < 10.0,
        "Health check too slow: {:.3}ms (threshold: 10ms)",
        avg_time_ms
    );
}

// ============================================================================
// BENCH-008: Deno Executor Startup (if available)
// ============================================================================

#[tokio::test]
async fn bench_deno_startup() {
    let iterations = 5;
    let mut startup_times = Vec::new();

    for _ in 0..iterations {
        let start = Instant::now();
        let result = DenoExecutor::with_defaults();
        let duration = start.elapsed();

        if result.is_ok() {
            startup_times.push(duration);
        }
    }

    if startup_times.is_empty() {
        println!("Deno startup benchmark: Deno not available, skipping");
        return;
    }

    let avg_time_ms: f64 = startup_times
        .iter()
        .map(|d| d.as_millis() as f64)
        .sum::<f64>()
        / startup_times.len() as f64;

    println!(
        "Deno startup: avg {:.2}ms over {} iterations",
        avg_time_ms,
        startup_times.len()
    );

    // Deno executor creation should be under 100ms
    assert!(
        avg_time_ms < 100.0,
        "Deno startup too slow: {:.2}ms (threshold: 100ms)",
        avg_time_ms
    );
}

// ============================================================================
// BENCH-009: Resource Limits Validation Performance
// ============================================================================

#[tokio::test]
async fn bench_resource_limits_validation() {
    let iterations = 10000;
    let start = Instant::now();

    for _ in 0..iterations {
        let limits = ResourceLimits {
            max_memory_bytes: 256 * 1024 * 1024,
            timeout: std::time::Duration::from_secs(30),
            max_output_bytes: 10 * 1024 * 1024,
            network_access: false,
            ..Default::default()
        };

        // Simulate limit checking
        let _ = limits.max_memory_bytes > 0;
        let _ = limits.timeout.as_secs() > 0;
        let _ = !limits.network_access;
    }

    let duration = start.elapsed();
    let ops_per_sec = iterations as f64 / duration.as_secs_f64();

    println!(
        "Resource limits validation: {} ops in {:?} ({:.0} ops/sec)",
        iterations, duration, ops_per_sec
    );

    // Should handle at least 1,000,000 validations per second
    assert!(
        ops_per_sec > 1_000_000.0,
        "Resource limits validation too slow: {:.0} ops/sec",
        ops_per_sec
    );
}

// ============================================================================
// BENCH-010: Complete Workflow Performance
// ============================================================================

#[tokio::test]
async fn bench_complete_workflow() {
    let manager = Arc::new({
        let mut m = ScriptExecutorManager::default();
        m.register(Arc::new(BenchmarkExecutor::new(
            "workflow",
            &["js", "ts", "py"],
        )));
        m
    });

    // Simulate realistic workflow: create request, execute, process result
    let iterations = 500;
    let start = Instant::now();

    for i in 0..iterations {
        let script = ScriptInfo {
            name: format!("task{}.js", i % 10),
            path: PathBuf::from(format!("/skills/task{}/scripts/run.js", i % 10)),
            executable: true,
        };

        let args = vec!["--task".to_string(), format!("task_{}", i)];
        let mut env = HashMap::new();
        env.insert("TASK_ID".to_string(), i.to_string());

        let result = manager.execute(&script, args, env, None).await;
        assert!(result.is_ok());

        // Simulate result processing
        let output = result.unwrap();
        let _ = output.exit_code == 0;
        let _ = output.stdout.len();
    }

    let duration = start.elapsed();
    let ops_per_sec = iterations as f64 / duration.as_secs_f64();
    let avg_latency_ms = duration.as_millis() as f64 / iterations as f64;

    println!(
        "Complete workflow: {} ops in {:?} ({:.0} ops/sec, {:.3}ms avg latency)",
        iterations, duration, ops_per_sec, avg_latency_ms
    );

    // Should handle at least 1000 complete workflows per second
    assert!(
        ops_per_sec > 1000.0,
        "Complete workflow too slow: {:.0} ops/sec (threshold: 1000)",
        ops_per_sec
    );
}

// ============================================================================
// BENCH-011: Extension Case Sensitivity Performance
// ============================================================================

#[tokio::test]
async fn bench_extension_case_handling() {
    let mut manager = ScriptExecutorManager::default();
    manager.register(Arc::new(BenchmarkExecutor::new("js", &["js"])));

    let extensions = ["js", "JS", "Js", "jS"];
    let iterations = 10000;
    let start = Instant::now();

    for _ in 0..iterations {
        for ext in &extensions {
            let _ = manager.supports(ext);
        }
    }

    let duration = start.elapsed();
    let ops_per_sec = (iterations * extensions.len()) as f64 / duration.as_secs_f64();

    println!(
        "Extension case handling: {} ops in {:?} ({:.0} ops/sec)",
        iterations * extensions.len(),
        duration,
        ops_per_sec
    );

    // Case-insensitive lookup should still be fast
    assert!(
        ops_per_sec > 500_000.0,
        "Extension case handling too slow: {:.0} ops/sec",
        ops_per_sec
    );
}

// ============================================================================
// Summary Report
// ============================================================================

#[tokio::test]
async fn bench_summary_report() {
    println!("\n========================================");
    println!("Executor Performance Benchmark Summary");
    println!("========================================");
    println!("All benchmarks measure worst-case scenarios");
    println!("with safety margins for CI environments.");
    println!("");
    println!("Key metrics:");
    println!("- Registration: < 10ms for 5 executors");
    println!("- Lookup: > 100,000 ops/sec");
    println!("- Routing: > 10,000 ops/sec");
    println!("- Concurrent: 100 tasks < 500ms");
    println!("- Health check: < 10ms for 10 executors");
    println!("========================================\n");
}
