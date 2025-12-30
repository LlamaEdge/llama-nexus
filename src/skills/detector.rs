//! Skill Detector for detecting skill usage requests in LLM responses
//!
//! Detects `<use_skill>skill-name</use_skill>` tags in LLM output
//! to trigger Phase 2 skill loading.

use std::sync::LazyLock;

use regex::Regex;

/// Regex pattern for detecting skill usage tags
/// Matches: <use_skill>skill-name</use_skill>
static USE_SKILL_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"<use_skill>\s*([a-z0-9][a-z0-9-]*[a-z0-9]|[a-z0-9])\s*</use_skill>")
        .expect("Invalid regex pattern")
});

/// Skill detector for identifying skill activation requests
pub struct SkillDetector;

impl SkillDetector {
    /// Detect all skill names requested in the text
    ///
    /// # Arguments
    /// * `text` - The LLM response text to scan
    ///
    /// # Returns
    /// A vector of skill names found in the text
    pub fn detect(text: &str) -> Vec<String> {
        USE_SKILL_PATTERN
            .captures_iter(text)
            .filter_map(|cap| cap.get(1).map(|m| m.as_str().to_string()))
            .collect()
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
}
