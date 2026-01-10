//! Reflection engine implementation.
//!
//! The reflection engine evaluates task execution results and provides
//! structured feedback for quality improvement and self-correction.

// Some public API methods are reserved for future features
#![allow(dead_code)]

use std::sync::Arc;

use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde_json::Value;
use tokio::sync::RwLock;

use super::{
    prompts::{
        build_deep_reflection_prompt, build_plan_reflection_prompt, build_subtask_reflection_prompt,
    },
    types::{
        IssueType, RecommendedAction, ReflectionConfig, ReflectionContext, ReflectionIssue,
        ReflectionResult, ReplanRequest,
    },
    validator::{
        ExpectedFormat, ResultValidator, ValidationContext, ValidationResult, check_constraints,
    },
    validators::{CodeValidator, JsonValidator, SemanticValidator},
};
use crate::{
    chat::{
        planner::SubTask,
        trace::{PlanTrace, SubtaskTrace},
    },
    error::{ServerError, ServerResult},
};

/// Server information for LLM calls in reflection.
#[derive(Debug, Clone)]
pub struct LlmServerInfo {
    /// Server URL (base URL for API calls)
    pub url: String,
    /// API key (optional)
    pub api_key: Option<String>,
}

/// The reflection engine for evaluating task execution results.
pub struct ReflectionEngine {
    /// HTTP client for LLM calls.
    client: reqwest::Client,
    /// Server info for LLM calls.
    server: Arc<RwLock<LlmServerInfo>>,
    /// Reflection configuration.
    config: ReflectionConfig,
}

impl ReflectionEngine {
    /// Creates a new reflection engine.
    pub fn new(server: Arc<RwLock<LlmServerInfo>>, config: ReflectionConfig) -> Self {
        Self {
            client: reqwest::Client::new(),
            server,
            config,
        }
    }

    /// Returns whether reflection is enabled.
    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    /// Returns whether result validation is enabled.
    pub fn is_validation_enabled(&self) -> bool {
        self.config.enable_result_validation
    }

    /// Returns the LLM server info.
    pub fn server(&self) -> Arc<RwLock<LlmServerInfo>> {
        Arc::clone(&self.server)
    }

    /// Validates a result before reflection.
    ///
    /// This performs structural validation (JSON, code syntax) and constraint
    /// checking before the LLM-driven reflection. This can catch obvious
    /// issues quickly without needing an LLM call.
    pub async fn validate_result(
        &self,
        result: &str,
        validation_context: &ValidationContext,
    ) -> ValidationResult {
        if !self.config.enable_result_validation {
            return ValidationResult::valid();
        }

        let mut combined_result = ValidationResult::valid();

        // 1. Check constraints first (fast, local checks)
        let constraint_result = check_constraints(result, &validation_context.constraints);
        if !constraint_result.valid {
            return constraint_result;
        }
        combined_result.merge(constraint_result);

        // 2. Structural validation based on expected format
        if let Some(ref format) = validation_context.expected_format {
            let structural_result = self.validate_structure(result, format).await;
            combined_result.merge(structural_result);

            // If structural validation failed, don't proceed to semantic
            if !combined_result.valid {
                return combined_result;
            }
        }

        combined_result
    }

    /// Validates the structure of a result based on expected format.
    async fn validate_structure(&self, result: &str, format: &ExpectedFormat) -> ValidationResult {
        let context = ValidationContext::default();

        match format {
            ExpectedFormat::Json => {
                let validator = JsonValidator::new();
                validator.validate(result, &context).await
            }
            ExpectedFormat::Code { language } => {
                if let Some(validator) = CodeValidator::from_language_str(language) {
                    validator.validate(result, &context).await
                } else {
                    // Unknown language, skip validation
                    ValidationResult::valid()
                }
            }
            ExpectedFormat::Markdown | ExpectedFormat::PlainText => {
                // No specific validation for markdown/plain text
                ValidationResult::valid()
            }
        }
    }

    /// Performs semantic validation using LLM.
    ///
    /// This is a deeper validation that evaluates the content quality,
    /// completeness, and correctness using an LLM.
    pub async fn validate_semantically(
        &self,
        result: &str,
        validation_context: &ValidationContext,
    ) -> ValidationResult {
        if !self.config.enable_result_validation {
            return ValidationResult::valid();
        }

        let semantic_validator = SemanticValidator::new(Arc::clone(&self.server));
        semantic_validator
            .validate(result, validation_context)
            .await
    }

    /// Performs full validation including structural, constraint, and semantic checks.
    ///
    /// This is a convenience method that runs all validation stages in order:
    /// 1. Constraint validation (fast, local)
    /// 2. Structural validation (format-specific, local)
    /// 3. Semantic validation (LLM-driven)
    pub async fn validate_full(
        &self,
        result: &str,
        validation_context: &ValidationContext,
    ) -> ValidationResult {
        // First pass: structural and constraint validation
        let mut combined = self.validate_result(result, validation_context).await;

        if !combined.valid {
            return combined;
        }

        // Second pass: semantic validation (only if structural passed)
        let semantic = self.validate_semantically(result, validation_context).await;
        combined.merge(semantic);

        combined
    }

    /// Reflects on a subtask result.
    ///
    /// This method evaluates the quality of a subtask execution result
    /// and provides structured feedback.
    pub async fn reflect_on_subtask(
        &self,
        subtask: &SubTask,
        result: &str,
        _trace: &SubtaskTrace,
        context: &ReflectionContext,
    ) -> ServerResult<ReflectionResult> {
        if !self.config.enabled {
            return Ok(ReflectionResult::passed());
        }

        // Build reflection prompt
        let prompt = build_subtask_reflection_prompt(result, context);

        // Call LLM for reflection
        let response = self.call_llm_for_reflection(&prompt).await?;

        // Parse reflection response
        let mut reflection = self.parse_reflection_response(&response)?;
        reflection.reflection_rounds = 1;

        // If confidence is low, trigger deep reflection
        if reflection.confidence < self.config.confidence_threshold {
            tracing::debug!(
                "Low confidence ({:.2}), triggering deep reflection for subtask {}",
                reflection.confidence,
                subtask.id
            );
            return self
                .deep_reflect(subtask, result, &reflection, context)
                .await;
        }

        tracing::debug!(
            "Reflection completed for subtask {}: {}",
            subtask.id,
            reflection.summary()
        );

        Ok(reflection)
    }

    /// Reflects on the overall plan execution.
    pub async fn reflect_on_plan(
        &self,
        original_goal: &str,
        results: &[(usize, String)],
        trace: &PlanTrace,
    ) -> ServerResult<ReflectionResult> {
        if !self.config.enabled {
            return Ok(ReflectionResult::passed());
        }

        // Build subtask results summary
        let subtask_results = results
            .iter()
            .map(|(id, result)| {
                let status = trace
                    .subtask_traces
                    .iter()
                    .find(|t| t.subtask_id == *id)
                    .map(|t| format!("{:?}", t.status))
                    .unwrap_or_else(|| "Unknown".to_string());
                format!(
                    "Subtask {}: [{}] {}",
                    id,
                    status,
                    truncate_result(result, 200)
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        let prompt = build_plan_reflection_prompt(
            original_goal,
            trace.subtask_count,
            trace.completed_count(),
            trace.failed_count(),
            &subtask_results,
        );

        let response = self.call_llm_for_reflection(&prompt).await?;
        let mut reflection = self.parse_reflection_response(&response)?;
        reflection.reflection_rounds = 1;

        tracing::debug!("Plan reflection completed: {}", reflection.summary());

        Ok(reflection)
    }

    /// Performs deep reflection when initial confidence is low.
    async fn deep_reflect(
        &self,
        subtask: &SubTask,
        result: &str,
        initial_reflection: &ReflectionResult,
        context: &ReflectionContext,
    ) -> ServerResult<ReflectionResult> {
        let mut current_reflection = initial_reflection.clone();

        for round in 1..=self.config.max_reflection_rounds {
            // Build deep reflection prompt
            let initial_json = serde_json::to_string_pretty(&current_reflection)
                .unwrap_or_else(|_| format!("{:?}", current_reflection));

            let prompt = build_deep_reflection_prompt(
                result,
                &initial_json,
                &context.task_description,
                round,
            );

            let response = self.call_llm_for_reflection(&prompt).await?;
            current_reflection = self.parse_reflection_response(&response)?;
            current_reflection.reflection_rounds = round + 1;

            tracing::debug!(
                "Deep reflection round {} for subtask {}: confidence={:.2}",
                round,
                subtask.id,
                current_reflection.confidence
            );

            // Early termination if confidence is high enough
            if current_reflection.confidence >= self.config.confidence_threshold {
                break;
            }
        }

        Ok(current_reflection)
    }

    /// Calls the LLM for reflection evaluation.
    async fn call_llm_for_reflection(&self, prompt: &str) -> ServerResult<String> {
        let server = self.server.read().await;

        let request_body = serde_json::json!({
            "model": "default",
            "messages": [
                {
                    "role": "system",
                    "content": "You are a reflection agent that evaluates task execution results. Always respond with valid JSON."
                },
                {
                    "role": "user",
                    "content": prompt
                }
            ],
            "temperature": 0.3,
            "max_tokens": 2048
        });

        let mut request = self
            .client
            .post(format!("{}/v1/chat/completions", server.url))
            .header(CONTENT_TYPE, "application/json")
            .json(&request_body);

        if let Some(api_key) = &server.api_key {
            request = request.header(AUTHORIZATION, format!("Bearer {}", api_key));
        }

        let response = request.send().await.map_err(|e| {
            ServerError::Operation(format!("Failed to call LLM for reflection: {}", e))
        })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(ServerError::Operation(format!(
                "LLM reflection call failed with status {}: {}",
                status, body
            )));
        }

        let response_json: Value = response.json().await.map_err(|e| {
            ServerError::Operation(format!("Failed to parse LLM reflection response: {}", e))
        })?;

        // Extract content from OpenAI-compatible response
        let content = response_json["choices"][0]["message"]["content"]
            .as_str()
            .ok_or_else(|| ServerError::Operation("No content in LLM response".to_string()))?;

        Ok(content.to_string())
    }

    /// Parses the LLM reflection response into a ReflectionResult.
    fn parse_reflection_response(&self, response: &str) -> ServerResult<ReflectionResult> {
        // Try to extract JSON from the response
        let json_str = extract_json_from_response(response);

        let parsed: Value = serde_json::from_str(&json_str).map_err(|e| {
            tracing::warn!(
                "Failed to parse reflection JSON: {}. Response: {}",
                e,
                response
            );
            ServerError::Operation(format!("Failed to parse reflection response: {}", e))
        })?;

        // Parse passed
        let passed = parsed["passed"].as_bool().unwrap_or(true);

        // Parse confidence
        let confidence = parsed["confidence"]
            .as_f64()
            .unwrap_or(if passed { 0.8 } else { 0.3 })
            .clamp(0.0, 1.0);

        // Parse issues
        let issues = if let Some(issues_array) = parsed["issues"].as_array() {
            issues_array
                .iter()
                .filter_map(|issue| {
                    let issue_type = issue["issue_type"].as_str().and_then(parse_issue_type)?;
                    let description = issue["description"].as_str()?.to_string();
                    let severity = issue["severity"].as_u64().unwrap_or(3) as u8;
                    let context = issue["context"].as_str().map(|s| s.to_string());

                    Some(ReflectionIssue {
                        issue_type,
                        description,
                        severity: severity.clamp(1, 5),
                        context,
                    })
                })
                .collect()
        } else {
            Vec::new()
        };

        // Parse suggestions
        let suggestions = if let Some(suggestions_array) = parsed["suggestions"].as_array() {
            suggestions_array
                .iter()
                .filter_map(|s| s.as_str().map(|s| s.to_string()))
                .collect()
        } else {
            Vec::new()
        };

        // Parse recommended action
        let recommended_action = parse_recommended_action(&parsed["recommended_action"]);

        Ok(ReflectionResult {
            passed,
            confidence,
            issues,
            suggestions,
            recommended_action,
            reflection_rounds: 0,
        })
    }
}

/// Extracts JSON from a response that may contain markdown code blocks.
fn extract_json_from_response(response: &str) -> String {
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

/// Parses an issue type string into an IssueType enum.
fn parse_issue_type(s: &str) -> Option<IssueType> {
    match s.to_lowercase().as_str() {
        "incomplete_result" | "incompleteresult" => Some(IssueType::IncompleteResult),
        "incorrect_result" | "incorrectresult" => Some(IssueType::IncorrectResult),
        "requirement_not_met" | "requirementnotmet" => Some(IssueType::RequirementNotMet),
        "inefficient_path" | "inefficientpath" => Some(IssueType::InefficientPath),
        "potential_risk" | "potentialrisk" => Some(IssueType::PotentialRisk),
        "format_error" | "formaterror" => Some(IssueType::FormatError),
        "logic_error" | "logicerror" => Some(IssueType::LogicError),
        _ => None,
    }
}

/// Parses a recommended action from JSON.
fn parse_recommended_action(value: &Value) -> RecommendedAction {
    let action_type = value["type"].as_str().unwrap_or("Accept");
    let details = value["details"].as_str().map(|s| s.to_string());

    match action_type.to_lowercase().as_str() {
        "accept" => RecommendedAction::Accept,
        "acceptwithfix" | "accept_with_fix" => {
            RecommendedAction::AcceptWithFix(details.unwrap_or_default())
        }
        "retry" => RecommendedAction::Retry,
        "retrywithstrategy" | "retry_with_strategy" => {
            RecommendedAction::RetryWithStrategy(details.unwrap_or_default())
        }
        "replan" => {
            let reason = details.unwrap_or_else(|| "Replanning required".to_string());
            RecommendedAction::Replan(ReplanRequest::new(reason))
        }
        "requestclarification" | "request_clarification" => {
            RecommendedAction::RequestClarification(details.unwrap_or_default())
        }
        "abort" => RecommendedAction::Abort(details.unwrap_or_else(|| "Task aborted".to_string())),
        _ => RecommendedAction::Accept,
    }
}

/// Truncates a result string for display.
fn truncate_result(result: &str, max_len: usize) -> String {
    if result.len() <= max_len {
        result.to_string()
    } else {
        format!("{}...", &result[..max_len])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_json_from_code_block() {
        let response = r#"Here's my analysis:

```json
{
  "passed": true,
  "confidence": 0.85
}
```

That's my reflection."#;

        let json = extract_json_from_response(response);
        assert!(json.contains("\"passed\": true"));
        assert!(json.contains("\"confidence\": 0.85"));
    }

    #[test]
    fn test_extract_json_raw() {
        let response = r#"{"passed": false, "confidence": 0.5}"#;
        let json = extract_json_from_response(response);
        assert!(json.contains("\"passed\": false"));
    }

    #[test]
    fn test_parse_issue_type() {
        assert_eq!(
            parse_issue_type("incomplete_result"),
            Some(IssueType::IncompleteResult)
        );
        assert_eq!(
            parse_issue_type("IncompleteResult"),
            Some(IssueType::IncompleteResult)
        );
        assert_eq!(
            parse_issue_type("INCORRECT_RESULT"),
            Some(IssueType::IncorrectResult)
        );
        assert_eq!(parse_issue_type("unknown"), None);
    }

    #[test]
    fn test_parse_recommended_action() {
        let accept = serde_json::json!({"type": "Accept"});
        assert_eq!(parse_recommended_action(&accept), RecommendedAction::Accept);

        let retry = serde_json::json!({"type": "Retry"});
        assert_eq!(parse_recommended_action(&retry), RecommendedAction::Retry);

        let retry_strategy = serde_json::json!({
            "type": "RetryWithStrategy",
            "details": "Use fallback API"
        });
        assert_eq!(
            parse_recommended_action(&retry_strategy),
            RecommendedAction::RetryWithStrategy("Use fallback API".to_string())
        );
    }

    #[test]
    fn test_truncate_result() {
        assert_eq!(truncate_result("short", 10), "short");
        assert_eq!(
            truncate_result("this is a longer string", 10),
            "this is a ..."
        );
    }
}
