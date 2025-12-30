//! SKILL.md file parser
//!
//! Parses SKILL.md files with YAML front matter

use std::path::Path;

use chrono::Utc;

use crate::skills::{
    error::{SkillError, SkillResult},
    types::{LoadedSkill, SkillMetadata},
    validator::{validate_compatibility, validate_description, validate_skill_name},
};

/// Parser for SKILL.md files
pub struct SkillParser;

impl SkillParser {
    /// Parse a SKILL.md file content
    ///
    /// # Arguments
    /// * `content` - The raw file content
    /// * `skill_dir` - The directory containing the skill
    ///
    /// # Returns
    /// * `Ok(LoadedSkill)` on success
    /// * `Err(SkillError)` on failure
    pub fn parse(content: &str, skill_dir: &Path) -> SkillResult<LoadedSkill> {
        let (front_matter, markdown) = Self::split_front_matter(content)?;

        let metadata: SkillMetadata = serde_yaml::from_str(&front_matter)
            .map_err(|e| SkillError::YamlError(e.to_string()))?;

        // Get directory name for validation
        let dir_name = skill_dir
            .file_name()
            .and_then(|n| n.to_str())
            .map(|s| s.to_string());

        // Validate fields
        validate_skill_name(&metadata.name, dir_name.as_deref())?;
        validate_description(&metadata.description)?;

        if let Some(ref compat) = metadata.compatibility {
            validate_compatibility(compat)?;
        }

        let file_path = skill_dir.join("SKILL.md");

        Ok(LoadedSkill {
            metadata,
            content: markdown,
            raw_content: content.to_string(),
            skill_dir: skill_dir.to_path_buf(),
            file_path: file_path.to_string_lossy().to_string(),
            enabled: true,
            loaded_at: Utc::now(),
        })
    }

    /// Split content into YAML front matter and markdown body
    fn split_front_matter(content: &str) -> SkillResult<(String, String)> {
        let content = content.trim();

        // Must start with ---
        if !content.starts_with("---") {
            return Err(SkillError::ParseError(
                "SKILL.md must start with YAML front matter (---)".to_string(),
            ));
        }

        // Find the closing ---
        let rest = &content[3..];
        let end_index = rest.find("---").ok_or_else(|| {
            SkillError::ParseError("SKILL.md front matter not properly closed (missing ---)".into())
        })?;

        let front_matter = rest[..end_index].trim().to_string();
        let markdown = rest[end_index + 3..].trim().to_string();

        if front_matter.is_empty() {
            return Err(SkillError::ParseError(
                "YAML front matter cannot be empty".to_string(),
            ));
        }

        Ok((front_matter, markdown))
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn test_parse_valid_skill() {
        let content = r#"---
name: weather-query
description: Query weather information for cities
---

# Weather Query Skill

This skill helps query weather.
"#;

        let skill_dir = PathBuf::from("/skills/weather-query");
        let result = SkillParser::parse(content, &skill_dir);

        assert!(result.is_ok());
        let skill = result.unwrap();
        assert_eq!(skill.metadata.name, "weather-query");
        assert_eq!(
            skill.metadata.description,
            "Query weather information for cities"
        );
        assert!(skill.content.contains("# Weather Query Skill"));
    }

    #[test]
    fn test_parse_with_all_fields() {
        let content = r#"---
name: code-review
description: Review code changes and provide feedback
license: MIT
compatibility: Requires git installed
allowed-tools: git-diff git-show
metadata:
  author: llama-nexus
  version: "1.0"
model: claude-3-opus
---

# Code Review Skill
"#;

        let skill_dir = PathBuf::from("/skills/code-review");
        let result = SkillParser::parse(content, &skill_dir);

        assert!(result.is_ok());
        let skill = result.unwrap();
        assert_eq!(skill.metadata.name, "code-review");
        assert_eq!(skill.metadata.license, Some("MIT".to_string()));
        assert_eq!(
            skill.metadata.compatibility,
            Some("Requires git installed".to_string())
        );
        assert_eq!(
            skill.metadata.get_allowed_tools(),
            vec!["git-diff", "git-show"]
        );
        assert_eq!(skill.metadata.model, Some("claude-3-opus".to_string()));

        let meta = skill.metadata.metadata.unwrap();
        assert_eq!(meta.get("author"), Some(&"llama-nexus".to_string()));
    }

    #[test]
    fn test_parse_missing_front_matter() {
        let content = "# No front matter";
        let skill_dir = PathBuf::from("/skills/test");
        let result = SkillParser::parse(content, &skill_dir);

        assert!(matches!(result, Err(SkillError::ParseError(_))));
    }

    #[test]
    fn test_parse_unclosed_front_matter() {
        let content = r#"---
name: test
description: Test
"#;

        let skill_dir = PathBuf::from("/skills/test");
        let result = SkillParser::parse(content, &skill_dir);

        assert!(matches!(result, Err(SkillError::ParseError(_))));
    }

    #[test]
    fn test_parse_name_mismatch() {
        let content = r#"---
name: different-name
description: Test skill
---
"#;

        let skill_dir = PathBuf::from("/skills/actual-name");
        let result = SkillParser::parse(content, &skill_dir);

        assert!(matches!(
            result,
            Err(SkillError::NameDirectoryMismatch { .. })
        ));
    }

    #[test]
    fn test_parse_invalid_name() {
        let content = r#"---
name: Invalid-Name
description: Test skill
---
"#;

        let skill_dir = PathBuf::from("/skills/Invalid-Name");
        let result = SkillParser::parse(content, &skill_dir);

        assert!(matches!(result, Err(SkillError::InvalidName { .. })));
    }

    #[test]
    fn test_parse_empty_description() {
        let content = r#"---
name: test
description: ""
---
"#;

        let skill_dir = PathBuf::from("/skills/test");
        let result = SkillParser::parse(content, &skill_dir);

        assert!(matches!(result, Err(SkillError::InvalidDescription(_))));
    }

    #[test]
    fn test_split_front_matter() {
        let content = r#"---
key: value
---

Body content here.
"#;

        let (front, body) = SkillParser::split_front_matter(content).unwrap();
        assert_eq!(front, "key: value");
        assert_eq!(body, "Body content here.");
    }
}
