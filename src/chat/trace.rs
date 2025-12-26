//! Execution tracing structures for React and Plan modes.
//!
//! This module provides structures for tracking the execution of React mode loops
//! and Plan mode task execution, including iteration details, tool calls, subtask
//! traces, and timing information.

use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::planner::SubTaskStatus;

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
        }
    }

    /// Adds a tool call trace to this iteration.
    pub fn add_tool_call(&mut self, tool_call: ToolCallTrace) {
        self.tool_calls.push(tool_call);
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
        }
    }

    /// Marks the subtask as started.
    pub fn start(&mut self) {
        self.start_time = Some(Utc::now());
        self.status = SubTaskStatus::InProgress;
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
        format!(
            "SubtaskTrace[id={}, iterations={}, tokens={}, retries={}, duration={:?}, status={:?}]",
            self.subtask_id,
            self.react_iterations.len(),
            tokens.total_tokens,
            self.retry_count,
            self.duration,
            self.status
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
        }
    }
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
}
