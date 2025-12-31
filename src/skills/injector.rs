//! Skill Injector for prompt enhancement
//!
//! Injects skill information into system prompts:
//! - Phase 1: Skill summaries (name + description)
//! - Phase 2: Full skill content when activated

use crate::skills::types::{LoadedSkill, SkillSummary};

/// Skill injector for enhancing system prompts
pub struct SkillInjector;

impl SkillInjector {
    /// Generate Phase 1 injection text with skill summaries
    ///
    /// Creates a formatted list of available skills with their descriptions
    /// for the LLM to consider when planning.
    ///
    /// # Arguments
    /// * `summaries` - List of skill summaries to inject
    ///
    /// # Returns
    /// Formatted markdown text for system prompt injection
    pub fn phase1_injection(summaries: &[SkillSummary]) -> String {
        if summaries.is_empty() {
            return String::new();
        }

        let mut output = String::from("\n## Available Skills\n\n");
        output.push_str("The following skills are available. To use a skill, output `<use_skill>skill-name</use_skill>`.\n\n");

        for summary in summaries {
            output.push_str(&format!(
                "- **{}**: {}\n",
                summary.name, summary.description
            ));
        }

        output.push('\n');
        output
    }

    /// Generate Phase 2 injection text with full skill content
    ///
    /// Injects the complete SKILL.md content for activated skills.
    ///
    /// # Arguments
    /// * `skill` - The fully loaded skill to inject
    ///
    /// # Returns
    /// Formatted text for system prompt injection
    pub fn phase2_injection(skill: &LoadedSkill) -> String {
        let mut output = format!("\n## Skill: {}\n\n", skill.metadata.name);

        // Add metadata section if there are additional fields
        if skill.metadata.license.is_some()
            || skill.metadata.compatibility.is_some()
            || skill.metadata.allowed_tools.is_some()
        {
            output.push_str("### Metadata\n\n");

            if let Some(license) = &skill.metadata.license {
                output.push_str(&format!("- **License**: {}\n", license));
            }

            if let Some(compatibility) = &skill.metadata.compatibility {
                output.push_str(&format!("- **Compatibility**: {}\n", compatibility));
            }

            if let Some(allowed_tools) = &skill.metadata.allowed_tools {
                output.push_str(&format!("- **Allowed Tools**: {}\n", allowed_tools));
            }

            output.push('\n');
        }

        // Add the main content
        output.push_str("### Instructions\n\n");
        output.push_str(&skill.content);

        if !skill.content.ends_with('\n') {
            output.push('\n');
        }

        output
    }

    /// Generate injection text for multiple skills
    ///
    /// # Arguments
    /// * `skills` - List of loaded skills to inject
    ///
    /// # Returns
    /// Combined injection text for all skills
    pub fn multi_skill_injection(skills: &[LoadedSkill]) -> String {
        if skills.is_empty() {
            return String::new();
        }

        let mut output = String::from("\n## Active Skills\n\n");

        for skill in skills {
            output.push_str(&Self::phase2_injection(skill));
        }

        output
    }

    /// Inject skill summaries into an existing system prompt
    ///
    /// # Arguments
    /// * `system_prompt` - The original system prompt
    /// * `summaries` - Skill summaries to inject
    ///
    /// # Returns
    /// Enhanced system prompt with skill information
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
            },
            content: content.to_string(),
            raw_content: String::new(),
            skill_dir: PathBuf::new(),
            file_path: String::new(),
            enabled: true,
            loaded_at: Utc::now(),
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

        assert!(result.contains("## Skill: test-skill"));
        assert!(result.contains("### Instructions"));
        assert!(result.contains("# Test Content"));
        assert!(result.contains("Do this and that."));
    }

    #[test]
    fn test_phase2_injection_with_metadata() {
        let mut skill = create_test_skill("test-skill", "Content here");
        skill.metadata.license = Some("MIT".to_string());
        skill.metadata.compatibility = Some("Requires Python 3.8+".to_string());
        skill.metadata.allowed_tools = Some("Bash Read".to_string());

        let result = SkillInjector::phase2_injection(&skill);

        assert!(result.contains("### Metadata"));
        assert!(result.contains("**License**: MIT"));
        assert!(result.contains("**Compatibility**: Requires Python 3.8+"));
        assert!(result.contains("**Allowed Tools**: Bash Read"));
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

        assert!(result.contains("## Active Skills"));
        assert!(result.contains("skill-a"));
        assert!(result.contains("Content A"));
        assert!(result.contains("skill-b"));
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
        assert!(result.contains("## Skill: my-skill"));
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

        assert!(result.contains("## Skill: empty-skill"));
        assert!(result.contains("### Instructions"));
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

        // Should add trailing newline
        assert!(result.ends_with('\n'));
    }

    #[test]
    fn test_phase2_injection_content_with_newline() {
        let skill = create_test_skill("has-newline", "Content with trailing newline\n");

        let result = SkillInjector::phase2_injection(&skill);

        // Should not add double newline
        assert!(!result.ends_with("\n\n"));
    }

    #[test]
    fn test_phase2_injection_partial_metadata() {
        // Only license
        let mut skill1 = create_test_skill("license-only", "Content");
        skill1.metadata.license = Some("Apache-2.0".to_string());
        let result1 = SkillInjector::phase2_injection(&skill1);
        assert!(result1.contains("### Metadata"));
        assert!(result1.contains("**License**: Apache-2.0"));
        assert!(!result1.contains("**Compatibility**"));

        // Only compatibility
        let mut skill2 = create_test_skill("compat-only", "Content");
        skill2.metadata.compatibility = Some("Linux only".to_string());
        let result2 = SkillInjector::phase2_injection(&skill2);
        assert!(result2.contains("### Metadata"));
        assert!(result2.contains("**Compatibility**: Linux only"));
        assert!(!result2.contains("**License**"));
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

        assert!(result.contains("## Skill: test"));
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
        assert!(result.contains("## Skill: long-desc"));
    }

    #[test]
    fn test_phase1_injection_markdown_formatting() {
        let result = SkillInjector::phase1_injection(&[create_test_summary("test", "desc")]);

        // Check markdown formatting
        assert!(result.contains("- **test**: desc"));
    }
}
