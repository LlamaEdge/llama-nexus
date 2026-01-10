//! Semantic validator using LLM for content evaluation.
//!
//! This validator uses an LLM to evaluate the semantic correctness,
//! completeness, and quality of task execution results.

use std::sync::Arc;

use async_trait::async_trait;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde_json::Value;
use tokio::sync::RwLock;

use crate::{
    error::{ServerError, ServerResult},
    reflection::{
        engine::LlmServerInfo,
        validator::{
            ResultValidator, ValidationContext, ValidationError, ValidationResult,
            ValidationWarning,
        },
    },
};

/// Semantic validation prompt template.
const SEMANTIC_VALIDATION_PROMPT: &str = r#"You are a validation agent that evaluates task execution results for semantic correctness and completeness.

## Task Description
{task_description}

## Result to Validate
{result}

## Evaluation Criteria
1. **Completeness**: Does the result fully address all aspects of the task?
2. **Correctness**: Is the information accurate and logically sound?
3. **Relevance**: Is the result relevant to the task requirements?
4. **Quality**: Is the result well-structured and clear?

## Required Response Format (JSON)
{{
    "valid": true/false,
    "score": 0.0-1.0,
    "issues": [
        {{
            "type": "completeness|correctness|relevance|quality",
            "description": "description of the issue",
            "severity": "low|medium|high"
        }}
    ],
    "suggestions": ["suggestion 1", "suggestion 2"],
    "summary": "brief summary of the evaluation"
}}

Provide your evaluation as valid JSON only, no additional text."#;

/// Configuration for semantic validation.
#[derive(Debug, Clone)]
pub struct SemanticValidatorConfig {
    /// LLM temperature for validation (lower = more deterministic).
    pub temperature: f32,
    /// Maximum tokens for the validation response.
    pub max_tokens: u32,
    /// Minimum score threshold to consider result valid.
    pub validity_threshold: f64,
}

impl Default for SemanticValidatorConfig {
    fn default() -> Self {
        Self {
            temperature: 0.2,
            max_tokens: 1024,
            validity_threshold: 0.6,
        }
    }
}

/// Semantic validator using LLM evaluation.
///
/// This validator sends the task result to an LLM for semantic evaluation,
/// checking for completeness, correctness, and quality.
pub struct SemanticValidator {
    /// HTTP client for LLM calls.
    client: reqwest::Client,
    /// Server info for LLM calls.
    server: Arc<RwLock<LlmServerInfo>>,
    /// Configuration.
    config: SemanticValidatorConfig,
}

impl SemanticValidator {
    /// Creates a new semantic validator.
    pub fn new(server: Arc<RwLock<LlmServerInfo>>) -> Self {
        Self {
            client: reqwest::Client::new(),
            server,
            config: SemanticValidatorConfig::default(),
        }
    }

    /// Creates a semantic validator with custom configuration.
    pub fn with_config(
        server: Arc<RwLock<LlmServerInfo>>,
        config: SemanticValidatorConfig,
    ) -> Self {
        Self {
            client: reqwest::Client::new(),
            server,
            config,
        }
    }

    /// Calls the LLM for semantic validation.
    async fn call_llm(&self, prompt: &str) -> ServerResult<String> {
        let server = self.server.read().await;

        let request_body = serde_json::json!({
            "model": "default",
            "messages": [
                {
                    "role": "system",
                    "content": "You are a validation agent. Always respond with valid JSON."
                },
                {
                    "role": "user",
                    "content": prompt
                }
            ],
            "temperature": self.config.temperature,
            "max_tokens": self.config.max_tokens
        });

        let mut request = self
            .client
            .post(format!(
                "{}/chat/completions",
                server.url.trim_end_matches('/')
            ))
            .header(CONTENT_TYPE, "application/json")
            .json(&request_body);

        if let Some(api_key) = &server.api_key {
            request = request.header(AUTHORIZATION, format!("Bearer {}", api_key));
        }

        let response = request.send().await.map_err(|e| {
            ServerError::Operation(format!("Failed to call LLM for semantic validation: {}", e))
        })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(ServerError::Operation(format!(
                "LLM semantic validation call failed with status {}: {}",
                status, body
            )));
        }

        let response_json: Value = response.json().await.map_err(|e| {
            ServerError::Operation(format!("Failed to parse LLM validation response: {}", e))
        })?;

        let content = response_json["choices"][0]["message"]["content"]
            .as_str()
            .ok_or_else(|| ServerError::Operation("No content in LLM response".to_string()))?;

        Ok(content.to_string())
    }

    /// Parses the LLM response into a ValidationResult.
    fn parse_response(&self, response: &str) -> ValidationResult {
        // Try to extract JSON from the response
        let json_str = extract_json(response);

        let parsed: Value = match serde_json::from_str(&json_str) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("Failed to parse semantic validation response: {}", e);
                return ValidationResult::invalid(ValidationError::new(
                    "PARSE_ERROR",
                    format!("Failed to parse LLM validation response: {}", e),
                ));
            }
        };

        // Extract validation result
        let valid = parsed["valid"].as_bool().unwrap_or(false);
        let score = parsed["score"].as_f64().unwrap_or(0.5).clamp(0.0, 1.0);

        // Parse issues
        let mut errors = Vec::new();
        let mut warnings = Vec::new();

        if let Some(issues) = parsed["issues"].as_array() {
            for issue in issues {
                let issue_type = issue["type"].as_str().unwrap_or("unknown");
                let description = issue["description"].as_str().unwrap_or("Unknown issue");
                let severity = issue["severity"].as_str().unwrap_or("medium");

                match severity {
                    "high" => {
                        errors.push(ValidationError::new(
                            format!("SEMANTIC_{}", issue_type.to_uppercase()),
                            description,
                        ));
                    }
                    "medium" | "low" => {
                        let mut warning = ValidationWarning::new(
                            format!("SEMANTIC_{}", issue_type.to_uppercase()),
                            description,
                        );
                        // Add suggestion if available from the issues
                        if let Some(suggestion) = issue["suggestion"].as_str() {
                            warning = warning.with_suggestion(suggestion);
                        }
                        warnings.push(warning);
                    }
                    _ => {
                        warnings.push(ValidationWarning::new(
                            format!("SEMANTIC_{}", issue_type.to_uppercase()),
                            description,
                        ));
                    }
                }
            }
        }

        // Parse suggestions and add them to warnings
        if let Some(suggestions) = parsed["suggestions"].as_array() {
            for suggestion in suggestions {
                if let Some(s) = suggestion.as_str()
                    && !s.is_empty()
                {
                    warnings.push(
                        ValidationWarning::new("SEMANTIC_SUGGESTION", "Improvement suggestion")
                            .with_suggestion(s),
                    );
                }
            }
        }

        // Determine final validity based on score threshold
        let final_valid = valid && score >= self.config.validity_threshold && errors.is_empty();

        ValidationResult {
            valid: final_valid,
            errors,
            warnings,
            score,
        }
    }

    /// Validates the result semantically using the LLM.
    async fn validate_with_llm(
        &self,
        result: &str,
        context: &ValidationContext,
    ) -> ValidationResult {
        // Build the validation prompt
        let prompt = SEMANTIC_VALIDATION_PROMPT
            .replace("{task_description}", &context.task_description)
            .replace("{result}", result);

        // Call LLM
        match self.call_llm(&prompt).await {
            Ok(response) => self.parse_response(&response),
            Err(e) => {
                tracing::warn!("Semantic validation LLM call failed: {}", e);
                // Return a warning result instead of failing completely
                ValidationResult::valid_with_warnings(
                    vec![ValidationWarning::new(
                        "SEMANTIC_VALIDATION_UNAVAILABLE",
                        format!("Semantic validation could not be performed: {}", e),
                    )],
                    0.5, // Neutral score when validation unavailable
                )
            }
        }
    }
}

#[async_trait]
impl ResultValidator for SemanticValidator {
    async fn validate(&self, result: &str, context: &ValidationContext) -> ValidationResult {
        // Skip semantic validation for very short results
        if result.trim().is_empty() {
            return ValidationResult::invalid(ValidationError::new(
                "EMPTY_RESULT",
                "Result is empty",
            ));
        }

        // Skip semantic validation if no task description
        if context.task_description.is_empty() {
            return ValidationResult::valid_with_warnings(
                vec![ValidationWarning::new(
                    "NO_TASK_DESCRIPTION",
                    "No task description provided for semantic validation",
                )],
                0.7,
            );
        }

        self.validate_with_llm(result, context).await
    }

    fn name(&self) -> &str {
        "semantic_validator"
    }
}

/// Extracts JSON from a response that may contain markdown code blocks.
fn extract_json(response: &str) -> String {
    // Try to find JSON in code blocks first
    if let Some(start) = response.find("```json") {
        let start = start + 7;
        if let Some(end) = response[start..].find("```") {
            return response[start..start + end].trim().to_string();
        }
    }

    // Try to find JSON in generic code blocks
    if let Some(start) = response.find("```") {
        let start = start + 3;
        // Skip language identifier if present
        let start = if let Some(newline) = response[start..].find('\n') {
            start + newline + 1
        } else {
            start
        };
        if let Some(end) = response[start..].find("```") {
            return response[start..start + end].trim().to_string();
        }
    }

    // Try to find raw JSON object
    if let Some(start) = response.find('{')
        && let Some(end) = response.rfind('}')
    {
        return response[start..=end].to_string();
    }

    response.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_json_from_code_block() {
        let response = r#"Here's the validation:

```json
{
    "valid": true,
    "score": 0.85,
    "issues": [],
    "suggestions": []
}
```

That's my evaluation."#;

        let json = extract_json(response);
        assert!(json.contains("\"valid\": true"));
        assert!(json.contains("\"score\": 0.85"));
    }

    #[test]
    fn test_extract_json_raw() {
        let response = r#"{"valid": false, "score": 0.3, "issues": [], "suggestions": []}"#;
        let json = extract_json(response);
        assert!(json.contains("\"valid\": false"));
    }

    #[test]
    fn test_semantic_validator_config_default() {
        let config = SemanticValidatorConfig::default();
        assert_eq!(config.temperature, 0.2);
        assert_eq!(config.max_tokens, 1024);
        assert_eq!(config.validity_threshold, 0.6);
    }

    #[test]
    fn test_parse_response_valid() {
        let server = Arc::new(RwLock::new(LlmServerInfo {
            url: "http://localhost".to_string(),
            api_key: None,
        }));
        let validator = SemanticValidator::new(server);

        let response = r#"{
            "valid": true,
            "score": 0.9,
            "issues": [],
            "suggestions": ["Consider adding more details"],
            "summary": "Good result"
        }"#;

        let result = validator.parse_response(response);
        assert!(result.valid);
        assert_eq!(result.score, 0.9);
        assert!(result.errors.is_empty());
    }

    #[test]
    fn test_parse_response_with_issues() {
        let server = Arc::new(RwLock::new(LlmServerInfo {
            url: "http://localhost".to_string(),
            api_key: None,
        }));
        let validator = SemanticValidator::new(server);

        let response = r#"{
            "valid": false,
            "score": 0.4,
            "issues": [
                {
                    "type": "completeness",
                    "description": "Missing required information",
                    "severity": "high"
                },
                {
                    "type": "quality",
                    "description": "Could be more detailed",
                    "severity": "low"
                }
            ],
            "suggestions": [],
            "summary": "Needs improvement"
        }"#;

        let result = validator.parse_response(response);
        assert!(!result.valid);
        assert_eq!(result.score, 0.4);
        assert_eq!(result.errors.len(), 1);
        assert_eq!(result.warnings.len(), 1);
    }

    #[test]
    fn test_parse_response_invalid_json() {
        let server = Arc::new(RwLock::new(LlmServerInfo {
            url: "http://localhost".to_string(),
            api_key: None,
        }));
        let validator = SemanticValidator::new(server);

        let response = "This is not valid JSON";
        let result = validator.parse_response(response);
        assert!(!result.valid);
        assert!(result.errors.iter().any(|e| e.code == "PARSE_ERROR"));
    }

    #[tokio::test]
    async fn test_semantic_validator_empty_result() {
        let server = Arc::new(RwLock::new(LlmServerInfo {
            url: "http://localhost".to_string(),
            api_key: None,
        }));
        let validator = SemanticValidator::new(server);
        let context = ValidationContext::new("Test task");

        let result = validator.validate("", &context).await;
        assert!(!result.valid);
        assert!(result.errors.iter().any(|e| e.code == "EMPTY_RESULT"));
    }

    #[tokio::test]
    async fn test_semantic_validator_no_task_description() {
        let server = Arc::new(RwLock::new(LlmServerInfo {
            url: "http://localhost".to_string(),
            api_key: None,
        }));
        let validator = SemanticValidator::new(server);
        let context = ValidationContext::default();

        let result = validator.validate("Some result", &context).await;
        assert!(result.valid);
        assert!(
            result
                .warnings
                .iter()
                .any(|w| w.code == "NO_TASK_DESCRIPTION")
        );
    }
}
