//! Plan mode chat handler.
//!
//! This module implements the Plan mode, which decomposes user requests into
//! subtasks and executes them according to a dependency-aware execution order.

use std::{
    collections::HashSet,
    sync::Arc,
    time::{Duration, Instant, SystemTime},
};

use axum::{
    Json,
    body::Body,
    extract::{Extension, State},
    http::{HeaderMap, Response, StatusCode},
};
use endpoints::chat::{
    ChatCompletionChunk, ChatCompletionChunkChoice, ChatCompletionChunkChoiceDelta,
    ChatCompletionObject, ChatCompletionRequest, ChatCompletionRole,
};
use futures_util::stream::{self, StreamExt};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use rmcp::model::{CallToolRequestParam, RawContent};
use tokio_util::sync::CancellationToken;

use super::trace::ToolCallTrace;
use crate::{
    AppState,
    chat::{
        gen_chat_id,
        planner::{TaskPlan, TaskPlanner, ToolDescription},
        trace::{TokenUsage, TraceStatus},
        utils::*,
    },
    dual_debug, dual_error, dual_info, dual_warn,
    error::{ServerError, ServerResult},
    mcp::{MCP_SEPARATOR, MCP_SERVICES},
    server::{RoutingPolicy, ServerKind},
};

// ============================================================================
// Plan Mode Trace Structures
// ============================================================================

/// Trace information for a complete Plan mode execution.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PlanTrace {
    /// Unique identifier for this request.
    pub request_id: String,
    /// Optional conversation ID if memory is enabled.
    pub conversation_id: Option<String>,
    /// The generated task plan.
    pub plan: Option<TaskPlan>,
    /// Trace information for each subtask.
    pub subtask_traces: Vec<SubtaskTrace>,
    /// Total duration of the Plan execution.
    #[serde(with = "duration_serde")]
    pub total_duration: Duration,
    /// Final status of the Plan execution.
    pub final_status: TraceStatus,
    /// Total token usage across all LLM calls.
    pub total_tokens: TokenUsage,
}

impl PlanTrace {
    /// Creates a new PlanTrace with the given request and conversation IDs.
    pub fn new(request_id: String, conversation_id: Option<String>) -> Self {
        Self {
            request_id,
            conversation_id,
            plan: None,
            subtask_traces: Vec::new(),
            total_duration: Duration::ZERO,
            final_status: TraceStatus::Success,
            total_tokens: TokenUsage::default(),
        }
    }

    /// Sets the task plan.
    pub fn set_plan(&mut self, plan: TaskPlan) {
        self.plan = Some(plan);
    }

    /// Adds a subtask trace.
    pub fn add_subtask_trace(&mut self, trace: SubtaskTrace) {
        self.subtask_traces.push(trace);
    }

    /// Sets the total duration and final status.
    pub fn finalize(&mut self, total_duration: Duration, status: TraceStatus) {
        self.total_duration = total_duration;
        self.final_status = status;
    }

    /// Adds tokens to the total count.
    pub fn add_tokens(&mut self, tokens: TokenUsage) {
        self.total_tokens.prompt_tokens += tokens.prompt_tokens;
        self.total_tokens.completion_tokens += tokens.completion_tokens;
        self.total_tokens.total_tokens += tokens.total_tokens;
    }

    /// Returns a summary of the trace for logging.
    pub fn summary(&self) -> String {
        let completed = self
            .subtask_traces
            .iter()
            .filter(|t| t.status == SubtaskTraceStatus::Completed)
            .count();
        let failed = self
            .subtask_traces
            .iter()
            .filter(|t| matches!(t.status, SubtaskTraceStatus::Failed(_)))
            .count();

        format!(
            "PlanTrace[request_id={}, subtasks={}, completed={}, failed={}, tokens={}, duration={:?}, status={:?}]",
            self.request_id,
            self.subtask_traces.len(),
            completed,
            failed,
            self.total_tokens.total_tokens,
            self.total_duration,
            self.final_status
        )
    }
}

/// Trace information for a single subtask execution.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SubtaskTrace {
    /// Subtask ID.
    pub subtask_id: usize,
    /// Subtask description.
    pub description: String,
    /// Tool call made for this subtask (if any).
    pub tool_call: Option<ToolCallTrace>,
    /// Status of the subtask execution.
    pub status: SubtaskTraceStatus,
    /// Duration of subtask execution.
    #[serde(with = "duration_serde")]
    pub duration: Duration,
}

impl SubtaskTrace {
    /// Creates a new SubtaskTrace.
    pub fn new(subtask_id: usize, description: String) -> Self {
        Self {
            subtask_id,
            description,
            tool_call: None,
            status: SubtaskTraceStatus::Pending,
            duration: Duration::ZERO,
        }
    }
}

/// Status of a subtask in the trace.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum SubtaskTraceStatus {
    Pending,
    InProgress,
    Completed,
    Failed(String),
    Skipped,
}

/// Custom serialization for Duration.
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

// ============================================================================
// Plan Mode Handler
// ============================================================================

/// Main entry point for Plan mode chat handling.
pub(crate) async fn chat(
    State(state): State<Arc<AppState>>,
    Extension(cancel_token): Extension<CancellationToken>,
    headers: HeaderMap,
    Json(mut request): Json<ChatCompletionRequest>,
    conv_id: Option<String>,
    request_id: impl AsRef<str>,
) -> ServerResult<axum::response::Response> {
    let request_id = request_id.as_ref();

    // Get target server
    let chat_server = get_chat_server(&state, request_id).await?;

    // Extract user message for planning
    let user_message = extract_user_message(&request);

    // Extract system message for memory storage
    let system_message = extract_system_message(&request);

    // Store the latest user message to memory
    if let Some(memory) = &state.memory
        && let Some(conv_id) = &conv_id
        && let Some(user_msg) = &user_message
    {
        // Handle system message storage
        if let Some(sys_msg) = &system_message
            && let Ok(updated) = memory.set_system_message(conv_id, sys_msg).await
            && updated
        {
            dual_debug!(
                "System message updated for conversation {} - request_id: {}",
                conv_id,
                request_id
            );
        }

        // Store user message
        if let Err(e) = memory.add_user_message(conv_id, user_msg.clone()).await {
            dual_error!(
                "Failed to add user message to memory: {} - request_id: {}",
                e,
                request_id
            );
        }
    }

    // Get plan mode configuration
    let (max_plan_subtasks, plan_timeout_secs, subtask_max_retries) = {
        let config = state.config.read().await;
        (
            config.server.max_plan_subtasks,
            config.server.plan_timeout_secs,
            config.server.subtask_max_retries,
        )
    };
    let plan_timeout = Duration::from_secs(plan_timeout_secs);
    let start_time = Instant::now();

    // Initialize execution trace
    let mut trace = PlanTrace::new(request_id.to_string(), conv_id.clone());

    // Store original stream setting
    let stream = request.stream.unwrap_or(false);
    request.stream = Some(false);

    // ========================================================================
    // Phase 1: Task Planning
    // ========================================================================

    dual_info!("📋 Starting task planning - request_id: {}", request_id);

    let user_request = user_message.clone().unwrap_or_default();

    // Get available tools from MCP services
    let available_tools = get_available_tools().await;

    // Create task planner
    let planner = TaskPlanner::with_chat_llm(
        format!("{}/chat/completions", chat_server.url.trim_end_matches('/')),
        chat_server.api_key.clone(),
        max_plan_subtasks,
    )
    .with_tools(available_tools);

    // Generate task plan
    let mut plan = match planner.plan(&user_request).await {
        Ok(plan) => plan,
        Err(e) => {
            dual_error!(
                "Failed to generate task plan: {} - request_id: {}",
                e,
                request_id
            );
            trace.finalize(start_time.elapsed(), TraceStatus::Error(e.to_string()));
            return Err(e);
        }
    };

    dual_info!(
        "📋 Task plan generated: {} subtasks - request_id: {}",
        plan.len(),
        request_id
    );

    // Log the plan
    for (i, subtask) in plan.subtasks.iter().enumerate() {
        dual_debug!(
            "  Subtask {}: {} (deps: {:?}, tools: {:?})",
            subtask.id,
            subtask.description,
            subtask.dependencies,
            subtask.required_tools
        );
        if i < plan.execution_order.len() {
            dual_debug!("  Execution order[{}]: {}", i, plan.execution_order[i]);
        }
    }

    trace.set_plan(plan.clone());

    // ========================================================================
    // Phase 2: Task Execution
    // ========================================================================

    dual_info!("🚀 Starting task execution - request_id: {}", request_id);

    let mut completed_subtasks: HashSet<usize> = HashSet::new();
    let mut subtask_results: Vec<(usize, String)> = Vec::new();

    for &subtask_idx in &plan.execution_order {
        // Check timeout
        if start_time.elapsed() > plan_timeout {
            dual_warn!(
                "Plan execution timeout after {} seconds - request_id: {}",
                plan_timeout_secs,
                request_id
            );
            trace.finalize(start_time.elapsed(), TraceStatus::Timeout);
            dual_info!("Plan trace: {}", trace.summary());
            return Err(ServerError::PlanTimeout(plan_timeout_secs));
        }

        // Check cancellation
        if cancel_token.is_cancelled() {
            let warn_msg = "Request was cancelled by client";
            dual_warn!("{} - request_id: {}", warn_msg, request_id);
            trace.finalize(
                start_time.elapsed(),
                TraceStatus::Error(warn_msg.to_string()),
            );
            return Err(ServerError::Operation(warn_msg.to_string()));
        }

        let subtask = match plan.subtasks.get_mut(subtask_idx) {
            Some(s) => s,
            None => continue,
        };

        let subtask_start = Instant::now();
        let mut subtask_trace = SubtaskTrace::new(subtask.id, subtask.description.clone());

        dual_info!(
            "▶️ Executing subtask {}: {} - request_id: {}",
            subtask.id,
            subtask.description,
            request_id
        );

        // Check dependencies
        if !subtask.is_ready(&completed_subtasks) {
            dual_warn!(
                "Subtask {} has unmet dependencies, skipping - request_id: {}",
                subtask.id,
                request_id
            );
            subtask.skip();
            subtask_trace.status = SubtaskTraceStatus::Skipped;
            subtask_trace.duration = subtask_start.elapsed();
            trace.add_subtask_trace(subtask_trace);
            continue;
        }

        subtask.start();
        subtask_trace.status = SubtaskTraceStatus::InProgress;

        // Execute the subtask
        let result = execute_subtask(
            &state,
            &headers,
            subtask,
            &subtask_results,
            subtask_max_retries,
            request_id,
        )
        .await;

        match result {
            Ok((result_text, tool_trace)) => {
                dual_info!(
                    "✅ Subtask {} completed - request_id: {}",
                    subtask.id,
                    request_id
                );

                subtask.complete(result_text.clone());
                completed_subtasks.insert(subtask.id);
                subtask_results.push((subtask.id, result_text));

                subtask_trace.tool_call = tool_trace;
                subtask_trace.status = SubtaskTraceStatus::Completed;
            }
            Err(e) => {
                dual_warn!(
                    "❌ Subtask {} failed: {} - request_id: {}",
                    subtask.id,
                    e,
                    request_id
                );

                subtask.fail(e.to_string());
                subtask_trace.status = SubtaskTraceStatus::Failed(e.to_string());

                // Continue execution (don't fail the entire plan)
            }
        }

        subtask_trace.duration = subtask_start.elapsed();
        trace.add_subtask_trace(subtask_trace);
    }

    // ========================================================================
    // Phase 3: Result Aggregation
    // ========================================================================

    dual_info!("📊 Aggregating results - request_id: {}", request_id);

    let final_response = generate_final_response(
        &state,
        &chat_server,
        &headers,
        &request,
        &plan,
        &subtask_results,
        request_id,
    )
    .await?;

    // Update trace with tokens from final response
    trace.add_tokens(TokenUsage::new(
        final_response.usage.prompt_tokens,
        final_response.usage.completion_tokens,
    ));

    let final_content = final_response.choices[0]
        .message
        .content
        .clone()
        .unwrap_or_default();

    dual_info!("✅ Plan execution completed - request_id: {}", request_id);

    // Store assistant message to memory
    if let (Some(memory), Some(conv_id)) = (&state.memory, &conv_id)
        && let Err(e) = memory
            .add_assistant_message(conv_id, &final_content, vec![])
            .await
    {
        dual_error!(
            "Failed to add assistant message to memory: {} - request_id: {}",
            e,
            request_id
        );
    }

    // Finalize trace
    trace.finalize(start_time.elapsed(), TraceStatus::Success);
    dual_info!("Plan trace: {}", trace.summary());
    dual_debug!(
        "Plan trace details:\n{}",
        serde_json::to_string_pretty(&trace).unwrap_or_default()
    );

    // Build response
    build_response(final_response, &final_content, stream, request_id)
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Gets the chat server for making LLM requests.
async fn get_chat_server(
    state: &Arc<AppState>,
    request_id: &str,
) -> ServerResult<crate::server::TargetServerInfo> {
    let servers = state.server_group.read().await;
    let chat_servers = match servers.get(&ServerKind::chat) {
        Some(servers) => servers,
        None => {
            let err_msg = "No chat server available";
            dual_error!("{} - request_id: {}", err_msg, request_id);
            return Err(ServerError::Operation(err_msg.to_string()));
        }
    };

    match chat_servers.next().await {
        Ok(target_server_info) => Ok(target_server_info),
        Err(e) => {
            let err_msg = format!("Failed to get the chat server: {e}");
            dual_error!("{} - request_id: {}", err_msg, request_id);
            Err(ServerError::Operation(err_msg))
        }
    }
}

/// Gets available tools from MCP services.
async fn get_available_tools() -> Vec<ToolDescription> {
    let mut tools = Vec::new();

    if let Some(services) = MCP_SERVICES.get() {
        let service_map = services.read().await;
        for (server_name, service) in service_map.iter() {
            let service_read = service.read().await;
            // tools is Vec<McpToolName> (Vec<String>), just tool names
            for tool_name in &service_read.tools {
                tools.push(ToolDescription {
                    name: format!("{}{}{}", tool_name, MCP_SEPARATOR, server_name),
                    description: format!("Tool {} from {}", tool_name, server_name),
                });
            }
        }
    }

    tools
}

/// Executes a single subtask by calling the appropriate tool.
async fn execute_subtask(
    _state: &Arc<AppState>,
    _headers: &HeaderMap,
    subtask: &crate::chat::planner::SubTask,
    previous_results: &[(usize, String)],
    max_retries: u32,
    request_id: &str,
) -> ServerResult<(String, Option<ToolCallTrace>)> {
    // If no tools are required, return the description as the result
    if subtask.required_tools.is_empty() {
        dual_debug!(
            "Subtask {} has no required tools, using description as result - request_id: {}",
            subtask.id,
            request_id
        );
        return Ok((subtask.description.clone(), None));
    }

    // Get the first required tool
    let tool_name_with_server = &subtask.required_tools[0];

    // Parse tool name and server name
    let parts: Vec<&str> = tool_name_with_server.split(MCP_SEPARATOR).collect();
    if parts.len() != 2 {
        // Tool name doesn't contain server info, try to find it
        dual_warn!(
            "Tool '{}' doesn't have server info, attempting to find - request_id: {}",
            tool_name_with_server,
            request_id
        );
        return Ok((
            format!(
                "Tool '{}' not found or not properly configured",
                tool_name_with_server
            ),
            None,
        ));
    }

    let tool_name = parts[0];
    let server_name = parts[1];

    // Build tool arguments from subtask context
    let tool_args = build_tool_arguments(subtask, previous_results);

    dual_debug!(
        "Calling tool: {} on server: {} with args: {:?} - request_id: {}",
        tool_name,
        server_name,
        tool_args,
        request_id
    );

    // Start tool call trace
    let tool_call_start = Instant::now();
    let mut tool_trace = ToolCallTrace::new(
        tool_name.to_string(),
        server_name.to_string(),
        tool_args.clone(),
    );

    // Get MCP service and call tool with retry
    let services = match MCP_SERVICES.get() {
        Some(s) => s,
        None => {
            let err_msg = "MCP services not initialized";
            tool_trace.set_error(err_msg.to_string(), tool_call_start.elapsed());
            return Ok((err_msg.to_string(), Some(tool_trace)));
        }
    };

    let service_map = services.read().await;
    let service = match service_map.get(server_name) {
        Some(s) => s,
        None => {
            let err_msg = format!("MCP server '{}' not found", server_name);
            tool_trace.set_error(err_msg.clone(), tool_call_start.elapsed());
            return Ok((err_msg, Some(tool_trace)));
        }
    };

    // Retry loop
    let mut last_error: Option<String> = None;
    let retry_delay = Duration::from_millis(500);

    for attempt in 0..=max_retries {
        if attempt > 0 {
            dual_debug!(
                "Retrying tool call {} (attempt {}/{}) - request_id: {}",
                tool_name,
                attempt + 1,
                max_retries + 1,
                request_id
            );
            tokio::time::sleep(retry_delay).await;
        }

        let request_param = CallToolRequestParam {
            name: tool_name.to_string().into(),
            arguments: serde_json::from_value(tool_args.clone()).ok(),
        };

        match service.read().await.raw.call_tool(request_param).await {
            Ok(result) => match result.is_error {
                Some(false) | None => {
                    if !result.content.is_empty()
                        && let RawContent::Text(text) = &result.content[0].raw
                    {
                        let result_text = text.text.clone();
                        dual_info!(
                            "Tool call succeeded: {} - request_id: {}",
                            tool_name,
                            request_id
                        );
                        tool_trace.set_result(result_text.clone(), tool_call_start.elapsed());
                        return Ok((result_text, Some(tool_trace)));
                    }
                    let err_msg = "Tool returned empty content";
                    tool_trace.set_error(err_msg.to_string(), tool_call_start.elapsed());
                    return Ok((err_msg.to_string(), Some(tool_trace)));
                }
                Some(true) => {
                    last_error = Some("Tool returned error".to_string());
                }
            },
            Err(e) => {
                last_error = Some(e.to_string());
                dual_warn!(
                    "Tool call failed (attempt {}/{}): {} - request_id: {}",
                    attempt + 1,
                    max_retries + 1,
                    e,
                    request_id
                );
            }
        }
    }

    // All retries exhausted - return error
    let err_msg = last_error.unwrap_or_else(|| "Unknown error".to_string());
    tool_trace.set_error(err_msg.clone(), tool_call_start.elapsed());
    Err(ServerError::SubtaskRetryExhausted {
        subtask_id: subtask.id,
        attempts: max_retries + 1,
        message: err_msg,
    })
}

/// Builds tool arguments from subtask context.
fn build_tool_arguments(
    subtask: &crate::chat::planner::SubTask,
    previous_results: &[(usize, String)],
) -> serde_json::Value {
    // Build context from dependencies
    let context: Vec<String> = subtask
        .dependencies
        .iter()
        .filter_map(|dep_id| {
            previous_results
                .iter()
                .find(|(id, _)| id == dep_id)
                .map(|(_, result)| result.clone())
        })
        .collect();

    // Create arguments JSON
    serde_json::json!({
        "query": subtask.description,
        "context": context.join("\n"),
    })
}

/// Generates the final response by asking LLM to summarize results.
async fn generate_final_response(
    _state: &Arc<AppState>,
    chat_server: &crate::server::TargetServerInfo,
    headers: &HeaderMap,
    _original_request: &ChatCompletionRequest,
    plan: &TaskPlan,
    results: &[(usize, String)],
    request_id: &str,
) -> ServerResult<ChatCompletionObject> {
    // Build summary prompt
    let results_summary: String = results
        .iter()
        .map(|(id, result)| {
            let subtask = plan.subtasks.iter().find(|s| s.id == *id);
            let desc = subtask
                .map(|s| s.description.as_str())
                .unwrap_or("Unknown task");
            format!("- Task {}: {}\n  Result: {}", id, desc, result)
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    let summary_prompt = format!(
        "Based on the following task execution results, provide a comprehensive answer to the user's original request.\n\n\
        Original Goal: {}\n\n\
        Execution Results:\n{}\n\n\
        Please synthesize these results into a clear, coherent response.",
        plan.original_goal, results_summary
    );

    // Create summary request (build new request instead of cloning)
    let summary_messages = vec![endpoints::chat::ChatCompletionRequestMessage::User(
        endpoints::chat::ChatCompletionUserMessage::new(
            endpoints::chat::ChatCompletionUserMessageContent::Text(summary_prompt),
            None,
        ),
    )];

    let summary_request = serde_json::json!({
        "model": "default",
        "messages": summary_messages,
        "stream": false
    });

    // Build request
    let url = format!("{}/chat/completions", chat_server.url.trim_end_matches('/'));
    let mut client = reqwest::Client::new().post(&url);
    client = client.header(CONTENT_TYPE, "application/json");

    if let Some(api_key) = &chat_server.api_key {
        let auth = if api_key.starts_with("Bearer ") {
            api_key.clone()
        } else {
            format!("Bearer {}", api_key)
        };
        client = client.header(AUTHORIZATION, auth);
    } else if let Some(auth) = headers.get("authorization")
        && let Ok(auth_str) = auth.to_str()
    {
        client = client.header(AUTHORIZATION, auth_str);
    }

    dual_debug!(
        "Sending summary request to LLM - request_id: {}",
        request_id
    );

    // Send request
    let response =
        client.json(&summary_request).send().await.map_err(|e| {
            ServerError::Operation(format!("Failed to send summary request: {}", e))
        })?;

    let chat_completion: ChatCompletionObject = response
        .json()
        .await
        .map_err(|e| ServerError::Operation(format!("Failed to parse summary response: {}", e)))?;

    Ok(chat_completion)
}

/// Builds the final HTTP response.
fn build_response(
    mut chat_completion: ChatCompletionObject,
    final_content: &str,
    stream: bool,
    request_id: &str,
) -> ServerResult<Response<Body>> {
    if stream {
        // Create streaming response
        let chunks = gen_chunks_with_formatting(final_content, 10);
        let id = gen_chat_id();
        let model = chat_completion.model.clone();
        let usage = chat_completion.usage;
        let chunks_len = chunks.len();

        let request_id_owned = request_id.to_string();
        let stream = stream::iter(chunks.into_iter().enumerate().map(move |(i, chunk)| {
            let created = SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);

            let mut chat_completion_chunk = ChatCompletionChunk {
                id: id.clone(),
                object: "chat.completion.chunk".to_string(),
                created,
                model: model.clone(),
                system_fingerprint: "fp_plan_mode".to_string(),
                choices: vec![ChatCompletionChunkChoice {
                    index: i as u32,
                    delta: ChatCompletionChunkChoiceDelta {
                        role: ChatCompletionRole::Assistant,
                        content: Some(chunk),
                        tool_calls: vec![],
                    },
                    logprobs: None,
                    finish_reason: None,
                }],
                usage: None,
            };

            if i == chunks_len - 1 {
                chat_completion_chunk.choices[0].finish_reason =
                    Some(endpoints::common::FinishReason::stop);
                chat_completion_chunk.usage = Some(usage);
            }

            let json_str = serde_json::to_string(&chat_completion_chunk).unwrap();
            format!("data: {json_str}\n\n")
        }))
        .chain(stream::once(async { "data: [DONE]\n\n".to_string() }))
        .map(|s| Ok::<_, std::convert::Infallible>(s.into_bytes()));

        Response::builder()
            .header(CONTENT_TYPE, "text/event-stream")
            .header("Cache-Control", "no-cache")
            .header("Connection", "keep-alive")
            .status(StatusCode::OK)
            .body(Body::from_stream(stream))
            .map_err(|e| {
                dual_error!(
                    "Failed to create streaming response: {} - request_id: {}",
                    e,
                    request_id_owned
                );
                ServerError::Operation(format!("Failed to create streaming response: {}", e))
            })
    } else {
        // Non-streaming response
        chat_completion.choices[0].message.content = Some(final_content.to_string());
        let response_body = serde_json::to_string(&chat_completion)
            .map_err(|e| ServerError::Operation(format!("Failed to serialize response: {}", e)))?;

        Response::builder()
            .header(CONTENT_TYPE, "application/json")
            .status(StatusCode::OK)
            .body(Body::from(response_body))
            .map_err(|e| {
                dual_error!(
                    "Failed to create response: {} - request_id: {}",
                    e,
                    request_id
                );
                ServerError::Operation(format!("Failed to create response: {}", e))
            })
    }
}
