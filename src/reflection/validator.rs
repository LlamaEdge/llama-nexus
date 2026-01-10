//! Result validation system for the reflection engine.
//!
//! This module defines the core validation traits and types used to validate
//! task execution results before and during reflection.

// Some validation types are reserved for future features
#![allow(dead_code)]

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Result of a validation check.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationResult {
    /// Whether the result passed validation.
    pub valid: bool,
    /// Errors found during validation.
    pub errors: Vec<ValidationError>,
    /// Warnings found during validation.
    pub warnings: Vec<ValidationWarning>,
    /// Validation score (0.0 - 1.0).
    pub score: f64,
}

impl Default for ValidationResult {
    fn default() -> Self {
        Self {
            valid: true,
            errors: Vec::new(),
            warnings: Vec::new(),
            score: 1.0,
        }
    }
}

impl ValidationResult {
    /// Creates a valid result with full score.
    pub fn valid() -> Self {
        Self::default()
    }

    /// Creates an invalid result with a single error.
    pub fn invalid(error: ValidationError) -> Self {
        Self {
            valid: false,
            errors: vec![error],
            warnings: Vec::new(),
            score: 0.0,
        }
    }

    /// Creates a valid result with warnings.
    pub fn valid_with_warnings(warnings: Vec<ValidationWarning>, score: f64) -> Self {
        Self {
            valid: true,
            errors: Vec::new(),
            warnings,
            score: score.clamp(0.0, 1.0),
        }
    }

    /// Merges another validation result into this one.
    pub fn merge(&mut self, other: ValidationResult) {
        self.valid = self.valid && other.valid;
        self.errors.extend(other.errors);
        self.warnings.extend(other.warnings);
        // Average the scores
        self.score = (self.score + other.score) / 2.0;
    }

    /// Returns a summary for logging.
    pub fn summary(&self) -> String {
        format!(
            "ValidationResult[valid={}, errors={}, warnings={}, score={:.2}]",
            self.valid,
            self.errors.len(),
            self.warnings.len(),
            self.score
        )
    }
}

/// A validation error.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationError {
    /// Error code for categorization.
    pub code: String,
    /// Human-readable error message.
    pub message: String,
    /// Optional location information (line number, position, etc.).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
}

impl ValidationError {
    /// Creates a new validation error.
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            location: None,
        }
    }

    /// Adds location information to the error.
    pub fn with_location(mut self, location: impl Into<String>) -> Self {
        self.location = Some(location.into());
        self
    }
}

/// A validation warning.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationWarning {
    /// Warning code for categorization.
    pub code: String,
    /// Human-readable warning message.
    pub message: String,
    /// Optional suggestion for improvement.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
}

impl ValidationWarning {
    /// Creates a new validation warning.
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            suggestion: None,
        }
    }

    /// Adds a suggestion to the warning.
    pub fn with_suggestion(mut self, suggestion: impl Into<String>) -> Self {
        self.suggestion = Some(suggestion.into());
        self
    }
}

/// Context for validation.
#[derive(Debug, Clone, Default)]
pub struct ValidationContext {
    /// Original task description.
    pub task_description: String,
    /// Expected output format.
    pub expected_format: Option<ExpectedFormat>,
    /// Constraints to check.
    pub constraints: Vec<Constraint>,
}

impl ValidationContext {
    /// Creates a new validation context.
    pub fn new(task_description: impl Into<String>) -> Self {
        Self {
            task_description: task_description.into(),
            expected_format: None,
            constraints: Vec::new(),
        }
    }

    /// Sets the expected format.
    pub fn with_format(mut self, format: ExpectedFormat) -> Self {
        self.expected_format = Some(format);
        self
    }

    /// Adds a constraint.
    pub fn with_constraint(mut self, constraint: Constraint) -> Self {
        self.constraints.push(constraint);
        self
    }

    /// Adds multiple constraints.
    pub fn with_constraints(mut self, constraints: Vec<Constraint>) -> Self {
        self.constraints.extend(constraints);
        self
    }
}

/// Expected output format for validation.
#[derive(Debug, Clone)]
pub enum ExpectedFormat {
    /// JSON format.
    Json,
    /// Code in a specific language.
    Code { language: String },
    /// Markdown format.
    Markdown,
    /// Plain text (no specific format).
    PlainText,
}

/// Constraint for validation.
#[derive(Debug, Clone)]
pub enum Constraint {
    /// Maximum length in characters.
    MaxLength(usize),
    /// Minimum length in characters.
    MinLength(usize),
    /// Must contain the specified string.
    MustContain(String),
    /// Must not contain the specified string.
    MustNotContain(String),
    /// Must match the regex pattern.
    Pattern(String),
}

impl Constraint {
    /// Checks if the result satisfies this constraint.
    pub fn check(&self, result: &str) -> Result<(), ValidationError> {
        match self {
            Constraint::MaxLength(max) => {
                if result.len() > *max {
                    Err(ValidationError::new(
                        "CONSTRAINT_MAX_LENGTH",
                        format!(
                            "Result length {} exceeds maximum allowed length {}",
                            result.len(),
                            max
                        ),
                    ))
                } else {
                    Ok(())
                }
            }
            Constraint::MinLength(min) => {
                if result.len() < *min {
                    Err(ValidationError::new(
                        "CONSTRAINT_MIN_LENGTH",
                        format!(
                            "Result length {} is below minimum required length {}",
                            result.len(),
                            min
                        ),
                    ))
                } else {
                    Ok(())
                }
            }
            Constraint::MustContain(s) => {
                if !result.contains(s) {
                    Err(ValidationError::new(
                        "CONSTRAINT_MUST_CONTAIN",
                        format!("Result must contain: {}", s),
                    ))
                } else {
                    Ok(())
                }
            }
            Constraint::MustNotContain(s) => {
                if result.contains(s) {
                    Err(ValidationError::new(
                        "CONSTRAINT_MUST_NOT_CONTAIN",
                        format!("Result must not contain: {}", s),
                    ))
                } else {
                    Ok(())
                }
            }
            Constraint::Pattern(pattern) => {
                let re = regex::Regex::new(pattern).map_err(|e| {
                    ValidationError::new(
                        "CONSTRAINT_INVALID_PATTERN",
                        format!("Invalid regex pattern: {}", e),
                    )
                })?;
                if !re.is_match(result) {
                    Err(ValidationError::new(
                        "CONSTRAINT_PATTERN_MISMATCH",
                        format!("Result does not match required pattern: {}", pattern),
                    ))
                } else {
                    Ok(())
                }
            }
        }
    }
}

/// Trait for result validators.
#[async_trait]
pub trait ResultValidator: Send + Sync {
    /// Validates the given result.
    async fn validate(&self, result: &str, context: &ValidationContext) -> ValidationResult;

    /// Returns the name of this validator.
    fn name(&self) -> &str;
}

/// Checks all constraints against a result.
pub fn check_constraints(result: &str, constraints: &[Constraint]) -> ValidationResult {
    let mut errors = Vec::new();

    for constraint in constraints {
        if let Err(error) = constraint.check(result) {
            errors.push(error);
        }
    }

    if errors.is_empty() {
        ValidationResult::valid()
    } else {
        ValidationResult {
            valid: false,
            errors,
            warnings: Vec::new(),
            score: 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validation_result_default() {
        let result = ValidationResult::default();
        assert!(result.valid);
        assert_eq!(result.score, 1.0);
        assert!(result.errors.is_empty());
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn test_validation_result_invalid() {
        let error = ValidationError::new("TEST_ERROR", "Test error message");
        let result = ValidationResult::invalid(error);
        assert!(!result.valid);
        assert_eq!(result.score, 0.0);
        assert_eq!(result.errors.len(), 1);
    }

    #[test]
    fn test_validation_result_merge() {
        let mut result1 = ValidationResult::valid();
        result1.score = 0.8;

        let error = ValidationError::new("TEST_ERROR", "Test error");
        let result2 = ValidationResult::invalid(error);

        result1.merge(result2);
        assert!(!result1.valid);
        assert_eq!(result1.errors.len(), 1);
        assert_eq!(result1.score, 0.4); // (0.8 + 0.0) / 2
    }

    #[test]
    fn test_validation_error_with_location() {
        let error = ValidationError::new("TEST", "Error message").with_location("line 42");
        assert_eq!(error.location, Some("line 42".to_string()));
    }

    #[test]
    fn test_validation_warning_with_suggestion() {
        let warning =
            ValidationWarning::new("TEST", "Warning message").with_suggestion("Try this instead");
        assert_eq!(warning.suggestion, Some("Try this instead".to_string()));
    }

    #[test]
    fn test_validation_context_builder() {
        let context = ValidationContext::new("Test task")
            .with_format(ExpectedFormat::Json)
            .with_constraint(Constraint::MaxLength(1000))
            .with_constraint(Constraint::MinLength(10));

        assert_eq!(context.task_description, "Test task");
        assert!(matches!(
            context.expected_format,
            Some(ExpectedFormat::Json)
        ));
        assert_eq!(context.constraints.len(), 2);
    }

    #[test]
    fn test_constraint_max_length() {
        let constraint = Constraint::MaxLength(10);
        assert!(constraint.check("short").is_ok());
        assert!(constraint.check("this is way too long").is_err());
    }

    #[test]
    fn test_constraint_min_length() {
        let constraint = Constraint::MinLength(5);
        assert!(constraint.check("hello world").is_ok());
        assert!(constraint.check("hi").is_err());
    }

    #[test]
    fn test_constraint_must_contain() {
        let constraint = Constraint::MustContain("hello".to_string());
        assert!(constraint.check("hello world").is_ok());
        assert!(constraint.check("goodbye world").is_err());
    }

    #[test]
    fn test_constraint_must_not_contain() {
        let constraint = Constraint::MustNotContain("TODO".to_string());
        assert!(constraint.check("completed task").is_ok());
        assert!(constraint.check("TODO: finish this").is_err());
    }

    #[test]
    fn test_constraint_pattern() {
        let constraint = Constraint::Pattern(r"^\d{4}-\d{2}-\d{2}$".to_string());
        assert!(constraint.check("2024-01-15").is_ok());
        assert!(constraint.check("invalid date").is_err());
    }

    #[test]
    fn test_check_constraints() {
        let constraints = vec![
            Constraint::MinLength(5),
            Constraint::MaxLength(100),
            Constraint::MustContain("hello".to_string()),
        ];

        let result = check_constraints("hello world", &constraints);
        assert!(result.valid);
        assert!(result.errors.is_empty());

        let result = check_constraints("hi", &constraints);
        assert!(!result.valid);
        assert!(!result.errors.is_empty());
    }

    #[test]
    fn test_validation_result_summary() {
        let result = ValidationResult::valid();
        let summary = result.summary();
        assert!(summary.contains("valid=true"));
        assert!(summary.contains("score=1.00"));
    }
}
