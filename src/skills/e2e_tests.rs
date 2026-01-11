//! End-to-end tests for the Skills module
//!
//! These tests verify the complete skills workflow from loading to injection.

use std::path::Path;

use tempfile::TempDir;

use super::{SkillDetector, SkillInjector, SkillRegistry};

// ============================================================================
// Test Fixtures
// ============================================================================

/// Configuration for creating test skills
struct TestSkillConfig {
    description: String,
    license: Option<String>,
    compatibility: Option<String>,
    allowed_tools: Option<String>,
    content: String,
    with_scripts: bool,
    with_references: bool,
    with_assets: bool,
}

impl Default for TestSkillConfig {
    fn default() -> Self {
        Self {
            description: "A test skill".to_string(),
            license: None,
            compatibility: None,
            allowed_tools: None,
            content: "# Test Skill\n\nThis is a test skill.".to_string(),
            with_scripts: false,
            with_references: false,
            with_assets: false,
        }
    }
}

/// Create a complete test skill directory with all standard components
fn create_complete_skill(dir: &Path, name: &str, config: TestSkillConfig) {
    let skill_dir = dir.join(name);
    std::fs::create_dir_all(&skill_dir).unwrap();

    // Create SKILL.md
    let mut front_matter = format!(
        r#"---
name: {}
description: {}
"#,
        name, config.description
    );

    if let Some(license) = config.license {
        front_matter.push_str(&format!("license: {}\n", license));
    }

    if let Some(compatibility) = config.compatibility {
        front_matter.push_str(&format!("compatibility: {}\n", compatibility));
    }

    if let Some(allowed_tools) = config.allowed_tools {
        front_matter.push_str(&format!("allowed-tools: {}\n", allowed_tools));
    }

    front_matter.push_str("---\n\n");
    front_matter.push_str(&config.content);

    std::fs::write(skill_dir.join("SKILL.md"), front_matter).unwrap();

    // Create optional directories
    if config.with_scripts {
        let scripts_dir = skill_dir.join("scripts");
        std::fs::create_dir_all(&scripts_dir).unwrap();
        std::fs::write(scripts_dir.join("helper.sh"), "#!/bin/bash\necho 'Hello'").unwrap();
    }

    if config.with_references {
        let refs_dir = skill_dir.join("references");
        std::fs::create_dir_all(&refs_dir).unwrap();
        std::fs::write(
            refs_dir.join("api-docs.md"),
            "# API Documentation\n\nSome docs.",
        )
        .unwrap();
    }

    if config.with_assets {
        let assets_dir = skill_dir.join("assets");
        std::fs::create_dir_all(&assets_dir).unwrap();
        std::fs::write(assets_dir.join("template.md"), "# Template\n\n{}").unwrap();
    }
}

// ============================================================================
// E2E Test: Complete Skill Loading Flow
// ============================================================================

#[tokio::test]
async fn test_e2e_skill_loading_complete_flow() {
    let temp_dir = TempDir::new().unwrap();

    // Create skills with different configurations
    create_complete_skill(
        temp_dir.path(),
        "weather-query",
        TestSkillConfig {
            description: "Query weather information for cities".to_string(),
            license: Some("MIT".to_string()),
            compatibility: Some("Requires weather MCP server".to_string()),
            allowed_tools: Some("weather forecast".to_string()),
            content: "# Weather Query\n\nUse weather tools to get forecasts.".to_string(),
            with_scripts: true,
            with_references: true,
            with_assets: true,
        },
    );

    create_complete_skill(
        temp_dir.path(),
        "code-review",
        TestSkillConfig {
            description: "Review code changes and provide feedback".to_string(),
            license: Some("Apache-2.0".to_string()),
            allowed_tools: Some("Bash(git:*) Read".to_string()),
            content: "# Code Review\n\nAnalyze code for issues.".to_string(),
            ..Default::default()
        },
    );

    create_complete_skill(
        temp_dir.path(),
        "simple-task",
        TestSkillConfig {
            description: "A simple skill without tool restrictions".to_string(),
            content: "# Simple Task\n\nHandle basic tasks.".to_string(),
            ..Default::default()
        },
    );

    // Load all skills
    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    let count = registry.load_all().await.unwrap();
    assert_eq!(count, 3);

    // Verify all skills loaded correctly
    assert!(registry.exists("weather-query").await);
    assert!(registry.exists("code-review").await);
    assert!(registry.exists("simple-task").await);

    // Verify skill metadata
    let weather_skill = registry.get("weather-query").await.unwrap();
    assert_eq!(weather_skill.metadata.license, Some("MIT".to_string()));
    assert_eq!(
        weather_skill.metadata.allowed_tools,
        Some("weather forecast".to_string())
    );

    // Verify summaries
    let summaries = registry.get_summaries().await;
    assert_eq!(summaries.len(), 3);
}

// ============================================================================
// E2E Test: Two-Phase Loading Workflow
// ============================================================================

#[tokio::test]
async fn test_e2e_two_phase_loading_workflow() {
    let temp_dir = TempDir::new().unwrap();

    create_complete_skill(
        temp_dir.path(),
        "data-analysis",
        TestSkillConfig {
            description: "Analyze data and generate reports".to_string(),
            allowed_tools: Some("python pandas matplotlib".to_string()),
            content: "# Data Analysis\n\n## Steps\n1. Load data\n2. Analyze\n3. Report".to_string(),
            ..Default::default()
        },
    );

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    registry.load_all().await.unwrap();

    // Phase 1: Get summaries for initial prompt
    let summaries = registry.get_summaries().await;
    let phase1_prompt = SkillInjector::phase1_injection(&summaries);

    // Verify Phase 1 prompt content
    assert!(phase1_prompt.contains("## Available Skills"));
    assert!(phase1_prompt.contains("data-analysis"));
    assert!(phase1_prompt.contains("Analyze data and generate reports"));
    assert!(phase1_prompt.contains("<use_skill>skill-name</use_skill>"));

    // Simulate LLM response requesting skill
    let llm_response = "I will use <use_skill>data-analysis</use_skill> to analyze the data.";

    // Detect skill request
    let detected_skill = SkillDetector::detect_first(llm_response);
    assert_eq!(detected_skill, Some("data-analysis".to_string()));

    // Phase 2: Load full skill content
    let skill = registry.get("data-analysis").await.unwrap();
    let phase2_prompt = SkillInjector::phase2_injection(&skill);

    // Verify Phase 2 prompt content
    assert!(phase2_prompt.contains("## Active Skill: data-analysis"));
    assert!(phase2_prompt.contains("---")); // Content wrapped in delimiters
    assert!(phase2_prompt.contains("# Data Analysis"));
    assert!(phase2_prompt.contains("1. Load data"));
}

// ============================================================================
// E2E Test: Skill Detection and Cleanup
// ============================================================================

#[tokio::test]
async fn test_e2e_skill_detection_and_cleanup() {
    // Test various LLM response formats

    // Simple tag
    let response1 = "<use_skill>weather-query</use_skill>";
    assert_eq!(
        SkillDetector::detect_first(response1),
        Some("weather-query".to_string())
    );

    // Tag with surrounding text
    let response2 = "I'll use <use_skill>code-review</use_skill> to analyze the changes.";
    assert_eq!(
        SkillDetector::detect_first(response2),
        Some("code-review".to_string())
    );
    let cleaned2 = SkillDetector::strip_tags(response2);
    assert!(!cleaned2.contains("<use_skill>"));
    assert!(cleaned2.contains("I'll use"));
    assert!(cleaned2.contains("to analyze the changes."));

    // Multiple tags
    let response3 = "<use_skill>skill-one</use_skill> then <use_skill>skill-two</use_skill>";
    let skills = SkillDetector::detect(response3);
    assert_eq!(skills.len(), 2);
    assert_eq!(skills[0], "skill-one");
    assert_eq!(skills[1], "skill-two");

    // Tag with whitespace
    let response4 = "<use_skill>  spaced-skill  </use_skill>";
    assert_eq!(
        SkillDetector::detect_first(response4),
        Some("spaced-skill".to_string())
    );

    // No tags
    let response5 = "Just a regular response without any skill tags.";
    assert!(SkillDetector::detect_first(response5).is_none());

    // Extract and clean combined
    let response6 = "Using <use_skill>my-skill</use_skill> for this task.";
    let (skills, cleaned) = SkillDetector::extract_and_clean(response6);
    assert_eq!(skills, vec!["my-skill"]);
    assert!(!cleaned.contains("my-skill"));
}

// ============================================================================
// E2E Test: Multi-Skill Subtask Execution
// ============================================================================

#[tokio::test]
async fn test_e2e_multi_skill_subtask_execution() {
    let temp_dir = TempDir::new().unwrap();

    // Create multiple skills for different subtasks
    create_complete_skill(
        temp_dir.path(),
        "research",
        TestSkillConfig {
            description: "Research topics and gather information".to_string(),
            content: "# Research\n\nGather information from various sources.".to_string(),
            ..Default::default()
        },
    );

    create_complete_skill(
        temp_dir.path(),
        "summarize",
        TestSkillConfig {
            description: "Summarize content into key points".to_string(),
            content: "# Summarize\n\nExtract key points from content.".to_string(),
            ..Default::default()
        },
    );

    create_complete_skill(
        temp_dir.path(),
        "report",
        TestSkillConfig {
            description: "Generate formatted reports".to_string(),
            content: "# Report\n\nCreate well-formatted reports.".to_string(),
            ..Default::default()
        },
    );

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    registry.load_all().await.unwrap();

    // Simulate multi-subtask workflow

    // Subtask 1: Research phase
    let research_skill = registry.get("research").await.unwrap();
    let research_prompt = SkillInjector::phase2_injection(&research_skill);
    assert!(research_prompt.contains("## Active Skill: research"));
    assert!(research_prompt.contains("Gather information"));

    // Subtask 2: Summarize phase
    let summarize_skill = registry.get("summarize").await.unwrap();
    let summarize_prompt = SkillInjector::phase2_injection(&summarize_skill);
    assert!(summarize_prompt.contains("## Active Skill: summarize"));
    assert!(summarize_prompt.contains("Extract key points"));

    // Subtask 3: Report phase
    let report_skill = registry.get("report").await.unwrap();
    let report_prompt = SkillInjector::phase2_injection(&report_skill);
    assert!(report_prompt.contains("## Active Skill: report"));
    assert!(report_prompt.contains("well-formatted reports"));
}

// ============================================================================
// E2E Test: Tool Filtering
// ============================================================================

#[tokio::test]
async fn test_e2e_tool_filtering() {
    let temp_dir = TempDir::new().unwrap();

    // Create skill with specific tool restrictions
    create_complete_skill(
        temp_dir.path(),
        "git-helper",
        TestSkillConfig {
            description: "Git operations helper".to_string(),
            allowed_tools: Some("Bash(git:*) Read Write".to_string()),
            content: "# Git Helper\n\nPerform git operations.".to_string(),
            ..Default::default()
        },
    );

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    registry.load_all().await.unwrap();

    let skill = registry.get("git-helper").await.unwrap();
    let allowed_tools = skill.metadata.get_allowed_tools();

    // Verify tool patterns parsed correctly
    assert_eq!(allowed_tools.len(), 3);
    assert!(allowed_tools.contains(&"Bash(git:*)".to_string()));
    assert!(allowed_tools.contains(&"Read".to_string()));
    assert!(allowed_tools.contains(&"Write".to_string()));
}

// ============================================================================
// E2E Test: Skill Enable/Disable
// ============================================================================

#[tokio::test]
async fn test_e2e_skill_enable_disable() {
    let temp_dir = TempDir::new().unwrap();

    create_complete_skill(
        temp_dir.path(),
        "toggleable",
        TestSkillConfig {
            description: "A skill that can be toggled".to_string(),
            ..Default::default()
        },
    );

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    registry.load_all().await.unwrap();

    // Initially enabled
    let summaries = registry.get_summaries().await;
    assert_eq!(summaries.len(), 1);

    // Disable skill
    registry.set_enabled("toggleable", false).await.unwrap();
    let summaries = registry.get_summaries().await;
    assert_eq!(summaries.len(), 0);

    // Skill still exists but not in summaries
    assert!(registry.exists("toggleable").await);
    assert!(registry.get("toggleable").await.is_some());

    // Re-enable skill
    registry.set_enabled("toggleable", true).await.unwrap();
    let summaries = registry.get_summaries().await;
    assert_eq!(summaries.len(), 1);
}

// ============================================================================
// E2E Test: Skill Reload
// ============================================================================

#[tokio::test]
async fn test_e2e_skill_reload() {
    let temp_dir = TempDir::new().unwrap();

    create_complete_skill(
        temp_dir.path(),
        "mutable",
        TestSkillConfig {
            description: "Original description".to_string(),
            content: "# Original Content".to_string(),
            ..Default::default()
        },
    );

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    registry.load_all().await.unwrap();

    // Verify original content
    let skill = registry.get("mutable").await.unwrap();
    assert_eq!(skill.metadata.description, "Original description");
    assert!(skill.content.contains("Original Content"));

    // Modify the skill file
    let skill_dir = temp_dir.path().join("mutable");
    let new_content = r#"---
name: mutable
description: Updated description
---

# Updated Content

New instructions here.
"#;
    std::fs::write(skill_dir.join("SKILL.md"), new_content).unwrap();

    // Reload the skill
    registry.reload("mutable").await.unwrap();

    // Verify updated content
    let skill = registry.get("mutable").await.unwrap();
    assert_eq!(skill.metadata.description, "Updated description");
    assert!(skill.content.contains("Updated Content"));
}

// ============================================================================
// E2E Test: Complete Workflow Integration
// ============================================================================

#[tokio::test]
async fn test_e2e_complete_workflow_integration() {
    let temp_dir = TempDir::new().unwrap();

    // Create a realistic skill setup
    create_complete_skill(
        temp_dir.path(),
        "web-search",
        TestSkillConfig {
            description: "Search the web for information".to_string(),
            license: Some("MIT".to_string()),
            compatibility: Some("Requires internet access".to_string()),
            allowed_tools: Some("WebSearch WebFetch".to_string()),
            content: r#"# Web Search Skill

## Purpose
Search the web for up-to-date information.

## Usage
1. Formulate search query
2. Execute search
3. Parse results
4. Summarize findings

## Output Format
- List key findings
- Include sources
"#
            .to_string(),
            with_references: true,
            ..Default::default()
        },
    );

    // Step 1: Initialize registry and load skills
    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    let count = registry.load_all().await.unwrap();
    assert_eq!(count, 1);

    // Step 2: Get summaries for planning phase
    let summaries = registry.get_summaries().await;
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].name, "web-search");

    // Step 3: Generate Phase 1 prompt
    let base_prompt = "You are a helpful assistant.";
    let phase1_prompt = SkillInjector::inject_summaries(base_prompt, &summaries);
    assert!(phase1_prompt.contains("You are a helpful assistant."));
    assert!(phase1_prompt.contains("web-search"));
    assert!(phase1_prompt.contains("Search the web for information"));

    // Step 4: Simulate LLM requesting the skill
    let llm_response = r#"I need to search for current information.
<use_skill>web-search</use_skill>
Let me proceed with the search."#;

    // Step 5: Detect skill request
    let (skills, cleaned_response) = SkillDetector::extract_and_clean(llm_response);
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0], "web-search");
    assert!(cleaned_response.contains("I need to search"));
    assert!(!cleaned_response.contains("<use_skill>"));

    // Step 6: Load full skill for Phase 2
    let skill = registry.get(&skills[0]).await.unwrap();
    assert_eq!(skill.metadata.name, "web-search");

    // Step 7: Generate Phase 2 prompt with full skill content
    let phase2_prompt = SkillInjector::inject_skill(base_prompt, &skill);
    assert!(phase2_prompt.contains("## Active Skill: web-search"));
    assert!(phase2_prompt.contains("---")); // Content wrapped in delimiters
    assert!(phase2_prompt.contains("# Web Search Skill"));
    assert!(phase2_prompt.contains("Formulate search query"));

    // Step 8: Verify tool filtering
    let allowed_tools = skill.metadata.get_allowed_tools();
    assert_eq!(allowed_tools.len(), 2);
    assert!(allowed_tools.contains(&"WebSearch".to_string()));
    assert!(allowed_tools.contains(&"WebFetch".to_string()));
}

// ============================================================================
// E2E Test: Error Handling
// ============================================================================

#[tokio::test]
async fn test_e2e_error_handling() {
    let temp_dir = TempDir::new().unwrap();

    // Create an invalid skill (missing required fields)
    let invalid_dir = temp_dir.path().join("invalid-skill");
    std::fs::create_dir_all(&invalid_dir).unwrap();
    std::fs::write(
        invalid_dir.join("SKILL.md"),
        r#"---
name: invalid-skill
---
# Missing description
"#,
    )
    .unwrap();

    // Create a valid skill
    create_complete_skill(temp_dir.path(), "valid-skill", TestSkillConfig::default());

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    let count = registry.load_all().await.unwrap();

    // Only valid skill should be loaded
    assert_eq!(count, 1);
    assert!(registry.exists("valid-skill").await);
    assert!(!registry.exists("invalid-skill").await);
}

// ============================================================================
// E2E Test: Parser Edge Cases
// ============================================================================

#[tokio::test]
async fn test_e2e_parser_edge_cases() {
    let temp_dir = TempDir::new().unwrap();

    // Skill with multiline description
    let skill1_dir = temp_dir.path().join("multiline-desc");
    std::fs::create_dir_all(&skill1_dir).unwrap();
    std::fs::write(
        skill1_dir.join("SKILL.md"),
        r#"---
name: multiline-desc
description: |
  This is a multiline
  description that spans
  multiple lines
---
# Content
"#,
    )
    .unwrap();

    // Skill with special characters in content
    let skill2_dir = temp_dir.path().join("special-chars");
    std::fs::create_dir_all(&skill2_dir).unwrap();
    std::fs::write(
        skill2_dir.join("SKILL.md"),
        r#"---
name: special-chars
description: Handle special characters
---
# Special Characters

Code with backticks: `code here`
Bold: **bold text**
Italic: *italic*

```python
def hello():
    print("Hello, World!")
```
"#,
    )
    .unwrap();

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    let count = registry.load_all().await.unwrap();
    assert_eq!(count, 2);

    // Verify multiline description
    let skill1 = registry.get("multiline-desc").await.unwrap();
    assert!(skill1.metadata.description.contains("multiline"));

    // Verify special characters preserved
    let skill2 = registry.get("special-chars").await.unwrap();
    assert!(skill2.content.contains("```python"));
    assert!(skill2.content.contains("def hello():"));
}

// ============================================================================
// E2E Test: Concurrent Access
// ============================================================================

#[tokio::test]
async fn test_e2e_concurrent_access() {
    let temp_dir = TempDir::new().unwrap();

    for i in 0..5 {
        create_complete_skill(
            temp_dir.path(),
            &format!("skill-{}", i),
            TestSkillConfig {
                description: format!("Skill number {}", i),
                ..Default::default()
            },
        );
    }

    let registry = std::sync::Arc::new(SkillRegistry::new(temp_dir.path().to_path_buf()));
    registry.load_all().await.unwrap();

    // Spawn multiple concurrent tasks
    let mut handles = vec![];

    for i in 0..5 {
        let reg = registry.clone();
        let handle = tokio::spawn(async move {
            // Each task accesses skills
            let skill_name = format!("skill-{}", i);
            let skill = reg.get(&skill_name).await;
            assert!(skill.is_some());

            // Get summaries
            let summaries = reg.get_summaries().await;
            assert_eq!(summaries.len(), 5);

            skill_name
        });
        handles.push(handle);
    }

    // Wait for all tasks
    for handle in handles {
        let result = handle.await.unwrap();
        assert!(result.starts_with("skill-"));
    }
}

// ============================================================================
// Performance Benchmark: Skill Loading
// ============================================================================

#[tokio::test]
async fn test_performance_skill_loading() {
    let temp_dir = TempDir::new().unwrap();

    // Create 20 skills
    for i in 0..20 {
        create_complete_skill(
            temp_dir.path(),
            &format!("perf-skill-{}", i),
            TestSkillConfig {
                description: format!("Performance test skill {}", i),
                content: "# Content\n\n".repeat(10), // Some content
                with_scripts: true,
                with_references: true,
                ..Default::default()
            },
        );
    }

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());

    // Measure loading time
    let start = std::time::Instant::now();
    let count = registry.load_all().await.unwrap();
    let duration = start.elapsed();

    assert_eq!(count, 20);

    // Loading 20 skills should complete in reasonable time (< 1 second)
    assert!(
        duration.as_millis() < 1000,
        "Skill loading took too long: {:?}",
        duration
    );

    // Measure summary generation time
    let start = std::time::Instant::now();
    let _summaries = registry.get_summaries().await;
    let duration = start.elapsed();

    // Summary generation should be very fast (< 10ms)
    assert!(
        duration.as_millis() < 10,
        "Summary generation took too long: {:?}",
        duration
    );
}

// ============================================================================
// Performance Benchmark: Injection
// ============================================================================

#[tokio::test]
async fn test_performance_injection() {
    let temp_dir = TempDir::new().unwrap();

    // Create a large skill
    let large_content = "# Large Skill\n\n".to_string() + &"Section content.\n\n".repeat(100);

    create_complete_skill(
        temp_dir.path(),
        "large-skill",
        TestSkillConfig {
            description: "A large skill with lots of content".to_string(),
            content: large_content,
            ..Default::default()
        },
    );

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    registry.load_all().await.unwrap();

    let skill = registry.get("large-skill").await.unwrap();

    // Measure injection time
    let start = std::time::Instant::now();
    for _ in 0..100 {
        let _ = SkillInjector::phase2_injection(&skill);
    }
    let duration = start.elapsed();

    // 100 injections should complete in reasonable time (< 100ms)
    assert!(
        duration.as_millis() < 100,
        "Injection took too long: {:?}",
        duration
    );

    // Measure detection time
    let text_with_skill = "Some text <use_skill>large-skill</use_skill> more text".repeat(10);
    let start = std::time::Instant::now();
    for _ in 0..1000 {
        let _ = SkillDetector::detect(&text_with_skill);
    }
    let duration = start.elapsed();

    // 1000 detections should complete quickly (< 200ms)
    // Note: threshold is relaxed for CI/debug builds
    assert!(
        duration.as_millis() < 200,
        "Detection took too long: {:?}",
        duration
    );
}

// ============================================================================
// TE2E-002: Skill Recommendation in Planning Phase
// ============================================================================

#[tokio::test]
async fn test_te2e_002_skill_recommendation_in_planning() {
    let temp_dir = TempDir::new().unwrap();

    // Create skills with different capabilities
    create_complete_skill(
        temp_dir.path(),
        "weather-query",
        TestSkillConfig {
            description: "Query weather information from various sources".to_string(),
            allowed_tools: Some("WebFetch WebSearch".to_string()),
            content: "# Weather Query\n\nFetch weather data.".to_string(),
            ..Default::default()
        },
    );

    create_complete_skill(
        temp_dir.path(),
        "code-review",
        TestSkillConfig {
            description: "Review code changes and provide feedback".to_string(),
            allowed_tools: Some("Read Grep".to_string()),
            content: "# Code Review\n\nAnalyze code quality.".to_string(),
            ..Default::default()
        },
    );

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    registry.load_all().await.unwrap();

    let summaries = registry.get_summaries().await;

    // Verify summaries are available for planning
    assert_eq!(summaries.len(), 2);

    // Verify Phase 1 injection includes all skills
    let phase1_prompt = SkillInjector::phase1_injection(&summaries);
    assert!(phase1_prompt.contains("## Available Skills"));
    assert!(phase1_prompt.contains("weather-query"));
    assert!(phase1_prompt.contains("code-review"));
    assert!(phase1_prompt.contains("<use_skill>"));

    // Verify skill descriptions are present
    let weather_summary = summaries.iter().find(|s| s.name == "weather-query");
    assert!(weather_summary.is_some());
    assert_eq!(
        weather_summary.unwrap().description,
        "Query weather information from various sources"
    );
}

// ============================================================================
// TE2E-005: References Auto Loading
// ============================================================================

#[tokio::test]
async fn test_te2e_005_references_auto_loading() {
    use super::SkillLoader;

    let temp_dir = TempDir::new().unwrap();

    // Create skill with references directory
    let skill_dir = temp_dir.path().join("ref-skill");
    let refs_dir = skill_dir.join("references");
    std::fs::create_dir_all(&refs_dir).unwrap();

    std::fs::write(
        skill_dir.join("SKILL.md"),
        r#"---
name: ref-skill
description: Skill with reference documents
---
# Reference Skill

This skill uses external reference documents.
"#,
    )
    .unwrap();

    // Create multiple reference documents
    std::fs::write(
        refs_dir.join("api-docs.md"),
        "# API Documentation\n\nAPI endpoints here.",
    )
    .unwrap();
    std::fs::write(
        refs_dir.join("examples.md"),
        "# Examples\n\nCode examples here.",
    )
    .unwrap();

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    registry.load_all().await.unwrap();

    let skill = registry.get("ref-skill").await.unwrap();

    // Verify references directory exists and has files
    let references = SkillLoader::load_references(&skill.skill_dir).await;
    assert!(!references.is_empty());

    // Verify Phase 2 injection with auto refs includes reference content
    let phase2_content = SkillInjector::phase2_injection_auto_refs(&skill, 0).await;
    assert!(phase2_content.contains("ref-skill"));
    assert!(phase2_content.contains("Reference Skill"));
    // Reference content should be included
    assert!(phase2_content.contains("API Documentation") || phase2_content.contains("Examples"));
}

// ============================================================================
// TE2E-006: Tool Restriction Enforcement
// ============================================================================

#[tokio::test]
async fn test_te2e_006_tool_restriction_enforcement() {
    let temp_dir = TempDir::new().unwrap();

    create_complete_skill(
        temp_dir.path(),
        "restricted-skill",
        TestSkillConfig {
            description: "Skill with strict tool restrictions".to_string(),
            allowed_tools: Some("weather forecast".to_string()),
            content: "# Restricted Skill\n\nOnly weather tools allowed.".to_string(),
            ..Default::default()
        },
    );

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    registry.load_all().await.unwrap();

    let skill = registry.get("restricted-skill").await.unwrap();
    let allowed = skill.metadata.get_allowed_tools();

    // Verify tool restrictions parsed correctly
    assert_eq!(allowed, vec!["weather", "forecast"]);

    // Simulate tool filtering logic
    let all_tools = vec!["weather", "forecast", "email", "calendar", "bash"];
    let filtered: Vec<_> = all_tools
        .iter()
        .filter(|t| allowed.contains(&t.to_string()))
        .collect();

    assert_eq!(filtered.len(), 2);
    assert!(filtered.contains(&&"weather"));
    assert!(filtered.contains(&&"forecast"));
    assert!(!filtered.contains(&&"email"));
    assert!(!filtered.contains(&&"bash"));
}

// ============================================================================
// TE2E-007: Skill Hot Reload
// ============================================================================

#[tokio::test]
async fn test_te2e_007_hot_reload() {
    let temp_dir = TempDir::new().unwrap();

    create_complete_skill(
        temp_dir.path(),
        "hot-reload-skill",
        TestSkillConfig {
            description: "Original description".to_string(),
            content: "# Original Content\n\nOriginal instructions.".to_string(),
            ..Default::default()
        },
    );

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    registry.load_all().await.unwrap();

    // Verify original content
    let skill = registry.get("hot-reload-skill").await.unwrap();
    assert_eq!(skill.metadata.description, "Original description");
    assert!(skill.content.contains("Original Content"));

    // Modify SKILL.md on disk
    let skill_path = temp_dir.path().join("hot-reload-skill/SKILL.md");
    std::fs::write(
        &skill_path,
        r#"---
name: hot-reload-skill
description: Updated description for Plan mode
---
# Updated Content

New instructions for updated skill.
"#,
    )
    .unwrap();

    // Reload the specific skill
    registry.reload("hot-reload-skill").await.unwrap();

    // Verify updated content
    let skill = registry.get("hot-reload-skill").await.unwrap();
    assert_eq!(
        skill.metadata.description,
        "Updated description for Plan mode"
    );
    assert!(skill.content.contains("Updated Content"));

    // Verify summaries are also updated
    let summaries = registry.get_summaries().await;
    let summary = summaries
        .iter()
        .find(|s| s.name == "hot-reload-skill")
        .unwrap();
    assert_eq!(summary.description, "Updated description for Plan mode");
}

// ============================================================================
// TMS-004: Multi-Skill Activation in Single Subtask
// ============================================================================

#[tokio::test]
async fn test_tms_004_multi_skill_activation() {
    let temp_dir = TempDir::new().unwrap();

    create_complete_skill(
        temp_dir.path(),
        "weather-query",
        TestSkillConfig {
            description: "Query weather information".to_string(),
            content: "# Weather Query\n\nGet weather forecasts.".to_string(),
            ..Default::default()
        },
    );

    create_complete_skill(
        temp_dir.path(),
        "code-review",
        TestSkillConfig {
            description: "Review code changes".to_string(),
            content: "# Code Review\n\nAnalyze code quality.".to_string(),
            ..Default::default()
        },
    );

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    registry.load_all().await.unwrap();

    // Simulate LLM response requesting multiple skills
    let llm_response =
        "I'll use <use_skill>weather-query</use_skill> and <use_skill>code-review</use_skill>.";
    let detected = SkillDetector::detect(llm_response);

    assert_eq!(detected.len(), 2);
    assert!(detected.contains(&"weather-query".to_string()));
    assert!(detected.contains(&"code-review".to_string()));

    // Load all detected skills
    let mut skills = Vec::new();
    for name in &detected {
        if let Some(skill) = registry.get(name).await {
            skills.push(skill);
        }
    }
    assert_eq!(skills.len(), 2);

    // Generate combined Phase 2 injection for multiple skills
    let combined_content: Vec<_> = skills
        .iter()
        .map(|s| SkillInjector::phase2_injection(s))
        .collect();

    // Verify both skills are included
    let full_content = combined_content.join("\n\n---\n\n");
    assert!(full_content.contains("weather-query"));
    assert!(full_content.contains("code-review"));
    assert!(full_content.contains("Weather Query"));
    assert!(full_content.contains("Code Review"));
}

// ============================================================================
// TER-001: Invalid SKILL.md Error Handling
// ============================================================================

#[tokio::test]
async fn test_ter_001_invalid_skill_md() {
    let temp_dir = TempDir::new().unwrap();

    // Create invalid skill - not valid YAML frontmatter
    let invalid_dir = temp_dir.path().join("invalid-yaml");
    std::fs::create_dir_all(&invalid_dir).unwrap();
    std::fs::write(
        invalid_dir.join("SKILL.md"),
        "This is not valid YAML frontmatter content",
    )
    .unwrap();

    // Create another invalid skill - malformed YAML
    let malformed_dir = temp_dir.path().join("malformed-yaml");
    std::fs::create_dir_all(&malformed_dir).unwrap();
    std::fs::write(
        malformed_dir.join("SKILL.md"),
        r#"---
name: [invalid yaml array
description: missing bracket
---
# Content
"#,
    )
    .unwrap();

    // Create a valid skill for comparison
    create_complete_skill(temp_dir.path(), "valid-skill", TestSkillConfig::default());

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    let count = registry.load_all().await.unwrap();

    // Only valid skill should be loaded
    assert_eq!(count, 1);
    assert!(registry.exists("valid-skill").await);
    assert!(!registry.exists("invalid-yaml").await);
    assert!(!registry.exists("malformed-yaml").await);
}

// ============================================================================
// TER-002: Missing Required Fields Error Handling
// ============================================================================

#[tokio::test]
async fn test_ter_002_missing_required_fields() {
    let temp_dir = TempDir::new().unwrap();

    // Missing description
    let no_desc_dir = temp_dir.path().join("no-description");
    std::fs::create_dir_all(&no_desc_dir).unwrap();
    std::fs::write(
        no_desc_dir.join("SKILL.md"),
        r#"---
name: no-description
---
# Skill without description
"#,
    )
    .unwrap();

    // Missing name
    let no_name_dir = temp_dir.path().join("no-name");
    std::fs::create_dir_all(&no_name_dir).unwrap();
    std::fs::write(
        no_name_dir.join("SKILL.md"),
        r#"---
description: A skill without name
---
# Skill without name
"#,
    )
    .unwrap();

    // Valid skill for comparison
    create_complete_skill(
        temp_dir.path(),
        "complete-skill",
        TestSkillConfig::default(),
    );

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    let count = registry.load_all().await.unwrap();

    // Only valid skill should be loaded
    assert_eq!(count, 1);
    assert!(registry.exists("complete-skill").await);
    assert!(!registry.exists("no-description").await);
    assert!(!registry.exists("no-name").await);
}

// ============================================================================
// TER-004: Empty Skills Directory
// ============================================================================

#[tokio::test]
async fn test_ter_004_empty_skills_directory() {
    let temp_dir = TempDir::new().unwrap();
    // Don't create any skills

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    let count = registry.load_all().await.unwrap();

    assert_eq!(count, 0);

    let summaries = registry.get_summaries().await;
    assert!(summaries.is_empty());

    // Phase 1 injection should still work with empty list
    let phase1_prompt = SkillInjector::phase1_injection(&summaries);
    assert!(
        phase1_prompt.contains("Available Skills")
            || phase1_prompt.is_empty()
            || phase1_prompt.contains("No skills")
    );
}

#[tokio::test]
async fn test_ter_004_nonexistent_skills_directory() {
    // Use a path that doesn't exist
    let registry = SkillRegistry::new(std::path::PathBuf::from("/nonexistent/path/to/skills"));
    let count = registry.load_all().await.unwrap();

    assert_eq!(count, 0);

    let summaries = registry.get_summaries().await;
    assert!(summaries.is_empty());
}

// ============================================================================
// TER-006: Skill Detection - No Results
// ============================================================================

#[tokio::test]
async fn test_ter_006_no_skill_detected() {
    // Various responses without skill tags
    let responses = vec![
        "I will complete this task without using any special skills.",
        "Let me analyze the data directly.",
        "Here's my response with <other_tag>not a skill</other_tag>.",
        "Using tools: Bash, Read, Write",
        "",
        "   ",
    ];

    for response in responses {
        assert!(
            !SkillDetector::has_skill_request(response),
            "Should not detect skill in: {}",
            response
        );
        let detected = SkillDetector::detect(response);
        assert!(
            detected.is_empty(),
            "Should not detect any skills in: {}",
            response
        );
    }

    // Malformed skill tags should not be detected
    let malformed = vec![
        "<use_skill>",                    // No closing tag
        "</use_skill>",                   // No opening tag
        "<use_skill></use_skill>",        // Empty skill name
        "<use_skill>   </use_skill>",     // Whitespace only
        "<use_skil>wrong-tag</use_skil>", // Typo in tag
    ];

    for response in malformed {
        let detected = SkillDetector::detect(response);
        assert!(
            detected.is_empty(),
            "Should not detect skill in malformed: {}",
            response
        );
    }
}

// ============================================================================
// Additional E2E: Skill Priority and Conflict Resolution
// ============================================================================

#[tokio::test]
async fn test_e2e_skill_priority_resolution() {
    let temp_dir = TempDir::new().unwrap();

    // Create skills with priorities
    let high_priority_dir = temp_dir.path().join("high-priority");
    std::fs::create_dir_all(&high_priority_dir).unwrap();
    std::fs::write(
        high_priority_dir.join("SKILL.md"),
        r#"---
name: high-priority
description: High priority skill
metadata:
  priority: "10"
---
# High Priority Skill
"#,
    )
    .unwrap();

    let low_priority_dir = temp_dir.path().join("low-priority");
    std::fs::create_dir_all(&low_priority_dir).unwrap();
    std::fs::write(
        low_priority_dir.join("SKILL.md"),
        r#"---
name: low-priority
description: Low priority skill
metadata:
  priority: "5"
---
# Low Priority Skill
"#,
    )
    .unwrap();

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    registry.load_all().await.unwrap();

    let high = registry.get("high-priority").await.unwrap();
    let low = registry.get("low-priority").await.unwrap();

    // Verify priorities are parsed using get_priority() method
    let high_priority = high.metadata.get_priority().unwrap_or(0);
    let low_priority = low.metadata.get_priority().unwrap_or(0);
    assert!(high_priority > low_priority);
}

// ============================================================================
// Additional E2E: Script Allowlist with Patterns
// ============================================================================

#[tokio::test]
async fn test_e2e_script_allowlist_patterns() {
    let temp_dir = TempDir::new().unwrap();

    let skill_dir = temp_dir.path().join("pattern-skill");
    std::fs::create_dir_all(&skill_dir).unwrap();

    std::fs::write(
        skill_dir.join("SKILL.md"),
        r#"---
name: pattern-skill
description: Skill with script patterns
metadata:
  allowed-scripts: "*.sh, process-*.py, build.js"
---
# Pattern Skill
"#,
    )
    .unwrap();

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    registry.load_all().await.unwrap();

    let skill = registry.get("pattern-skill").await.unwrap();

    // Test pattern matching
    assert!(skill.metadata.is_script_allowed("test.sh"));
    assert!(skill.metadata.is_script_allowed("helper.sh"));
    assert!(skill.metadata.is_script_allowed("process-data.py"));
    assert!(skill.metadata.is_script_allowed("process-image.py"));
    assert!(skill.metadata.is_script_allowed("build.js"));

    // These should not match
    assert!(!skill.metadata.is_script_allowed("random.py"));
    assert!(!skill.metadata.is_script_allowed("deploy.js"));
    assert!(!skill.metadata.is_script_allowed("test.rb"));
}

// ============================================================================
// Additional E2E: Reload All Skills
// ============================================================================

#[tokio::test]
async fn test_e2e_reload_all_skills() {
    let temp_dir = TempDir::new().unwrap();

    // Create initial skills
    create_complete_skill(
        temp_dir.path(),
        "skill-1",
        TestSkillConfig {
            description: "First skill".to_string(),
            ..Default::default()
        },
    );

    create_complete_skill(
        temp_dir.path(),
        "skill-2",
        TestSkillConfig {
            description: "Second skill".to_string(),
            ..Default::default()
        },
    );

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    registry.load_all().await.unwrap();

    assert_eq!(registry.get_summaries().await.len(), 2);

    // Add a new skill on disk
    create_complete_skill(
        temp_dir.path(),
        "skill-3",
        TestSkillConfig {
            description: "Third skill added later".to_string(),
            ..Default::default()
        },
    );

    // Reload all skills
    let count = registry.reload_all().await.unwrap();
    assert_eq!(count, 3);

    // Verify all three skills are now available
    let summaries = registry.get_summaries().await;
    assert_eq!(summaries.len(), 3);
    assert!(summaries.iter().any(|s| s.name == "skill-3"));
}

// ============================================================================
// E2E Test: Resource Loading (scripts, references, assets)
// ============================================================================

#[tokio::test]
async fn test_e2e_resource_loading() {
    use super::SkillLoader;

    let temp_dir = TempDir::new().unwrap();

    create_complete_skill(
        temp_dir.path(),
        "full-skill",
        TestSkillConfig {
            description: "A skill with all resources".to_string(),
            content: "# Full Skill".to_string(),
            with_scripts: true,
            with_references: true,
            with_assets: true,
            ..Default::default()
        },
    );

    let skill_dir = temp_dir.path().join("full-skill");

    // Test reference loading
    let references = SkillLoader::load_references(&skill_dir).await;
    assert_eq!(references.len(), 1);
    assert!(references[0].contains("API Documentation"));

    // Test script listing
    let scripts = SkillLoader::list_scripts(&skill_dir).await;
    assert_eq!(scripts.len(), 1);
    assert_eq!(scripts[0].name, "helper.sh");

    // Test asset loading
    let asset = SkillLoader::load_asset(&skill_dir, "template.md").await;
    assert!(asset.is_some());
    let content = String::from_utf8(asset.unwrap()).unwrap();
    assert!(content.contains("Template"));

    // Test has_resources
    assert!(SkillLoader::has_resources(&skill_dir));

    // Test skill without resources
    create_complete_skill(temp_dir.path(), "minimal-skill", TestSkillConfig::default());
    let minimal_dir = temp_dir.path().join("minimal-skill");
    assert!(!SkillLoader::has_resources(&minimal_dir));
}

// ============================================================================
// TE2E-001: Complete Workflow from Planning to Phase 2 Injection
// ============================================================================

#[tokio::test]
async fn test_te2e_001_complete_workflow_planning_to_phase2() {
    use crate::chat::{
        plan::build_context_for_react,
        planner::{SubTask, SubTaskStatus, ToolDescription},
    };

    let temp_dir = TempDir::new().unwrap();

    // Create test skills
    create_complete_skill(
        temp_dir.path(),
        "weather-query",
        TestSkillConfig {
            description: "Query weather information from various sources".to_string(),
            allowed_tools: Some("WebFetch WebSearch".to_string()),
            content: "# Weather Query Skill\n\nUse WebFetch to get weather data.".to_string(),
            ..Default::default()
        },
    );

    create_complete_skill(
        temp_dir.path(),
        "code-review",
        TestSkillConfig {
            description: "Review code changes and provide feedback".to_string(),
            allowed_tools: Some("Read Grep".to_string()),
            content: "# Code Review Skill\n\nAnalyze code quality.".to_string(),
            ..Default::default()
        },
    );

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    registry.load_all().await.unwrap();
    let summaries = registry.get_summaries().await;

    // Create a subtask
    let subtask = SubTask {
        id: 1,
        description: "Query the current weather in Tokyo".to_string(),
        dependencies: vec![],
        required_tools: vec!["WebFetch".to_string()],
        recommended_skill: Some("weather-query".to_string()),
        status: SubTaskStatus::Pending,
        result: None,
    };

    let tools = vec![
        ToolDescription {
            name: "WebFetch".to_string(),
            description: "Fetch content from URLs".to_string(),
        },
        ToolDescription {
            name: "WebSearch".to_string(),
            description: "Search the web".to_string(),
        },
    ];

    // ========== Phase 1: Planning with Skills Summaries ==========
    let phase1_messages = build_context_for_react(
        &subtask,
        &[],              // no previous results
        &tools,           // available tools
        Some(&summaries), // skill summaries for Phase 1
        &[],              // no active skills yet
        0,
    )
    .await;

    assert!(!phase1_messages.is_empty());

    let phase1_content = phase1_messages
        .iter()
        .find_map(|m| {
            if let endpoints::chat::ChatCompletionRequestMessage::System(sys) = m {
                Some(sys.content().to_string())
            } else {
                None
            }
        })
        .expect("Should have a system message");

    // Verify Phase 1 includes skills summaries
    assert!(
        phase1_content.contains("Available Skills"),
        "Phase 1 should contain Available Skills section"
    );
    assert!(
        phase1_content.contains("weather-query"),
        "Phase 1 should list weather-query skill"
    );
    assert!(
        phase1_content.contains("code-review"),
        "Phase 1 should list code-review skill"
    );
    assert!(
        phase1_content.contains("<use_skill>"),
        "Phase 1 should contain skill activation instructions"
    );

    // ========== Simulate LLM selecting a skill ==========
    let llm_response = "I'll use the weather skill for this. <use_skill>weather-query</use_skill>";
    let detected = SkillDetector::detect(llm_response);
    assert_eq!(detected.len(), 1);
    assert_eq!(detected[0], "weather-query");

    // Load the detected skill
    let active_skill = registry.get(&detected[0]).await.unwrap();

    // ========== Phase 2: Execution with Full Skill Content ==========
    let phase2_messages = build_context_for_react(
        &subtask,
        &[],
        &tools,
        None,                    // no summaries in Phase 2
        &[active_skill.clone()], // active skill with full content
        0,
    )
    .await;

    assert!(!phase2_messages.is_empty());

    let phase2_content = phase2_messages
        .iter()
        .find_map(|m| {
            if let endpoints::chat::ChatCompletionRequestMessage::System(sys) = m {
                Some(sys.content().to_string())
            } else {
                None
            }
        })
        .expect("Should have a system message");

    // Verify Phase 2 includes full skill content (not just summary)
    assert!(
        phase2_content.contains("Active Skill") || phase2_content.contains("weather-query"),
        "Phase 2 should indicate active skill"
    );
    assert!(
        phase2_content.contains("Weather Query Skill")
            || phase2_content.contains("WebFetch to get weather"),
        "Phase 2 should contain full skill instructions"
    );

    // Verify Phase 2 does NOT contain the skills table (that's for Phase 1 only)
    assert!(
        !phase2_content.contains("| Name | Description |"),
        "Phase 2 should NOT contain skills table"
    );
}

// ============================================================================
// TE2E-003: Skill Activation During Subtask Execution
// ============================================================================

#[tokio::test]
async fn test_te2e_003_skill_activation_during_execution() {
    use crate::chat::{
        plan::build_context_for_react,
        planner::{SubTask, SubTaskStatus, ToolDescription},
    };

    let temp_dir = TempDir::new().unwrap();

    create_complete_skill(
        temp_dir.path(),
        "data-analysis",
        TestSkillConfig {
            description: "Analyze and visualize data".to_string(),
            allowed_tools: Some("Read Write Bash".to_string()),
            content: "# Data Analysis Skill\n\n## Workflow\n1. Load data\n2. Process\n3. Visualize"
                .to_string(),
            ..Default::default()
        },
    );

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    registry.load_all().await.unwrap();

    let subtask = SubTask {
        id: 1,
        description: "Analyze the sales data from Q4".to_string(),
        dependencies: vec![],
        required_tools: vec!["Read".to_string()],
        recommended_skill: Some("data-analysis".to_string()),
        status: SubTaskStatus::InProgress,
        result: None,
    };

    let tools = vec![
        ToolDescription {
            name: "Read".to_string(),
            description: "Read file contents".to_string(),
        },
        ToolDescription {
            name: "Write".to_string(),
            description: "Write file contents".to_string(),
        },
    ];

    // First, verify skill exists and can be loaded
    let skill = registry.get("data-analysis").await;
    assert!(skill.is_some(), "data-analysis skill should exist");

    let active_skill = skill.unwrap();

    // Verify skill content was loaded correctly
    assert_eq!(active_skill.metadata.name, "data-analysis");
    assert!(active_skill.content.contains("Data Analysis Skill"));

    // Build context with active skill
    let messages =
        build_context_for_react(&subtask, &[], &tools, None, &[active_skill.clone()], 0).await;

    let system_content = messages
        .iter()
        .find_map(|m| {
            if let endpoints::chat::ChatCompletionRequestMessage::System(sys) = m {
                Some(sys.content().to_string())
            } else {
                None
            }
        })
        .expect("Should have a system message");

    // Verify skill is activated in context
    assert!(
        system_content.contains("data-analysis") || system_content.contains("Data Analysis"),
        "Context should contain activated skill"
    );
    assert!(
        system_content.contains("Workflow") || system_content.contains("Load data"),
        "Context should contain skill instructions"
    );
}

// ============================================================================
// TE2E-004: Tool Calls Work Correctly After Skill Activation
// ============================================================================

#[tokio::test]
async fn test_te2e_004_tool_calls_after_skill_activation() {
    use crate::chat::{
        plan::build_context_for_react,
        planner::{SubTask, SubTaskStatus, ToolDescription},
    };

    let temp_dir = TempDir::new().unwrap();

    // Create skill with specific tool requirements
    create_complete_skill(
        temp_dir.path(),
        "file-processor",
        TestSkillConfig {
            description: "Process and transform files".to_string(),
            allowed_tools: Some("Read Write Bash".to_string()),
            content: "# File Processor\n\nUse Read to load, Write to save.".to_string(),
            ..Default::default()
        },
    );

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    registry.load_all().await.unwrap();

    let skill = registry.get("file-processor").await.unwrap();

    // Verify allowed tools are correctly parsed
    let allowed = skill.metadata.get_allowed_tools();
    assert!(allowed.contains(&"Read".to_string()));
    assert!(allowed.contains(&"Write".to_string()));
    assert!(allowed.contains(&"Bash".to_string()));

    let subtask = SubTask {
        id: 1,
        description: "Transform the input file".to_string(),
        dependencies: vec![],
        required_tools: vec!["Read".to_string(), "Write".to_string()],
        recommended_skill: Some("file-processor".to_string()),
        status: SubTaskStatus::InProgress,
        result: None,
    };

    // Create full tools list (includes tools not allowed by skill)
    let all_tools = vec![
        ToolDescription {
            name: "Read".to_string(),
            description: "Read file contents".to_string(),
        },
        ToolDescription {
            name: "Write".to_string(),
            description: "Write file contents".to_string(),
        },
        ToolDescription {
            name: "Bash".to_string(),
            description: "Execute bash commands".to_string(),
        },
        ToolDescription {
            name: "WebFetch".to_string(),
            description: "Fetch from URLs".to_string(),
        },
    ];

    // Build context with skill
    let messages =
        build_context_for_react(&subtask, &[], &all_tools, None, &[skill.clone()], 0).await;

    let system_content = messages
        .iter()
        .find_map(|m| {
            if let endpoints::chat::ChatCompletionRequestMessage::System(sys) = m {
                Some(sys.content().to_string())
            } else {
                None
            }
        })
        .expect("Should have a system message");

    // Verify tools are available in the context
    assert!(
        system_content.contains("Read") || system_content.contains("read"),
        "Context should mention Read tool"
    );
    assert!(
        system_content.contains("Write") || system_content.contains("write"),
        "Context should mention Write tool"
    );

    // Verify skill instructions are present
    assert!(
        system_content.contains("File Processor") || system_content.contains("file-processor"),
        "Context should contain skill"
    );
}

// ============================================================================
// TMS-001: Dependent Subtasks Using Different Skills
// ============================================================================

#[tokio::test]
async fn test_tms_001_dependent_subtasks_different_skills() {
    use crate::chat::{
        plan::build_context_for_react,
        planner::{SubTask, SubTaskStatus, ToolDescription},
    };

    let temp_dir = TempDir::new().unwrap();

    // Create two different skills for two subtasks
    create_complete_skill(
        temp_dir.path(),
        "data-fetcher",
        TestSkillConfig {
            description: "Fetch data from external sources".to_string(),
            allowed_tools: Some("WebFetch".to_string()),
            content: "# Data Fetcher\n\nFetch external data using WebFetch.".to_string(),
            ..Default::default()
        },
    );

    create_complete_skill(
        temp_dir.path(),
        "report-generator",
        TestSkillConfig {
            description: "Generate reports from data".to_string(),
            allowed_tools: Some("Write".to_string()),
            content: "# Report Generator\n\nGenerate formatted reports.".to_string(),
            ..Default::default()
        },
    );

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    registry.load_all().await.unwrap();

    let tools = vec![
        ToolDescription {
            name: "WebFetch".to_string(),
            description: "Fetch from URLs".to_string(),
        },
        ToolDescription {
            name: "Write".to_string(),
            description: "Write files".to_string(),
        },
    ];

    // ========== Subtask 1: Fetch data (uses data-fetcher skill) ==========
    let subtask1 = SubTask {
        id: 1,
        description: "Fetch sales data from API".to_string(),
        dependencies: vec![],
        required_tools: vec!["WebFetch".to_string()],
        recommended_skill: Some("data-fetcher".to_string()),
        status: SubTaskStatus::InProgress,
        result: None,
    };

    let skill1 = registry.get("data-fetcher").await.unwrap();

    let messages1 =
        build_context_for_react(&subtask1, &[], &tools, None, &[skill1.clone()], 0).await;

    let content1 = messages1
        .iter()
        .find_map(|m| {
            if let endpoints::chat::ChatCompletionRequestMessage::System(sys) = m {
                Some(sys.content().to_string())
            } else {
                None
            }
        })
        .expect("Should have system message");

    assert!(
        content1.contains("Data Fetcher") || content1.contains("data-fetcher"),
        "Subtask 1 should use data-fetcher skill"
    );

    // ========== Subtask 2: Generate report (depends on subtask 1, uses different skill) ==========
    let subtask2 = SubTask {
        id: 2,
        description: "Generate quarterly report from fetched data".to_string(),
        dependencies: vec![1], // Depends on subtask 1
        required_tools: vec!["Write".to_string()],
        recommended_skill: Some("report-generator".to_string()),
        status: SubTaskStatus::Pending,
        result: None,
    };

    let skill2 = registry.get("report-generator").await.unwrap();

    // Pass previous results from subtask 1
    let previous_results = vec![(1, "Sales data: Q4 revenue $1.2M".to_string())];

    let messages2 = build_context_for_react(
        &subtask2,
        &previous_results,
        &tools,
        None,
        &[skill2.clone()],
        0,
    )
    .await;

    let content2 = messages2
        .iter()
        .find_map(|m| {
            if let endpoints::chat::ChatCompletionRequestMessage::System(sys) = m {
                Some(sys.content().to_string())
            } else {
                None
            }
        })
        .expect("Should have system message");

    // Verify subtask 2 uses different skill
    assert!(
        content2.contains("Report Generator") || content2.contains("report-generator"),
        "Subtask 2 should use report-generator skill"
    );
    assert!(
        !content2.contains("Data Fetcher"),
        "Subtask 2 should NOT contain data-fetcher skill content"
    );
}

// ============================================================================
// TMS-002: Independent Subtasks Can Use Different Skills in Parallel
// ============================================================================

#[tokio::test]
async fn test_tms_002_parallel_subtasks_different_skills() {
    use crate::chat::{
        plan::build_context_for_react,
        planner::{SubTask, SubTaskStatus, ToolDescription},
    };

    let temp_dir = TempDir::new().unwrap();

    // Create multiple skills for parallel tasks
    create_complete_skill(
        temp_dir.path(),
        "weather-skill",
        TestSkillConfig {
            description: "Get weather information".to_string(),
            content: "# Weather Skill\n\nGet current weather.".to_string(),
            ..Default::default()
        },
    );

    create_complete_skill(
        temp_dir.path(),
        "news-skill",
        TestSkillConfig {
            description: "Get news headlines".to_string(),
            content: "# News Skill\n\nGet latest news.".to_string(),
            ..Default::default()
        },
    );

    create_complete_skill(
        temp_dir.path(),
        "stock-skill",
        TestSkillConfig {
            description: "Get stock prices".to_string(),
            content: "# Stock Skill\n\nGet market data.".to_string(),
            ..Default::default()
        },
    );

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    registry.load_all().await.unwrap();

    let tools = vec![ToolDescription {
        name: "WebFetch".to_string(),
        description: "Fetch from URLs".to_string(),
    }];

    // Create 3 independent subtasks (no dependencies)
    let subtasks = vec![
        SubTask {
            id: 1,
            description: "Get Tokyo weather".to_string(),
            dependencies: vec![],
            required_tools: vec!["WebFetch".to_string()],
            recommended_skill: Some("weather-skill".to_string()),
            status: SubTaskStatus::Pending,
            result: None,
        },
        SubTask {
            id: 2,
            description: "Get tech news".to_string(),
            dependencies: vec![],
            required_tools: vec!["WebFetch".to_string()],
            recommended_skill: Some("news-skill".to_string()),
            status: SubTaskStatus::Pending,
            result: None,
        },
        SubTask {
            id: 3,
            description: "Get AAPL stock price".to_string(),
            dependencies: vec![],
            required_tools: vec!["WebFetch".to_string()],
            recommended_skill: Some("stock-skill".to_string()),
            status: SubTaskStatus::Pending,
            result: None,
        },
    ];

    // Verify all subtasks are independent (can run in parallel)
    for subtask in &subtasks {
        assert!(
            subtask.dependencies.is_empty(),
            "All subtasks should be independent"
        );
    }

    // Each subtask can load and use its own skill independently
    for subtask in &subtasks {
        let skill_name = subtask.recommended_skill.as_ref().unwrap();
        let skill = registry.get(skill_name).await;
        assert!(
            skill.is_some(),
            "Each subtask's recommended skill should be loadable"
        );

        let messages =
            build_context_for_react(subtask, &[], &tools, None, &[skill.unwrap()], 0).await;

        assert!(
            !messages.is_empty(),
            "Each subtask should generate valid context"
        );
    }
}

// ============================================================================
// TMS-003: Previous Subtask Skill Results Passed to Subsequent Tasks
// ============================================================================

#[tokio::test]
async fn test_tms_003_skill_results_passed_to_next_subtask() {
    use crate::chat::{
        plan::build_context_for_react,
        planner::{SubTask, SubTaskStatus, ToolDescription},
    };

    let temp_dir = TempDir::new().unwrap();

    create_complete_skill(
        temp_dir.path(),
        "translator",
        TestSkillConfig {
            description: "Translate text between languages".to_string(),
            content: "# Translator Skill\n\nTranslate text accurately.".to_string(),
            ..Default::default()
        },
    );

    create_complete_skill(
        temp_dir.path(),
        "summarizer",
        TestSkillConfig {
            description: "Summarize long texts".to_string(),
            content: "# Summarizer Skill\n\nCreate concise summaries.".to_string(),
            ..Default::default()
        },
    );

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    registry.load_all().await.unwrap();

    let tools = vec![ToolDescription {
        name: "Read".to_string(),
        description: "Read files".to_string(),
    }];

    // Subtask 1 completes with a result
    let subtask1_result = "Translated content: This is the Japanese document translated to English. \
        It discusses the quarterly financial results showing 15% revenue growth.";

    // Subtask 2 depends on subtask 1 and needs its result
    let subtask2 = SubTask {
        id: 2,
        description: "Summarize the translated document".to_string(),
        dependencies: vec![1],
        required_tools: vec!["Read".to_string()],
        recommended_skill: Some("summarizer".to_string()),
        status: SubTaskStatus::Pending,
        result: None,
    };

    let skill2 = registry.get("summarizer").await.unwrap();

    // Pass the result from subtask 1
    let previous_results = vec![(1, subtask1_result.to_string())];

    let messages =
        build_context_for_react(&subtask2, &previous_results, &tools, None, &[skill2], 0).await;

    // Check if previous results are included in the context
    let has_previous_result = messages.iter().any(|m| match m {
        endpoints::chat::ChatCompletionRequestMessage::System(sys) => {
            sys.content()
                .to_string()
                .contains("quarterly financial results")
                || sys.content().to_string().contains("15% revenue growth")
        }
        endpoints::chat::ChatCompletionRequestMessage::User(usr) => match usr.content() {
            endpoints::chat::ChatCompletionUserMessageContent::Text(text) => {
                text.contains("quarterly financial results") || text.contains("15% revenue growth")
            }
            endpoints::chat::ChatCompletionUserMessageContent::Parts(parts) => {
                parts.iter().any(|p| {
                    if let endpoints::chat::ContentPart::Text(t) = p {
                        t.text().contains("quarterly financial results")
                            || t.text().contains("15% revenue growth")
                    } else {
                        false
                    }
                })
            }
        },
        _ => false,
    });

    assert!(
        has_previous_result,
        "Subtask 2 context should contain results from subtask 1"
    );
}

// ============================================================================
// TER-003: Recommended Skill Not Loaded (Graceful Degradation)
// ============================================================================

#[tokio::test]
async fn test_ter_003_recommended_skill_not_found() {
    use crate::chat::{
        plan::build_context_for_react,
        planner::{SubTask, SubTaskStatus, ToolDescription},
    };

    let temp_dir = TempDir::new().unwrap();

    // Only create one skill
    create_complete_skill(
        temp_dir.path(),
        "existing-skill",
        TestSkillConfig {
            description: "An existing skill".to_string(),
            content: "# Existing Skill\n\nThis skill exists.".to_string(),
            ..Default::default()
        },
    );

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    registry.load_all().await.unwrap();

    let summaries = registry.get_summaries().await;

    // Create subtask that recommends a non-existent skill
    let subtask = SubTask {
        id: 1,
        description: "Perform a task".to_string(),
        dependencies: vec![],
        required_tools: vec!["WebFetch".to_string()],
        recommended_skill: Some("nonexistent-skill".to_string()), // Does not exist!
        status: SubTaskStatus::Pending,
        result: None,
    };

    let tools = vec![ToolDescription {
        name: "WebFetch".to_string(),
        description: "Fetch from URLs".to_string(),
    }];

    // Attempt to get the recommended skill
    let skill = registry.get("nonexistent-skill").await;
    assert!(
        skill.is_none(),
        "nonexistent-skill should not be found in registry"
    );

    // Graceful degradation: use Phase 1 context with summaries instead
    let messages = build_context_for_react(
        &subtask,
        &[],
        &tools,
        Some(&summaries), // Fall back to Phase 1 with summaries
        &[],              // No active skills (couldn't load the recommended one)
        0,
    )
    .await;

    assert!(!messages.is_empty(), "Should still generate valid context");

    let system_content = messages
        .iter()
        .find_map(|m| {
            if let endpoints::chat::ChatCompletionRequestMessage::System(sys) = m {
                Some(sys.content().to_string())
            } else {
                None
            }
        })
        .expect("Should have a system message");

    // Verify graceful degradation: summaries are still available
    assert!(
        system_content.contains("Available Skills") || system_content.contains("existing-skill"),
        "Should fall back to showing available skills"
    );

    // Verify the task can still proceed
    assert!(
        system_content.contains("Perform a task"),
        "Subtask description should still be present"
    );
}

#[tokio::test]
async fn test_ter_003_graceful_fallback_to_phase1() {
    use crate::chat::{
        plan::build_context_for_react,
        planner::{SubTask, SubTaskStatus},
    };

    let temp_dir = TempDir::new().unwrap();

    // Create valid skills
    create_complete_skill(
        temp_dir.path(),
        "skill-a",
        TestSkillConfig {
            description: "Skill A for testing".to_string(),
            content: "# Skill A".to_string(),
            ..Default::default()
        },
    );

    create_complete_skill(
        temp_dir.path(),
        "skill-b",
        TestSkillConfig {
            description: "Skill B for testing".to_string(),
            content: "# Skill B".to_string(),
            ..Default::default()
        },
    );

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    registry.load_all().await.unwrap();

    let summaries = registry.get_summaries().await;
    assert_eq!(summaries.len(), 2);

    // Subtask recommends a skill that doesn't exist
    let subtask = SubTask {
        id: 1,
        description: "Execute with missing skill".to_string(),
        dependencies: vec![],
        required_tools: vec![],
        recommended_skill: Some("missing-skill".to_string()),
        status: SubTaskStatus::Pending,
        result: None,
    };

    let tools = vec![];

    // Since skill doesn't exist, fall back to Phase 1
    let messages = build_context_for_react(
        &subtask,
        &[],
        &tools,
        Some(&summaries), // Phase 1 with all summaries
        &[],              // No active skill
        0,
    )
    .await;

    let system_content = messages
        .iter()
        .find_map(|m| {
            if let endpoints::chat::ChatCompletionRequestMessage::System(sys) = m {
                Some(sys.content().to_string())
            } else {
                None
            }
        })
        .expect("Should have system message");

    // LLM can still see available skills and choose one
    assert!(
        system_content.contains("skill-a") || system_content.contains("skill-b"),
        "Available skills should be listed for fallback"
    );
    assert!(
        system_content.contains("<use_skill>"),
        "Should still allow skill selection"
    );
}
