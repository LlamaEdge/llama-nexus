//! Execution tracing structures for React and Plan modes.
//!
//! This module provides structures for tracking the execution of React mode loops
//! and Plan mode task execution, including iteration details, tool calls, subtask
//! traces, and timing information.

use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::planner::SubTaskStatus;
use crate::reflection::types::ReflectionResult;

/// Trace information for a complete React mode execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReactTrace {
    /// Unique identifier for this request.
    pub request_id: String,
    /// Optional conversation ID if memory is enabled.
    pub conversation_id: Option<String>,
    /// Trace information for each iteration.
    pub iterations: Vec<IterationTrace>,
    /// Total duration of the React loop.
    #[serde(with = "duration_serde")]
    pub total_duration: Duration,
    /// Final status of the React execution.
    pub final_status: TraceStatus,
}

impl ReactTrace {
    /// Creates a new ReactTrace with the given request and conversation IDs.
    pub fn new(request_id: String, conversation_id: Option<String>) -> Self {
        Self {
            request_id,
            conversation_id,
            iterations: Vec::new(),
            total_duration: Duration::ZERO,
            final_status: TraceStatus::Success,
        }
    }

    /// Adds an iteration trace to this React trace.
    pub fn add_iteration(&mut self, iteration: IterationTrace) {
        self.iterations.push(iteration);
    }

    /// Sets the total duration and final status.
    pub fn finalize(&mut self, total_duration: Duration, status: TraceStatus) {
        self.total_duration = total_duration;
        self.final_status = status;
    }

    /// Returns a summary of the trace for logging.
    pub fn summary(&self) -> String {
        let total_tool_calls: usize = self.iterations.iter().map(|i| i.tool_calls.len()).sum();
        let total_tokens: u64 = self
            .iterations
            .iter()
            .map(|i| i.llm_tokens.total_tokens)
            .sum();

        format!(
            "ReactTrace[request_id={}, iterations={}, tool_calls={}, tokens={}, duration={:?}, status={:?}]",
            self.request_id,
            self.iterations.len(),
            total_tool_calls,
            total_tokens,
            self.total_duration,
            self.final_status
        )
    }
}

/// Trace information for a single iteration of the React loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IterationTrace {
    /// Iteration number (1-indexed).
    pub iteration: u32,
    /// The thought extracted from LLM response (if any).
    pub thought: Option<String>,
    /// The action extracted from LLM response (if any).
    pub action: Option<String>,
    /// Tool calls made during this iteration.
    pub tool_calls: Vec<ToolCallTrace>,
    /// The observation/result after tool execution (if any).
    pub observation: Option<String>,
    /// Duration of this iteration.
    #[serde(with = "duration_serde")]
    pub duration: Duration,
    /// Token usage for LLM calls in this iteration.
    pub llm_tokens: TokenUsage,
    /// Skill requested via <use_skill> tag in this iteration (if any).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skill_requested: Option<String>,
    /// Whether the requested skill was successfully loaded.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub skill_loaded: bool,
}

impl IterationTrace {
    /// Creates a new IterationTrace for the given iteration number.
    pub fn new(iteration: u32) -> Self {
        Self {
            iteration,
            thought: None,
            action: None,
            tool_calls: Vec::new(),
            observation: None,
            duration: Duration::ZERO,
            llm_tokens: TokenUsage::default(),
            skill_requested: None,
            skill_loaded: false,
        }
    }

    /// Adds a tool call trace to this iteration.
    pub fn add_tool_call(&mut self, tool_call: ToolCallTrace) {
        self.tool_calls.push(tool_call);
    }

    /// Records a skill request in this iteration.
    pub fn set_skill_request(&mut self, skill_name: String, loaded: bool) {
        self.skill_requested = Some(skill_name);
        self.skill_loaded = loaded;
    }
}

/// Trace information for a single tool call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallTrace {
    /// Name of the tool being called.
    pub tool_name: String,
    /// Name of the MCP server hosting the tool.
    pub server_name: String,
    /// Arguments passed to the tool.
    pub arguments: serde_json::Value,
    /// Result of the tool call (if successful).
    pub result: Option<String>,
    /// Error message (if the tool call failed).
    pub error: Option<String>,
    /// Duration of the tool call.
    #[serde(with = "duration_serde")]
    pub duration: Duration,
}

impl ToolCallTrace {
    /// Creates a new ToolCallTrace.
    pub fn new(tool_name: String, server_name: String, arguments: serde_json::Value) -> Self {
        Self {
            tool_name,
            server_name,
            arguments,
            result: None,
            error: None,
            duration: Duration::ZERO,
        }
    }

    /// Sets the result for a successful tool call.
    pub fn set_result(&mut self, result: String, duration: Duration) {
        self.result = Some(result);
        self.duration = duration;
    }

    /// Sets the error for a failed tool call.
    pub fn set_error(&mut self, error: String, duration: Duration) {
        self.error = Some(error);
        self.duration = duration;
    }
}

/// Token usage information for LLM calls.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    /// Number of tokens in the prompt.
    pub prompt_tokens: u64,
    /// Number of tokens in the completion.
    pub completion_tokens: u64,
    /// Total number of tokens used.
    pub total_tokens: u64,
}

impl TokenUsage {
    /// Creates a new TokenUsage with the given values.
    pub fn new(prompt_tokens: u64, completion_tokens: u64) -> Self {
        Self {
            prompt_tokens,
            completion_tokens,
            total_tokens: prompt_tokens + completion_tokens,
        }
    }
}

/// Final status of a React execution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TraceStatus {
    /// Execution completed successfully with a final answer.
    Success,
    /// Execution stopped due to reaching maximum iterations.
    MaxIterationsExceeded,
    /// Execution stopped due to timeout.
    Timeout,
    /// Execution failed with an error.
    Error(String),
}

// ============================================================================
// Plan Mode Trace Structures
// ============================================================================

/// Trace information for a single subtask execution in Plan mode.
///
/// This structure captures the complete execution history of a subtask,
/// including React iterations if the subtask uses React mode for execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubtaskTrace {
    /// Subtask ID (corresponds to SubTask.id).
    pub subtask_id: usize,
    /// Description of the subtask.
    pub description: String,
    /// Final status of the subtask execution.
    pub status: SubTaskStatus,
    /// When the subtask started executing.
    #[serde(with = "option_datetime_serde")]
    pub start_time: Option<DateTime<Utc>>,
    /// When the subtask finished executing.
    #[serde(with = "option_datetime_serde")]
    pub end_time: Option<DateTime<Utc>>,
    /// Total duration of the subtask execution.
    #[serde(with = "option_duration_serde")]
    pub duration: Option<Duration>,
    /// React iteration traces (for React-mode subtask execution).
    pub react_iterations: Vec<IterationTrace>,
    /// Status of the React loop execution.
    pub react_status: TraceStatus,
    /// Result of the subtask execution (if successful).
    pub result: Option<String>,
    /// Number of retry attempts made for this subtask.
    pub retry_count: u32,
    /// History of retry attempts with their error messages.
    pub retry_history: Vec<RetryAttempt>,
    /// Active skills used during execution (supports multi-skill activation).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub active_skills: Vec<String>,
    /// Reflection evaluation result (if reflection is enabled).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reflection: Option<SubtaskReflectionSummary>,
}

/// Information about a single retry attempt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryAttempt {
    /// Retry attempt number (1-indexed).
    pub attempt: u32,
    /// Error that triggered this retry.
    pub error: String,
    /// When this retry attempt occurred.
    #[serde(with = "datetime_serde")]
    pub timestamp: DateTime<Utc>,
    /// React iterations from this attempt (preserved for debugging).
    pub iterations: Vec<IterationTrace>,
}

impl SubtaskTrace {
    /// Creates a new SubtaskTrace for the given subtask.
    pub fn new(subtask_id: usize, description: String) -> Self {
        Self {
            subtask_id,
            description,
            status: SubTaskStatus::Pending,
            start_time: None,
            end_time: None,
            duration: None,
            react_iterations: Vec::new(),
            react_status: TraceStatus::Success,
            result: None,
            retry_count: 0,
            retry_history: Vec::new(),
            active_skills: Vec::new(),
            reflection: None,
        }
    }

    /// Marks the subtask as started.
    pub fn start(&mut self) {
        self.start_time = Some(Utc::now());
        self.status = SubTaskStatus::InProgress;
    }

    /// Adds an active skill to this subtask (supports multi-skill activation).
    /// If the skill is already active, it will not be added again.
    #[allow(dead_code)]
    pub fn add_active_skill(&mut self, skill_name: String) {
        if !self.active_skills.contains(&skill_name) {
            self.active_skills.push(skill_name);
        }
    }

    /// Sets the active skills for this subtask (replaces all existing skills).
    pub fn set_active_skills(&mut self, skill_names: Vec<String>) {
        self.active_skills = skill_names;
    }

    /// Returns true if the subtask has any active skills.
    #[allow(dead_code)]
    pub fn has_active_skills(&self) -> bool {
        !self.active_skills.is_empty()
    }

    /// Returns the active skills as a slice.
    #[allow(dead_code)]
    pub fn get_active_skills(&self) -> &[String] {
        &self.active_skills
    }

    /// Sets the reflection summary for this subtask.
    pub fn set_reflection(&mut self, summary: SubtaskReflectionSummary) {
        self.reflection = Some(summary);
    }

    /// Returns the reflection summary if available.
    #[allow(dead_code)]
    pub fn get_reflection(&self) -> Option<&SubtaskReflectionSummary> {
        self.reflection.as_ref()
    }

    /// Marks the subtask as completed successfully.
    pub fn complete(&mut self, result: String) {
        let end_time = Utc::now();
        self.end_time = Some(end_time);
        if let Some(start) = self.start_time {
            self.duration = Some((end_time - start).to_std().unwrap_or_default());
        }
        self.status = SubTaskStatus::Completed;
        self.result = Some(result);
    }

    /// Marks the subtask as failed.
    pub fn fail(&mut self, error: String) {
        let end_time = Utc::now();
        self.end_time = Some(end_time);
        if let Some(start) = self.start_time {
            self.duration = Some((end_time - start).to_std().unwrap_or_default());
        }
        self.status = SubTaskStatus::Failed(error.clone());
        self.react_status = TraceStatus::Error(error);
    }

    /// Marks the subtask as timed out.
    pub fn timeout(&mut self) {
        let end_time = Utc::now();
        self.end_time = Some(end_time);
        if let Some(start) = self.start_time {
            self.duration = Some((end_time - start).to_std().unwrap_or_default());
        }
        self.status = SubTaskStatus::Failed("Timeout".to_string());
        self.react_status = TraceStatus::Timeout;
    }

    /// Adds a React iteration trace.
    pub fn add_iteration(&mut self, iteration: IterationTrace) {
        self.react_iterations.push(iteration);
    }

    /// Sets the React execution status.
    pub fn set_react_status(&mut self, status: TraceStatus) {
        self.react_status = status;
    }

    /// Calculates total token usage across all React iterations.
    pub fn total_tokens(&self) -> TokenUsage {
        let prompt_tokens: u64 = self
            .react_iterations
            .iter()
            .map(|i| i.llm_tokens.prompt_tokens)
            .sum();
        let completion_tokens: u64 = self
            .react_iterations
            .iter()
            .map(|i| i.llm_tokens.completion_tokens)
            .sum();
        TokenUsage::new(prompt_tokens, completion_tokens)
    }

    /// Returns a summary of the subtask trace for logging.
    pub fn summary(&self) -> String {
        let tokens = self.total_tokens();
        let skill_info = if self.active_skills.is_empty() {
            String::new()
        } else if self.active_skills.len() == 1 {
            format!(", skill={}", self.active_skills[0])
        } else {
            format!(", skills=[{}]", self.active_skills.join(", "))
        };
        format!(
            "SubtaskTrace[id={}, iterations={}, tokens={}, retries={}, duration={:?}, status={:?}{}]",
            self.subtask_id,
            self.react_iterations.len(),
            tokens.total_tokens,
            self.retry_count,
            self.duration,
            self.status,
            skill_info
        )
    }

    /// Records a retry attempt with the error message.
    /// Preserves the current React iterations in the retry history.
    pub fn record_retry(&mut self, error: String) {
        self.retry_count += 1;
        let attempt = RetryAttempt {
            attempt: self.retry_count,
            error,
            timestamp: Utc::now(),
            iterations: std::mem::take(&mut self.react_iterations),
        };
        self.retry_history.push(attempt);
        // Reset React status for the next attempt
        self.react_status = TraceStatus::Success;
    }

    /// Calculates total token usage across all React iterations including retry history.
    pub fn total_tokens_with_retries(&self) -> TokenUsage {
        let mut prompt_tokens: u64 = self
            .react_iterations
            .iter()
            .map(|i| i.llm_tokens.prompt_tokens)
            .sum();
        let mut completion_tokens: u64 = self
            .react_iterations
            .iter()
            .map(|i| i.llm_tokens.completion_tokens)
            .sum();

        // Add tokens from retry attempts
        for retry in &self.retry_history {
            prompt_tokens += retry
                .iterations
                .iter()
                .map(|i| i.llm_tokens.prompt_tokens)
                .sum::<u64>();
            completion_tokens += retry
                .iterations
                .iter()
                .map(|i| i.llm_tokens.completion_tokens)
                .sum::<u64>();
        }

        TokenUsage::new(prompt_tokens, completion_tokens)
    }
}

impl Default for SubtaskTrace {
    fn default() -> Self {
        Self {
            subtask_id: 0,
            description: String::new(),
            status: SubTaskStatus::Pending,
            start_time: None,
            end_time: None,
            duration: None,
            react_iterations: Vec::new(),
            react_status: TraceStatus::Success,
            result: None,
            retry_count: 0,
            retry_history: Vec::new(),
            active_skills: Vec::new(),
            reflection: None,
        }
    }
}

/// Trace information for a complete Plan mode execution.
///
/// This structure captures the full execution history of a plan,
/// including all subtask traces and aggregate statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanTrace {
    /// Unique identifier for this plan execution.
    pub plan_id: String,
    /// Original user goal.
    pub original_goal: String,
    /// Number of subtasks in the plan.
    pub subtask_count: usize,
    /// Execution order of subtasks.
    pub execution_order: Vec<usize>,
    /// Trace information for each subtask.
    pub subtask_traces: Vec<SubtaskTrace>,
    /// Total token usage across all subtasks.
    pub total_tokens: TokenUsage,
    /// Overall status of the plan execution.
    pub plan_status: TraceStatus,
    /// Total duration of the plan execution.
    #[serde(with = "duration_serde")]
    pub total_duration: Duration,
    /// When the plan started executing.
    #[serde(with = "option_datetime_serde")]
    pub start_time: Option<DateTime<Utc>>,
    /// When the plan finished executing.
    #[serde(with = "option_datetime_serde")]
    pub end_time: Option<DateTime<Utc>>,
    /// Plan-level reflection summary (computed from subtask reflections).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reflection_report: Option<PlanReflectionSummary>,
    /// History of replanning events that occurred during execution.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub replan_history: Vec<ReplanEvent>,
}

impl PlanTrace {
    /// Creates a new PlanTrace.
    pub fn new(plan_id: String, original_goal: String, execution_order: Vec<usize>) -> Self {
        let subtask_count = execution_order.len();
        Self {
            plan_id,
            original_goal,
            subtask_count,
            execution_order,
            subtask_traces: Vec::new(),
            total_tokens: TokenUsage::default(),
            plan_status: TraceStatus::Success,
            total_duration: Duration::ZERO,
            start_time: None,
            end_time: None,
            reflection_report: None,
            replan_history: Vec::new(),
        }
    }

    /// Marks the plan as started.
    pub fn start(&mut self) {
        self.start_time = Some(Utc::now());
    }

    /// Adds a subtask trace and accumulates token usage.
    /// Uses `total_tokens_with_retries()` to include tokens from all retry attempts.
    pub fn add_subtask_trace(&mut self, trace: SubtaskTrace) {
        let tokens = trace.total_tokens_with_retries();
        self.total_tokens.prompt_tokens += tokens.prompt_tokens;
        self.total_tokens.completion_tokens += tokens.completion_tokens;
        self.total_tokens.total_tokens += tokens.total_tokens;
        self.subtask_traces.push(trace);
    }

    /// Finalizes the plan trace with status and duration.
    pub fn finalize(&mut self, status: TraceStatus) {
        let end_time = Utc::now();
        self.end_time = Some(end_time);
        if let Some(start) = self.start_time {
            self.total_duration = (end_time - start).to_std().unwrap_or_default();
        }
        self.plan_status = status;
    }

    /// Returns the number of completed subtasks.
    pub fn completed_count(&self) -> usize {
        self.subtask_traces
            .iter()
            .filter(|t| matches!(t.status, SubTaskStatus::Completed))
            .count()
    }

    /// Returns the number of failed subtasks.
    pub fn failed_count(&self) -> usize {
        self.subtask_traces
            .iter()
            .filter(|t| matches!(t.status, SubTaskStatus::Failed(_)))
            .count()
    }

    /// Returns a summary of the plan trace for logging.
    pub fn summary(&self) -> String {
        format!(
            "PlanTrace[plan_id={}, subtasks={}/{} completed, failed={}, tokens={}, duration={:?}, status={:?}]",
            self.plan_id,
            self.completed_count(),
            self.subtask_count,
            self.failed_count(),
            self.total_tokens.total_tokens,
            self.total_duration,
            self.plan_status
        )
    }

    /// Computes the reflection summary from all subtask reflections.
    ///
    /// This should be called before finalizing the plan to populate
    /// the `reflection_report` field.
    pub fn compute_reflection_summary(&mut self) {
        let mut total_rounds = 0u32;
        let mut passed = 0usize;
        let mut failed = 0usize;
        let mut total_confidence = 0.0f64;
        let mut count = 0usize;

        for trace in &self.subtask_traces {
            if let Some(ref refl) = trace.reflection {
                total_rounds += refl.reflection_rounds;
                total_confidence += refl.confidence;
                count += 1;
                if refl.passed {
                    passed += 1;
                } else {
                    failed += 1;
                }
            }
        }

        if count > 0 {
            self.reflection_report = Some(PlanReflectionSummary {
                total_reflection_rounds: total_rounds,
                passed_subtasks: passed,
                failed_subtasks: failed,
                avg_confidence: total_confidence / count as f64,
                replan_count: self.replan_history.len() as u32,
            });
        }
    }

    /// Adds a replanning event to the history.
    pub fn add_replan_event(&mut self, event: ReplanEvent) {
        self.replan_history.push(event);
    }

    /// Returns the number of replanning events.
    #[allow(dead_code)]
    pub fn replan_count(&self) -> usize {
        self.replan_history.len()
    }
}

impl Default for PlanTrace {
    fn default() -> Self {
        Self {
            plan_id: String::new(),
            original_goal: String::new(),
            subtask_count: 0,
            execution_order: Vec::new(),
            subtask_traces: Vec::new(),
            total_tokens: TokenUsage::default(),
            plan_status: TraceStatus::Success,
            total_duration: Duration::ZERO,
            start_time: None,
            end_time: None,
            reflection_report: None,
            replan_history: Vec::new(),
        }
    }
}

// ============================================================================
// Reflection Trace Structures (R5.5)
// ============================================================================

/// Summary of subtask reflection for API responses.
///
/// This is a condensed version of `ReflectionResult` suitable for
/// inclusion in trace data and API responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubtaskReflectionSummary {
    /// Whether the reflection passed.
    pub passed: bool,
    /// Confidence score (0.0 to 1.0).
    pub confidence: f64,
    /// Number of issues found.
    pub issue_count: usize,
    /// Number of reflection rounds performed.
    pub reflection_rounds: u32,
    /// Recommended action as a string.
    pub recommended_action: String,
    /// Whether the result came from cache.
    pub from_cache: bool,
}

impl SubtaskReflectionSummary {
    /// Creates a summary from a reflection result.
    pub fn from_result(result: &ReflectionResult, from_cache: bool) -> Self {
        Self {
            passed: result.passed,
            confidence: result.confidence,
            issue_count: result.issues.len(),
            reflection_rounds: result.reflection_rounds,
            recommended_action: format!("{:?}", result.recommended_action),
            from_cache,
        }
    }
}

/// Summary of plan-level reflection for API responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanReflectionSummary {
    /// Total reflection rounds across all subtasks.
    pub total_reflection_rounds: u32,
    /// Number of subtasks that passed reflection.
    pub passed_subtasks: usize,
    /// Number of subtasks that failed reflection.
    pub failed_subtasks: usize,
    /// Average confidence across all reflections.
    pub avg_confidence: f64,
    /// Number of replanning events.
    pub replan_count: u32,
}

/// A replanning event that occurred during plan execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplanEvent {
    /// When the replanning occurred.
    #[serde(with = "datetime_serde")]
    pub timestamp: DateTime<Utc>,
    /// The trigger that caused replanning.
    pub trigger: String,
    /// Number of subtasks preserved from the original plan.
    pub preserved_count: usize,
    /// Number of new subtasks added.
    pub added_count: usize,
    /// Number of subtasks removed.
    pub removed_count: usize,
}

/// Custom serialization for Duration to make it JSON-friendly.
mod duration_serde {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    #[derive(Serialize, Deserialize)]
    struct DurationRepr {
        secs: u64,
        millis: u32,
    }

    pub fn serialize<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let repr = DurationRepr {
            secs: duration.as_secs(),
            millis: duration.subsec_millis(),
        };
        repr.serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        let repr = DurationRepr::deserialize(deserializer)?;
        Ok(Duration::new(repr.secs, repr.millis * 1_000_000))
    }
}

/// Custom serialization for Option<Duration> to make it JSON-friendly.
mod option_duration_serde {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    #[derive(Serialize, Deserialize)]
    struct DurationRepr {
        secs: u64,
        millis: u32,
    }

    pub fn serialize<S>(duration: &Option<Duration>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match duration {
            Some(d) => {
                let repr = DurationRepr {
                    secs: d.as_secs(),
                    millis: d.subsec_millis(),
                };
                Some(repr).serialize(serializer)
            }
            None => None::<DurationRepr>.serialize(serializer),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<Duration>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let repr: Option<DurationRepr> = Option::deserialize(deserializer)?;
        Ok(repr.map(|r| Duration::new(r.secs, r.millis * 1_000_000)))
    }
}

/// Custom serialization for Option<DateTime<Utc>> to make it JSON-friendly.
mod option_datetime_serde {
    use chrono::{DateTime, Utc};
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S>(datetime: &Option<DateTime<Utc>>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match datetime {
            Some(dt) => dt.to_rfc3339().serialize(serializer),
            None => None::<String>.serialize(serializer),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<DateTime<Utc>>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s: Option<String> = Option::deserialize(deserializer)?;
        match s {
            Some(s) => DateTime::parse_from_rfc3339(&s)
                .map(|dt| Some(dt.with_timezone(&Utc)))
                .map_err(serde::de::Error::custom),
            None => Ok(None),
        }
    }
}

/// Custom serialization for DateTime<Utc> to make it JSON-friendly.
mod datetime_serde {
    use chrono::{DateTime, Utc};
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S>(datetime: &DateTime<Utc>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        datetime.to_rfc3339().serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<DateTime<Utc>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s: String = String::deserialize(deserializer)?;
        DateTime::parse_from_rfc3339(&s)
            .map(|dt| dt.with_timezone(&Utc))
            .map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_react_trace_creation() {
        let trace = ReactTrace::new("req-123".to_string(), Some("conv-456".to_string()));
        assert_eq!(trace.request_id, "req-123");
        assert_eq!(trace.conversation_id, Some("conv-456".to_string()));
        assert!(trace.iterations.is_empty());
    }

    #[test]
    fn test_iteration_trace_creation() {
        let mut iter = IterationTrace::new(1);
        iter.thought = Some("I need to search for information".to_string());
        iter.action = Some("search".to_string());

        let tool_call = ToolCallTrace::new(
            "search".to_string(),
            "search-server".to_string(),
            serde_json::json!({"query": "test"}),
        );
        iter.add_tool_call(tool_call);

        assert_eq!(iter.iteration, 1);
        assert_eq!(iter.tool_calls.len(), 1);
    }

    #[test]
    fn test_trace_summary() {
        let mut trace = ReactTrace::new("req-123".to_string(), None);

        let mut iter1 = IterationTrace::new(1);
        iter1.llm_tokens = TokenUsage::new(100u64, 50u64);
        iter1.add_tool_call(ToolCallTrace::new(
            "tool1".to_string(),
            "server1".to_string(),
            serde_json::json!({}),
        ));
        trace.add_iteration(iter1);

        let mut iter2 = IterationTrace::new(2);
        iter2.llm_tokens = TokenUsage::new(150u64, 75u64);
        trace.add_iteration(iter2);

        trace.finalize(Duration::from_secs(5), TraceStatus::Success);

        let summary = trace.summary();
        assert!(summary.contains("iterations=2"));
        assert!(summary.contains("tool_calls=1"));
        assert!(summary.contains("tokens=375"));
    }

    // Plan mode trace tests

    #[test]
    fn test_subtask_trace_creation() {
        let trace = SubtaskTrace::new(1, "Test subtask".to_string());
        assert_eq!(trace.subtask_id, 1);
        assert_eq!(trace.description, "Test subtask");
        assert!(matches!(trace.status, SubTaskStatus::Pending));
        assert!(trace.react_iterations.is_empty());
    }

    #[test]
    fn test_subtask_trace_lifecycle() {
        let mut trace = SubtaskTrace::new(1, "Test subtask".to_string());

        // Start
        trace.start();
        assert!(matches!(trace.status, SubTaskStatus::InProgress));
        assert!(trace.start_time.is_some());

        // Add iterations
        let mut iter1 = IterationTrace::new(1);
        iter1.llm_tokens = TokenUsage::new(100, 50);
        trace.add_iteration(iter1);

        let mut iter2 = IterationTrace::new(2);
        iter2.llm_tokens = TokenUsage::new(80, 40);
        trace.add_iteration(iter2);

        // Complete
        trace.complete("Task completed successfully".to_string());
        assert!(matches!(trace.status, SubTaskStatus::Completed));
        assert!(trace.end_time.is_some());
        assert!(trace.duration.is_some());
        assert_eq!(
            trace.result,
            Some("Task completed successfully".to_string())
        );

        // Check token calculation
        let tokens = trace.total_tokens();
        assert_eq!(tokens.prompt_tokens, 180);
        assert_eq!(tokens.completion_tokens, 90);
        assert_eq!(tokens.total_tokens, 270);
    }

    #[test]
    fn test_subtask_trace_failure() {
        let mut trace = SubtaskTrace::new(1, "Test subtask".to_string());
        trace.start();
        trace.fail("Something went wrong".to_string());

        assert!(matches!(trace.status, SubTaskStatus::Failed(_)));
        assert!(matches!(trace.react_status, TraceStatus::Error(_)));
    }

    #[test]
    fn test_subtask_trace_timeout() {
        let mut trace = SubtaskTrace::new(1, "Test subtask".to_string());
        trace.start();
        trace.timeout();

        assert!(matches!(trace.status, SubTaskStatus::Failed(_)));
        assert!(matches!(trace.react_status, TraceStatus::Timeout));
    }

    #[test]
    fn test_plan_trace_creation() {
        let trace = PlanTrace::new(
            "plan-123".to_string(),
            "User goal".to_string(),
            vec![0, 1, 2],
        );
        assert_eq!(trace.plan_id, "plan-123");
        assert_eq!(trace.original_goal, "User goal");
        assert_eq!(trace.subtask_count, 3);
        assert_eq!(trace.execution_order, vec![0, 1, 2]);
        assert!(trace.subtask_traces.is_empty());
    }

    #[test]
    fn test_plan_trace_lifecycle() {
        let mut plan_trace =
            PlanTrace::new("plan-123".to_string(), "User goal".to_string(), vec![0, 1]);

        // Start
        plan_trace.start();
        assert!(plan_trace.start_time.is_some());

        // Add subtask traces
        let mut subtask1 = SubtaskTrace::new(0, "Subtask 1".to_string());
        subtask1.start();
        let mut iter = IterationTrace::new(1);
        iter.llm_tokens = TokenUsage::new(100, 50);
        subtask1.add_iteration(iter);
        subtask1.complete("Result 1".to_string());
        plan_trace.add_subtask_trace(subtask1);

        let mut subtask2 = SubtaskTrace::new(1, "Subtask 2".to_string());
        subtask2.start();
        let mut iter = IterationTrace::new(1);
        iter.llm_tokens = TokenUsage::new(80, 40);
        subtask2.add_iteration(iter);
        subtask2.complete("Result 2".to_string());
        plan_trace.add_subtask_trace(subtask2);

        // Finalize
        plan_trace.finalize(TraceStatus::Success);

        assert_eq!(plan_trace.completed_count(), 2);
        assert_eq!(plan_trace.failed_count(), 0);
        assert_eq!(plan_trace.total_tokens.prompt_tokens, 180);
        assert_eq!(plan_trace.total_tokens.completion_tokens, 90);
        assert!(plan_trace.end_time.is_some());
    }

    #[test]
    fn test_subtask_trace_serialization() {
        let mut trace = SubtaskTrace::new(1, "Test subtask".to_string());
        trace.start();

        let mut iter = IterationTrace::new(1);
        iter.llm_tokens = TokenUsage::new(100, 50);
        iter.thought = Some("Thinking...".to_string());
        trace.add_iteration(iter);

        trace.complete("Done".to_string());

        // Serialize
        let json = serde_json::to_string(&trace).expect("Failed to serialize");

        // Deserialize
        let deserialized: SubtaskTrace =
            serde_json::from_str(&json).expect("Failed to deserialize");

        assert_eq!(deserialized.subtask_id, 1);
        assert_eq!(deserialized.description, "Test subtask");
        assert!(matches!(deserialized.status, SubTaskStatus::Completed));
        assert_eq!(deserialized.react_iterations.len(), 1);
    }

    #[test]
    fn test_plan_trace_serialization() {
        let mut plan_trace =
            PlanTrace::new("plan-123".to_string(), "User goal".to_string(), vec![0]);
        plan_trace.start();

        let mut subtask = SubtaskTrace::new(0, "Subtask".to_string());
        subtask.start();
        subtask.complete("Result".to_string());
        plan_trace.add_subtask_trace(subtask);

        plan_trace.finalize(TraceStatus::Success);

        // Serialize
        let json = serde_json::to_string(&plan_trace).expect("Failed to serialize");

        // Deserialize
        let deserialized: PlanTrace = serde_json::from_str(&json).expect("Failed to deserialize");

        assert_eq!(deserialized.plan_id, "plan-123");
        assert_eq!(deserialized.subtask_count, 1);
        assert_eq!(deserialized.subtask_traces.len(), 1);
    }

    // ========================================================================
    // Subtask retry mechanism tests
    // ========================================================================

    #[test]
    fn test_subtask_trace_record_retry() {
        let mut trace = SubtaskTrace::new(1, "Test subtask".to_string());
        trace.start();

        // Add initial iteration
        let mut iter1 = IterationTrace::new(1);
        iter1.llm_tokens = TokenUsage::new(100, 50);
        trace.add_iteration(iter1);

        // Record first retry
        trace.record_retry("First error".to_string());

        assert_eq!(trace.retry_count, 1);
        assert_eq!(trace.retry_history.len(), 1);
        assert_eq!(trace.retry_history[0].attempt, 1);
        assert_eq!(trace.retry_history[0].error, "First error");
        assert_eq!(trace.retry_history[0].iterations.len(), 1);
        // Current iterations should be cleared
        assert!(trace.react_iterations.is_empty());
        // React status should be reset
        assert!(matches!(trace.react_status, TraceStatus::Success));
    }

    #[test]
    fn test_subtask_trace_multiple_retries() {
        let mut trace = SubtaskTrace::new(1, "Test subtask".to_string());
        trace.start();

        // First attempt
        let mut iter1 = IterationTrace::new(1);
        iter1.llm_tokens = TokenUsage::new(100, 50);
        trace.add_iteration(iter1);
        trace.record_retry("Error 1".to_string());

        // Second attempt
        let mut iter2 = IterationTrace::new(1);
        iter2.llm_tokens = TokenUsage::new(80, 40);
        trace.add_iteration(iter2);
        trace.record_retry("Error 2".to_string());

        // Third attempt (successful, no retry needed)
        let mut iter3 = IterationTrace::new(1);
        iter3.llm_tokens = TokenUsage::new(60, 30);
        trace.add_iteration(iter3);
        trace.complete("Finally done".to_string());

        assert_eq!(trace.retry_count, 2);
        assert_eq!(trace.retry_history.len(), 2);
        assert_eq!(trace.retry_history[0].attempt, 1);
        assert_eq!(trace.retry_history[1].attempt, 2);
        assert_eq!(trace.react_iterations.len(), 1);
    }

    #[test]
    fn test_subtask_trace_total_tokens_with_retries() {
        let mut trace = SubtaskTrace::new(1, "Test subtask".to_string());
        trace.start();

        // First attempt: 100 + 50 = 150 tokens
        let mut iter1 = IterationTrace::new(1);
        iter1.llm_tokens = TokenUsage::new(100, 50);
        trace.add_iteration(iter1);
        trace.record_retry("Error 1".to_string());

        // Second attempt: 80 + 40 = 120 tokens
        let mut iter2 = IterationTrace::new(1);
        iter2.llm_tokens = TokenUsage::new(80, 40);
        trace.add_iteration(iter2);
        trace.complete("Done".to_string());

        // Total should include both attempts
        let tokens = trace.total_tokens_with_retries();
        assert_eq!(tokens.prompt_tokens, 180); // 100 + 80
        assert_eq!(tokens.completion_tokens, 90); // 50 + 40
        assert_eq!(tokens.total_tokens, 270);
    }

    #[test]
    fn test_subtask_trace_total_tokens_no_retries() {
        let mut trace = SubtaskTrace::new(1, "Test subtask".to_string());
        trace.start();

        let mut iter = IterationTrace::new(1);
        iter.llm_tokens = TokenUsage::new(100, 50);
        trace.add_iteration(iter);
        trace.complete("Done".to_string());

        // Should be same as total_tokens when no retries
        let tokens_with_retries = trace.total_tokens_with_retries();
        let tokens = trace.total_tokens();

        assert_eq!(tokens_with_retries.prompt_tokens, tokens.prompt_tokens);
        assert_eq!(
            tokens_with_retries.completion_tokens,
            tokens.completion_tokens
        );
        assert_eq!(tokens_with_retries.total_tokens, tokens.total_tokens);
    }

    #[test]
    fn test_subtask_trace_summary_with_retries() {
        let mut trace = SubtaskTrace::new(1, "Test subtask".to_string());
        trace.start();

        let mut iter = IterationTrace::new(1);
        iter.llm_tokens = TokenUsage::new(100, 50);
        trace.add_iteration(iter);
        trace.record_retry("Error".to_string());

        let mut iter2 = IterationTrace::new(1);
        iter2.llm_tokens = TokenUsage::new(80, 40);
        trace.add_iteration(iter2);
        trace.complete("Done".to_string());

        let summary = trace.summary();
        assert!(summary.contains("retries=1"));
        assert!(summary.contains("id=1"));
    }

    #[test]
    fn test_plan_trace_add_subtask_with_retries() {
        let mut plan_trace =
            PlanTrace::new("plan-123".to_string(), "User goal".to_string(), vec![0]);
        plan_trace.start();

        // Create subtask with retries
        let mut subtask = SubtaskTrace::new(0, "Subtask".to_string());
        subtask.start();

        // First attempt
        let mut iter1 = IterationTrace::new(1);
        iter1.llm_tokens = TokenUsage::new(100, 50);
        subtask.add_iteration(iter1);
        subtask.record_retry("Error".to_string());

        // Second attempt
        let mut iter2 = IterationTrace::new(1);
        iter2.llm_tokens = TokenUsage::new(80, 40);
        subtask.add_iteration(iter2);
        subtask.complete("Result".to_string());

        // Add to plan - should count all tokens including retries
        plan_trace.add_subtask_trace(subtask);

        // Total should include tokens from retry history
        assert_eq!(plan_trace.total_tokens.prompt_tokens, 180); // 100 + 80
        assert_eq!(plan_trace.total_tokens.completion_tokens, 90); // 50 + 40
    }

    #[test]
    fn test_retry_attempt_serialization() {
        let mut trace = SubtaskTrace::new(1, "Test".to_string());
        trace.start();

        let mut iter = IterationTrace::new(1);
        iter.llm_tokens = TokenUsage::new(100, 50);
        trace.add_iteration(iter);
        trace.record_retry("Test error".to_string());

        // Serialize
        let json = serde_json::to_string(&trace).expect("Failed to serialize");

        // Deserialize
        let deserialized: SubtaskTrace =
            serde_json::from_str(&json).expect("Failed to deserialize");

        assert_eq!(deserialized.retry_count, 1);
        assert_eq!(deserialized.retry_history.len(), 1);
        assert_eq!(deserialized.retry_history[0].error, "Test error");
        assert_eq!(deserialized.retry_history[0].iterations.len(), 1);
    }

    #[test]
    fn test_plan_trace_summary_with_failures() {
        let mut plan_trace =
            PlanTrace::new("plan-123".to_string(), "User goal".to_string(), vec![0, 1]);
        plan_trace.start();

        // Add successful subtask
        let mut subtask1 = SubtaskTrace::new(0, "Subtask 1".to_string());
        subtask1.start();
        subtask1.complete("Result".to_string());
        plan_trace.add_subtask_trace(subtask1);

        // Add failed subtask
        let mut subtask2 = SubtaskTrace::new(1, "Subtask 2".to_string());
        subtask2.start();
        subtask2.fail("Error occurred".to_string());
        plan_trace.add_subtask_trace(subtask2);

        plan_trace.finalize(TraceStatus::Error("Partial failure".to_string()));

        let summary = plan_trace.summary();
        // Format: "subtasks=1/2 completed, failed=1"
        assert!(summary.contains("subtasks=1/2 completed"));
        assert!(summary.contains("failed=1"));
    }

    // ========================================================================
    // Multi-skill tracing tests
    // ========================================================================

    #[test]
    fn test_subtask_trace_add_single_skill() {
        let mut trace = SubtaskTrace::new(1, "Test subtask".to_string());
        assert!(trace.active_skills.is_empty());
        assert!(!trace.has_active_skills());

        trace.add_active_skill("git-workflow".to_string());
        assert_eq!(trace.active_skills.len(), 1);
        assert!(trace.has_active_skills());
        assert_eq!(trace.get_active_skills(), &["git-workflow"]);
    }

    #[test]
    fn test_subtask_trace_add_multiple_skills() {
        let mut trace = SubtaskTrace::new(1, "Test subtask".to_string());

        trace.add_active_skill("git-workflow".to_string());
        trace.add_active_skill("code-review".to_string());
        trace.add_active_skill("documentation".to_string());

        assert_eq!(trace.active_skills.len(), 3);
        assert_eq!(
            trace.get_active_skills(),
            &["git-workflow", "code-review", "documentation"]
        );
    }

    #[test]
    fn test_subtask_trace_add_duplicate_skill() {
        let mut trace = SubtaskTrace::new(1, "Test subtask".to_string());

        trace.add_active_skill("git-workflow".to_string());
        trace.add_active_skill("git-workflow".to_string()); // Duplicate
        trace.add_active_skill("code-review".to_string());
        trace.add_active_skill("git-workflow".to_string()); // Another duplicate

        // Should only have 2 unique skills
        assert_eq!(trace.active_skills.len(), 2);
        assert_eq!(trace.get_active_skills(), &["git-workflow", "code-review"]);
    }

    #[test]
    fn test_subtask_trace_set_active_skills() {
        let mut trace = SubtaskTrace::new(1, "Test subtask".to_string());

        trace.add_active_skill("initial-skill".to_string());
        assert_eq!(trace.active_skills.len(), 1);

        // Replace all skills
        trace.set_active_skills(vec![
            "skill-a".to_string(),
            "skill-b".to_string(),
            "skill-c".to_string(),
        ]);

        assert_eq!(trace.active_skills.len(), 3);
        assert_eq!(
            trace.get_active_skills(),
            &["skill-a", "skill-b", "skill-c"]
        );
    }

    #[test]
    fn test_subtask_trace_summary_no_skills() {
        let trace = SubtaskTrace::new(1, "Test subtask".to_string());
        let summary = trace.summary();

        // Should not contain skill info when no skills active
        assert!(!summary.contains("skill"));
        assert!(!summary.contains("skills"));
    }

    #[test]
    fn test_subtask_trace_summary_single_skill() {
        let mut trace = SubtaskTrace::new(1, "Test subtask".to_string());
        trace.add_active_skill("git-workflow".to_string());

        let summary = trace.summary();

        // Single skill uses singular format: "skill=name"
        assert!(summary.contains("skill=git-workflow"));
        assert!(!summary.contains("skills="));
    }

    #[test]
    fn test_subtask_trace_summary_multiple_skills() {
        let mut trace = SubtaskTrace::new(1, "Test subtask".to_string());
        trace.add_active_skill("git-workflow".to_string());
        trace.add_active_skill("code-review".to_string());

        let summary = trace.summary();

        // Multiple skills use plural format: "skills=[name1, name2]"
        assert!(summary.contains("skills=[git-workflow, code-review]"));
        assert!(!summary.contains(", skill="));
    }

    #[test]
    fn test_subtask_trace_multi_skill_serialization() {
        let mut trace = SubtaskTrace::new(1, "Multi-skill task".to_string());
        trace.start();
        trace.add_active_skill("skill-1".to_string());
        trace.add_active_skill("skill-2".to_string());
        trace.complete("Done".to_string());

        // Serialize
        let json = serde_json::to_string(&trace).expect("Failed to serialize");

        // Deserialize
        let deserialized: SubtaskTrace =
            serde_json::from_str(&json).expect("Failed to deserialize");

        assert_eq!(deserialized.active_skills.len(), 2);
        assert_eq!(deserialized.get_active_skills(), &["skill-1", "skill-2"]);
    }

    #[test]
    fn test_subtask_trace_empty_skills_not_serialized() {
        let trace = SubtaskTrace::new(1, "No skills task".to_string());

        // Serialize
        let json = serde_json::to_string(&trace).expect("Failed to serialize");

        // Empty active_skills should be skipped in serialization
        assert!(!json.contains("active_skills"));
    }

    #[test]
    fn test_subtask_trace_skills_serialized_when_present() {
        let mut trace = SubtaskTrace::new(1, "With skills task".to_string());
        trace.add_active_skill("test-skill".to_string());

        // Serialize
        let json = serde_json::to_string(&trace).expect("Failed to serialize");

        // Non-empty active_skills should be present in serialization
        assert!(json.contains("active_skills"));
        assert!(json.contains("test-skill"));
    }

    #[test]
    fn test_subtask_trace_default_has_empty_skills() {
        let trace = SubtaskTrace::default();
        assert!(trace.active_skills.is_empty());
        assert!(!trace.has_active_skills());
    }

    // ========================================================================
    // R5.5 Reflection Trace Tests
    // ========================================================================

    #[test]
    fn test_subtask_reflection_summary_creation() {
        use crate::reflection::types::{RecommendedAction, ReflectionResult};

        let result = ReflectionResult {
            passed: true,
            confidence: 0.92,
            issues: vec![],
            suggestions: vec!["Good job".to_string()],
            recommended_action: RecommendedAction::Accept,
            reflection_rounds: 1,
        };

        let summary = SubtaskReflectionSummary::from_result(&result, false);

        assert!(summary.passed);
        assert!((summary.confidence - 0.92).abs() < 0.001);
        assert_eq!(summary.issue_count, 0);
        assert_eq!(summary.reflection_rounds, 1);
        assert!(summary.recommended_action.contains("Accept"));
        assert!(!summary.from_cache);
    }

    #[test]
    fn test_subtask_reflection_summary_from_cache() {
        use crate::reflection::types::{RecommendedAction, ReflectionResult};

        let result = ReflectionResult {
            passed: true,
            confidence: 0.85,
            issues: vec![],
            suggestions: vec![],
            recommended_action: RecommendedAction::Accept,
            reflection_rounds: 2,
        };

        let summary = SubtaskReflectionSummary::from_result(&result, true);

        assert!(summary.from_cache);
        assert_eq!(summary.reflection_rounds, 2);
    }

    #[test]
    fn test_subtask_trace_set_reflection() {
        use crate::reflection::types::ReflectionResult;

        let mut trace = SubtaskTrace::new(1, "Test task".to_string());
        assert!(trace.reflection.is_none());
        assert!(trace.get_reflection().is_none());

        let result = ReflectionResult::passed();
        let summary = SubtaskReflectionSummary::from_result(&result, false);
        trace.set_reflection(summary);

        assert!(trace.reflection.is_some());
        assert!(trace.get_reflection().unwrap().passed);
    }

    #[test]
    fn test_subtask_trace_reflection_serialization() {
        use crate::reflection::types::{RecommendedAction, ReflectionResult};

        let mut trace = SubtaskTrace::new(1, "Task with reflection".to_string());
        trace.start();

        let result = ReflectionResult {
            passed: true,
            confidence: 0.9,
            issues: vec![],
            suggestions: vec![],
            recommended_action: RecommendedAction::Accept,
            reflection_rounds: 1,
        };
        trace.set_reflection(SubtaskReflectionSummary::from_result(&result, false));
        trace.complete("Done".to_string());

        // Serialize
        let json = serde_json::to_string(&trace).expect("Failed to serialize");
        assert!(json.contains("reflection"));
        assert!(json.contains("\"passed\":true"));
        assert!(json.contains("\"confidence\":0.9"));

        // Deserialize
        let deserialized: SubtaskTrace =
            serde_json::from_str(&json).expect("Failed to deserialize");
        assert!(deserialized.reflection.is_some());
        let refl = deserialized.reflection.unwrap();
        assert!(refl.passed);
        assert!((refl.confidence - 0.9).abs() < 0.001);
    }

    #[test]
    fn test_subtask_trace_no_reflection_not_serialized() {
        let trace = SubtaskTrace::new(1, "Task without reflection".to_string());

        let json = serde_json::to_string(&trace).expect("Failed to serialize");

        // reflection field should be skipped when None
        assert!(!json.contains("\"reflection\""));
    }

    #[test]
    fn test_plan_trace_compute_reflection_summary() {
        use crate::reflection::types::{RecommendedAction, ReflectionResult};

        let mut plan_trace = PlanTrace::new(
            "plan-123".to_string(),
            "Test goal".to_string(),
            vec![0, 1, 2],
        );
        plan_trace.start();

        // Add subtask 1 with passed reflection
        let mut subtask1 = SubtaskTrace::new(0, "Subtask 1".to_string());
        subtask1.start();
        let result1 = ReflectionResult {
            passed: true,
            confidence: 0.9,
            issues: vec![],
            suggestions: vec![],
            recommended_action: RecommendedAction::Accept,
            reflection_rounds: 1,
        };
        subtask1.set_reflection(SubtaskReflectionSummary::from_result(&result1, false));
        subtask1.complete("Result 1".to_string());
        plan_trace.add_subtask_trace(subtask1);

        // Add subtask 2 with failed reflection
        let mut subtask2 = SubtaskTrace::new(1, "Subtask 2".to_string());
        subtask2.start();
        let result2 = ReflectionResult {
            passed: false,
            confidence: 0.6,
            issues: vec![],
            suggestions: vec![],
            recommended_action: RecommendedAction::Retry,
            reflection_rounds: 2,
        };
        subtask2.set_reflection(SubtaskReflectionSummary::from_result(&result2, false));
        subtask2.complete("Result 2".to_string());
        plan_trace.add_subtask_trace(subtask2);

        // Add subtask 3 without reflection (reflection disabled)
        let mut subtask3 = SubtaskTrace::new(2, "Subtask 3".to_string());
        subtask3.start();
        subtask3.complete("Result 3".to_string());
        plan_trace.add_subtask_trace(subtask3);

        // Compute summary
        plan_trace.compute_reflection_summary();

        assert!(plan_trace.reflection_report.is_some());
        let report = plan_trace.reflection_report.unwrap();
        assert_eq!(report.passed_subtasks, 1);
        assert_eq!(report.failed_subtasks, 1);
        assert_eq!(report.total_reflection_rounds, 3); // 1 + 2
        assert!((report.avg_confidence - 0.75).abs() < 0.001); // (0.9 + 0.6) / 2
        assert_eq!(report.replan_count, 0);
    }

    #[test]
    fn test_plan_trace_add_replan_event() {
        let mut plan_trace =
            PlanTrace::new("plan-123".to_string(), "Test goal".to_string(), vec![0, 1]);

        assert!(plan_trace.replan_history.is_empty());
        assert_eq!(plan_trace.replan_count(), 0);

        // Add a replan event
        plan_trace.add_replan_event(ReplanEvent {
            timestamp: chrono::Utc::now(),
            trigger: "ConsecutiveFailures { count: 2, threshold: 2 }".to_string(),
            preserved_count: 1,
            added_count: 2,
            removed_count: 1,
        });

        assert_eq!(plan_trace.replan_history.len(), 1);
        assert_eq!(plan_trace.replan_count(), 1);

        let event = &plan_trace.replan_history[0];
        assert!(event.trigger.contains("ConsecutiveFailures"));
        assert_eq!(event.preserved_count, 1);
        assert_eq!(event.added_count, 2);
        assert_eq!(event.removed_count, 1);
    }

    #[test]
    fn test_plan_trace_reflection_with_replans() {
        use crate::reflection::types::ReflectionResult;

        let mut plan_trace =
            PlanTrace::new("plan-123".to_string(), "Test goal".to_string(), vec![0]);

        // Add subtask with reflection
        let mut subtask = SubtaskTrace::new(0, "Subtask".to_string());
        subtask.start();
        let result = ReflectionResult::passed();
        subtask.set_reflection(SubtaskReflectionSummary::from_result(&result, false));
        subtask.complete("Done".to_string());
        plan_trace.add_subtask_trace(subtask);

        // Add replan events
        plan_trace.add_replan_event(ReplanEvent {
            timestamp: chrono::Utc::now(),
            trigger: "Trigger 1".to_string(),
            preserved_count: 1,
            added_count: 1,
            removed_count: 0,
        });
        plan_trace.add_replan_event(ReplanEvent {
            timestamp: chrono::Utc::now(),
            trigger: "Trigger 2".to_string(),
            preserved_count: 2,
            added_count: 0,
            removed_count: 1,
        });

        // Compute summary
        plan_trace.compute_reflection_summary();

        let report = plan_trace.reflection_report.unwrap();
        assert_eq!(report.replan_count, 2);
    }

    #[test]
    fn test_plan_trace_empty_reflection_summary() {
        let mut plan_trace =
            PlanTrace::new("plan-123".to_string(), "Test goal".to_string(), vec![0]);

        // Add subtask without reflection
        let mut subtask = SubtaskTrace::new(0, "Subtask".to_string());
        subtask.start();
        subtask.complete("Done".to_string());
        plan_trace.add_subtask_trace(subtask);

        // Compute summary - should remain None since no reflections
        plan_trace.compute_reflection_summary();

        assert!(plan_trace.reflection_report.is_none());
    }

    #[test]
    fn test_replan_event_serialization() {
        let event = ReplanEvent {
            timestamp: chrono::Utc::now(),
            trigger: "TestTrigger".to_string(),
            preserved_count: 2,
            added_count: 3,
            removed_count: 1,
        };

        let json = serde_json::to_string(&event).expect("Failed to serialize");
        assert!(json.contains("timestamp"));
        assert!(json.contains("TestTrigger"));
        assert!(json.contains("preserved_count"));
        assert!(json.contains("added_count"));
        assert!(json.contains("removed_count"));

        let deserialized: ReplanEvent = serde_json::from_str(&json).expect("Failed to deserialize");
        assert_eq!(deserialized.trigger, "TestTrigger");
        assert_eq!(deserialized.preserved_count, 2);
        assert_eq!(deserialized.added_count, 3);
        assert_eq!(deserialized.removed_count, 1);
    }

    #[test]
    fn test_plan_trace_serialization_with_reflection() {
        use crate::reflection::types::{RecommendedAction, ReflectionResult};

        let mut plan_trace =
            PlanTrace::new("plan-123".to_string(), "Test goal".to_string(), vec![0]);
        plan_trace.start();

        // Add subtask with reflection
        let mut subtask = SubtaskTrace::new(0, "Subtask".to_string());
        subtask.start();
        let result = ReflectionResult {
            passed: true,
            confidence: 0.95,
            issues: vec![],
            suggestions: vec![],
            recommended_action: RecommendedAction::Accept,
            reflection_rounds: 1,
        };
        subtask.set_reflection(SubtaskReflectionSummary::from_result(&result, false));
        subtask.complete("Done".to_string());
        plan_trace.add_subtask_trace(subtask);

        // Add replan event
        plan_trace.add_replan_event(ReplanEvent {
            timestamp: chrono::Utc::now(),
            trigger: "Test".to_string(),
            preserved_count: 1,
            added_count: 0,
            removed_count: 0,
        });

        // Compute summary
        plan_trace.compute_reflection_summary();
        plan_trace.finalize(TraceStatus::Success);

        // Serialize
        let json = serde_json::to_string(&plan_trace).expect("Failed to serialize");
        assert!(json.contains("reflection_report"));
        assert!(json.contains("replan_history"));
        assert!(json.contains("passed_subtasks"));
        assert!(json.contains("avg_confidence"));

        // Deserialize
        let deserialized: PlanTrace = serde_json::from_str(&json).expect("Failed to deserialize");
        assert!(deserialized.reflection_report.is_some());
        assert_eq!(deserialized.replan_history.len(), 1);
    }

    #[test]
    fn test_plan_trace_empty_replan_not_serialized() {
        let plan_trace = PlanTrace::new("plan-123".to_string(), "Test goal".to_string(), vec![0]);

        let json = serde_json::to_string(&plan_trace).expect("Failed to serialize");

        // Empty replan_history should be skipped
        assert!(!json.contains("replan_history"));
    }
}
