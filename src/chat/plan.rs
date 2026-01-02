//! Plan mode chat handler.
//!
//! This module implements the Plan mode, which decomposes user requests into
//! subtasks and executes them according to a dependency-aware execution order.
//! Each subtask is executed using a React loop for iterative reasoning.

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
    ChatCompletionAssistantMessage, ChatCompletionChunk, ChatCompletionChunkChoice,
    ChatCompletionChunkChoiceDelta, ChatCompletionObject, ChatCompletionRequest,
    ChatCompletionRequestMessage, ChatCompletionRole, ChatCompletionSystemMessage,
    ChatCompletionToolMessage, ChatCompletionUserMessage, ChatCompletionUserMessageContent,
};
use futures_util::stream::{self, StreamExt};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use rmcp::model::{CallToolRequestParam, RawContent};
use tokio::select;
use tokio_util::sync::CancellationToken;

use super::{
    shared::TimeBudget,
    trace::{IterationTrace, ToolCallTrace},
    xml_parser::{extract_action, extract_final_answer, extract_thought, has_final_answer_tag},
};
use crate::{
    AppState,
    chat::{
        gen_chat_id,
        planner::{SubTask, TaskPlan, TaskPlanner, ToolDescription},
        trace::{PlanTrace, SubtaskTrace, TokenUsage, TraceStatus},
        utils::*,
    },
    dual_debug, dual_error, dual_info, dual_warn,
    error::{ServerError, ServerResult},
    mcp::{
        DEFAULT_SEARCH_FALLBACK_MESSAGE, MCP_SERVICES, SEARCH_MCP_SERVER_NAMES, extract_tool_name,
        format_mcp_tool_name, parse_mcp_tool_name,
    },
    server::{RoutingPolicy, ServerKind},
    skills::{LoadedSkill, SkillDetector, SkillInjector, SkillRegistry, SkillSummary},
};

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
    let (
        max_plan_subtasks,
        plan_timeout_secs,
        subtask_max_retries,
        subtask_react_max_iterations,
        subtask_react_timeout_secs,
        max_tools_per_iteration,
        tool_call_max_retries,
        tool_call_retry_delay_ms,
    ) = {
        let config = state.config.read().await;
        (
            config.server.max_plan_subtasks,
            config.server.plan_timeout_secs,
            config.server.subtask_max_retries,
            config.server.subtask_react_max_iterations,
            config.server.subtask_react_timeout_secs,
            config.server.max_tools_per_iteration,
            config.server.tool_call_max_retries,
            config.server.tool_call_retry_delay_ms,
        )
    };

    // Initialize time budget for the entire plan
    let time_budget = TimeBudget::new(plan_timeout_secs);

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

    // Get available Skills summaries from global registry
    let skills_summaries = match SkillRegistry::global() {
        Ok(registry) => {
            let summaries = registry.get_summaries().await;
            if !summaries.is_empty() {
                dual_info!(
                    "📚 Loaded {} skills for planning - request_id: {}",
                    summaries.len(),
                    request_id
                );
            }
            summaries
        }
        Err(_) => {
            // Skills registry not initialized, continue without skills
            dual_debug!("Skills registry not available - request_id: {}", request_id);
            vec![]
        }
    };

    // Create task planner
    let planner = TaskPlanner::with_chat_llm(
        format!("{}/chat/completions", chat_server.url.trim_end_matches('/')),
        chat_server.api_key.clone(),
        max_plan_subtasks,
    )
    .with_tools(available_tools.clone())
    .with_skills(skills_summaries.clone());

    // Generate task plan
    let mut plan = match planner.plan(&user_request).await {
        Ok(plan) => plan,
        Err(e) => {
            dual_error!(
                "Failed to generate task plan: {} - request_id: {}",
                e,
                request_id
            );
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
        let skill_info = subtask
            .recommended_skill
            .as_ref()
            .map(|s| format!(", skill: {}", s))
            .unwrap_or_default();
        dual_debug!(
            "  Subtask {}: {} (deps: {:?}, tools: {:?}{})",
            subtask.id,
            subtask.description,
            subtask.dependencies,
            subtask.required_tools,
            skill_info
        );
        if i < plan.execution_order.len() {
            dual_debug!("  Execution order[{}]: {}", i, plan.execution_order[i]);
        }
    }

    // Initialize execution trace
    let mut trace = PlanTrace::new(
        request_id.to_string(),
        plan.original_goal.clone(),
        plan.execution_order.clone(),
    );
    trace.start();

    // ========================================================================
    // Phase 2: Task Execution (React Loop per Subtask)
    // ========================================================================

    dual_info!("🚀 Starting task execution - request_id: {}", request_id);

    let mut completed_subtasks: HashSet<usize> = HashSet::new();
    let mut subtask_results: Vec<(usize, String)> = Vec::new();

    // Calculate pending subtask count for time allocation
    let mut pending_count = plan.execution_order.len();

    for &subtask_idx in &plan.execution_order {
        // Check time budget
        if time_budget.is_exhausted() {
            dual_warn!(
                "Plan time budget exhausted after {} seconds - request_id: {}",
                time_budget.elapsed().as_secs(),
                request_id
            );
            trace.finalize(TraceStatus::Timeout);
            dual_info!("Plan trace: {}", trace.summary());
            return Err(ServerError::TimeBudgetExhausted {
                elapsed_secs: time_budget.elapsed().as_secs(),
            });
        }

        // Check cancellation
        if cancel_token.is_cancelled() {
            let warn_msg = "Request was cancelled by client";
            dual_warn!("{} - request_id: {}", warn_msg, request_id);
            trace.finalize(TraceStatus::Error(warn_msg.to_string()));
            return Err(ServerError::Operation(warn_msg.to_string()));
        }

        let subtask = match plan.subtasks.get_mut(subtask_idx) {
            Some(s) => s,
            None => continue,
        };

        // Initialize subtask trace
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
            subtask_trace.status = crate::chat::planner::SubTaskStatus::Skipped;
            dual_info!("Subtask trace: {}", subtask_trace.summary());
            trace.add_subtask_trace(subtask_trace);
            pending_count = pending_count.saturating_sub(1);
            continue;
        }

        // Start subtask execution
        subtask.start();
        subtask_trace.start();

        // Allocate time budget for this subtask (including potential retries)
        let subtask_time_budget = time_budget.allocate(pending_count);
        // Ensure we don't exceed the configured subtask timeout
        let effective_timeout =
            subtask_time_budget.min(Duration::from_secs(subtask_react_timeout_secs));

        dual_debug!(
            "Allocated {:?} for subtask {} (pending: {}, max_retries: {}) - request_id: {}",
            effective_timeout,
            subtask.id,
            pending_count,
            subtask_max_retries,
            request_id
        );

        // Execute the subtask with retry loop
        let mut last_error: Option<ServerError> = None;
        let subtask_start_time = Instant::now();

        for attempt in 0..=subtask_max_retries {
            // Check if we've exceeded the total time budget for this subtask
            let elapsed = subtask_start_time.elapsed();
            if elapsed >= subtask_time_budget {
                dual_warn!(
                    "Subtask {} time budget exhausted after {:?} - request_id: {}",
                    subtask.id,
                    elapsed,
                    request_id
                );
                last_error = Some(ServerError::SubtaskTimeout {
                    subtask_id: subtask.id,
                    timeout_secs: subtask_time_budget.as_secs(),
                });
                break;
            }

            // Calculate remaining time for this attempt
            let remaining_time = subtask_time_budget.saturating_sub(elapsed);
            let attempt_timeout = remaining_time.min(effective_timeout);

            if attempt > 0 {
                dual_info!(
                    "🔄 Retrying subtask {} (attempt {}/{}) - request_id: {}",
                    subtask.id,
                    attempt + 1,
                    subtask_max_retries + 1,
                    request_id
                );
            }

            let result = execute_subtask_with_react(
                &state,
                &chat_server,
                &headers,
                subtask,
                &subtask_results,
                &available_tools,
                Some(&skills_summaries),
                attempt_timeout,
                subtask_react_max_iterations,
                max_tools_per_iteration,
                tool_call_max_retries,
                tool_call_retry_delay_ms,
                &cancel_token,
                request_id,
                &mut subtask_trace,
            )
            .await;

            match result {
                Ok(result_text) => {
                    dual_info!(
                        "✅ Subtask {} completed (attempt {}) - request_id: {}",
                        subtask.id,
                        attempt + 1,
                        request_id
                    );

                    subtask.complete(result_text.clone());
                    completed_subtasks.insert(subtask.id);
                    subtask_results.push((subtask.id, result_text.clone()));
                    subtask_trace.complete(result_text);
                    last_error = None;
                    break;
                }
                Err(e) => {
                    // Check if this error is retryable
                    if is_retryable_error(&e) && attempt < subtask_max_retries {
                        dual_warn!(
                            "⚠️ Subtask {} failed with retryable error: {} - request_id: {}",
                            subtask.id,
                            e,
                            request_id
                        );
                        subtask_trace.record_retry(e.to_string());
                        last_error = Some(e);
                        // Continue to next retry attempt
                    } else {
                        // Non-retryable error or max retries exceeded
                        dual_warn!(
                            "❌ Subtask {} failed: {} - request_id: {}",
                            subtask.id,
                            e,
                            request_id
                        );
                        last_error = Some(e);
                        break;
                    }
                }
            }
        }

        // Handle final result after retry loop
        if let Some(error) = last_error {
            // Check if we exhausted all retries
            if subtask_trace.retry_count >= subtask_max_retries && subtask_max_retries > 0 {
                let retry_exhausted_error = ServerError::SubtaskRetryExhausted {
                    subtask_id: subtask.id,
                    attempts: subtask_trace.retry_count + 1,
                    message: error.to_string(),
                };
                subtask.fail(retry_exhausted_error.to_string());
                subtask_trace.fail(retry_exhausted_error.to_string());
            } else {
                subtask.fail(error.to_string());
                subtask_trace.fail(error.to_string());
            }
            // Continue execution (don't fail the entire plan)
        }

        dual_info!("Subtask trace: {}", subtask_trace.summary());
        trace.add_subtask_trace(subtask_trace);
        pending_count = pending_count.saturating_sub(1);
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
    trace.finalize(TraceStatus::Success);
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
                    name: format_mcp_tool_name(server_name, tool_name),
                    description: format!("Tool {} from {}", tool_name, server_name),
                });
            }
        }
    }

    tools
}

/// Executes a single subtask using a React loop.
///
/// This function implements a React (Reason + Act) loop for executing subtasks,
/// allowing the LLM to iteratively think, call tools, observe results, and
/// produce a final answer.
///
/// Supports two-phase skill loading:
/// - Phase 1: Skills summaries are shown, LLM can request a skill via `<use_skill>` tag
/// - Phase 2: Full skill content is loaded and injected into context
#[allow(clippy::too_many_arguments)]
async fn execute_subtask_with_react(
    state: &Arc<AppState>,
    chat_server: &crate::server::TargetServerInfo,
    headers: &HeaderMap,
    subtask: &SubTask,
    previous_results: &[(usize, String)],
    available_tools: &[ToolDescription],
    skills_summaries: Option<&[SkillSummary]>,
    timeout: Duration,
    max_iterations: u32,
    max_tools_per_iteration: usize,
    tool_call_max_retries: u32,
    tool_call_retry_delay_ms: u64,
    cancel_token: &CancellationToken,
    request_id: &str,
    subtask_trace: &mut SubtaskTrace,
) -> ServerResult<String> {
    let start_time = Instant::now();
    let tool_call_retry_delay = Duration::from_millis(tool_call_retry_delay_ms);

    // Track the active skill for Phase 2
    let mut active_skill: Option<LoadedSkill> = None;

    // Build initial messages for React loop (Phase 1: no active skill yet)
    let mut messages = build_context_for_react(
        subtask,
        previous_results,
        available_tools,
        skills_summaries,
        None, // No active skill in initial context
    );

    // React loop
    let mut iteration_count: u32 = 0;

    loop {
        let iter_start = Instant::now();

        // Check iteration limit
        iteration_count += 1;
        if iteration_count > max_iterations {
            dual_warn!(
                "Subtask {} React loop exceeded maximum iterations ({}) - request_id: {}",
                subtask.id,
                max_iterations,
                request_id
            );
            subtask_trace.set_react_status(TraceStatus::MaxIterationsExceeded);
            return Err(ServerError::MaxIterationsExceeded(max_iterations));
        }

        // Check timeout
        if start_time.elapsed() > timeout {
            dual_warn!(
                "Subtask {} React loop timeout after {:?} - request_id: {}",
                subtask.id,
                start_time.elapsed(),
                request_id
            );
            subtask_trace.timeout();
            return Err(ServerError::SubtaskTimeout {
                subtask_id: subtask.id,
                timeout_secs: timeout.as_secs(),
            });
        }

        // Check cancellation
        if cancel_token.is_cancelled() {
            return Err(ServerError::Operation(
                "Request was cancelled by client".to_string(),
            ));
        }

        // Initialize iteration trace
        let mut iter_trace = IterationTrace::new(iteration_count);

        dual_debug!(
            "Subtask {} React iteration {}/{} - request_id: {}",
            subtask.id,
            iteration_count,
            max_iterations,
            request_id
        );

        // Build and send request to LLM
        let url = format!("{}/chat/completions", chat_server.url.trim_end_matches('/'));
        let mut client = reqwest::Client::new().post(&url);
        client = client.header(CONTENT_TYPE, "application/json");

        if let Some(api_key) = &chat_server.api_key
            && !api_key.is_empty()
        {
            let auth_info = if api_key.starts_with("Bearer ") {
                api_key.clone()
            } else {
                format!("Bearer {api_key}")
            };
            client = client.header(AUTHORIZATION, auth_info);
        } else if let Some(auth) = headers.get("authorization")
            && let Ok(auth_str) = auth.to_str()
        {
            client = client.header(AUTHORIZATION, auth_str);
        }

        // Build request with tools (filtered by active skill if any)
        let allowed_patterns = active_skill
            .as_ref()
            .map(|skill| skill.metadata.get_allowed_tools());
        let tools_json = build_tools_json(available_tools, allowed_patterns.as_deref());
        let request_json = serde_json::json!({
            "model": "default",
            "messages": messages,
            "tools": tools_json,
            "stream": false
        });

        // Send request with cancellation support
        let ds_response = select! {
            response = client.json(&request_json).send() => {
                response.map_err(|e| ServerError::Operation(format!("Failed to forward request: {e}")))
            }
            _ = cancel_token.cancelled() => {
                return Err(ServerError::Operation("Request was cancelled by client".to_string()));
            }
        }?;

        // Parse response
        let chat_completion: ChatCompletionObject = ds_response
            .json()
            .await
            .map_err(|e| ServerError::Operation(format!("Failed to parse response: {e}")))?;

        // Record token usage
        let usage = &chat_completion.usage;
        iter_trace.llm_tokens = TokenUsage::new(usage.prompt_tokens, usage.completion_tokens);

        // Check for tool calls
        let requires_tool_call = !chat_completion.choices[0].message.tool_calls.is_empty();

        if requires_tool_call {
            // Process tool calls
            let all_tool_calls = &chat_completion.choices[0].message.tool_calls;
            let tool_calls_to_execute = if all_tool_calls.len() > max_tools_per_iteration {
                &all_tool_calls[..max_tools_per_iteration]
            } else {
                all_tool_calls.as_slice()
            };

            // Extract thought from content if present
            if let Some(content) = chat_completion.choices[0].message.content.as_ref() {
                if let Some(thought) = extract_thought(content) {
                    dual_info!("💭 Subtask {} Thought: {}", subtask.id, thought);
                    iter_trace.thought = Some(thought);
                }
                if let Some(action) = extract_action(content) {
                    dual_info!("🔧 Subtask {} Action: {}", subtask.id, action);
                    iter_trace.action = Some(action);
                }
            }

            // Execute tool calls
            for tool_call in tool_calls_to_execute {
                let tool_result = execute_tool_call(
                    state,
                    tool_call,
                    tool_call_max_retries,
                    tool_call_retry_delay,
                    request_id,
                    &mut iter_trace,
                )
                .await?;

                // Format observation
                let observation = format!("<observation>{}</observation>", tool_result);
                iter_trace.observation = Some(tool_result.clone());

                // Append assistant message with tool call
                messages.push(ChatCompletionRequestMessage::Assistant(
                    ChatCompletionAssistantMessage::new(
                        chat_completion.choices[0].message.content.clone(),
                        None,
                        Some(vec![tool_call.clone()]),
                    ),
                ));

                // Append tool result message
                messages.push(ChatCompletionRequestMessage::Tool(
                    ChatCompletionToolMessage::new(&observation, &tool_call.id),
                ));
            }

            // Finalize iteration trace
            iter_trace.duration = iter_start.elapsed();
            subtask_trace.add_iteration(iter_trace);
        } else {
            // No tool calls - check for skill request or final answer
            if let Some(content) = chat_completion.choices[0].message.content.as_ref() {
                // Extract thought if present
                if let Some(thought) = extract_thought(content) {
                    dual_info!("💭 Subtask {} Thought: {}", subtask.id, thought);
                    iter_trace.thought = Some(thought);
                }

                // Check for skill request (Phase 1 -> Phase 2 transition)
                if active_skill.is_none()
                    && let Some(skill_name) = SkillDetector::detect_first(content)
                {
                    dual_info!(
                        "🎯 Subtask {} requested skill: {} - request_id: {}",
                        subtask.id,
                        skill_name,
                        request_id
                    );

                    // Try to load the requested skill
                    if let Ok(registry) = SkillRegistry::global() {
                        if let Some(loaded_skill) = registry.get(&skill_name).await {
                            dual_info!(
                                "📖 Loaded skill '{}' for subtask {} - request_id: {}",
                                skill_name,
                                subtask.id,
                                request_id
                            );

                            // Record skill request in iteration trace (successful load)
                            iter_trace.set_skill_request(skill_name.clone(), true);

                            // Record skill activation in subtask trace
                            subtask_trace.set_active_skill(skill_name.clone());

                            // Store the active skill
                            active_skill = Some(loaded_skill.clone());

                            // Rebuild context with the active skill (Phase 2)
                            messages = build_context_for_react(
                                subtask,
                                previous_results,
                                available_tools,
                                None, // No need for summaries in Phase 2
                                Some(&loaded_skill),
                            );

                            // Finalize iteration trace and continue loop
                            iter_trace.duration = iter_start.elapsed();
                            subtask_trace.add_iteration(iter_trace);
                            continue;
                        } else {
                            // Record skill request in iteration trace (failed to load)
                            iter_trace.set_skill_request(skill_name.clone(), false);

                            dual_warn!(
                                "⚠️ Skill '{}' not found, continuing without skill - request_id: {}",
                                skill_name,
                                request_id
                            );
                        }
                    }
                }

                // Check for final answer
                if has_final_answer_tag(content)
                    && let Some(final_answer) = extract_final_answer(content)
                {
                    dual_info!("✅ Subtask {} Final answer: {}", subtask.id, final_answer);

                    iter_trace.duration = iter_start.elapsed();
                    subtask_trace.add_iteration(iter_trace);
                    subtask_trace.set_react_status(TraceStatus::Success);

                    return Ok(final_answer);
                }

                // No final answer tag - treat content as the final answer
                dual_info!(
                    "✅ Subtask {} completed (no final_answer tag): {}",
                    subtask.id,
                    content
                );

                iter_trace.duration = iter_start.elapsed();
                subtask_trace.add_iteration(iter_trace);
                subtask_trace.set_react_status(TraceStatus::Success);

                return Ok(content.clone());
            }

            // No content in response - this shouldn't happen
            iter_trace.duration = iter_start.elapsed();
            subtask_trace.add_iteration(iter_trace);
            return Err(ServerError::Operation(
                "LLM returned empty response".to_string(),
            ));
        }
    }
}

/// Executes a single tool call with retry logic.
async fn execute_tool_call(
    _state: &Arc<AppState>,
    tool_call: &endpoints::chat::ToolCall,
    max_retries: u32,
    retry_delay: Duration,
    request_id: &str,
    iter_trace: &mut IterationTrace,
) -> ServerResult<String> {
    let tool_call_start = Instant::now();

    // Parse tool name and server name
    let (server_name, tool_name) =
        parse_mcp_tool_name(&tool_call.function.name).ok_or_else(|| {
            let err_msg = format!("Invalid tool name format: {}", tool_call.function.name);
            ServerError::Operation(err_msg)
        })?;
    let tool_args: serde_json::Value =
        serde_json::from_str(&tool_call.function.arguments).unwrap_or(serde_json::json!({}));

    // Initialize tool trace
    let mut tool_trace = ToolCallTrace::new(
        tool_name.to_string(),
        server_name.to_string(),
        tool_args.clone(),
    );

    // Get MCP service
    let services = MCP_SERVICES
        .get()
        .ok_or_else(|| ServerError::Operation("MCP services not initialized".to_string()))?;

    let service_map = services.read().await;
    let service = service_map.get(server_name).ok_or_else(|| {
        let err_msg = format!("MCP server '{}' not found", server_name);
        tool_trace.set_error(err_msg.clone(), tool_call_start.elapsed());
        iter_trace.add_tool_call(tool_trace.clone());
        ServerError::McpOperation(err_msg)
    })?;

    // Retry loop
    let mut last_error: Option<String> = None;

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
            Ok(result) => {
                if result.is_error == Some(true) {
                    last_error = Some("Tool returned error".to_string());
                    continue;
                }

                if !result.content.is_empty()
                    && let RawContent::Text(text) = &result.content[0].raw
                {
                    let result_text = text.text.clone();
                    dual_info!(
                        "Tool call succeeded: {} - request_id: {}",
                        tool_name,
                        request_id
                    );

                    // Check if this is a search server and wrap accordingly
                    let final_result = if SEARCH_MCP_SERVER_NAMES.contains(&server_name) {
                        // Get fallback message
                        let fallback = if service.read().await.has_fallback_message() {
                            service.read().await.fallback_message.clone().unwrap()
                        } else {
                            DEFAULT_SEARCH_FALLBACK_MESSAGE.to_string()
                        };

                        format!(
                            "Please answer the question based on the information between **---BEGIN CONTEXT---** and **---END CONTEXT---**. Do not use any external knowledge. If the information between **---BEGIN CONTEXT---** and **---END CONTEXT---** is empty, please respond with `{fallback}`. Note that DO NOT use any tools if provided.\n\n---BEGIN CONTEXT---\n\n{context}\n\n---END CONTEXT---",
                            fallback = fallback,
                            context = result_text,
                        )
                    } else {
                        result_text
                    };

                    tool_trace.set_result(final_result.clone(), tool_call_start.elapsed());
                    iter_trace.add_tool_call(tool_trace);
                    return Ok(final_result);
                }

                let err_msg = "Tool returned empty content";
                tool_trace.set_error(err_msg.to_string(), tool_call_start.elapsed());
                iter_trace.add_tool_call(tool_trace);
                return Err(ServerError::McpEmptyContent);
            }
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

    // All retries exhausted
    let err_msg = last_error.unwrap_or_else(|| "Unknown error".to_string());
    tool_trace.set_error(err_msg.clone(), tool_call_start.elapsed());
    iter_trace.add_tool_call(tool_trace);
    Err(ServerError::ToolCallRetryExhausted {
        tool_name: tool_name.to_string(),
        attempts: max_retries + 1,
        message: err_msg,
    })
}

/// Determines if an error is retryable for subtask execution.
///
/// Retryable errors include:
/// - Tool call failures (including retry exhausted)
/// - Maximum iterations exceeded
/// - Timeout errors
/// - MCP operation errors
///
/// Non-retryable errors include:
/// - Client cancellation
/// - Configuration errors
/// - Parse errors
fn is_retryable_error(error: &ServerError) -> bool {
    matches!(
        error,
        ServerError::ToolCallRetryExhausted { .. }
            | ServerError::MaxIterationsExceeded(_)
            | ServerError::SubtaskTimeout { .. }
            | ServerError::McpOperation(_)
            | ServerError::McpEmptyContent
            | ServerError::ReactTimeout(_)
    )
}

/// Builds the initial context messages for React loop execution.
///
/// This function supports two-phase skill loading:
/// - Phase 1 (no active_skill): Injects skills summaries, allows LLM to request a skill
/// - Phase 2 (with active_skill): Injects full skill content and filtered tools
fn build_context_for_react(
    subtask: &SubTask,
    previous_results: &[(usize, String)],
    available_tools: &[ToolDescription],
    skills_summaries: Option<&[SkillSummary]>,
    active_skill: Option<&LoadedSkill>,
) -> Vec<ChatCompletionRequestMessage> {
    let mut messages = Vec::new();

    // Build tools description
    let tools_desc = if available_tools.is_empty() {
        "No tools are currently available.".to_string()
    } else {
        available_tools
            .iter()
            .map(|t| format!("- {}: {}", t.name, t.description))
            .collect::<Vec<_>>()
            .join("\n")
    };

    // Build the system prompt based on whether we have an active skill
    let system_prompt = match active_skill {
        // Phase 2: Active skill - inject full skill content using SkillInjector
        Some(skill) => {
            let skill_section = SkillInjector::phase2_injection(skill);
            format!(
                r#"You are an AI assistant executing a specific subtask as part of a larger plan.

## Your Task
{}

{}

## Available Tools
{}

## Instructions
1. Follow the skill instructions above to complete the task
2. Use the available tools as needed
3. When you have completed the task, provide your final answer wrapped in <final_answer></final_answer> tags

## Response Format
- Use <thought></thought> tags to explain your reasoning
- Use <action></action> tags to specify the tool to use
- Use <action_input></action_input> tags for tool parameters (JSON format)
- When done, use <final_answer></final_answer> tags for your final response

Remember: Focus only on this specific subtask. Follow the skill instructions carefully."#,
                subtask.description, skill_section, tools_desc
            )
        }
        // Phase 1: No active skill - show skills summaries if available using SkillInjector
        None => {
            // Build skills section using SkillInjector
            let skills_section = match skills_summaries {
                Some(summaries) => SkillInjector::phase1_injection(summaries),
                None => String::new(),
            };

            format!(
                r#"You are an AI assistant executing a specific subtask as part of a larger plan.

## Your Task
{}
{}
## Available Tools
{}

## Instructions
1. Analyze the task and think about how to accomplish it
2. If a skill would help, request it using <use_skill>skill-name</use_skill> tags
3. Use the available tools as needed to complete the task
4. When you have completed the task, provide your final answer wrapped in <final_answer></final_answer> tags

## Response Format
- Use <thought></thought> tags to explain your reasoning
- Use <action></action> tags to specify the tool to use
- Use <action_input></action_input> tags for tool parameters (JSON format)
- When done, use <final_answer></final_answer> tags for your final response

Remember: Focus only on this specific subtask. Use the context from previous results if needed."#,
                subtask.description, skills_section, tools_desc
            )
        }
    };

    messages.push(ChatCompletionRequestMessage::System(
        ChatCompletionSystemMessage::new(system_prompt, None),
    ));

    // Add context from previous results if there are dependencies
    if !subtask.dependencies.is_empty() {
        let context_parts: Vec<String> = subtask
            .dependencies
            .iter()
            .filter_map(|dep_id| {
                previous_results
                    .iter()
                    .find(|(id, _)| id == dep_id)
                    .map(|(id, result)| format!("Result from subtask {}: {}", id, result))
            })
            .collect();

        if !context_parts.is_empty() {
            let context_message = format!(
                "Here are the results from previous subtasks that this task depends on:\n\n{}",
                context_parts.join("\n\n")
            );
            messages.push(ChatCompletionRequestMessage::User(
                ChatCompletionUserMessage::new(
                    ChatCompletionUserMessageContent::Text(context_message),
                    None,
                ),
            ));
        }
    }

    // Add the task prompt
    let task_prompt = format!(
        "Please complete the following task: {}",
        subtask.description
    );
    messages.push(ChatCompletionRequestMessage::User(
        ChatCompletionUserMessage::new(ChatCompletionUserMessageContent::Text(task_prompt), None),
    ));

    messages
}

/// Filters tools based on allowed patterns from a Skill's allowed-tools field.
///
/// Pattern formats supported:
/// - Exact match: "tool-name" matches "tool-name---server"
/// - Wildcard: "Bash(git:*)" matches "Bash(git:status)---server", "Bash(git:commit)---server"
/// - Simple wildcard: "Bash*" matches any tool starting with "Bash"
///
/// If allowed_patterns is None or empty, all tools are allowed.
fn filter_tools_by_patterns<'a>(
    tools: &'a [ToolDescription],
    allowed_patterns: Option<&[String]>,
) -> Vec<&'a ToolDescription> {
    match allowed_patterns {
        None | Some([]) => tools.iter().collect(),
        Some(patterns) => tools
            .iter()
            .filter(|tool| {
                // Extract the tool name part from MCP tool name
                let tool_name = extract_tool_name(&tool.name);

                patterns.iter().any(|pattern| {
                    if pattern.contains('*') {
                        // Wildcard pattern matching
                        match_wildcard_pattern(pattern, tool_name)
                    } else {
                        // Exact match (case-insensitive)
                        tool_name.eq_ignore_ascii_case(pattern)
                    }
                })
            })
            .collect(),
    }
}

/// Matches a tool name against a wildcard pattern.
///
/// Supports:
/// - "*" at the end: "Bash*" matches "Bash", "Bash(git:status)", etc.
/// - "*" within parentheses: "Bash(git:*)" matches "Bash(git:status)", "Bash(git:commit)"
fn match_wildcard_pattern(pattern: &str, tool_name: &str) -> bool {
    if pattern == "*" {
        return true;
    }

    // Handle "prefix*" pattern
    if pattern.ends_with('*') && !pattern.contains('(') {
        let prefix = &pattern[..pattern.len() - 1];
        return tool_name.starts_with(prefix);
    }

    // Handle "Name(prefix:*)" pattern - e.g., "Bash(git:*)"
    if let Some(star_pos) = pattern.find('*') {
        let pattern_prefix = &pattern[..star_pos];
        let pattern_suffix = &pattern[star_pos + 1..];

        // Tool name must start with pattern_prefix and end with pattern_suffix
        if tool_name.starts_with(pattern_prefix) && tool_name.ends_with(pattern_suffix) {
            return true;
        }
    }

    false
}

/// Builds the tools JSON for the LLM request.
///
/// If `allowed_patterns` is provided, only tools matching the patterns are included.
fn build_tools_json(
    available_tools: &[ToolDescription],
    allowed_patterns: Option<&[String]>,
) -> serde_json::Value {
    let filtered_tools = filter_tools_by_patterns(available_tools, allowed_patterns);

    let tools: Vec<serde_json::Value> = filtered_tools
        .iter()
        .map(|tool| {
            serde_json::json!({
                "type": "function",
                "function": {
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "query": {
                                "type": "string",
                                "description": "The query or input for the tool"
                            }
                        },
                        "required": ["query"]
                    }
                }
            })
        })
        .collect();

    serde_json::Value::Array(tools)
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

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::planner::{SubTask, SubTaskStatus};

    #[test]
    fn test_build_context_for_react_no_dependencies() {
        let subtask = SubTask::new(0, "Query weather".to_string());
        let previous_results: Vec<(usize, String)> = vec![];
        let available_tools = vec![ToolDescription {
            name: "weather---weather-server".to_string(),
            description: "Get weather information".to_string(),
        }];

        let messages =
            build_context_for_react(&subtask, &previous_results, &available_tools, None, None);

        // Should have system message + user message with task
        assert_eq!(messages.len(), 2);
    }

    #[test]
    fn test_build_context_for_react_with_dependencies() {
        let subtask =
            SubTask::new(2, "Summarize results".to_string()).with_dependencies(vec![0, 1]);
        let previous_results = vec![
            (0, "Beijing: Sunny".to_string()),
            (1, "Shanghai: Rainy".to_string()),
        ];
        let available_tools = vec![];

        let messages =
            build_context_for_react(&subtask, &previous_results, &available_tools, None, None);

        // Should have system message + context message + task message
        assert_eq!(messages.len(), 3);
    }

    #[test]
    fn test_build_context_for_react_with_skills_summaries() {
        let subtask = SubTask::new(0, "Query weather".to_string());
        let previous_results: Vec<(usize, String)> = vec![];
        let available_tools = vec![ToolDescription {
            name: "weather---weather-server".to_string(),
            description: "Get weather information".to_string(),
        }];
        let skills = vec![SkillSummary {
            name: "weather-query".to_string(),
            description: "Query weather for a city".to_string(),
        }];

        let messages = build_context_for_react(
            &subtask,
            &previous_results,
            &available_tools,
            Some(&skills),
            None,
        );

        // Should have system message + user message with task
        assert_eq!(messages.len(), 2);

        // Verify skills are mentioned in system prompt
        if let ChatCompletionRequestMessage::System(sys_msg) = &messages[0] {
            let content = sys_msg.content();
            assert!(content.contains("Available Skills"));
            assert!(content.contains("weather-query"));
            assert!(content.contains("<use_skill>"));
        } else {
            panic!("Expected system message");
        }
    }

    #[test]
    fn test_build_context_for_react_with_active_skill() {
        use std::path::PathBuf;

        use chrono::Utc;

        let subtask = SubTask::new(0, "Query weather".to_string());
        let previous_results: Vec<(usize, String)> = vec![];
        let available_tools = vec![ToolDescription {
            name: "weather---weather-server".to_string(),
            description: "Get weather information".to_string(),
        }];
        let active_skill = LoadedSkill {
            metadata: crate::skills::SkillMetadata {
                name: "weather-query".to_string(),
                description: "Query weather for a city".to_string(),
                license: None,
                compatibility: None,
                metadata: None,
                allowed_tools: None,
                model: None,
            },
            content: "Use the weather tool to query weather.".to_string(),
            raw_content: "".to_string(),
            skill_dir: PathBuf::new(),
            file_path: "".to_string(),
            enabled: true,
            loaded_at: Utc::now(),
        };

        let messages = build_context_for_react(
            &subtask,
            &previous_results,
            &available_tools,
            None,
            Some(&active_skill),
        );

        // Should have system message + user message with task
        assert_eq!(messages.len(), 2);

        // Verify skill content is injected in system prompt
        if let ChatCompletionRequestMessage::System(sys_msg) = &messages[0] {
            let content = sys_msg.content();
            assert!(content.contains("Active Skill: weather-query"));
            assert!(content.contains("Use the weather tool to query weather."));
        } else {
            panic!("Expected system message");
        }
    }

    #[test]
    fn test_build_tools_json() {
        let tools = vec![
            ToolDescription {
                name: "weather---weather-server".to_string(),
                description: "Get weather information".to_string(),
            },
            ToolDescription {
                name: "search---search-server".to_string(),
                description: "Search the web".to_string(),
            },
        ];

        let tools_json = build_tools_json(&tools, None);

        assert!(tools_json.is_array());
        assert_eq!(tools_json.as_array().unwrap().len(), 2);
        assert_eq!(
            tools_json[0]["function"]["name"],
            "weather---weather-server"
        );
        assert_eq!(tools_json[1]["function"]["name"], "search---search-server");
    }

    #[test]
    fn test_build_tools_json_with_filter() {
        let tools = vec![
            ToolDescription {
                name: "mcp__weather-server__weather".to_string(),
                description: "Get weather information".to_string(),
            },
            ToolDescription {
                name: "mcp__search-server__search".to_string(),
                description: "Search the web".to_string(),
            },
            ToolDescription {
                name: "mcp__mcp-server__Bash(git:status)".to_string(),
                description: "Git status".to_string(),
            },
        ];

        // Filter to only weather tool
        let patterns = vec!["weather".to_string()];
        let tools_json = build_tools_json(&tools, Some(&patterns));

        assert!(tools_json.is_array());
        assert_eq!(tools_json.as_array().unwrap().len(), 1);
        assert_eq!(
            tools_json[0]["function"]["name"],
            "mcp__weather-server__weather"
        );
    }

    #[test]
    fn test_filter_tools_by_patterns_wildcard() {
        let tools = vec![
            ToolDescription {
                name: "mcp__mcp-server__Bash(git:status)".to_string(),
                description: "Git status".to_string(),
            },
            ToolDescription {
                name: "mcp__mcp-server__Bash(git:commit)".to_string(),
                description: "Git commit".to_string(),
            },
            ToolDescription {
                name: "mcp__mcp-server__Bash(npm:install)".to_string(),
                description: "NPM install".to_string(),
            },
            ToolDescription {
                name: "mcp__weather-server__weather".to_string(),
                description: "Get weather".to_string(),
            },
        ];

        // Filter with wildcard pattern "Bash(git:*)"
        let patterns = vec!["Bash(git:*)".to_string()];
        let filtered = filter_tools_by_patterns(&tools, Some(&patterns));

        assert_eq!(filtered.len(), 2);
        assert!(
            filtered
                .iter()
                .any(|t| t.name == "mcp__mcp-server__Bash(git:status)")
        );
        assert!(
            filtered
                .iter()
                .any(|t| t.name == "mcp__mcp-server__Bash(git:commit)")
        );
    }

    #[test]
    fn test_filter_tools_by_patterns_prefix_wildcard() {
        let tools = vec![
            ToolDescription {
                name: "Bash(git:status)---mcp-server".to_string(),
                description: "Git status".to_string(),
            },
            ToolDescription {
                name: "BashScript---mcp-server".to_string(),
                description: "Bash script".to_string(),
            },
            ToolDescription {
                name: "weather---weather-server".to_string(),
                description: "Get weather".to_string(),
            },
        ];

        // Filter with prefix wildcard "Bash*"
        let patterns = vec!["Bash*".to_string()];
        let filtered = filter_tools_by_patterns(&tools, Some(&patterns));

        assert_eq!(filtered.len(), 2);
        assert!(
            filtered
                .iter()
                .any(|t| t.name == "Bash(git:status)---mcp-server")
        );
        assert!(filtered.iter().any(|t| t.name == "BashScript---mcp-server"));
    }

    #[test]
    fn test_filter_tools_empty_patterns() {
        let tools = vec![ToolDescription {
            name: "weather---weather-server".to_string(),
            description: "Get weather".to_string(),
        }];

        // Empty patterns should return all tools
        let patterns: Vec<String> = vec![];
        let filtered = filter_tools_by_patterns(&tools, Some(&patterns));

        assert_eq!(filtered.len(), 1);
    }

    #[test]
    fn test_subtask_trace_lifecycle() {
        let mut trace = SubtaskTrace::new(0, "Test task".to_string());

        assert_eq!(trace.subtask_id, 0);
        assert_eq!(trace.description, "Test task");
        assert!(matches!(trace.status, SubTaskStatus::Pending));
        assert!(trace.react_iterations.is_empty());

        // Start the trace
        trace.start();
        assert!(trace.start_time.is_some());

        // Add an iteration
        let iter_trace = IterationTrace::new(1);
        trace.add_iteration(iter_trace);
        assert_eq!(trace.react_iterations.len(), 1);

        // Complete the trace
        trace.complete("Result".to_string());
        assert!(matches!(trace.status, SubTaskStatus::Completed));
        assert_eq!(trace.result, Some("Result".to_string()));
        assert!(trace.end_time.is_some());
        assert!(trace.duration.is_some());
    }

    #[test]
    fn test_subtask_trace_failure() {
        let mut trace = SubtaskTrace::new(1, "Test task".to_string());
        trace.start();

        trace.fail("Error occurred".to_string());

        assert!(matches!(trace.status, SubTaskStatus::Failed(_)));
        if let SubTaskStatus::Failed(msg) = &trace.status {
            assert_eq!(msg, "Error occurred");
        }
    }

    #[test]
    fn test_plan_trace_lifecycle() {
        let mut trace = PlanTrace::new(
            "plan-123".to_string(),
            "Test goal".to_string(),
            vec![0, 1, 2],
        );

        assert_eq!(trace.plan_id, "plan-123");
        assert_eq!(trace.original_goal, "Test goal");
        assert_eq!(trace.execution_order, vec![0, 1, 2]);
        assert_eq!(trace.subtask_count, 3);

        // Start the trace
        trace.start();
        assert!(trace.start_time.is_some());

        // Add subtask traces
        let subtask_trace = SubtaskTrace::new(0, "Task 0".to_string());
        trace.add_subtask_trace(subtask_trace);
        assert_eq!(trace.subtask_traces.len(), 1);

        // Finalize
        trace.finalize(TraceStatus::Success);
        assert!(matches!(trace.plan_status, TraceStatus::Success));
        assert!(trace.end_time.is_some());
    }

    // ========================================================================
    // is_retryable_error tests
    // ========================================================================

    #[test]
    fn test_is_retryable_error_tool_call_retry_exhausted() {
        let error = ServerError::ToolCallRetryExhausted {
            tool_name: "test_tool".to_string(),
            attempts: 3,
            message: "Failed".to_string(),
        };
        assert!(is_retryable_error(&error));
    }

    #[test]
    fn test_is_retryable_error_max_iterations_exceeded() {
        let error = ServerError::MaxIterationsExceeded(10);
        assert!(is_retryable_error(&error));
    }

    #[test]
    fn test_is_retryable_error_subtask_timeout() {
        let error = ServerError::SubtaskTimeout {
            subtask_id: 1,
            timeout_secs: 60,
        };
        assert!(is_retryable_error(&error));
    }

    #[test]
    fn test_is_retryable_error_mcp_operation() {
        let error = ServerError::McpOperation("Connection failed".to_string());
        assert!(is_retryable_error(&error));
    }

    #[test]
    fn test_is_retryable_error_mcp_empty_content() {
        let error = ServerError::McpEmptyContent;
        assert!(is_retryable_error(&error));
    }

    #[test]
    fn test_is_retryable_error_react_timeout() {
        let error = ServerError::ReactTimeout(300);
        assert!(is_retryable_error(&error));
    }

    #[test]
    fn test_is_not_retryable_error_operation() {
        let error = ServerError::Operation("Client cancelled".to_string());
        assert!(!is_retryable_error(&error));
    }

    #[test]
    fn test_is_not_retryable_error_plan_parse_error() {
        let error = ServerError::PlanParseError("Invalid XML".to_string());
        assert!(!is_retryable_error(&error));
    }

    #[test]
    fn test_is_not_retryable_error_cyclic_dependency() {
        let error = ServerError::CyclicDependency;
        assert!(!is_retryable_error(&error));
    }

    #[test]
    fn test_is_not_retryable_error_empty_plan() {
        let error = ServerError::EmptyPlan;
        assert!(!is_retryable_error(&error));
    }

    // ========================================================================
    // build_context_for_react additional tests
    // ========================================================================

    #[test]
    fn test_build_context_for_react_partial_dependencies() {
        // Subtask depends on 0 and 1, but only 0 is available
        let subtask =
            SubTask::new(2, "Summarize results".to_string()).with_dependencies(vec![0, 1]);
        let previous_results = vec![(0, "Beijing: Sunny".to_string())];
        let available_tools = vec![];

        let messages =
            build_context_for_react(&subtask, &previous_results, &available_tools, None, None);

        // Should have system message + context message (with partial deps) + task message
        assert_eq!(messages.len(), 3);
    }

    #[test]
    fn test_build_context_for_react_system_message_contains_task() {
        let subtask = SubTask::new(0, "Query weather for Beijing".to_string());
        let previous_results: Vec<(usize, String)> = vec![];
        let available_tools = vec![ToolDescription {
            name: "weather---weather-server".to_string(),
            description: "Get weather information".to_string(),
        }];

        let messages =
            build_context_for_react(&subtask, &previous_results, &available_tools, None, None);

        // Check system message contains task description
        if let ChatCompletionRequestMessage::System(sys_msg) = &messages[0] {
            assert!(sys_msg.content().contains("Query weather for Beijing"));
            assert!(sys_msg.content().contains("weather---weather-server"));
        } else {
            panic!("First message should be system message");
        }
    }

    #[test]
    fn test_build_context_for_react_empty_tools() {
        let subtask = SubTask::new(0, "Simple task".to_string());
        let previous_results: Vec<(usize, String)> = vec![];
        let available_tools: Vec<ToolDescription> = vec![];

        let messages =
            build_context_for_react(&subtask, &previous_results, &available_tools, None, None);

        assert_eq!(messages.len(), 2);
        // System message should still exist even without tools
        assert!(matches!(
            &messages[0],
            ChatCompletionRequestMessage::System(_)
        ));
    }

    // ========================================================================
    // build_tools_json additional tests
    // ========================================================================

    #[test]
    fn test_build_tools_json_empty() {
        let tools: Vec<ToolDescription> = vec![];
        let tools_json = build_tools_json(&tools, None);

        assert!(tools_json.is_array());
        assert_eq!(tools_json.as_array().unwrap().len(), 0);
    }

    #[test]
    fn test_build_tools_json_structure() {
        let tools = vec![ToolDescription {
            name: "test-tool---test-server".to_string(),
            description: "A test tool".to_string(),
        }];

        let tools_json = build_tools_json(&tools, None);

        let tool = &tools_json[0];
        assert_eq!(tool["type"], "function");
        assert_eq!(tool["function"]["name"], "test-tool---test-server");
        assert_eq!(tool["function"]["description"], "A test tool");
        assert!(tool["function"]["parameters"]["properties"]["query"].is_object());
        assert_eq!(tool["function"]["parameters"]["required"][0], "query");
    }

    #[test]
    fn test_match_wildcard_pattern() {
        // Universal wildcard
        assert!(match_wildcard_pattern("*", "anything"));

        // Prefix wildcard
        assert!(match_wildcard_pattern("Bash*", "Bash"));
        assert!(match_wildcard_pattern("Bash*", "Bash(git:status)"));
        assert!(match_wildcard_pattern("Bash*", "BashScript"));
        assert!(!match_wildcard_pattern("Bash*", "NotBash"));

        // Pattern with wildcard in parentheses
        assert!(match_wildcard_pattern("Bash(git:*)", "Bash(git:status)"));
        assert!(match_wildcard_pattern("Bash(git:*)", "Bash(git:commit)"));
        assert!(!match_wildcard_pattern("Bash(git:*)", "Bash(npm:install)"));
        assert!(!match_wildcard_pattern("Bash(git:*)", "Bash"));
    }

    // ==========================================================================
    // Integration Tests: Planning Phase + Skills
    // ==========================================================================

    /// Test that skills summaries are properly injected into the planning context
    #[test]
    fn test_integration_planning_with_skills_summaries() {
        let subtask = SubTask::new(0, "Search for information".to_string());
        let previous_results: Vec<(usize, String)> = vec![];
        let tools = vec![
            ToolDescription {
                name: "search---search-server".to_string(),
                description: "Search the web".to_string(),
            },
            ToolDescription {
                name: "Bash(git:status)---mcp-server".to_string(),
                description: "Git status".to_string(),
            },
        ];
        let skills = vec![
            SkillSummary {
                name: "web-search".to_string(),
                description: "Perform web searches with advanced filtering".to_string(),
            },
            SkillSummary {
                name: "git-workflow".to_string(),
                description: "Help with git operations and workflows".to_string(),
            },
        ];

        let messages =
            build_context_for_react(&subtask, &previous_results, &tools, Some(&skills), None);

        // Verify system message contains skills information
        if let ChatCompletionRequestMessage::System(sys_msg) = &messages[0] {
            let content = sys_msg.content();
            // Should contain skills section
            assert!(content.contains("Available Skills"));
            // Should list both skills
            assert!(content.contains("web-search"));
            assert!(content.contains("git-workflow"));
            // Should contain usage instruction
            assert!(content.contains("<use_skill>"));
            // Should contain skill descriptions
            assert!(content.contains("Perform web searches"));
            assert!(content.contains("git operations"));
        } else {
            panic!("Expected system message as first message");
        }
    }

    /// Test that empty skills list doesn't add skills table section
    #[test]
    fn test_integration_planning_without_skills() {
        let subtask = SubTask::new(0, "Simple task".to_string());
        let previous_results: Vec<(usize, String)> = vec![];
        let tools = vec![ToolDescription {
            name: "tool---server".to_string(),
            description: "A tool".to_string(),
        }];
        let empty_skills: Vec<SkillSummary> = vec![];

        let messages = build_context_for_react(
            &subtask,
            &previous_results,
            &tools,
            Some(&empty_skills),
            None,
        );

        // Verify system message does NOT contain skills table section
        // Note: The base template still contains <use_skill> instruction,
        // but no "Available Skills" table should be present
        if let ChatCompletionRequestMessage::System(sys_msg) = &messages[0] {
            let content = sys_msg.content();
            // Should NOT have the skills table
            assert!(!content.contains("Available Skills"));
            // Should NOT list any skill names
            assert!(!content.contains("| Name |"));
        } else {
            panic!("Expected system message");
        }
    }

    // ==========================================================================
    // Integration Tests: Two-Phase Skill Loading
    // ==========================================================================

    /// Test phase 1: Skills summaries injection without active skill
    #[test]
    fn test_integration_two_phase_loading_phase1() {
        let subtask = SubTask::new(0, "Task requiring skill".to_string());
        let previous_results: Vec<(usize, String)> = vec![];
        let tools = vec![
            ToolDescription {
                name: "Bash(git:status)---mcp".to_string(),
                description: "Git status".to_string(),
            },
            ToolDescription {
                name: "Bash(git:commit)---mcp".to_string(),
                description: "Git commit".to_string(),
            },
            ToolDescription {
                name: "Read---mcp".to_string(),
                description: "Read file".to_string(),
            },
        ];
        let skills = vec![SkillSummary {
            name: "git-workflow".to_string(),
            description: "Git workflow assistance".to_string(),
        }];

        // Phase 1: no active skill
        let messages =
            build_context_for_react(&subtask, &previous_results, &tools, Some(&skills), None);

        if let ChatCompletionRequestMessage::System(sys_msg) = &messages[0] {
            let content = sys_msg.content();
            // Should show available skills
            assert!(content.contains("Available Skills"));
            assert!(content.contains("git-workflow"));
            // Should NOT show active skill section
            assert!(!content.contains("Active Skill:"));
        } else {
            panic!("Expected system message");
        }
    }

    /// Test phase 2: Active skill injection with full content
    #[test]
    fn test_integration_two_phase_loading_phase2() {
        use std::path::PathBuf;

        use chrono::Utc;

        let subtask = SubTask::new(0, "Git commit task".to_string());
        let previous_results: Vec<(usize, String)> = vec![];
        let tools = vec![
            ToolDescription {
                name: "Bash(git:status)---mcp".to_string(),
                description: "Git status".to_string(),
            },
            ToolDescription {
                name: "Bash(git:commit)---mcp".to_string(),
                description: "Git commit".to_string(),
            },
            ToolDescription {
                name: "Read---mcp".to_string(),
                description: "Read file".to_string(),
            },
        ];

        let active_skill = LoadedSkill {
            metadata: crate::skills::SkillMetadata {
                name: "git-workflow".to_string(),
                description: "Git workflow assistance".to_string(),
                license: Some("MIT".to_string()),
                compatibility: Some("Requires git".to_string()),
                metadata: None,
                allowed_tools: Some("Bash(git:*) Read".to_string()),
                model: None,
            },
            content: r#"# Git Workflow

## Commit Guidelines
1. Use descriptive commit messages
2. Keep commits atomic

## Example
```bash
git add .
git commit -m "feat: add new feature"
```
"#
            .to_string(),
            raw_content: String::new(),
            skill_dir: PathBuf::new(),
            file_path: String::new(),
            enabled: true,
            loaded_at: Utc::now(),
        };

        // Phase 2: with active skill
        let messages = build_context_for_react(
            &subtask,
            &previous_results,
            &tools,
            None, // No summaries needed in phase 2
            Some(&active_skill),
        );

        if let ChatCompletionRequestMessage::System(sys_msg) = &messages[0] {
            let content = sys_msg.content();
            // Should show active skill section
            assert!(content.contains("Active Skill: git-workflow"));
            // Should contain skill content
            assert!(content.contains("Git Workflow"));
            assert!(content.contains("Commit Guidelines"));
            assert!(content.contains("Keep commits atomic"));
            // Should NOT show "Available Skills" section
            assert!(!content.contains("Available Skills"));
        } else {
            panic!("Expected system message");
        }
    }

    // ==========================================================================
    // Integration Tests: Tool Filtering with Skills
    // ==========================================================================

    /// Test tool filtering with skill's allowed-tools
    #[test]
    fn test_integration_tool_filtering_with_skill() {
        use std::path::PathBuf;

        use chrono::Utc;

        let tools = vec![
            ToolDescription {
                name: "mcp__mcp__Bash(git:status)".to_string(),
                description: "Git status".to_string(),
            },
            ToolDescription {
                name: "mcp__mcp__Bash(git:commit)".to_string(),
                description: "Git commit".to_string(),
            },
            ToolDescription {
                name: "mcp__mcp__Bash(npm:install)".to_string(),
                description: "NPM install".to_string(),
            },
            ToolDescription {
                name: "mcp__mcp__Read".to_string(),
                description: "Read file".to_string(),
            },
            ToolDescription {
                name: "mcp__mcp__Write".to_string(),
                description: "Write file".to_string(),
            },
        ];

        let skill = LoadedSkill {
            metadata: crate::skills::SkillMetadata {
                name: "git-skill".to_string(),
                description: "Git operations".to_string(),
                license: None,
                compatibility: None,
                metadata: None,
                allowed_tools: Some("Bash(git:*) Read".to_string()),
                model: None,
            },
            content: "Git skill content".to_string(),
            raw_content: String::new(),
            skill_dir: PathBuf::new(),
            file_path: String::new(),
            enabled: true,
            loaded_at: Utc::now(),
        };

        let allowed_patterns = skill.metadata.get_allowed_tools();
        let tools_json = build_tools_json(&tools, Some(&allowed_patterns));

        let tool_array = tools_json.as_array().unwrap();
        // Should only include Bash(git:*) and Read
        assert_eq!(tool_array.len(), 3);

        let tool_names: Vec<&str> = tool_array
            .iter()
            .map(|t| t["function"]["name"].as_str().unwrap())
            .collect();

        assert!(tool_names.contains(&"mcp__mcp__Bash(git:status)"));
        assert!(tool_names.contains(&"mcp__mcp__Bash(git:commit)"));
        assert!(tool_names.contains(&"mcp__mcp__Read"));
        // Should NOT include npm or Write
        assert!(!tool_names.contains(&"mcp__mcp__Bash(npm:install)"));
        assert!(!tool_names.contains(&"mcp__mcp__Write"));
    }

    /// Test tool filtering with multiple patterns
    #[test]
    fn test_integration_tool_filtering_multiple_patterns() {
        let tools = vec![
            ToolDescription {
                name: "mcp__mcp__Bash(git:status)".to_string(),
                description: "Git status".to_string(),
            },
            ToolDescription {
                name: "mcp__mcp__Bash(npm:install)".to_string(),
                description: "NPM install".to_string(),
            },
            ToolDescription {
                name: "mcp__mcp__Read".to_string(),
                description: "Read file".to_string(),
            },
            ToolDescription {
                name: "mcp__mcp__Write".to_string(),
                description: "Write file".to_string(),
            },
            ToolDescription {
                name: "mcp__mcp__Search".to_string(),
                description: "Search".to_string(),
            },
        ];

        // Multiple patterns: git commands + Read + Write
        let patterns = vec![
            "Bash(git:*)".to_string(),
            "Read".to_string(),
            "Write".to_string(),
        ];

        let filtered = filter_tools_by_patterns(&tools, Some(&patterns));

        assert_eq!(filtered.len(), 3);
        let names: Vec<&str> = filtered.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"mcp__mcp__Bash(git:status)"));
        assert!(names.contains(&"mcp__mcp__Read"));
        assert!(names.contains(&"mcp__mcp__Write"));
    }

    /// Test tool filtering with no restrictions (None patterns)
    #[test]
    fn test_integration_tool_filtering_no_restrictions() {
        let tools = vec![
            ToolDescription {
                name: "mcp__server__tool1".to_string(),
                description: "Tool 1".to_string(),
            },
            ToolDescription {
                name: "mcp__server__tool2".to_string(),
                description: "Tool 2".to_string(),
            },
            ToolDescription {
                name: "mcp__server__tool3".to_string(),
                description: "Tool 3".to_string(),
            },
        ];

        // No patterns = all tools allowed
        let filtered = filter_tools_by_patterns(&tools, None);
        assert_eq!(filtered.len(), 3);

        let tools_json = build_tools_json(&tools, None);
        assert_eq!(tools_json.as_array().unwrap().len(), 3);
    }

    // ==========================================================================
    // Integration Tests: Error Handling
    // ==========================================================================

    /// Test that is_retryable_error correctly identifies retryable errors
    #[test]
    fn test_integration_error_handling_retryable() {
        // MCP operation errors are retryable
        let mcp_error = ServerError::McpOperation("Connection timeout".to_string());
        assert!(is_retryable_error(&mcp_error));

        // Tool call retry exhausted is retryable (at subtask level)
        let retry_error = ServerError::ToolCallRetryExhausted {
            tool_name: "test-tool".to_string(),
            attempts: 3,
            message: "Failed after retries".to_string(),
        };
        assert!(is_retryable_error(&retry_error));

        // Empty MCP content is retryable
        let empty_content = ServerError::McpEmptyContent;
        assert!(is_retryable_error(&empty_content));
    }

    /// Test that is_retryable_error correctly identifies non-retryable errors
    #[test]
    fn test_integration_error_handling_non_retryable() {
        // Cyclic dependency is not retryable
        let cyclic_error = ServerError::CyclicDependency;
        assert!(!is_retryable_error(&cyclic_error));

        // Empty plan is not retryable
        let empty_plan = ServerError::EmptyPlan;
        assert!(!is_retryable_error(&empty_plan));

        // Plan parse error is not retryable
        let parse_error = ServerError::PlanParseError("Invalid format".to_string());
        assert!(!is_retryable_error(&parse_error));
    }

    /// Test iteration trace records skill requests
    #[test]
    fn test_integration_iteration_trace_skill_request() {
        let mut iter_trace = IterationTrace::new(1);

        // Record a successful skill request
        iter_trace.set_skill_request("git-workflow".to_string(), true);

        assert_eq!(iter_trace.skill_requested, Some("git-workflow".to_string()));
        assert!(iter_trace.skill_loaded);

        // Test failed skill request
        let mut iter_trace2 = IterationTrace::new(2);
        iter_trace2.set_skill_request("nonexistent-skill".to_string(), false);

        assert_eq!(
            iter_trace2.skill_requested,
            Some("nonexistent-skill".to_string())
        );
        assert!(!iter_trace2.skill_loaded);
    }

    /// Test subtask trace records active skill
    #[test]
    fn test_integration_subtask_trace_active_skill() {
        let mut subtask_trace = SubtaskTrace::new(0, "Test task".to_string());

        // Initially no active skill
        assert!(subtask_trace.active_skill.is_none());

        // Set active skill
        subtask_trace.set_active_skill("git-workflow".to_string());
        assert_eq!(subtask_trace.active_skill, Some("git-workflow".to_string()));

        // Verify summary includes skill info
        let summary = subtask_trace.summary();
        assert!(summary.contains("skill=git-workflow"));
    }

    /// Test complete workflow: planning -> skill detection -> tool filtering
    #[test]
    fn test_integration_complete_skill_workflow() {
        use std::path::PathBuf;

        use chrono::Utc;

        use crate::skills::SkillDetector;

        // Step 1: Simulate LLM response with skill request
        let llm_response = r#"
            I need to help with git operations.
            <use_skill>git-workflow</use_skill>
            Let me check the status first.
        "#;

        // Step 2: Detect skill request
        let detected_skill = SkillDetector::detect_first(llm_response);
        assert_eq!(detected_skill, Some("git-workflow".to_string()));

        // Step 3: Load skill (simulated)
        let loaded_skill = LoadedSkill {
            metadata: crate::skills::SkillMetadata {
                name: "git-workflow".to_string(),
                description: "Git workflow assistance".to_string(),
                license: None,
                compatibility: None,
                metadata: None,
                allowed_tools: Some("Bash(git:*) Read".to_string()),
                model: None,
            },
            content: "Git workflow instructions".to_string(),
            raw_content: String::new(),
            skill_dir: PathBuf::new(),
            file_path: String::new(),
            enabled: true,
            loaded_at: Utc::now(),
        };

        // Step 4: Get allowed tools from skill
        let allowed_tools = loaded_skill.metadata.get_allowed_tools();
        assert_eq!(allowed_tools, vec!["Bash(git:*)", "Read"]);

        // Step 5: Filter tools
        let all_tools = vec![
            ToolDescription {
                name: "mcp__mcp__Bash(git:status)".to_string(),
                description: "Git status".to_string(),
            },
            ToolDescription {
                name: "mcp__mcp__Bash(npm:install)".to_string(),
                description: "NPM install".to_string(),
            },
            ToolDescription {
                name: "mcp__mcp__Read".to_string(),
                description: "Read".to_string(),
            },
        ];

        let filtered = filter_tools_by_patterns(&all_tools, Some(&allowed_tools));
        assert_eq!(filtered.len(), 2);

        // Step 6: Build context with active skill
        let subtask = SubTask::new(0, "Git task".to_string());
        let messages =
            build_context_for_react(&subtask, &[], &all_tools, None, Some(&loaded_skill));

        // Verify context includes skill
        if let ChatCompletionRequestMessage::System(sys_msg) = &messages[0] {
            assert!(sys_msg.content().contains("Active Skill: git-workflow"));
        }

        // Step 7: Record in trace
        let mut subtask_trace = SubtaskTrace::new(0, "Git task".to_string());
        subtask_trace.set_active_skill("git-workflow".to_string());

        let mut iter_trace = IterationTrace::new(1);
        iter_trace.set_skill_request("git-workflow".to_string(), true);

        subtask_trace.add_iteration(iter_trace);

        assert!(subtask_trace.summary().contains("skill=git-workflow"));
    }

    /// Test skill detection with strip_tags for clean response
    #[test]
    fn test_integration_skill_detection_and_cleanup() {
        use crate::skills::SkillDetector;

        let response_with_skill = "I will use <use_skill>code-review</use_skill> to help you.";

        // Extract skill and clean response
        let (skills, cleaned) = SkillDetector::extract_and_clean(response_with_skill);

        assert_eq!(skills, vec!["code-review"]);
        assert_eq!(cleaned, "I will use  to help you.");
        assert!(!cleaned.contains("<use_skill>"));
    }
}
