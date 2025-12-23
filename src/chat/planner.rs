//! Task planning module for Plan mode.
//!
//! This module provides the task planning functionality that decomposes
//! user requests into structured subtasks with dependencies.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::Arc,
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::ServerError;

// ============================================================================
// Data Structures
// ============================================================================

/// A complete task plan containing subtasks and their execution order.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskPlan {
    /// Unique identifier for this plan.
    pub plan_id: String,
    /// The original user goal/request.
    pub original_goal: String,
    /// List of subtasks to execute.
    pub subtasks: Vec<SubTask>,
    /// Execution order (indices into subtasks vec, topologically sorted).
    pub execution_order: Vec<usize>,
}

impl TaskPlan {
    /// Creates a new TaskPlan with the given goal and subtasks.
    ///
    /// This will automatically compute the execution order via topological sort.
    pub fn new(goal: String, subtasks: Vec<SubTask>) -> Result<Self, ServerError> {
        let plan_id = format!("plan-{}", uuid::Uuid::new_v4());
        let execution_order = compute_execution_order(&subtasks)?;

        Ok(Self {
            plan_id,
            original_goal: goal,
            subtasks,
            execution_order,
        })
    }

    /// Returns the number of subtasks in this plan.
    pub fn len(&self) -> usize {
        self.subtasks.len()
    }

    /// Returns true if the plan has no subtasks.
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.subtasks.is_empty()
    }

    /// Gets a subtask by its ID.
    #[allow(dead_code)]
    pub fn get_subtask(&self, id: usize) -> Option<&SubTask> {
        self.subtasks.iter().find(|s| s.id == id)
    }

    /// Gets a mutable reference to a subtask by its ID.
    #[allow(dead_code)]
    pub fn get_subtask_mut(&mut self, id: usize) -> Option<&mut SubTask> {
        self.subtasks.iter_mut().find(|s| s.id == id)
    }

    /// Returns an iterator over subtasks in execution order.
    #[allow(dead_code)]
    pub fn iter_in_order(&self) -> impl Iterator<Item = &SubTask> {
        self.execution_order
            .iter()
            .filter_map(|&idx| self.subtasks.get(idx))
    }
}

/// A single subtask within a task plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubTask {
    /// Unique identifier for this subtask (0-indexed).
    pub id: usize,
    /// Human-readable description of what this subtask does.
    pub description: String,
    /// IDs of subtasks that must complete before this one.
    pub dependencies: Vec<usize>,
    /// Names of tools that may be needed for this subtask.
    pub required_tools: Vec<String>,
    /// Current execution status.
    pub status: SubTaskStatus,
    /// Result of execution, if completed.
    pub result: Option<String>,
}

impl SubTask {
    /// Creates a new subtask with Pending status.
    pub fn new(id: usize, description: String) -> Self {
        Self {
            id,
            description,
            dependencies: vec![],
            required_tools: vec![],
            status: SubTaskStatus::Pending,
            result: None,
        }
    }

    /// Sets the dependencies for this subtask.
    pub fn with_dependencies(mut self, deps: Vec<usize>) -> Self {
        self.dependencies = deps;
        self
    }

    /// Sets the required tools for this subtask.
    pub fn with_tools(mut self, tools: Vec<String>) -> Self {
        self.required_tools = tools;
        self
    }

    /// Returns true if this subtask has no unmet dependencies.
    pub fn is_ready(&self, completed: &HashSet<usize>) -> bool {
        self.dependencies.iter().all(|dep| completed.contains(dep))
    }

    /// Marks this subtask as in progress.
    pub fn start(&mut self) {
        self.status = SubTaskStatus::InProgress;
    }

    /// Marks this subtask as completed with a result.
    pub fn complete(&mut self, result: String) {
        self.status = SubTaskStatus::Completed;
        self.result = Some(result);
    }

    /// Marks this subtask as failed with an error message.
    pub fn fail(&mut self, error: String) {
        self.status = SubTaskStatus::Failed(error);
    }

    /// Marks this subtask as skipped.
    pub fn skip(&mut self) {
        self.status = SubTaskStatus::Skipped;
    }
}

/// Status of a subtask's execution.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SubTaskStatus {
    /// Waiting to be executed.
    Pending,
    /// Currently being executed.
    InProgress,
    /// Successfully completed.
    Completed,
    /// Failed with an error message.
    Failed(String),
    /// Skipped (e.g., due to dependency failure).
    Skipped,
}

impl SubTaskStatus {
    /// Returns true if the status represents a terminal state.
    #[allow(dead_code)]
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            SubTaskStatus::Completed | SubTaskStatus::Failed(_) | SubTaskStatus::Skipped
        )
    }

    /// Returns true if the subtask completed successfully.
    #[allow(dead_code)]
    pub fn is_success(&self) -> bool {
        matches!(self, SubTaskStatus::Completed)
    }
}

// ============================================================================
// LLM Provider Abstraction
// ============================================================================

/// Message structure for LLM communication.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannerMessage {
    pub role: String,
    pub content: String,
}

impl PlannerMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".to_string(),
            content: content.into(),
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".to_string(),
            content: content.into(),
        }
    }

    #[allow(dead_code)]
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: "assistant".to_string(),
            content: content.into(),
        }
    }
}

/// Trait for LLM service providers.
///
/// This abstraction allows the TaskPlanner to work with different LLM backends,
/// defaulting to the Chat LLM but allowing for future dedicated planner LLMs.
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Sends messages to the LLM and returns the response content.
    async fn complete(&self, messages: Vec<PlannerMessage>) -> Result<String, ServerError>;

    /// Returns the name of this provider (for logging).
    fn name(&self) -> &str;
}

/// Default LLM provider that uses the Chat service.
pub struct ChatLlmProvider {
    /// URL of the chat service.
    chat_url: String,
    /// Optional API key for authentication.
    api_key: Option<String>,
    /// HTTP client for making requests.
    client: reqwest::Client,
}

impl ChatLlmProvider {
    /// Creates a new ChatLlmProvider.
    pub fn new(chat_url: String, api_key: Option<String>) -> Self {
        Self {
            chat_url,
            api_key,
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl LlmProvider for ChatLlmProvider {
    async fn complete(&self, messages: Vec<PlannerMessage>) -> Result<String, ServerError> {
        // Build request body
        let body = serde_json::json!({
            "model": "default",
            "messages": messages,
            "temperature": 0.7,
            "stream": false
        });

        // Build request
        let mut request = self.client.post(&self.chat_url).json(&body);

        if let Some(ref key) = self.api_key {
            request = request.header("Authorization", format!("Bearer {}", key));
        }

        // Send request
        let response = request.send().await.map_err(|e| {
            ServerError::Operation(format!("Failed to send planning request: {}", e))
        })?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(ServerError::Operation(format!(
                "Planning request failed with status {}: {}",
                status, text
            )));
        }

        // Parse response
        let json: serde_json::Value = response.json().await.map_err(|e| {
            ServerError::Operation(format!("Failed to parse planning response: {}", e))
        })?;

        // Extract content from OpenAI-compatible response format
        json["choices"][0]["message"]["content"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| ServerError::Operation("Invalid response format from LLM".to_string()))
    }

    fn name(&self) -> &str {
        "chat"
    }
}

// ============================================================================
// Task Planner
// ============================================================================

/// Tool description for the planner prompt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDescription {
    /// Name of the tool.
    pub name: String,
    /// Description of what the tool does.
    pub description: String,
}

/// The main task planner that generates execution plans from user requests.
pub struct TaskPlanner {
    /// LLM provider for generating plans.
    llm_provider: Arc<dyn LlmProvider>,
    /// Available tools that can be used in plans.
    available_tools: Vec<ToolDescription>,
    /// Maximum number of subtasks allowed.
    max_subtasks: usize,
}

impl TaskPlanner {
    /// Creates a TaskPlanner using the Chat LLM as the provider.
    pub fn with_chat_llm(chat_url: String, api_key: Option<String>, max_subtasks: usize) -> Self {
        Self {
            llm_provider: Arc::new(ChatLlmProvider::new(chat_url, api_key)),
            available_tools: vec![],
            max_subtasks,
        }
    }

    /// Creates a TaskPlanner with a custom LLM provider.
    #[allow(dead_code)]
    pub fn with_provider(provider: Arc<dyn LlmProvider>, max_subtasks: usize) -> Self {
        Self {
            llm_provider: provider,
            available_tools: vec![],
            max_subtasks,
        }
    }

    /// Sets the available tools for planning.
    pub fn with_tools(mut self, tools: Vec<ToolDescription>) -> Self {
        self.available_tools = tools;
        self
    }

    /// Generates a task plan for the given user request.
    pub async fn plan(&self, user_request: &str) -> Result<TaskPlan, ServerError> {
        // Build the planning prompt
        let system_prompt = self.build_system_prompt();
        let user_prompt = self.build_user_prompt(user_request);

        let messages = vec![
            PlannerMessage::system(system_prompt),
            PlannerMessage::user(user_prompt),
        ];

        // Get LLM response
        tracing::debug!(
            provider = self.llm_provider.name(),
            "Requesting task plan from LLM"
        );

        let response = self.llm_provider.complete(messages).await?;

        // Parse the response into a TaskPlan
        let raw_plan = crate::chat::xml_parser::extract_task_plan(&response).ok_or_else(|| {
            if crate::chat::xml_parser::has_task_plan_tag(&response) {
                ServerError::PlanParseError(
                    "Found <task_plan> tag but failed to parse its content".to_string(),
                )
            } else {
                ServerError::PlanParseError(
                    "LLM response does not contain a valid <task_plan> tag".to_string(),
                )
            }
        })?;

        // Validate and convert to TaskPlan
        self.validate_and_build_plan(raw_plan)
    }

    /// Builds the system prompt for the planner.
    fn build_system_prompt(&self) -> String {
        let tools_desc = if self.available_tools.is_empty() {
            "No tools are currently available.".to_string()
        } else {
            self.available_tools
                .iter()
                .map(|t| format!("- {}: {}", t.name, t.description))
                .collect::<Vec<_>>()
                .join("\n")
        };

        format!(
            r#"你是一个任务规划专家。请分析用户的请求，将其分解为可执行的子任务。

## 可用工具
{tools_desc}

## 输出格式
请使用以下 XML 格式输出任务计划：

<task_plan>
  <goal>用户的最终目标描述</goal>
  <subtasks>
    <subtask id="1">
      <description>子任务描述</description>
      <dependencies></dependencies>
      <tools>可能需要的工具名称</tools>
    </subtask>
    <subtask id="2">
      <description>子任务描述</description>
      <dependencies>1</dependencies>
      <tools>可能需要的工具名称</tools>
    </subtask>
  </subtasks>
</task_plan>

## 规划原则
1. 每个子任务应该是原子性的、可独立执行的
2. 每个子任务对应一次工具调用（细粒度规划）
3. 明确标注子任务之间的依赖关系
4. 依赖关系中的 ID 必须是已定义的子任务 ID
5. 如果任务简单，可以只有一个子任务
6. 子任务数量不应超过 {max_subtasks} 个"#,
            tools_desc = tools_desc,
            max_subtasks = self.max_subtasks
        )
    }

    /// Builds the user prompt with the request.
    fn build_user_prompt(&self, user_request: &str) -> String {
        format!("请为以下用户请求生成任务计划：\n\n{}", user_request)
    }

    /// Validates a raw plan and builds a TaskPlan.
    fn validate_and_build_plan(
        &self,
        raw: crate::chat::xml_parser::TaskPlanRaw,
    ) -> Result<TaskPlan, ServerError> {
        // Check for empty plan
        if raw.subtasks.is_empty() {
            return Err(ServerError::EmptyPlan);
        }

        // Check subtask count
        if raw.subtasks.len() > self.max_subtasks {
            return Err(ServerError::PlanParseError(format!(
                "Plan has {} subtasks, exceeding maximum of {}",
                raw.subtasks.len(),
                self.max_subtasks
            )));
        }

        // Build subtasks and validate references
        let mut subtasks = Vec::new();
        let mut id_set: HashSet<usize> = HashSet::new();

        for raw_subtask in &raw.subtasks {
            // Parse ID
            let id: usize = raw_subtask.id.parse().map_err(|_| {
                ServerError::PlanParseError(format!("Invalid subtask ID: '{}'", raw_subtask.id))
            })?;

            if !id_set.insert(id) {
                return Err(ServerError::PlanParseError(format!(
                    "Duplicate subtask ID: {}",
                    id
                )));
            }

            // Parse dependencies
            let mut dependencies = Vec::new();
            for dep_str in &raw_subtask.dependencies {
                let dep_id: usize = dep_str.parse().map_err(|_| {
                    ServerError::InvalidReference(format!(
                        "Invalid dependency ID '{}' in subtask {}",
                        dep_str, id
                    ))
                })?;
                dependencies.push(dep_id);
            }

            let subtask = SubTask::new(id, raw_subtask.description.clone())
                .with_dependencies(dependencies)
                .with_tools(raw_subtask.tools.clone());

            subtasks.push(subtask);
        }

        // Validate all dependency references exist
        for subtask in &subtasks {
            for dep_id in &subtask.dependencies {
                if !id_set.contains(dep_id) {
                    return Err(ServerError::InvalidReference(format!(
                        "Subtask {} references non-existent dependency {}",
                        subtask.id, dep_id
                    )));
                }
            }
        }

        // Build the plan (this will also check for cycles via topological sort)
        TaskPlan::new(raw.goal, subtasks)
    }
}

// ============================================================================
// Topological Sort / Cycle Detection
// ============================================================================

/// Computes the execution order of subtasks using topological sort.
///
/// Returns an error if a cyclic dependency is detected.
fn compute_execution_order(subtasks: &[SubTask]) -> Result<Vec<usize>, ServerError> {
    if subtasks.is_empty() {
        return Ok(vec![]);
    }

    // Build adjacency list and in-degree map
    let mut in_degree: HashMap<usize, usize> = HashMap::new();
    let mut adj: HashMap<usize, Vec<usize>> = HashMap::new();

    // Initialize
    for subtask in subtasks {
        in_degree.entry(subtask.id).or_insert(0);
        adj.entry(subtask.id).or_default();
    }

    // Build graph
    for subtask in subtasks {
        for &dep_id in &subtask.dependencies {
            adj.entry(dep_id).or_default().push(subtask.id);
            *in_degree.entry(subtask.id).or_insert(0) += 1;
        }
    }

    // Kahn's algorithm for topological sort
    let mut queue: VecDeque<usize> = VecDeque::new();
    for (&id, &degree) in &in_degree {
        if degree == 0 {
            queue.push_back(id);
        }
    }

    let mut order = Vec::new();
    while let Some(id) = queue.pop_front() {
        order.push(id);

        if let Some(neighbors) = adj.get(&id) {
            for &neighbor in neighbors {
                if let Some(degree) = in_degree.get_mut(&neighbor) {
                    *degree -= 1;
                    if *degree == 0 {
                        queue.push_back(neighbor);
                    }
                }
            }
        }
    }

    // Check for cycle
    if order.len() != subtasks.len() {
        return Err(ServerError::CyclicDependency);
    }

    // Convert to indices
    let id_to_idx: HashMap<usize, usize> = subtasks
        .iter()
        .enumerate()
        .map(|(idx, s)| (s.id, idx))
        .collect();

    let execution_order = order
        .into_iter()
        .filter_map(|id| id_to_idx.get(&id).copied())
        .collect();

    Ok(execution_order)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_subtask_creation() {
        let subtask = SubTask::new(1, "Test task".to_string())
            .with_dependencies(vec![0])
            .with_tools(vec!["search".to_string()]);

        assert_eq!(subtask.id, 1);
        assert_eq!(subtask.description, "Test task");
        assert_eq!(subtask.dependencies, vec![0]);
        assert_eq!(subtask.required_tools, vec!["search".to_string()]);
        assert_eq!(subtask.status, SubTaskStatus::Pending);
    }

    #[test]
    fn test_subtask_status_transitions() {
        let mut subtask = SubTask::new(0, "Test".to_string());
        assert_eq!(subtask.status, SubTaskStatus::Pending);

        subtask.start();
        assert_eq!(subtask.status, SubTaskStatus::InProgress);

        subtask.complete("Done".to_string());
        assert_eq!(subtask.status, SubTaskStatus::Completed);
        assert_eq!(subtask.result, Some("Done".to_string()));
    }

    #[test]
    fn test_subtask_is_ready() {
        let subtask = SubTask::new(2, "Test".to_string()).with_dependencies(vec![0, 1]);

        let mut completed = HashSet::new();
        assert!(!subtask.is_ready(&completed));

        completed.insert(0);
        assert!(!subtask.is_ready(&completed));

        completed.insert(1);
        assert!(subtask.is_ready(&completed));
    }

    #[test]
    fn test_task_plan_simple() {
        let subtasks = vec![
            SubTask::new(0, "First task".to_string()),
            SubTask::new(1, "Second task".to_string()).with_dependencies(vec![0]),
        ];

        let plan = TaskPlan::new("Test goal".to_string(), subtasks).unwrap();
        assert_eq!(plan.len(), 2);
        assert_eq!(plan.execution_order.len(), 2);
    }

    #[test]
    fn test_topological_sort_linear() {
        let subtasks = vec![
            SubTask::new(0, "A".to_string()),
            SubTask::new(1, "B".to_string()).with_dependencies(vec![0]),
            SubTask::new(2, "C".to_string()).with_dependencies(vec![1]),
        ];

        let order = compute_execution_order(&subtasks).unwrap();
        // Should execute in order: 0 -> 1 -> 2
        assert_eq!(order.len(), 3);
    }

    #[test]
    fn test_topological_sort_parallel() {
        let subtasks = vec![
            SubTask::new(0, "A".to_string()),
            SubTask::new(1, "B".to_string()),
            SubTask::new(2, "C".to_string()).with_dependencies(vec![0, 1]),
        ];

        let order = compute_execution_order(&subtasks).unwrap();
        assert_eq!(order.len(), 3);
        // C (index 2) should come after A and B
    }

    #[test]
    fn test_cyclic_dependency_detection() {
        let subtasks = vec![
            SubTask::new(0, "A".to_string()).with_dependencies(vec![1]),
            SubTask::new(1, "B".to_string()).with_dependencies(vec![0]),
        ];

        let result = compute_execution_order(&subtasks);
        assert!(matches!(result, Err(ServerError::CyclicDependency)));
    }

    #[test]
    fn test_task_plan_iteration() {
        let subtasks = vec![
            SubTask::new(0, "First".to_string()),
            SubTask::new(1, "Second".to_string()).with_dependencies(vec![0]),
        ];

        let plan = TaskPlan::new("Goal".to_string(), subtasks).unwrap();
        let descriptions: Vec<_> = plan.iter_in_order().map(|s| &s.description).collect();

        assert_eq!(descriptions.len(), 2);
        assert_eq!(descriptions[0], "First");
        assert_eq!(descriptions[1], "Second");
    }

    #[test]
    fn test_status_is_terminal() {
        assert!(!SubTaskStatus::Pending.is_terminal());
        assert!(!SubTaskStatus::InProgress.is_terminal());
        assert!(SubTaskStatus::Completed.is_terminal());
        assert!(SubTaskStatus::Failed("error".to_string()).is_terminal());
        assert!(SubTaskStatus::Skipped.is_terminal());
    }
}
