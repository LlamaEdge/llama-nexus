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
    /// * `limits` - Optional resource limits (uses global default if None)
    ///
    /// # Returns
    /// * `Ok(ScriptOutput)` - Execution result including stdout, stderr, exit code
    /// * `Err(ExecutionError)` - If script not found, no executor available, or execution fails
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

        // Execute the script
        manager.execute(script, args, env, limits).await
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_allowed_tools_space_separated() {
        let metadata = SkillMetadata {
            name: "test".to_string(),
            description: "test".to_string(),
            license: None,
            compatibility: None,
            metadata: None,
            allowed_tools: Some("tool1 tool2 tool3".to_string()),
            model: None,
        };

        let tools = metadata.get_allowed_tools();
        assert_eq!(tools, vec!["tool1", "tool2", "tool3"]);
    }

    #[test]
    fn test_get_allowed_tools_empty() {
        let metadata = SkillMetadata {
            name: "test".to_string(),
            description: "test".to_string(),
            license: None,
            compatibility: None,
            metadata: None,
            allowed_tools: None,
            model: None,
        };

        let tools = metadata.get_allowed_tools();
        assert!(tools.is_empty());
    }

    #[test]
    fn test_get_allowed_tools_with_extra_whitespace() {
        let metadata = SkillMetadata {
            name: "test".to_string(),
            description: "test".to_string(),
            license: None,
            compatibility: None,
            metadata: None,
            allowed_tools: Some("  tool1   tool2  ".to_string()),
            model: None,
        };

        let tools = metadata.get_allowed_tools();
        assert_eq!(tools, vec!["tool1", "tool2"]);
    }

    #[test]
    fn test_skill_summary_from_loaded_skill() {
        let skill = LoadedSkill {
            metadata: SkillMetadata {
                name: "weather-query".to_string(),
                description: "Query weather information".to_string(),
                license: None,
                compatibility: None,
                metadata: None,
                allowed_tools: None,
                model: None,
            },
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
        let metadata = SkillMetadata {
            name: "code-review".to_string(),
            description: "Review code for best practices".to_string(),
            license: Some("MIT".to_string()),
            compatibility: None,
            metadata: None,
            allowed_tools: None,
            model: None,
        };

        let summary = SkillSummary::from(&metadata);
        assert_eq!(summary.name, "code-review");
        assert_eq!(summary.description, "Review code for best practices");
    }

    #[test]
    fn test_skill_metadata_serialization() {
        let metadata = SkillMetadata {
            name: "test-skill".to_string(),
            description: "A test skill".to_string(),
            license: Some("Apache-2.0".to_string()),
            compatibility: Some("Requires network access".to_string()),
            metadata: Some(HashMap::from([
                ("author".to_string(), "test".to_string()),
                ("version".to_string(), "1.0".to_string()),
            ])),
            allowed_tools: Some("Bash Read Write".to_string()),
            model: Some("claude-sonnet".to_string()),
        };

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
        let metadata = SkillMetadata {
            name: "test".to_string(),
            description: "test".to_string(),
            license: None,
            compatibility: None,
            metadata: None,
            allowed_tools: Some("Bash".to_string()),
            model: None,
        };

        let tools = metadata.get_allowed_tools();
        assert_eq!(tools, vec!["Bash"]);
    }

    #[test]
    fn test_get_allowed_tools_with_wildcards() {
        let metadata = SkillMetadata {
            name: "test".to_string(),
            description: "test".to_string(),
            license: None,
            compatibility: None,
            metadata: None,
            allowed_tools: Some("Bash(git:*) Read Write".to_string()),
            model: None,
        };

        let tools = metadata.get_allowed_tools();
        assert_eq!(tools, vec!["Bash(git:*)", "Read", "Write"]);
    }

    #[test]
    fn test_get_allowed_tools_empty_string() {
        let metadata = SkillMetadata {
            name: "test".to_string(),
            description: "test".to_string(),
            license: None,
            compatibility: None,
            metadata: None,
            allowed_tools: Some("".to_string()),
            model: None,
        };

        let tools = metadata.get_allowed_tools();
        assert!(tools.is_empty());
    }

    #[test]
    fn test_get_allowed_tools_whitespace_only() {
        let metadata = SkillMetadata {
            name: "test".to_string(),
            description: "test".to_string(),
            license: None,
            compatibility: None,
            metadata: None,
            allowed_tools: Some("   \t\n  ".to_string()),
            model: None,
        };

        let tools = metadata.get_allowed_tools();
        assert!(tools.is_empty());
    }

    #[test]
    fn test_get_allowed_tools_comma_separated() {
        let metadata = SkillMetadata {
            name: "test".to_string(),
            description: "test".to_string(),
            license: None,
            compatibility: None,
            metadata: None,
            allowed_tools: Some("tool1, tool2, tool3".to_string()),
            model: None,
        };

        let tools = metadata.get_allowed_tools();
        assert_eq!(tools, vec!["tool1", "tool2", "tool3"]);
    }

    #[test]
    fn test_get_allowed_tools_comma_no_space() {
        let metadata = SkillMetadata {
            name: "test".to_string(),
            description: "test".to_string(),
            license: None,
            compatibility: None,
            metadata: None,
            allowed_tools: Some("tool1,tool2,tool3".to_string()),
            model: None,
        };

        let tools = metadata.get_allowed_tools();
        assert_eq!(tools, vec!["tool1", "tool2", "tool3"]);
    }

    #[test]
    fn test_get_allowed_tools_comma_extra_whitespace() {
        let metadata = SkillMetadata {
            name: "test".to_string(),
            description: "test".to_string(),
            license: None,
            compatibility: None,
            metadata: None,
            allowed_tools: Some("  tool1 ,  tool2  ,  tool3  ".to_string()),
            model: None,
        };

        let tools = metadata.get_allowed_tools();
        assert_eq!(tools, vec!["tool1", "tool2", "tool3"]);
    }

    #[test]
    fn test_get_allowed_tools_mcp_tool_names_comma() {
        let metadata = SkillMetadata {
            name: "test".to_string(),
            description: "test".to_string(),
            license: None,
            compatibility: None,
            metadata: None,
            allowed_tools: Some(
                "mcp__cardea-calculator__sum, mcp__cardea-calculator__sub".to_string(),
            ),
            model: None,
        };

        let tools = metadata.get_allowed_tools();
        assert_eq!(
            tools,
            vec!["mcp__cardea-calculator__sum", "mcp__cardea-calculator__sub"]
        );
    }

    #[test]
    fn test_get_allowed_tools_mcp_tool_names_space() {
        let metadata = SkillMetadata {
            name: "test".to_string(),
            description: "test".to_string(),
            license: None,
            compatibility: None,
            metadata: None,
            allowed_tools: Some(
                "mcp__cardea-calculator__sum mcp__cardea-calculator__sub".to_string(),
            ),
            model: None,
        };

        let tools = metadata.get_allowed_tools();
        assert_eq!(
            tools,
            vec!["mcp__cardea-calculator__sum", "mcp__cardea-calculator__sub"]
        );
    }

    #[test]
    fn test_skill_summary_from_loaded_skill_with_allowed_tools() {
        let skill = LoadedSkill {
            metadata: SkillMetadata {
                name: "calculator".to_string(),
                description: "Calculator skill".to_string(),
                license: None,
                compatibility: None,
                metadata: None,
                allowed_tools: Some("mcp__calc__sum, mcp__calc__sub".to_string()),
                model: None,
            },
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
        let metadata = SkillMetadata {
            name: "search".to_string(),
            description: "Search skill".to_string(),
            license: None,
            compatibility: None,
            metadata: None,
            allowed_tools: Some("mcp__search__query mcp__search__lookup".to_string()),
            model: None,
        };

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
            metadata: SkillMetadata {
                name: "test-skill".to_string(),
                description: "A test skill".to_string(),
                license: None,
                compatibility: None,
                metadata: None,
                allowed_tools: None,
                model: None,
            },
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
}
