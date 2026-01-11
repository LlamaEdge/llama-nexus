//! Integration tests for TaskPlanner Skills Integration
//!
//! This module implements the test cases defined in docs/skills/skills-test-plan.md
//! under "TaskPlanner Skills 集成" (TaskPlanner Skills Integration).
//!
//! Test cases:
//! - TTP-001: TaskPlanner.with_skills() - Skills摘要正确注入
//! - TTP-002: 规划 System Prompt 格式 - 包含Skills表格
//! - TTP-003: SubTask.recommended_skill - 子任务包含推荐Skill
//! - TTP-004: 无 Skills 时的规划 - 规划正常进行

use std::{path::Path, sync::Arc};

use async_trait::async_trait;
use tempfile::TempDir;

use super::{
    planner::{LlmProvider, PlannerMessage, SubTask, TaskPlanner, ToolDescription},
    xml_parser::extract_task_plan,
};
use crate::{
    error::ServerError,
    skills::{SkillRegistry, SkillSummary},
};

// ============================================================================
// Test Fixtures
// ============================================================================

/// Configuration for creating test skills
struct TestSkillConfig {
    description: String,
    content: String,
}

impl Default for TestSkillConfig {
    fn default() -> Self {
        Self {
            description: "A test skill".to_string(),
            content: "# Test Skill\n\nThis is a test skill.".to_string(),
        }
    }
}

/// Create a test skill directory with SKILL.md
fn create_test_skill(dir: &Path, name: &str, config: TestSkillConfig) {
    let skill_dir = dir.join(name);
    std::fs::create_dir_all(&skill_dir).unwrap();

    let front_matter = format!(
        r#"---
name: {}
description: {}
---

{}"#,
        name, config.description, config.content
    );

    std::fs::write(skill_dir.join("SKILL.md"), front_matter).unwrap();
}

/// Setup a set of test skills for TaskPlanner integration tests
fn setup_test_skills(dir: &TempDir) {
    // Weather query skill
    create_test_skill(
        dir.path(),
        "weather-query",
        TestSkillConfig {
            description: "Query weather information for any location".to_string(),
            content: r#"# Weather Query Skill

## Workflow
1. Parse location from user request
2. Call weather API
3. Format and return weather data

## Tools
- weather-api: Get current weather data"#
                .to_string(),
        },
    );

    // Code review skill
    create_test_skill(
        dir.path(),
        "code-review",
        TestSkillConfig {
            description: "Review code changes and provide feedback".to_string(),
            content: r#"# Code Review Skill

## Workflow
1. Read the code changes
2. Analyze for issues and improvements
3. Provide structured feedback

## Tools
- read_file: Read source files
- git-diff: Get code changes"#
                .to_string(),
        },
    );

    // Data analysis skill
    create_test_skill(
        dir.path(),
        "data-analysis",
        TestSkillConfig {
            description: "Analyze data and generate insights".to_string(),
            content: r#"# Data Analysis Skill

## Workflow
1. Load and validate data
2. Perform statistical analysis
3. Generate visualizations and reports"#
                .to_string(),
        },
    );
}

/// Get standard test tools for TaskPlanner
fn get_test_tools() -> Vec<ToolDescription> {
    vec![
        ToolDescription {
            name: "weather".to_string(),
            description: "Get weather information".to_string(),
        },
        ToolDescription {
            name: "read_file".to_string(),
            description: "Read file contents".to_string(),
        },
        ToolDescription {
            name: "search".to_string(),
            description: "Search for information".to_string(),
        },
    ]
}

// ============================================================================
// Mock LLM Provider for Testing
// ============================================================================

/// Mock LLM provider that returns predefined responses for testing
struct MockLlmProvider {
    /// Response to return (in TaskPlan XML format)
    response: String,
}

impl MockLlmProvider {
    fn new(response: impl Into<String>) -> Self {
        Self {
            response: response.into(),
        }
    }

    /// Create a mock that returns a simple plan with recommended skills
    fn with_skills_plan() -> Self {
        Self::new(
            r#"
<task_plan>
  <goal>Query weather for Tokyo and review the API code</goal>
  <subtasks>
    <subtask id="1">
      <description>查询东京的天气信息</description>
      <dependencies></dependencies>
      <tools>weather</tools>
      <recommended_skill>weather-query</recommended_skill>
    </subtask>
    <subtask id="2">
      <description>审查天气 API 的代码实现</description>
      <dependencies>1</dependencies>
      <tools>read_file</tools>
      <recommended_skill>code-review</recommended_skill>
    </subtask>
  </subtasks>
</task_plan>
"#,
        )
    }

    /// Create a mock that returns a plan without recommended skills
    fn without_skills_plan() -> Self {
        Self::new(
            r#"
<task_plan>
  <goal>Simple task without skills</goal>
  <subtasks>
    <subtask id="1">
      <description>执行简单的搜索操作</description>
      <dependencies></dependencies>
      <tools>search</tools>
    </subtask>
  </subtasks>
</task_plan>
"#,
        )
    }
}

#[async_trait]
impl LlmProvider for MockLlmProvider {
    async fn complete(&self, _messages: Vec<PlannerMessage>) -> Result<String, ServerError> {
        Ok(self.response.clone())
    }

    fn name(&self) -> &str {
        "mock"
    }
}

// ============================================================================
// TTP-001: TaskPlanner.with_skills() - Skills摘要正确注入
// ============================================================================

#[tokio::test]
async fn test_ttp_001_with_skills_basic() {
    // Setup test skills
    let temp_dir = TempDir::new().unwrap();
    setup_test_skills(&temp_dir);

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    registry.load_all().await.unwrap();
    let summaries = registry.get_summaries().await;

    // Create TaskPlanner with skills
    let planner = TaskPlanner::with_provider(Arc::new(MockLlmProvider::with_skills_plan()), 10)
        .with_tools(get_test_tools())
        .with_skills(summaries.clone());

    // Verify skills are set (via system prompt)
    let system_prompt = planner.build_system_prompt();

    // System prompt should contain the skills section
    assert!(
        system_prompt.contains("## 可用 Skills"),
        "System prompt should contain Skills section"
    );
}

#[tokio::test]
async fn test_ttp_001_with_skills_all_summaries_included() {
    let temp_dir = TempDir::new().unwrap();
    setup_test_skills(&temp_dir);

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    registry.load_all().await.unwrap();
    let summaries = registry.get_summaries().await;

    let planner = TaskPlanner::with_provider(Arc::new(MockLlmProvider::with_skills_plan()), 10)
        .with_skills(summaries.clone());

    let system_prompt = planner.build_system_prompt();

    // All skill summaries should be in the prompt
    for summary in &summaries {
        assert!(
            system_prompt.contains(&summary.name),
            "System prompt should contain skill name: {}",
            summary.name
        );
        assert!(
            system_prompt.contains(&summary.description),
            "System prompt should contain skill description: {}",
            summary.description
        );
    }
}

#[tokio::test]
async fn test_ttp_001_with_skills_empty_summaries() {
    // Test with empty skills list
    let planner = TaskPlanner::with_provider(Arc::new(MockLlmProvider::without_skills_plan()), 10)
        .with_tools(get_test_tools())
        .with_skills(vec![]);

    let system_prompt = planner.build_system_prompt();

    // Should not contain skills section when empty
    assert!(
        !system_prompt.contains("## 可用 Skills"),
        "System prompt should NOT contain Skills section when no skills provided"
    );
}

#[tokio::test]
async fn test_ttp_001_with_skills_chain_methods() {
    let summaries = vec![SkillSummary {
        allowed_tools: vec![],
        name: "test-skill".to_string(),
        description: "Test skill description".to_string(),
    }];

    // Test method chaining
    let planner = TaskPlanner::with_provider(Arc::new(MockLlmProvider::with_skills_plan()), 10)
        .with_tools(get_test_tools())
        .with_skills(summaries);

    let system_prompt = planner.build_system_prompt();

    assert!(system_prompt.contains("test-skill"));
    assert!(system_prompt.contains("Test skill description"));
}

// ============================================================================
// TTP-002: 规划 System Prompt 格式 - 包含Skills表格
// ============================================================================

#[tokio::test]
async fn test_ttp_002_system_prompt_contains_skills_table() {
    let summaries = vec![
        SkillSummary {
            allowed_tools: vec![],
            name: "weather-query".to_string(),
            description: "Query weather information".to_string(),
        },
        SkillSummary {
            allowed_tools: vec![],
            name: "code-review".to_string(),
            description: "Review code changes".to_string(),
        },
    ];

    let planner = TaskPlanner::with_provider(Arc::new(MockLlmProvider::with_skills_plan()), 10)
        .with_skills(summaries);

    let system_prompt = planner.build_system_prompt();

    // Should contain markdown table headers
    assert!(
        system_prompt.contains("| Skill | 描述 |"),
        "System prompt should contain Skills table header"
    );
    assert!(
        system_prompt.contains("|-------|------|"),
        "System prompt should contain table separator"
    );

    // Should contain skill entries in table format
    assert!(
        system_prompt.contains("| weather-query | Query weather information |"),
        "System prompt should contain weather-query skill row"
    );
    assert!(
        system_prompt.contains("| code-review | Review code changes |"),
        "System prompt should contain code-review skill row"
    );
}

#[tokio::test]
async fn test_ttp_002_system_prompt_recommended_skill_tag() {
    let summaries = vec![SkillSummary {
        allowed_tools: vec![],
        name: "test-skill".to_string(),
        description: "Test".to_string(),
    }];

    let planner = TaskPlanner::with_provider(Arc::new(MockLlmProvider::with_skills_plan()), 10)
        .with_skills(summaries);

    let system_prompt = planner.build_system_prompt();

    // Should include recommended_skill tag in output format section
    assert!(
        system_prompt.contains("<recommended_skill>"),
        "System prompt should mention recommended_skill tag"
    );
    assert!(
        system_prompt.contains("</recommended_skill>"),
        "System prompt should mention recommended_skill closing tag"
    );
}

#[tokio::test]
async fn test_ttp_002_system_prompt_skill_recommendation_rule() {
    let summaries = vec![SkillSummary {
        allowed_tools: vec![],
        name: "test-skill".to_string(),
        description: "Test".to_string(),
    }];

    let planner = TaskPlanner::with_provider(Arc::new(MockLlmProvider::with_skills_plan()), 10)
        .with_skills(summaries);

    let system_prompt = planner.build_system_prompt();

    // Should include rule about recommending skills
    assert!(
        system_prompt.contains("Skill"),
        "System prompt should contain skill-related rules"
    );
}

#[tokio::test]
async fn test_ttp_002_system_prompt_tools_section_preserved() {
    let summaries = vec![SkillSummary {
        allowed_tools: vec![],
        name: "test-skill".to_string(),
        description: "Test".to_string(),
    }];

    let planner = TaskPlanner::with_provider(Arc::new(MockLlmProvider::with_skills_plan()), 10)
        .with_tools(get_test_tools())
        .with_skills(summaries);

    let system_prompt = planner.build_system_prompt();

    // Should still contain tools section
    assert!(
        system_prompt.contains("## 可用工具"),
        "System prompt should still contain Tools section"
    );

    // Should contain tool names
    assert!(system_prompt.contains("weather"));
    assert!(system_prompt.contains("read_file"));
}

// ============================================================================
// TTP-003: SubTask.recommended_skill - 子任务包含推荐Skill
// ============================================================================

#[tokio::test]
async fn test_ttp_003_subtask_has_recommended_skill() {
    // Test XML parsing of recommended_skill
    let plan_xml = r#"
<task_plan>
  <goal>Query weather and review code</goal>
  <subtasks>
    <subtask id="1">
      <description>查询东京的天气信息</description>
      <dependencies></dependencies>
      <tools>weather</tools>
      <recommended_skill>weather-query</recommended_skill>
    </subtask>
    <subtask id="2">
      <description>审查代码</description>
      <dependencies>1</dependencies>
      <tools>read_file</tools>
      <recommended_skill>code-review</recommended_skill>
    </subtask>
  </subtasks>
</task_plan>
"#;

    let raw_plan = extract_task_plan(plan_xml).unwrap();

    assert_eq!(raw_plan.subtasks.len(), 2);
    assert_eq!(
        raw_plan.subtasks[0].recommended_skill,
        Some("weather-query".to_string())
    );
    assert_eq!(
        raw_plan.subtasks[1].recommended_skill,
        Some("code-review".to_string())
    );
}

#[tokio::test]
async fn test_ttp_003_subtask_optional_recommended_skill() {
    // Test that recommended_skill is optional
    let plan_xml = r#"
<task_plan>
  <goal>Mixed tasks</goal>
  <subtasks>
    <subtask id="1">
      <description>任务有推荐 Skill</description>
      <dependencies></dependencies>
      <tools>weather</tools>
      <recommended_skill>weather-query</recommended_skill>
    </subtask>
    <subtask id="2">
      <description>任务没有推荐 Skill</description>
      <dependencies>1</dependencies>
      <tools>search</tools>
    </subtask>
  </subtasks>
</task_plan>
"#;

    let raw_plan = extract_task_plan(plan_xml).unwrap();

    assert_eq!(raw_plan.subtasks.len(), 2);
    assert_eq!(
        raw_plan.subtasks[0].recommended_skill,
        Some("weather-query".to_string())
    );
    assert_eq!(raw_plan.subtasks[1].recommended_skill, None);
}

#[tokio::test]
async fn test_ttp_003_subtask_empty_recommended_skill() {
    // Test that empty recommended_skill is treated as None
    let plan_xml = r#"
<task_plan>
  <goal>Test empty skill</goal>
  <subtasks>
    <subtask id="1">
      <description>任务有空的推荐 Skill</description>
      <dependencies></dependencies>
      <tools>search</tools>
      <recommended_skill></recommended_skill>
    </subtask>
  </subtasks>
</task_plan>
"#;

    let raw_plan = extract_task_plan(plan_xml).unwrap();

    assert_eq!(raw_plan.subtasks.len(), 1);
    // Empty recommended_skill should be treated as None
    assert_eq!(raw_plan.subtasks[0].recommended_skill, None);
}

#[tokio::test]
async fn test_ttp_003_subtask_with_skill_builder() {
    // Test SubTask builder with_skill method
    let subtask = SubTask::new(1, "Test task".to_string())
        .with_dependencies(vec![0])
        .with_tools(vec!["search".to_string()])
        .with_skill(Some("test-skill".to_string()));

    assert_eq!(subtask.recommended_skill, Some("test-skill".to_string()));
}

#[tokio::test]
async fn test_ttp_003_subtask_with_skill_none() {
    let subtask = SubTask::new(1, "Test task".to_string()).with_skill(None);

    assert_eq!(subtask.recommended_skill, None);
}

#[tokio::test]
async fn test_ttp_003_plan_end_to_end_with_mock() {
    let temp_dir = TempDir::new().unwrap();
    setup_test_skills(&temp_dir);

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    registry.load_all().await.unwrap();
    let summaries = registry.get_summaries().await;

    // Create planner with mock that returns skills
    let planner = TaskPlanner::with_provider(Arc::new(MockLlmProvider::with_skills_plan()), 10)
        .with_tools(get_test_tools())
        .with_skills(summaries);

    // Execute planning
    let plan = planner.plan("查询东京天气并审查代码").await.unwrap();

    assert_eq!(plan.len(), 2);

    // Verify recommended skills are preserved in the plan
    assert_eq!(
        plan.subtasks[0].recommended_skill,
        Some("weather-query".to_string())
    );
    assert_eq!(
        plan.subtasks[1].recommended_skill,
        Some("code-review".to_string())
    );
}

// ============================================================================
// TTP-004: 无 Skills 时的规划 - 规划正常进行
// ============================================================================

#[tokio::test]
async fn test_ttp_004_planner_without_skills() {
    // Create planner without calling with_skills()
    let planner = TaskPlanner::with_provider(Arc::new(MockLlmProvider::without_skills_plan()), 10)
        .with_tools(get_test_tools());

    let system_prompt = planner.build_system_prompt();

    // Should NOT contain skills section
    assert!(
        !system_prompt.contains("## 可用 Skills"),
        "System prompt should NOT contain Skills section"
    );

    // Should still contain tools section
    assert!(
        system_prompt.contains("## 可用工具"),
        "System prompt should still contain Tools section"
    );
}

#[tokio::test]
async fn test_ttp_004_planner_without_skills_plan_works() {
    let planner = TaskPlanner::with_provider(Arc::new(MockLlmProvider::without_skills_plan()), 10)
        .with_tools(get_test_tools());

    // Planning should still work
    let plan = planner.plan("执行简单搜索").await.unwrap();

    assert!(!plan.subtasks.is_empty());
    assert_eq!(plan.subtasks[0].recommended_skill, None);
}

#[tokio::test]
async fn test_ttp_004_no_recommended_skill_tag_in_prompt() {
    let planner = TaskPlanner::with_provider(Arc::new(MockLlmProvider::without_skills_plan()), 10)
        .with_tools(get_test_tools());

    let system_prompt = planner.build_system_prompt();

    // Should NOT contain recommended_skill tag instruction when no skills
    assert!(
        !system_prompt.contains("<recommended_skill>"),
        "System prompt should NOT mention recommended_skill when no skills"
    );
}

#[tokio::test]
async fn test_ttp_004_empty_skills_same_as_no_skills() {
    // with_skills(vec![]) should behave same as not calling with_skills
    let planner_empty =
        TaskPlanner::with_provider(Arc::new(MockLlmProvider::without_skills_plan()), 10)
            .with_tools(get_test_tools())
            .with_skills(vec![]);

    let planner_none =
        TaskPlanner::with_provider(Arc::new(MockLlmProvider::without_skills_plan()), 10)
            .with_tools(get_test_tools());

    let prompt_empty = planner_empty.build_system_prompt();
    let prompt_none = planner_none.build_system_prompt();

    // Both should not contain skills section
    assert!(!prompt_empty.contains("## 可用 Skills"));
    assert!(!prompt_none.contains("## 可用 Skills"));
}

// ============================================================================
// Additional Integration Tests
// ============================================================================

#[tokio::test]
async fn test_integration_full_workflow() {
    let temp_dir = TempDir::new().unwrap();
    setup_test_skills(&temp_dir);

    // Phase 1: Load skills and get summaries
    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    let count = registry.load_all().await.unwrap();
    assert!(count >= 3, "Should load at least 3 test skills");

    let summaries = registry.get_summaries().await;
    assert!(!summaries.is_empty());

    // Phase 2: Create planner with skills
    let planner = TaskPlanner::with_provider(Arc::new(MockLlmProvider::with_skills_plan()), 10)
        .with_tools(get_test_tools())
        .with_skills(summaries.clone());

    // Verify system prompt
    let system_prompt = planner.build_system_prompt();
    assert!(system_prompt.contains("## 可用 Skills"));
    assert!(system_prompt.contains("## 可用工具"));

    // Phase 3: Execute planning
    let plan = planner.plan("查询天气并审查代码").await.unwrap();
    assert!(!plan.subtasks.is_empty());

    // Verify skills integration
    for subtask in &plan.subtasks {
        if let Some(ref skill_name) = subtask.recommended_skill {
            // The recommended skill should be in our skill registry
            let skill_exists = summaries.iter().any(|s| &s.name == skill_name);
            assert!(
                skill_exists,
                "Recommended skill '{}' should exist in registry",
                skill_name
            );
        }
    }
}

#[tokio::test]
async fn test_integration_skills_table_format_matches_injector() {
    let summaries = vec![
        SkillSummary {
            allowed_tools: vec![],
            name: "skill-a".to_string(),
            description: "Description A".to_string(),
        },
        SkillSummary {
            allowed_tools: vec![],
            name: "skill-b".to_string(),
            description: "Description B".to_string(),
        },
    ];

    let planner = TaskPlanner::with_provider(Arc::new(MockLlmProvider::with_skills_plan()), 10)
        .with_skills(summaries.clone());

    let system_prompt = planner.build_system_prompt();

    // The system prompt should use the same table format as SkillInjector
    // Both should use markdown table format with Skill | Description
    assert!(system_prompt.contains("| Skill | 描述 |") || system_prompt.contains("| skill-a |"));
}

#[tokio::test]
async fn test_raw_plan_parsing_preserves_all_fields() {
    let plan_xml = r#"
<task_plan>
  <goal>Complete complex task</goal>
  <subtasks>
    <subtask id="1">
      <description>First step</description>
      <dependencies></dependencies>
      <tools>tool1 tool2</tools>
      <recommended_skill>skill-a</recommended_skill>
    </subtask>
    <subtask id="2">
      <description>Second step</description>
      <dependencies>1</dependencies>
      <tools>tool3</tools>
      <recommended_skill>skill-b</recommended_skill>
    </subtask>
    <subtask id="3">
      <description>Third step without skill</description>
      <dependencies>1 2</dependencies>
      <tools>tool4</tools>
    </subtask>
  </subtasks>
</task_plan>
"#;

    let raw_plan = extract_task_plan(plan_xml).unwrap();

    assert_eq!(raw_plan.goal, "Complete complex task");
    assert_eq!(raw_plan.subtasks.len(), 3);

    // First subtask
    assert_eq!(raw_plan.subtasks[0].id, "1");
    assert_eq!(raw_plan.subtasks[0].description, "First step");
    assert!(raw_plan.subtasks[0].dependencies.is_empty());
    assert_eq!(raw_plan.subtasks[0].tools, vec!["tool1", "tool2"]);
    assert_eq!(
        raw_plan.subtasks[0].recommended_skill,
        Some("skill-a".to_string())
    );

    // Second subtask
    assert_eq!(raw_plan.subtasks[1].id, "2");
    assert_eq!(raw_plan.subtasks[1].dependencies, vec!["1"]);
    assert_eq!(
        raw_plan.subtasks[1].recommended_skill,
        Some("skill-b".to_string())
    );

    // Third subtask (no recommended skill)
    assert_eq!(raw_plan.subtasks[2].id, "3");
    assert_eq!(raw_plan.subtasks[2].dependencies, vec!["1", "2"]);
    assert_eq!(raw_plan.subtasks[2].recommended_skill, None);
}

// ============================================================================
// Performance / Edge Case Tests
// ============================================================================

#[tokio::test]
async fn test_many_skills_in_prompt() {
    // Test with many skills to ensure table formatting works
    let mut summaries = Vec::new();
    for i in 0..20 {
        summaries.push(SkillSummary {
            allowed_tools: vec![],
            name: format!("skill-{:02}", i),
            description: format!("Description for skill {}", i),
        });
    }

    let planner = TaskPlanner::with_provider(Arc::new(MockLlmProvider::with_skills_plan()), 10)
        .with_skills(summaries.clone());

    let system_prompt = planner.build_system_prompt();

    // All skills should be present
    for summary in &summaries {
        assert!(
            system_prompt.contains(&summary.name),
            "System prompt should contain: {}",
            summary.name
        );
    }
}

#[tokio::test]
async fn test_skill_name_with_special_characters() {
    let summaries = vec![SkillSummary {
        allowed_tools: vec![],
        name: "my-awesome_skill.v2".to_string(),
        description: "A skill with special chars in name".to_string(),
    }];

    let planner = TaskPlanner::with_provider(Arc::new(MockLlmProvider::with_skills_plan()), 10)
        .with_skills(summaries);

    let system_prompt = planner.build_system_prompt();

    assert!(system_prompt.contains("my-awesome_skill.v2"));
}

#[tokio::test]
async fn test_skill_description_with_pipe_character() {
    // Pipe character could break markdown table
    let summaries = vec![SkillSummary {
        allowed_tools: vec![],
        name: "test-skill".to_string(),
        description: "Query | Filter | Transform data".to_string(),
    }];

    let planner = TaskPlanner::with_provider(Arc::new(MockLlmProvider::with_skills_plan()), 10)
        .with_skills(summaries);

    let system_prompt = planner.build_system_prompt();

    // Table should still contain the skill
    assert!(system_prompt.contains("test-skill"));
}

#[tokio::test]
async fn test_concurrent_planning_with_skills() {
    let summaries = vec![SkillSummary {
        allowed_tools: vec![],
        name: "shared-skill".to_string(),
        description: "A shared skill".to_string(),
    }];

    // Create multiple planners sharing the same skill data
    let mut handles = Vec::new();

    for i in 0..5 {
        let summaries_clone = summaries.clone();
        let handle = tokio::spawn(async move {
            let planner =
                TaskPlanner::with_provider(Arc::new(MockLlmProvider::without_skills_plan()), 10)
                    .with_skills(summaries_clone);

            let prompt = planner.build_system_prompt();
            assert!(prompt.contains("shared-skill"));
            i
        });
        handles.push(handle);
    }

    // Wait for all to complete
    for handle in handles {
        handle.await.unwrap();
    }
}

// ============================================================================
// Build System Prompt Visibility Test
// ============================================================================

/// This test verifies that build_system_prompt is accessible (it's private in the actual code)
/// If this test fails to compile, we need to make build_system_prompt pub(crate) or add a test helper
#[tokio::test]
async fn test_build_system_prompt_accessible() {
    let planner = TaskPlanner::with_provider(Arc::new(MockLlmProvider::without_skills_plan()), 10);

    // This should compile - proving build_system_prompt is accessible
    let _prompt = planner.build_system_prompt();
}
