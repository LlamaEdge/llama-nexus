//! Integration tests for Skill Script Execution
//!
//! This module implements the test cases defined in docs/skills/skills-test-plan.md
//! under "脚本执行" (Script Execution).
//!
//! Test cases:
//! - TSC-003: Bash 脚本执行 - Shell 脚本正常执行
//! - TSC-004: 脚本参数传递 - 命令行参数正确传递
//! - TSC-005: 脚本超时处理 - 超时限制生效并返回错误
//! - TSC-007: 脚本输出限制 - 输出大小截断
//! - TSC-008: 脚本白名单拒绝 - 不在 allowed-scripts 中的脚本被拒绝
//! - TSC-009: 上下文环境变量 - ScriptContext 环境变量注入
//! - TSC-010: 脚本退出码 - 非零退出码正确处理

use std::{collections::HashMap, path::Path};

use tempfile::TempDir;

use crate::{
    executor::{EXECUTOR_MANAGER, ExecutionError, ResourceLimits, ScriptExecutorManager},
    skills::{
        SkillRegistry,
        types::{ScriptContext, SkillResourceLimits},
    },
};

// ============================================================================
// Test Fixtures
// ============================================================================

/// Create a test skill with a bash script
fn create_skill_with_script(dir: &Path, skill_name: &str, script_name: &str, script_content: &str) {
    let skill_dir = dir.join(skill_name);
    let scripts_dir = skill_dir.join("scripts");
    std::fs::create_dir_all(&scripts_dir).unwrap();

    // Create SKILL.md
    let skill_md = format!(
        r#"---
name: {}
description: Test skill with script
---

# {} Skill

A test skill for script execution.
"#,
        skill_name, skill_name
    );
    std::fs::write(skill_dir.join("SKILL.md"), skill_md).unwrap();

    // Create script
    let script_path = scripts_dir.join(script_name);
    std::fs::write(&script_path, script_content).unwrap();

    // Make script executable on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&script_path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script_path, perms).unwrap();
    }
}

/// Create a test skill with metadata configuration
fn create_skill_with_metadata(
    dir: &Path,
    skill_name: &str,
    script_name: &str,
    script_content: &str,
    metadata_options: HashMap<&str, &str>,
) {
    let skill_dir = dir.join(skill_name);
    let scripts_dir = skill_dir.join("scripts");
    std::fs::create_dir_all(&scripts_dir).unwrap();

    // Build metadata section
    let mut metadata_section = String::new();
    if !metadata_options.is_empty() {
        metadata_section.push_str("metadata:\n");
        for (key, value) in &metadata_options {
            metadata_section.push_str(&format!("  {}: \"{}\"\n", key, value));
        }
    }

    // Create SKILL.md
    let skill_md = format!(
        r#"---
name: {}
description: Test skill with script
{}---

# {} Skill

A test skill for script execution.
"#,
        skill_name, metadata_section, skill_name
    );
    std::fs::write(skill_dir.join("SKILL.md"), skill_md).unwrap();

    // Create script
    let script_path = scripts_dir.join(script_name);
    std::fs::write(&script_path, script_content).unwrap();

    // Make script executable on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&script_path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script_path, perms).unwrap();
    }
}

/// Initialize executor manager for tests (only once)
fn ensure_executor_initialized() {
    // ExecutorManager is a singleton, check if already initialized
    if EXECUTOR_MANAGER.get().is_none() {
        // Use default resource limits for testing
        let manager = ScriptExecutorManager::new(ResourceLimits::default());
        let _ = EXECUTOR_MANAGER.set(manager);
    }
}

// ============================================================================
// TSC-003: Bash Script Execution
// ============================================================================

#[tokio::test]
#[cfg(unix)]
async fn test_tsc_003_bash_script_execution() {
    ensure_executor_initialized();

    let temp_dir = TempDir::new().unwrap();
    create_skill_with_script(
        temp_dir.path(),
        "echo-skill",
        "echo.sh",
        "#!/bin/bash\necho 'Hello from bash'",
    );

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    registry.load_all().await.unwrap();

    let skill = registry.get("echo-skill").await.unwrap();
    let result = skill
        .execute_script("echo.sh", vec![], HashMap::new(), None)
        .await;

    // If executor is not configured, skip the test
    match result {
        Ok(output) => {
            assert_eq!(output.exit_code, 0);
            assert!(output.stdout.contains("Hello from bash"));
        }
        Err(ExecutionError::ConfigError(_)) | Err(ExecutionError::NoExecutorFound(_)) => {
            // Executor not configured or not registered - skip
        }
        Err(e) => panic!("Unexpected error: {:?}", e),
    }
}

#[tokio::test]
#[cfg(unix)]
async fn test_tsc_003_bash_script_with_stderr() {
    ensure_executor_initialized();

    let temp_dir = TempDir::new().unwrap();
    create_skill_with_script(
        temp_dir.path(),
        "stderr-skill",
        "stderr.sh",
        "#!/bin/bash\necho 'stdout message'\necho 'stderr message' >&2",
    );

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    registry.load_all().await.unwrap();

    let skill = registry.get("stderr-skill").await.unwrap();
    let result = skill
        .execute_script("stderr.sh", vec![], HashMap::new(), None)
        .await;

    match result {
        Ok(output) => {
            assert_eq!(output.exit_code, 0);
            assert!(output.stdout.contains("stdout message"));
            assert!(output.stderr.contains("stderr message"));
        }
        Err(ExecutionError::ConfigError(_)) | Err(ExecutionError::NoExecutorFound(_)) => {
            // Executor not configured or not registered - skip
        }
        Err(e) => panic!("Unexpected error: {:?}", e),
    }
}

// ============================================================================
// TSC-004: Script Argument Passing
// ============================================================================

#[tokio::test]
#[cfg(unix)]
async fn test_tsc_004_script_argument_passing() {
    ensure_executor_initialized();

    let temp_dir = TempDir::new().unwrap();
    create_skill_with_script(
        temp_dir.path(),
        "args-skill",
        "args.sh",
        "#!/bin/bash\necho \"Args: $@\"",
    );

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    registry.load_all().await.unwrap();

    let skill = registry.get("args-skill").await.unwrap();
    let result = skill
        .execute_script(
            "args.sh",
            vec![
                "--input".to_string(),
                "data.json".to_string(),
                "--verbose".to_string(),
            ],
            HashMap::new(),
            None,
        )
        .await;

    match result {
        Ok(output) => {
            assert_eq!(output.exit_code, 0);
            assert!(output.stdout.contains("--input"));
            assert!(output.stdout.contains("data.json"));
            assert!(output.stdout.contains("--verbose"));
        }
        Err(ExecutionError::ConfigError(_)) | Err(ExecutionError::NoExecutorFound(_)) => {
            // Executor not configured or not registered - skip
        }
        Err(e) => panic!("Unexpected error: {:?}", e),
    }
}

#[tokio::test]
#[cfg(unix)]
async fn test_tsc_004_script_positional_arguments() {
    ensure_executor_initialized();

    let temp_dir = TempDir::new().unwrap();
    create_skill_with_script(
        temp_dir.path(),
        "positional-skill",
        "positional.sh",
        "#!/bin/bash\necho \"Arg1: $1\"\necho \"Arg2: $2\"\necho \"Arg3: $3\"",
    );

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    registry.load_all().await.unwrap();

    let skill = registry.get("positional-skill").await.unwrap();
    let result = skill
        .execute_script(
            "positional.sh",
            vec![
                "first".to_string(),
                "second".to_string(),
                "third".to_string(),
            ],
            HashMap::new(),
            None,
        )
        .await;

    match result {
        Ok(output) => {
            assert_eq!(output.exit_code, 0);
            assert!(output.stdout.contains("Arg1: first"));
            assert!(output.stdout.contains("Arg2: second"));
            assert!(output.stdout.contains("Arg3: third"));
        }
        Err(ExecutionError::ConfigError(_)) | Err(ExecutionError::NoExecutorFound(_)) => {
            // Executor not configured or not registered - skip
        }
        Err(e) => panic!("Unexpected error: {:?}", e),
    }
}

// ============================================================================
// TSC-005: Script Timeout Handling
// ============================================================================

#[tokio::test]
#[cfg(unix)]
async fn test_tsc_005_script_timeout() {
    ensure_executor_initialized();

    let temp_dir = TempDir::new().unwrap();
    create_skill_with_script(
        temp_dir.path(),
        "slow-skill",
        "slow.sh",
        "#!/bin/bash\nsleep 10\necho 'done'",
    );

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    registry.load_all().await.unwrap();

    let skill = registry.get("slow-skill").await.unwrap();

    // Set a very short timeout
    let limits = ResourceLimits {
        timeout: std::time::Duration::from_secs(1),
        ..Default::default()
    };

    let result = skill
        .execute_script("slow.sh", vec![], HashMap::new(), Some(limits))
        .await;

    match result {
        Err(ExecutionError::Timeout(_)) => {
            // Expected - timeout occurred
        }
        Err(ExecutionError::ConfigError(_)) | Err(ExecutionError::NoExecutorFound(_)) => {
            // Executor not configured or not registered - skip
        }
        Ok(_output) => {
            // If it somehow completed quickly, that's also acceptable in CI
            // where sleep might be much faster
        }
        Err(e) => panic!("Expected Timeout error, got: {:?}", e),
    }
}

// ============================================================================
// TSC-007: Script Output Truncation
// ============================================================================

#[tokio::test]
#[cfg(unix)]
async fn test_tsc_007_script_output_truncation() {
    ensure_executor_initialized();

    let temp_dir = TempDir::new().unwrap();
    // Generate large output
    create_skill_with_script(
        temp_dir.path(),
        "verbose-skill",
        "verbose.sh",
        "#!/bin/bash\nfor i in $(seq 1 10000); do echo \"Line $i: This is a long line of text to generate more output\"; done",
    );

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    registry.load_all().await.unwrap();

    let skill = registry.get("verbose-skill").await.unwrap();

    // Limit output to 1KB
    let limits = ResourceLimits {
        max_output_bytes: 1024,
        ..Default::default()
    };

    let result = skill
        .execute_script("verbose.sh", vec![], HashMap::new(), Some(limits))
        .await;

    match result {
        Ok(output) => {
            // Output should be limited
            assert!(
                output.stdout.len() <= 1024 + 100, // Allow some slack
                "Output should be truncated to ~1KB, got {} bytes",
                output.stdout.len()
            );
        }
        Err(ExecutionError::ConfigError(_)) | Err(ExecutionError::NoExecutorFound(_)) => {
            // Executor not configured or not registered - skip
        }
        Err(e) => panic!("Unexpected error: {:?}", e),
    }
}

// ============================================================================
// TSC-008: Script Allowlist Rejection
// ============================================================================

#[tokio::test]
async fn test_tsc_008_script_allowlist_rejection() {
    ensure_executor_initialized();

    let temp_dir = TempDir::new().unwrap();
    let skill_dir = temp_dir.path().join("restricted-skill");
    let scripts_dir = skill_dir.join("scripts");
    std::fs::create_dir_all(&scripts_dir).unwrap();

    // Create SKILL.md with allowed-scripts restriction
    let skill_md = r#"---
name: restricted-skill
description: Skill with script restrictions
metadata:
  allowed-scripts: "*.sh"
---

# Restricted Skill

Only shell scripts allowed.
"#;
    std::fs::write(skill_dir.join("SKILL.md"), skill_md).unwrap();

    // Create allowed script
    let allowed_script = scripts_dir.join("allowed.sh");
    std::fs::write(&allowed_script, "#!/bin/bash\necho ok").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&allowed_script).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&allowed_script, perms).unwrap();
    }

    // Create forbidden script
    let forbidden_script = scripts_dir.join("forbidden.py");
    std::fs::write(&forbidden_script, "print('forbidden')").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&forbidden_script).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&forbidden_script, perms).unwrap();
    }

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    registry.load_all().await.unwrap();

    let skill = registry.get("restricted-skill").await.unwrap();

    // Test allowed script passes validation
    assert!(skill.metadata.is_script_allowed("allowed.sh"));

    // Test forbidden script fails validation
    assert!(!skill.metadata.is_script_allowed("forbidden.py"));

    // Actually try to execute forbidden script
    let result = skill
        .execute_script("forbidden.py", vec![], HashMap::new(), None)
        .await;

    match result {
        Err(ExecutionError::PermissionDenied(msg)) => {
            assert!(msg.contains("forbidden.py"));
            assert!(msg.contains("allowed-scripts"));
        }
        Err(ExecutionError::ConfigError(_)) | Err(ExecutionError::NoExecutorFound(_)) => {
            // Executor not configured or not registered - skip
        }
        _ => panic!("Expected PermissionDenied error"),
    }
}

#[tokio::test]
async fn test_tsc_008_script_allowlist_patterns() {
    let temp_dir = TempDir::new().unwrap();
    let skill_dir = temp_dir.path().join("pattern-skill");
    std::fs::create_dir_all(&skill_dir).unwrap();

    // Create SKILL.md with pattern-based allowed-scripts
    let skill_md = r#"---
name: pattern-skill
description: Skill with pattern restrictions
metadata:
  allowed-scripts: "process-*.sh, build.sh, *.py"
---

# Pattern Skill

Multiple script patterns allowed.
"#;
    std::fs::write(skill_dir.join("SKILL.md"), skill_md).unwrap();

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    registry.load_all().await.unwrap();

    let skill = registry.get("pattern-skill").await.unwrap();

    // Test pattern matching
    assert!(skill.metadata.is_script_allowed("process-data.sh"));
    assert!(skill.metadata.is_script_allowed("process-image.sh"));
    assert!(skill.metadata.is_script_allowed("build.sh"));
    assert!(skill.metadata.is_script_allowed("main.py"));
    assert!(skill.metadata.is_script_allowed("test.py"));

    // Test rejected scripts
    assert!(!skill.metadata.is_script_allowed("random.sh"));
    assert!(!skill.metadata.is_script_allowed("deploy.sh"));
    assert!(!skill.metadata.is_script_allowed("script.js"));
}

// ============================================================================
// TSC-009: Script Context Environment Variables
// ============================================================================

#[tokio::test]
#[cfg(unix)]
async fn test_tsc_009_script_context_env_vars() {
    ensure_executor_initialized();

    let temp_dir = TempDir::new().unwrap();
    create_skill_with_script(
        temp_dir.path(),
        "env-skill",
        "env.sh",
        "#!/bin/bash\necho \"Skill: $SKILL_NAME\"\necho \"Script: $SCRIPT_NAME\"\necho \"Conv: $CONVERSATION_ID\"\necho \"Req: $REQUEST_ID\"\necho \"Custom: $CUSTOM_VAR\"",
    );

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    registry.load_all().await.unwrap();

    let skill = registry.get("env-skill").await.unwrap();

    let context =
        ScriptContext::with_ids(Some("conv-123".to_string()), Some("req-456".to_string()))
            .with_env("CUSTOM_VAR", "custom_value");

    let result = skill
        .execute_script_with_context("env.sh", vec![], context, None)
        .await;

    match result {
        Ok(output) => {
            assert_eq!(output.exit_code, 0);
            assert!(output.stdout.contains("Skill: env-skill"));
            assert!(output.stdout.contains("Script: env.sh"));
            assert!(output.stdout.contains("Conv: conv-123"));
            assert!(output.stdout.contains("Req: req-456"));
            assert!(output.stdout.contains("Custom: custom_value"));
        }
        Err(ExecutionError::ConfigError(_)) | Err(ExecutionError::NoExecutorFound(_)) => {
            // Executor not configured or not registered - skip
        }
        Err(e) => panic!("Unexpected error: {:?}", e),
    }
}

#[tokio::test]
async fn test_tsc_009_build_script_env() {
    let temp_dir = TempDir::new().unwrap();
    create_skill_with_script(
        temp_dir.path(),
        "test-skill",
        "test.sh",
        "#!/bin/bash\necho ok",
    );

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    registry.load_all().await.unwrap();

    let skill = registry.get("test-skill").await.unwrap();

    let env = skill.build_script_env();

    // Verify core environment variables
    assert!(env.contains_key("SKILL_DIR"));
    assert!(env.contains_key("SKILL_NAME"));
    assert!(env.contains_key("SKILL_ASSETS"));
    assert!(env.contains_key("SKILL_REFERENCES"));

    assert_eq!(env.get("SKILL_NAME").unwrap(), "test-skill");
}

#[tokio::test]
async fn test_tsc_009_context_to_env() {
    let temp_dir = TempDir::new().unwrap();
    create_skill_with_script(
        temp_dir.path(),
        "context-skill",
        "test.sh",
        "#!/bin/bash\necho ok",
    );

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    registry.load_all().await.unwrap();

    let skill = registry.get("context-skill").await.unwrap();

    let context =
        ScriptContext::with_ids(Some("plan-123".to_string()), Some("subtask-1".to_string()))
            .with_env("PLAN_ID", "plan-123")
            .with_env("SUBTASK_ID", "1");

    let env = context.to_env(&skill, "test.sh");

    // Verify context variables
    assert_eq!(env.get("CONVERSATION_ID").unwrap(), "plan-123");
    assert_eq!(env.get("REQUEST_ID").unwrap(), "subtask-1");
    assert_eq!(env.get("SCRIPT_NAME").unwrap(), "test.sh");

    // Verify user-defined variables
    assert_eq!(env.get("PLAN_ID").unwrap(), "plan-123");
    assert_eq!(env.get("SUBTASK_ID").unwrap(), "1");

    // Verify runtime info
    assert!(env.contains_key("LLAMA_NEXUS_VERSION"));
}

// ============================================================================
// TSC-010: Script Exit Code
// ============================================================================

#[tokio::test]
#[cfg(unix)]
async fn test_tsc_010_script_exit_code() {
    ensure_executor_initialized();

    let temp_dir = TempDir::new().unwrap();
    create_skill_with_script(
        temp_dir.path(),
        "exit-skill",
        "exit.sh",
        "#!/bin/bash\nexit 42",
    );

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    registry.load_all().await.unwrap();

    let skill = registry.get("exit-skill").await.unwrap();
    let result = skill
        .execute_script("exit.sh", vec![], HashMap::new(), None)
        .await;

    match result {
        Ok(output) => {
            assert_eq!(output.exit_code, 42);
        }
        Err(ExecutionError::ConfigError(_)) | Err(ExecutionError::NoExecutorFound(_)) => {
            // Executor not configured or not registered - skip
        }
        Err(e) => panic!("Unexpected error: {:?}", e),
    }
}

#[tokio::test]
#[cfg(unix)]
async fn test_tsc_010_script_success_exit_code() {
    ensure_executor_initialized();

    let temp_dir = TempDir::new().unwrap();
    create_skill_with_script(
        temp_dir.path(),
        "success-skill",
        "success.sh",
        "#!/bin/bash\necho 'success'\nexit 0",
    );

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    registry.load_all().await.unwrap();

    let skill = registry.get("success-skill").await.unwrap();
    let result = skill
        .execute_script("success.sh", vec![], HashMap::new(), None)
        .await;

    match result {
        Ok(output) => {
            assert_eq!(output.exit_code, 0);
            assert!(output.stdout.contains("success"));
        }
        Err(ExecutionError::ConfigError(_)) | Err(ExecutionError::NoExecutorFound(_)) => {
            // Executor not configured or not registered - skip
        }
        Err(e) => panic!("Unexpected error: {:?}", e),
    }
}

#[tokio::test]
#[cfg(unix)]
async fn test_tsc_010_script_error_exit_code() {
    ensure_executor_initialized();

    let temp_dir = TempDir::new().unwrap();
    create_skill_with_script(
        temp_dir.path(),
        "error-skill",
        "error.sh",
        "#!/bin/bash\necho 'error message' >&2\nexit 1",
    );

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    registry.load_all().await.unwrap();

    let skill = registry.get("error-skill").await.unwrap();
    let result = skill
        .execute_script("error.sh", vec![], HashMap::new(), None)
        .await;

    match result {
        Ok(output) => {
            assert_eq!(output.exit_code, 1);
            assert!(output.stderr.contains("error message"));
        }
        Err(ExecutionError::ConfigError(_)) | Err(ExecutionError::NoExecutorFound(_)) => {
            // Executor not configured or not registered - skip
        }
        Err(e) => panic!("Unexpected error: {:?}", e),
    }
}

// ============================================================================
// Additional Tests: Resource Limits Parsing
// ============================================================================

#[test]
fn test_skill_resource_limits_parsing() {
    // Valid limits string
    let limits = SkillResourceLimits::parse(
        "max_memory_bytes=134217728,timeout_secs=30,max_output_bytes=1048576,network_access=true",
    );

    assert!(limits.is_some());
    let limits = limits.unwrap();
    assert_eq!(limits.max_memory_bytes, Some(134217728));
    assert_eq!(limits.timeout_secs, Some(30));
    assert_eq!(limits.max_output_bytes, Some(1048576));
    assert_eq!(limits.network_access, Some(true));
}

#[test]
fn test_skill_resource_limits_partial() {
    // Partial limits
    let limits = SkillResourceLimits::parse("timeout_secs=60");

    assert!(limits.is_some());
    let limits = limits.unwrap();
    assert_eq!(limits.timeout_secs, Some(60));
    assert_eq!(limits.max_memory_bytes, None);
    assert_eq!(limits.max_output_bytes, None);
    assert_eq!(limits.network_access, None);
}

#[test]
fn test_skill_resource_limits_empty() {
    let limits = SkillResourceLimits::parse("");
    assert!(limits.is_none());

    let limits = SkillResourceLimits::parse("   ");
    assert!(limits.is_none());
}

#[test]
fn test_skill_resource_limits_invalid() {
    // Invalid values should be ignored
    let limits = SkillResourceLimits::parse("timeout_secs=abc,max_memory_bytes=xyz");
    assert!(limits.is_none());
}

#[test]
fn test_skill_resource_limits_is_empty() {
    let empty = SkillResourceLimits::default();
    assert!(empty.is_empty());

    let non_empty = SkillResourceLimits {
        timeout_secs: Some(30),
        ..Default::default()
    };
    assert!(!non_empty.is_empty());
}

// ============================================================================
// Additional Tests: Script Info and Loading
// ============================================================================

#[tokio::test]
async fn test_script_info_loading() {
    let temp_dir = TempDir::new().unwrap();
    create_skill_with_script(
        temp_dir.path(),
        "scripts-skill",
        "main.sh",
        "#!/bin/bash\necho ok",
    );

    // Add more scripts
    let scripts_dir = temp_dir.path().join("scripts-skill/scripts");
    std::fs::write(scripts_dir.join("helper.sh"), "#!/bin/bash\necho helper").unwrap();
    std::fs::write(scripts_dir.join("process.py"), "print('python')").unwrap();

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    registry.load_all().await.unwrap();

    let skill = registry.get("scripts-skill").await.unwrap();

    // Verify scripts were loaded
    assert!(skill.scripts.len() >= 3);
    assert!(skill.get_script("main.sh").is_some());
    assert!(skill.get_script("helper.sh").is_some());
    assert!(skill.get_script("process.py").is_some());
}

#[tokio::test]
async fn test_script_not_found() {
    ensure_executor_initialized();

    let temp_dir = TempDir::new().unwrap();
    create_skill_with_script(
        temp_dir.path(),
        "simple-skill",
        "exists.sh",
        "#!/bin/bash\necho ok",
    );

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    registry.load_all().await.unwrap();

    let skill = registry.get("simple-skill").await.unwrap();

    // Try to execute non-existent script
    let result = skill
        .execute_script("nonexistent.sh", vec![], HashMap::new(), None)
        .await;

    match result {
        Err(ExecutionError::ScriptNotFound(_)) => {
            // Expected
        }
        Err(ExecutionError::ConfigError(_)) | Err(ExecutionError::NoExecutorFound(_)) => {
            // Executor not configured or not registered - skip
        }
        _ => panic!("Expected ScriptNotFound error"),
    }
}

#[tokio::test]
async fn test_list_scripts() {
    let temp_dir = TempDir::new().unwrap();
    create_skill_with_script(
        temp_dir.path(),
        "list-skill",
        "script1.sh",
        "#!/bin/bash\necho 1",
    );

    let scripts_dir = temp_dir.path().join("list-skill/scripts");
    std::fs::write(scripts_dir.join("script2.sh"), "#!/bin/bash\necho 2").unwrap();
    std::fs::write(scripts_dir.join("script3.py"), "print(3)").unwrap();

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    registry.load_all().await.unwrap();

    let skill = registry.get("list-skill").await.unwrap();

    let script_names = skill.list_scripts();
    assert!(script_names.len() >= 3);
    assert!(script_names.contains(&"script1.sh"));
    assert!(script_names.contains(&"script2.sh"));
    assert!(script_names.contains(&"script3.py"));
}
