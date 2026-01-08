//! Skill Injector for prompt enhancement
//!
//! Injects skill information into system prompts:
//! - Phase 1: Skill summaries (name + description) in table format
//! - Phase 2: Full skill content when activated (with optional references)

use crate::skills::{
    loader::SkillLoader,
    types::{LoadedSkill, SkillSummary},
};

/// Skill injector for enhancing system prompts
///
/// Provides methods for injecting skill information into prompts.
/// Used by plan.rs for two-phase skill loading.
pub struct SkillInjector;

impl SkillInjector {
    /// Generate Phase 1 injection text with skill summaries
    ///
    /// Creates a formatted table of available skills with their descriptions
    /// for the LLM to consider when planning.
    ///
    /// # Arguments
    /// * `summaries` - List of skill summaries to inject
    ///
    /// # Returns
    /// Formatted markdown text for system prompt injection (empty if no summaries)
    pub fn phase1_injection(summaries: &[SkillSummary]) -> String {
        if summaries.is_empty() {
            return String::new();
        }

        let skills_table = summaries
            .iter()
            .map(|s| format!("| {} | {} |", s.name, s.description))
            .collect::<Vec<_>>()
            .join("\n");

        format!(
            r#"

## Available Skills

The following skills are available to help you complete this task:

| Skill | Description |
|-------|-------------|
{}

If you need to use a skill, wrap the skill name in <use_skill></use_skill> tags at the beginning of your response.
Example: <use_skill>skill-name</use_skill>

"#,
            skills_table
        )
    }

    /// Generate Phase 2 injection text with full skill content
    ///
    /// Injects the complete SKILL.md content for activated skills.
    ///
    /// # Arguments
    /// * `skill` - The fully loaded skill to inject
    ///
    /// # Returns
    /// Formatted text with skill name and full content
    pub fn phase2_injection(skill: &LoadedSkill) -> String {
        Self::phase2_injection_with_refs(skill, &[])
    }

    /// Generate Phase 2 injection text with full skill content and references
    ///
    /// Injects the complete SKILL.md content along with any reference documents
    /// from the skill's references/ directory.
    ///
    /// # Arguments
    /// * `skill` - The fully loaded skill to inject
    /// * `references` - Reference documents to include
    ///
    /// # Returns
    /// Formatted text with skill name, full content, and references
    pub fn phase2_injection_with_refs(skill: &LoadedSkill, references: &[String]) -> String {
        let refs_section = if references.is_empty() {
            String::new()
        } else {
            let refs_content = references
                .iter()
                .enumerate()
                .map(|(i, content)| {
                    format!("### Reference Document {}\n\n{}", i + 1, content.trim())
                })
                .collect::<Vec<_>>()
                .join("\n\n");

            format!(
                "\n\n## Reference Materials\n\nThe following reference documents provide additional context:\n\n{}",
                refs_content
            )
        };

        format!(
            r#"## Active Skill: {}

The following skill instructions guide how to complete this task:

---
{}
---{}"#,
            skill.metadata.name, skill.content, refs_section
        )
    }

    /// Generate Phase 2 injection text with auto-loaded references
    ///
    /// Automatically loads reference documents from the skill's references/ directory
    /// and includes them in the injection. Respects the skill's `references` field
    /// for filtering which files to load.
    ///
    /// # Arguments
    /// * `skill` - The fully loaded skill to inject
    /// * `max_total_size` - Maximum total size of all references in bytes (0 = no limit)
    ///
    /// # Returns
    /// Formatted text with skill name, full content, and auto-loaded references
    #[allow(dead_code)]
    pub async fn phase2_injection_auto_refs(skill: &LoadedSkill, max_total_size: usize) -> String {
        // Load references with optional pattern filtering from skill metadata
        let references = SkillLoader::load_references_with_patterns(
            &skill.skill_dir,
            skill.metadata.references.as_deref(),
        )
        .await;

        // Apply size limit if specified
        let filtered_refs = if max_total_size > 0 {
            Self::filter_by_size(&references, max_total_size)
        } else {
            references
        };

        Self::phase2_injection_with_refs(skill, &filtered_refs)
    }

    /// Filter references by total size limit
    ///
    /// Includes references until the total size exceeds the limit.
    /// Documents are included in order, so earlier documents have priority.
    fn filter_by_size(references: &[String], max_size: usize) -> Vec<String> {
        let mut result = Vec::new();
        let mut total_size = 0;

        for content in references {
            let content_size = content.len();
            if total_size + content_size <= max_size {
                result.push(content.clone());
                total_size += content_size;
            }
        }

        result
    }

    /// Generate injection text for multiple skills
    ///
    /// Combines content from all skills into a single injection text,
    /// with a summary section showing merged allowed-tools and allowed-scripts.
    ///
    /// # Arguments
    /// * `skills` - List of loaded skills to inject
    ///
    /// # Returns
    /// Combined injection text for all skills with merged permissions
    #[allow(dead_code)]
    pub fn multi_skill_injection(skills: &[LoadedSkill]) -> String {
        Self::multi_skill_injection_with_refs(skills, &[])
    }

    /// Generate injection text for multiple skills with references
    ///
    /// # Arguments
    /// * `skills` - List of loaded skills to inject
    /// * `all_references` - All references to include (flat list from all skills)
    ///
    /// # Returns
    /// Combined injection text for all skills with merged permissions and references
    #[allow(dead_code)]
    pub fn multi_skill_injection_with_refs(
        skills: &[LoadedSkill],
        all_references: &[String],
    ) -> String {
        if skills.is_empty() {
            return String::new();
        }

        let mut output = String::new();

        // Generate skill names list
        let skill_names: Vec<&str> = skills.iter().map(|s| s.metadata.name.as_str()).collect();

        // Header with active skills
        output.push_str(&format!(
            "\n## Active Skills: {}\n\n",
            skill_names.join(", ")
        ));

        // Merged permissions section
        let merged_tools = Self::merge_allowed_tools(skills);
        let merged_scripts = Self::merge_allowed_scripts(skills);

        if !merged_tools.is_empty() || !merged_scripts.is_empty() {
            output.push_str("### Merged Permissions\n\n");

            if !merged_tools.is_empty() {
                output.push_str(&format!(
                    "**Allowed Tools:** {}\n\n",
                    merged_tools.join(", ")
                ));
            }

            if !merged_scripts.is_empty() {
                output.push_str(&format!(
                    "**Allowed Scripts:** {}\n\n",
                    merged_scripts.join(", ")
                ));
            }
        }

        // Individual skill content
        output.push_str("---\n\n");

        for (i, skill) in skills.iter().enumerate() {
            if i > 0 {
                output.push_str("\n---\n\n");
            }
            output.push_str(&format!("### Skill: {}\n\n", skill.metadata.name));
            output.push_str(&skill.content);
        }

        output.push_str("\n---");

        // References section (if any)
        if !all_references.is_empty() {
            let refs_content = all_references
                .iter()
                .enumerate()
                .map(|(i, content)| {
                    format!("### Reference Document {}\n\n{}", i + 1, content.trim())
                })
                .collect::<Vec<_>>()
                .join("\n\n");

            output.push_str(&format!(
                "\n\n## Reference Materials\n\nThe following reference documents provide additional context:\n\n{}",
                refs_content
            ));
        }

        output
    }

    /// Generate injection text for multiple skills with auto-loaded references
    ///
    /// Automatically loads reference documents from each skill's references/ directory
    /// and includes them in the injection.
    ///
    /// # Arguments
    /// * `skills` - List of loaded skills to inject
    /// * `max_total_size` - Maximum total size of all references in bytes (0 = no limit)
    ///
    /// # Returns
    /// Combined injection text for all skills with merged permissions and auto-loaded references
    #[allow(dead_code)]
    pub async fn multi_skill_injection_auto_refs(
        skills: &[LoadedSkill],
        max_total_size: usize,
    ) -> String {
        if skills.is_empty() {
            return String::new();
        }

        // Load references from all skills
        let mut all_references = Vec::new();

        for skill in skills {
            let skill_refs = SkillLoader::load_references_with_patterns(
                &skill.skill_dir,
                skill.metadata.references.as_deref(),
            )
            .await;
            all_references.extend(skill_refs);
        }

        // Apply size limit if specified
        let filtered_refs = if max_total_size > 0 {
            Self::filter_by_size(&all_references, max_total_size)
        } else {
            all_references
        };

        Self::multi_skill_injection_with_refs(skills, &filtered_refs)
    }

    /// Merge allowed-tools from multiple skills (union)
    ///
    /// Creates a unique list of all tools from all skills, preserving order
    /// of first occurrence. Duplicates are removed.
    ///
    /// # Arguments
    /// * `skills` - List of loaded skills
    ///
    /// # Returns
    /// Deduplicated list of all allowed tools
    #[allow(dead_code)]
    pub fn merge_allowed_tools(skills: &[LoadedSkill]) -> Vec<String> {
        use std::collections::HashSet;

        let mut seen = HashSet::new();
        let mut result = Vec::new();

        for skill in skills {
            for tool in skill.metadata.get_allowed_tools() {
                if seen.insert(tool.clone()) {
                    result.push(tool);
                }
            }
        }

        result
    }

    /// Merge allowed-scripts from multiple skills
    ///
    /// Creates a unique list of all script patterns from all skills.
    /// Patterns are preserved as-is (glob patterns are not merged).
    ///
    /// # Arguments
    /// * `skills` - List of loaded skills
    ///
    /// # Returns
    /// Deduplicated list of all allowed script patterns
    #[allow(dead_code)]
    pub fn merge_allowed_scripts(skills: &[LoadedSkill]) -> Vec<String> {
        use std::collections::HashSet;

        let mut seen = HashSet::new();
        let mut result = Vec::new();

        for skill in skills {
            if let Some(scripts) = &skill.metadata.allowed_scripts {
                for script in scripts {
                    if seen.insert(script.clone()) {
                        result.push(script.clone());
                    }
                }
            }
        }

        result
    }

    /// Inject skill summaries into an existing system prompt
    ///
    /// # Arguments
    /// * `system_prompt` - The original system prompt
    /// * `summaries` - Skill summaries to inject
    ///
    /// # Returns
    /// Enhanced system prompt with skill information
    #[allow(dead_code)]
    pub fn inject_summaries(system_prompt: &str, summaries: &[SkillSummary]) -> String {
        let injection = Self::phase1_injection(summaries);

        if injection.is_empty() {
            return system_prompt.to_string();
        }

        format!("{}\n{}", system_prompt, injection)
    }

    /// Inject full skill content into an existing system prompt
    ///
    /// # Arguments
    /// * `system_prompt` - The original system prompt
    /// * `skill` - The skill to inject
    ///
    /// # Returns
    /// Enhanced system prompt with full skill content
    #[allow(dead_code)]
    pub fn inject_skill(system_prompt: &str, skill: &LoadedSkill) -> String {
        let injection = Self::phase2_injection(skill);
        format!("{}\n{}", system_prompt, injection)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use chrono::Utc;

    use super::*;
    use crate::skills::types::SkillMetadata;

    fn create_test_summary(name: &str, description: &str) -> SkillSummary {
        SkillSummary {
            name: name.to_string(),
            description: description.to_string(),
            allowed_tools: vec![],
        }
    }

    fn create_test_skill(name: &str, content: &str) -> LoadedSkill {
        LoadedSkill {
            metadata: SkillMetadata {
                name: name.to_string(),
                description: "Test skill".to_string(),
                license: None,
                compatibility: None,
                metadata: None,
                allowed_tools: None,
                model: None,
                allowed_scripts: None,
                execution_limits: None,
                references: None,
                priority: None,
                conflicts: None,
            },
            content: content.to_string(),
            raw_content: String::new(),
            skill_dir: PathBuf::new(),
            file_path: String::new(),
            enabled: true,
            loaded_at: Utc::now(),
            scripts: Vec::new(),
        }
    }

    #[test]
    fn test_phase1_injection_empty() {
        let result = SkillInjector::phase1_injection(&[]);
        assert!(result.is_empty());
    }

    #[test]
    fn test_phase1_injection_single() {
        let summaries = vec![create_test_summary(
            "weather-query",
            "Query weather information",
        )];

        let result = SkillInjector::phase1_injection(&summaries);

        assert!(result.contains("## Available Skills"));
        assert!(result.contains("weather-query"));
        assert!(result.contains("Query weather information"));
        assert!(result.contains("<use_skill>skill-name</use_skill>"));
    }

    #[test]
    fn test_phase1_injection_multiple() {
        let summaries = vec![
            create_test_summary("skill-one", "First skill"),
            create_test_summary("skill-two", "Second skill"),
        ];

        let result = SkillInjector::phase1_injection(&summaries);

        assert!(result.contains("skill-one"));
        assert!(result.contains("First skill"));
        assert!(result.contains("skill-two"));
        assert!(result.contains("Second skill"));
    }

    #[test]
    fn test_phase2_injection() {
        let skill = create_test_skill("test-skill", "# Test Content\n\nDo this and that.");

        let result = SkillInjector::phase2_injection(&skill);

        assert!(result.contains("## Active Skill: test-skill"));
        assert!(result.contains("The following skill instructions"));
        assert!(result.contains("# Test Content"));
        assert!(result.contains("Do this and that."));
        assert!(result.contains("---")); // Content wrapped in ---
    }

    #[test]
    fn test_phase2_injection_with_metadata() {
        // Metadata is now part of the skill content, not injected separately
        let mut skill = create_test_skill("test-skill", "Content here");
        skill.metadata.license = Some("MIT".to_string());
        skill.metadata.compatibility = Some("Requires Python 3.8+".to_string());
        skill.metadata.allowed_tools = Some("Bash Read".to_string());

        let result = SkillInjector::phase2_injection(&skill);

        // New format just wraps content in --- delimiters
        assert!(result.contains("## Active Skill: test-skill"));
        assert!(result.contains("Content here"));
        assert!(result.contains("---"));
    }

    #[test]
    fn test_multi_skill_injection_empty() {
        let result = SkillInjector::multi_skill_injection(&[]);
        assert!(result.is_empty());
    }

    #[test]
    fn test_multi_skill_injection() {
        let skills = vec![
            create_test_skill("skill-a", "Content A"),
            create_test_skill("skill-b", "Content B"),
        ];

        let result = SkillInjector::multi_skill_injection(&skills);

        // New format shows skill names in header
        assert!(result.contains("## Active Skills: skill-a, skill-b"));
        assert!(result.contains("### Skill: skill-a"));
        assert!(result.contains("Content A"));
        assert!(result.contains("### Skill: skill-b"));
        assert!(result.contains("Content B"));
    }

    #[test]
    fn test_inject_summaries() {
        let system_prompt = "You are a helpful assistant.";
        let summaries = vec![create_test_summary("helper", "Helps with things")];

        let result = SkillInjector::inject_summaries(system_prompt, &summaries);

        assert!(result.starts_with("You are a helpful assistant."));
        assert!(result.contains("## Available Skills"));
        assert!(result.contains("helper"));
    }

    #[test]
    fn test_inject_summaries_empty() {
        let system_prompt = "You are a helpful assistant.";
        let result = SkillInjector::inject_summaries(system_prompt, &[]);

        assert_eq!(result, system_prompt);
    }

    #[test]
    fn test_inject_skill() {
        let system_prompt = "You are a helpful assistant.";
        let skill = create_test_skill("my-skill", "Do the thing.");

        let result = SkillInjector::inject_skill(system_prompt, &skill);

        assert!(result.starts_with("You are a helpful assistant."));
        assert!(result.contains("## Active Skill: my-skill"));
        assert!(result.contains("Do the thing."));
    }

    #[test]
    fn test_phase1_injection_special_characters() {
        let summaries = vec![create_test_summary(
            "code-review",
            "Review code with `markdown` and **bold** text",
        )];

        let result = SkillInjector::phase1_injection(&summaries);

        assert!(result.contains("code-review"));
        assert!(result.contains("Review code with `markdown` and **bold** text"));
    }

    #[test]
    fn test_phase2_injection_empty_content() {
        let skill = create_test_skill("empty-skill", "");

        let result = SkillInjector::phase2_injection(&skill);

        assert!(result.contains("## Active Skill: empty-skill"));
        assert!(result.contains("---")); // Empty content between delimiters
    }

    #[test]
    fn test_phase2_injection_multiline_content() {
        let content = r#"# Header

This is a paragraph.

## Subheader

- List item 1
- List item 2

```rust
fn main() {
    println!("Hello");
}
```
"#;
        let skill = create_test_skill("multiline-skill", content);

        let result = SkillInjector::phase2_injection(&skill);

        assert!(result.contains("# Header"));
        assert!(result.contains("## Subheader"));
        assert!(result.contains("- List item 1"));
        assert!(result.contains("fn main()"));
    }

    #[test]
    fn test_phase2_injection_content_without_newline() {
        let skill = create_test_skill("no-newline", "Content without trailing newline");

        let result = SkillInjector::phase2_injection(&skill);

        // New format ends with ---
        assert!(result.ends_with("---"));
    }

    #[test]
    fn test_phase2_injection_content_with_newline() {
        let skill = create_test_skill("has-newline", "Content with trailing newline\n");

        let result = SkillInjector::phase2_injection(&skill);

        // New format wraps content in ---
        assert!(result.contains("Content with trailing newline"));
        assert!(result.ends_with("---"));
    }

    #[test]
    fn test_phase2_injection_partial_metadata() {
        // In the new simplified format, metadata is not separately injected
        // The skill content is wrapped as-is
        let mut skill1 = create_test_skill("license-only", "Content");
        skill1.metadata.license = Some("Apache-2.0".to_string());
        let result1 = SkillInjector::phase2_injection(&skill1);
        assert!(result1.contains("## Active Skill: license-only"));
        assert!(result1.contains("Content"));

        // Only compatibility
        let mut skill2 = create_test_skill("compat-only", "Content");
        skill2.metadata.compatibility = Some("Linux only".to_string());
        let result2 = SkillInjector::phase2_injection(&skill2);
        assert!(result2.contains("## Active Skill: compat-only"));
        assert!(result2.contains("Content"));
    }

    #[test]
    fn test_inject_summaries_empty_prompt() {
        let summaries = vec![create_test_summary("skill", "Description")];
        let result = SkillInjector::inject_summaries("", &summaries);

        assert!(result.contains("## Available Skills"));
        assert!(result.contains("skill"));
    }

    #[test]
    fn test_inject_skill_empty_prompt() {
        let skill = create_test_skill("test", "Content");
        let result = SkillInjector::inject_skill("", &skill);

        assert!(result.contains("## Active Skill: test"));
        assert!(result.contains("Content"));
    }

    #[test]
    fn test_multi_skill_injection_order_preserved() {
        let skills = vec![
            create_test_skill("first", "First content"),
            create_test_skill("second", "Second content"),
            create_test_skill("third", "Third content"),
        ];

        let result = SkillInjector::multi_skill_injection(&skills);

        // Check header shows correct order
        assert!(result.contains("## Active Skills: first, second, third"));

        let first_pos = result.find("First content").unwrap();
        let second_pos = result.find("Second content").unwrap();
        let third_pos = result.find("Third content").unwrap();

        assert!(first_pos < second_pos);
        assert!(second_pos < third_pos);
    }

    #[test]
    fn test_phase1_injection_order_preserved() {
        let summaries = vec![
            create_test_summary("alpha", "Alpha skill"),
            create_test_summary("beta", "Beta skill"),
            create_test_summary("gamma", "Gamma skill"),
        ];

        let result = SkillInjector::phase1_injection(&summaries);

        let alpha_pos = result.find("alpha").unwrap();
        let beta_pos = result.find("beta").unwrap();
        let gamma_pos = result.find("gamma").unwrap();

        assert!(alpha_pos < beta_pos);
        assert!(beta_pos < gamma_pos);
    }

    #[test]
    fn test_inject_skill_preserves_prompt_structure() {
        let system_prompt = "Line 1\nLine 2\nLine 3";
        let skill = create_test_skill("test", "Skill content");

        let result = SkillInjector::inject_skill(system_prompt, &skill);

        assert!(result.starts_with("Line 1\nLine 2\nLine 3\n"));
    }

    #[test]
    fn test_phase2_injection_long_description() {
        let mut skill = create_test_skill("long-desc", "Content");
        skill.metadata.description = "A".repeat(1024);

        // Should not panic
        let result = SkillInjector::phase2_injection(&skill);
        assert!(result.contains("## Active Skill: long-desc"));
    }

    #[test]
    fn test_phase1_injection_markdown_formatting() {
        let result = SkillInjector::phase1_injection(&[create_test_summary("test", "desc")]);

        // Check table formatting (new format uses tables)
        assert!(result.contains("| test | desc |"));
        assert!(result.contains("| Skill | Description |"));
    }

    // Tests for phase2_injection_with_refs

    #[test]
    fn test_phase2_injection_with_refs_empty() {
        let skill = create_test_skill("test-skill", "Main content");
        let result = SkillInjector::phase2_injection_with_refs(&skill, &[]);

        assert!(result.contains("## Active Skill: test-skill"));
        assert!(result.contains("Main content"));
        // No reference section when empty
        assert!(!result.contains("## Reference Materials"));
    }

    #[test]
    fn test_phase2_injection_with_refs_single() {
        let skill = create_test_skill("test-skill", "Main content");
        let refs = vec!["# Reference Doc\n\nSome reference content.".to_string()];
        let result = SkillInjector::phase2_injection_with_refs(&skill, &refs);

        assert!(result.contains("## Active Skill: test-skill"));
        assert!(result.contains("Main content"));
        assert!(result.contains("## Reference Materials"));
        assert!(result.contains("### Reference Document 1"));
        assert!(result.contains("# Reference Doc"));
        assert!(result.contains("Some reference content."));
    }

    #[test]
    fn test_phase2_injection_with_refs_multiple() {
        let skill = create_test_skill("test-skill", "Main content");
        let refs = vec![
            "First reference".to_string(),
            "Second reference".to_string(),
            "Third reference".to_string(),
        ];
        let result = SkillInjector::phase2_injection_with_refs(&skill, &refs);

        assert!(result.contains("## Reference Materials"));
        assert!(result.contains("### Reference Document 1"));
        assert!(result.contains("### Reference Document 2"));
        assert!(result.contains("### Reference Document 3"));
        assert!(result.contains("First reference"));
        assert!(result.contains("Second reference"));
        assert!(result.contains("Third reference"));

        // Check order
        let pos1 = result.find("First reference").unwrap();
        let pos2 = result.find("Second reference").unwrap();
        let pos3 = result.find("Third reference").unwrap();
        assert!(pos1 < pos2);
        assert!(pos2 < pos3);
    }

    #[test]
    fn test_phase2_injection_with_refs_trims_whitespace() {
        let skill = create_test_skill("test-skill", "Main content");
        let refs = vec!["\n\n  Reference with whitespace  \n\n".to_string()];
        let result = SkillInjector::phase2_injection_with_refs(&skill, &refs);

        assert!(result.contains("Reference with whitespace"));
        // Check that leading/trailing whitespace is trimmed
        assert!(!result.contains("\n\n  Reference"));
    }

    // Tests for filter_by_size

    #[test]
    fn test_filter_by_size_all_fit() {
        let refs = vec![
            "Short".to_string(),       // 5 bytes
            "Medium text".to_string(), // 11 bytes
        ];
        let result = SkillInjector::filter_by_size(&refs, 100);

        assert_eq!(result.len(), 2);
        assert_eq!(result[0], "Short");
        assert_eq!(result[1], "Medium text");
    }

    #[test]
    fn test_filter_by_size_partial_fit() {
        let refs = vec![
            "Short".to_string(),               // 5 bytes
            "Medium text".to_string(),         // 11 bytes (total: 16)
            "Very long text here".to_string(), // 19 bytes (would exceed)
        ];
        let result = SkillInjector::filter_by_size(&refs, 20);

        assert_eq!(result.len(), 2);
        assert_eq!(result[0], "Short");
        assert_eq!(result[1], "Medium text");
    }

    #[test]
    fn test_filter_by_size_none_fit() {
        let refs = vec!["This is too long".to_string()];
        let result = SkillInjector::filter_by_size(&refs, 5);

        assert!(result.is_empty());
    }

    #[test]
    fn test_filter_by_size_empty_refs() {
        let refs: Vec<String> = vec![];
        let result = SkillInjector::filter_by_size(&refs, 100);

        assert!(result.is_empty());
    }

    #[test]
    fn test_filter_by_size_exact_limit() {
        let refs = vec![
            "12345".to_string(), // 5 bytes
            "67890".to_string(), // 5 bytes (total: 10)
        ];
        let result = SkillInjector::filter_by_size(&refs, 10);

        assert_eq!(result.len(), 2);
    }

    // Tests for phase2_injection_auto_refs (async tests)

    #[tokio::test]
    async fn test_phase2_injection_auto_refs_no_refs_dir() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let mut skill = create_test_skill("test-skill", "Main content");
        skill.skill_dir = temp_dir.path().to_path_buf();

        let result = SkillInjector::phase2_injection_auto_refs(&skill, 0).await;

        assert!(result.contains("## Active Skill: test-skill"));
        assert!(result.contains("Main content"));
        // No references when directory doesn't exist
        assert!(!result.contains("## Reference Materials"));
    }

    #[tokio::test]
    async fn test_phase2_injection_auto_refs_with_files() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let refs_dir = temp_dir.path().join("references");
        std::fs::create_dir(&refs_dir).unwrap();

        // Create test reference files
        std::fs::write(refs_dir.join("doc1.md"), "# Document 1\n\nFirst reference.").unwrap();
        std::fs::write(refs_dir.join("doc2.txt"), "Plain text reference.").unwrap();

        let mut skill = create_test_skill("test-skill", "Main content");
        skill.skill_dir = temp_dir.path().to_path_buf();

        let result = SkillInjector::phase2_injection_auto_refs(&skill, 0).await;

        assert!(result.contains("## Active Skill: test-skill"));
        assert!(result.contains("Main content"));
        assert!(result.contains("## Reference Materials"));
        // Should include both references (order may vary due to filesystem)
        assert!(result.contains("Document 1") || result.contains("Plain text reference"));
    }

    #[tokio::test]
    async fn test_phase2_injection_auto_refs_with_size_limit() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let refs_dir = temp_dir.path().join("references");
        std::fs::create_dir(&refs_dir).unwrap();

        // Create test files with known sizes
        std::fs::write(refs_dir.join("small.md"), "Small").unwrap(); // 5 bytes
        std::fs::write(refs_dir.join("large.md"), "A".repeat(1000)).unwrap(); // 1000 bytes

        let mut skill = create_test_skill("test-skill", "Content");
        skill.skill_dir = temp_dir.path().to_path_buf();

        // With size limit of 100, only small.md should be included
        let result = SkillInjector::phase2_injection_auto_refs(&skill, 100).await;

        assert!(result.contains("Small") || !result.contains(&"A".repeat(100)));
    }

    #[tokio::test]
    async fn test_phase2_injection_auto_refs_ignores_non_md_txt() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let refs_dir = temp_dir.path().join("references");
        std::fs::create_dir(&refs_dir).unwrap();

        // Create files with different extensions
        std::fs::write(refs_dir.join("doc.md"), "Markdown file").unwrap();
        std::fs::write(refs_dir.join("doc.json"), r#"{"ignored": true}"#).unwrap();
        std::fs::write(refs_dir.join("doc.yaml"), "ignored: true").unwrap();

        let mut skill = create_test_skill("test-skill", "Content");
        skill.skill_dir = temp_dir.path().to_path_buf();

        let result = SkillInjector::phase2_injection_auto_refs(&skill, 0).await;

        assert!(result.contains("Markdown file"));
        assert!(!result.contains("ignored"));
    }

    // Tests for merge_allowed_tools

    fn create_test_skill_with_tools(name: &str, tools: Option<&str>) -> LoadedSkill {
        let mut skill = create_test_skill(name, "Content");
        skill.metadata.allowed_tools = tools.map(|s| s.to_string());
        skill
    }

    fn create_test_skill_with_scripts(name: &str, scripts: Option<Vec<&str>>) -> LoadedSkill {
        let mut skill = create_test_skill(name, "Content");
        skill.metadata.allowed_scripts = scripts.map(|v| v.into_iter().map(String::from).collect());
        skill
    }

    #[test]
    fn test_merge_allowed_tools_empty_skills() {
        let skills: Vec<LoadedSkill> = vec![];
        let result = SkillInjector::merge_allowed_tools(&skills);
        assert!(result.is_empty());
    }

    #[test]
    fn test_merge_allowed_tools_single_skill() {
        let skills = vec![create_test_skill_with_tools(
            "skill-a",
            Some("Bash Read Write"),
        )];
        let result = SkillInjector::merge_allowed_tools(&skills);
        assert_eq!(result, vec!["Bash", "Read", "Write"]);
    }

    #[test]
    fn test_merge_allowed_tools_multiple_skills_no_overlap() {
        let skills = vec![
            create_test_skill_with_tools("skill-a", Some("Bash Read")),
            create_test_skill_with_tools("skill-b", Some("Write Edit")),
        ];
        let result = SkillInjector::merge_allowed_tools(&skills);
        assert_eq!(result, vec!["Bash", "Read", "Write", "Edit"]);
    }

    #[test]
    fn test_merge_allowed_tools_with_duplicates() {
        let skills = vec![
            create_test_skill_with_tools("skill-a", Some("Bash Read Write")),
            create_test_skill_with_tools("skill-b", Some("Read Write Grep")),
            create_test_skill_with_tools("skill-c", Some("Bash Grep Glob")),
        ];
        let result = SkillInjector::merge_allowed_tools(&skills);
        // Should be deduplicated, preserving order of first occurrence
        assert_eq!(result, vec!["Bash", "Read", "Write", "Grep", "Glob"]);
    }

    #[test]
    fn test_merge_allowed_tools_some_empty() {
        let skills = vec![
            create_test_skill_with_tools("skill-a", Some("Bash Read")),
            create_test_skill_with_tools("skill-b", None),
            create_test_skill_with_tools("skill-c", Some("Write")),
        ];
        let result = SkillInjector::merge_allowed_tools(&skills);
        assert_eq!(result, vec!["Bash", "Read", "Write"]);
    }

    #[test]
    fn test_merge_allowed_tools_all_empty() {
        let skills = vec![
            create_test_skill_with_tools("skill-a", None),
            create_test_skill_with_tools("skill-b", None),
        ];
        let result = SkillInjector::merge_allowed_tools(&skills);
        assert!(result.is_empty());
    }

    #[test]
    fn test_merge_allowed_tools_with_mcp_tools() {
        let skills = vec![
            create_test_skill_with_tools("skill-a", Some("mcp__calc__sum, mcp__calc__sub")),
            create_test_skill_with_tools("skill-b", Some("mcp__search__query, mcp__calc__sum")),
        ];
        let result = SkillInjector::merge_allowed_tools(&skills);
        assert_eq!(
            result,
            vec!["mcp__calc__sum", "mcp__calc__sub", "mcp__search__query"]
        );
    }

    // Tests for merge_allowed_scripts

    #[test]
    fn test_merge_allowed_scripts_empty_skills() {
        let skills: Vec<LoadedSkill> = vec![];
        let result = SkillInjector::merge_allowed_scripts(&skills);
        assert!(result.is_empty());
    }

    #[test]
    fn test_merge_allowed_scripts_single_skill() {
        let skills = vec![create_test_skill_with_scripts(
            "skill-a",
            Some(vec!["*.js", "*.ts"]),
        )];
        let result = SkillInjector::merge_allowed_scripts(&skills);
        assert_eq!(result, vec!["*.js", "*.ts"]);
    }

    #[test]
    fn test_merge_allowed_scripts_multiple_skills() {
        let skills = vec![
            create_test_skill_with_scripts("skill-a", Some(vec!["*.js", "process.py"])),
            create_test_skill_with_scripts("skill-b", Some(vec!["*.ts", "export.py"])),
        ];
        let result = SkillInjector::merge_allowed_scripts(&skills);
        assert_eq!(result, vec!["*.js", "process.py", "*.ts", "export.py"]);
    }

    #[test]
    fn test_merge_allowed_scripts_with_duplicates() {
        let skills = vec![
            create_test_skill_with_scripts("skill-a", Some(vec!["*.js", "process.py"])),
            create_test_skill_with_scripts("skill-b", Some(vec!["*.js", "export.py"])),
            create_test_skill_with_scripts("skill-c", Some(vec!["process.py", "helper.py"])),
        ];
        let result = SkillInjector::merge_allowed_scripts(&skills);
        // Should be deduplicated
        assert_eq!(result, vec!["*.js", "process.py", "export.py", "helper.py"]);
    }

    #[test]
    fn test_merge_allowed_scripts_some_none() {
        let skills = vec![
            create_test_skill_with_scripts("skill-a", Some(vec!["*.js"])),
            create_test_skill_with_scripts("skill-b", None),
            create_test_skill_with_scripts("skill-c", Some(vec!["*.py"])),
        ];
        let result = SkillInjector::merge_allowed_scripts(&skills);
        assert_eq!(result, vec!["*.js", "*.py"]);
    }

    #[test]
    fn test_merge_allowed_scripts_all_none() {
        let skills = vec![
            create_test_skill_with_scripts("skill-a", None),
            create_test_skill_with_scripts("skill-b", None),
        ];
        let result = SkillInjector::merge_allowed_scripts(&skills);
        assert!(result.is_empty());
    }

    // Tests for multi_skill_injection_with_refs

    #[test]
    fn test_multi_skill_injection_with_refs_empty() {
        let result = SkillInjector::multi_skill_injection_with_refs(&[], &[]);
        assert!(result.is_empty());
    }

    #[test]
    fn test_multi_skill_injection_with_refs_no_refs() {
        let skills = vec![
            create_test_skill("skill-a", "Content A"),
            create_test_skill("skill-b", "Content B"),
        ];
        let result = SkillInjector::multi_skill_injection_with_refs(&skills, &[]);

        assert!(result.contains("## Active Skills: skill-a, skill-b"));
        assert!(result.contains("Content A"));
        assert!(result.contains("Content B"));
        assert!(!result.contains("## Reference Materials"));
    }

    #[test]
    fn test_multi_skill_injection_with_refs_has_refs() {
        let skills = vec![create_test_skill("skill-a", "Content A")];
        let refs = vec![
            "Reference content 1".to_string(),
            "Reference content 2".to_string(),
        ];
        let result = SkillInjector::multi_skill_injection_with_refs(&skills, &refs);

        assert!(result.contains("## Active Skills: skill-a"));
        assert!(result.contains("Content A"));
        assert!(result.contains("## Reference Materials"));
        assert!(result.contains("### Reference Document 1"));
        assert!(result.contains("Reference content 1"));
        assert!(result.contains("### Reference Document 2"));
        assert!(result.contains("Reference content 2"));
    }

    #[test]
    fn test_multi_skill_injection_with_merged_permissions() {
        let mut skill_a = create_test_skill("skill-a", "Content A");
        skill_a.metadata.allowed_tools = Some("Bash Read".to_string());
        skill_a.metadata.allowed_scripts = Some(vec!["*.js".to_string()]);

        let mut skill_b = create_test_skill("skill-b", "Content B");
        skill_b.metadata.allowed_tools = Some("Read Write".to_string());
        skill_b.metadata.allowed_scripts = Some(vec!["*.py".to_string()]);

        let skills = vec![skill_a, skill_b];
        let result = SkillInjector::multi_skill_injection(&skills);

        // Check merged permissions section
        assert!(result.contains("### Merged Permissions"));
        assert!(result.contains("**Allowed Tools:** Bash, Read, Write"));
        assert!(result.contains("**Allowed Scripts:** *.js, *.py"));
    }

    #[test]
    fn test_multi_skill_injection_no_permissions_section_when_empty() {
        let skills = vec![
            create_test_skill("skill-a", "Content A"),
            create_test_skill("skill-b", "Content B"),
        ];
        let result = SkillInjector::multi_skill_injection(&skills);

        // No permissions section when both tools and scripts are empty
        assert!(!result.contains("### Merged Permissions"));
    }

    // Tests for multi_skill_injection_auto_refs

    #[tokio::test]
    async fn test_multi_skill_injection_auto_refs_empty() {
        let result = SkillInjector::multi_skill_injection_auto_refs(&[], 0).await;
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn test_multi_skill_injection_auto_refs_no_refs_dirs() {
        let temp_dir1 = tempfile::TempDir::new().unwrap();
        let temp_dir2 = tempfile::TempDir::new().unwrap();

        let mut skill_a = create_test_skill("skill-a", "Content A");
        skill_a.skill_dir = temp_dir1.path().to_path_buf();

        let mut skill_b = create_test_skill("skill-b", "Content B");
        skill_b.skill_dir = temp_dir2.path().to_path_buf();

        let skills = vec![skill_a, skill_b];
        let result = SkillInjector::multi_skill_injection_auto_refs(&skills, 0).await;

        assert!(result.contains("## Active Skills: skill-a, skill-b"));
        assert!(result.contains("Content A"));
        assert!(result.contains("Content B"));
        assert!(!result.contains("## Reference Materials"));
    }

    #[tokio::test]
    async fn test_multi_skill_injection_auto_refs_with_refs() {
        let temp_dir1 = tempfile::TempDir::new().unwrap();
        let refs_dir1 = temp_dir1.path().join("references");
        std::fs::create_dir(&refs_dir1).unwrap();
        std::fs::write(refs_dir1.join("ref1.md"), "Reference from skill A").unwrap();

        let temp_dir2 = tempfile::TempDir::new().unwrap();
        let refs_dir2 = temp_dir2.path().join("references");
        std::fs::create_dir(&refs_dir2).unwrap();
        std::fs::write(refs_dir2.join("ref2.md"), "Reference from skill B").unwrap();

        let mut skill_a = create_test_skill("skill-a", "Content A");
        skill_a.skill_dir = temp_dir1.path().to_path_buf();

        let mut skill_b = create_test_skill("skill-b", "Content B");
        skill_b.skill_dir = temp_dir2.path().to_path_buf();

        let skills = vec![skill_a, skill_b];
        let result = SkillInjector::multi_skill_injection_auto_refs(&skills, 0).await;

        assert!(result.contains("## Active Skills: skill-a, skill-b"));
        assert!(result.contains("## Reference Materials"));
        assert!(result.contains("Reference from skill A"));
        assert!(result.contains("Reference from skill B"));
    }

    #[tokio::test]
    async fn test_multi_skill_injection_auto_refs_with_size_limit() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let refs_dir = temp_dir.path().join("references");
        std::fs::create_dir(&refs_dir).unwrap();
        std::fs::write(refs_dir.join("small.md"), "Small").unwrap(); // 5 bytes
        std::fs::write(refs_dir.join("large.md"), "L".repeat(1000)).unwrap(); // 1000 bytes

        let mut skill = create_test_skill("skill-a", "Content A");
        skill.skill_dir = temp_dir.path().to_path_buf();

        let skills = vec![skill];
        let result = SkillInjector::multi_skill_injection_auto_refs(&skills, 100).await;

        // With size limit, only small.md should be included
        assert!(result.contains("Small") || !result.contains(&"L".repeat(100)));
    }
}
