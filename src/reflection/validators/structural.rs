//! Structural validators for format and syntax checking.
//!
//! These validators perform fast, local checks without LLM calls.

use async_trait::async_trait;

use crate::reflection::validator::{
    ResultValidator, ValidationContext, ValidationError, ValidationResult, ValidationWarning,
};

/// JSON format validator.
///
/// Validates that the input is valid JSON and optionally checks for
/// required fields or schema compliance.
pub struct JsonValidator {
    /// Required top-level fields (if any).
    required_fields: Vec<String>,
}

impl Default for JsonValidator {
    fn default() -> Self {
        Self::new()
    }
}

impl JsonValidator {
    /// Creates a new JSON validator.
    pub fn new() -> Self {
        Self {
            required_fields: Vec::new(),
        }
    }

    /// Creates a JSON validator with required fields.
    pub fn with_required_fields(fields: Vec<String>) -> Self {
        Self {
            required_fields: fields,
        }
    }
}

#[async_trait]
impl ResultValidator for JsonValidator {
    async fn validate(&self, result: &str, _context: &ValidationContext) -> ValidationResult {
        // Try to parse as JSON
        let parsed: serde_json::Value = match serde_json::from_str(result) {
            Ok(v) => v,
            Err(e) => {
                return ValidationResult::invalid(ValidationError::new(
                    "INVALID_JSON",
                    format!("Invalid JSON: {}", e),
                ));
            }
        };

        // Check required fields if any
        let mut errors = Vec::new();
        let mut warnings = Vec::new();

        if let Some(obj) = parsed.as_object() {
            for field in &self.required_fields {
                if !obj.contains_key(field) {
                    errors.push(ValidationError::new(
                        "MISSING_REQUIRED_FIELD",
                        format!("Missing required field: {}", field),
                    ));
                }
            }

            // Warn about null values
            for (key, value) in obj {
                if value.is_null() {
                    warnings.push(ValidationWarning::new(
                        "NULL_VALUE",
                        format!("Field '{}' has null value", key),
                    ));
                }
            }
        }

        if errors.is_empty() {
            let score = if warnings.is_empty() { 1.0 } else { 0.9 };
            ValidationResult::valid_with_warnings(warnings, score)
        } else {
            ValidationResult {
                valid: false,
                errors,
                warnings,
                score: 0.0,
            }
        }
    }

    fn name(&self) -> &str {
        "json_validator"
    }
}

/// Supported programming languages for code validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupportedLanguage {
    /// Rust programming language.
    Rust,
    /// Python programming language.
    Python,
    /// JavaScript.
    JavaScript,
    /// TypeScript.
    TypeScript,
}

impl SupportedLanguage {
    /// Parses a language from a string.
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "rust" | "rs" => Some(Self::Rust),
            "python" | "py" => Some(Self::Python),
            "javascript" | "js" => Some(Self::JavaScript),
            "typescript" | "ts" => Some(Self::TypeScript),
            _ => None,
        }
    }

    /// Returns the file extension for this language.
    pub fn extension(&self) -> &str {
        match self {
            Self::Rust => "rs",
            Self::Python => "py",
            Self::JavaScript => "js",
            Self::TypeScript => "ts",
        }
    }
}

/// Code syntax validator.
///
/// Validates code syntax for supported languages using
/// language-specific parsers.
pub struct CodeValidator {
    /// The programming language to validate.
    language: SupportedLanguage,
}

impl CodeValidator {
    /// Creates a new code validator for the specified language.
    pub fn new(language: SupportedLanguage) -> Self {
        Self { language }
    }

    /// Creates a code validator from a language string.
    pub fn from_language_str(lang: &str) -> Option<Self> {
        SupportedLanguage::from_str(lang).map(Self::new)
    }

    /// Validates Rust code syntax using basic heuristic checks.
    ///
    /// Note: This is a simplified validator that checks for common syntax issues
    /// like unmatched brackets and basic keyword patterns. For full Rust syntax
    /// validation, the code would need to be compiled or use the syn crate.
    fn validate_rust(&self, code: &str) -> ValidationResult {
        let mut errors = Vec::new();

        // Check for basic syntax issues
        let mut paren_count = 0i32;
        let mut bracket_count = 0i32;
        let mut brace_count = 0i32;
        let mut angle_count = 0i32;
        let mut in_string = false;
        let mut in_char = false;
        let mut in_raw_string = false;
        let mut in_block_comment = false;

        for line in code.lines() {
            let in_line_comment = false;

            let chars: Vec<char> = line.chars().collect();
            let mut i = 0;

            while i < chars.len() {
                let c = chars[i];
                let next = chars.get(i + 1).copied();

                // Handle comments
                if !in_string && !in_char && !in_raw_string {
                    if !in_block_comment && c == '/' && next == Some('/') {
                        // Rest of line is a comment
                        break;
                    }
                    if !in_line_comment && c == '/' && next == Some('*') {
                        in_block_comment = true;
                        i += 2;
                        continue;
                    }
                    if in_block_comment && c == '*' && next == Some('/') {
                        in_block_comment = false;
                        i += 2;
                        continue;
                    }
                }

                if in_line_comment || in_block_comment {
                    i += 1;
                    continue;
                }

                // Handle raw strings (r#"..."#)
                if !in_string && !in_char && c == 'r' && next == Some('#') {
                    in_raw_string = true;
                    i += 1;
                    continue;
                }

                if in_raw_string && c == '"' && i > 0 && chars[i - 1] == '#' {
                    in_raw_string = false;
                    i += 1;
                    continue;
                }

                // Handle regular strings
                if !in_raw_string && !in_char && c == '"' && (i == 0 || chars[i - 1] != '\\') {
                    in_string = !in_string;
                    i += 1;
                    continue;
                }

                // Handle char literals
                if !in_raw_string && !in_string && c == '\'' && (i == 0 || chars[i - 1] != '\\') {
                    // Check if it's a char literal or lifetime
                    let is_lifetime = i > 0
                        && (chars[i - 1].is_alphabetic()
                            || chars[i - 1] == '<'
                            || chars[i - 1] == ','
                            || chars[i - 1] == '&');
                    if !is_lifetime {
                        in_char = !in_char;
                    }
                    i += 1;
                    continue;
                }

                if !in_string && !in_char && !in_raw_string {
                    match c {
                        '(' => paren_count += 1,
                        ')' => paren_count -= 1,
                        '[' => bracket_count += 1,
                        ']' => bracket_count -= 1,
                        '{' => brace_count += 1,
                        '}' => brace_count -= 1,
                        '<' => {
                            // Only count angle brackets in generic contexts
                            if i > 0 && (chars[i - 1].is_alphabetic() || chars[i - 1] == ':') {
                                angle_count += 1;
                            }
                        }
                        '>' => {
                            if angle_count > 0 {
                                angle_count -= 1;
                            }
                        }
                        _ => {}
                    }
                }

                i += 1;
            }
        }

        // Check unmatched brackets
        if paren_count != 0 {
            errors.push(ValidationError::new(
                "UNMATCHED_PARENTHESES",
                format!(
                    "Unmatched parentheses: {} {}",
                    paren_count.abs(),
                    if paren_count > 0 {
                        "unclosed '('"
                    } else {
                        "extra ')'"
                    }
                ),
            ));
        }

        if bracket_count != 0 {
            errors.push(ValidationError::new(
                "UNMATCHED_BRACKETS",
                format!(
                    "Unmatched brackets: {} {}",
                    bracket_count.abs(),
                    if bracket_count > 0 {
                        "unclosed '['"
                    } else {
                        "extra ']'"
                    }
                ),
            ));
        }

        if brace_count != 0 {
            errors.push(ValidationError::new(
                "UNMATCHED_BRACES",
                format!(
                    "Unmatched braces: {} {}",
                    brace_count.abs(),
                    if brace_count > 0 {
                        "unclosed '{{'"
                    } else {
                        "extra '}}'"
                    }
                ),
            ));
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

    /// Validates Python code using basic heuristic checks.
    ///
    /// Note: This is a simplified validator that checks for common syntax issues.
    /// For full Python syntax validation, consider using an external Python process.
    fn validate_python(&self, code: &str) -> ValidationResult {
        let mut errors = Vec::new();
        let mut warnings = Vec::new();

        // Check for basic syntax issues
        let mut paren_count = 0i32;
        let mut bracket_count = 0i32;
        let mut brace_count = 0i32;
        let mut in_string = false;
        let mut string_char = ' ';

        for (line_num, line) in code.lines().enumerate() {
            let line_num = line_num + 1;
            let trimmed = line.trim();

            // Skip comments
            if trimmed.starts_with('#') {
                continue;
            }

            // Track string boundaries and count brackets
            let chars: Vec<char> = line.chars().collect();
            for &c in chars.iter() {
                // Check for string delimiters
                if !in_string && (c == '"' || c == '\'') {
                    // Start string (both single and triple quotes handled the same way for bracket counting)
                    in_string = true;
                    string_char = c;
                } else if in_string && c == string_char {
                    in_string = false;
                }

                if !in_string {
                    match c {
                        '(' => paren_count += 1,
                        ')' => paren_count -= 1,
                        '[' => bracket_count += 1,
                        ']' => bracket_count -= 1,
                        '{' => brace_count += 1,
                        '}' => brace_count -= 1,
                        _ => {}
                    }
                }
            }

            // Check for common Python syntax patterns
            if trimmed.ends_with(':') && !trimmed.starts_with('#') {
                // This is likely a block statement (def, class, if, for, etc.)
                // Check if indentation follows
            }

            // Check for tabs vs spaces (Python style warning)
            if line.starts_with('\t') && line.contains("    ") {
                warnings.push(
                    ValidationWarning::new(
                        "MIXED_INDENTATION",
                        format!("Line {} has mixed tabs and spaces", line_num),
                    )
                    .with_suggestion("Use consistent indentation (preferably 4 spaces)"),
                );
            }
        }

        // Check unmatched brackets
        if paren_count != 0 {
            errors.push(ValidationError::new(
                "UNMATCHED_PARENTHESES",
                format!(
                    "Unmatched parentheses: {} {}",
                    paren_count.abs(),
                    if paren_count > 0 {
                        "unclosed '('"
                    } else {
                        "extra ')'"
                    }
                ),
            ));
        }

        if bracket_count != 0 {
            errors.push(ValidationError::new(
                "UNMATCHED_BRACKETS",
                format!(
                    "Unmatched brackets: {} {}",
                    bracket_count.abs(),
                    if bracket_count > 0 {
                        "unclosed '['"
                    } else {
                        "extra ']'"
                    }
                ),
            ));
        }

        if brace_count != 0 {
            errors.push(ValidationError::new(
                "UNMATCHED_BRACES",
                format!(
                    "Unmatched braces: {} {}",
                    brace_count.abs(),
                    if brace_count > 0 {
                        "unclosed '{{'"
                    } else {
                        "extra '}}'"
                    }
                ),
            ));
        }

        if errors.is_empty() {
            let score = if warnings.is_empty() { 1.0 } else { 0.9 };
            ValidationResult::valid_with_warnings(warnings, score)
        } else {
            ValidationResult {
                valid: false,
                errors,
                warnings,
                score: 0.0,
            }
        }
    }

    /// Validates JavaScript/TypeScript code using basic heuristic checks.
    ///
    /// Note: This is a simplified validator. For full JS/TS validation,
    /// consider using swc_ecma_parser or an external Node.js process.
    fn validate_js(&self, code: &str) -> ValidationResult {
        let mut errors = Vec::new();
        let mut warnings = Vec::new();

        // Check for basic syntax issues
        let mut paren_count = 0i32;
        let mut bracket_count = 0i32;
        let mut brace_count = 0i32;
        let mut in_string = false;
        let mut in_template = false;
        let mut string_char = ' ';

        for (line_num, line) in code.lines().enumerate() {
            let line_num = line_num + 1;
            let trimmed = line.trim();

            // Skip comments
            if trimmed.starts_with("//") {
                continue;
            }

            let chars: Vec<char> = line.chars().collect();
            let mut i = 0;

            while i < chars.len() {
                let c = chars[i];

                // Handle template literals
                if c == '`' && !in_string {
                    in_template = !in_template;
                    i += 1;
                    continue;
                }

                // Handle regular strings
                if !in_template && !in_string && (c == '"' || c == '\'') {
                    in_string = true;
                    string_char = c;
                } else if in_string && c == string_char && (i == 0 || chars[i - 1] != '\\') {
                    in_string = false;
                }

                if !in_string && !in_template {
                    match c {
                        '(' => paren_count += 1,
                        ')' => paren_count -= 1,
                        '[' => bracket_count += 1,
                        ']' => bracket_count -= 1,
                        '{' => brace_count += 1,
                        '}' => brace_count -= 1,
                        _ => {}
                    }
                }

                i += 1;
            }

            // Check for common JS issues
            if trimmed.contains("var ") {
                warnings.push(
                    ValidationWarning::new(
                        "VAR_USAGE",
                        format!(
                            "Line {}: 'var' is used instead of 'let' or 'const'",
                            line_num
                        ),
                    )
                    .with_suggestion("Consider using 'let' or 'const' for better scoping"),
                );
            }
        }

        // Check unmatched brackets
        if paren_count != 0 {
            errors.push(ValidationError::new(
                "UNMATCHED_PARENTHESES",
                format!(
                    "Unmatched parentheses: {} {}",
                    paren_count.abs(),
                    if paren_count > 0 {
                        "unclosed '('"
                    } else {
                        "extra ')'"
                    }
                ),
            ));
        }

        if bracket_count != 0 {
            errors.push(ValidationError::new(
                "UNMATCHED_BRACKETS",
                format!(
                    "Unmatched brackets: {} {}",
                    bracket_count.abs(),
                    if bracket_count > 0 {
                        "unclosed '['"
                    } else {
                        "extra ']'"
                    }
                ),
            ));
        }

        if brace_count != 0 {
            errors.push(ValidationError::new(
                "UNMATCHED_BRACES",
                format!(
                    "Unmatched braces: {} {}",
                    brace_count.abs(),
                    if brace_count > 0 {
                        "unclosed '{{'"
                    } else {
                        "extra '}}'"
                    }
                ),
            ));
        }

        if errors.is_empty() {
            let score = if warnings.is_empty() { 1.0 } else { 0.9 };
            ValidationResult::valid_with_warnings(warnings, score)
        } else {
            ValidationResult {
                valid: false,
                errors,
                warnings,
                score: 0.0,
            }
        }
    }
}

#[async_trait]
impl ResultValidator for CodeValidator {
    async fn validate(&self, result: &str, _context: &ValidationContext) -> ValidationResult {
        match self.language {
            SupportedLanguage::Rust => self.validate_rust(result),
            SupportedLanguage::Python => self.validate_python(result),
            SupportedLanguage::JavaScript | SupportedLanguage::TypeScript => {
                self.validate_js(result)
            }
        }
    }

    fn name(&self) -> &str {
        "code_validator"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_json_validator_valid() {
        let validator = JsonValidator::new();
        let context = ValidationContext::default();

        let result = validator
            .validate(r#"{"name": "test", "value": 42}"#, &context)
            .await;
        assert!(result.valid);
        assert_eq!(result.score, 1.0);
    }

    #[tokio::test]
    async fn test_json_validator_invalid() {
        let validator = JsonValidator::new();
        let context = ValidationContext::default();

        let result = validator
            .validate(r#"{"name": "test", invalid}"#, &context)
            .await;
        assert!(!result.valid);
        assert!(!result.errors.is_empty());
        assert_eq!(result.errors[0].code, "INVALID_JSON");
    }

    #[tokio::test]
    async fn test_json_validator_required_fields() {
        let validator =
            JsonValidator::with_required_fields(vec!["name".to_string(), "id".to_string()]);
        let context = ValidationContext::default();

        // Missing required field
        let result = validator.validate(r#"{"name": "test"}"#, &context).await;
        assert!(!result.valid);
        assert!(
            result
                .errors
                .iter()
                .any(|e| e.code == "MISSING_REQUIRED_FIELD")
        );

        // All required fields present
        let result = validator
            .validate(r#"{"name": "test", "id": 1}"#, &context)
            .await;
        assert!(result.valid);
    }

    #[tokio::test]
    async fn test_json_validator_null_warning() {
        let validator = JsonValidator::new();
        let context = ValidationContext::default();

        let result = validator.validate(r#"{"name": null}"#, &context).await;
        assert!(result.valid);
        assert!(!result.warnings.is_empty());
        assert_eq!(result.warnings[0].code, "NULL_VALUE");
    }

    #[tokio::test]
    async fn test_code_validator_rust_valid() {
        let validator = CodeValidator::new(SupportedLanguage::Rust);
        let context = ValidationContext::default();

        let code = r#"
fn main() {
    println!("Hello, world!");
}
"#;

        let result = validator.validate(code, &context).await;
        assert!(result.valid);
    }

    #[tokio::test]
    async fn test_code_validator_rust_invalid() {
        let validator = CodeValidator::new(SupportedLanguage::Rust);
        let context = ValidationContext::default();

        let code = r#"
fn main() {
    println!("Hello, world!"
}
"#;

        let result = validator.validate(code, &context).await;
        assert!(!result.valid);
        // Our heuristic validator catches unmatched parentheses
        assert!(
            result
                .errors
                .iter()
                .any(|e| e.code == "UNMATCHED_PARENTHESES")
        );
    }

    #[tokio::test]
    async fn test_code_validator_python_valid() {
        let validator = CodeValidator::new(SupportedLanguage::Python);
        let context = ValidationContext::default();

        let code = r#"
def hello():
    print("Hello, world!")

if __name__ == "__main__":
    hello()
"#;

        let result = validator.validate(code, &context).await;
        assert!(result.valid);
    }

    #[tokio::test]
    async fn test_code_validator_python_unmatched() {
        let validator = CodeValidator::new(SupportedLanguage::Python);
        let context = ValidationContext::default();

        let code = r#"
def hello(:
    print("Hello")
"#;

        let result = validator.validate(code, &context).await;
        assert!(!result.valid);
        assert!(
            result
                .errors
                .iter()
                .any(|e| e.code == "UNMATCHED_PARENTHESES")
        );
    }

    #[tokio::test]
    async fn test_code_validator_js_valid() {
        let validator = CodeValidator::new(SupportedLanguage::JavaScript);
        let context = ValidationContext::default();

        let code = r#"
function hello() {
    console.log("Hello, world!");
}

hello();
"#;

        let result = validator.validate(code, &context).await;
        assert!(result.valid);
    }

    #[tokio::test]
    async fn test_code_validator_js_var_warning() {
        let validator = CodeValidator::new(SupportedLanguage::JavaScript);
        let context = ValidationContext::default();

        let code = r#"
var x = 10;
console.log(x);
"#;

        let result = validator.validate(code, &context).await;
        assert!(result.valid);
        assert!(!result.warnings.is_empty());
        assert!(result.warnings.iter().any(|w| w.code == "VAR_USAGE"));
    }

    #[test]
    fn test_supported_language_from_str() {
        assert_eq!(
            SupportedLanguage::from_str("rust"),
            Some(SupportedLanguage::Rust)
        );
        assert_eq!(
            SupportedLanguage::from_str("Python"),
            Some(SupportedLanguage::Python)
        );
        assert_eq!(
            SupportedLanguage::from_str("JS"),
            Some(SupportedLanguage::JavaScript)
        );
        assert_eq!(SupportedLanguage::from_str("unknown"), None);
    }

    #[test]
    fn test_code_validator_from_language_str() {
        assert!(CodeValidator::from_language_str("rust").is_some());
        assert!(CodeValidator::from_language_str("unknown").is_none());
    }
}
