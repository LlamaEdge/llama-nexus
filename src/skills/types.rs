//! Type definitions for Skills module
//!
//! Compliant with [Agent Skills Standard](https://agentskills.io/specification)

use std::{collections::HashMap, path::PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

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
    /// Standard format: space-separated (not comma-separated)
    pub fn get_allowed_tools(&self) -> Vec<String> {
        self.allowed_tools
            .as_ref()
            .map(|s| s.split_whitespace().map(|t| t.to_string()).collect())
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
    #[allow(dead_code)]
    pub skill_dir: PathBuf,

    /// Path to SKILL.md file
    #[allow(dead_code)]
    pub file_path: String,

    /// Whether this skill is enabled
    pub enabled: bool,

    /// When this skill was loaded
    #[allow(dead_code)]
    pub loaded_at: DateTime<Utc>,
}

/// Skill summary for phase 1 injection
///
/// Contains only name and description to minimize context usage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillSummary {
    /// Skill name
    pub name: String,

    /// Skill description
    pub description: String,
}

impl From<&LoadedSkill> for SkillSummary {
    fn from(skill: &LoadedSkill) -> Self {
        Self {
            name: skill.metadata.name.clone(),
            description: skill.metadata.description.clone(),
        }
    }
}

impl From<&SkillMetadata> for SkillSummary {
    fn from(metadata: &SkillMetadata) -> Self {
        Self {
            name: metadata.name.clone(),
            description: metadata.description.clone(),
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
        };

        let json = serde_json::to_string(&summary).unwrap();
        assert!(json.contains("git-commit"));
        assert!(json.contains("Create git commits"));

        let deserialized: SkillSummary = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.name, "git-commit");
        assert_eq!(deserialized.description, "Create git commits");
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
}
