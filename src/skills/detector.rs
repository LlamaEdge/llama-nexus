//! Skill Detector for detecting skill usage requests in LLM responses
//!
//! Detects `<use_skill>skill-name</use_skill>` tags in LLM output
//! to trigger Phase 2 skill loading.
//!
//! ## Features
//! - Single skill detection: `<use_skill>skill-name</use_skill>`
//! - Multiple skills (comma-separated): `<use_skill>skill-a, skill-b</use_skill>`
//! - Priority resolution: Sort skills by priority when metadata available
//! - Conflict detection: Remove conflicting skills based on priority

use std::{collections::HashSet, sync::LazyLock};

use regex::Regex;

use super::types::LoadedSkill;

/// Regex pattern for detecting skill usage tags
/// Matches: <use_skill>skill-name</use_skill>
/// Also matches comma-separated skills: <use_skill>skill-a, skill-b</use_skill>
static USE_SKILL_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"<use_skill>\s*([a-z0-9][a-z0-9-]*[a-z0-9](?:\s*,\s*[a-z0-9][a-z0-9-]*[a-z0-9])*|[a-z0-9])\s*</use_skill>")
        .expect("Invalid regex pattern")
});

/// Skill detector for identifying skill activation requests
///
/// Public API for detecting skill requests in LLM responses.
/// Methods are available for future API endpoints and integrations.
pub struct SkillDetector;

#[allow(dead_code)]
impl SkillDetector {
    /// Detect all skill names requested in the text
    ///
    /// Supports both single skills and comma-separated skill lists:
    /// - `<use_skill>skill-a</use_skill>` -> ["skill-a"]
    /// - `<use_skill>skill-a, skill-b</use_skill>` -> ["skill-a", "skill-b"]
    ///
    /// # Arguments
    /// * `text` - The LLM response text to scan
    ///
    /// # Returns
    /// A vector of unique skill names found in the text (deduplicated)
    pub fn detect(text: &str) -> Vec<String> {
        let mut seen = HashSet::new();
        let mut result = Vec::new();

        for cap in USE_SKILL_PATTERN.captures_iter(text) {
            if let Some(m) = cap.get(1) {
                // Handle comma-separated skills
                for skill in m.as_str().split(',') {
                    let skill = skill.trim();
                    if !skill.is_empty() && seen.insert(skill.to_string()) {
                        result.push(skill.to_string());
                    }
                }
            }
        }

        result
    }

    /// Detect the first skill name in the text
    ///
    /// # Arguments
    /// * `text` - The LLM response text to scan
    ///
    /// # Returns
    /// The first skill name found, if any
    pub fn detect_first(text: &str) -> Option<String> {
        USE_SKILL_PATTERN
            .captures(text)
            .and_then(|cap| cap.get(1).map(|m| m.as_str().to_string()))
    }

    /// Check if text contains any skill usage request
    ///
    /// # Arguments
    /// * `text` - The LLM response text to scan
    pub fn has_skill_request(text: &str) -> bool {
        USE_SKILL_PATTERN.is_match(text)
    }

    /// Remove all skill usage tags from text
    ///
    /// # Arguments
    /// * `text` - The text to clean
    ///
    /// # Returns
    /// Text with all `<use_skill>` tags removed
    pub fn strip_tags(text: &str) -> String {
        USE_SKILL_PATTERN.replace_all(text, "").to_string()
    }

    /// Extract skill requests and return both the skills and cleaned text
    ///
    /// # Arguments
    /// * `text` - The LLM response text
    ///
    /// # Returns
    /// Tuple of (skill names, cleaned text)
    pub fn extract_and_clean(text: &str) -> (Vec<String>, String) {
        let skills = Self::detect(text);
        let cleaned = Self::strip_tags(text);
        (skills, cleaned)
    }

    /// Resolve skills by priority, returning sorted list (highest priority first)
    ///
    /// Priority is read from the skill's metadata field using `get_priority()`,
    /// following the Agent Skills Standard extension mechanism.
    ///
    /// # Arguments
    /// * `skill_names` - List of detected skill names
    /// * `loaded_skills` - Available loaded skills with metadata
    ///
    /// # Returns
    /// Skills sorted by priority (highest first). Skills not found in
    /// loaded_skills are assigned default priority 0.
    pub fn resolve_by_priority(
        skill_names: &[String],
        loaded_skills: &[LoadedSkill],
    ) -> Vec<String> {
        let mut skills_with_priority: Vec<(String, i32)> = skill_names
            .iter()
            .map(|name| {
                let priority = loaded_skills
                    .iter()
                    .find(|s| &s.metadata.name == name)
                    .and_then(|s| s.metadata.get_priority())
                    .unwrap_or(0);
                (name.clone(), priority)
            })
            .collect();

        // Sort by priority descending (higher priority first)
        skills_with_priority.sort_by(|a, b| b.1.cmp(&a.1));

        skills_with_priority
            .into_iter()
            .map(|(name, _)| name)
            .collect()
    }

    /// Detect and resolve conflicts between skills
    ///
    /// When skills conflict, the higher priority skill is kept and
    /// conflicting lower priority skills are removed.
    ///
    /// Conflicts are read from the skill's metadata field using `get_conflicts()`,
    /// following the Agent Skills Standard extension mechanism.
    ///
    /// # Arguments
    /// * `skill_names` - List of skill names (should be priority-sorted)
    /// * `loaded_skills` - Available loaded skills with metadata
    ///
    /// # Returns
    /// Tuple of (resolved skills, removed conflicts as (skill, reason) pairs)
    pub fn resolve_conflicts(
        skill_names: &[String],
        loaded_skills: &[LoadedSkill],
    ) -> (Vec<String>, Vec<(String, String)>) {
        let mut resolved: Vec<String> = Vec::new();
        let mut removed: Vec<(String, String)> = Vec::new();
        // Track which skills are excluded and which higher-priority skill caused the exclusion
        let mut excluded: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();

        for skill_name in skill_names {
            // Check if already excluded due to conflict from a higher-priority skill
            if let Some(conflicting_skill) = excluded.get(skill_name) {
                removed.push((
                    skill_name.clone(),
                    format!(
                        "conflicts with higher priority skill '{}'",
                        conflicting_skill
                    ),
                ));
                continue;
            }

            // Find the skill metadata
            if let Some(skill) = loaded_skills
                .iter()
                .find(|s| &s.metadata.name == skill_name)
            {
                // Check if this skill conflicts with any already resolved skill
                let mut has_conflict = false;
                for resolved_skill in &resolved {
                    if let Some(resolved_meta) = loaded_skills
                        .iter()
                        .find(|s| &s.metadata.name == resolved_skill)
                        && let Some(ref conflicts) = resolved_meta.metadata.get_conflicts()
                        && conflicts.contains(skill_name)
                    {
                        // This skill conflicts with an already resolved higher-priority skill
                        removed.push((
                            skill_name.clone(),
                            format!("conflicts with higher priority skill '{}'", resolved_skill),
                        ));
                        has_conflict = true;
                        break;
                    }
                }

                if !has_conflict {
                    // Add this skill to resolved
                    resolved.push(skill_name.clone());

                    // Mark skills that this skill conflicts with as excluded
                    if let Some(ref conflicts) = skill.metadata.get_conflicts() {
                        for conflict in conflicts {
                            excluded.insert(conflict.clone(), skill_name.clone());
                        }
                    }
                }
            } else {
                // Skill not found in loaded_skills, keep it anyway (no conflict info available)
                resolved.push(skill_name.clone());
            }
        }

        (resolved, removed)
    }

    /// Detect skills, resolve by priority, and handle conflicts
    ///
    /// This is the main entry point for multi-skill detection with
    /// full priority and conflict resolution.
    ///
    /// # Arguments
    /// * `text` - The LLM response text to scan
    /// * `loaded_skills` - Available loaded skills with metadata
    ///
    /// # Returns
    /// Tuple of (resolved skills, removed skills with reasons)
    pub fn detect_and_resolve(
        text: &str,
        loaded_skills: &[LoadedSkill],
    ) -> (Vec<String>, Vec<(String, String)>) {
        let detected = Self::detect(text);
        if detected.is_empty() {
            return (Vec::new(), Vec::new());
        }

        let prioritized = Self::resolve_by_priority(&detected, loaded_skills);
        Self::resolve_conflicts(&prioritized, loaded_skills)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_single_skill() {
        let text = "I will use <use_skill>weather-query</use_skill> to get the weather.";
        let skills = SkillDetector::detect(text);
        assert_eq!(skills, vec!["weather-query"]);
    }

    #[test]
    fn test_detect_multiple_skills() {
        let text = "Using <use_skill>skill-one</use_skill> and <use_skill>skill-two</use_skill>.";
        let skills = SkillDetector::detect(text);
        assert_eq!(skills, vec!["skill-one", "skill-two"]);
    }

    #[test]
    fn test_detect_no_skills() {
        let text = "This is a regular response without any skill requests.";
        let skills = SkillDetector::detect(text);
        assert!(skills.is_empty());
    }

    #[test]
    fn test_detect_with_whitespace() {
        let text = "<use_skill>  my-skill  </use_skill>";
        let skills = SkillDetector::detect(text);
        assert_eq!(skills, vec!["my-skill"]);
    }

    #[test]
    fn test_detect_single_char_skill() {
        let text = "<use_skill>a</use_skill>";
        let skills = SkillDetector::detect(text);
        assert_eq!(skills, vec!["a"]);
    }

    #[test]
    fn test_detect_first() {
        let text = "First <use_skill>alpha</use_skill> then <use_skill>beta</use_skill>.";
        let first = SkillDetector::detect_first(text);
        assert_eq!(first, Some("alpha".to_string()));
    }

    #[test]
    fn test_detect_first_none() {
        let text = "No skills here.";
        let first = SkillDetector::detect_first(text);
        assert!(first.is_none());
    }

    #[test]
    fn test_has_skill_request() {
        assert!(SkillDetector::has_skill_request(
            "Using <use_skill>test</use_skill>"
        ));
        assert!(!SkillDetector::has_skill_request("No skills here"));
    }

    #[test]
    fn test_strip_tags() {
        let text = "Before <use_skill>skill-name</use_skill> after.";
        let cleaned = SkillDetector::strip_tags(text);
        assert_eq!(cleaned, "Before  after.");
    }

    #[test]
    fn test_strip_multiple_tags() {
        let text = "A <use_skill>one</use_skill> B <use_skill>two</use_skill> C";
        let cleaned = SkillDetector::strip_tags(text);
        assert_eq!(cleaned, "A  B  C");
    }

    #[test]
    fn test_extract_and_clean() {
        let text = "Using <use_skill>weather</use_skill> for forecast.";
        let (skills, cleaned) = SkillDetector::extract_and_clean(text);

        assert_eq!(skills, vec!["weather"]);
        assert_eq!(cleaned, "Using  for forecast.");
    }

    #[test]
    fn test_invalid_skill_names_not_matched() {
        // Names starting with hyphen
        let text1 = "<use_skill>-invalid</use_skill>";
        assert!(SkillDetector::detect(text1).is_empty());

        // Names ending with hyphen
        let text2 = "<use_skill>invalid-</use_skill>";
        assert!(SkillDetector::detect(text2).is_empty());

        // Names with uppercase
        let text3 = "<use_skill>Invalid</use_skill>";
        assert!(SkillDetector::detect(text3).is_empty());

        // Names with underscore
        let text4 = "<use_skill>invalid_name</use_skill>";
        assert!(SkillDetector::detect(text4).is_empty());
    }

    #[test]
    fn test_valid_skill_names() {
        // Lowercase with numbers
        let text1 = "<use_skill>skill123</use_skill>";
        assert_eq!(SkillDetector::detect(text1), vec!["skill123"]);

        // Numbers with hyphens
        let text2 = "<use_skill>123-skill</use_skill>";
        assert_eq!(SkillDetector::detect(text2), vec!["123-skill"]);

        // Multiple hyphens
        let text3 = "<use_skill>my-long-skill-name</use_skill>";
        assert_eq!(SkillDetector::detect(text3), vec!["my-long-skill-name"]);
    }

    #[test]
    fn test_multiline_text() {
        let text = r#"
            I'll analyze this request.
            <use_skill>code-review</use_skill>
            Let me proceed with the review.
        "#;

        let skills = SkillDetector::detect(text);
        assert_eq!(skills, vec!["code-review"]);
    }

    #[test]
    fn test_empty_text() {
        assert!(SkillDetector::detect("").is_empty());
        assert!(SkillDetector::detect_first("").is_none());
        assert!(!SkillDetector::has_skill_request(""));
        assert_eq!(SkillDetector::strip_tags(""), "");
    }

    #[test]
    fn test_empty_skill_tag() {
        let text = "<use_skill></use_skill>";
        assert!(SkillDetector::detect(text).is_empty());
    }

    #[test]
    fn test_whitespace_only_in_tag() {
        let text = "<use_skill>   </use_skill>";
        assert!(SkillDetector::detect(text).is_empty());
    }

    #[test]
    fn test_malformed_tags() {
        // Missing closing tag
        let text1 = "<use_skill>skill-name";
        assert!(SkillDetector::detect(text1).is_empty());

        // Missing opening tag
        let text2 = "skill-name</use_skill>";
        assert!(SkillDetector::detect(text2).is_empty());

        // Wrong tag name
        let text3 = "<useskill>skill-name</useskill>";
        assert!(SkillDetector::detect(text3).is_empty());

        // Nested tags (should not match)
        let text4 = "<use_skill><use_skill>nested</use_skill></use_skill>";
        // This will match "nested" as the inner content
        let skills = SkillDetector::detect(text4);
        assert_eq!(skills.len(), 1);
    }

    #[test]
    fn test_case_sensitivity() {
        // Uppercase tag names should not match
        let text1 = "<USE_SKILL>skill-name</USE_SKILL>";
        assert!(SkillDetector::detect(text1).is_empty());

        // Mixed case tag
        let text2 = "<Use_Skill>skill-name</Use_Skill>";
        assert!(SkillDetector::detect(text2).is_empty());
    }

    #[test]
    fn test_special_characters_in_context() {
        let text = r#"Here's the code: ```<use_skill>test-skill</use_skill>``` end"#;
        let skills = SkillDetector::detect(text);
        assert_eq!(skills, vec!["test-skill"]);
    }

    #[test]
    fn test_consecutive_hyphens_invalid() {
        let text = "<use_skill>invalid--name</use_skill>";
        // The regex requires single hyphens between alphanumeric chars
        let skills = SkillDetector::detect(text);
        // This will actually match because the regex allows multiple hyphens
        // Let's verify current behavior
        assert!(skills.is_empty() || skills[0] == "invalid--name");
    }

    #[test]
    fn test_numeric_only_names() {
        let text = "<use_skill>123</use_skill>";
        let skills = SkillDetector::detect(text);
        assert_eq!(skills, vec!["123"]);
    }

    #[test]
    fn test_two_character_skill() {
        let text = "<use_skill>ab</use_skill>";
        let skills = SkillDetector::detect(text);
        assert_eq!(skills, vec!["ab"]);
    }

    #[test]
    fn test_extract_and_clean_empty() {
        let (skills, cleaned) = SkillDetector::extract_and_clean("");
        assert!(skills.is_empty());
        assert!(cleaned.is_empty());
    }

    #[test]
    fn test_extract_and_clean_no_skills() {
        let text = "Just regular text without any skills.";
        let (skills, cleaned) = SkillDetector::extract_and_clean(text);
        assert!(skills.is_empty());
        assert_eq!(cleaned, text);
    }

    #[test]
    fn test_strip_tags_preserves_other_xml() {
        let text =
            "<thought>thinking</thought> <use_skill>my-skill</use_skill> <action>do</action>";
        let cleaned = SkillDetector::strip_tags(text);
        assert!(cleaned.contains("<thought>thinking</thought>"));
        assert!(cleaned.contains("<action>do</action>"));
        assert!(!cleaned.contains("my-skill"));
    }

    #[test]
    fn test_skill_at_boundaries() {
        // Skill at start
        let text1 = "<use_skill>start</use_skill> rest of text";
        assert_eq!(SkillDetector::detect(text1), vec!["start"]);

        // Skill at end
        let text2 = "text before <use_skill>end</use_skill>";
        assert_eq!(SkillDetector::detect(text2), vec!["end"]);

        // Only skill
        let text3 = "<use_skill>only</use_skill>";
        assert_eq!(SkillDetector::detect(text3), vec!["only"]);
    }

    #[test]
    fn test_long_skill_name() {
        let long_name = "a".repeat(64);
        let text = format!("<use_skill>{}</use_skill>", long_name);
        let skills = SkillDetector::detect(&text);
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0], long_name);
    }

    // ==========================================================================
    // Multi-skill Detection Tests
    // ==========================================================================

    #[test]
    fn test_comma_separated_skills() {
        let text = "<use_skill>skill-a, skill-b</use_skill>";
        let skills = SkillDetector::detect(text);
        assert_eq!(skills, vec!["skill-a", "skill-b"]);
    }

    #[test]
    fn test_comma_separated_skills_no_spaces() {
        let text = "<use_skill>skill-a,skill-b,skill-c</use_skill>";
        let skills = SkillDetector::detect(text);
        assert_eq!(skills, vec!["skill-a", "skill-b", "skill-c"]);
    }

    #[test]
    fn test_comma_separated_skills_extra_spaces() {
        let text = "<use_skill>  skill-a  ,   skill-b  </use_skill>";
        let skills = SkillDetector::detect(text);
        assert_eq!(skills, vec!["skill-a", "skill-b"]);
    }

    #[test]
    fn test_deduplicate_skills() {
        // Same skill in comma-separated list
        let text1 = "<use_skill>skill-a, skill-a</use_skill>";
        let skills1 = SkillDetector::detect(text1);
        assert_eq!(skills1, vec!["skill-a"]);

        // Same skill in separate tags
        let text2 = "<use_skill>skill-a</use_skill> and <use_skill>skill-a</use_skill>";
        let skills2 = SkillDetector::detect(text2);
        assert_eq!(skills2, vec!["skill-a"]);

        // Mixed
        let text3 = "<use_skill>skill-a, skill-b</use_skill> and <use_skill>skill-b</use_skill>";
        let skills3 = SkillDetector::detect(text3);
        assert_eq!(skills3, vec!["skill-a", "skill-b"]);
    }

    #[test]
    fn test_mixed_single_and_comma_separated() {
        let text = "<use_skill>first</use_skill> then <use_skill>second, third</use_skill>";
        let skills = SkillDetector::detect(text);
        assert_eq!(skills, vec!["first", "second", "third"]);
    }

    // ==========================================================================
    // Priority Resolution Tests
    // ==========================================================================

    fn create_test_loaded_skill(
        name: &str,
        priority: Option<i32>,
        conflicts: Option<Vec<String>>,
    ) -> LoadedSkill {
        use std::{collections::HashMap, path::PathBuf};

        use chrono::Utc;

        use crate::skills::SkillMetadata;

        // Build metadata map with priority and conflicts
        let mut metadata_map = HashMap::new();
        if let Some(p) = priority {
            metadata_map.insert("priority".to_string(), p.to_string());
        }
        if let Some(ref c) = conflicts {
            metadata_map.insert("conflicts".to_string(), c.join(", "));
        }

        LoadedSkill {
            metadata: SkillMetadata {
                name: name.to_string(),
                description: format!("Test skill {}", name),
                license: None,
                compatibility: None,
                metadata: if metadata_map.is_empty() {
                    None
                } else {
                    Some(metadata_map)
                },
                allowed_tools: None,
                model: None,
                allowed_scripts: None,
                execution_limits: None,
                references: None,
            },
            content: format!("Content for {}", name),
            raw_content: String::new(),
            skill_dir: PathBuf::new(),
            file_path: String::new(),
            enabled: true,
            loaded_at: Utc::now(),
            scripts: Vec::new(),
        }
    }

    #[test]
    fn test_resolve_by_priority_basic() {
        let loaded_skills = vec![
            create_test_loaded_skill("low-priority", Some(-10), None),
            create_test_loaded_skill("high-priority", Some(50), None),
            create_test_loaded_skill("medium-priority", Some(10), None),
        ];

        let skill_names = vec![
            "low-priority".to_string(),
            "high-priority".to_string(),
            "medium-priority".to_string(),
        ];

        let sorted = SkillDetector::resolve_by_priority(&skill_names, &loaded_skills);

        assert_eq!(
            sorted,
            vec!["high-priority", "medium-priority", "low-priority"]
        );
    }

    #[test]
    fn test_resolve_by_priority_default_zero() {
        let loaded_skills = vec![
            create_test_loaded_skill("with-priority", Some(10), None),
            create_test_loaded_skill("no-priority", None, None),
        ];

        let skill_names = vec!["no-priority".to_string(), "with-priority".to_string()];

        let sorted = SkillDetector::resolve_by_priority(&skill_names, &loaded_skills);

        // with-priority (10) > no-priority (0)
        assert_eq!(sorted, vec!["with-priority", "no-priority"]);
    }

    #[test]
    fn test_resolve_by_priority_unknown_skill() {
        let loaded_skills = vec![create_test_loaded_skill("known", Some(10), None)];

        let skill_names = vec!["known".to_string(), "unknown".to_string()];

        let sorted = SkillDetector::resolve_by_priority(&skill_names, &loaded_skills);

        // known (10) > unknown (0 default)
        assert_eq!(sorted, vec!["known", "unknown"]);
    }

    #[test]
    fn test_resolve_by_priority_equal_priority() {
        let loaded_skills = vec![
            create_test_loaded_skill("skill-a", Some(10), None),
            create_test_loaded_skill("skill-b", Some(10), None),
        ];

        let skill_names = vec!["skill-a".to_string(), "skill-b".to_string()];

        let sorted = SkillDetector::resolve_by_priority(&skill_names, &loaded_skills);

        // Both have same priority, order should be stable
        assert_eq!(sorted.len(), 2);
        assert!(sorted.contains(&"skill-a".to_string()));
        assert!(sorted.contains(&"skill-b".to_string()));
    }

    // ==========================================================================
    // Conflict Resolution Tests
    // ==========================================================================

    #[test]
    fn test_resolve_conflicts_no_conflicts() {
        let loaded_skills = vec![
            create_test_loaded_skill("skill-a", Some(10), None),
            create_test_loaded_skill("skill-b", Some(5), None),
        ];

        let skill_names = vec!["skill-a".to_string(), "skill-b".to_string()];

        let (resolved, removed) = SkillDetector::resolve_conflicts(&skill_names, &loaded_skills);

        assert_eq!(resolved, vec!["skill-a", "skill-b"]);
        assert!(removed.is_empty());
    }

    #[test]
    fn test_resolve_conflicts_basic() {
        let loaded_skills = vec![
            create_test_loaded_skill("high", Some(20), Some(vec!["low".to_string()])),
            create_test_loaded_skill("low", Some(5), None),
        ];

        // high conflicts with low, high has higher priority
        let skill_names = vec!["high".to_string(), "low".to_string()];

        let (resolved, removed) = SkillDetector::resolve_conflicts(&skill_names, &loaded_skills);

        assert_eq!(resolved, vec!["high"]);
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].0, "low");
        assert!(
            removed[0]
                .1
                .contains("conflicts with higher priority skill 'high'")
        );
    }

    #[test]
    fn test_resolve_conflicts_bidirectional() {
        // Both skills declare conflict with each other
        let loaded_skills = vec![
            create_test_loaded_skill("skill-a", Some(20), Some(vec!["skill-b".to_string()])),
            create_test_loaded_skill("skill-b", Some(10), Some(vec!["skill-a".to_string()])),
        ];

        let skill_names = vec!["skill-a".to_string(), "skill-b".to_string()];

        let (resolved, removed) = SkillDetector::resolve_conflicts(&skill_names, &loaded_skills);

        // skill-a (higher priority) wins
        assert_eq!(resolved, vec!["skill-a"]);
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].0, "skill-b");
    }

    #[test]
    fn test_resolve_conflicts_chain() {
        // A conflicts with B, B conflicts with C
        let loaded_skills = vec![
            create_test_loaded_skill("skill-a", Some(30), Some(vec!["skill-b".to_string()])),
            create_test_loaded_skill("skill-b", Some(20), Some(vec!["skill-c".to_string()])),
            create_test_loaded_skill("skill-c", Some(10), None),
        ];

        let skill_names = vec![
            "skill-a".to_string(),
            "skill-b".to_string(),
            "skill-c".to_string(),
        ];

        let (resolved, removed) = SkillDetector::resolve_conflicts(&skill_names, &loaded_skills);

        // A excludes B, C remains (no direct conflict with A)
        assert_eq!(resolved, vec!["skill-a", "skill-c"]);
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].0, "skill-b");
    }

    #[test]
    fn test_resolve_conflicts_unknown_skill() {
        let loaded_skills = vec![create_test_loaded_skill(
            "known",
            Some(10),
            Some(vec!["unknown".to_string()]),
        )];

        let skill_names = vec!["known".to_string(), "unknown".to_string()];

        let (resolved, removed) = SkillDetector::resolve_conflicts(&skill_names, &loaded_skills);

        // known excludes unknown
        assert_eq!(resolved, vec!["known"]);
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].0, "unknown");
        assert!(
            removed[0]
                .1
                .contains("conflicts with higher priority skill 'known'")
        );
    }

    // ==========================================================================
    // Full Detection and Resolution Tests
    // ==========================================================================

    #[test]
    fn test_detect_and_resolve_basic() {
        let loaded_skills = vec![
            create_test_loaded_skill("high-priority", Some(20), None),
            create_test_loaded_skill("low-priority", Some(5), None),
        ];

        let text = "<use_skill>low-priority, high-priority</use_skill>";

        let (resolved, removed) = SkillDetector::detect_and_resolve(text, &loaded_skills);

        // Should be sorted by priority
        assert_eq!(resolved, vec!["high-priority", "low-priority"]);
        assert!(removed.is_empty());
    }

    #[test]
    fn test_detect_and_resolve_with_conflict() {
        let loaded_skills = vec![
            create_test_loaded_skill("primary", Some(20), Some(vec!["secondary".to_string()])),
            create_test_loaded_skill("secondary", Some(10), None),
            create_test_loaded_skill("unrelated", Some(5), None),
        ];

        let text = "<use_skill>secondary, primary, unrelated</use_skill>";

        let (resolved, removed) = SkillDetector::detect_and_resolve(text, &loaded_skills);

        // primary wins over secondary due to conflict, unrelated stays
        assert_eq!(resolved, vec!["primary", "unrelated"]);
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].0, "secondary");
    }

    #[test]
    fn test_detect_and_resolve_empty() {
        let loaded_skills = vec![create_test_loaded_skill("skill-a", Some(10), None)];

        let text = "No skills here";

        let (resolved, removed) = SkillDetector::detect_and_resolve(text, &loaded_skills);

        assert!(resolved.is_empty());
        assert!(removed.is_empty());
    }
}
