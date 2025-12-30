//! Skill name validator
//!
//! Validates skill names according to Agent Skills Standard:
//! - 1-64 characters
//! - Lowercase letters, numbers, and hyphens only
//! - Cannot start or end with hyphen
//! - Cannot contain consecutive hyphens
//! - Must match parent directory name

use regex::Regex;

use crate::skills::error::{SkillError, SkillResult};

/// Validate a skill name according to the Agent Skills Standard
///
/// # Arguments
/// * `name` - The skill name to validate
/// * `parent_dir` - Optional parent directory name to match against
///
/// # Returns
/// * `Ok(())` if valid
/// * `Err(SkillError::InvalidName)` if invalid
pub fn validate_skill_name(name: &str, parent_dir: Option<&str>) -> SkillResult<()> {
    // Length check: 1-64 characters
    if name.is_empty() {
        return Err(SkillError::InvalidName {
            name: name.to_string(),
            reason: "name cannot be empty".to_string(),
        });
    }

    if name.len() > 64 {
        return Err(SkillError::InvalidName {
            name: name.to_string(),
            reason: format!("name exceeds 64 characters (got {})", name.len()),
        });
    }

    // Character check: lowercase letters, numbers, hyphens only
    let re = Regex::new(r"^[a-z0-9-]+$").unwrap();
    if !re.is_match(name) {
        return Err(SkillError::InvalidName {
            name: name.to_string(),
            reason: "name must contain only lowercase letters, numbers, and hyphens".to_string(),
        });
    }

    // Cannot start with hyphen
    if name.starts_with('-') {
        return Err(SkillError::InvalidName {
            name: name.to_string(),
            reason: "name cannot start with a hyphen".to_string(),
        });
    }

    // Cannot end with hyphen
    if name.ends_with('-') {
        return Err(SkillError::InvalidName {
            name: name.to_string(),
            reason: "name cannot end with a hyphen".to_string(),
        });
    }

    // Cannot contain consecutive hyphens
    if name.contains("--") {
        return Err(SkillError::InvalidName {
            name: name.to_string(),
            reason: "name cannot contain consecutive hyphens".to_string(),
        });
    }

    // Must match parent directory name (if provided)
    if let Some(dir) = parent_dir
        && name != dir
    {
        return Err(SkillError::NameDirectoryMismatch {
            name: name.to_string(),
            directory: dir.to_string(),
        });
    }

    Ok(())
}

/// Validate description field
///
/// # Arguments
/// * `description` - The description to validate
///
/// # Returns
/// * `Ok(())` if valid (1-1024 characters)
/// * `Err(SkillError::InvalidDescription)` if invalid
pub fn validate_description(description: &str) -> SkillResult<()> {
    if description.is_empty() {
        return Err(SkillError::InvalidDescription(
            "description cannot be empty".to_string(),
        ));
    }

    if description.len() > 1024 {
        return Err(SkillError::InvalidDescription(format!(
            "description exceeds 1024 characters (got {})",
            description.len()
        )));
    }

    Ok(())
}

/// Validate compatibility field (if present)
///
/// # Arguments
/// * `compatibility` - The compatibility string to validate
///
/// # Returns
/// * `Ok(())` if valid (≤500 characters)
/// * `Err(SkillError::InvalidCompatibility)` if invalid
pub fn validate_compatibility(compatibility: &str) -> SkillResult<()> {
    if compatibility.len() > 500 {
        return Err(SkillError::InvalidCompatibility(format!(
            "compatibility exceeds 500 characters (got {})",
            compatibility.len()
        )));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_names() {
        assert!(validate_skill_name("weather-query", None).is_ok());
        assert!(validate_skill_name("code-review", None).is_ok());
        assert!(validate_skill_name("simple", None).is_ok());
        assert!(validate_skill_name("skill123", None).is_ok());
        assert!(validate_skill_name("my-skill-v2", None).is_ok());
        assert!(validate_skill_name("a", None).is_ok()); // 1 char
    }

    #[test]
    fn test_invalid_empty() {
        let result = validate_skill_name("", None);
        assert!(matches!(result, Err(SkillError::InvalidName { .. })));
    }

    #[test]
    fn test_invalid_too_long() {
        let long_name = "a".repeat(65);
        let result = validate_skill_name(&long_name, None);
        assert!(matches!(result, Err(SkillError::InvalidName { .. })));
    }

    #[test]
    fn test_invalid_uppercase() {
        let result = validate_skill_name("Weather-Query", None);
        assert!(matches!(result, Err(SkillError::InvalidName { .. })));
    }

    #[test]
    fn test_invalid_special_chars() {
        assert!(validate_skill_name("weather_query", None).is_err()); // underscore
        assert!(validate_skill_name("weather.query", None).is_err()); // dot
        assert!(validate_skill_name("weather query", None).is_err()); // space
        assert!(validate_skill_name("weather@query", None).is_err()); // @
    }

    #[test]
    fn test_invalid_starts_with_hyphen() {
        let result = validate_skill_name("-weather", None);
        assert!(matches!(result, Err(SkillError::InvalidName { .. })));
    }

    #[test]
    fn test_invalid_ends_with_hyphen() {
        let result = validate_skill_name("weather-", None);
        assert!(matches!(result, Err(SkillError::InvalidName { .. })));
    }

    #[test]
    fn test_invalid_consecutive_hyphens() {
        let result = validate_skill_name("weather--query", None);
        assert!(matches!(result, Err(SkillError::InvalidName { .. })));
    }

    #[test]
    fn test_directory_match() {
        // Should pass when name matches directory
        assert!(validate_skill_name("weather-query", Some("weather-query")).is_ok());

        // Should fail when name doesn't match directory
        let result = validate_skill_name("weather-query", Some("different-name"));
        assert!(matches!(
            result,
            Err(SkillError::NameDirectoryMismatch { .. })
        ));
    }

    #[test]
    fn test_valid_description() {
        assert!(validate_description("A simple skill").is_ok());
        assert!(validate_description("a").is_ok()); // 1 char
        assert!(validate_description(&"a".repeat(1024)).is_ok()); // max length
    }

    #[test]
    fn test_invalid_description() {
        assert!(validate_description("").is_err()); // empty
        assert!(validate_description(&"a".repeat(1025)).is_err()); // too long
    }

    #[test]
    fn test_valid_compatibility() {
        assert!(validate_compatibility("").is_ok()); // empty is ok
        assert!(validate_compatibility("Requires Python 3.8+").is_ok());
        assert!(validate_compatibility(&"a".repeat(500)).is_ok()); // max length
    }

    #[test]
    fn test_invalid_compatibility() {
        assert!(validate_compatibility(&"a".repeat(501)).is_err()); // too long
    }
}
