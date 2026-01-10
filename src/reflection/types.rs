//! Reflection system type definitions.
//!
//! This module defines the core types for the reflection and self-correction
//! system used in Plan mode to evaluate task execution results.

// Some types are reserved for future integration
#![allow(dead_code)]

use serde::{Deserialize, Serialize};

/// Result of a reflection evaluation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReflectionResult {
    /// Whether the result passed reflection validation.
    pub passed: bool,
    /// Confidence score (0.0 - 1.0).
    pub confidence: f64,
    /// Issues identified during reflection.
    pub issues: Vec<ReflectionIssue>,
    /// Improvement suggestions.
    pub suggestions: Vec<String>,
    /// Recommended action based on reflection.
    pub recommended_action: RecommendedAction,
    /// Number of reflection rounds performed.
    pub reflection_rounds: u32,
}

impl Default for ReflectionResult {
    fn default() -> Self {
        Self {
            passed: true,
            confidence: 1.0,
            issues: Vec::new(),
            suggestions: Vec::new(),
            recommended_action: RecommendedAction::Accept,
            reflection_rounds: 0,
        }
    }
}

impl ReflectionResult {
    /// Creates a new passing reflection result with high confidence.
    pub fn passed() -> Self {
        Self::default()
    }

    /// Creates a reflection result indicating failure.
    pub fn failed(issues: Vec<ReflectionIssue>, suggestions: Vec<String>) -> Self {
        let confidence = if issues.is_empty() {
            0.5
        } else {
            // Lower confidence based on issue severity
            let max_severity = issues.iter().map(|i| i.severity).max().unwrap_or(1);
            1.0 - (max_severity as f64 * 0.15).min(0.8)
        };

        Self {
            passed: false,
            confidence,
            issues,
            suggestions,
            recommended_action: RecommendedAction::Retry,
            reflection_rounds: 1,
        }
    }

    /// Returns a summary for logging.
    pub fn summary(&self) -> String {
        format!(
            "ReflectionResult[passed={}, confidence={:.2}, issues={}, rounds={}, action={:?}]",
            self.passed,
            self.confidence,
            self.issues.len(),
            self.reflection_rounds,
            self.recommended_action
        )
    }
}

/// An issue identified during reflection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReflectionIssue {
    /// Type of issue.
    pub issue_type: IssueType,
    /// Description of the issue.
    pub description: String,
    /// Severity level (1-5, where 5 is most severe).
    pub severity: u8,
    /// Related context (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
}

impl ReflectionIssue {
    /// Creates a new reflection issue.
    pub fn new(issue_type: IssueType, description: impl Into<String>, severity: u8) -> Self {
        Self {
            issue_type,
            description: description.into(),
            severity: severity.clamp(1, 5),
            context: None,
        }
    }

    /// Adds context to the issue.
    pub fn with_context(mut self, context: impl Into<String>) -> Self {
        self.context = Some(context.into());
        self
    }
}

/// Types of issues that can be identified during reflection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueType {
    /// Result is incomplete.
    IncompleteResult,
    /// Result is incorrect.
    IncorrectResult,
    /// Requirements not met.
    RequirementNotMet,
    /// Reasoning path was inefficient.
    InefficientPath,
    /// Potential risk identified.
    PotentialRisk,
    /// Format error in output.
    FormatError,
    /// Logic error in reasoning.
    LogicError,
}

impl IssueType {
    /// Returns a human-readable description of the issue type.
    pub fn description(&self) -> &'static str {
        match self {
            IssueType::IncompleteResult => "The result is incomplete or missing key elements",
            IssueType::IncorrectResult => "The result contains factual or logical errors",
            IssueType::RequirementNotMet => "The result does not satisfy the original requirements",
            IssueType::InefficientPath => "The reasoning path was inefficient or suboptimal",
            IssueType::PotentialRisk => "The result contains potential risks or issues",
            IssueType::FormatError => "The result has formatting or structural issues",
            IssueType::LogicError => "The reasoning contains logical errors or contradictions",
        }
    }
}

/// Recommended action based on reflection results.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "details")]
pub enum RecommendedAction {
    /// Accept the current result as-is.
    #[default]
    Accept,
    /// Accept the result with a specified fix applied.
    AcceptWithFix(String),
    /// Retry the current step.
    Retry,
    /// Retry with a different strategy.
    RetryWithStrategy(String),
    /// Trigger replanning.
    Replan(ReplanRequest),
    /// Request clarification from user.
    RequestClarification(String),
    /// Abort the task.
    Abort(String),
}

/// Request to replan the task.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReplanRequest {
    /// Reason for replanning.
    pub reason: String,
    /// IDs of failed subtasks.
    pub failed_subtasks: Vec<usize>,
    /// Suggested new subtasks to add.
    pub suggested_subtasks: Vec<String>,
}

impl ReplanRequest {
    /// Creates a new replan request.
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
            failed_subtasks: Vec::new(),
            suggested_subtasks: Vec::new(),
        }
    }

    /// Adds a failed subtask ID.
    pub fn with_failed_subtask(mut self, subtask_id: usize) -> Self {
        self.failed_subtasks.push(subtask_id);
        self
    }

    /// Adds a suggested subtask.
    pub fn with_suggested_subtask(mut self, subtask: impl Into<String>) -> Self {
        self.suggested_subtasks.push(subtask.into());
        self
    }
}

/// Context for reflection evaluation.
#[derive(Debug, Clone)]
pub struct ReflectionContext {
    /// Original task description.
    pub task_description: String,
    /// Dependencies results (if any).
    pub dependencies_results: Vec<String>,
    /// Number of iterations executed.
    pub iteration_count: u32,
    /// Tool calls made during execution.
    pub tool_calls: Vec<String>,
    /// Errors encountered during execution.
    pub errors: Vec<String>,
    /// Time taken for execution.
    pub time_taken_ms: u64,
}

impl ReflectionContext {
    /// Creates a new reflection context.
    pub fn new(task_description: impl Into<String>) -> Self {
        Self {
            task_description: task_description.into(),
            dependencies_results: Vec::new(),
            iteration_count: 0,
            tool_calls: Vec::new(),
            errors: Vec::new(),
            time_taken_ms: 0,
        }
    }

    /// Adds dependency results.
    pub fn with_dependencies(mut self, results: Vec<String>) -> Self {
        self.dependencies_results = results;
        self
    }

    /// Sets iteration count.
    pub fn with_iterations(mut self, count: u32) -> Self {
        self.iteration_count = count;
        self
    }

    /// Adds tool calls.
    pub fn with_tool_calls(mut self, calls: Vec<String>) -> Self {
        self.tool_calls = calls;
        self
    }

    /// Adds errors.
    pub fn with_errors(mut self, errors: Vec<String>) -> Self {
        self.errors = errors;
        self
    }

    /// Sets time taken.
    pub fn with_time_taken(mut self, time_ms: u64) -> Self {
        self.time_taken_ms = time_ms;
        self
    }
}

/// Configuration for the reflection system.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReflectionConfig {
    /// Whether reflection is enabled.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Confidence threshold below which deep reflection is triggered.
    #[serde(default = "default_confidence_threshold")]
    pub confidence_threshold: f64,
    /// Maximum number of reflection rounds.
    #[serde(default = "default_max_rounds")]
    pub max_reflection_rounds: u32,
    /// Whether result validation is enabled.
    #[serde(default = "default_enabled")]
    pub enable_result_validation: bool,
    /// Whether path optimization suggestions are enabled.
    #[serde(default)]
    pub enable_path_optimization: bool,
}

fn default_enabled() -> bool {
    true
}

fn default_confidence_threshold() -> f64 {
    0.7
}

fn default_max_rounds() -> u32 {
    3
}

impl Default for ReflectionConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            confidence_threshold: default_confidence_threshold(),
            max_reflection_rounds: default_max_rounds(),
            enable_result_validation: default_enabled(),
            enable_path_optimization: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reflection_result_default() {
        let result = ReflectionResult::default();
        assert!(result.passed);
        assert_eq!(result.confidence, 1.0);
        assert!(result.issues.is_empty());
        assert!(result.suggestions.is_empty());
        assert_eq!(result.recommended_action, RecommendedAction::Accept);
    }

    #[test]
    fn test_reflection_result_passed() {
        let result = ReflectionResult::passed();
        assert!(result.passed);
        assert_eq!(result.confidence, 1.0);
    }

    #[test]
    fn test_reflection_result_failed() {
        let issues = vec![
            ReflectionIssue::new(IssueType::IncompleteResult, "Missing step", 3),
            ReflectionIssue::new(IssueType::FormatError, "Wrong format", 2),
        ];
        let suggestions = vec!["Add missing step".to_string()];

        let result = ReflectionResult::failed(issues.clone(), suggestions);

        assert!(!result.passed);
        assert!(result.confidence < 1.0);
        assert_eq!(result.issues.len(), 2);
        assert_eq!(result.suggestions.len(), 1);
        assert_eq!(result.recommended_action, RecommendedAction::Retry);
    }

    #[test]
    fn test_reflection_issue_creation() {
        let issue = ReflectionIssue::new(IssueType::IncorrectResult, "Wrong answer", 4)
            .with_context("Expected 42, got 43");

        assert_eq!(issue.issue_type, IssueType::IncorrectResult);
        assert_eq!(issue.description, "Wrong answer");
        assert_eq!(issue.severity, 4);
        assert_eq!(issue.context, Some("Expected 42, got 43".to_string()));
    }

    #[test]
    fn test_issue_severity_clamped() {
        let issue = ReflectionIssue::new(IssueType::PotentialRisk, "Test", 10);
        assert_eq!(issue.severity, 5);

        let issue = ReflectionIssue::new(IssueType::PotentialRisk, "Test", 0);
        assert_eq!(issue.severity, 1);
    }

    #[test]
    fn test_replan_request() {
        let request = ReplanRequest::new("File not found")
            .with_failed_subtask(1)
            .with_failed_subtask(2)
            .with_suggested_subtask("Create the file first");

        assert_eq!(request.reason, "File not found");
        assert_eq!(request.failed_subtasks, vec![1, 2]);
        assert_eq!(request.suggested_subtasks, vec!["Create the file first"]);
    }

    #[test]
    fn test_reflection_context() {
        let context = ReflectionContext::new("Test task")
            .with_dependencies(vec!["Result 1".to_string()])
            .with_iterations(3)
            .with_tool_calls(vec!["tool1".to_string(), "tool2".to_string()])
            .with_errors(vec!["Error 1".to_string()])
            .with_time_taken(1500);

        assert_eq!(context.task_description, "Test task");
        assert_eq!(context.dependencies_results.len(), 1);
        assert_eq!(context.iteration_count, 3);
        assert_eq!(context.tool_calls.len(), 2);
        assert_eq!(context.errors.len(), 1);
        assert_eq!(context.time_taken_ms, 1500);
    }

    #[test]
    fn test_reflection_config_default() {
        let config = ReflectionConfig::default();
        assert!(config.enabled);
        assert_eq!(config.confidence_threshold, 0.7);
        assert_eq!(config.max_reflection_rounds, 3);
        assert!(config.enable_result_validation);
        assert!(!config.enable_path_optimization);
    }

    #[test]
    fn test_reflection_result_summary() {
        let result = ReflectionResult::default();
        let summary = result.summary();
        assert!(summary.contains("passed=true"));
        assert!(summary.contains("confidence=1.00"));
    }

    #[test]
    fn test_recommended_action_serialization() {
        let action = RecommendedAction::RetryWithStrategy("Use fallback API".to_string());
        let json = serde_json::to_string(&action).unwrap();
        // serde(tag = "type", content = "details") produces {"type":"RetryWithStrategy","details":"..."}
        assert!(json.contains("RetryWithStrategy"));
        assert!(json.contains("Use fallback API"));

        let deserialized: RecommendedAction = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, action);
    }

    #[test]
    fn test_reflection_result_serialization() {
        let result = ReflectionResult {
            passed: true,
            confidence: 0.85,
            issues: vec![ReflectionIssue::new(IssueType::InefficientPath, "Slow", 2)],
            suggestions: vec!["Optimize".to_string()],
            recommended_action: RecommendedAction::AcceptWithFix("Minor fix".to_string()),
            reflection_rounds: 2,
        };

        let json = serde_json::to_string(&result).unwrap();
        let deserialized: ReflectionResult = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.passed, result.passed);
        assert_eq!(deserialized.confidence, result.confidence);
        assert_eq!(deserialized.issues.len(), 1);
        assert_eq!(deserialized.suggestions.len(), 1);
        assert_eq!(deserialized.reflection_rounds, 2);
    }
}
