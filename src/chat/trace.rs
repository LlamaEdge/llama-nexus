//! React mode execution tracing structures.
//!
//! This module provides structures for tracking the execution of React mode loops,
//! including iteration details, tool calls, and timing information.

use std::time::Duration;

use serde::{Deserialize, Serialize};

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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
}
