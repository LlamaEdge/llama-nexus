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

    // Fix mismatched </subtasks> closing tag when it should be </subtask>
    // LLM sometimes outputs </subtasks> instead of </subtask> for individual subtask elements
    result = fix_mismatched_subtask_closing_tag(&result);

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

/// Fixes mismatched subtask closing tags.
///
/// LLM sometimes outputs `</subtasks>` instead of `</subtask>` for individual
/// subtask elements (confusing the plural container tag with the singular element tag).
///
/// This function detects patterns like:
/// ```xml
/// <subtask id="2">
///   ...content...
/// </subtasks>  <!-- Should be </subtask> -->
/// ```
///
/// And fixes them to:
/// ```xml
/// <subtask id="2">
///   ...content...
/// </subtask>
/// ```
///
/// The fix works by counting open/close tags: if we find a `</subtasks>` that
/// doesn't match a `<subtasks>` container, it's likely a typo for `</subtask>`.
fn fix_mismatched_subtask_closing_tag(content: &str) -> String {
    use regex::Regex;

    // Regex patterns for detecting tag positions
    static SUBTASKS_OPEN: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"(?i)<\s*subtasks\s*>").unwrap());
    static SUBTASKS_CLOSE: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"(?i)<\s*/\s*subtasks\s*>").unwrap());
    static SUBTASK_OPEN: Lazy<Regex> =
        Lazy::new(|| Regex::new(r#"(?i)<\s*subtask\s+id\s*=\s*"?\d+"?\s*>"#).unwrap());
    static SUBTASK_CLOSE: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"(?i)<\s*/\s*subtask\s*>").unwrap());

    // Count container tags (plural)
    let container_open_count = SUBTASKS_OPEN.find_iter(content).count();
    let container_close_count = SUBTASKS_CLOSE.find_iter(content).count();

    // Count element tags (singular)
    let element_open_count = SUBTASK_OPEN.find_iter(content).count();
    let element_close_count = SUBTASK_CLOSE.find_iter(content).count();

    // If we have more container closes than opens, AND
    // more element opens than closes, then some </subtasks> are typos
    if container_close_count > container_open_count
        && element_open_count > element_close_count
    {
        // Calculate how many </subtasks> should be </subtask>
        let extra_container_closes = container_close_count - container_open_count;
        let missing_element_closes = element_open_count - element_close_count;

        // Replace the minimum of these counts
        let fixes_needed = extra_container_closes.min(missing_element_closes);

        if fixes_needed > 0 {
            // Find all </subtasks> positions
            let positions: Vec<_> = SUBTASKS_CLOSE
                .find_iter(content)
                .map(|m| (m.start(), m.end()))
                .collect();

            // We need to keep `container_open_count` legitimate container closes
            // The legitimate ones are typically at the end (outermost)
            // So we replace the first N ones that appear to be misplaced

            // Strategy: keep the LAST container_open_count occurrences as legitimate
            // Replace the first `fixes_needed` occurrences
            let legitimate_count = container_open_count;
            let total_closes = positions.len();

            if total_closes > legitimate_count {
                let mut result = content.to_string();

                // Get positions to replace (skip the last `legitimate_count` ones)
                // These are the misplaced </subtasks> that should be </subtask>
                let to_replace: Vec<_> = positions
                    .into_iter()
                    .rev()
                    .skip(legitimate_count)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .take(fixes_needed)
                    .collect();

                // Replace from back to front to preserve indices
                for (start, end) in to_replace.into_iter().rev() {
                    result = format!(
                        "{}</subtask>{}",
                        &result[..start],
                        &result[end..]
                    );
                }

                return result;
            }
        }
    }

    content.to_string()
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
/// Note: This pattern only matches `</subtask>` closing tag. The `fix_mismatched_subtask_closing_tag`
/// function handles the case where LLM outputs `</subtasks>` instead of `</subtask>`.
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

// ============================================================================
// XML Tool Call Parsing (JSON-embedded format)
// ============================================================================

/// Represents a parsed XML tool call with JSON-embedded format.
///
/// Supports the LlamaEdge format:
/// `<action>{"name": "tool_name", "arguments": {"param": "value"}}</action>`
#[derive(Debug, Clone)]
pub struct XmlToolCall {
    /// Tool name extracted from JSON "name" field
    pub tool_name: String,
    /// Tool arguments as JSON string
    pub arguments: String,
}

/// Extracts a tool call from XML with JSON-embedded format.
///
/// Parses the LlamaEdge format:
/// `<action>{"name": "tool_name", "arguments": {"param": "value"}}</action>`
///
/// Returns Some(XmlToolCall) if parsing succeeds, None otherwise.
pub fn extract_xml_tool_call(content: &str) -> Option<XmlToolCall> {
    // Extract content from <action> tag
    let action_content = extract_action(content)?;

    // Check if it's JSON format (starts with '{')
    let trimmed = action_content.trim();
    if !trimmed.starts_with('{') {
        // Not JSON format, return None
        return None;
    }

    // Parse JSON
    let json: serde_json::Value = serde_json::from_str(trimmed).ok()?;

    // Extract "name" field
    let name = json.get("name")?.as_str()?;

    // Extract "arguments" field (default to empty object if missing)
    let arguments = json
        .get("arguments")
        .map(|v| v.to_string())
        .unwrap_or_else(|| "{}".to_string());

    Some(XmlToolCall {
        tool_name: name.to_string(),
        arguments,
    })
}

/// Checks if the content contains a valid XML tool call with JSON format.
#[allow(dead_code)]
pub fn has_xml_tool_call(content: &str) -> bool {
    if !has_action_tag(content) {
        return false;
    }

    // Try to extract and parse
    extract_xml_tool_call(content).is_some()
}

// ============================================================================
// Task Plan Extraction (detailed)
// ============================================================================

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

    // ============================================================================
    // XML Tool Call Parsing Tests (JSON-embedded format)
    // ============================================================================

    #[test]
    fn test_extract_xml_tool_call_json_embedded() {
        let content = r#"
<thought>需要计算两个数的和</thought>
<action>{"name": "mcp__cardea-calculator__sum", "arguments": {"a": 23, "b": 32}}</action>
"#;
        let tool_call = extract_xml_tool_call(content).unwrap();
        assert_eq!(tool_call.tool_name, "mcp__cardea-calculator__sum");
        assert!(tool_call.arguments.contains("23"));
        assert!(tool_call.arguments.contains("32"));
    }

    #[test]
    fn test_extract_xml_tool_call_no_arguments() {
        let content = r#"<action>{"name": "simple_tool"}</action>"#;
        let tool_call = extract_xml_tool_call(content).unwrap();
        assert_eq!(tool_call.tool_name, "simple_tool");
        assert_eq!(tool_call.arguments, "{}");
    }

    #[test]
    fn test_extract_xml_tool_call_empty_arguments() {
        let content = r#"<action>{"name": "tool", "arguments": {}}</action>"#;
        let tool_call = extract_xml_tool_call(content).unwrap();
        assert_eq!(tool_call.tool_name, "tool");
        assert_eq!(tool_call.arguments, "{}");
    }

    #[test]
    fn test_extract_xml_tool_call_complex_arguments() {
        let content =
            r#"<action>{"name": "search", "arguments": {"query": "test", "limit": 10}}</action>"#;
        let tool_call = extract_xml_tool_call(content).unwrap();
        assert_eq!(tool_call.tool_name, "search");
        assert!(tool_call.arguments.contains("test"));
        assert!(tool_call.arguments.contains("10"));
    }

    #[test]
    fn test_extract_xml_tool_call_not_json() {
        // Non-JSON format should return None
        let content = r#"<action>simple_tool_name</action>"#;
        assert!(extract_xml_tool_call(content).is_none());
    }

    #[test]
    fn test_extract_xml_tool_call_invalid_json() {
        // Invalid JSON should return None
        let content = r#"<action>{invalid json}</action>"#;
        assert!(extract_xml_tool_call(content).is_none());
    }

    #[test]
    fn test_extract_xml_tool_call_missing_name() {
        // Missing name field should return None
        let content = r#"<action>{"arguments": {"a": 1}}</action>"#;
        assert!(extract_xml_tool_call(content).is_none());
    }

    #[test]
    fn test_has_xml_tool_call_valid() {
        let content = r#"<action>{"name": "tool", "arguments": {}}</action>"#;
        assert!(has_xml_tool_call(content));
    }

    #[test]
    fn test_has_xml_tool_call_invalid() {
        let content = r#"<action>not_json</action>"#;
        assert!(!has_xml_tool_call(content));
    }

    #[test]
    fn test_has_xml_tool_call_no_action_tag() {
        let content = r#"{"name": "tool", "arguments": {}}"#;
        assert!(!has_xml_tool_call(content));
    }

    #[test]
    fn test_extract_xml_tool_call_with_array_arguments() {
        let content = r#"<action>{"name": "multi_search", "arguments": {"queries": ["a", "b", "c"]}}</action>"#;
        let tool_call = extract_xml_tool_call(content).unwrap();
        assert_eq!(tool_call.tool_name, "multi_search");
        assert!(tool_call.arguments.contains("["));
        assert!(tool_call.arguments.contains("]"));
    }

    #[test]
    fn test_extract_xml_tool_call_mcp_tool_name() {
        let content = r#"
<thought>I need to call the MCP calculator</thought>
<action>{"name": "mcp__cardea-calculator__multiply", "arguments": {"x": 5, "y": 10}}</action>
"#;
        let tool_call = extract_xml_tool_call(content).unwrap();
        assert_eq!(tool_call.tool_name, "mcp__cardea-calculator__multiply");
        assert!(tool_call.arguments.contains("5"));
        assert!(tool_call.arguments.contains("10"));
    }

    #[test]
    fn test_extract_xml_tool_call_with_final_answer_should_still_parse() {
        // extract_xml_tool_call only parses <action>, it doesn't check for final_answer
        // The caller (plan.rs) is responsible for checking has_final_answer_tag
        let content = r#"
<action>{"name": "tool", "arguments": {}}</action>
<final_answer>Done!</final_answer>
"#;
        let tool_call = extract_xml_tool_call(content);
        // Should still parse the action tag
        assert!(tool_call.is_some());
        assert_eq!(tool_call.unwrap().tool_name, "tool");
    }

    #[test]
    fn test_extract_xml_tool_call_case_insensitive() {
        let content = r#"<ACTION>{"name": "tool", "arguments": {}}</ACTION>"#;
        let tool_call = extract_xml_tool_call(content).unwrap();
        assert_eq!(tool_call.tool_name, "tool");
    }

    #[test]
    fn test_extract_xml_tool_call_with_whitespace() {
        let content = r#"< action >{"name": "tool", "arguments": {}}</ action >"#;
        let tool_call = extract_xml_tool_call(content).unwrap();
        assert_eq!(tool_call.tool_name, "tool");
    }

    // ==================== Tests for </subtasks> -> </subtask> fix ====================

    #[test]
    fn test_fix_mismatched_subtask_closing_tag_single() {
        let content = r#"<subtask id="1"><description>Test</description></subtasks>"#;
        let fixed = fix_mismatched_subtask_closing_tag(content);
        assert_eq!(
            fixed,
            r#"<subtask id="1"><description>Test</description></subtask>"#
        );
    }

    #[test]
    fn test_fix_mismatched_subtask_closing_tag_multiple() {
        let content = r#"
<subtask id="1">
  <description>First task</description>
</subtask>
<subtask id="2">
  <description>Second task</description>
</subtasks>
"#;
        let fixed = fix_mismatched_subtask_closing_tag(content);
        assert!(fixed.contains("</subtask>\n<subtask id=\"2\">"));
        assert!(fixed.ends_with("</subtask>\n"));
    }

    #[test]
    fn test_fix_mismatched_subtask_closing_tag_preserves_correct() {
        let content = r#"<subtask id="1"><description>Test</description></subtask>"#;
        let fixed = fix_mismatched_subtask_closing_tag(content);
        // Should remain unchanged
        assert_eq!(fixed, content);
    }

    #[test]
    fn test_fix_common_typos_fixes_subtasks_closing_tag() {
        let content = r#"<subtask id="1"><description>Test</description></subtasks>"#;
        let fixed = fix_common_typos(content);
        assert!(fixed.contains("</subtask>"));
        assert!(!fixed.contains("</subtasks>"));
    }

    #[test]
    fn test_subtask_pattern_matches_subtask_closing_tag() {
        // Test that SUBTASK_PATTERN matches correct </subtask> closing tag
        let content = r#"<subtask id="1"><description>Test</description></subtask>"#;
        let captures = SUBTASK_PATTERN.captures(content);
        assert!(captures.is_some());
        let caps = captures.unwrap();
        assert_eq!(caps.get(1).unwrap().as_str(), "1");
    }

    #[test]
    fn test_subtask_pattern_after_sanitization() {
        // After sanitization, </subtasks> should be fixed to </subtask>
        let content = r#"<subtask id="2"><description>Test</description></subtasks>"#;
        let sanitized = sanitize_xml_content(content);
        let captures = SUBTASK_PATTERN.captures(&sanitized);
        assert!(captures.is_some());
        let caps = captures.unwrap();
        assert_eq!(caps.get(1).unwrap().as_str(), "2");
    }

    #[test]
    fn test_extract_task_plan_with_mismatched_closing_tag() {
        // This is the actual LLM output that was failing
        let content = r#"
<task_plan>
<goal>计算 23 + 32 + 33 的结果</goal>
<subtasks>
<subtask id="1">
<description>首先计算 23 + 32。</description>
<dependencies></dependencies>
<tools>mcp__cardea-calculator__sum</tools>
<recommended_skill>cardea-calculator</recommended_skill>
</subtask>
<subtask id="2">
<description>然后将第一步的结果与 33 相加。</description>
<dependencies>1</dependencies>
<tools>mcp__cardea-calculator__sum</tools>
<recommended_skill>cardea-calculator</recommended_skill>
</subtasks>
</subtasks>
</task_plan>
"#;
        let result = extract_task_plan(content);
        assert!(result.is_some());
        let plan = result.unwrap();
        assert_eq!(plan.subtasks.len(), 2);
        assert_eq!(plan.subtasks[0].id, "1");
        assert_eq!(plan.subtasks[1].id, "2");
    }

    #[test]
    fn test_extract_task_plan_with_correct_closing_tags() {
        // Verify normal case still works
        let content = r#"
<task_plan>
<goal>Test goal</goal>
<subtasks>
<subtask id="1">
<description>Task 1</description>
<dependencies></dependencies>
<tools>tool1</tools>
</subtask>
<subtask id="2">
<description>Task 2</description>
<dependencies>1</dependencies>
<tools>tool2</tools>
</subtask>
</subtasks>
</task_plan>
"#;
        let result = extract_task_plan(content);
        assert!(result.is_some());
        let plan = result.unwrap();
        assert_eq!(plan.subtasks.len(), 2);
    }
}
