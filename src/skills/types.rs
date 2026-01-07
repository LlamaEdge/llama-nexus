//! Type definitions for Skills module
//!
//! Compliant with [Agent Skills Standard](https://agentskills.io/specification)

use std::{collections::HashMap, path::PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::executor::{EXECUTOR_MANAGER, ExecutionError, ResourceLimits, ScriptOutput};

/// Skill metadata from YAML front matter
///
/// Fields follow the Agent Skills Standard specification:
/// - Required: name, description
/// - Optional: license, compatibility, metadata, allowed-tools
/// - Extension: model (for Claude Code compatibility)
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SkillMetadata {
    /// Skill name (1-64 chars, lowercase letters/numbers/hyphens)
    /// Must match the parent directory name
    pub name: String,

    /// Skill description (1-1024 chars)
    /// Should explain when and how to use the skill
    pub description: String,

    /// License information (optional)
    #[serde(default)]
    pub license: Option<String>,

    /// Compatibility requirements (optional, ≤500 chars)
    /// e.g., system packages, network access requirements
    #[serde(default)]
    pub compatibility: Option<String>,

    /// Additional metadata as key-value pairs (optional)
    #[serde(default)]
    pub metadata: Option<HashMap<String, String>>,

    /// Pre-approved tools (optional, space-separated list)
    /// Experimental field per Agent Skills Standard
    #[serde(rename = "allowed-tools", default)]
    pub allowed_tools: Option<String>,

    /// Model recommendation (optional, Claude Code extension)
    /// Not part of the standard, for compatibility
    #[serde(default)]
    pub model: Option<String>,

    /// Allowed scripts for execution (optional, extension field)
    ///
    /// Controls which scripts from the scripts/ directory can be executed.
    /// Supports glob patterns (e.g., "*.js", "process-*.py").
    ///
    /// - If None or empty: all scripts in scripts/ are allowed (default permissive)
    /// - If Some with patterns: only matching scripts are allowed
    ///
    /// Examples:
    /// - `["*.js", "*.ts"]` - allow all JavaScript and TypeScript files
    /// - `["process.py", "export.py"]` - allow only specific files
    /// - `["data-*.js"]` - allow files matching pattern
    #[serde(rename = "allowed-scripts", default)]
    pub allowed_scripts: Option<Vec<String>>,

    /// Execution resource limits for this skill (optional, extension field)
    ///
    /// Overrides global default limits for scripts executed by this skill.
    /// Any field not specified inherits from the global default.
    #[serde(rename = "execution-limits", default)]
    pub execution_limits: Option<SkillResourceLimits>,
}

/// Skill-specific resource limits configuration
///
/// Allows skills to override global execution limits.
/// Fields are optional - unset fields inherit from global defaults.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct SkillResourceLimits {
    /// Maximum memory in bytes
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_memory_bytes: Option<u64>,

    /// Maximum execution time in seconds
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,

    /// Maximum output size in bytes
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_bytes: Option<u64>,

    /// Whether network access is allowed
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_access: Option<bool>,
}

impl SkillResourceLimits {
    /// Merge skill-specific limits with global defaults
    ///
    /// Returns a `ResourceLimits` instance where skill-specific values
    /// override the corresponding global defaults.
    ///
    /// # Arguments
    /// * `global` - The global default resource limits
    ///
    /// # Returns
    /// A new `ResourceLimits` with merged values
    pub fn merge_with(&self, global: &ResourceLimits) -> ResourceLimits {
        use std::time::Duration;

        ResourceLimits {
            max_memory_bytes: self.max_memory_bytes.unwrap_or(global.max_memory_bytes),
            timeout: self
                .timeout_secs
                .map(Duration::from_secs)
                .unwrap_or(global.timeout),
            max_output_bytes: self.max_output_bytes.unwrap_or(global.max_output_bytes),
            network_access: self.network_access.unwrap_or(global.network_access),
            filesystem_access: global.filesystem_access.clone(),
        }
    }

    /// Check if any limits are specified
    pub fn is_empty(&self) -> bool {
        self.max_memory_bytes.is_none()
            && self.timeout_secs.is_none()
            && self.max_output_bytes.is_none()
            && self.network_access.is_none()
    }
}

impl SkillMetadata {
    /// Check if a script is allowed to be executed by this skill
    ///
    /// # Arguments
    /// * `script_name` - The name of the script file (e.g., "process.js")
    ///
    /// # Returns
    /// * `true` if the script is allowed (matches patterns or no restrictions)
    /// * `false` if the script is explicitly disallowed
    ///
    /// # Pattern matching
    /// - If `allowed_scripts` is None or empty, all scripts are allowed
    /// - If `allowed_scripts` contains patterns, the script must match at least one
    /// - Supports glob patterns: `*` (any chars), `?` (single char)
    pub fn is_script_allowed(&self, script_name: &str) -> bool {
        match &self.allowed_scripts {
            None => true,                                  // No restrictions - allow all
            Some(patterns) if patterns.is_empty() => true, // Empty list - allow all
            Some(patterns) => {
                // Check if script matches any pattern
                patterns.iter().any(|pattern| {
                    if pattern.contains('*') || pattern.contains('?') {
                        // Glob pattern matching
                        Self::glob_match(pattern, script_name)
                    } else {
                        // Exact match
                        pattern == script_name
                    }
                })
            }
        }
    }

    /// Get the list of allowed script patterns
    ///
    /// Returns None if all scripts are allowed, Some with patterns otherwise.
    pub fn get_allowed_scripts(&self) -> Option<&[String]> {
        self.allowed_scripts
            .as_ref()
            .filter(|v| !v.is_empty())
            .map(|v| v.as_slice())
    }

    /// Simple glob pattern matching using dynamic programming
    ///
    /// Supports:
    /// - `*` matches any sequence of characters (including empty)
    /// - `?` matches exactly one character
    fn glob_match(pattern: &str, text: &str) -> bool {
        let p: Vec<char> = pattern.chars().collect();
        let t: Vec<char> = text.chars().collect();
        let (m, n) = (p.len(), t.len());

        // dp[i][j] = true if pattern[0..i] matches text[0..j]
        let mut dp = vec![vec![false; n + 1]; m + 1];

        // Empty pattern matches empty text
        dp[0][0] = true;

        // Handle patterns starting with *
        for i in 1..=m {
            if p[i - 1] == '*' {
                dp[i][0] = dp[i - 1][0];
            } else {
                break;
            }
        }

        // Fill the DP table
        for i in 1..=m {
            for j in 1..=n {
                if p[i - 1] == '*' {
                    // * can match empty (dp[i-1][j]) or match one more char (dp[i][j-1])
                    dp[i][j] = dp[i - 1][j] || dp[i][j - 1];
                } else if p[i - 1] == '?' || p[i - 1] == t[j - 1] {
                    // ? matches any single char, or exact char match
                    dp[i][j] = dp[i - 1][j - 1];
                }
            }
        }

        dp[m][n]
    }
}

impl SkillMetadata {
    /// Parse allowed-tools string into a list of tool names
    ///
    /// Supports either space-separated or comma-separated formats (not mixed):
    /// - "tool1 tool2 tool3" (space-separated, per Agent Skills Standard)
    /// - "tool1, tool2, tool3" (comma-separated)
    ///
    /// Detection logic: if the string contains a comma, use comma as delimiter;
    /// otherwise use whitespace.
    pub fn get_allowed_tools(&self) -> Vec<String> {
        self.allowed_tools
            .as_ref()
            .map(|s| {
                if s.contains(',') {
                    // Comma-separated format
                    s.split(',')
                        .map(|t| t.trim().to_string())
                        .filter(|t| !t.is_empty())
                        .collect()
                } else {
                    // Space-separated format (Agent Skills Standard)
                    s.split_whitespace().map(|t| t.to_string()).collect()
                }
            })
            .unwrap_or_default()
    }
}

/// A fully loaded skill with content
#[derive(Debug, Clone)]
pub struct LoadedSkill {
    /// Parsed metadata from YAML front matter
    pub metadata: SkillMetadata,

    /// Markdown content (after front matter)
    pub content: String,

    /// Raw file content (for debugging)
    #[allow(dead_code)]
    pub raw_content: String,

    /// Directory containing the skill
    pub skill_dir: PathBuf,

    /// Path to SKILL.md file
    #[allow(dead_code)]
    pub file_path: String,

    /// Whether this skill is enabled
    pub enabled: bool,

    /// When this skill was loaded
    #[allow(dead_code)]
    pub loaded_at: DateTime<Utc>,

    /// Available scripts from the scripts/ directory
    pub scripts: Vec<ScriptInfo>,
}

impl LoadedSkill {
    /// Get a script by name
    ///
    /// Returns the script info if found, or None if the script doesn't exist.
    pub fn get_script(&self, script_name: &str) -> Option<&ScriptInfo> {
        self.scripts.iter().find(|s| s.name == script_name)
    }

    /// Check if a script exists in this skill
    pub fn has_script(&self, script_name: &str) -> bool {
        self.scripts.iter().any(|s| s.name == script_name)
    }

    /// Get the path to the assets directory for this skill
    pub fn assets_dir(&self) -> PathBuf {
        self.skill_dir.join("assets")
    }

    /// Get the path to the references directory for this skill
    pub fn references_dir(&self) -> PathBuf {
        self.skill_dir.join("references")
    }

    /// Get the path to the scripts directory for this skill
    pub fn scripts_dir(&self) -> PathBuf {
        self.skill_dir.join("scripts")
    }

    /// Build environment variables for script execution
    ///
    /// Creates the standard set of environment variables passed to scripts:
    /// - SKILL_DIR: Absolute path to the skill directory
    /// - SKILL_NAME: Name of the skill
    /// - SKILL_ASSETS: Path to the assets directory
    /// - SKILL_REFERENCES: Path to the references directory
    ///
    /// Additional variables can be merged with the returned map.
    pub fn build_script_env(&self) -> HashMap<String, String> {
        let mut env = HashMap::new();

        // Core environment variables
        env.insert(
            "SKILL_DIR".to_string(),
            self.skill_dir.to_string_lossy().to_string(),
        );
        env.insert("SKILL_NAME".to_string(), self.metadata.name.clone());

        // Derived paths
        env.insert(
            "SKILL_ASSETS".to_string(),
            self.assets_dir().to_string_lossy().to_string(),
        );
        env.insert(
            "SKILL_REFERENCES".to_string(),
            self.references_dir().to_string_lossy().to_string(),
        );

        env
    }

    /// Execute a script from this skill
    ///
    /// # Arguments
    /// * `script_name` - Name of the script file (e.g., "process.js")
    /// * `args` - Command line arguments to pass to the script
    /// * `env` - Additional environment variables (merged with skill env)
    /// * `limits` - Optional resource limits (overrides skill-level limits)
    ///
    /// # Returns
    /// * `Ok(ScriptOutput)` - Execution result including stdout, stderr, exit code
    /// * `Err(ExecutionError)` - If script not found, permission denied, no executor available, or execution fails
    ///
    /// # Resource Limits Priority (highest to lowest)
    /// 1. `limits` parameter (if provided)
    /// 2. Skill-level `execution-limits` from SKILL.md
    /// 3. Global default limits from ExecutorManager
    ///
    /// # Example
    /// ```rust,ignore
    /// let output = skill.execute_script(
    ///     "process.js",
    ///     vec!["--input".to_string(), "data.json".to_string()],
    ///     HashMap::new(),
    ///     None,
    /// ).await?;
    /// ```
    pub async fn execute_script(
        &self,
        script_name: &str,
        args: Vec<String>,
        additional_env: HashMap<String, String>,
        limits: Option<ResourceLimits>,
    ) -> Result<ScriptOutput, ExecutionError> {
        // Check if script is allowed by the skill's allowed_scripts configuration
        if !self.metadata.is_script_allowed(script_name) {
            return Err(ExecutionError::PermissionDenied(format!(
                "script '{}' is not in the allowed-scripts list for skill '{}'",
                script_name, self.metadata.name
            )));
        }

        // Get the script
        let script = self
            .get_script(script_name)
            .ok_or_else(|| ExecutionError::ScriptNotFound(self.scripts_dir().join(script_name)))?;

        // Get the global executor manager
        let manager = EXECUTOR_MANAGER.get().ok_or_else(|| {
            ExecutionError::ConfigError("Executor manager not initialized".to_string())
        })?;

        // Build environment with skill context
        let mut env = self.build_script_env();
        env.insert("SCRIPT_NAME".to_string(), script_name.to_string());
        env.extend(additional_env);

        // Resolve resource limits: request > skill > global
        let resolved_limits = limits.or_else(|| self.resolve_resource_limits());

        // Execute the script
        manager.execute(script, args, env, resolved_limits).await
    }

    /// Resolve resource limits for script execution
    ///
    /// Returns skill-level limits merged with global defaults,
    /// or None to use global defaults directly.
    fn resolve_resource_limits(&self) -> Option<ResourceLimits> {
        self.metadata
            .execution_limits
            .as_ref()
            .and_then(|skill_limits| {
                // Only create merged limits if skill has custom limits
                if skill_limits.is_empty() {
                    return None;
                }

                // Get global defaults from executor manager
                EXECUTOR_MANAGER.get().map(|manager| {
                    // Access the default limits from manager
                    // For now, use ResourceLimits::default() as the base
                    // In production, this should come from manager's configured defaults
                    skill_limits.merge_with(&ResourceLimits::default())
                })
            })
    }

    /// List all available scripts in this skill
    pub fn list_scripts(&self) -> Vec<&str> {
        self.scripts.iter().map(|s| s.name.as_str()).collect()
    }

    /// Check if the executor manager supports a given script
    ///
    /// Returns true if there's an executor registered for the script's file extension.
    pub fn is_script_supported(&self, script_name: &str) -> bool {
        if let Some(script) = self.get_script(script_name) {
            if let Some(manager) = EXECUTOR_MANAGER.get() {
                if let Some(ext) = script.path.extension().and_then(|e| e.to_str()) {
                    return manager.supports(ext);
                }
            }
        }
        false
    }

    /// Execute a script with context
    ///
    /// This is the recommended method for executing scripts from tool handlers,
    /// as it properly handles context information like conversation and request IDs.
    ///
    /// # Arguments
    /// * `script_name` - Name of the script file (e.g., "process.js")
    /// * `args` - Command line arguments to pass to the script
    /// * `context` - Execution context including conversation ID and custom env vars
    /// * `limits` - Optional resource limits (overrides skill-level limits)
    ///
    /// # Returns
    /// * `Ok(ScriptOutput)` - Execution result including stdout, stderr, exit code
    /// * `Err(ExecutionError)` - If script not found, permission denied, or execution fails
    ///
    /// # Example
    /// ```rust,ignore
    /// let context = ScriptContext::with_ids(Some(conv_id), Some(req_id));
    /// let output = skill.execute_script_with_context(
    ///     "process.js",
    ///     vec!["--input".to_string(), "data.json".to_string()],
    ///     context,
    ///     None,
    /// ).await?;
    /// ```
    pub async fn execute_script_with_context(
        &self,
        script_name: &str,
        args: Vec<String>,
        context: ScriptContext,
        limits: Option<ResourceLimits>,
    ) -> Result<ScriptOutput, ExecutionError> {
        // Build environment from context
        let env = context.to_env(self, script_name);

        // Delegate to execute_script with empty additional_env since context already has everything
        self.execute_script(script_name, args, env, limits).await
    }
}

/// Skill summary for phase 1 injection
///
/// Contains name, description, and allowed tools for tool filtering
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillSummary {
    /// Skill name
    pub name: String,

    /// Skill description
    pub description: String,

    /// Tools covered by this skill (should be hidden in Phase 1)
    #[serde(default)]
    pub allowed_tools: Vec<String>,
}

impl From<&LoadedSkill> for SkillSummary {
    fn from(skill: &LoadedSkill) -> Self {
        Self {
            name: skill.metadata.name.clone(),
            description: skill.metadata.description.clone(),
            allowed_tools: skill.metadata.get_allowed_tools(),
        }
    }
}

impl From<&SkillMetadata> for SkillSummary {
    fn from(metadata: &SkillMetadata) -> Self {
        Self {
            name: metadata.name.clone(),
            description: metadata.description.clone(),
            allowed_tools: metadata.get_allowed_tools(),
        }
    }
}

/// Script information from the scripts/ directory
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ScriptInfo {
    /// Script filename
    pub name: String,

    /// Full path to the script
    pub path: PathBuf,

    /// Whether the script is executable
    pub executable: bool,
}

/// Context for script execution
///
/// Provides execution context including conversation tracking and user-defined
/// environment variables. Used when executing scripts via the `skill_run_script` tool.
///
/// # Environment Variables
///
/// When converted to environment variables via `to_env()`, includes:
///
/// ## Core variables (automatically set):
/// - `SKILL_DIR`: Absolute path to the skill directory
/// - `SKILL_NAME`: Name of the skill
/// - `SCRIPT_NAME`: Name of the script being executed
///
/// ## Derived paths:
/// - `SKILL_ASSETS`: Path to the assets directory
/// - `SKILL_REFERENCES`: Path to the references directory
///
/// ## Optional context:
/// - `CONVERSATION_ID`: Conversation session ID (if provided)
/// - `REQUEST_ID`: Current request ID (if provided)
///
/// ## Runtime info:
/// - `LLAMA_NEXUS_VERSION`: Server version
///
/// ## User-defined:
/// - Any additional variables from `user_env`
#[derive(Debug, Clone, Default)]
pub struct ScriptContext {
    /// Conversation session ID for tracking
    pub conversation_id: Option<String>,

    /// Current request ID for tracking
    pub request_id: Option<String>,

    /// User-defined environment variables
    ///
    /// These are merged with the automatically generated variables,
    /// with user-defined values taking precedence.
    pub user_env: HashMap<String, String>,
}

impl ScriptContext {
    /// Creates a new ScriptContext with default values
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a ScriptContext with conversation and request IDs
    pub fn with_ids(conversation_id: Option<String>, request_id: Option<String>) -> Self {
        Self {
            conversation_id,
            request_id,
            user_env: HashMap::new(),
        }
    }

    /// Adds a user-defined environment variable
    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.user_env.insert(key.into(), value.into());
        self
    }

    /// Converts the context to environment variables for script execution
    ///
    /// This method is called internally by `LoadedSkill::execute_script_with_context()`
    /// to build the complete environment variable map.
    ///
    /// # Arguments
    /// * `skill` - The skill containing the script
    /// * `script_name` - Name of the script being executed
    ///
    /// # Returns
    /// A HashMap of environment variables to pass to the script
    pub fn to_env(&self, skill: &LoadedSkill, script_name: &str) -> HashMap<String, String> {
        // Start with skill's base environment
        let mut env = skill.build_script_env();

        // Add script name
        env.insert("SCRIPT_NAME".to_string(), script_name.to_string());

        // Add optional context variables
        if let Some(id) = &self.conversation_id {
            env.insert("CONVERSATION_ID".to_string(), id.clone());
        }
        if let Some(id) = &self.request_id {
            env.insert("REQUEST_ID".to_string(), id.clone());
        }

        // Add runtime info
        env.insert(
            "LLAMA_NEXUS_VERSION".to_string(),
            env!("CARGO_PKG_VERSION").to_string(),
        );

        // Merge user-defined environment variables (takes precedence)
        env.extend(self.user_env.clone());

        env
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to create a minimal SkillMetadata for testing
    fn test_metadata(name: &str, description: &str) -> SkillMetadata {
        SkillMetadata {
            name: name.to_string(),
            description: description.to_string(),
            license: None,
            compatibility: None,
            metadata: None,
            allowed_tools: None,
            model: None,
            allowed_scripts: None,
            execution_limits: None,
        }
    }

    #[test]
    fn test_get_allowed_tools_space_separated() {
        let mut metadata = test_metadata("test", "test");
        metadata.allowed_tools = Some("tool1 tool2 tool3".to_string());

        let tools = metadata.get_allowed_tools();
        assert_eq!(tools, vec!["tool1", "tool2", "tool3"]);
    }

    #[test]
    fn test_get_allowed_tools_empty() {
        let metadata = test_metadata("test", "test");

        let tools = metadata.get_allowed_tools();
        assert!(tools.is_empty());
    }

    #[test]
    fn test_get_allowed_tools_with_extra_whitespace() {
        let mut metadata = test_metadata("test", "test");
        metadata.allowed_tools = Some("  tool1   tool2  ".to_string());

        let tools = metadata.get_allowed_tools();
        assert_eq!(tools, vec!["tool1", "tool2"]);
    }

    #[test]
    fn test_skill_summary_from_loaded_skill() {
        let skill = LoadedSkill {
            metadata: test_metadata("weather-query", "Query weather information"),
            content: "# Weather Query".to_string(),
            raw_content: "---\nname: weather-query\n---\n# Weather Query".to_string(),
            skill_dir: PathBuf::from("/skills/weather-query"),
            file_path: "/skills/weather-query/SKILL.md".to_string(),
            enabled: true,
            loaded_at: Utc::now(),
            scripts: Vec::new(),
        };

        let summary = SkillSummary::from(&skill);
        assert_eq!(summary.name, "weather-query");
        assert_eq!(summary.description, "Query weather information");
    }

    #[test]
    fn test_skill_summary_from_metadata() {
        let mut metadata = test_metadata("code-review", "Review code for best practices");
        metadata.license = Some("MIT".to_string());

        let summary = SkillSummary::from(&metadata);
        assert_eq!(summary.name, "code-review");
        assert_eq!(summary.description, "Review code for best practices");
    }

    #[test]
    fn test_skill_metadata_serialization() {
        let mut metadata = test_metadata("test-skill", "A test skill");
        metadata.license = Some("Apache-2.0".to_string());
        metadata.compatibility = Some("Requires network access".to_string());
        metadata.metadata = Some(HashMap::from([
            ("author".to_string(), "test".to_string()),
            ("version".to_string(), "1.0".to_string()),
        ]));
        metadata.allowed_tools = Some("Bash Read Write".to_string());
        metadata.model = Some("claude-sonnet".to_string());

        let json = serde_json::to_string(&metadata).unwrap();
        let deserialized: SkillMetadata = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.name, "test-skill");
        assert_eq!(deserialized.description, "A test skill");
        assert_eq!(deserialized.license, Some("Apache-2.0".to_string()));
        assert_eq!(
            deserialized.compatibility,
            Some("Requires network access".to_string())
        );
        assert_eq!(
            deserialized.metadata.as_ref().unwrap().get("author"),
            Some(&"test".to_string())
        );
        assert_eq!(
            deserialized.allowed_tools,
            Some("Bash Read Write".to_string())
        );
        assert_eq!(deserialized.model, Some("claude-sonnet".to_string()));
    }

    #[test]
    fn test_skill_metadata_deserialization_with_defaults() {
        let json = r#"{"name": "minimal", "description": "Minimal skill"}"#;
        let metadata: SkillMetadata = serde_json::from_str(json).unwrap();

        assert_eq!(metadata.name, "minimal");
        assert_eq!(metadata.description, "Minimal skill");
        assert!(metadata.license.is_none());
        assert!(metadata.compatibility.is_none());
        assert!(metadata.metadata.is_none());
        assert!(metadata.allowed_tools.is_none());
        assert!(metadata.model.is_none());
        assert!(metadata.allowed_scripts.is_none());
        assert!(metadata.execution_limits.is_none());
    }

    #[test]
    fn test_skill_summary_serialization() {
        let summary = SkillSummary {
            name: "git-commit".to_string(),
            description: "Create git commits".to_string(),
            allowed_tools: vec!["Bash".to_string(), "Read".to_string()],
        };

        let json = serde_json::to_string(&summary).unwrap();
        assert!(json.contains("git-commit"));
        assert!(json.contains("Create git commits"));
        assert!(json.contains("allowed_tools"));

        let deserialized: SkillSummary = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.name, "git-commit");
        assert_eq!(deserialized.description, "Create git commits");
        assert_eq!(deserialized.allowed_tools, vec!["Bash", "Read"]);
    }

    #[test]
    fn test_skill_summary_deserialization_without_allowed_tools() {
        // Test backward compatibility: allowed_tools defaults to empty vec
        let json = r#"{"name": "old-skill", "description": "Old skill without allowed_tools"}"#;
        let summary: SkillSummary = serde_json::from_str(json).unwrap();

        assert_eq!(summary.name, "old-skill");
        assert_eq!(summary.description, "Old skill without allowed_tools");
        assert!(summary.allowed_tools.is_empty());
    }

    #[test]
    fn test_get_allowed_tools_single_tool() {
        let mut metadata = test_metadata("test", "test");
        metadata.allowed_tools = Some("Bash".to_string());

        let tools = metadata.get_allowed_tools();
        assert_eq!(tools, vec!["Bash"]);
    }

    #[test]
    fn test_get_allowed_tools_with_wildcards() {
        let mut metadata = test_metadata("test", "test");
        metadata.allowed_tools = Some("Bash(git:*) Read Write".to_string());

        let tools = metadata.get_allowed_tools();
        assert_eq!(tools, vec!["Bash(git:*)", "Read", "Write"]);
    }

    #[test]
    fn test_get_allowed_tools_empty_string() {
        let mut metadata = test_metadata("test", "test");
        metadata.allowed_tools = Some("".to_string());

        let tools = metadata.get_allowed_tools();
        assert!(tools.is_empty());
    }

    #[test]
    fn test_get_allowed_tools_whitespace_only() {
        let mut metadata = test_metadata("test", "test");
        metadata.allowed_tools = Some("   \t\n  ".to_string());

        let tools = metadata.get_allowed_tools();
        assert!(tools.is_empty());
    }

    #[test]
    fn test_get_allowed_tools_comma_separated() {
        let mut metadata = test_metadata("test", "test");
        metadata.allowed_tools = Some("tool1, tool2, tool3".to_string());

        let tools = metadata.get_allowed_tools();
        assert_eq!(tools, vec!["tool1", "tool2", "tool3"]);
    }

    #[test]
    fn test_get_allowed_tools_comma_no_space() {
        let mut metadata = test_metadata("test", "test");
        metadata.allowed_tools = Some("tool1,tool2,tool3".to_string());

        let tools = metadata.get_allowed_tools();
        assert_eq!(tools, vec!["tool1", "tool2", "tool3"]);
    }

    #[test]
    fn test_get_allowed_tools_comma_extra_whitespace() {
        let mut metadata = test_metadata("test", "test");
        metadata.allowed_tools = Some("  tool1 ,  tool2  ,  tool3  ".to_string());

        let tools = metadata.get_allowed_tools();
        assert_eq!(tools, vec!["tool1", "tool2", "tool3"]);
    }

    #[test]
    fn test_get_allowed_tools_mcp_tool_names_comma() {
        let mut metadata = test_metadata("test", "test");
        metadata.allowed_tools =
            Some("mcp__cardea-calculator__sum, mcp__cardea-calculator__sub".to_string());

        let tools = metadata.get_allowed_tools();
        assert_eq!(
            tools,
            vec!["mcp__cardea-calculator__sum", "mcp__cardea-calculator__sub"]
        );
    }

    #[test]
    fn test_get_allowed_tools_mcp_tool_names_space() {
        let mut metadata = test_metadata("test", "test");
        metadata.allowed_tools =
            Some("mcp__cardea-calculator__sum mcp__cardea-calculator__sub".to_string());

        let tools = metadata.get_allowed_tools();
        assert_eq!(
            tools,
            vec!["mcp__cardea-calculator__sum", "mcp__cardea-calculator__sub"]
        );
    }

    #[test]
    fn test_skill_summary_from_loaded_skill_with_allowed_tools() {
        let mut metadata = test_metadata("calculator", "Calculator skill");
        metadata.allowed_tools = Some("mcp__calc__sum, mcp__calc__sub".to_string());

        let skill = LoadedSkill {
            metadata,
            content: "# Calculator".to_string(),
            raw_content: "---\nname: calculator\n---\n# Calculator".to_string(),
            skill_dir: PathBuf::from("/skills/calculator"),
            file_path: "/skills/calculator/SKILL.md".to_string(),
            enabled: true,
            loaded_at: Utc::now(),
            scripts: Vec::new(),
        };

        let summary = SkillSummary::from(&skill);
        assert_eq!(summary.name, "calculator");
        assert_eq!(summary.description, "Calculator skill");
        assert_eq!(
            summary.allowed_tools,
            vec!["mcp__calc__sum", "mcp__calc__sub"]
        );
    }

    #[test]
    fn test_skill_summary_from_metadata_with_allowed_tools() {
        let mut metadata = test_metadata("search", "Search skill");
        metadata.allowed_tools = Some("mcp__search__query mcp__search__lookup".to_string());

        let summary = SkillSummary::from(&metadata);
        assert_eq!(summary.name, "search");
        assert_eq!(summary.description, "Search skill");
        assert_eq!(
            summary.allowed_tools,
            vec!["mcp__search__query", "mcp__search__lookup"]
        );
    }

    // Tests for LoadedSkill methods

    fn create_test_skill_with_scripts() -> LoadedSkill {
        LoadedSkill {
            metadata: test_metadata("test-skill", "A test skill"),
            content: "# Test".to_string(),
            raw_content: "".to_string(),
            skill_dir: PathBuf::from("/skills/test-skill"),
            file_path: "/skills/test-skill/SKILL.md".to_string(),
            enabled: true,
            loaded_at: Utc::now(),
            scripts: vec![
                ScriptInfo {
                    name: "process.js".to_string(),
                    path: PathBuf::from("/skills/test-skill/scripts/process.js"),
                    executable: true,
                },
                ScriptInfo {
                    name: "helper.py".to_string(),
                    path: PathBuf::from("/skills/test-skill/scripts/helper.py"),
                    executable: true,
                },
            ],
        }
    }

    #[test]
    fn test_loaded_skill_get_script() {
        let skill = create_test_skill_with_scripts();

        // Found
        let script = skill.get_script("process.js");
        assert!(script.is_some());
        assert_eq!(script.unwrap().name, "process.js");

        // Not found
        let script = skill.get_script("nonexistent.js");
        assert!(script.is_none());
    }

    #[test]
    fn test_loaded_skill_has_script() {
        let skill = create_test_skill_with_scripts();

        assert!(skill.has_script("process.js"));
        assert!(skill.has_script("helper.py"));
        assert!(!skill.has_script("nonexistent.js"));
    }

    #[test]
    fn test_loaded_skill_directory_paths() {
        let skill = create_test_skill_with_scripts();

        assert_eq!(
            skill.assets_dir(),
            PathBuf::from("/skills/test-skill/assets")
        );
        assert_eq!(
            skill.references_dir(),
            PathBuf::from("/skills/test-skill/references")
        );
        assert_eq!(
            skill.scripts_dir(),
            PathBuf::from("/skills/test-skill/scripts")
        );
    }

    #[test]
    fn test_loaded_skill_build_script_env() {
        let skill = create_test_skill_with_scripts();
        let env = skill.build_script_env();

        assert_eq!(
            env.get("SKILL_DIR"),
            Some(&"/skills/test-skill".to_string())
        );
        assert_eq!(env.get("SKILL_NAME"), Some(&"test-skill".to_string()));
        assert_eq!(
            env.get("SKILL_ASSETS"),
            Some(&"/skills/test-skill/assets".to_string())
        );
        assert_eq!(
            env.get("SKILL_REFERENCES"),
            Some(&"/skills/test-skill/references".to_string())
        );
    }

    #[test]
    fn test_loaded_skill_list_scripts() {
        let skill = create_test_skill_with_scripts();
        let scripts = skill.list_scripts();

        assert_eq!(scripts.len(), 2);
        assert!(scripts.contains(&"process.js"));
        assert!(scripts.contains(&"helper.py"));
    }

    #[test]
    fn test_loaded_skill_list_scripts_empty() {
        let mut skill = create_test_skill_with_scripts();
        skill.scripts = Vec::new();
        let scripts = skill.list_scripts();

        assert!(scripts.is_empty());
    }

    // Tests for allowed_scripts functionality

    #[test]
    fn test_is_script_allowed_no_restrictions() {
        let metadata = test_metadata("test", "test");

        // No restrictions - all scripts allowed
        assert!(metadata.is_script_allowed("process.js"));
        assert!(metadata.is_script_allowed("helper.py"));
        assert!(metadata.is_script_allowed("any-script.sh"));
    }

    #[test]
    fn test_is_script_allowed_empty_list() {
        let mut metadata = test_metadata("test", "test");
        metadata.allowed_scripts = Some(vec![]);

        // Empty list - all scripts allowed
        assert!(metadata.is_script_allowed("process.js"));
    }

    #[test]
    fn test_is_script_allowed_exact_match() {
        let mut metadata = test_metadata("test", "test");
        metadata.allowed_scripts = Some(vec!["process.js".to_string(), "export.py".to_string()]);

        // Exact matches
        assert!(metadata.is_script_allowed("process.js"));
        assert!(metadata.is_script_allowed("export.py"));

        // Not in list
        assert!(!metadata.is_script_allowed("helper.py"));
        assert!(!metadata.is_script_allowed("process.ts"));
    }

    #[test]
    fn test_is_script_allowed_glob_star() {
        let mut metadata = test_metadata("test", "test");
        metadata.allowed_scripts = Some(vec!["*.js".to_string()]);

        // Matches *.js
        assert!(metadata.is_script_allowed("process.js"));
        assert!(metadata.is_script_allowed("helper.js"));
        assert!(metadata.is_script_allowed("a.js"));

        // Does not match
        assert!(!metadata.is_script_allowed("process.py"));
        assert!(!metadata.is_script_allowed("process.ts"));
    }

    #[test]
    fn test_is_script_allowed_glob_multiple_patterns() {
        let mut metadata = test_metadata("test", "test");
        metadata.allowed_scripts = Some(vec!["*.js".to_string(), "*.ts".to_string()]);

        // Matches either pattern
        assert!(metadata.is_script_allowed("process.js"));
        assert!(metadata.is_script_allowed("process.ts"));

        // Does not match
        assert!(!metadata.is_script_allowed("process.py"));
    }

    #[test]
    fn test_is_script_allowed_glob_prefix() {
        let mut metadata = test_metadata("test", "test");
        metadata.allowed_scripts = Some(vec!["process-*.js".to_string()]);

        // Matches prefix pattern
        assert!(metadata.is_script_allowed("process-data.js"));
        assert!(metadata.is_script_allowed("process-export.js"));
        assert!(metadata.is_script_allowed("process-.js"));

        // Does not match
        assert!(!metadata.is_script_allowed("process.js"));
        assert!(!metadata.is_script_allowed("helper-data.js"));
    }

    #[test]
    fn test_is_script_allowed_glob_question_mark() {
        let mut metadata = test_metadata("test", "test");
        metadata.allowed_scripts = Some(vec!["script?.js".to_string()]);

        // Matches exactly one character
        assert!(metadata.is_script_allowed("script1.js"));
        assert!(metadata.is_script_allowed("scripta.js"));

        // Does not match
        assert!(!metadata.is_script_allowed("script.js"));
        assert!(!metadata.is_script_allowed("script12.js"));
    }

    #[test]
    fn test_get_allowed_scripts_none() {
        let metadata = test_metadata("test", "test");
        assert!(metadata.get_allowed_scripts().is_none());
    }

    #[test]
    fn test_get_allowed_scripts_empty() {
        let mut metadata = test_metadata("test", "test");
        metadata.allowed_scripts = Some(vec![]);
        assert!(metadata.get_allowed_scripts().is_none());
    }

    #[test]
    fn test_get_allowed_scripts_with_values() {
        let mut metadata = test_metadata("test", "test");
        metadata.allowed_scripts = Some(vec!["*.js".to_string(), "process.py".to_string()]);

        let scripts = metadata.get_allowed_scripts();
        assert!(scripts.is_some());
        let scripts = scripts.unwrap();
        assert_eq!(scripts.len(), 2);
        assert!(scripts.contains(&"*.js".to_string()));
        assert!(scripts.contains(&"process.py".to_string()));
    }

    // Tests for SkillResourceLimits

    #[test]
    fn test_skill_resource_limits_default() {
        let limits = SkillResourceLimits::default();

        assert!(limits.max_memory_bytes.is_none());
        assert!(limits.timeout_secs.is_none());
        assert!(limits.max_output_bytes.is_none());
        assert!(limits.network_access.is_none());
        assert!(limits.is_empty());
    }

    #[test]
    fn test_skill_resource_limits_is_empty() {
        let mut limits = SkillResourceLimits::default();
        assert!(limits.is_empty());

        limits.timeout_secs = Some(30);
        assert!(!limits.is_empty());
    }

    #[test]
    fn test_skill_resource_limits_merge_with() {
        use std::time::Duration;

        use crate::executor::FilesystemPolicy;

        let global = ResourceLimits {
            max_memory_bytes: 256 * 1024 * 1024,
            timeout: Duration::from_secs(60),
            max_output_bytes: 1024 * 1024,
            network_access: false,
            filesystem_access: FilesystemPolicy::None,
        };

        // Partial override
        let skill_limits = SkillResourceLimits {
            max_memory_bytes: Some(128 * 1024 * 1024),
            timeout_secs: Some(30),
            max_output_bytes: None,
            network_access: Some(true),
        };

        let merged = skill_limits.merge_with(&global);

        // Skill values override
        assert_eq!(merged.max_memory_bytes, 128 * 1024 * 1024);
        assert_eq!(merged.timeout, Duration::from_secs(30));
        assert!(merged.network_access);

        // Global values inherited
        assert_eq!(merged.max_output_bytes, 1024 * 1024);
    }

    #[test]
    fn test_skill_resource_limits_merge_with_empty() {
        use std::time::Duration;

        use crate::executor::FilesystemPolicy;

        let global = ResourceLimits {
            max_memory_bytes: 256 * 1024 * 1024,
            timeout: Duration::from_secs(60),
            max_output_bytes: 1024 * 1024,
            network_access: false,
            filesystem_access: FilesystemPolicy::None,
        };

        // Empty skill limits
        let skill_limits = SkillResourceLimits::default();
        let merged = skill_limits.merge_with(&global);

        // All values from global
        assert_eq!(merged.max_memory_bytes, 256 * 1024 * 1024);
        assert_eq!(merged.timeout, Duration::from_secs(60));
        assert_eq!(merged.max_output_bytes, 1024 * 1024);
        assert!(!merged.network_access);
    }

    #[test]
    fn test_skill_resource_limits_serialization() {
        let limits = SkillResourceLimits {
            max_memory_bytes: Some(128 * 1024 * 1024),
            timeout_secs: Some(30),
            max_output_bytes: None,
            network_access: Some(true),
        };

        let json = serde_json::to_string(&limits).unwrap();

        // None fields should be skipped
        assert!(json.contains("max_memory_bytes"));
        assert!(json.contains("timeout_secs"));
        assert!(json.contains("network_access"));
        assert!(!json.contains("max_output_bytes"));

        let deserialized: SkillResourceLimits = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, limits);
    }

    #[test]
    fn test_skill_metadata_with_execution_limits() {
        let yaml = r#"
name: test-skill
description: A test skill
execution-limits:
  max_memory_bytes: 134217728
  timeout_secs: 30
  network_access: true
"#;

        let metadata: SkillMetadata = serde_yaml::from_str(yaml).unwrap();

        assert!(metadata.execution_limits.is_some());
        let limits = metadata.execution_limits.unwrap();
        assert_eq!(limits.max_memory_bytes, Some(128 * 1024 * 1024));
        assert_eq!(limits.timeout_secs, Some(30));
        assert_eq!(limits.network_access, Some(true));
        assert!(limits.max_output_bytes.is_none());
    }

    #[test]
    fn test_skill_metadata_with_allowed_scripts() {
        let yaml = r#"
name: test-skill
description: A test skill
allowed-scripts:
  - "*.js"
  - "*.ts"
  - "process.py"
"#;

        let metadata: SkillMetadata = serde_yaml::from_str(yaml).unwrap();

        assert!(metadata.allowed_scripts.is_some());
        let scripts = metadata.allowed_scripts.unwrap();
        assert_eq!(scripts.len(), 3);
        assert!(scripts.contains(&"*.js".to_string()));
        assert!(scripts.contains(&"*.ts".to_string()));
        assert!(scripts.contains(&"process.py".to_string()));
    }

    #[test]
    fn test_glob_match_complex_patterns() {
        // Test the glob matching function directly
        assert!(SkillMetadata::glob_match("*.js", "test.js"));
        assert!(SkillMetadata::glob_match("*.js", ".js"));
        assert!(!SkillMetadata::glob_match("*.js", "test.ts"));

        // Multiple stars
        assert!(SkillMetadata::glob_match("*-*-*.js", "a-b-c.js"));
        assert!(SkillMetadata::glob_match("*-*-*.js", "foo-bar-baz.js"));

        // Star at end
        assert!(SkillMetadata::glob_match("process*", "process.js"));
        assert!(SkillMetadata::glob_match("process*", "process-data.js"));
        assert!(SkillMetadata::glob_match("process*", "process"));

        // No wildcards
        assert!(SkillMetadata::glob_match("exact", "exact"));
        assert!(!SkillMetadata::glob_match("exact", "exacta"));
    }

    // Tests for ScriptContext

    #[test]
    fn test_script_context_new() {
        let context = ScriptContext::new();

        assert!(context.conversation_id.is_none());
        assert!(context.request_id.is_none());
        assert!(context.user_env.is_empty());
    }

    #[test]
    fn test_script_context_with_ids() {
        let context =
            ScriptContext::with_ids(Some("conv_123".to_string()), Some("req_456".to_string()));

        assert_eq!(context.conversation_id, Some("conv_123".to_string()));
        assert_eq!(context.request_id, Some("req_456".to_string()));
        assert!(context.user_env.is_empty());
    }

    #[test]
    fn test_script_context_with_env() {
        let context = ScriptContext::new()
            .with_env("CUSTOM_VAR", "value1")
            .with_env("ANOTHER_VAR", "value2");

        assert_eq!(
            context.user_env.get("CUSTOM_VAR"),
            Some(&"value1".to_string())
        );
        assert_eq!(
            context.user_env.get("ANOTHER_VAR"),
            Some(&"value2".to_string())
        );
    }

    #[test]
    fn test_script_context_to_env() {
        let skill = create_test_skill_with_scripts();
        let context =
            ScriptContext::with_ids(Some("conv_123".to_string()), Some("req_456".to_string()))
                .with_env("USER_VAR", "custom_value");

        let env = context.to_env(&skill, "process.js");

        // Core variables from skill
        assert_eq!(
            env.get("SKILL_DIR"),
            Some(&"/skills/test-skill".to_string())
        );
        assert_eq!(env.get("SKILL_NAME"), Some(&"test-skill".to_string()));

        // Script name
        assert_eq!(env.get("SCRIPT_NAME"), Some(&"process.js".to_string()));

        // Optional context
        assert_eq!(env.get("CONVERSATION_ID"), Some(&"conv_123".to_string()));
        assert_eq!(env.get("REQUEST_ID"), Some(&"req_456".to_string()));

        // Runtime info
        assert!(env.get("LLAMA_NEXUS_VERSION").is_some());

        // User-defined
        assert_eq!(env.get("USER_VAR"), Some(&"custom_value".to_string()));
    }

    #[test]
    fn test_script_context_user_env_overrides() {
        let skill = create_test_skill_with_scripts();
        // User env should override skill env
        let context = ScriptContext::new().with_env("SKILL_NAME", "overridden-name");

        let env = context.to_env(&skill, "process.js");

        // User env takes precedence
        assert_eq!(env.get("SKILL_NAME"), Some(&"overridden-name".to_string()));
    }

    #[test]
    fn test_script_context_without_optional_ids() {
        let skill = create_test_skill_with_scripts();
        let context = ScriptContext::new();

        let env = context.to_env(&skill, "script.js");

        // Optional IDs should not be present
        assert!(env.get("CONVERSATION_ID").is_none());
        assert!(env.get("REQUEST_ID").is_none());

        // But core variables should still be present
        assert!(env.get("SKILL_DIR").is_some());
        assert!(env.get("SCRIPT_NAME").is_some());
    }
}
