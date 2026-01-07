//! SKILL.md file parser
//!
//! Parses SKILL.md files with YAML front matter

use std::path::Path;

use chrono::Utc;

use crate::skills::{
    error::{SkillError, SkillResult},
    loader::SkillLoader,
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
    pub async fn parse(content: &str, skill_dir: &Path) -> SkillResult<LoadedSkill> {
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

        // Load scripts from scripts/ directory
        let scripts = SkillLoader::list_scripts(skill_dir).await;

        Ok(LoadedSkill {
            metadata,
            content: markdown,
            raw_content: content.to_string(),
            skill_dir: skill_dir.to_path_buf(),
            file_path: file_path.to_string_lossy().to_string(),
            enabled: true,
            loaded_at: Utc::now(),
            scripts,
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

    use tempfile::TempDir;

    use super::*;

    /// Helper to create a test skill directory with SKILL.md
    fn create_test_skill_dir(name: &str, content: &str) -> TempDir {
        let temp_dir = TempDir::new().unwrap();
        let skill_dir = temp_dir.path().join(name);
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(skill_dir.join("SKILL.md"), content).unwrap();
        temp_dir
    }

    #[tokio::test]
    async fn test_parse_valid_skill() {
        let content = r#"---
name: weather-query
description: Query weather information for cities
---

# Weather Query Skill

This skill helps query weather.
"#;

        let temp_dir = create_test_skill_dir("weather-query", content);
        let skill_dir = temp_dir.path().join("weather-query");
        let result = SkillParser::parse(content, &skill_dir).await;

        assert!(result.is_ok());
        let skill = result.unwrap();
        assert_eq!(skill.metadata.name, "weather-query");
        assert_eq!(
            skill.metadata.description,
            "Query weather information for cities"
        );
        assert!(skill.content.contains("# Weather Query Skill"));
        assert!(skill.scripts.is_empty()); // No scripts directory
    }

    #[tokio::test]
    async fn test_parse_with_all_fields() {
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

        let temp_dir = create_test_skill_dir("code-review", content);
        let skill_dir = temp_dir.path().join("code-review");
        let result = SkillParser::parse(content, &skill_dir).await;

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

    #[tokio::test]
    async fn test_parse_with_scripts() {
        let content = r#"---
name: scripted-skill
description: A skill with scripts
---

# Scripted Skill
"#;

        let temp_dir = create_test_skill_dir("scripted-skill", content);
        let skill_dir = temp_dir.path().join("scripted-skill");

        // Create scripts directory with some scripts
        let scripts_dir = skill_dir.join("scripts");
        std::fs::create_dir_all(&scripts_dir).unwrap();
        std::fs::write(scripts_dir.join("process.js"), "console.log('test');").unwrap();
        std::fs::write(scripts_dir.join("helper.py"), "print('test')").unwrap();

        let result = SkillParser::parse(content, &skill_dir).await;

        assert!(result.is_ok());
        let skill = result.unwrap();
        assert_eq!(skill.scripts.len(), 2);
    }

    #[tokio::test]
    async fn test_parse_missing_front_matter() {
        let content = "# No front matter";
        let temp_dir = TempDir::new().unwrap();
        let skill_dir = temp_dir.path().join("test");
        std::fs::create_dir_all(&skill_dir).unwrap();
        let result = SkillParser::parse(content, &skill_dir).await;

        assert!(matches!(result, Err(SkillError::ParseError(_))));
    }

    #[tokio::test]
    async fn test_parse_unclosed_front_matter() {
        let content = r#"---
name: test
description: Test
"#;

        let temp_dir = TempDir::new().unwrap();
        let skill_dir = temp_dir.path().join("test");
        std::fs::create_dir_all(&skill_dir).unwrap();
        let result = SkillParser::parse(content, &skill_dir).await;

        assert!(matches!(result, Err(SkillError::ParseError(_))));
    }

    #[tokio::test]
    async fn test_parse_name_mismatch() {
        let content = r#"---
name: different-name
description: Test skill
---
"#;

        let temp_dir = TempDir::new().unwrap();
        let skill_dir = temp_dir.path().join("actual-name");
        std::fs::create_dir_all(&skill_dir).unwrap();
        let result = SkillParser::parse(content, &skill_dir).await;

        assert!(matches!(
            result,
            Err(SkillError::NameDirectoryMismatch { .. })
        ));
    }

    #[tokio::test]
    async fn test_parse_invalid_name() {
        let content = r#"---
name: Invalid-Name
description: Test skill
---
"#;

        let temp_dir = TempDir::new().unwrap();
        let skill_dir = temp_dir.path().join("Invalid-Name");
        std::fs::create_dir_all(&skill_dir).unwrap();
        let result = SkillParser::parse(content, &skill_dir).await;

        assert!(matches!(result, Err(SkillError::InvalidName { .. })));
    }

    #[tokio::test]
    async fn test_parse_empty_description() {
        let content = r#"---
name: test
description: ""
---
"#;

        let temp_dir = TempDir::new().unwrap();
        let skill_dir = temp_dir.path().join("test");
        std::fs::create_dir_all(&skill_dir).unwrap();
        let result = SkillParser::parse(content, &skill_dir).await;

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

    #[tokio::test]
    async fn test_parse_empty_front_matter() {
        let content = r#"---
---
Body content
"#;
        let temp_dir = TempDir::new().unwrap();
        let skill_dir = temp_dir.path().join("test");
        std::fs::create_dir_all(&skill_dir).unwrap();
        let result = SkillParser::parse(content, &skill_dir).await;

        assert!(matches!(result, Err(SkillError::ParseError(_))));
    }

    #[tokio::test]
    async fn test_parse_yaml_syntax_error() {
        let content = r#"---
name: test
description: "unclosed quote
---
"#;
        let temp_dir = TempDir::new().unwrap();
        let skill_dir = temp_dir.path().join("test");
        std::fs::create_dir_all(&skill_dir).unwrap();
        let result = SkillParser::parse(content, &skill_dir).await;

        assert!(matches!(result, Err(SkillError::YamlError(_))));
    }

    #[tokio::test]
    async fn test_parse_missing_required_fields() {
        let content = r#"---
name: test
---
"#;
        let temp_dir = TempDir::new().unwrap();
        let skill_dir = temp_dir.path().join("test");
        std::fs::create_dir_all(&skill_dir).unwrap();
        let result = SkillParser::parse(content, &skill_dir).await;

        assert!(matches!(result, Err(SkillError::YamlError(_))));
    }

    #[tokio::test]
    async fn test_parse_description_too_long() {
        let long_desc = "x".repeat(1025);
        let content = format!(
            r#"---
name: test
description: "{}"
---
"#,
            long_desc
        );
        let temp_dir = TempDir::new().unwrap();
        let skill_dir = temp_dir.path().join("test");
        std::fs::create_dir_all(&skill_dir).unwrap();
        let result = SkillParser::parse(&content, &skill_dir).await;

        assert!(matches!(result, Err(SkillError::InvalidDescription(_))));
    }

    #[tokio::test]
    async fn test_parse_compatibility_too_long() {
        let long_compat = "x".repeat(501);
        let content = format!(
            r#"---
name: test
description: Valid description
compatibility: "{}"
---
"#,
            long_compat
        );
        let temp_dir = TempDir::new().unwrap();
        let skill_dir = temp_dir.path().join("test");
        std::fs::create_dir_all(&skill_dir).unwrap();
        let result = SkillParser::parse(&content, &skill_dir).await;

        assert!(matches!(result, Err(SkillError::InvalidCompatibility(_))));
    }

    #[tokio::test]
    async fn test_parse_with_whitespace_around_front_matter() {
        let content = r#"

---
name: test-skill
description: A test skill
---

# Content

"#;
        let temp_dir = create_test_skill_dir("test-skill", content);
        let skill_dir = temp_dir.path().join("test-skill");
        let result = SkillParser::parse(content, &skill_dir).await;

        assert!(result.is_ok());
        let skill = result.unwrap();
        assert_eq!(skill.metadata.name, "test-skill");
    }

    #[tokio::test]
    async fn test_parse_markdown_with_triple_dashes() {
        let content = r#"---
name: test
description: A test skill
---

# Content

Some text here.

---

More content after horizontal rule.
"#;
        let temp_dir = create_test_skill_dir("test", content);
        let skill_dir = temp_dir.path().join("test");
        let result = SkillParser::parse(content, &skill_dir).await;

        assert!(result.is_ok());
        let skill = result.unwrap();
        // The markdown content should preserve --- as horizontal rules
        assert!(skill.content.contains("---"));
        assert!(skill.content.contains("More content after horizontal rule"));
    }

    #[tokio::test]
    async fn test_parse_empty_markdown_body() {
        let content = r#"---
name: test
description: A test skill
---
"#;
        let temp_dir = create_test_skill_dir("test", content);
        let skill_dir = temp_dir.path().join("test");
        let result = SkillParser::parse(content, &skill_dir).await;

        assert!(result.is_ok());
        let skill = result.unwrap();
        assert!(skill.content.is_empty());
    }

    #[tokio::test]
    async fn test_parse_name_too_long() {
        let long_name = "a".repeat(65);
        let content = format!(
            r#"---
name: {}
description: A test skill
---
"#,
            long_name
        );
        let temp_dir = TempDir::new().unwrap();
        let skill_dir = temp_dir.path().join(&long_name);
        std::fs::create_dir_all(&skill_dir).unwrap();
        let result = SkillParser::parse(&content, &skill_dir).await;

        assert!(matches!(result, Err(SkillError::InvalidName { .. })));
    }

    #[tokio::test]
    async fn test_parse_preserves_raw_content() {
        let content = r#"---
name: test
description: A test skill
---

# Content here
"#;
        let temp_dir = create_test_skill_dir("test", content);
        let skill_dir = temp_dir.path().join("test");
        let result = SkillParser::parse(content, &skill_dir).await;

        assert!(result.is_ok());
        let skill = result.unwrap();
        assert_eq!(skill.raw_content, content);
    }

    #[tokio::test]
    async fn test_parse_sets_file_path() {
        let content = r#"---
name: my-skill
description: A test skill
---
"#;
        let temp_dir = create_test_skill_dir("my-skill", content);
        let skill_dir = temp_dir.path().join("my-skill");
        let result = SkillParser::parse(content, &skill_dir).await;

        assert!(result.is_ok());
        let skill = result.unwrap();
        assert!(skill.file_path.ends_with("SKILL.md"));
        assert!(skill.file_path.contains("my-skill"));
    }

    #[tokio::test]
    async fn test_parse_enabled_by_default() {
        let content = r#"---
name: test
description: A test skill
---
"#;
        let temp_dir = create_test_skill_dir("test", content);
        let skill_dir = temp_dir.path().join("test");
        let result = SkillParser::parse(content, &skill_dir).await;

        assert!(result.is_ok());
        let skill = result.unwrap();
        assert!(skill.enabled);
    }

    #[test]
    fn test_split_front_matter_no_body() {
        let content = r#"---
key: value
---"#;

        let (front, body) = SkillParser::split_front_matter(content).unwrap();
        assert_eq!(front, "key: value");
        assert!(body.is_empty());
    }

    #[test]
    fn test_split_front_matter_multiline_yaml() {
        let content = r#"---
name: test
description: |
  Multi-line
  description here
metadata:
  key1: value1
  key2: value2
---

Body
"#;

        let (front, body) = SkillParser::split_front_matter(content).unwrap();
        assert!(front.contains("Multi-line"));
        assert!(front.contains("key2: value2"));
        assert_eq!(body, "Body");
    }
}
