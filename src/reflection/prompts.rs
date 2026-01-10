//! Reflection prompt templates for the reflection system.
//!
//! This module provides prompt templates used by the reflection engine
//! to evaluate task execution results.

// Some prompts are reserved for plan-level reflection
#![allow(dead_code)]

use super::types::ReflectionContext;

/// Subtask reflection prompt template.
///
/// This prompt is used for initial evaluation of a subtask result,
/// assessing completeness, correctness, efficiency, and potential risks.
pub const SUBTASK_REFLECTION_PROMPT: &str = r#"You are a reflection agent tasked with evaluating the quality of a task execution result.

## Original Task
Description: {task_description}
Dependencies completed: {dependencies_results}

## Execution Result
{result}

## Execution Trace
- Iterations: {iteration_count}
- Tool calls: {tool_calls}
- Errors encountered: {errors}
- Time taken: {time_taken}

## Your Task
Analyze the execution result and provide a structured reflection:

1. **Completeness**: Does the result fully address the task requirements?
2. **Correctness**: Is the result factually and logically correct?
3. **Efficiency**: Was the execution path optimal?
4. **Risks**: Are there any potential issues or risks?

Respond in the following JSON format:
```json
{
  "passed": true/false,
  "confidence": 0.0-1.0,
  "issues": [
    {
      "issue_type": "incomplete_result|incorrect_result|requirement_not_met|inefficient_path|potential_risk|format_error|logic_error",
      "description": "...",
      "severity": 1-5,
      "context": "optional context"
    }
  ],
  "suggestions": ["suggestion1", "suggestion2"],
  "recommended_action": {
    "type": "Accept|AcceptWithFix|Retry|RetryWithStrategy|Replan|RequestClarification|Abort",
    "details": "..."
  }
}
```"#;

/// Deep reflection prompt template.
///
/// This prompt is used for deeper multi-round analysis when
/// the initial reflection confidence is below the threshold.
pub const DEEP_REFLECTION_PROMPT: &str = r#"You are performing a deeper reflection on a task result that did not pass initial validation.

## Initial Reflection
{initial_reflection}

## Task Context
{task_context}

## Current Result
{result}

## Reflection Round: {round}

Please provide a more thorough analysis:
1. Why might the initial concerns be valid or invalid?
2. What specific changes would address the identified issues?
3. Is the result acceptable with minor fixes, or does it require a complete redo?

Provide your updated reflection in JSON format:
```json
{
  "passed": true/false,
  "confidence": 0.0-1.0,
  "issues": [...],
  "suggestions": [...],
  "recommended_action": {
    "type": "Accept|AcceptWithFix|Retry|RetryWithStrategy|Replan|RequestClarification|Abort",
    "details": "..."
  }
}
```"#;

/// Plan-level reflection prompt template.
///
/// This prompt is used for evaluating the overall plan execution results.
pub const PLAN_REFLECTION_PROMPT: &str = r#"You are evaluating the overall execution of a task plan.

## Original Goal
{original_goal}

## Plan Summary
Total subtasks: {total_subtasks}
Completed: {completed_count}
Failed: {failed_count}

## Subtask Results
{subtask_results}

## Your Task
Evaluate whether the original goal was achieved:

1. **Goal Achievement**: Did the completed subtasks achieve the original goal?
2. **Result Quality**: Is the overall quality acceptable?
3. **Missing Elements**: Are there any missing elements that need to be addressed?

Respond in JSON format:
```json
{
  "passed": true/false,
  "confidence": 0.0-1.0,
  "issues": [...],
  "suggestions": [...],
  "recommended_action": {
    "type": "Accept|AcceptWithFix|Retry|RetryWithStrategy|Replan|RequestClarification|Abort",
    "details": "..."
  }
}
```"#;

/// Builds the subtask reflection prompt with context.
pub fn build_subtask_reflection_prompt(result: &str, context: &ReflectionContext) -> String {
    let dependencies_str = if context.dependencies_results.is_empty() {
        "None".to_string()
    } else {
        context.dependencies_results.join("\n")
    };

    let tool_calls_str = if context.tool_calls.is_empty() {
        "None".to_string()
    } else {
        context.tool_calls.join(", ")
    };

    let errors_str = if context.errors.is_empty() {
        "None".to_string()
    } else {
        context.errors.join(", ")
    };

    let time_taken = format!("{}ms", context.time_taken_ms);

    SUBTASK_REFLECTION_PROMPT
        .replace("{task_description}", &context.task_description)
        .replace("{dependencies_results}", &dependencies_str)
        .replace("{result}", result)
        .replace("{iteration_count}", &context.iteration_count.to_string())
        .replace("{tool_calls}", &tool_calls_str)
        .replace("{errors}", &errors_str)
        .replace("{time_taken}", &time_taken)
}

/// Builds the deep reflection prompt.
pub fn build_deep_reflection_prompt(
    result: &str,
    initial_reflection: &str,
    task_context: &str,
    round: u32,
) -> String {
    DEEP_REFLECTION_PROMPT
        .replace("{initial_reflection}", initial_reflection)
        .replace("{task_context}", task_context)
        .replace("{result}", result)
        .replace("{round}", &round.to_string())
}

/// Builds the plan reflection prompt.
pub fn build_plan_reflection_prompt(
    original_goal: &str,
    total_subtasks: usize,
    completed_count: usize,
    failed_count: usize,
    subtask_results: &str,
) -> String {
    PLAN_REFLECTION_PROMPT
        .replace("{original_goal}", original_goal)
        .replace("{total_subtasks}", &total_subtasks.to_string())
        .replace("{completed_count}", &completed_count.to_string())
        .replace("{failed_count}", &failed_count.to_string())
        .replace("{subtask_results}", subtask_results)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_subtask_reflection_prompt() {
        let context = ReflectionContext::new("Calculate fibonacci(10)")
            .with_dependencies(vec!["n = 10".to_string()])
            .with_iterations(2)
            .with_tool_calls(vec!["calculate".to_string()])
            .with_time_taken(150);

        let prompt = build_subtask_reflection_prompt("Result: 55", &context);

        assert!(prompt.contains("Calculate fibonacci(10)"));
        assert!(prompt.contains("n = 10"));
        assert!(prompt.contains("Result: 55"));
        assert!(prompt.contains("Iterations: 2"));
        assert!(prompt.contains("calculate"));
        assert!(prompt.contains("150ms"));
    }

    #[test]
    fn test_build_subtask_reflection_prompt_empty_context() {
        let context = ReflectionContext::new("Simple task");

        let prompt = build_subtask_reflection_prompt("Done", &context);

        assert!(prompt.contains("Simple task"));
        assert!(prompt.contains("Dependencies completed: None"));
        assert!(prompt.contains("Tool calls: None"));
        assert!(prompt.contains("Errors encountered: None"));
    }

    #[test]
    fn test_build_deep_reflection_prompt() {
        let prompt = build_deep_reflection_prompt(
            "Current result",
            "Initial reflection: low confidence",
            "Task context here",
            2,
        );

        assert!(prompt.contains("Current result"));
        assert!(prompt.contains("Initial reflection: low confidence"));
        assert!(prompt.contains("Task context here"));
        assert!(prompt.contains("Reflection Round: 2"));
    }

    #[test]
    fn test_build_plan_reflection_prompt() {
        let prompt = build_plan_reflection_prompt(
            "Build a calculator",
            5,
            4,
            1,
            "1. Done\n2. Done\n3. Done\n4. Done\n5. Failed",
        );

        assert!(prompt.contains("Build a calculator"));
        assert!(prompt.contains("Total subtasks: 5"));
        assert!(prompt.contains("Completed: 4"));
        assert!(prompt.contains("Failed: 1"));
        assert!(prompt.contains("5. Failed"));
    }
}
