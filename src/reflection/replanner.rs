//! Dynamic replanning system for Plan mode.
//!
//! This module implements dynamic replanning capabilities that allow the Plan mode
//! to automatically revise the task plan when failures or issues are detected.

// DynamicReplanner will be integrated in R5.2
#![allow(dead_code)]

use std::{collections::HashMap, sync::Arc, time::Duration};

use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::RwLock;

use super::engine::LlmServerInfo;
use crate::{
    chat::{planner::SubTaskStatus, trace::PlanTrace},
    error::{ServerError, ServerResult},
};

// ============================================================================
// Replan Trigger Types
// ============================================================================

/// Trigger conditions for replanning.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ReplanTrigger {
    /// A critical subtask (with many dependents) has failed.
    CriticalSubtaskFailed {
        /// ID of the failed subtask.
        subtask_id: usize,
        /// Error message.
        error: String,
        /// Number of subtasks that depend on this one.
        dependent_count: usize,
    },
    /// Multiple subtasks have failed, exceeding the failure rate threshold.
    MultipleFailures {
        /// IDs of failed subtasks.
        failed_ids: Vec<usize>,
        /// Failure rate (failed / total).
        failure_rate: f64,
    },
    /// The reflection engine has suggested replanning.
    ReflectionSuggested {
        /// Reason for replanning.
        reason: String,
        /// Suggested changes.
        suggested_changes: Vec<String>,
    },
    /// External context has changed.
    ContextChanged {
        /// Description of the old context.
        old_context: String,
        /// Description of the new context.
        new_context: String,
    },
    /// Time budget is critically low.
    TimeBudgetCritical {
        /// Remaining time in seconds.
        remaining_secs: u64,
        /// Number of pending subtasks.
        pending_subtasks: usize,
    },
}

impl ReplanTrigger {
    /// Returns a human-readable description of the trigger.
    pub fn description(&self) -> String {
        match self {
            ReplanTrigger::CriticalSubtaskFailed {
                subtask_id,
                error,
                dependent_count,
            } => {
                format!(
                    "Critical subtask {} failed ({} dependents blocked): {}",
                    subtask_id, dependent_count, error
                )
            }
            ReplanTrigger::MultipleFailures {
                failed_ids,
                failure_rate,
            } => {
                format!(
                    "Multiple failures detected: {} subtasks failed ({:.1}% failure rate)",
                    failed_ids.len(),
                    failure_rate * 100.0
                )
            }
            ReplanTrigger::ReflectionSuggested { reason, .. } => {
                format!("Reflection engine suggested replanning: {}", reason)
            }
            ReplanTrigger::ContextChanged {
                old_context,
                new_context,
            } => {
                format!(
                    "Context changed from '{}' to '{}'",
                    truncate_str(old_context, 50),
                    truncate_str(new_context, 50)
                )
            }
            ReplanTrigger::TimeBudgetCritical {
                remaining_secs,
                pending_subtasks,
            } => {
                format!(
                    "Time budget critical: {}s remaining for {} pending subtasks",
                    remaining_secs, pending_subtasks
                )
            }
        }
    }

    /// Checks if replanning should be triggered based on the current plan state.
    ///
    /// This method examines the plan trace and configuration to determine if
    /// any trigger conditions are met.
    pub fn should_replan(
        trace: &PlanTrace,
        config: &ReplanConfig,
        dependency_graph: &DependencyGraph,
    ) -> Option<ReplanTrigger> {
        if !config.enabled {
            return None;
        }

        // Collect failed subtasks
        let failed_subtasks: Vec<_> = trace
            .subtask_traces
            .iter()
            .filter(|t| matches!(t.status, SubTaskStatus::Failed(_)))
            .collect();

        // 1. Check for critical subtask failure
        for subtask in &failed_subtasks {
            let dependent_count = dependency_graph.get_dependents(subtask.subtask_id).len();
            if dependent_count >= config.critical_dependent_threshold {
                let error = if let SubTaskStatus::Failed(ref e) = subtask.status {
                    e.clone()
                } else {
                    "Unknown error".to_string()
                };

                return Some(ReplanTrigger::CriticalSubtaskFailed {
                    subtask_id: subtask.subtask_id,
                    error,
                    dependent_count,
                });
            }
        }

        // 2. Check failure rate
        let total = trace.subtask_traces.len();
        if total > 0 {
            let failure_rate = failed_subtasks.len() as f64 / total as f64;
            if failure_rate >= config.failure_rate_threshold {
                let failed_ids: Vec<_> = failed_subtasks.iter().map(|t| t.subtask_id).collect();

                return Some(ReplanTrigger::MultipleFailures {
                    failed_ids,
                    failure_rate,
                });
            }
        }

        // 3. Check time budget (if available)
        if let Some(ref time_budget) = config.time_budget {
            let remaining = time_budget.remaining();
            let pending_count = trace
                .subtask_traces
                .iter()
                .filter(|t| matches!(t.status, SubTaskStatus::Pending))
                .count();

            let min_required =
                Duration::from_secs(config.min_time_per_subtask * pending_count as u64);
            if remaining < min_required && pending_count > 0 {
                return Some(ReplanTrigger::TimeBudgetCritical {
                    remaining_secs: remaining.as_secs(),
                    pending_subtasks: pending_count,
                });
            }
        }

        None
    }
}

// ============================================================================
// Replan Configuration
// ============================================================================

/// Configuration for the dynamic replanning system.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplanConfig {
    /// Whether replanning is enabled.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Maximum number of replan attempts.
    #[serde(default = "default_max_attempts")]
    pub max_replan_attempts: u32,
    /// Threshold for considering a subtask critical (based on dependent count).
    #[serde(default = "default_critical_threshold")]
    pub critical_dependent_threshold: usize,
    /// Failure rate threshold for triggering replanning.
    #[serde(default = "default_failure_rate")]
    pub failure_rate_threshold: f64,
    /// Minimum time in seconds required per subtask.
    #[serde(default = "default_min_time")]
    pub min_time_per_subtask: u64,
    /// Optional time budget for the plan execution.
    #[serde(skip)]
    pub time_budget: Option<TimeBudget>,
}

fn default_enabled() -> bool {
    true
}

fn default_max_attempts() -> u32 {
    2
}

fn default_critical_threshold() -> usize {
    2
}

fn default_failure_rate() -> f64 {
    0.5
}

fn default_min_time() -> u64 {
    30
}

impl Default for ReplanConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            max_replan_attempts: default_max_attempts(),
            critical_dependent_threshold: default_critical_threshold(),
            failure_rate_threshold: default_failure_rate(),
            min_time_per_subtask: default_min_time(),
            time_budget: None,
        }
    }
}

// ============================================================================
// Time Budget
// ============================================================================

/// Time budget for plan execution.
#[derive(Debug, Clone)]
pub struct TimeBudget {
    /// Total allowed duration.
    total: Duration,
    /// Start time.
    start: std::time::Instant,
}

impl TimeBudget {
    /// Creates a new time budget.
    pub fn new(total: Duration) -> Self {
        Self {
            total,
            start: std::time::Instant::now(),
        }
    }

    /// Returns the elapsed time.
    pub fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }

    /// Returns the remaining time.
    pub fn remaining(&self) -> Duration {
        self.total.saturating_sub(self.elapsed())
    }

    /// Returns true if the budget is exhausted.
    pub fn is_exhausted(&self) -> bool {
        self.elapsed() >= self.total
    }
}

// ============================================================================
// Dependency Graph
// ============================================================================

/// Simple dependency graph for tracking subtask dependencies.
#[derive(Debug, Clone, Default)]
pub struct DependencyGraph {
    /// Map of subtask ID to its dependencies (subtasks it depends on).
    dependencies: HashMap<usize, Vec<usize>>,
    /// Map of subtask ID to its dependents (subtasks that depend on it).
    dependents: HashMap<usize, Vec<usize>>,
}

impl DependencyGraph {
    /// Creates a new empty dependency graph.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a dependency: `subtask_id` depends on `dependency_id`.
    pub fn add_dependency(&mut self, subtask_id: usize, dependency_id: usize) {
        self.dependencies
            .entry(subtask_id)
            .or_default()
            .push(dependency_id);
        self.dependents
            .entry(dependency_id)
            .or_default()
            .push(subtask_id);
    }

    /// Returns the dependencies of a subtask.
    pub fn get_dependencies(&self, subtask_id: usize) -> &[usize] {
        self.dependencies
            .get(&subtask_id)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Returns the dependents of a subtask.
    pub fn get_dependents(&self, subtask_id: usize) -> &[usize] {
        self.dependents
            .get(&subtask_id)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Returns true if the subtask has dependents.
    pub fn has_dependents(&self, subtask_id: usize) -> bool {
        !self.get_dependents(subtask_id).is_empty()
    }

    /// Builds a dependency graph from subtask dependency lists.
    pub fn from_subtask_dependencies(subtask_ids: &[usize], dependencies: &[Vec<usize>]) -> Self {
        let mut graph = Self::new();
        for (i, deps) in dependencies.iter().enumerate() {
            if i < subtask_ids.len() {
                let subtask_id = subtask_ids[i];
                for &dep_id in deps {
                    graph.add_dependency(subtask_id, dep_id);
                }
            }
        }
        graph
    }
}

// ============================================================================
// Replan Context
// ============================================================================

/// Context for replanning.
#[derive(Debug, Clone)]
pub struct ReplanContext {
    /// Original goal.
    pub original_goal: String,
    /// Completed subtask results (subtask_id -> result).
    pub completed_results: HashMap<usize, String>,
    /// Failed subtasks with their errors.
    pub failed_subtasks: Vec<FailedSubtaskInfo>,
    /// Pending subtask descriptions.
    pub pending_subtasks: Vec<SubtaskInfo>,
    /// The trigger that caused replanning.
    pub trigger: ReplanTrigger,
    /// Remaining time (if time budget is set).
    pub remaining_time: Option<Duration>,
}

/// Information about a failed subtask.
#[derive(Debug, Clone)]
pub struct FailedSubtaskInfo {
    /// Subtask ID.
    pub id: usize,
    /// Subtask description.
    pub description: String,
    /// Error message.
    pub error: String,
    /// Number of retry attempts made.
    pub retry_count: u32,
}

/// Basic subtask information.
#[derive(Debug, Clone)]
pub struct SubtaskInfo {
    /// Subtask ID.
    pub id: usize,
    /// Subtask description.
    pub description: String,
    /// Dependencies.
    pub dependencies: Vec<usize>,
}

// ============================================================================
// Replan Result
// ============================================================================

/// Result of a replanning operation.
#[derive(Debug, Clone)]
pub struct ReplanResult {
    /// The new plan (list of subtasks).
    pub new_subtasks: Vec<NewSubtask>,
    /// Mapping of preserved subtasks (old_id -> new_id).
    pub preserved_subtasks: HashMap<usize, usize>,
    /// IDs of newly added subtasks.
    pub added_subtasks: Vec<usize>,
    /// IDs of removed subtasks.
    pub removed_subtasks: Vec<usize>,
    /// Reason for the changes.
    pub reason: String,
    /// Notes from the replanning process.
    pub notes: String,
}

/// A subtask in the new plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewSubtask {
    /// Subtask ID in the new plan.
    pub id: usize,
    /// Subtask description.
    pub description: String,
    /// Dependencies (IDs of subtasks this depends on).
    pub dependencies: Vec<usize>,
    /// Whether this subtask is preserved from the original plan.
    pub preserved: bool,
    /// Original subtask ID if preserved.
    pub original_id: Option<usize>,
}

// ============================================================================
// Plan Diff
// ============================================================================

/// Difference between two plans.
#[derive(Debug, Clone, Default)]
pub struct PlanDiff {
    /// Subtasks that are preserved (old_id -> new_id).
    pub preserved: HashMap<usize, usize>,
    /// IDs of added subtasks.
    pub added: Vec<usize>,
    /// IDs of removed subtasks.
    pub removed: Vec<usize>,
}

impl PlanDiff {
    /// Computes the diff between original subtasks and new subtasks.
    ///
    /// A subtask is considered preserved if it has the same description
    /// (or is explicitly marked as preserved with an original_id).
    pub fn compute(original_subtasks: &[(usize, String)], new_subtasks: &[NewSubtask]) -> Self {
        let mut diff = PlanDiff::default();
        let mut original_matched: HashMap<usize, bool> = HashMap::new();

        // Initialize all original subtasks as unmatched
        for (id, _) in original_subtasks {
            original_matched.insert(*id, false);
        }

        // Find preserved subtasks
        for new_subtask in new_subtasks {
            if new_subtask.preserved {
                if let Some(original_id) = new_subtask.original_id {
                    // Explicitly marked as preserved
                    diff.preserved.insert(original_id, new_subtask.id);
                    original_matched.insert(original_id, true);
                }
            } else {
                // Check if description matches any original subtask
                for (orig_id, orig_desc) in original_subtasks {
                    if !original_matched.get(orig_id).copied().unwrap_or(false)
                        && descriptions_similar(orig_desc, &new_subtask.description)
                    {
                        diff.preserved.insert(*orig_id, new_subtask.id);
                        original_matched.insert(*orig_id, true);
                        break;
                    }
                }
            }
        }

        // Collect added subtasks (new subtasks that aren't preserved)
        for new_subtask in new_subtasks {
            if !diff.preserved.values().any(|&id| id == new_subtask.id) {
                diff.added.push(new_subtask.id);
            }
        }

        // Collect removed subtasks (original subtasks that aren't preserved)
        for (orig_id, matched) in &original_matched {
            if !matched {
                diff.removed.push(*orig_id);
            }
        }

        diff
    }

    /// Returns a summary of the diff.
    pub fn summary(&self) -> String {
        format!(
            "PlanDiff[preserved={}, added={}, removed={}]",
            self.preserved.len(),
            self.added.len(),
            self.removed.len()
        )
    }
}

/// Checks if two descriptions are similar enough to be considered the same subtask.
fn descriptions_similar(a: &str, b: &str) -> bool {
    // Simple comparison - exact match or very close
    let a_normalized = a.trim().to_lowercase();
    let b_normalized = b.trim().to_lowercase();

    if a_normalized == b_normalized {
        return true;
    }

    // Check if one contains the other (for minor wording changes)
    if a_normalized.len() > 10 && b_normalized.len() > 10 {
        let similarity = jaccard_similarity(&a_normalized, &b_normalized);
        return similarity > 0.8;
    }

    false
}

/// Computes Jaccard similarity between two strings based on words.
fn jaccard_similarity(a: &str, b: &str) -> f64 {
    let words_a: std::collections::HashSet<_> = a.split_whitespace().collect();
    let words_b: std::collections::HashSet<_> = b.split_whitespace().collect();

    let intersection = words_a.intersection(&words_b).count();
    let union = words_a.union(&words_b).count();

    if union == 0 {
        return 0.0;
    }

    intersection as f64 / union as f64
}

// ============================================================================
// Dynamic Replanner
// ============================================================================

/// Prompt template for replanning.
const REPLAN_PROMPT: &str = r#"You are a task replanning agent. The original plan has encountered issues and needs to be revised.

## Original Goal
{goal}

## Original Subtasks
{original_subtasks}

## Completed Subtasks (preserve these)
{completed_subtasks}

## Failed Subtasks
{failed_subtasks}

## Pending Subtasks
{pending_subtasks}

## Trigger for Replanning
{trigger_description}

## Constraints
- Time remaining: {remaining_time}
- Preserve completed work where possible
- Address the root cause of failures
- Ensure all dependencies are valid

## Your Task
Create a revised plan that:
1. Keeps all completed subtasks unchanged
2. Addresses the failures with alternative approaches
3. Ensures all dependencies are valid
4. Can complete within the remaining time

Respond with a new plan in JSON format:
```json
{
    "subtasks": [
        {
            "id": 1,
            "description": "Subtask description",
            "dependencies": [],
            "preserved": true,
            "original_id": 1
        },
        {
            "id": 2,
            "description": "New or modified subtask",
            "dependencies": [1],
            "preserved": false,
            "original_id": null
        }
    ],
    "notes": "Brief explanation of changes made"
}
```

Provide your response as valid JSON only."#;

/// Dynamic replanner for revising plans when failures occur.
pub struct DynamicReplanner {
    /// HTTP client for LLM calls.
    client: reqwest::Client,
    /// Server info for LLM calls.
    server: Arc<RwLock<LlmServerInfo>>,
    /// Replanning configuration.
    config: ReplanConfig,
}

impl DynamicReplanner {
    /// Creates a new dynamic replanner.
    pub fn new(server: Arc<RwLock<LlmServerInfo>>, config: ReplanConfig) -> Self {
        Self {
            client: reqwest::Client::new(),
            server,
            config,
        }
    }

    /// Creates a replanner with default configuration.
    pub fn with_defaults(server: Arc<RwLock<LlmServerInfo>>) -> Self {
        Self::new(server, ReplanConfig::default())
    }

    /// Returns the replanning configuration.
    pub fn config(&self) -> &ReplanConfig {
        &self.config
    }

    /// Returns whether replanning is enabled.
    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    /// Executes replanning based on the given context.
    pub async fn replan(&self, context: &ReplanContext) -> ServerResult<ReplanResult> {
        // Build the prompt
        let prompt = self.build_prompt(context);

        // Call LLM
        let response = self.call_llm(&prompt).await?;

        // Parse the response
        let (new_subtasks, notes) = self.parse_response(&response)?;

        // Build the result
        let original_subtasks: Vec<_> = context
            .completed_results
            .keys()
            .map(|id| (*id, format!("Completed subtask {}", id)))
            .chain(
                context
                    .pending_subtasks
                    .iter()
                    .map(|s| (s.id, s.description.clone())),
            )
            .chain(
                context
                    .failed_subtasks
                    .iter()
                    .map(|s| (s.id, s.description.clone())),
            )
            .collect();

        let diff = PlanDiff::compute(&original_subtasks, &new_subtasks);

        Ok(ReplanResult {
            new_subtasks,
            preserved_subtasks: diff.preserved,
            added_subtasks: diff.added,
            removed_subtasks: diff.removed,
            reason: context.trigger.description(),
            notes,
        })
    }

    /// Builds the replanning prompt.
    fn build_prompt(&self, context: &ReplanContext) -> String {
        let original_subtasks = format_subtask_list(&context.pending_subtasks);

        let completed_subtasks = if context.completed_results.is_empty() {
            "None".to_string()
        } else {
            context
                .completed_results
                .iter()
                .map(|(id, result)| format!("- Subtask {}: {}", id, truncate_str(result, 100)))
                .collect::<Vec<_>>()
                .join("\n")
        };

        let failed_subtasks = if context.failed_subtasks.is_empty() {
            "None".to_string()
        } else {
            context
                .failed_subtasks
                .iter()
                .map(|s| {
                    format!(
                        "- Subtask {} ({}): {} (retried {} times)",
                        s.id, s.description, s.error, s.retry_count
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        };

        let pending_subtasks = if context.pending_subtasks.is_empty() {
            "None".to_string()
        } else {
            context
                .pending_subtasks
                .iter()
                .map(|s| {
                    format!(
                        "- Subtask {} (deps: {:?}): {}",
                        s.id, s.dependencies, s.description
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        };

        let remaining_time = context
            .remaining_time
            .map(|d| format!("{}s", d.as_secs()))
            .unwrap_or_else(|| "No limit".to_string());

        REPLAN_PROMPT
            .replace("{goal}", &context.original_goal)
            .replace("{original_subtasks}", &original_subtasks)
            .replace("{completed_subtasks}", &completed_subtasks)
            .replace("{failed_subtasks}", &failed_subtasks)
            .replace("{pending_subtasks}", &pending_subtasks)
            .replace("{trigger_description}", &context.trigger.description())
            .replace("{remaining_time}", &remaining_time)
    }

    /// Calls the LLM for replanning.
    async fn call_llm(&self, prompt: &str) -> ServerResult<String> {
        let server = self.server.read().await;

        let request_body = serde_json::json!({
            "model": "default",
            "messages": [
                {
                    "role": "system",
                    "content": "You are a task replanning agent. Always respond with valid JSON."
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
            ServerError::Operation(format!("Failed to call LLM for replanning: {}", e))
        })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(ServerError::Operation(format!(
                "LLM replanning call failed with status {}: {}",
                status, body
            )));
        }

        let response_json: Value = response.json().await.map_err(|e| {
            ServerError::Operation(format!("Failed to parse LLM replanning response: {}", e))
        })?;

        let content = response_json["choices"][0]["message"]["content"]
            .as_str()
            .ok_or_else(|| ServerError::Operation("No content in LLM response".to_string()))?;

        Ok(content.to_string())
    }

    /// Parses the LLM response into a list of new subtasks.
    fn parse_response(&self, response: &str) -> ServerResult<(Vec<NewSubtask>, String)> {
        let json_str = extract_json(response);

        let parsed: Value = serde_json::from_str(&json_str).map_err(|e| {
            tracing::warn!(
                "Failed to parse replanning response: {}. Response: {}",
                e,
                response
            );
            ServerError::Operation(format!("Failed to parse replanning response: {}", e))
        })?;

        let subtasks = if let Some(subtasks_array) = parsed["subtasks"].as_array() {
            subtasks_array
                .iter()
                .filter_map(|s| {
                    let id = s["id"].as_u64()? as usize;
                    let description = s["description"].as_str()?.to_string();
                    let dependencies = s["dependencies"]
                        .as_array()
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_u64().map(|n| n as usize))
                                .collect()
                        })
                        .unwrap_or_default();
                    let preserved = s["preserved"].as_bool().unwrap_or(false);
                    let original_id = s["original_id"].as_u64().map(|n| n as usize);

                    Some(NewSubtask {
                        id,
                        description,
                        dependencies,
                        preserved,
                        original_id,
                    })
                })
                .collect()
        } else {
            return Err(ServerError::Operation(
                "No subtasks array in replanning response".to_string(),
            ));
        };

        let notes = parsed["notes"]
            .as_str()
            .unwrap_or("No notes provided")
            .to_string();

        Ok((subtasks, notes))
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Truncates a string to a maximum length.
fn truncate_str(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len])
    }
}

/// Formats a list of subtasks for display.
fn format_subtask_list(subtasks: &[SubtaskInfo]) -> String {
    if subtasks.is_empty() {
        return "None".to_string();
    }

    subtasks
        .iter()
        .map(|s| format!("- Subtask {}: {}", s.id, s.description))
        .collect::<Vec<_>>()
        .join("\n")
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

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ReplanTrigger tests

    #[test]
    fn test_replan_trigger_critical_subtask_description() {
        let trigger = ReplanTrigger::CriticalSubtaskFailed {
            subtask_id: 2,
            error: "File not found".to_string(),
            dependent_count: 3,
        };
        let desc = trigger.description();
        assert!(desc.contains("subtask 2"));
        assert!(desc.contains("3 dependents"));
        assert!(desc.contains("File not found"));
    }

    #[test]
    fn test_replan_trigger_multiple_failures_description() {
        let trigger = ReplanTrigger::MultipleFailures {
            failed_ids: vec![1, 3, 5],
            failure_rate: 0.6,
        };
        let desc = trigger.description();
        assert!(desc.contains("3 subtasks failed"));
        assert!(desc.contains("60.0%"));
    }

    #[test]
    fn test_replan_trigger_reflection_suggested_description() {
        let trigger = ReplanTrigger::ReflectionSuggested {
            reason: "Approach is inefficient".to_string(),
            suggested_changes: vec!["Use caching".to_string()],
        };
        let desc = trigger.description();
        assert!(desc.contains("Approach is inefficient"));
    }

    #[test]
    fn test_replan_trigger_context_changed_description() {
        let trigger = ReplanTrigger::ContextChanged {
            old_context: "API v1".to_string(),
            new_context: "API v2".to_string(),
        };
        let desc = trigger.description();
        assert!(desc.contains("API v1"));
        assert!(desc.contains("API v2"));
    }

    #[test]
    fn test_replan_trigger_time_budget_description() {
        let trigger = ReplanTrigger::TimeBudgetCritical {
            remaining_secs: 30,
            pending_subtasks: 5,
        };
        let desc = trigger.description();
        assert!(desc.contains("30s"));
        assert!(desc.contains("5 pending"));
    }

    // ReplanConfig tests

    #[test]
    fn test_replan_config_default() {
        let config = ReplanConfig::default();
        assert!(config.enabled);
        assert_eq!(config.max_replan_attempts, 2);
        assert_eq!(config.critical_dependent_threshold, 2);
        assert_eq!(config.failure_rate_threshold, 0.5);
        assert_eq!(config.min_time_per_subtask, 30);
        assert!(config.time_budget.is_none());
    }

    // TimeBudget tests

    #[test]
    fn test_time_budget_creation() {
        let budget = TimeBudget::new(Duration::from_secs(60));
        assert!(!budget.is_exhausted());
        assert!(budget.remaining() <= Duration::from_secs(60));
    }

    #[test]
    fn test_time_budget_exhausted() {
        let budget = TimeBudget::new(Duration::from_millis(1));
        std::thread::sleep(Duration::from_millis(10));
        assert!(budget.is_exhausted());
    }

    // DependencyGraph tests

    #[test]
    fn test_dependency_graph_creation() {
        let graph = DependencyGraph::new();
        assert!(graph.get_dependencies(1).is_empty());
        assert!(graph.get_dependents(1).is_empty());
    }

    #[test]
    fn test_dependency_graph_add_dependency() {
        let mut graph = DependencyGraph::new();
        graph.add_dependency(2, 1); // 2 depends on 1
        graph.add_dependency(3, 1); // 3 depends on 1
        graph.add_dependency(3, 2); // 3 also depends on 2

        assert_eq!(graph.get_dependencies(2), &[1]);
        assert_eq!(graph.get_dependencies(3), &[1, 2]);
        assert_eq!(graph.get_dependents(1), &[2, 3]);
        assert_eq!(graph.get_dependents(2), &[3]);
        assert!(graph.has_dependents(1));
        assert!(graph.has_dependents(2));
        assert!(!graph.has_dependents(3));
    }

    #[test]
    fn test_dependency_graph_from_subtask_dependencies() {
        let subtask_ids = vec![0, 1, 2];
        let dependencies = vec![
            vec![],     // 0 has no deps
            vec![0],    // 1 depends on 0
            vec![0, 1], // 2 depends on 0 and 1
        ];

        let graph = DependencyGraph::from_subtask_dependencies(&subtask_ids, &dependencies);

        assert!(graph.get_dependencies(0).is_empty());
        assert_eq!(graph.get_dependencies(1), &[0]);
        assert_eq!(graph.get_dependencies(2), &[0, 1]);
        assert_eq!(graph.get_dependents(0), &[1, 2]);
        assert_eq!(graph.get_dependents(1), &[2]);
    }

    // PlanDiff tests

    #[test]
    fn test_plan_diff_all_preserved() {
        let original = vec![(1, "Task A".to_string()), (2, "Task B".to_string())];
        let new_subtasks = vec![
            NewSubtask {
                id: 1,
                description: "Task A".to_string(),
                dependencies: vec![],
                preserved: true,
                original_id: Some(1),
            },
            NewSubtask {
                id: 2,
                description: "Task B".to_string(),
                dependencies: vec![1],
                preserved: true,
                original_id: Some(2),
            },
        ];

        let diff = PlanDiff::compute(&original, &new_subtasks);
        assert_eq!(diff.preserved.len(), 2);
        assert!(diff.added.is_empty());
        assert!(diff.removed.is_empty());
    }

    #[test]
    fn test_plan_diff_with_additions() {
        let original = vec![(1, "Task A".to_string())];
        let new_subtasks = vec![
            NewSubtask {
                id: 1,
                description: "Task A".to_string(),
                dependencies: vec![],
                preserved: true,
                original_id: Some(1),
            },
            NewSubtask {
                id: 2,
                description: "New task B".to_string(),
                dependencies: vec![1],
                preserved: false,
                original_id: None,
            },
        ];

        let diff = PlanDiff::compute(&original, &new_subtasks);
        assert_eq!(diff.preserved.len(), 1);
        assert_eq!(diff.added, vec![2]);
        assert!(diff.removed.is_empty());
    }

    #[test]
    fn test_plan_diff_with_removals() {
        let original = vec![
            (1, "Task A".to_string()),
            (2, "Task B".to_string()),
            (3, "Task C".to_string()),
        ];
        let new_subtasks = vec![NewSubtask {
            id: 1,
            description: "Task A".to_string(),
            dependencies: vec![],
            preserved: true,
            original_id: Some(1),
        }];

        let diff = PlanDiff::compute(&original, &new_subtasks);
        assert_eq!(diff.preserved.len(), 1);
        assert!(diff.added.is_empty());
        assert_eq!(diff.removed.len(), 2);
        assert!(diff.removed.contains(&2));
        assert!(diff.removed.contains(&3));
    }

    #[test]
    fn test_plan_diff_summary() {
        let diff = PlanDiff {
            preserved: [(1, 1), (2, 2)].into_iter().collect(),
            added: vec![3],
            removed: vec![4, 5],
        };
        let summary = diff.summary();
        assert!(summary.contains("preserved=2"));
        assert!(summary.contains("added=1"));
        assert!(summary.contains("removed=2"));
    }

    // Similarity tests

    #[test]
    fn test_descriptions_similar_exact() {
        assert!(descriptions_similar("Search for users", "Search for users"));
        assert!(descriptions_similar(
            "Search for users",
            "  Search for users  "
        ));
    }

    #[test]
    fn test_descriptions_similar_case_insensitive() {
        assert!(descriptions_similar("Search for Users", "search for users"));
    }

    #[test]
    fn test_descriptions_not_similar() {
        assert!(!descriptions_similar("Search", "Create"));
        assert!(!descriptions_similar("Find users", "Delete records"));
    }

    #[test]
    fn test_jaccard_similarity_identical() {
        assert_eq!(jaccard_similarity("hello world", "hello world"), 1.0);
    }

    #[test]
    fn test_jaccard_similarity_partial() {
        let sim = jaccard_similarity("hello world foo", "hello world bar");
        assert!(sim > 0.4 && sim < 0.8);
    }

    #[test]
    fn test_jaccard_similarity_none() {
        assert_eq!(jaccard_similarity("hello", "world"), 0.0);
    }

    #[test]
    fn test_jaccard_similarity_empty() {
        assert_eq!(jaccard_similarity("", ""), 0.0);
    }

    // Helper function tests

    #[test]
    fn test_truncate_str() {
        assert_eq!(truncate_str("hello", 10), "hello");
        assert_eq!(truncate_str("hello world", 5), "hello...");
    }

    #[test]
    fn test_format_subtask_list_empty() {
        assert_eq!(format_subtask_list(&[]), "None");
    }

    #[test]
    fn test_format_subtask_list_with_items() {
        let subtasks = vec![
            SubtaskInfo {
                id: 1,
                description: "Task A".to_string(),
                dependencies: vec![],
            },
            SubtaskInfo {
                id: 2,
                description: "Task B".to_string(),
                dependencies: vec![1],
            },
        ];
        let result = format_subtask_list(&subtasks);
        assert!(result.contains("Subtask 1: Task A"));
        assert!(result.contains("Subtask 2: Task B"));
    }

    #[test]
    fn test_extract_json_from_code_block() {
        let response = r#"Here's the plan:

```json
{
    "subtasks": [],
    "notes": "Test"
}
```

That's the plan."#;
        let json = extract_json(response);
        assert!(json.contains("\"subtasks\""));
        assert!(json.contains("\"notes\""));
    }

    #[test]
    fn test_extract_json_raw() {
        let response = r#"{"subtasks": [], "notes": "Test"}"#;
        let json = extract_json(response);
        assert_eq!(json, response);
    }

    // ReplanContext tests

    #[test]
    fn test_failed_subtask_info_creation() {
        let info = FailedSubtaskInfo {
            id: 1,
            description: "Parse file".to_string(),
            error: "File not found".to_string(),
            retry_count: 2,
        };
        assert_eq!(info.id, 1);
        assert_eq!(info.retry_count, 2);
    }

    // NewSubtask tests

    #[test]
    fn test_new_subtask_serialization() {
        let subtask = NewSubtask {
            id: 1,
            description: "Test task".to_string(),
            dependencies: vec![],
            preserved: false,
            original_id: None,
        };
        let json = serde_json::to_string(&subtask).unwrap();
        assert!(json.contains("\"id\":1"));
        assert!(json.contains("\"description\":\"Test task\""));
    }

    #[test]
    fn test_new_subtask_deserialization() {
        let json = r#"{
            "id": 2,
            "description": "Another task",
            "dependencies": [1],
            "preserved": true,
            "original_id": 2
        }"#;
        let subtask: NewSubtask = serde_json::from_str(json).unwrap();
        assert_eq!(subtask.id, 2);
        assert_eq!(subtask.dependencies, vec![1]);
        assert!(subtask.preserved);
        assert_eq!(subtask.original_id, Some(2));
    }

    // ReplanResult tests

    #[test]
    fn test_replan_result_creation() {
        let result = ReplanResult {
            new_subtasks: vec![],
            preserved_subtasks: HashMap::new(),
            added_subtasks: vec![1, 2],
            removed_subtasks: vec![3],
            reason: "Test reason".to_string(),
            notes: "Test notes".to_string(),
        };
        assert_eq!(result.added_subtasks.len(), 2);
        assert_eq!(result.removed_subtasks.len(), 1);
    }

    // DynamicReplanner tests

    #[tokio::test]
    async fn test_dynamic_replanner_creation() {
        let server = Arc::new(RwLock::new(LlmServerInfo {
            url: "http://localhost".to_string(),
            api_key: None,
        }));
        let replanner = DynamicReplanner::with_defaults(server);
        assert!(replanner.is_enabled());
    }

    #[test]
    fn test_dynamic_replanner_parse_response_valid() {
        let server = Arc::new(RwLock::new(LlmServerInfo {
            url: "http://localhost".to_string(),
            api_key: None,
        }));
        let replanner = DynamicReplanner::with_defaults(
            tokio::runtime::Runtime::new()
                .unwrap()
                .block_on(async { server }),
        );

        let response = r#"{
            "subtasks": [
                {
                    "id": 1,
                    "description": "First task",
                    "dependencies": [],
                    "preserved": true,
                    "original_id": 1
                },
                {
                    "id": 2,
                    "description": "Second task",
                    "dependencies": [1],
                    "preserved": false,
                    "original_id": null
                }
            ],
            "notes": "Modified plan to address failures"
        }"#;

        let (subtasks, notes) = replanner.parse_response(response).unwrap();
        assert_eq!(subtasks.len(), 2);
        assert_eq!(subtasks[0].id, 1);
        assert!(subtasks[0].preserved);
        assert_eq!(subtasks[1].id, 2);
        assert!(!subtasks[1].preserved);
        assert!(notes.contains("Modified plan"));
    }

    #[test]
    fn test_dynamic_replanner_parse_response_with_code_block() {
        let server = Arc::new(RwLock::new(LlmServerInfo {
            url: "http://localhost".to_string(),
            api_key: None,
        }));
        let replanner = DynamicReplanner::with_defaults(
            tokio::runtime::Runtime::new()
                .unwrap()
                .block_on(async { server }),
        );

        let response = r#"Here's the revised plan:

```json
{
    "subtasks": [
        {
            "id": 1,
            "description": "Task one",
            "dependencies": [],
            "preserved": false,
            "original_id": null
        }
    ],
    "notes": "Simple plan"
}
```"#;

        let (subtasks, _) = replanner.parse_response(response).unwrap();
        assert_eq!(subtasks.len(), 1);
        assert_eq!(subtasks[0].description, "Task one");
    }

    // ReplanTrigger::should_replan tests

    #[test]
    fn test_should_replan_disabled() {
        let trace = PlanTrace::default();
        let mut config = ReplanConfig::default();
        config.enabled = false;
        let graph = DependencyGraph::new();

        assert!(ReplanTrigger::should_replan(&trace, &config, &graph).is_none());
    }

    #[test]
    fn test_should_replan_no_failures() {
        let trace = PlanTrace::default();
        let config = ReplanConfig::default();
        let graph = DependencyGraph::new();

        assert!(ReplanTrigger::should_replan(&trace, &config, &graph).is_none());
    }
}
