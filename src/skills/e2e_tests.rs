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
