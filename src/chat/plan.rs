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
    xml_parser::{
        extract_action, extract_final_answer, extract_thought, extract_xml_tool_call,
        has_action_tag, has_final_answer_tag,
    },
};
use crate::{
    AppState,
    chat::{
        gen_chat_id,
        planner::{SubTask, TaskPlan, TaskPlanner, ToolDescription},
        trace::{
            PlanTrace, ReplanEvent, SubtaskReflectionSummary, SubtaskTrace, TokenUsage, TraceStatus,
        },
        utils::*,
    },
    dual_debug, dual_error, dual_info, dual_warn,
    error::{ServerError, ServerResult},
    mcp::{
        DEFAULT_SEARCH_FALLBACK_MESSAGE, MCP_SERVICES, SEARCH_MCP_SERVER_NAMES, extract_tool_name,
        format_mcp_tool_name, parse_mcp_tool_name,
    },
    reflection::{
        AdaptiveStrategy, CacheConfig, DependencyGraph, DynamicReplanner, FailedSubtaskInfo,
        LlmServerInfo, RecommendedAction, ReflectionCache, ReflectionContext, ReflectionEngine,
        ReplanContext, ReplanTrigger, SubtaskInfo,
    },
    server::{RoutingPolicy, ServerKind},
    skills::{
        LoadedSkill, ScriptContext, SkillDetector, SkillInjector, SkillLoader, SkillRegistry,
        SkillSummary,
    },
};

// ============================================================================
// Internal Tool Constants
// ============================================================================

/// Internal tool prefix for non-MCP tools
const INTERNAL_TOOL_PREFIX: &str = "internal";

/// Skill script execution tool name
const SKILL_RUN_SCRIPT_TOOL: &str = "skill_run_script";

/// Skill asset loading tool name
const SKILL_LOAD_ASSET_TOOL: &str = "skill_load_asset";

/// Full name for the skill_run_script internal tool
fn internal_tool_name(tool_name: &str) -> String {
    format!("{INTERNAL_TOOL_PREFIX}__{tool_name}")
}

/// Check if a tool name is an internal tool
fn is_internal_tool(tool_name: &str) -> bool {
    tool_name.starts_with(&format!("{INTERNAL_TOOL_PREFIX}__"))
}

/// Parse an internal tool name, returning the tool name if valid
fn parse_internal_tool_name(full_name: &str) -> Option<&str> {
    full_name.strip_prefix(&format!("{INTERNAL_TOOL_PREFIX}__"))
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
    let (
        max_plan_subtasks,
        plan_timeout_secs,
        subtask_max_retries,
        subtask_react_max_iterations,
        subtask_react_timeout_secs,
        max_tools_per_iteration,
        tool_call_max_retries,
        tool_call_retry_delay_ms,
        reflection_config,
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
            config.reflection.clone().unwrap_or_default(),
        )
    };

    // Initialize time budget for the entire plan
    let time_budget = TimeBudget::new(plan_timeout_secs);

    // Initialize reflection system (if enabled)
    let reflection_engine = if reflection_config.enabled {
        let server_info = Arc::new(tokio::sync::RwLock::new(LlmServerInfo {
            url: chat_server.url.clone(),
            api_key: chat_server.api_key.clone(),
        }));
        Some(ReflectionEngine::new(
            server_info,
            reflection_config.clone(),
        ))
    } else {
        None
    };

    // Initialize reflection cache (if reflection is enabled)
    let reflection_cache = if reflection_config.enabled {
        Some(ReflectionCache::new(CacheConfig::default()))
    } else {
        None
    };

    // Initialize adaptive strategy (if reflection is enabled)
    let adaptive_strategy = if reflection_config.enabled {
        Some(AdaptiveStrategy::new(Default::default()))
    } else {
        None
    };

    // Initialize dynamic replanner (if reflection is enabled)
    let dynamic_replanner = if reflection_config.enabled {
        let server_info = Arc::new(tokio::sync::RwLock::new(LlmServerInfo {
            url: chat_server.url.clone(),
            api_key: chat_server.api_key.clone(),
        }));
        Some(DynamicReplanner::with_defaults(server_info))
    } else {
        None
    };

    if reflection_config.enabled {
        dual_info!("🔍 Reflection system enabled - request_id: {}", request_id);
    }

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

    // Get model name from request, fallback to "default" if not specified
    let model_name = request
        .model
        .clone()
        .unwrap_or_else(|| "default".to_string());

    // Create task planner
    let planner = TaskPlanner::with_chat_llm(
        format!("{}/chat/completions", chat_server.url.trim_end_matches('/')),
        chat_server.api_key.clone(),
        model_name.clone(),
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

    // Build dependency graph for critical subtask detection (R5.2)
    #[allow(unused_variables)]
    let dependency_graph = DependencyGraph::from_subtask_dependencies(
        &plan.subtasks.iter().map(|s| s.id).collect::<Vec<_>>(),
        &plan
            .subtasks
            .iter()
            .map(|s| s.dependencies.clone())
            .collect::<Vec<_>>(),
    );

    // Calculate pending subtask count for time allocation
    let mut pending_count = plan.execution_order.len();

    // Replan tracking (for R5.2 dynamic replanner integration)
    #[allow(unused_variables, unused_mut)]
    let mut replan_count = 0u32;
    #[allow(dead_code)]
    const MAX_REPLAN_ATTEMPTS: u32 = 3;

    'execution: loop {
        let execution_order = plan.execution_order.clone();

        for &subtask_idx in &execution_order {
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
                    conv_id.as_deref(),
                    attempt_timeout,
                    subtask_react_max_iterations,
                    max_tools_per_iteration,
                    tool_call_max_retries,
                    tool_call_retry_delay_ms,
                    &cancel_token,
                    request_id,
                    &mut subtask_trace,
                    &model_name,
                )
                .await;

                match result {
                    Ok(result_text) => {
                        // Perform reflection on the result (if enabled)
                        let should_retry = if let Some(ref engine) = reflection_engine {
                            // Build reflection context
                            let deps_results: Vec<String> = subtask
                                .dependencies
                                .iter()
                                .filter_map(|dep_id| {
                                    subtask_results
                                        .iter()
                                        .find(|(id, _)| *id == *dep_id)
                                        .map(|(_, r)| r.clone())
                                })
                                .collect();

                            let tool_calls: Vec<String> = subtask_trace
                                .react_iterations
                                .iter()
                                .flat_map(|it| it.tool_calls.iter().map(|tc| tc.tool_name.clone()))
                                .collect();

                            let context = ReflectionContext::new(&subtask.description)
                                .with_dependencies(deps_results)
                                .with_iterations(subtask_trace.react_iterations.len() as u32)
                                .with_tool_calls(tool_calls)
                                .with_time_taken(subtask_start_time.elapsed().as_millis() as u64);

                            // Check cache first (if enabled)
                            let cached_result = if let Some(ref cache) = reflection_cache {
                                cache.get(&subtask.description, &result_text)
                            } else {
                                None
                            };

                            if let Some(cached) = cached_result {
                                dual_debug!(
                                    "🔍 Using cached reflection for subtask {} - request_id: {}",
                                    subtask.id,
                                    request_id
                                );

                                // Record cached reflection result to subtask trace
                                subtask_trace.set_reflection(
                                    SubtaskReflectionSummary::from_result(
                                        &cached, true, // from_cache = true
                                    ),
                                );

                                // Use cached reflection result
                                match cached.recommended_action {
                                    RecommendedAction::Accept
                                    | RecommendedAction::AcceptWithFix(_) => false,
                                    RecommendedAction::Retry
                                    | RecommendedAction::RetryWithStrategy(_) => {
                                        attempt < subtask_max_retries
                                    }
                                    _ => false,
                                }
                            } else {
                                // Perform fresh reflection
                                match engine
                                    .reflect_on_subtask(
                                        subtask,
                                        &result_text,
                                        &subtask_trace,
                                        &context,
                                    )
                                    .await
                                {
                                    Ok(reflection) => {
                                        dual_info!(
                                            "🔍 Reflection for subtask {}: {} - request_id: {}",
                                            subtask.id,
                                            reflection.summary(),
                                            request_id
                                        );

                                        // Record fresh reflection result to subtask trace
                                        subtask_trace.set_reflection(
                                            SubtaskReflectionSummary::from_result(
                                                &reflection,
                                                false, // from_cache = false
                                            ),
                                        );

                                        // Store in cache
                                        if let Some(ref cache) = reflection_cache {
                                            cache.put(
                                                &subtask.description,
                                                &result_text,
                                                reflection.clone(),
                                            );
                                        }

                                        // Update adaptive strategy
                                        if let Some(ref strategy) = adaptive_strategy {
                                            strategy.record_outcome(
                                                &subtask.description,
                                                reflection.passed,
                                                reflection.reflection_rounds,
                                                reflection.confidence,
                                            );
                                        }

                                        // Determine if retry is needed based on reflection
                                        match &reflection.recommended_action {
                                            RecommendedAction::Accept
                                            | RecommendedAction::AcceptWithFix(_) => false,
                                            RecommendedAction::Retry
                                            | RecommendedAction::RetryWithStrategy(_) => {
                                                if !reflection.passed
                                                    && attempt < subtask_max_retries
                                                {
                                                    dual_warn!(
                                                        "🔄 Reflection suggests retry for subtask {} (confidence: {:.2}) - request_id: {}",
                                                        subtask.id,
                                                        reflection.confidence,
                                                        request_id
                                                    );
                                                    subtask_trace.record_retry(format!(
                                                        "Reflection: {}",
                                                        reflection
                                                            .issues
                                                            .first()
                                                            .map(|i| i.description.as_str())
                                                            .unwrap_or("Low confidence")
                                                    ));
                                                    true
                                                } else {
                                                    false
                                                }
                                            }
                                            RecommendedAction::Replan(replan_request) => {
                                                // R5.2: Handle reflection-suggested replan
                                                // Note: Actual replanning is deferred to after subtask borrow ends
                                                dual_info!(
                                                    "🔄 Reflection suggests replan for subtask {}: {} - request_id: {}",
                                                    subtask.id,
                                                    replan_request.reason,
                                                    request_id
                                                );
                                                // For now, just log and accept the result
                                                // Full replanning integration is handled via failure path
                                                false
                                            }
                                            RecommendedAction::RequestClarification(msg) => {
                                                dual_warn!(
                                                    "❓ Reflection requests clarification for subtask {}: {} - request_id: {}",
                                                    subtask.id,
                                                    msg,
                                                    request_id
                                                );
                                                false
                                            }
                                            RecommendedAction::Abort(reason) => {
                                                dual_warn!(
                                                    "⛔ Reflection suggests abort for subtask {}: {} - request_id: {}",
                                                    subtask.id,
                                                    reason,
                                                    request_id
                                                );
                                                false
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        // Reflection failed, log and continue without retry
                                        dual_warn!(
                                            "⚠️ Reflection failed for subtask {}: {} - request_id: {}",
                                            subtask.id,
                                            e,
                                            request_id
                                        );
                                        false
                                    }
                                }
                            }
                        } else {
                            false
                        };

                        if should_retry {
                            // Continue to next retry attempt
                            last_error = Some(ServerError::Operation(
                                "Reflection suggested retry".to_string(),
                            ));
                            continue;
                        }

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

            // R5.2: Check if replanning should be triggered after failure
            // This is done after subtask borrow ends to avoid conflicts
            if let Some(ref replanner) = dynamic_replanner {
                let replan_config = replanner.config().clone();

                if let Some(trigger) =
                    ReplanTrigger::should_replan(&trace, &replan_config, &dependency_graph)
                {
                    if replan_count < MAX_REPLAN_ATTEMPTS {
                        dual_info!(
                            "🔄 Replan triggered: {} - request_id: {}",
                            trigger.description(),
                            request_id
                        );

                        // Capture plan info
                        let original_goal = plan.original_goal.clone();
                        let pending = extract_pending_subtasks(&plan, &completed_subtasks);

                        // Capture trigger description before move
                        let trigger_desc = format!("{:?}", trigger);

                        // Execute replanning
                        match execute_replan(
                            replanner,
                            &original_goal,
                            pending,
                            &subtask_results,
                            &trace,
                            trigger,
                            Some(time_budget.remaining()),
                            request_id,
                        )
                        .await
                        {
                            Ok(replan_result) => {
                                // Record the replan event to trace
                                trace.add_replan_event(ReplanEvent {
                                    timestamp: chrono::Utc::now(),
                                    trigger: trigger_desc,
                                    preserved_count: replan_result.preserved_subtasks.len(),
                                    added_count: replan_result.added_subtasks.len(),
                                    removed_count: replan_result.removed_subtasks.len(),
                                });

                                // Apply the new plan
                                apply_new_plan(&mut plan, replan_result, request_id);
                                replan_count += 1;

                                // Reset state for new plan execution
                                completed_subtasks.clear();
                                for (id, _) in &subtask_results {
                                    completed_subtasks.insert(*id);
                                }
                                pending_count = plan.execution_order.len();

                                dual_info!(
                                    "🔄 Restarting execution with new plan (attempt {}/{}) - request_id: {}",
                                    replan_count,
                                    MAX_REPLAN_ATTEMPTS,
                                    request_id
                                );
                                continue 'execution;
                            }
                            Err(e) => {
                                dual_warn!(
                                    "⚠️ Replanning failed: {} - continuing with current plan - request_id: {}",
                                    e,
                                    request_id
                                );
                            }
                        }
                    } else {
                        dual_warn!(
                            "⚠️ Max replan attempts ({}) reached - request_id: {}",
                            MAX_REPLAN_ATTEMPTS,
                            request_id
                        );
                    }
                }
            }
        }

        // All subtasks in current plan completed, exit the execution loop
        break 'execution;
    }

    // ========================================================================
    // Phase 3: Result Aggregation
    // ========================================================================

    dual_info!("📊 Aggregating results - request_id: {}", request_id);

    // Compute reflection summary before finalizing trace
    trace.compute_reflection_summary();

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

/// Gets available tools from MCP services and internal tools.
///
/// Returns both MCP tools (from registered MCP servers) and internal tools
/// (like `skill_run_script`).
async fn get_available_tools() -> Vec<ToolDescription> {
    let mut tools = Vec::new();

    // Add MCP tools
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

    // Add internal tools (only available when skills are loaded)
    if SkillRegistry::global().is_ok() {
        tools.push(ToolDescription {
            name: internal_tool_name(SKILL_RUN_SCRIPT_TOOL),
            description: "Execute a script from an active skill. Use this tool to run scripts in the skill's scripts/ directory.".to_string(),
        });
        tools.push(ToolDescription {
            name: internal_tool_name(SKILL_LOAD_ASSET_TOOL),
            description: "Load an asset file from the active skill's assets/ directory. Supports template variable replacement and JSON/YAML parsing.".to_string(),
        });
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
    conv_id: Option<&str>,
    timeout: Duration,
    max_iterations: u32,
    max_tools_per_iteration: usize,
    tool_call_max_retries: u32,
    tool_call_retry_delay_ms: u64,
    cancel_token: &CancellationToken,
    request_id: &str,
    subtask_trace: &mut SubtaskTrace,
    model: &str,
) -> ServerResult<String> {
    let start_time = Instant::now();
    let tool_call_retry_delay = Duration::from_millis(tool_call_retry_delay_ms);

    // Get max_reference_size from config
    let max_reference_size = state
        .config
        .read()
        .await
        .skill
        .as_ref()
        .map(|s| s.max_reference_size)
        .unwrap_or(102400); // Default 100KB

    // Track active skills for Phase 2 (supports multi-skill activation)
    let mut active_skills: Vec<LoadedSkill> = Vec::new();

    // Build initial messages for React loop (Phase 1: no active skills yet)
    let mut messages = build_context_for_react(
        subtask,
        previous_results,
        available_tools,
        skills_summaries,
        &[], // No active skills in initial context
        max_reference_size,
    )
    .await;

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

        // Build request with tools (filtered by active skills if any)
        let allowed_patterns = if active_skills.is_empty() {
            None
        } else {
            Some(SkillInjector::merge_allowed_tools(&active_skills))
        };
        let tools_json = build_tools_json(available_tools, allowed_patterns.as_deref());
        let request_json = serde_json::json!({
            "model": model,
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

        // Check for tool calls - support both OpenAI JSON format and XML JSON-embedded format
        let json_tool_calls = &chat_completion.choices[0].message.tool_calls;
        let content = chat_completion.choices[0].message.content.as_ref();

        // Priority: OpenAI JSON format > XML JSON-embedded format
        let requires_tool_call = if !json_tool_calls.is_empty() {
            true
        } else if let Some(content) = content {
            // Check for XML tool call with JSON-embedded format
            // Must have <action> tag, not have <final_answer> tag, and be valid JSON
            has_action_tag(content)
                && !has_final_answer_tag(content)
                && extract_xml_tool_call(content).is_some()
        } else {
            false
        };

        if requires_tool_call {
            // Check which format is used
            if !json_tool_calls.is_empty() {
                // === OpenAI JSON format tool calls ===
                let tool_calls_to_execute = if json_tool_calls.len() > max_tools_per_iteration {
                    &json_tool_calls[..max_tools_per_iteration]
                } else {
                    json_tool_calls.as_slice()
                };

                // Extract thought from content if present
                if let Some(content) = content {
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
                        &active_skills,
                        conv_id,
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
            } else if let Some(content) = content {
                // === XML JSON-embedded format tool call ===
                if let Some(xml_tool_call) = extract_xml_tool_call(content) {
                    // Extract thought
                    if let Some(thought) = extract_thought(content) {
                        dual_info!("💭 Subtask {} Thought: {}", subtask.id, thought);
                        iter_trace.thought = Some(thought);
                    }

                    dual_info!(
                        "🔧 Subtask {} Action (XML JSON): {}",
                        subtask.id,
                        xml_tool_call.tool_name
                    );
                    iter_trace.action = Some(xml_tool_call.tool_name.clone());

                    // Construct ToolCall structure for execution
                    let tool_call = endpoints::chat::ToolCall {
                        id: format!("xml_call_{}", gen_chat_id()),
                        ty: "function".to_string(),
                        function: endpoints::chat::Function {
                            name: xml_tool_call.tool_name,
                            arguments: xml_tool_call.arguments,
                        },
                    };

                    // Execute tool call
                    let tool_result = execute_tool_call(
                        state,
                        &tool_call,
                        &active_skills,
                        conv_id,
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
                            Some(content.clone()),
                            None,
                            Some(vec![tool_call.clone()]),
                        ),
                    ));

                    // Append tool result message
                    messages.push(ChatCompletionRequestMessage::Tool(
                        ChatCompletionToolMessage::new(&observation, &tool_call.id),
                    ));
                }
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
                // Supports multi-skill activation with priority and conflict resolution
                if active_skills.is_empty()
                    && let Ok(registry) = SkillRegistry::global()
                {
                    // Get all loaded skills for priority/conflict resolution
                    let all_loaded_skills = registry.get_all_loaded().await;

                    // Detect and resolve skills (priority sorting + conflict resolution)
                    let (resolved_skills, removed_skills) =
                        SkillDetector::detect_and_resolve(content, &all_loaded_skills);

                    if !resolved_skills.is_empty() {
                        // Log resolved skills
                        if resolved_skills.len() == 1 {
                            dual_info!(
                                "🎯 Subtask {} requested skill: {} - request_id: {}",
                                subtask.id,
                                resolved_skills[0],
                                request_id
                            );
                        } else {
                            dual_info!(
                                "🎯 Subtask {} requested {} skills: [{}] - request_id: {}",
                                subtask.id,
                                resolved_skills.len(),
                                resolved_skills.join(", "),
                                request_id
                            );
                        }

                        // Log removed skills (if any)
                        for (removed, reason) in &removed_skills {
                            dual_warn!(
                                "⚠️ Skill '{}' removed: {} - request_id: {}",
                                removed,
                                reason,
                                request_id
                            );
                        }

                        // Load all resolved skills
                        let mut loaded_skills_list: Vec<LoadedSkill> = Vec::new();
                        let mut skill_names_loaded: Vec<String> = Vec::new();

                        for skill_name in &resolved_skills {
                            if let Some(loaded_skill) = registry.get(skill_name).await {
                                dual_info!(
                                    "📖 Loaded skill '{}' for subtask {} - request_id: {}",
                                    skill_name,
                                    subtask.id,
                                    request_id
                                );
                                skill_names_loaded.push(skill_name.clone());
                                loaded_skills_list.push(loaded_skill);
                            } else {
                                dual_warn!(
                                    "⚠️ Skill '{}' not found, skipping - request_id: {}",
                                    skill_name,
                                    request_id
                                );
                            }
                        }

                        if !loaded_skills_list.is_empty() {
                            // Record skill request in iteration trace
                            // For multi-skill, we record the primary (first) skill
                            iter_trace.set_skill_request(skill_names_loaded[0].clone(), true);

                            // Record skill activation in subtask trace (using set_active_skills for multi-skill)
                            subtask_trace.set_active_skills(skill_names_loaded.clone());

                            // Store the active skills
                            active_skills = loaded_skills_list;

                            // Rebuild context with the active skills (Phase 2)
                            messages = build_context_for_react(
                                subtask,
                                previous_results,
                                available_tools,
                                None, // No need for summaries in Phase 2
                                &active_skills,
                                max_reference_size,
                            )
                            .await;

                            // Finalize iteration trace and continue loop
                            iter_trace.duration = iter_start.elapsed();
                            subtask_trace.add_iteration(iter_trace);
                            continue;
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
///
/// Supports both MCP tools (format: `mcp__{server}__{tool}`) and internal tools
/// (format: `internal__{tool}`).
///
/// # Internal Tools
/// - `internal__skill_run_script`: Execute a script from the active skill
#[allow(clippy::too_many_arguments)]
async fn execute_tool_call(
    _state: &Arc<AppState>,
    tool_call: &endpoints::chat::ToolCall,
    active_skills: &[LoadedSkill],
    conv_id: Option<&str>,
    max_retries: u32,
    retry_delay: Duration,
    request_id: &str,
    iter_trace: &mut IterationTrace,
) -> ServerResult<String> {
    let tool_call_start = Instant::now();
    let tool_args: serde_json::Value =
        serde_json::from_str(&tool_call.function.arguments).unwrap_or(serde_json::json!({}));

    // Check if this is an internal tool
    if is_internal_tool(&tool_call.function.name) {
        return execute_internal_tool(
            &tool_call.function.name,
            tool_args,
            active_skills,
            conv_id,
            request_id,
            iter_trace,
            tool_call_start,
        )
        .await;
    }

    // Parse MCP tool name and server name
    let (server_name, tool_name) =
        parse_mcp_tool_name(&tool_call.function.name).ok_or_else(|| {
            let err_msg = format!("Invalid tool name format: {}", tool_call.function.name);
            ServerError::Operation(err_msg)
        })?;

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

/// Executes an internal tool (non-MCP tool).
///
/// # Supported Internal Tools
///
/// - `internal__skill_run_script`: Execute a script from the active skill
///   - Arguments: `script_name` (required), `args` (optional array)
///   - Requires an active skill to be loaded
///
/// - `internal__skill_load_asset`: Load an asset file from the active skill
///   - Arguments: `asset_name` (required), `variables` (optional object), `parse_as` (optional)
///   - Requires an active skill to be loaded
async fn execute_internal_tool(
    full_tool_name: &str,
    tool_args: serde_json::Value,
    active_skills: &[LoadedSkill],
    conv_id: Option<&str>,
    request_id: &str,
    iter_trace: &mut IterationTrace,
    start_time: Instant,
) -> ServerResult<String> {
    let tool_name = parse_internal_tool_name(full_tool_name).ok_or_else(|| {
        ServerError::Operation(format!("Invalid internal tool name: {}", full_tool_name))
    })?;

    // Initialize tool trace for internal tool
    let mut tool_trace = ToolCallTrace::new(
        tool_name.to_string(),
        INTERNAL_TOOL_PREFIX.to_string(),
        tool_args.clone(),
    );

    match tool_name {
        SKILL_RUN_SCRIPT_TOOL => {
            execute_skill_run_script(
                tool_args,
                active_skills,
                conv_id,
                request_id,
                &mut tool_trace,
                iter_trace,
                start_time,
            )
            .await
        }
        SKILL_LOAD_ASSET_TOOL => {
            execute_skill_load_asset(
                tool_args,
                active_skills,
                request_id,
                &mut tool_trace,
                iter_trace,
                start_time,
            )
            .await
        }
        _ => {
            let err_msg = format!("Unknown internal tool: {}", tool_name);
            tool_trace.set_error(err_msg.clone(), start_time.elapsed());
            iter_trace.add_tool_call(tool_trace);
            Err(ServerError::Operation(err_msg))
        }
    }
}

/// Executes the skill_run_script internal tool.
///
/// # Arguments Schema
/// ```json
/// {
///   "script_name": "process.js",  // Required: name of the script file
///   "args": ["--input", "data.json"]  // Optional: command line arguments
/// }
/// ```
///
/// # Returns
/// The script output as a formatted string including stdout, stderr, and exit code.
async fn execute_skill_run_script(
    tool_args: serde_json::Value,
    active_skills: &[LoadedSkill],
    conv_id: Option<&str>,
    request_id: &str,
    tool_trace: &mut ToolCallTrace,
    iter_trace: &mut IterationTrace,
    start_time: Instant,
) -> ServerResult<String> {
    // Require at least one active skill
    // For multi-skill scenarios, use the first skill that has the requested script
    let skill = active_skills.first().ok_or_else(|| {
        let err_msg =
            "skill_run_script requires an active skill. Use <use_skill> to activate a skill first.";
        tool_trace.set_error(err_msg.to_string(), start_time.elapsed());
        iter_trace.add_tool_call(tool_trace.clone());
        ServerError::Operation(err_msg.to_string())
    })?;

    // Parse arguments
    let script_name = tool_args
        .get("script_name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            let err_msg = "skill_run_script requires 'script_name' argument";
            tool_trace.set_error(err_msg.to_string(), start_time.elapsed());
            iter_trace.add_tool_call(tool_trace.clone());
            ServerError::Operation(err_msg.to_string())
        })?;

    let args: Vec<String> = tool_args
        .get("args")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    dual_info!(
        "🔧 Executing skill script: {} (skill: {}, args: {:?}) - request_id: {}",
        script_name,
        skill.metadata.name,
        args,
        request_id
    );

    // Build execution context
    let context =
        ScriptContext::with_ids(conv_id.map(|s| s.to_string()), Some(request_id.to_string()));

    // Execute the script
    match skill
        .execute_script_with_context(script_name, args.clone(), context, None)
        .await
    {
        Ok(output) => {
            // Format result for LLM
            let result = if output.exit_code == 0 {
                format!(
                    "Script '{}' executed successfully.\n\nOutput:\n{}",
                    script_name,
                    output.stdout.trim()
                )
            } else {
                format!(
                    "Script '{}' failed with exit code {}.\n\nStdout:\n{}\n\nStderr:\n{}",
                    script_name,
                    output.exit_code,
                    output.stdout.trim(),
                    output.stderr.trim()
                )
            };

            dual_info!(
                "✅ Script execution completed: {} (exit_code: {}, duration: {:?}) - request_id: {}",
                script_name,
                output.exit_code,
                output.duration,
                request_id
            );

            tool_trace.set_result(result.clone(), start_time.elapsed());
            iter_trace.add_tool_call(tool_trace.clone());
            Ok(result)
        }
        Err(e) => {
            let err_msg = format!("Script execution failed: {}", e);
            dual_warn!(
                "❌ Script execution failed: {} - {} - request_id: {}",
                script_name,
                e,
                request_id
            );
            tool_trace.set_error(err_msg.clone(), start_time.elapsed());
            iter_trace.add_tool_call(tool_trace.clone());
            Err(ServerError::Operation(err_msg))
        }
    }
}

/// Executes the skill_load_asset internal tool.
///
/// # Arguments Schema
/// ```json
/// {
///   "asset_name": "template.md",  // Required: name of the asset file
///   "variables": {                 // Optional: variables for template replacement
///     "name": "John",
///     "date": "2024-01-15"
///   },
///   "parse_as": "json"            // Optional: parse content as "json", "yaml", or "markdown"
/// }
/// ```
///
/// # Returns
/// The asset content as a formatted string, optionally with variables replaced
/// and/or parsed as structured data.
async fn execute_skill_load_asset(
    tool_args: serde_json::Value,
    active_skills: &[LoadedSkill],
    request_id: &str,
    tool_trace: &mut ToolCallTrace,
    iter_trace: &mut IterationTrace,
    start_time: Instant,
) -> ServerResult<String> {
    // Require at least one active skill
    // For multi-skill scenarios, use the first skill that has the requested asset
    let skill = active_skills.first().ok_or_else(|| {
        let err_msg =
            "skill_load_asset requires an active skill. Use <use_skill> to activate a skill first.";
        tool_trace.set_error(err_msg.to_string(), start_time.elapsed());
        iter_trace.add_tool_call(tool_trace.clone());
        ServerError::Operation(err_msg.to_string())
    })?;

    // Parse arguments
    let asset_name = tool_args
        .get("asset_name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            let err_msg = "skill_load_asset requires 'asset_name' argument";
            tool_trace.set_error(err_msg.to_string(), start_time.elapsed());
            iter_trace.add_tool_call(tool_trace.clone());
            ServerError::Operation(err_msg.to_string())
        })?;

    let variables = tool_args
        .get("variables")
        .and_then(|v| v.as_object())
        .cloned();

    let parse_as = tool_args
        .get("parse_as")
        .and_then(|v| v.as_str())
        .map(|s| s.to_lowercase());

    dual_info!(
        "📦 Loading skill asset: {} (skill: {}, parse_as: {:?}) - request_id: {}",
        asset_name,
        skill.metadata.name,
        parse_as,
        request_id
    );

    // Load asset content
    let content = SkillLoader::load_asset_string(&skill.skill_dir, asset_name)
        .await
        .ok_or_else(|| {
            let err_msg = format!(
                "Asset '{}' not found in skill '{}'",
                asset_name, skill.metadata.name
            );
            tool_trace.set_error(err_msg.clone(), start_time.elapsed());
            iter_trace.add_tool_call(tool_trace.clone());
            ServerError::Operation(err_msg)
        })?;

    // Apply template variable replacement if variables provided
    let content = if let Some(vars) = variables {
        apply_template_variables(&content, &vars)
    } else {
        content
    };

    // Parse content if requested
    let result = match parse_as.as_deref() {
        Some("json") => parse_as_json(&content, asset_name)?,
        Some("yaml") => parse_as_yaml(&content, asset_name)?,
        Some("markdown") | Some("md") => format_as_markdown(&content, asset_name),
        Some(unknown) => {
            let err_msg = format!(
                "Unknown parse_as format '{}'. Supported: json, yaml, markdown",
                unknown
            );
            tool_trace.set_error(err_msg.clone(), start_time.elapsed());
            iter_trace.add_tool_call(tool_trace.clone());
            return Err(ServerError::Operation(err_msg));
        }
        None => format!(
            "Asset '{}' loaded successfully.\n\nContent:\n{}",
            asset_name, content
        ),
    };

    dual_info!(
        "✅ Asset loaded: {} (size: {} bytes) - request_id: {}",
        asset_name,
        result.len(),
        request_id
    );

    tool_trace.set_result(result.clone(), start_time.elapsed());
    iter_trace.add_tool_call(tool_trace.clone());
    Ok(result)
}

/// Apply template variable replacement using {{variable}} syntax.
///
/// Replaces occurrences of `{{variable_name}}` with the corresponding value
/// from the variables map. Variables that are not found remain unchanged.
fn apply_template_variables(
    content: &str,
    variables: &serde_json::Map<String, serde_json::Value>,
) -> String {
    let mut result = content.to_string();

    for (key, value) in variables {
        let placeholder = format!("{{{{{}}}}}", key); // {{key}}
        let replacement = match value {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Number(n) => n.to_string(),
            serde_json::Value::Bool(b) => b.to_string(),
            serde_json::Value::Null => "null".to_string(),
            _ => value.to_string(), // For arrays/objects, use JSON representation
        };
        result = result.replace(&placeholder, &replacement);
    }

    result
}

/// Parse content as JSON and format for display.
fn parse_as_json(content: &str, asset_name: &str) -> ServerResult<String> {
    let parsed: serde_json::Value = serde_json::from_str(content).map_err(|e| {
        ServerError::Operation(format!("Failed to parse '{}' as JSON: {}", asset_name, e))
    })?;

    let formatted = serde_json::to_string_pretty(&parsed).map_err(|e| {
        ServerError::Operation(format!(
            "Failed to format JSON from '{}': {}",
            asset_name, e
        ))
    })?;

    Ok(format!(
        "Asset '{}' loaded and parsed as JSON.\n\nContent:\n```json\n{}\n```",
        asset_name, formatted
    ))
}

/// Parse content as YAML and format for display.
fn parse_as_yaml(content: &str, asset_name: &str) -> ServerResult<String> {
    // Parse YAML to validate it
    let parsed: serde_yaml::Value = serde_yaml::from_str(content).map_err(|e| {
        ServerError::Operation(format!("Failed to parse '{}' as YAML: {}", asset_name, e))
    })?;

    // Re-serialize for consistent formatting
    let formatted = serde_yaml::to_string(&parsed).map_err(|e| {
        ServerError::Operation(format!(
            "Failed to format YAML from '{}': {}",
            asset_name, e
        ))
    })?;

    Ok(format!(
        "Asset '{}' loaded and parsed as YAML.\n\nContent:\n```yaml\n{}\n```",
        asset_name, formatted
    ))
}

/// Format content as Markdown for display.
fn format_as_markdown(content: &str, asset_name: &str) -> String {
    format!(
        "Asset '{}' loaded as Markdown.\n\nContent:\n\n{}",
        asset_name, content
    )
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

/// Filters tools based on skills' allowed_tools.
///
/// - Phase 1 (no active_skills): Filters out tools that are covered by any skill's allowed_tools
/// - Phase 2 (with active_skills): Only shows tools declared in the skills' merged allowed_tools (if any)
///
/// This prevents redundancy between skill descriptions and tool listings.
fn filter_tools_by_skills<'a>(
    tools: &'a [ToolDescription],
    skills_summaries: Option<&[SkillSummary]>,
    active_skills: &[LoadedSkill],
) -> Vec<&'a ToolDescription> {
    if !active_skills.is_empty() {
        // Phase 2: Only show skill-related tools (merged from all active skills)
        let merged_tools = SkillInjector::merge_allowed_tools(active_skills);
        if merged_tools.is_empty() {
            // No allowed-tools specified in any skill, show all tools (backward compatibility)
            tools.iter().collect()
        } else {
            // Only show tools declared in merged allowed-tools
            let skill_tools_set: HashSet<&str> = merged_tools.iter().map(|s| s.as_str()).collect();

            let filtered: Vec<&ToolDescription> = tools
                .iter()
                .filter(|t| skill_tools_set.contains(t.name.as_str()))
                .collect();

            if active_skills.len() == 1 {
                dual_debug!(
                    "Phase 2 tool filtering: skill '{}' allows {} tools, showing {} of {} available",
                    active_skills[0].metadata.name,
                    merged_tools.len(),
                    filtered.len(),
                    tools.len()
                );
            } else {
                let skill_names: Vec<&str> = active_skills
                    .iter()
                    .map(|s| s.metadata.name.as_str())
                    .collect();
                dual_debug!(
                    "Phase 2 tool filtering: {} skills [{}] allow {} merged tools, showing {} of {} available",
                    active_skills.len(),
                    skill_names.join(", "),
                    merged_tools.len(),
                    filtered.len(),
                    tools.len()
                );
            }

            filtered
        }
    } else {
        // Phase 1: Filter out tools covered by skills
        // Collect all tools covered by any skill
        let covered_tools: HashSet<&str> = skills_summaries
            .map(|summaries| {
                summaries
                    .iter()
                    .flat_map(|s| s.allowed_tools.iter().map(|t| t.as_str()))
                    .collect()
            })
            .unwrap_or_default();

        if covered_tools.is_empty() {
            // No tools to filter, show all
            return tools.iter().collect();
        }

        // Filter out covered tools
        let filtered: Vec<&ToolDescription> = tools
            .iter()
            .filter(|t| !covered_tools.contains(t.name.as_str()))
            .collect();

        dual_debug!(
            "Phase 1 tool filtering: {} tools covered by skills, showing {} of {} available",
            covered_tools.len(),
            filtered.len(),
            tools.len()
        );

        filtered
    }
}

/// Builds the initial context messages for React loop execution.
///
/// This function supports two-phase skill loading:
/// - Phase 1 (no active_skills): Injects skills summaries, allows LLM to request skills
/// - Phase 2 (with active_skills): Injects full skill content, references, and filtered tools
///
/// Supports multi-skill activation: when multiple skills are active, their content
/// is merged using `SkillInjector::multi_skill_injection_auto_refs()`.
///
/// # Arguments
/// * `subtask` - The current subtask being executed
/// * `previous_results` - Results from dependent subtasks
/// * `available_tools` - List of available MCP tools
/// * `skills_summaries` - Optional skill summaries for Phase 1
/// * `active_skills` - Active skills for Phase 2 (empty slice for Phase 1)
/// * `max_reference_size` - Maximum total size of reference documents to load (0 = no limit)
pub(crate) async fn build_context_for_react(
    subtask: &SubTask,
    previous_results: &[(usize, String)],
    available_tools: &[ToolDescription],
    skills_summaries: Option<&[SkillSummary]>,
    active_skills: &[LoadedSkill],
    max_reference_size: usize,
) -> Vec<ChatCompletionRequestMessage> {
    let mut messages = Vec::new();

    // Filter tools based on skills' allowed_tools
    let filtered_tools = filter_tools_by_skills(available_tools, skills_summaries, active_skills);

    // Build tools description from filtered list
    let tools_desc = if filtered_tools.is_empty() {
        "No tools are currently available.".to_string()
    } else {
        filtered_tools
            .iter()
            .map(|t| format!("- {}: {}", t.name, t.description))
            .collect::<Vec<_>>()
            .join("\n")
    };

    // Build the system prompt based on whether we have active skills
    let system_prompt = if !active_skills.is_empty() {
        // Phase 2: Active skills - inject full skill content with references
        // For multi-skill, use multi_skill_injection_auto_refs to merge all skills
        let skill_section =
            SkillInjector::multi_skill_injection_auto_refs(active_skills, max_reference_size).await;
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
- Use <action></action> tags for tool calls with JSON format:
  <action>{{"name": "tool_name", "arguments": {{"param": "value"}}}}</action>
- When done, use <final_answer></final_answer> tags for your final response

## Tool Call Examples

### Example 1: MCP Tool Call
<thought>I need to calculate the sum of two numbers</thought>
<action>{{"name": "mcp__cardea-calculator__sum", "arguments": {{"a": 23, "b": 32}}}}</action>

### Example 2: Run a Script from the Active Skill
<thought>I need to run a script from the skill to process data</thought>
<action>{{"name": "internal__skill_run_script", "arguments": {{"script_name": "process.py", "args": ["--input", "data.csv", "--output", "result.json"]}}}}</action>

### Example 3: Load an Asset with Template Variables
<thought>I need to load a template and fill in the variables</thought>
<action>{{"name": "internal__skill_load_asset", "arguments": {{"asset_name": "report-template.md", "variables": {{"title": "Monthly Report", "date": "2024-01-15"}}}}}}</action>

### Example 4: Load and Parse a JSON Configuration
<thought>I need to read the configuration file as structured data</thought>
<action>{{"name": "internal__skill_load_asset", "arguments": {{"asset_name": "config.json", "parse_as": "json"}}}}</action>

**Important**: When using internal tools:
- `internal__skill_run_script`:
  - `script_name`: Just the filename (e.g., "convert.py"), not the full path
  - `args`: Array of command line arguments to pass to the script
- `internal__skill_load_asset`:
  - `asset_name`: Just the filename in the assets/ directory
  - `variables`: Object with key-value pairs to replace {{key}} in the template
  - `parse_as`: Optional format ("json", "yaml", "markdown") for structured parsing

After receiving the observation, provide your final answer:
<thought>I received the result</thought>
<final_answer>The task is complete.</final_answer>

Remember: Focus only on this specific subtask. Follow the skill instructions carefully."#,
            subtask.description, skill_section, tools_desc
        )
    } else {
        // Phase 1: No active skills - show skills summaries if available using SkillInjector
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
   - You can request multiple skills: <use_skill>skill-a, skill-b</use_skill>
3. Use the available tools as needed to complete the task
4. When you have completed the task, provide your final answer wrapped in <final_answer></final_answer> tags

## Response Format
- Use <thought></thought> tags to explain your reasoning
- Use <action></action> tags for tool calls with JSON format:
  <action>{{"name": "tool_name", "arguments": {{"param": "value"}}}}</action>
- When done, use <final_answer></final_answer> tags for your final response

## Tool Call Example
When you need to call a tool, output like this:
<thought>I need to search for information</thought>
<action>{{"name": "mcp__search__query", "arguments": {{"query": "example search"}}}}</action>

Remember: Focus only on this specific subtask. Use the context from previous results if needed."#,
            subtask.description, skills_section, tools_desc
        )
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
/// Internal tools (like `skill_run_script`, `skill_load_asset`) get custom parameter schemas.
fn build_tools_json(
    available_tools: &[ToolDescription],
    allowed_patterns: Option<&[String]>,
) -> serde_json::Value {
    let filtered_tools = filter_tools_by_patterns(available_tools, allowed_patterns);

    let tools: Vec<serde_json::Value> = filtered_tools
        .iter()
        .map(|tool| {
            // Check if this is an internal tool with custom schema
            let is_skill_run_script = tool.name == internal_tool_name(SKILL_RUN_SCRIPT_TOOL);
            let is_skill_load_asset = tool.name == internal_tool_name(SKILL_LOAD_ASSET_TOOL);

            let parameters = if is_skill_run_script {
                // Custom schema for skill_run_script
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "script_name": {
                            "type": "string",
                            "description": "Name of the script file to execute (e.g., 'process.js', 'export.py')"
                        },
                        "args": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Optional command line arguments to pass to the script"
                        }
                    },
                    "required": ["script_name"]
                })
            } else if is_skill_load_asset {
                // Custom schema for skill_load_asset
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "asset_name": {
                            "type": "string",
                            "description": "Name of the asset file to load (e.g., 'template.md', 'config.json')"
                        },
                        "variables": {
                            "type": "object",
                            "description": "Optional variables for template replacement. Use {{variable}} syntax in the template."
                        },
                        "parse_as": {
                            "type": "string",
                            "enum": ["json", "yaml", "markdown"],
                            "description": "Optional format to parse the content as. If not specified, returns raw content."
                        }
                    },
                    "required": ["asset_name"]
                })
            } else {
                // Default schema for MCP tools
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "The query or input for the tool"
                        }
                    },
                    "required": ["query"]
                })
            };

            serde_json::json!({
                "type": "function",
                "function": {
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": parameters
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
    original_request: &ChatCompletionRequest,
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

    let model_name = original_request.model.as_deref().unwrap_or("default");

    let summary_request = serde_json::json!({
        "model": model_name,
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
// Replanning Helper Functions (R5.2)
// ============================================================================

/// Extracts failed subtask information from the plan trace.
fn extract_failed_subtasks(trace: &PlanTrace) -> Vec<FailedSubtaskInfo> {
    use crate::chat::planner::SubTaskStatus;
    trace
        .subtask_traces
        .iter()
        .filter_map(|t| {
            if let SubTaskStatus::Failed(ref error) = t.status {
                Some(FailedSubtaskInfo {
                    id: t.subtask_id,
                    description: t.description.clone(),
                    error: error.clone(),
                    retry_count: t.retry_count,
                })
            } else {
                None
            }
        })
        .collect()
}

/// Extracts pending subtask information from the plan.
fn extract_pending_subtasks(plan: &TaskPlan, completed_ids: &HashSet<usize>) -> Vec<SubtaskInfo> {
    plan.subtasks
        .iter()
        .filter(|s| !completed_ids.contains(&s.id))
        .map(|s| SubtaskInfo {
            id: s.id,
            description: s.description.clone(),
            dependencies: s.dependencies.clone(),
        })
        .collect()
}

/// Executes the replanning process.
#[allow(clippy::too_many_arguments)]
async fn execute_replan(
    replanner: &DynamicReplanner,
    original_goal: &str,
    pending_subtasks: Vec<SubtaskInfo>,
    subtask_results: &[(usize, String)],
    trace: &PlanTrace,
    trigger: ReplanTrigger,
    remaining_time: Option<Duration>,
    request_id: &str,
) -> ServerResult<crate::reflection::ReplanResult> {
    use std::collections::HashMap;

    dual_info!(
        "🔄 Starting replan due to: {} - request_id: {}",
        trigger.description(),
        request_id
    );

    // Build completed results map
    let completed_results: HashMap<usize, String> = subtask_results.iter().cloned().collect();

    // Extract failed subtasks from trace
    let failed_subtasks = extract_failed_subtasks(trace);

    // Build replan context
    let context = ReplanContext {
        original_goal: original_goal.to_string(),
        completed_results,
        failed_subtasks,
        pending_subtasks,
        trigger,
        remaining_time,
    };

    // Execute replanning
    replanner.replan(&context).await
}

/// Applies a new plan from replanning result.
#[allow(dead_code)]
fn apply_new_plan(
    current_plan: &mut TaskPlan,
    replan_result: crate::reflection::ReplanResult,
    request_id: &str,
) {
    use crate::chat::planner::SubTaskStatus;

    dual_info!(
        "📝 Applying new plan: preserved={}, added={}, removed={} - request_id: {}",
        replan_result.preserved_subtasks.len(),
        replan_result.added_subtasks.len(),
        replan_result.removed_subtasks.len(),
        request_id
    );

    // Create new subtasks from replan result
    let mut new_subtasks: Vec<SubTask> = Vec::new();

    for new_subtask in replan_result.new_subtasks {
        let subtask = if new_subtask.preserved {
            // Find the original subtask and preserve its state
            if let Some(original_id) = new_subtask.original_id {
                if let Some(original) = current_plan.subtasks.iter().find(|s| s.id == original_id) {
                    let mut s = original.clone();
                    s.id = new_subtask.id;
                    s.dependencies = new_subtask.dependencies;
                    s
                } else {
                    SubTask {
                        id: new_subtask.id,
                        description: new_subtask.description,
                        dependencies: new_subtask.dependencies,
                        required_tools: vec![],
                        recommended_skill: None,
                        status: SubTaskStatus::Pending,
                        result: None,
                    }
                }
            } else {
                SubTask {
                    id: new_subtask.id,
                    description: new_subtask.description,
                    dependencies: new_subtask.dependencies,
                    required_tools: vec![],
                    recommended_skill: None,
                    status: SubTaskStatus::Pending,
                    result: None,
                }
            }
        } else {
            SubTask {
                id: new_subtask.id,
                description: new_subtask.description,
                dependencies: new_subtask.dependencies,
                required_tools: vec![],
                recommended_skill: None,
                status: SubTaskStatus::Pending,
                result: None,
            }
        };
        new_subtasks.push(subtask);
    }

    // Update the plan
    current_plan.subtasks = new_subtasks;

    // Recalculate execution order
    current_plan.execution_order = current_plan.subtasks.iter().map(|s| s.id).collect();

    dual_info!(
        "✅ New plan applied with {} subtasks - request_id: {}",
        current_plan.subtasks.len(),
        request_id
    );
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::planner::{SubTask, SubTaskStatus};

    /// Test helper: synchronous wrapper for build_context_for_react
    fn build_context_for_react_sync(
        subtask: &SubTask,
        previous_results: &[(usize, String)],
        available_tools: &[ToolDescription],
        skills_summaries: Option<&[SkillSummary]>,
        active_skills: &[LoadedSkill],
    ) -> Vec<ChatCompletionRequestMessage> {
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(build_context_for_react(
                subtask,
                previous_results,
                available_tools,
                skills_summaries,
                active_skills,
                0, // No reference size limit in tests
            ))
    }

    #[test]
    fn test_build_context_for_react_no_dependencies() {
        let subtask = SubTask::new(0, "Query weather".to_string());
        let previous_results: Vec<(usize, String)> = vec![];
        let available_tools = vec![ToolDescription {
            name: "weather---weather-server".to_string(),
            description: "Get weather information".to_string(),
        }];

        let messages =
            build_context_for_react_sync(&subtask, &previous_results, &available_tools, None, &[]);

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
            build_context_for_react_sync(&subtask, &previous_results, &available_tools, None, &[]);

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
            allowed_tools: vec![],
        }];

        let messages = build_context_for_react_sync(
            &subtask,
            &previous_results,
            &available_tools,
            Some(&skills),
            &[],
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
            scripts: Vec::new(),
        };

        let messages = build_context_for_react_sync(
            &subtask,
            &previous_results,
            &available_tools,
            None,
            &[active_skill],
        );

        // Should have system message + user message with task
        assert_eq!(messages.len(), 2);

        // Verify skill content is injected in system prompt
        if let ChatCompletionRequestMessage::System(sys_msg) = &messages[0] {
            let content = sys_msg.content();
            // Multi-skill format uses "Active Skills:" header
            assert!(content.contains("Active Skills: weather-query"));
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
            build_context_for_react_sync(&subtask, &previous_results, &available_tools, None, &[]);

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
            build_context_for_react_sync(&subtask, &previous_results, &available_tools, None, &[]);

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
            build_context_for_react_sync(&subtask, &previous_results, &available_tools, None, &[]);

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
                allowed_tools: vec![],
            },
            SkillSummary {
                name: "git-workflow".to_string(),
                description: "Help with git operations and workflows".to_string(),
                allowed_tools: vec![],
            },
        ];

        let messages =
            build_context_for_react_sync(&subtask, &previous_results, &tools, Some(&skills), &[]);

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

        let messages = build_context_for_react_sync(
            &subtask,
            &previous_results,
            &tools,
            Some(&empty_skills),
            &[],
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
            allowed_tools: vec![],
        }];

        // Phase 1: no active skill
        let messages =
            build_context_for_react_sync(&subtask, &previous_results, &tools, Some(&skills), &[]);

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
            scripts: Vec::new(),
        };

        // Phase 2: with active skill
        let messages = build_context_for_react_sync(
            &subtask,
            &previous_results,
            &tools,
            None, // No summaries needed in phase 2
            &[active_skill],
        );

        if let ChatCompletionRequestMessage::System(sys_msg) = &messages[0] {
            let content = sys_msg.content();
            // Should show active skills section (multi-skill format)
            assert!(content.contains("Active Skills: git-workflow"));
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
            scripts: Vec::new(),
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

        // Initially no active skills
        assert!(subtask_trace.active_skills.is_empty());

        // Add active skill
        subtask_trace.add_active_skill("git-workflow".to_string());
        assert_eq!(
            subtask_trace.active_skills,
            vec!["git-workflow".to_string()]
        );

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
            scripts: Vec::new(),
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
            build_context_for_react_sync(&subtask, &[], &all_tools, None, &[loaded_skill.clone()]);

        // Verify context includes skill (multi-skill format)
        if let ChatCompletionRequestMessage::System(sys_msg) = &messages[0] {
            assert!(sys_msg.content().contains("Active Skills: git-workflow"));
        }

        // Step 7: Record in trace
        let mut subtask_trace = SubtaskTrace::new(0, "Git task".to_string());
        subtask_trace.set_active_skills(vec!["git-workflow".to_string()]);

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

    // ==========================================================================
    // Multi-Skill Integration Tests
    // ==========================================================================

    /// Test multi-skill context building with merged tools
    #[test]
    fn test_integration_multi_skill_context() {
        use std::path::PathBuf;

        use chrono::Utc;

        let subtask = SubTask::new(0, "Review and document code".to_string());
        let previous_results: Vec<(usize, String)> = vec![];

        // Create tools
        let tools = vec![
            ToolDescription {
                name: "tool-a".to_string(),
                description: "Tool A".to_string(),
            },
            ToolDescription {
                name: "tool-b".to_string(),
                description: "Tool B".to_string(),
            },
            ToolDescription {
                name: "tool-c".to_string(),
                description: "Tool C".to_string(),
            },
        ];

        // Create two skills with different allowed tools
        let skill_a = LoadedSkill {
            metadata: crate::skills::SkillMetadata {
                name: "skill-a".to_string(),
                description: "Skill A".to_string(),
                license: None,
                compatibility: None,
                metadata: None,
                allowed_tools: Some("tool-a".to_string()),
                model: None,
            },
            content: "Skill A content".to_string(),
            raw_content: String::new(),
            skill_dir: PathBuf::new(),
            file_path: String::new(),
            enabled: true,
            loaded_at: Utc::now(),
            scripts: Vec::new(),
        };

        let skill_b = LoadedSkill {
            metadata: crate::skills::SkillMetadata {
                name: "skill-b".to_string(),
                description: "Skill B".to_string(),
                license: None,
                compatibility: None,
                metadata: None,
                allowed_tools: Some("tool-b".to_string()),
                model: None,
            },
            content: "Skill B content".to_string(),
            raw_content: String::new(),
            skill_dir: PathBuf::new(),
            file_path: String::new(),
            enabled: true,
            loaded_at: Utc::now(),
            scripts: Vec::new(),
        };

        let active_skills = vec![skill_a, skill_b];

        // Build context with multiple active skills
        let messages =
            build_context_for_react_sync(&subtask, &previous_results, &tools, None, &active_skills);

        // Verify multi-skill format in system prompt
        if let ChatCompletionRequestMessage::System(sys_msg) = &messages[0] {
            let content = sys_msg.content();
            // Should show both skills
            assert!(content.contains("Active Skills: skill-a, skill-b"));
            // Should contain both skill contents
            assert!(content.contains("Skill A content"));
            assert!(content.contains("Skill B content"));
        } else {
            panic!("Expected system message");
        }
    }

    /// Test multi-skill tool filtering (merged allowed_tools)
    #[test]
    fn test_integration_multi_skill_tool_filtering() {
        use std::path::PathBuf;

        use chrono::Utc;

        let tools = vec![
            ToolDescription {
                name: "tool-a".to_string(),
                description: "Tool A".to_string(),
            },
            ToolDescription {
                name: "tool-b".to_string(),
                description: "Tool B".to_string(),
            },
            ToolDescription {
                name: "tool-c".to_string(),
                description: "Tool C".to_string(),
            },
            ToolDescription {
                name: "tool-shared".to_string(),
                description: "Shared Tool".to_string(),
            },
        ];

        // Skill A allows tool-a and tool-shared
        let skill_a = LoadedSkill {
            metadata: crate::skills::SkillMetadata {
                name: "skill-a".to_string(),
                description: "Skill A".to_string(),
                license: None,
                compatibility: None,
                metadata: None,
                allowed_tools: Some("tool-a tool-shared".to_string()),
                model: None,
            },
            content: "Content A".to_string(),
            raw_content: String::new(),
            skill_dir: PathBuf::new(),
            file_path: String::new(),
            enabled: true,
            loaded_at: Utc::now(),
            scripts: Vec::new(),
        };

        // Skill B allows tool-b and tool-shared
        let skill_b = LoadedSkill {
            metadata: crate::skills::SkillMetadata {
                name: "skill-b".to_string(),
                description: "Skill B".to_string(),
                license: None,
                compatibility: None,
                metadata: None,
                allowed_tools: Some("tool-b tool-shared".to_string()),
                model: None,
            },
            content: "Content B".to_string(),
            raw_content: String::new(),
            skill_dir: PathBuf::new(),
            file_path: String::new(),
            enabled: true,
            loaded_at: Utc::now(),
            scripts: Vec::new(),
        };

        let active_skills = vec![skill_a, skill_b];

        // Filter with multiple skills - should get merged allowed_tools
        let filtered = filter_tools_by_skills(&tools, None, &active_skills);

        // Should have tool-a, tool-b, tool-shared (merged, deduplicated)
        assert_eq!(filtered.len(), 3);
        let names: Vec<&str> = filtered.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"tool-a"));
        assert!(names.contains(&"tool-b"));
        assert!(names.contains(&"tool-shared"));
        // tool-c should NOT be included
        assert!(!names.contains(&"tool-c"));
    }

    /// Test multi-skill tracing with set_active_skills
    #[test]
    fn test_integration_multi_skill_tracing() {
        let mut subtask_trace = SubtaskTrace::new(0, "Multi-skill task".to_string());

        // Initially empty
        assert!(subtask_trace.active_skills.is_empty());

        // Set multiple skills at once
        subtask_trace.set_active_skills(vec![
            "skill-a".to_string(),
            "skill-b".to_string(),
            "skill-c".to_string(),
        ]);

        assert_eq!(subtask_trace.active_skills.len(), 3);
        assert_eq!(
            subtask_trace.active_skills,
            vec!["skill-a", "skill-b", "skill-c"]
        );

        // Verify summary shows multi-skill format
        let summary = subtask_trace.summary();
        assert!(summary.contains("skills=[skill-a, skill-b, skill-c]"));
    }

    // ==========================================================================
    // Unit Tests: filter_tools_by_skills
    // ==========================================================================

    /// Test Phase 1 filtering: tools covered by skills are hidden
    #[test]
    fn test_filter_tools_by_skills_phase1_basic() {
        let tools = vec![
            ToolDescription {
                name: "mcp__calc__sum".to_string(),
                description: "Sum numbers".to_string(),
            },
            ToolDescription {
                name: "mcp__calc__sub".to_string(),
                description: "Subtract numbers".to_string(),
            },
            ToolDescription {
                name: "mcp__search__query".to_string(),
                description: "Search query".to_string(),
            },
        ];

        let skills = vec![SkillSummary {
            name: "calculator".to_string(),
            description: "Calculator operations".to_string(),
            allowed_tools: vec!["mcp__calc__sum".to_string(), "mcp__calc__sub".to_string()],
        }];

        // Phase 1: no active skill, with skill summaries
        let filtered = filter_tools_by_skills(&tools, Some(&skills), &[]);

        // Only search tool should remain (calc tools are covered by skill)
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name, "mcp__search__query");
    }

    /// Test Phase 1 filtering with multiple skills
    #[test]
    fn test_filter_tools_by_skills_phase1_multiple_skills() {
        let tools = vec![
            ToolDescription {
                name: "mcp__calc__sum".to_string(),
                description: "Sum".to_string(),
            },
            ToolDescription {
                name: "mcp__search__query".to_string(),
                description: "Search".to_string(),
            },
            ToolDescription {
                name: "mcp__git__status".to_string(),
                description: "Git status".to_string(),
            },
            ToolDescription {
                name: "mcp__generic__tool".to_string(),
                description: "Generic tool".to_string(),
            },
        ];

        let skills = vec![
            SkillSummary {
                name: "calculator".to_string(),
                description: "Calculator".to_string(),
                allowed_tools: vec!["mcp__calc__sum".to_string()],
            },
            SkillSummary {
                name: "search".to_string(),
                description: "Search".to_string(),
                allowed_tools: vec!["mcp__search__query".to_string()],
            },
        ];

        let filtered = filter_tools_by_skills(&tools, Some(&skills), &[]);

        // Only git and generic tools should remain
        assert_eq!(filtered.len(), 2);
        let names: Vec<&str> = filtered.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"mcp__git__status"));
        assert!(names.contains(&"mcp__generic__tool"));
    }

    /// Test Phase 1 filtering with no skill summaries
    #[test]
    fn test_filter_tools_by_skills_phase1_no_skills() {
        let tools = vec![
            ToolDescription {
                name: "tool1".to_string(),
                description: "Tool 1".to_string(),
            },
            ToolDescription {
                name: "tool2".to_string(),
                description: "Tool 2".to_string(),
            },
        ];

        // No skills = all tools shown
        let filtered = filter_tools_by_skills(&tools, None, &[]);
        assert_eq!(filtered.len(), 2);

        // Empty skills = all tools shown
        let empty_skills: Vec<SkillSummary> = vec![];
        let filtered = filter_tools_by_skills(&tools, Some(&empty_skills), &[]);
        assert_eq!(filtered.len(), 2);
    }

    /// Test Phase 1 filtering with skills having no allowed_tools
    #[test]
    fn test_filter_tools_by_skills_phase1_skills_without_allowed_tools() {
        let tools = vec![
            ToolDescription {
                name: "tool1".to_string(),
                description: "Tool 1".to_string(),
            },
            ToolDescription {
                name: "tool2".to_string(),
                description: "Tool 2".to_string(),
            },
        ];

        // Skill exists but has no allowed_tools = all tools shown
        let skills = vec![SkillSummary {
            name: "generic-skill".to_string(),
            description: "A skill without tool restrictions".to_string(),
            allowed_tools: vec![],
        }];

        let filtered = filter_tools_by_skills(&tools, Some(&skills), &[]);
        assert_eq!(filtered.len(), 2);
    }

    /// Test Phase 2 filtering: only skill's allowed tools are shown
    #[test]
    fn test_filter_tools_by_skills_phase2_basic() {
        use std::path::PathBuf;

        use chrono::Utc;

        let tools = vec![
            ToolDescription {
                name: "mcp__calc__sum".to_string(),
                description: "Sum".to_string(),
            },
            ToolDescription {
                name: "mcp__calc__sub".to_string(),
                description: "Subtract".to_string(),
            },
            ToolDescription {
                name: "mcp__search__query".to_string(),
                description: "Search".to_string(),
            },
        ];

        let active_skill = LoadedSkill {
            metadata: crate::skills::SkillMetadata {
                name: "calculator".to_string(),
                description: "Calculator".to_string(),
                license: None,
                compatibility: None,
                metadata: None,
                allowed_tools: Some("mcp__calc__sum, mcp__calc__sub".to_string()),
                model: None,
            },
            content: "Calculator instructions".to_string(),
            raw_content: String::new(),
            skill_dir: PathBuf::new(),
            file_path: String::new(),
            enabled: true,
            loaded_at: Utc::now(),
            scripts: Vec::new(),
        };

        // Phase 2: active skill present
        let filtered = filter_tools_by_skills(&tools, None, &[active_skill]);

        // Only calc tools should be shown
        assert_eq!(filtered.len(), 2);
        let names: Vec<&str> = filtered.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"mcp__calc__sum"));
        assert!(names.contains(&"mcp__calc__sub"));
        assert!(!names.contains(&"mcp__search__query"));
    }

    /// Test Phase 2 filtering: skill without allowed_tools shows all tools
    #[test]
    fn test_filter_tools_by_skills_phase2_no_restrictions() {
        use std::path::PathBuf;

        use chrono::Utc;

        let tools = vec![
            ToolDescription {
                name: "tool1".to_string(),
                description: "Tool 1".to_string(),
            },
            ToolDescription {
                name: "tool2".to_string(),
                description: "Tool 2".to_string(),
            },
        ];

        let active_skill = LoadedSkill {
            metadata: crate::skills::SkillMetadata {
                name: "generic-skill".to_string(),
                description: "A skill without tool restrictions".to_string(),
                license: None,
                compatibility: None,
                metadata: None,
                allowed_tools: None, // No restrictions
                model: None,
            },
            content: "Skill content".to_string(),
            raw_content: String::new(),
            skill_dir: PathBuf::new(),
            file_path: String::new(),
            enabled: true,
            loaded_at: Utc::now(),
            scripts: Vec::new(),
        };

        // Phase 2 with no restrictions = all tools shown
        let filtered = filter_tools_by_skills(&tools, None, &[active_skill]);
        assert_eq!(filtered.len(), 2);
    }

    /// Test Phase 2 takes precedence over Phase 1
    #[test]
    fn test_filter_tools_by_skills_phase2_overrides_phase1() {
        use std::path::PathBuf;

        use chrono::Utc;

        let tools = vec![
            ToolDescription {
                name: "mcp__calc__sum".to_string(),
                description: "Sum".to_string(),
            },
            ToolDescription {
                name: "mcp__search__query".to_string(),
                description: "Search".to_string(),
            },
        ];

        // Even if skill summaries say to hide calc tools...
        let skills = vec![SkillSummary {
            name: "calculator".to_string(),
            description: "Calculator".to_string(),
            allowed_tools: vec!["mcp__calc__sum".to_string()],
        }];

        // ...when active skill is present, Phase 2 logic applies
        let active_skill = LoadedSkill {
            metadata: crate::skills::SkillMetadata {
                name: "calculator".to_string(),
                description: "Calculator".to_string(),
                license: None,
                compatibility: None,
                metadata: None,
                allowed_tools: Some("mcp__calc__sum".to_string()),
                model: None,
            },
            content: "Calculator".to_string(),
            raw_content: String::new(),
            skill_dir: PathBuf::new(),
            file_path: String::new(),
            enabled: true,
            loaded_at: Utc::now(),
            scripts: Vec::new(),
        };

        // Phase 2: only show skill's allowed tools
        let filtered = filter_tools_by_skills(&tools, Some(&skills), &[active_skill]);

        // Only calc tool should be shown (Phase 2 filtering)
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name, "mcp__calc__sum");
    }

    /// Test filter_tools_by_skills with empty tools list
    #[test]
    fn test_filter_tools_by_skills_empty_tools() {
        let tools: Vec<ToolDescription> = vec![];

        let skills = vec![SkillSummary {
            name: "skill".to_string(),
            description: "A skill".to_string(),
            allowed_tools: vec!["some_tool".to_string()],
        }];

        let filtered = filter_tools_by_skills(&tools, Some(&skills), &[]);
        assert!(filtered.is_empty());
    }

    // ============================================================================
    // Template Variable Replacement Tests
    // ============================================================================

    #[test]
    fn test_apply_template_variables_string() {
        let mut vars = serde_json::Map::new();
        vars.insert("name".to_string(), serde_json::json!("John"));
        vars.insert("age".to_string(), serde_json::json!(30));

        let content = "Hello, {{name}}! You are {{age}} years old.";
        let result = apply_template_variables(content, &vars);

        assert_eq!(result, "Hello, John! You are 30 years old.");
    }

    #[test]
    fn test_apply_template_variables_multiple_same_var() {
        let mut vars = serde_json::Map::new();
        vars.insert("item".to_string(), serde_json::json!("apple"));

        let content = "Buy {{item}}, eat {{item}}, enjoy {{item}}.";
        let result = apply_template_variables(content, &vars);

        assert_eq!(result, "Buy apple, eat apple, enjoy apple.");
    }

    #[test]
    fn test_apply_template_variables_missing_var() {
        let vars = serde_json::Map::new();

        let content = "Hello, {{name}}!";
        let result = apply_template_variables(content, &vars);

        // Missing variables remain unchanged
        assert_eq!(result, "Hello, {{name}}!");
    }

    #[test]
    fn test_apply_template_variables_bool_and_null() {
        let mut vars = serde_json::Map::new();
        vars.insert("active".to_string(), serde_json::json!(true));
        vars.insert("empty".to_string(), serde_json::json!(null));

        let content = "Active: {{active}}, Empty: {{empty}}";
        let result = apply_template_variables(content, &vars);

        assert_eq!(result, "Active: true, Empty: null");
    }

    #[test]
    fn test_apply_template_variables_array_object() {
        let mut vars = serde_json::Map::new();
        vars.insert("list".to_string(), serde_json::json!([1, 2, 3]));
        vars.insert("obj".to_string(), serde_json::json!({"key": "value"}));

        let content = "List: {{list}}, Obj: {{obj}}";
        let result = apply_template_variables(content, &vars);

        assert!(result.contains("[1,2,3]"));
        assert!(result.contains(r#"{"key":"value"}"#));
    }

    #[test]
    fn test_apply_template_variables_empty_content() {
        let mut vars = serde_json::Map::new();
        vars.insert("name".to_string(), serde_json::json!("John"));

        let content = "";
        let result = apply_template_variables(content, &vars);

        assert_eq!(result, "");
    }

    // ============================================================================
    // JSON/YAML Parsing Tests
    // ============================================================================

    #[test]
    fn test_parse_as_json_valid() {
        let content = r#"{"name": "test", "value": 42}"#;
        let result = parse_as_json(content, "config.json").unwrap();

        assert!(result.contains("config.json"));
        assert!(result.contains("parsed as JSON"));
        assert!(result.contains("\"name\": \"test\""));
        assert!(result.contains("\"value\": 42"));
    }

    #[test]
    fn test_parse_as_json_invalid() {
        let content = "not valid json {";
        let result = parse_as_json(content, "bad.json");

        assert!(result.is_err());
        if let Err(e) = result {
            assert!(e.to_string().contains("Failed to parse"));
        }
    }

    #[test]
    fn test_parse_as_yaml_valid() {
        let content = "name: test\nvalue: 42";
        let result = parse_as_yaml(content, "config.yaml").unwrap();

        assert!(result.contains("config.yaml"));
        assert!(result.contains("parsed as YAML"));
        assert!(result.contains("name:"));
        assert!(result.contains("test"));
    }

    #[test]
    fn test_parse_as_yaml_invalid() {
        let content = ":\n  invalid: [unclosed";
        let result = parse_as_yaml(content, "bad.yaml");

        assert!(result.is_err());
        if let Err(e) = result {
            assert!(e.to_string().contains("Failed to parse"));
        }
    }

    #[test]
    fn test_format_as_markdown() {
        let content = "# Title\n\nSome content here.";
        let result = format_as_markdown(content, "doc.md");

        assert!(result.contains("doc.md"));
        assert!(result.contains("loaded as Markdown"));
        assert!(result.contains("# Title"));
        assert!(result.contains("Some content here."));
    }

    // ============================================================================
    // Internal Tool Name Tests
    // ============================================================================

    #[test]
    fn test_internal_tool_names() {
        assert_eq!(
            internal_tool_name(SKILL_RUN_SCRIPT_TOOL),
            "internal__skill_run_script"
        );
        assert_eq!(
            internal_tool_name(SKILL_LOAD_ASSET_TOOL),
            "internal__skill_load_asset"
        );
    }

    #[test]
    fn test_is_internal_tool() {
        assert!(is_internal_tool("internal__skill_run_script"));
        assert!(is_internal_tool("internal__skill_load_asset"));
        assert!(is_internal_tool("internal__any_tool"));
        assert!(!is_internal_tool("mcp__server__tool"));
        assert!(!is_internal_tool("some_tool"));
    }

    #[test]
    fn test_parse_internal_tool_name() {
        assert_eq!(
            parse_internal_tool_name("internal__skill_run_script"),
            Some("skill_run_script")
        );
        assert_eq!(
            parse_internal_tool_name("internal__skill_load_asset"),
            Some("skill_load_asset")
        );
        assert_eq!(parse_internal_tool_name("mcp__server__tool"), None);
    }
}
