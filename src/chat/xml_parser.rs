//! XML tag parsing utilities for React mode and Plan mode.
//!
//! This module provides robust parsing of XML-like tags from LLM responses,
//! with support for format variations and automatic repair of common issues.

use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};

/// Regex patterns for flexible XML tag matching.
/// These patterns support:
/// - Case-insensitive matching
/// - Optional whitespace within tags
/// - Multiline content
static THOUGHT_PATTERN: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?si)<\s*thought\s*>(.*?)<\s*/\s*thought\s*>").unwrap());

static ACTION_PATTERN: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?si)<\s*action\s*>(.*?)<\s*/\s*action\s*>").unwrap());

static FINAL_ANSWER_PATTERN: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?si)<\s*final_answer\s*>(.*?)<\s*/\s*final_answer\s*>").unwrap());

/// Extracts content from a `<thought>` tag.
///
/// Supports format variations like:
/// - `<thought>content</thought>`
/// - `< thought >content</ thought >`
/// - `<THOUGHT>content</THOUGHT>`
pub fn extract_thought(content: &str) -> Option<String> {
    // First try sanitized content
    let sanitized = sanitize_xml_content(content);
    THOUGHT_PATTERN
        .captures(&sanitized)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().trim().to_string())
}

/// Extracts content from an `<action>` tag.
///
/// Supports format variations like:
/// - `<action>content</action>`
/// - `< action >content</ action >`
/// - `<ACTION>content</ACTION>`
pub fn extract_action(content: &str) -> Option<String> {
    // First try sanitized content
    let sanitized = sanitize_xml_content(content);
    ACTION_PATTERN
        .captures(&sanitized)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().trim().to_string())
}

/// Extracts content from a `<final_answer>` tag.
///
/// Supports format variations like:
/// - `<final_answer>content</final_answer>`
/// - `< final_answer >content</ final_answer >`
/// - `<FINAL_ANSWER>content</FINAL_ANSWER>`
pub fn extract_final_answer(content: &str) -> Option<String> {
    // First try sanitized content
    let sanitized = sanitize_xml_content(content);
    FINAL_ANSWER_PATTERN
        .captures(&sanitized)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().trim().to_string())
}

/// Checks if the content contains any recognized XML tags.
pub fn has_thought_tag(content: &str) -> bool {
    let lower = content.to_lowercase();
    lower.contains("<thought") || lower.contains("< thought")
}

/// Checks if the content contains an action tag.
pub fn has_action_tag(content: &str) -> bool {
    let lower = content.to_lowercase();
    lower.contains("<action") || lower.contains("< action")
}

/// Checks if the content contains a final_answer tag.
pub fn has_final_answer_tag(content: &str) -> bool {
    let lower = content.to_lowercase();
    lower.contains("<final_answer") || lower.contains("< final_answer")
}

/// Sanitizes XML content by fixing common formatting issues.
///
/// This function attempts to repair:
/// 1. HTML entity escapes (`&lt;` -> `<`, `&gt;` -> `>`)
/// 2. Missing closing tags
/// 3. Common typos in tag names
pub fn sanitize_xml_content(content: &str) -> String {
    let mut result = content.to_string();

    // 1. Fix HTML entity escapes
    result = result
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&apos;", "'");

    // 2. Fix missing closing tags (only add if opening tag exists without closing)
    // React mode tags
    result = fix_missing_closing_tag(&result, "thought");
    result = fix_missing_closing_tag(&result, "action");
    result = fix_missing_closing_tag(&result, "final_answer");
    result = fix_missing_closing_tag(&result, "observation");
    // Plan mode tags
    result = fix_missing_closing_tag(&result, "task_plan");
    result = fix_missing_closing_tag(&result, "goal");
    result = fix_missing_closing_tag(&result, "subtasks");
    result = fix_missing_closing_tag(&result, "subtask");
    result = fix_missing_closing_tag(&result, "description");
    result = fix_missing_closing_tag(&result, "dependencies");
    result = fix_missing_closing_tag(&result, "tools");

    // 3. Fix common typos
    result = fix_common_typos(&result);

    result
}

/// Fixes a missing closing tag for a specific tag name.
fn fix_missing_closing_tag(content: &str, tag_name: &str) -> String {
    let lower = content.to_lowercase();
    let open_tag_pattern = format!("<{}", tag_name.to_lowercase());
    let close_tag_pattern = format!("</{}", tag_name.to_lowercase());

    // Check if opening tag exists but closing tag is missing
    if lower.contains(&open_tag_pattern) && !lower.contains(&close_tag_pattern) {
        // Find the position after the opening tag
        if let Some(open_pos) = lower.find(&open_tag_pattern) {
            // Find the end of the opening tag (the '>')
            if content[open_pos..].find('>').is_some() {
                // Append closing tag at the end
                return format!("{}</{}>", content, tag_name);
            }
        }
        // Fallback: just append closing tag
        return format!("{}</{}>", content, tag_name);
    }

    content.to_string()
}

/// Fixes common typos in tag names.
fn fix_common_typos(content: &str) -> String {
    let mut result = content.to_string();

    // Common typos for thought
    result = result.replace("<thougt>", "<thought>");
    result = result.replace("</thougt>", "</thought>");
    result = result.replace("<thougth>", "<thought>");
    result = result.replace("</thougth>", "</thought>");
    result = result.replace("<tought>", "<thought>");
    result = result.replace("</tought>", "</thought>");

    // Common typos for action
    result = result.replace("<acton>", "<action>");
    result = result.replace("</acton>", "</action>");
    result = result.replace("<acion>", "<action>");
    result = result.replace("</acion>", "</action>");

    // Common typos for final_answer
    result = result.replace("<finalanswer>", "<final_answer>");
    result = result.replace("</finalanswer>", "</final_answer>");
    result = result.replace("<final-answer>", "<final_answer>");
    result = result.replace("</final-answer>", "</final_answer>");
    result = result.replace("<FinalAnswer>", "<final_answer>");
    result = result.replace("</FinalAnswer>", "</final_answer>");

    // Common typos for task_plan (Plan mode)
    result = result.replace("<taskplan>", "<task_plan>");
    result = result.replace("</taskplan>", "</task_plan>");
    result = result.replace("<task-plan>", "<task_plan>");
    result = result.replace("</task-plan>", "</task_plan>");
    result = result.replace("<TaskPlan>", "<task_plan>");
    result = result.replace("</TaskPlan>", "</task_plan>");
    result = result.replace("<Taskplan>", "<task_plan>");
    result = result.replace("</Taskplan>", "</task_plan>");

    // Common typos for subtask
    result = result.replace("<sub_task", "<subtask");
    result = result.replace("</sub_task>", "</subtask>");
    result = result.replace("<sub-task", "<subtask");
    result = result.replace("</sub-task>", "</subtask>");
    result = result.replace("<SubTask", "<subtask");
    result = result.replace("</SubTask>", "</subtask>");

    // Fix unquoted subtask id attributes: id=1 -> id="1"
    // This handles cases like <subtask id=1> or <subtask id=2>
    result = fix_unquoted_subtask_id(&result);

    result
}

/// Fixes unquoted subtask id attributes.
/// Converts `<subtask id=1>` to `<subtask id="1">`.
fn fix_unquoted_subtask_id(content: &str) -> String {
    use regex::Regex;
    static UNQUOTED_ID_PATTERN: Lazy<Regex> =
        Lazy::new(|| Regex::new(r#"(?i)<\s*subtask\s+id\s*=\s*(\d+)\s*>"#).unwrap());

    UNQUOTED_ID_PATTERN
        .replace_all(content, r#"<subtask id="$1">"#)
        .to_string()
}

/// Result of XML tag extraction with diagnostic information.
/// Reserved for future use with more detailed extraction feedback.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct ExtractionResult {
    /// The extracted content, if successful.
    pub content: Option<String>,
    /// Whether the content was sanitized/repaired.
    pub was_repaired: bool,
    /// Description of repairs made, if any.
    pub repair_notes: Option<String>,
}

#[allow(dead_code)]
impl ExtractionResult {
    /// Creates a successful result without repairs.
    pub fn success(content: String) -> Self {
        Self {
            content: Some(content),
            was_repaired: false,
            repair_notes: None,
        }
    }

    /// Creates a successful result with repairs.
    pub fn repaired(content: String, notes: String) -> Self {
        Self {
            content: Some(content),
            was_repaired: true,
            repair_notes: Some(notes),
        }
    }

    /// Creates a failed result.
    pub fn failed() -> Self {
        Self {
            content: None,
            was_repaired: false,
            repair_notes: None,
        }
    }
}

/// Extracts thought with detailed result information.
/// Reserved for future use with more detailed extraction feedback.
#[allow(dead_code)]
pub fn extract_thought_detailed(content: &str) -> ExtractionResult {
    // Try direct extraction first
    if let Some(result) = THOUGHT_PATTERN
        .captures(content)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().trim().to_string())
    {
        return ExtractionResult::success(result);
    }

    // Try with sanitization
    let sanitized = sanitize_xml_content(content);
    if sanitized != content
        && let Some(result) = THOUGHT_PATTERN
            .captures(&sanitized)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().trim().to_string())
    {
        return ExtractionResult::repaired(result, "Content was sanitized".to_string());
    }

    ExtractionResult::failed()
}

/// Extracts action with detailed result information.
/// Reserved for future use with more detailed extraction feedback.
#[allow(dead_code)]
pub fn extract_action_detailed(content: &str) -> ExtractionResult {
    // Try direct extraction first
    if let Some(result) = ACTION_PATTERN
        .captures(content)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().trim().to_string())
    {
        return ExtractionResult::success(result);
    }

    // Try with sanitization
    let sanitized = sanitize_xml_content(content);
    if sanitized != content
        && let Some(result) = ACTION_PATTERN
            .captures(&sanitized)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().trim().to_string())
    {
        return ExtractionResult::repaired(result, "Content was sanitized".to_string());
    }

    ExtractionResult::failed()
}

/// Extracts final_answer with detailed result information.
/// Reserved for future use with more detailed extraction feedback.
#[allow(dead_code)]
pub fn extract_final_answer_detailed(content: &str) -> ExtractionResult {
    // Try direct extraction first
    if let Some(result) = FINAL_ANSWER_PATTERN
        .captures(content)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().trim().to_string())
    {
        return ExtractionResult::success(result);
    }

    // Try with sanitization
    let sanitized = sanitize_xml_content(content);
    if sanitized != content
        && let Some(result) = FINAL_ANSWER_PATTERN
            .captures(&sanitized)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().trim().to_string())
    {
        return ExtractionResult::repaired(result, "Content was sanitized".to_string());
    }

    ExtractionResult::failed()
}

// ============================================================================
// Task Plan Parsing (Plan Mode)
// ============================================================================

/// Regex pattern for task_plan extraction.
static TASK_PLAN_PATTERN: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?si)<\s*task_plan\s*>(.*?)<\s*/\s*task_plan\s*>").unwrap());

/// Regex pattern for goal extraction within task_plan.
static GOAL_PATTERN: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?si)<\s*goal\s*>(.*?)<\s*/\s*goal\s*>").unwrap());

/// Regex pattern for subtasks container.
static SUBTASKS_PATTERN: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?si)<\s*subtasks\s*>(.*?)<\s*/\s*subtasks\s*>").unwrap());

/// Regex pattern for individual subtask with id attribute.
static SUBTASK_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?si)<\s*subtask\s+id\s*=\s*"?(\d+)"?\s*>(.*?)<\s*/\s*subtask\s*>"#).unwrap()
});

/// Regex pattern for description within subtask.
static DESCRIPTION_PATTERN: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?si)<\s*description\s*>(.*?)<\s*/\s*description\s*>").unwrap());

/// Regex pattern for dependencies within subtask.
static DEPENDENCIES_PATTERN: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?si)<\s*dependencies\s*>(.*?)<\s*/\s*dependencies\s*>").unwrap());

/// Regex pattern for tools within subtask.
static TOOLS_PATTERN: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?si)<\s*tools\s*>(.*?)<\s*/\s*tools\s*>").unwrap());

/// Regex pattern for recommended_skill within subtask.
static RECOMMENDED_SKILL_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?si)<\s*recommended_skill\s*>(.*?)<\s*/\s*recommended_skill\s*>").unwrap()
});

/// Raw task plan structure (before validation).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskPlanRaw {
    /// The goal extracted from <goal> tag.
    pub goal: String,
    /// List of raw subtasks.
    pub subtasks: Vec<SubTaskRaw>,
}

/// Raw subtask structure (before validation).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubTaskRaw {
    /// Subtask ID as string (will be validated later).
    pub id: String,
    /// Description of the subtask.
    pub description: String,
    /// Dependencies as string IDs.
    pub dependencies: Vec<String>,
    /// Tool names.
    pub tools: Vec<String>,
    /// Recommended skill name (optional).
    pub recommended_skill: Option<String>,
}

/// Extracts a task plan from LLM response content.
///
/// Expected format:
/// ```xml
/// <task_plan>
///   <goal>User's goal</goal>
///   <subtasks>
///     <subtask id="1">
///       <description>Task description</description>
///       <dependencies></dependencies>
///       <tools>tool_name</tools>
///     </subtask>
///   </subtasks>
/// </task_plan>
/// ```
pub fn extract_task_plan(content: &str) -> Option<TaskPlanRaw> {
    // First sanitize the content
    let sanitized = sanitize_xml_content(content);

    // Extract the task_plan block
    let plan_content = TASK_PLAN_PATTERN
        .captures(&sanitized)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str())?;

    // Extract goal
    let goal = GOAL_PATTERN
        .captures(plan_content)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().trim().to_string())?;

    // Extract subtasks container
    let subtasks_content = SUBTASKS_PATTERN
        .captures(plan_content)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str())?;

    // Extract individual subtasks
    let mut subtasks = Vec::new();
    for cap in SUBTASK_PATTERN.captures_iter(subtasks_content) {
        let id = cap.get(1)?.as_str().to_string();
        let subtask_content = cap.get(2)?.as_str();

        // Extract description
        let description = DESCRIPTION_PATTERN
            .captures(subtask_content)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().trim().to_string())
            .unwrap_or_default();

        // Extract dependencies (comma or space separated)
        let dependencies = DEPENDENCIES_PATTERN
            .captures(subtask_content)
            .and_then(|c| c.get(1))
            .map(|m| {
                m.as_str()
                    .split(|c: char| c == ',' || c.is_whitespace())
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        // Extract tools (comma or space separated)
        let tools = TOOLS_PATTERN
            .captures(subtask_content)
            .and_then(|c| c.get(1))
            .map(|m| {
                m.as_str()
                    .split(|c: char| c == ',' || c.is_whitespace())
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        // Extract recommended skill (optional)
        let recommended_skill = RECOMMENDED_SKILL_PATTERN
            .captures(subtask_content)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().trim().to_string())
            .filter(|s| !s.is_empty());

        subtasks.push(SubTaskRaw {
            id,
            description,
            dependencies,
            tools,
            recommended_skill,
        });
    }

    if subtasks.is_empty() {
        return None;
    }

    Some(TaskPlanRaw { goal, subtasks })
}

/// Checks if the content contains a task_plan tag.
pub fn has_task_plan_tag(content: &str) -> bool {
    let lower = content.to_lowercase();
    lower.contains("<task_plan") || lower.contains("< task_plan")
}

/// Result of task plan extraction with diagnostic information.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct TaskPlanExtractionResult {
    /// The extracted task plan, if successful.
    pub plan: Option<TaskPlanRaw>,
    /// Whether the content was sanitized/repaired before extraction.
    pub was_repaired: bool,
    /// Description of repairs made, if any.
    pub repair_notes: Option<String>,
}

#[allow(dead_code)]
impl TaskPlanExtractionResult {
    /// Creates a successful result without repairs.
    pub fn success(plan: TaskPlanRaw) -> Self {
        Self {
            plan: Some(plan),
            was_repaired: false,
            repair_notes: None,
        }
    }

    /// Creates a successful result after repairs were applied.
    pub fn repaired(plan: TaskPlanRaw, notes: String) -> Self {
        Self {
            plan: Some(plan),
            was_repaired: true,
            repair_notes: Some(notes),
        }
    }

    /// Creates a failed result.
    pub fn failed() -> Self {
        Self {
            plan: None,
            was_repaired: false,
            repair_notes: None,
        }
    }

    /// Returns true if extraction was successful.
    pub fn is_success(&self) -> bool {
        self.plan.is_some()
    }
}

/// Extracts a task plan with detailed result information.
///
/// This function provides diagnostic information about the extraction process,
/// including whether repairs were needed and what was fixed.
#[allow(dead_code)]
pub fn extract_task_plan_detailed(content: &str) -> TaskPlanExtractionResult {
    // Try direct extraction first (on original content)
    if let Some(plan) = try_extract_task_plan_raw(content) {
        return TaskPlanExtractionResult::success(plan);
    }

    // Try with sanitization
    let sanitized = sanitize_xml_content(content);
    if sanitized != content
        && let Some(plan) = try_extract_task_plan_raw(&sanitized)
    {
        // Collect repair notes
        let mut repairs = Vec::new();

        if content.contains("&lt;") || content.contains("&gt;") {
            repairs.push("fixed HTML entity escapes");
        }
        if content.contains("<taskplan>") || content.contains("<task-plan>") {
            repairs.push("fixed task_plan tag typo");
        }
        if content.contains("<sub_task") || content.contains("<sub-task") {
            repairs.push("fixed subtask tag typo");
        }
        // Check for unquoted id attribute
        if regex::Regex::new(r#"(?i)<\s*subtask\s+id\s*=\s*\d+\s*>"#)
            .unwrap()
            .is_match(content)
        {
            repairs.push("fixed unquoted subtask id attribute");
        }

        let notes = if repairs.is_empty() {
            "Content was sanitized".to_string()
        } else {
            repairs.join(", ")
        };

        return TaskPlanExtractionResult::repaired(plan, notes);
    }

    TaskPlanExtractionResult::failed()
}

/// Internal helper to extract task plan without sanitization.
fn try_extract_task_plan_raw(content: &str) -> Option<TaskPlanRaw> {
    // Extract the task_plan block
    let plan_content = TASK_PLAN_PATTERN
        .captures(content)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str())?;

    // Extract goal
    let goal = GOAL_PATTERN
        .captures(plan_content)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().trim().to_string())?;

    // Extract subtasks container
    let subtasks_content = SUBTASKS_PATTERN
        .captures(plan_content)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str())?;

    // Extract individual subtasks
    let mut subtasks = Vec::new();
    for cap in SUBTASK_PATTERN.captures_iter(subtasks_content) {
        let id = cap.get(1)?.as_str().to_string();
        let subtask_content = cap.get(2)?.as_str();

        // Extract description
        let description = DESCRIPTION_PATTERN
            .captures(subtask_content)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().trim().to_string())
            .unwrap_or_default();

        // Extract dependencies (comma or space separated)
        let dependencies = DEPENDENCIES_PATTERN
            .captures(subtask_content)
            .and_then(|c| c.get(1))
            .map(|m| {
                m.as_str()
                    .split(|c: char| c == ',' || c.is_whitespace())
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        // Extract tools (comma or space separated)
        let tools = TOOLS_PATTERN
            .captures(subtask_content)
            .and_then(|c| c.get(1))
            .map(|m| {
                m.as_str()
                    .split(|c: char| c == ',' || c.is_whitespace())
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        // Extract recommended skill (optional)
        let recommended_skill = RECOMMENDED_SKILL_PATTERN
            .captures(subtask_content)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().trim().to_string())
            .filter(|s| !s.is_empty());

        subtasks.push(SubTaskRaw {
            id,
            description,
            dependencies,
            tools,
            recommended_skill,
        });
    }

    if subtasks.is_empty() {
        return None;
    }

    Some(TaskPlanRaw { goal, subtasks })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_thought_basic() {
        let content = "<thought>I need to search for information</thought>";
        assert_eq!(
            extract_thought(content),
            Some("I need to search for information".to_string())
        );
    }

    #[test]
    fn test_extract_thought_with_spaces() {
        let content = "< thought >I need to search for information</ thought >";
        assert_eq!(
            extract_thought(content),
            Some("I need to search for information".to_string())
        );
    }

    #[test]
    fn test_extract_thought_case_insensitive() {
        let content = "<THOUGHT>I need to search for information</THOUGHT>";
        assert_eq!(
            extract_thought(content),
            Some("I need to search for information".to_string())
        );
    }

    #[test]
    fn test_extract_thought_mixed_case() {
        let content = "<Thought>I need to search for information</Thought>";
        assert_eq!(
            extract_thought(content),
            Some("I need to search for information".to_string())
        );
    }

    #[test]
    fn test_extract_action_basic() {
        let content = "<action>search</action>";
        assert_eq!(extract_action(content), Some("search".to_string()));
    }

    #[test]
    fn test_extract_action_with_spaces() {
        let content = "< action >search</ action >";
        assert_eq!(extract_action(content), Some("search".to_string()));
    }

    #[test]
    fn test_extract_final_answer_basic() {
        let content = "<final_answer>The answer is 42</final_answer>";
        assert_eq!(
            extract_final_answer(content),
            Some("The answer is 42".to_string())
        );
    }

    #[test]
    fn test_extract_final_answer_case_insensitive() {
        let content = "<FINAL_ANSWER>The answer is 42</FINAL_ANSWER>";
        assert_eq!(
            extract_final_answer(content),
            Some("The answer is 42".to_string())
        );
    }

    #[test]
    fn test_sanitize_html_entities() {
        let content = "&lt;thought&gt;test&lt;/thought&gt;";
        let sanitized = sanitize_xml_content(content);
        assert_eq!(sanitized, "<thought>test</thought>");
        assert_eq!(extract_thought(&sanitized), Some("test".to_string()));
    }

    #[test]
    fn test_sanitize_missing_closing_tag() {
        let content = "<thought>I need to search";
        let sanitized = sanitize_xml_content(content);
        assert!(sanitized.contains("</thought>"));
    }

    #[test]
    fn test_fix_common_typos_thought() {
        let content = "<thougt>test</thougt>";
        let fixed = fix_common_typos(content);
        assert_eq!(fixed, "<thought>test</thought>");
    }

    #[test]
    fn test_fix_common_typos_final_answer() {
        let content = "<finalanswer>test</finalanswer>";
        let fixed = fix_common_typos(content);
        assert_eq!(fixed, "<final_answer>test</final_answer>");
    }

    #[test]
    fn test_fix_common_typos_final_answer_hyphen() {
        let content = "<final-answer>test</final-answer>";
        let fixed = fix_common_typos(content);
        assert_eq!(fixed, "<final_answer>test</final_answer>");
    }

    #[test]
    fn test_has_thought_tag() {
        assert!(has_thought_tag("<thought>test</thought>"));
        assert!(has_thought_tag("< thought >test</ thought >"));
        assert!(has_thought_tag("<THOUGHT>test</THOUGHT>"));
        assert!(!has_thought_tag("<action>test</action>"));
    }

    #[test]
    fn test_has_action_tag() {
        assert!(has_action_tag("<action>test</action>"));
        assert!(has_action_tag("< action >test</ action >"));
        assert!(!has_action_tag("<thought>test</thought>"));
    }

    #[test]
    fn test_has_final_answer_tag() {
        assert!(has_final_answer_tag("<final_answer>test</final_answer>"));
        assert!(has_final_answer_tag(
            "< final_answer >test</ final_answer >"
        ));
        assert!(!has_final_answer_tag("<thought>test</thought>"));
    }

    #[test]
    fn test_extract_thought_detailed_success() {
        let content = "<thought>test</thought>";
        let result = extract_thought_detailed(content);
        assert_eq!(result.content, Some("test".to_string()));
        assert!(!result.was_repaired);
    }

    #[test]
    fn test_extract_thought_detailed_repaired() {
        let content = "&lt;thought&gt;test&lt;/thought&gt;";
        let result = extract_thought_detailed(content);
        assert_eq!(result.content, Some("test".to_string()));
        assert!(result.was_repaired);
    }

    #[test]
    fn test_multiline_content() {
        let content =
            "<thought>\nI need to think about this.\nLet me analyze the problem.\n</thought>";
        let result = extract_thought(content);
        assert!(result.is_some());
        assert!(result.unwrap().contains("I need to think about this."));
    }

    #[test]
    fn test_nested_content_preservation() {
        let content = "<thought>The user asked about <code>function()</code></thought>";
        let result = extract_thought(content);
        assert_eq!(
            result,
            Some("The user asked about <code>function()</code>".to_string())
        );
    }

    // Task Plan parsing tests

    #[test]
    fn test_extract_task_plan_basic() {
        let content = r#"
<task_plan>
  <goal>Query weather for two cities</goal>
  <subtasks>
    <subtask id="1">
      <description>Query Beijing weather</description>
      <dependencies></dependencies>
      <tools>weather_query</tools>
    </subtask>
    <subtask id="2">
      <description>Query Shanghai weather</description>
      <dependencies></dependencies>
      <tools>weather_query</tools>
    </subtask>
  </subtasks>
</task_plan>
"#;
        let plan = extract_task_plan(content).unwrap();
        assert_eq!(plan.goal, "Query weather for two cities");
        assert_eq!(plan.subtasks.len(), 2);
        assert_eq!(plan.subtasks[0].id, "1");
        assert_eq!(plan.subtasks[0].description, "Query Beijing weather");
        assert!(plan.subtasks[0].dependencies.is_empty());
        assert_eq!(plan.subtasks[0].tools, vec!["weather_query"]);
    }

    #[test]
    fn test_extract_task_plan_with_dependencies() {
        let content = r#"
<task_plan>
  <goal>Send weather summary email</goal>
  <subtasks>
    <subtask id="1">
      <description>Query Beijing weather</description>
      <dependencies></dependencies>
      <tools>weather_query</tools>
    </subtask>
    <subtask id="2">
      <description>Query Shanghai weather</description>
      <dependencies></dependencies>
      <tools>weather_query</tools>
    </subtask>
    <subtask id="3">
      <description>Send summary email</description>
      <dependencies>1, 2</dependencies>
      <tools>send_email</tools>
    </subtask>
  </subtasks>
</task_plan>
"#;
        let plan = extract_task_plan(content).unwrap();
        assert_eq!(plan.subtasks.len(), 3);
        assert_eq!(plan.subtasks[2].id, "3");
        assert_eq!(plan.subtasks[2].dependencies, vec!["1", "2"]);
    }

    #[test]
    fn test_extract_task_plan_case_insensitive() {
        let content = r#"
<TASK_PLAN>
  <GOAL>Test goal</GOAL>
  <SUBTASKS>
    <SUBTASK id="1">
      <DESCRIPTION>Test task</DESCRIPTION>
      <DEPENDENCIES></DEPENDENCIES>
      <TOOLS>test_tool</TOOLS>
    </SUBTASK>
  </SUBTASKS>
</TASK_PLAN>
"#;
        let plan = extract_task_plan(content).unwrap();
        assert_eq!(plan.goal, "Test goal");
        assert_eq!(plan.subtasks.len(), 1);
    }

    #[test]
    fn test_extract_task_plan_with_spaces() {
        let content = r#"
< task_plan >
  < goal >Test goal</ goal >
  < subtasks >
    < subtask id="1" >
      < description >Test task</ description >
      < dependencies ></ dependencies >
      < tools >test_tool</ tools >
    </ subtask >
  </ subtasks >
</ task_plan >
"#;
        let plan = extract_task_plan(content).unwrap();
        assert_eq!(plan.goal, "Test goal");
        assert_eq!(plan.subtasks.len(), 1);
    }

    #[test]
    fn test_extract_task_plan_empty_subtasks() {
        let content = r#"
<task_plan>
  <goal>Test goal</goal>
  <subtasks>
  </subtasks>
</task_plan>
"#;
        let plan = extract_task_plan(content);
        assert!(plan.is_none());
    }

    #[test]
    fn test_has_task_plan_tag() {
        assert!(has_task_plan_tag("<task_plan>"));
        assert!(has_task_plan_tag("< task_plan >"));
        assert!(has_task_plan_tag("<TASK_PLAN>"));
        assert!(!has_task_plan_tag("<thought>"));
    }

    #[test]
    fn test_extract_task_plan_multiple_tools() {
        let content = r#"
<task_plan>
  <goal>Complex task</goal>
  <subtasks>
    <subtask id="1">
      <description>Multi-tool task</description>
      <dependencies></dependencies>
      <tools>search, analyze, report</tools>
    </subtask>
  </subtasks>
</task_plan>
"#;
        let plan = extract_task_plan(content).unwrap();
        assert_eq!(plan.subtasks[0].tools, vec!["search", "analyze", "report"]);
    }

    #[test]
    fn test_extract_task_plan_with_recommended_skill() {
        let content = r#"
<task_plan>
  <goal>Query weather and send email</goal>
  <subtasks>
    <subtask id="1">
      <description>Query weather for Beijing</description>
      <dependencies></dependencies>
      <tools>weather_query</tools>
      <recommended_skill>weather-query</recommended_skill>
    </subtask>
    <subtask id="2">
      <description>Send email with results</description>
      <dependencies>1</dependencies>
      <tools>send_email</tools>
    </subtask>
  </subtasks>
</task_plan>
"#;
        let plan = extract_task_plan(content).unwrap();
        assert_eq!(plan.subtasks.len(), 2);

        // First subtask has recommended_skill
        assert_eq!(
            plan.subtasks[0].recommended_skill,
            Some("weather-query".to_string())
        );

        // Second subtask has no recommended_skill
        assert_eq!(plan.subtasks[1].recommended_skill, None);
    }

    #[test]
    fn test_extract_task_plan_recommended_skill_empty() {
        let content = r#"
<task_plan>
  <goal>Simple task</goal>
  <subtasks>
    <subtask id="1">
      <description>Task description</description>
      <dependencies></dependencies>
      <tools>tool1</tools>
      <recommended_skill></recommended_skill>
    </subtask>
  </subtasks>
</task_plan>
"#;
        let plan = extract_task_plan(content).unwrap();
        // Empty recommended_skill should be treated as None
        assert_eq!(plan.subtasks[0].recommended_skill, None);
    }

    // Plan mode XML parsing enhancement tests

    #[test]
    fn test_fix_common_typos_task_plan() {
        let content = "<taskplan><goal>test</goal></taskplan>";
        let fixed = fix_common_typos(content);
        assert_eq!(fixed, "<task_plan><goal>test</goal></task_plan>");
    }

    #[test]
    fn test_fix_common_typos_task_plan_hyphen() {
        let content = "<task-plan><goal>test</goal></task-plan>";
        let fixed = fix_common_typos(content);
        assert_eq!(fixed, "<task_plan><goal>test</goal></task_plan>");
    }

    #[test]
    fn test_fix_common_typos_task_plan_camel_case() {
        let content = "<TaskPlan><goal>test</goal></TaskPlan>";
        let fixed = fix_common_typos(content);
        assert_eq!(fixed, "<task_plan><goal>test</goal></task_plan>");
    }

    #[test]
    fn test_fix_common_typos_subtask() {
        let content = r#"<sub_task id="1">test</sub_task>"#;
        let fixed = fix_common_typos(content);
        assert!(fixed.contains("<subtask"));
        assert!(fixed.contains("</subtask>"));
    }

    #[test]
    fn test_fix_common_typos_subtask_hyphen() {
        let content = r#"<sub-task id="1">test</sub-task>"#;
        let fixed = fix_common_typos(content);
        assert!(fixed.contains("<subtask"));
        assert!(fixed.contains("</subtask>"));
    }

    #[test]
    fn test_fix_unquoted_subtask_id() {
        let content = "<subtask id=1><description>test</description></subtask>";
        let fixed = fix_unquoted_subtask_id(content);
        assert_eq!(
            fixed,
            r#"<subtask id="1"><description>test</description></subtask>"#
        );
    }

    #[test]
    fn test_fix_unquoted_subtask_id_multiple() {
        let content = "<subtask id=1>first</subtask><subtask id=2>second</subtask>";
        let fixed = fix_unquoted_subtask_id(content);
        assert!(fixed.contains(r#"id="1""#));
        assert!(fixed.contains(r#"id="2""#));
    }

    #[test]
    fn test_sanitize_missing_closing_tag_task_plan() {
        let content = "<task_plan><goal>test</goal><subtasks>";
        let sanitized = sanitize_xml_content(content);
        assert!(sanitized.contains("</task_plan>"));
        assert!(sanitized.contains("</subtasks>"));
    }

    #[test]
    fn test_extract_task_plan_with_typos() {
        let content = r#"
<taskplan>
  <goal>Test goal</goal>
  <subtasks>
    <sub-task id=1>
      <description>Test task</description>
      <dependencies></dependencies>
      <tools>test_tool</tools>
    </sub-task>
  </subtasks>
</taskplan>
"#;
        let plan = extract_task_plan(content).unwrap();
        assert_eq!(plan.goal, "Test goal");
        assert_eq!(plan.subtasks.len(), 1);
        assert_eq!(plan.subtasks[0].id, "1");
    }

    #[test]
    fn test_extract_task_plan_detailed_success() {
        let content = r#"
<task_plan>
  <goal>Test goal</goal>
  <subtasks>
    <subtask id="1">
      <description>Test task</description>
      <dependencies></dependencies>
      <tools>test_tool</tools>
    </subtask>
  </subtasks>
</task_plan>
"#;
        let result = extract_task_plan_detailed(content);
        assert!(result.is_success());
        assert!(!result.was_repaired);
        assert!(result.repair_notes.is_none());
        assert_eq!(result.plan.unwrap().goal, "Test goal");
    }

    #[test]
    fn test_extract_task_plan_detailed_repaired_html_entities() {
        let content = r#"
&lt;task_plan&gt;
  &lt;goal&gt;Test goal&lt;/goal&gt;
  &lt;subtasks&gt;
    &lt;subtask id="1"&gt;
      &lt;description&gt;Test task&lt;/description&gt;
      &lt;dependencies&gt;&lt;/dependencies&gt;
      &lt;tools&gt;test_tool&lt;/tools&gt;
    &lt;/subtask&gt;
  &lt;/subtasks&gt;
&lt;/task_plan&gt;
"#;
        let result = extract_task_plan_detailed(content);
        assert!(result.is_success());
        assert!(result.was_repaired);
        assert!(
            result
                .repair_notes
                .as_ref()
                .unwrap()
                .contains("HTML entity")
        );
    }

    #[test]
    fn test_extract_task_plan_detailed_repaired_typos() {
        let content = r#"
<taskplan>
  <goal>Test goal</goal>
  <subtasks>
    <subtask id="1">
      <description>Test task</description>
      <dependencies></dependencies>
      <tools>test_tool</tools>
    </subtask>
  </subtasks>
</taskplan>
"#;
        let result = extract_task_plan_detailed(content);
        assert!(result.is_success());
        assert!(result.was_repaired);
        assert!(
            result
                .repair_notes
                .as_ref()
                .unwrap()
                .contains("task_plan tag typo")
        );
    }

    #[test]
    fn test_extract_task_plan_detailed_unquoted_id_direct_match() {
        // Note: The regex pattern already supports unquoted id attributes with "?
        // so this will match directly without needing repair
        let content = r#"
<task_plan>
  <goal>Test goal</goal>
  <subtasks>
    <subtask id=1>
      <description>Test task</description>
      <dependencies></dependencies>
      <tools>test_tool</tools>
    </subtask>
  </subtasks>
</task_plan>
"#;
        let result = extract_task_plan_detailed(content);
        assert!(result.is_success());
        // Direct match - no repair needed because regex already handles unquoted id
        assert!(!result.was_repaired);
    }

    #[test]
    fn test_extract_task_plan_detailed_repaired_subtask_typo() {
        // This tests a case that actually requires repair (sub-task -> subtask)
        let content = r#"
<task_plan>
  <goal>Test goal</goal>
  <subtasks>
    <sub-task id="1">
      <description>Test task</description>
      <dependencies></dependencies>
      <tools>test_tool</tools>
    </sub-task>
  </subtasks>
</task_plan>
"#;
        let result = extract_task_plan_detailed(content);
        assert!(result.is_success());
        assert!(result.was_repaired);
        assert!(
            result
                .repair_notes
                .as_ref()
                .unwrap()
                .contains("subtask tag typo")
        );
    }

    #[test]
    fn test_extract_task_plan_detailed_failed() {
        let content = "no task plan here";
        let result = extract_task_plan_detailed(content);
        assert!(!result.is_success());
        assert!(result.plan.is_none());
    }

    #[test]
    fn test_task_plan_extraction_result_methods() {
        // Test success
        let plan = TaskPlanRaw {
            goal: "test".to_string(),
            subtasks: vec![],
        };
        let result = TaskPlanExtractionResult::success(plan.clone());
        assert!(result.is_success());
        assert!(!result.was_repaired);

        // Test repaired
        let result = TaskPlanExtractionResult::repaired(plan, "fixed something".to_string());
        assert!(result.is_success());
        assert!(result.was_repaired);
        assert_eq!(result.repair_notes, Some("fixed something".to_string()));

        // Test failed
        let result = TaskPlanExtractionResult::failed();
        assert!(!result.is_success());
        assert!(result.plan.is_none());
    }
}
