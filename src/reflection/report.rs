//! Structured reflection reports for API responses.
//!
//! This module provides types for generating human-readable and machine-readable
//! reports from reflection results, suitable for inclusion in API responses
//! and frontend visualization.

// ReflectionReport will be integrated in R5.5
#![allow(dead_code)]

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{
    replanner::{PlanDiff, ReplanResult, ReplanTrigger},
    types::{IssueType, RecommendedAction, ReflectionIssue, ReflectionResult},
    validator::ValidationResult,
};

// ============================================================================
// Reflection Summary
// ============================================================================

/// Summary of a reflection evaluation for API responses.
///
/// This is a condensed version of `ReflectionResult` suitable for
/// inclusion in API responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReflectionSummary {
    /// Whether the reflection passed.
    pub passed: bool,
    /// Confidence score (0.0 - 1.0).
    pub confidence: f64,
    /// Number of issues identified.
    pub issue_count: usize,
    /// Number of reflection rounds performed.
    pub reflection_rounds: u32,
    /// Improvement suggestions.
    pub suggestions: Vec<String>,
    /// Brief summary of the evaluation.
    pub summary: String,
}

impl ReflectionSummary {
    /// Creates a summary from a reflection result.
    pub fn from_result(result: &ReflectionResult) -> Self {
        let summary = if result.passed {
            if result.issues.is_empty() {
                "Reflection passed with no issues".to_string()
            } else {
                format!(
                    "Reflection passed with {} minor issue(s)",
                    result.issues.len()
                )
            }
        } else {
            format!("Reflection failed with {} issue(s)", result.issues.len())
        };

        Self {
            passed: result.passed,
            confidence: result.confidence,
            issue_count: result.issues.len(),
            reflection_rounds: result.reflection_rounds,
            suggestions: result.suggestions.clone(),
            summary,
        }
    }

    /// Creates a passed summary with high confidence.
    pub fn passed() -> Self {
        Self {
            passed: true,
            confidence: 1.0,
            issue_count: 0,
            reflection_rounds: 1,
            suggestions: vec![],
            summary: "Reflection passed".to_string(),
        }
    }

    /// Creates a failed summary.
    pub fn failed(issue_count: usize, summary: impl Into<String>) -> Self {
        Self {
            passed: false,
            confidence: 0.0,
            issue_count,
            reflection_rounds: 1,
            suggestions: vec![],
            summary: summary.into(),
        }
    }
}

// ============================================================================
// Issue Report
// ============================================================================

/// Detailed report of an issue found during reflection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueReport {
    /// Type of issue.
    pub issue_type: String,
    /// Human-readable description.
    pub description: String,
    /// Severity (1-5).
    pub severity: u8,
    /// Severity level as text.
    pub severity_text: String,
    /// Related context (if any).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    /// Suggested fix (if any).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggested_fix: Option<String>,
}

impl IssueReport {
    /// Creates an issue report from a reflection issue.
    pub fn from_issue(issue: &ReflectionIssue) -> Self {
        let severity_text = match issue.severity {
            1 => "Very Low",
            2 => "Low",
            3 => "Medium",
            4 => "High",
            5 => "Critical",
            _ => "Unknown",
        };

        Self {
            issue_type: issue_type_to_string(&issue.issue_type),
            description: issue.description.clone(),
            severity: issue.severity,
            severity_text: severity_text.to_string(),
            context: issue.context.clone(),
            suggested_fix: None,
        }
    }

    /// Adds a suggested fix to the issue report.
    pub fn with_suggested_fix(mut self, fix: impl Into<String>) -> Self {
        self.suggested_fix = Some(fix.into());
        self
    }
}

/// Converts an IssueType to a human-readable string.
fn issue_type_to_string(issue_type: &IssueType) -> String {
    match issue_type {
        IssueType::IncompleteResult => "Incomplete Result".to_string(),
        IssueType::IncorrectResult => "Incorrect Result".to_string(),
        IssueType::RequirementNotMet => "Requirement Not Met".to_string(),
        IssueType::InefficientPath => "Inefficient Path".to_string(),
        IssueType::PotentialRisk => "Potential Risk".to_string(),
        IssueType::FormatError => "Format Error".to_string(),
        IssueType::LogicError => "Logic Error".to_string(),
    }
}

// ============================================================================
// Validation Report
// ============================================================================

/// Report of validation results.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationReport {
    /// Whether validation passed.
    pub valid: bool,
    /// Validation score (0.0 - 1.0).
    pub score: f64,
    /// Number of errors.
    pub error_count: usize,
    /// Number of warnings.
    pub warning_count: usize,
    /// Error messages.
    pub errors: Vec<String>,
    /// Warning messages.
    pub warnings: Vec<String>,
}

impl ValidationReport {
    /// Creates a validation report from a validation result.
    pub fn from_result(result: &ValidationResult) -> Self {
        Self {
            valid: result.valid,
            score: result.score,
            error_count: result.errors.len(),
            warning_count: result.warnings.len(),
            errors: result.errors.iter().map(|e| e.message.clone()).collect(),
            warnings: result.warnings.iter().map(|w| w.message.clone()).collect(),
        }
    }

    /// Creates a passed validation report.
    pub fn passed() -> Self {
        Self {
            valid: true,
            score: 1.0,
            error_count: 0,
            warning_count: 0,
            errors: vec![],
            warnings: vec![],
        }
    }
}

// ============================================================================
// Replan Summary
// ============================================================================

/// Summary of replanning activity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplanSummary {
    /// Number of replan attempts made.
    pub replan_count: u32,
    /// Trigger for the last replan.
    pub last_trigger: String,
    /// Summary of plan changes.
    pub changes: PlanChangeSummary,
}

impl ReplanSummary {
    /// Creates a replan summary from a replan result.
    pub fn from_result(result: &ReplanResult, replan_count: u32) -> Self {
        Self {
            replan_count,
            last_trigger: result.reason.clone(),
            changes: PlanChangeSummary::from_diff(result),
        }
    }

    /// Creates a summary from a trigger.
    pub fn from_trigger(trigger: &ReplanTrigger, replan_count: u32) -> Self {
        Self {
            replan_count,
            last_trigger: trigger.description(),
            changes: PlanChangeSummary::default(),
        }
    }
}

/// Summary of plan changes during replanning.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlanChangeSummary {
    /// Number of preserved subtasks.
    pub preserved_count: usize,
    /// Number of added subtasks.
    pub added_count: usize,
    /// Number of removed subtasks.
    pub removed_count: usize,
    /// Brief description of changes.
    pub description: String,
}

impl PlanChangeSummary {
    /// Creates a change summary from a replan result.
    pub fn from_diff(result: &ReplanResult) -> Self {
        let description = if result.added_subtasks.is_empty() && result.removed_subtasks.is_empty()
        {
            "Plan unchanged".to_string()
        } else {
            format!(
                "{} preserved, {} added, {} removed",
                result.preserved_subtasks.len(),
                result.added_subtasks.len(),
                result.removed_subtasks.len()
            )
        };

        Self {
            preserved_count: result.preserved_subtasks.len(),
            added_count: result.added_subtasks.len(),
            removed_count: result.removed_subtasks.len(),
            description,
        }
    }

    /// Creates a change summary from a plan diff.
    pub fn from_plan_diff(diff: &PlanDiff) -> Self {
        let description = format!(
            "{} preserved, {} added, {} removed",
            diff.preserved.len(),
            diff.added.len(),
            diff.removed.len()
        );

        Self {
            preserved_count: diff.preserved.len(),
            added_count: diff.added.len(),
            removed_count: diff.removed.len(),
            description,
        }
    }
}

// ============================================================================
// Action Report
// ============================================================================

/// Report of the recommended action from reflection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionReport {
    /// Action type.
    pub action_type: String,
    /// Action details (if any).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
    /// Whether user intervention is required.
    pub requires_user_action: bool,
}

impl ActionReport {
    /// Creates an action report from a recommended action.
    pub fn from_action(action: &RecommendedAction) -> Self {
        match action {
            RecommendedAction::Accept => Self {
                action_type: "accept".to_string(),
                details: None,
                requires_user_action: false,
            },
            RecommendedAction::AcceptWithFix(fix) => Self {
                action_type: "accept_with_fix".to_string(),
                details: Some(fix.clone()),
                requires_user_action: false,
            },
            RecommendedAction::Retry => Self {
                action_type: "retry".to_string(),
                details: None,
                requires_user_action: false,
            },
            RecommendedAction::RetryWithStrategy(strategy) => Self {
                action_type: "retry_with_strategy".to_string(),
                details: Some(strategy.clone()),
                requires_user_action: false,
            },
            RecommendedAction::Replan(request) => Self {
                action_type: "replan".to_string(),
                details: Some(request.reason.clone()),
                requires_user_action: false,
            },
            RecommendedAction::RequestClarification(question) => Self {
                action_type: "request_clarification".to_string(),
                details: Some(question.clone()),
                requires_user_action: true,
            },
            RecommendedAction::Abort(reason) => Self {
                action_type: "abort".to_string(),
                details: Some(reason.clone()),
                requires_user_action: true,
            },
        }
    }
}

// ============================================================================
// Full Reflection Report
// ============================================================================

/// Complete reflection report including all details.
///
/// This comprehensive report is suitable for detailed API responses
/// or frontend visualization of the reflection process.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReflectionReport {
    /// Report generation timestamp.
    pub timestamp: DateTime<Utc>,
    /// Overall reflection summary.
    pub summary: ReflectionSummary,
    /// Detailed issue reports.
    pub issues: Vec<IssueReport>,
    /// Validation report (if validation was performed).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation: Option<ValidationReport>,
    /// Recommended action.
    pub action: ActionReport,
    /// Replan summary (if replanning was performed).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replan: Option<ReplanSummary>,
    /// Additional metadata.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<ReportMetadata>,
}

impl ReflectionReport {
    /// Creates a new reflection report from a reflection result.
    pub fn from_result(result: &ReflectionResult) -> Self {
        Self {
            timestamp: Utc::now(),
            summary: ReflectionSummary::from_result(result),
            issues: result.issues.iter().map(IssueReport::from_issue).collect(),
            validation: None,
            action: ActionReport::from_action(&result.recommended_action),
            replan: None,
            metadata: None,
        }
    }

    /// Adds validation information to the report.
    pub fn with_validation(mut self, result: &ValidationResult) -> Self {
        self.validation = Some(ValidationReport::from_result(result));
        self
    }

    /// Adds replan information to the report.
    pub fn with_replan(mut self, result: &ReplanResult, count: u32) -> Self {
        self.replan = Some(ReplanSummary::from_result(result, count));
        self
    }

    /// Adds metadata to the report.
    pub fn with_metadata(mut self, metadata: ReportMetadata) -> Self {
        self.metadata = Some(metadata);
        self
    }

    /// Creates a simple passed report.
    pub fn passed() -> Self {
        Self {
            timestamp: Utc::now(),
            summary: ReflectionSummary::passed(),
            issues: vec![],
            validation: None,
            action: ActionReport::from_action(&RecommendedAction::Accept),
            replan: None,
            metadata: None,
        }
    }
}

/// Additional metadata for the reflection report.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReportMetadata {
    /// Task description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_description: Option<String>,
    /// Subtask ID (if applicable).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtask_id: Option<usize>,
    /// Time taken for reflection (in milliseconds).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reflection_time_ms: Option<u64>,
    /// Whether result was cached.
    #[serde(default)]
    pub from_cache: bool,
    /// Model used for reflection.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

impl ReportMetadata {
    /// Creates new metadata with a task description.
    pub fn new(task_description: impl Into<String>) -> Self {
        Self {
            task_description: Some(task_description.into()),
            ..Default::default()
        }
    }

    /// Sets the subtask ID.
    pub fn with_subtask_id(mut self, id: usize) -> Self {
        self.subtask_id = Some(id);
        self
    }

    /// Sets the reflection time.
    pub fn with_reflection_time(mut self, time_ms: u64) -> Self {
        self.reflection_time_ms = Some(time_ms);
        self
    }

    /// Marks the result as cached.
    pub fn mark_cached(mut self) -> Self {
        self.from_cache = true;
        self
    }

    /// Sets the model used.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }
}

// ============================================================================
// Subtask Reflection Report
// ============================================================================

/// Reflection report for a single subtask.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubtaskReflectionReport {
    /// Subtask ID.
    pub subtask_id: usize,
    /// Subtask description.
    pub description: String,
    /// Reflection report.
    pub reflection: ReflectionReport,
}

impl SubtaskReflectionReport {
    /// Creates a new subtask reflection report.
    pub fn new(
        subtask_id: usize,
        description: impl Into<String>,
        result: &ReflectionResult,
    ) -> Self {
        Self {
            subtask_id,
            description: description.into(),
            reflection: ReflectionReport::from_result(result),
        }
    }
}

// ============================================================================
// Plan Reflection Report
// ============================================================================

/// Complete reflection report for an entire plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanReflectionReport {
    /// Plan ID.
    pub plan_id: String,
    /// Original goal.
    pub goal: String,
    /// Overall plan reflection.
    pub overall: ReflectionReport,
    /// Per-subtask reflections.
    pub subtasks: Vec<SubtaskReflectionReport>,
    /// Number of replan attempts.
    pub replan_count: u32,
    /// Total reflection rounds across all subtasks.
    pub total_reflection_rounds: u32,
}

impl PlanReflectionReport {
    /// Creates a new plan reflection report.
    pub fn new(
        plan_id: impl Into<String>,
        goal: impl Into<String>,
        overall: &ReflectionResult,
    ) -> Self {
        Self {
            plan_id: plan_id.into(),
            goal: goal.into(),
            overall: ReflectionReport::from_result(overall),
            subtasks: vec![],
            replan_count: 0,
            total_reflection_rounds: overall.reflection_rounds,
        }
    }

    /// Adds a subtask reflection report.
    pub fn add_subtask(&mut self, report: SubtaskReflectionReport) {
        self.total_reflection_rounds += report.reflection.summary.reflection_rounds;
        self.subtasks.push(report);
    }

    /// Sets the replan count.
    pub fn with_replan_count(mut self, count: u32) -> Self {
        self.replan_count = count;
        self
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reflection::types::{ReflectionIssue, ReplanRequest};

    fn make_test_result() -> ReflectionResult {
        ReflectionResult {
            passed: true,
            confidence: 0.85,
            issues: vec![ReflectionIssue::new(
                IssueType::InefficientPath,
                "Could be more efficient",
                2,
            )],
            suggestions: vec!["Consider caching".to_string()],
            recommended_action: RecommendedAction::Accept,
            reflection_rounds: 2,
        }
    }

    #[test]
    fn test_reflection_summary_from_result() {
        let result = make_test_result();
        let summary = ReflectionSummary::from_result(&result);

        assert!(summary.passed);
        assert_eq!(summary.confidence, 0.85);
        assert_eq!(summary.issue_count, 1);
        assert_eq!(summary.reflection_rounds, 2);
        assert_eq!(summary.suggestions.len(), 1);
    }

    #[test]
    fn test_reflection_summary_passed() {
        let summary = ReflectionSummary::passed();
        assert!(summary.passed);
        assert_eq!(summary.confidence, 1.0);
        assert_eq!(summary.issue_count, 0);
    }

    #[test]
    fn test_reflection_summary_failed() {
        let summary = ReflectionSummary::failed(3, "Multiple errors found");
        assert!(!summary.passed);
        assert_eq!(summary.issue_count, 3);
        assert!(summary.summary.contains("Multiple errors"));
    }

    #[test]
    fn test_issue_report_from_issue() {
        let issue = ReflectionIssue::new(IssueType::IncorrectResult, "Wrong answer", 4)
            .with_context("Expected 42");

        let report = IssueReport::from_issue(&issue);
        assert_eq!(report.issue_type, "Incorrect Result");
        assert_eq!(report.severity, 4);
        assert_eq!(report.severity_text, "High");
        assert_eq!(report.context, Some("Expected 42".to_string()));
    }

    #[test]
    fn test_issue_report_with_fix() {
        let issue = ReflectionIssue::new(IssueType::FormatError, "Invalid JSON", 3);
        let report = IssueReport::from_issue(&issue).with_suggested_fix("Add missing bracket");

        assert_eq!(
            report.suggested_fix,
            Some("Add missing bracket".to_string())
        );
    }

    #[test]
    fn test_validation_report_from_result() {
        let result = ValidationResult {
            valid: true,
            errors: vec![],
            warnings: vec![],
            score: 0.95,
        };

        let report = ValidationReport::from_result(&result);
        assert!(report.valid);
        assert_eq!(report.score, 0.95);
        assert_eq!(report.error_count, 0);
    }

    #[test]
    fn test_validation_report_passed() {
        let report = ValidationReport::passed();
        assert!(report.valid);
        assert_eq!(report.score, 1.0);
    }

    #[test]
    fn test_action_report_accept() {
        let action = RecommendedAction::Accept;
        let report = ActionReport::from_action(&action);

        assert_eq!(report.action_type, "accept");
        assert!(report.details.is_none());
        assert!(!report.requires_user_action);
    }

    #[test]
    fn test_action_report_retry_with_strategy() {
        let action = RecommendedAction::RetryWithStrategy("Use fallback API".to_string());
        let report = ActionReport::from_action(&action);

        assert_eq!(report.action_type, "retry_with_strategy");
        assert_eq!(report.details, Some("Use fallback API".to_string()));
    }

    #[test]
    fn test_action_report_request_clarification() {
        let action = RecommendedAction::RequestClarification("Which format?".to_string());
        let report = ActionReport::from_action(&action);

        assert_eq!(report.action_type, "request_clarification");
        assert!(report.requires_user_action);
    }

    #[test]
    fn test_action_report_replan() {
        let request = ReplanRequest::new("Task failed");
        let action = RecommendedAction::Replan(request);
        let report = ActionReport::from_action(&action);

        assert_eq!(report.action_type, "replan");
        assert_eq!(report.details, Some("Task failed".to_string()));
    }

    #[test]
    fn test_plan_change_summary_default() {
        let summary = PlanChangeSummary::default();
        assert_eq!(summary.preserved_count, 0);
        assert_eq!(summary.added_count, 0);
        assert_eq!(summary.removed_count, 0);
    }

    #[test]
    fn test_plan_change_summary_from_diff() {
        let diff = PlanDiff {
            preserved: [(1, 1), (2, 2)].into_iter().collect(),
            added: vec![3],
            removed: vec![4, 5],
        };

        let summary = PlanChangeSummary::from_plan_diff(&diff);
        assert_eq!(summary.preserved_count, 2);
        assert_eq!(summary.added_count, 1);
        assert_eq!(summary.removed_count, 2);
        assert!(summary.description.contains("2 preserved"));
    }

    #[test]
    fn test_reflection_report_from_result() {
        let result = make_test_result();
        let report = ReflectionReport::from_result(&result);

        assert!(report.summary.passed);
        assert_eq!(report.issues.len(), 1);
        assert_eq!(report.action.action_type, "accept");
        assert!(report.validation.is_none());
        assert!(report.replan.is_none());
    }

    #[test]
    fn test_reflection_report_with_validation() {
        let result = make_test_result();
        let validation = ValidationResult::valid();

        let report = ReflectionReport::from_result(&result).with_validation(&validation);

        assert!(report.validation.is_some());
        assert!(report.validation.unwrap().valid);
    }

    #[test]
    fn test_reflection_report_passed() {
        let report = ReflectionReport::passed();
        assert!(report.summary.passed);
        assert!(report.issues.is_empty());
    }

    #[test]
    fn test_report_metadata() {
        let metadata = ReportMetadata::new("Test task")
            .with_subtask_id(5)
            .with_reflection_time(150)
            .with_model("gpt-4")
            .mark_cached();

        assert_eq!(metadata.task_description, Some("Test task".to_string()));
        assert_eq!(metadata.subtask_id, Some(5));
        assert_eq!(metadata.reflection_time_ms, Some(150));
        assert_eq!(metadata.model, Some("gpt-4".to_string()));
        assert!(metadata.from_cache);
    }

    #[test]
    fn test_subtask_reflection_report() {
        let result = make_test_result();
        let report = SubtaskReflectionReport::new(1, "Calculate sum", &result);

        assert_eq!(report.subtask_id, 1);
        assert_eq!(report.description, "Calculate sum");
        assert!(report.reflection.summary.passed);
    }

    #[test]
    fn test_plan_reflection_report() {
        let result = make_test_result();
        let mut report = PlanReflectionReport::new("plan-123", "Complete the project", &result);

        assert_eq!(report.plan_id, "plan-123");
        assert_eq!(report.goal, "Complete the project");
        assert_eq!(report.total_reflection_rounds, 2);

        // Create a subtask result with 1 reflection round
        let subtask_result = ReflectionResult {
            reflection_rounds: 1,
            ..ReflectionResult::passed()
        };
        let subtask_report = SubtaskReflectionReport::new(1, "First task", &subtask_result);
        report.add_subtask(subtask_report);

        assert_eq!(report.subtasks.len(), 1);
        assert_eq!(report.total_reflection_rounds, 3); // 2 + 1
    }

    #[test]
    fn test_plan_reflection_report_with_replan() {
        let result = make_test_result();
        let report = PlanReflectionReport::new("plan-123", "Goal", &result).with_replan_count(2);

        assert_eq!(report.replan_count, 2);
    }

    #[test]
    fn test_issue_type_to_string() {
        assert_eq!(
            issue_type_to_string(&IssueType::IncompleteResult),
            "Incomplete Result"
        );
        assert_eq!(issue_type_to_string(&IssueType::LogicError), "Logic Error");
    }

    #[test]
    fn test_reflection_report_serialization() {
        let result = make_test_result();
        let report = ReflectionReport::from_result(&result);

        let json = serde_json::to_string(&report).expect("Failed to serialize");
        let deserialized: ReflectionReport =
            serde_json::from_str(&json).expect("Failed to deserialize");

        assert_eq!(deserialized.summary.passed, report.summary.passed);
        assert_eq!(deserialized.issues.len(), report.issues.len());
    }
}
