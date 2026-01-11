//! Integration tests for Plan Mode Two-Phase Skill Loading
//!
//! This module implements the test cases defined in docs/skills/skills-test-plan.md
//! under "Plan 模式两阶段加载流程" (Plan Mode Two-Phase Loading Flow).
//!
//! Test cases:
//! - TPL-001: SkillRegistry.load_all()
//! - TPL-002: SkillRegistry.get_summaries()
//! - TPL-003: SkillInjector.phase1_injection()
//! - TPL-004: SkillDetector.detect()
//! - TPL-005: SkillRegistry.get()
//! - TPL-006: SkillInjector.phase2_injection()
//! - TPL-007: Skill priority resolution

use std::path::Path;

use tempfile::TempDir;

use super::{SkillDetector, SkillInjector, SkillLoader, SkillRegistry};
use crate::skills::types::SkillSummary;

// ============================================================================
// Test Fixtures
// ============================================================================

/// Configuration for creating test skills with priority and conflict metadata
struct TestSkillConfig {
    description: String,
    license: Option<String>,
    allowed_tools: Option<String>,
    content: String,
    priority: Option<i32>,
    conflicts: Option<Vec<String>>,
    with_references: bool,
}

impl Default for TestSkillConfig {
    fn default() -> Self {
        Self {
            description: "A test skill".to_string(),
            license: None,
            allowed_tools: None,
            content: "# Test Skill\n\nThis is a test skill.".to_string(),
            priority: None,
            conflicts: None,
            with_references: false,
        }
    }
}

/// Create a test skill directory with SKILL.md and optional resources
fn create_test_skill(dir: &Path, name: &str, config: TestSkillConfig) {
    let skill_dir = dir.join(name);
    std::fs::create_dir_all(&skill_dir).unwrap();

    // Build YAML front matter
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

    if let Some(allowed_tools) = config.allowed_tools {
        front_matter.push_str(&format!("allowed-tools: {}\n", allowed_tools));
    }

    // Add metadata section if priority or conflicts are set
    if config.priority.is_some() || config.conflicts.is_some() {
        front_matter.push_str("metadata:\n");
        if let Some(priority) = config.priority {
            front_matter.push_str(&format!("  priority: \"{}\"\n", priority));
        }
        if let Some(conflicts) = config.conflicts {
            front_matter.push_str(&format!("  conflicts: \"{}\"\n", conflicts.join(", ")));
        }
    }

    front_matter.push_str("---\n\n");
    front_matter.push_str(&config.content);

    std::fs::write(skill_dir.join("SKILL.md"), front_matter).unwrap();

    // Create references directory if needed
    if config.with_references {
        let refs_dir = skill_dir.join("references");
        std::fs::create_dir_all(&refs_dir).unwrap();
        std::fs::write(
            refs_dir.join("api-docs.md"),
            "# API Documentation\n\nReference content here.",
        )
        .unwrap();
    }
}

/// Setup multiple test skills for two-phase loading tests
fn setup_test_skills(dir: &Path) {
    // High priority weather skill
    create_test_skill(
        dir,
        "weather-query",
        TestSkillConfig {
            description: "Query weather information for any location".to_string(),
            allowed_tools: Some("weather forecast".to_string()),
            content: r#"# Weather Query Skill

## Purpose
Query current weather and forecasts for cities worldwide.

## Usage
1. Use the weather tool with city name
2. Parse the JSON response
3. Format for user display

## Example
```
<use_skill>weather-query</use_skill>
```
"#
            .to_string(),
            priority: Some(10),
            ..Default::default()
        },
    );

    // Medium priority code review skill
    create_test_skill(
        dir,
        "code-review",
        TestSkillConfig {
            description: "Review code changes and provide feedback".to_string(),
            allowed_tools: Some("Bash(git:*) Read Write".to_string()),
            content: r#"# Code Review Skill

## Purpose
Analyze code changes and provide constructive feedback.

## Checklist
- [ ] Check for bugs
- [ ] Review code style
- [ ] Suggest improvements

## Output Format
Provide structured review with severity levels.
"#
            .to_string(),
            priority: Some(5),
            ..Default::default()
        },
    );

    // Low priority simple skill
    create_test_skill(
        dir,
        "simple-task",
        TestSkillConfig {
            description: "Handle simple tasks without special tools".to_string(),
            content: "# Simple Task\n\nBasic task handling.".to_string(),
            priority: Some(0),
            ..Default::default()
        },
    );
}

// ============================================================================
// TPL-001: SkillRegistry.load_all()
// Verify all skills from the skills directory are loaded correctly
// ============================================================================

#[tokio::test]
async fn test_tpl_001_skill_registry_load_all() {
    let temp_dir = TempDir::new().unwrap();
    setup_test_skills(temp_dir.path());

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    let count = registry.load_all().await.unwrap();

    // Verify all 3 skills loaded
    assert_eq!(count, 3, "Should load all 3 test skills");

    // Verify each skill exists in registry
    assert!(registry.exists("weather-query").await);
    assert!(registry.exists("code-review").await);
    assert!(registry.exists("simple-task").await);

    // Verify non-existent skill returns false
    assert!(!registry.exists("nonexistent-skill").await);
}

#[tokio::test]
async fn test_tpl_001_load_all_with_invalid_skills() {
    let temp_dir = TempDir::new().unwrap();

    // Create a valid skill
    create_test_skill(temp_dir.path(), "valid-skill", TestSkillConfig::default());

    // Create an invalid skill (missing description)
    let invalid_dir = temp_dir.path().join("invalid-skill");
    std::fs::create_dir_all(&invalid_dir).unwrap();
    std::fs::write(
        invalid_dir.join("SKILL.md"),
        r#"---
name: invalid-skill
---
# No description field
"#,
    )
    .unwrap();

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    let count = registry.load_all().await.unwrap();

    // Only valid skill should be loaded
    assert_eq!(count, 1);
    assert!(registry.exists("valid-skill").await);
    assert!(!registry.exists("invalid-skill").await);
}

#[tokio::test]
async fn test_tpl_001_load_all_empty_directory() {
    let temp_dir = TempDir::new().unwrap();

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    let count = registry.load_all().await.unwrap();

    assert_eq!(count, 0);
    let summaries = registry.get_summaries().await;
    assert!(summaries.is_empty());
}

// ============================================================================
// TPL-002: SkillRegistry.get_summaries()
// Verify summaries contain name and description only
// ============================================================================

#[tokio::test]
async fn test_tpl_002_get_summaries() {
    let temp_dir = TempDir::new().unwrap();
    setup_test_skills(temp_dir.path());

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    registry.load_all().await.unwrap();

    let summaries = registry.get_summaries().await;

    // Verify count
    assert_eq!(summaries.len(), 3);

    // Verify summary fields
    for summary in &summaries {
        // Name should be non-empty
        assert!(!summary.name.is_empty());
        // Description should be non-empty
        assert!(!summary.description.is_empty());

        // Verify specific skill summaries
        match summary.name.as_str() {
            "weather-query" => {
                assert!(summary.description.contains("weather"));
                assert!(summary.allowed_tools.contains(&"weather".to_string()));
            }
            "code-review" => {
                assert!(summary.description.contains("code"));
                assert!(summary.allowed_tools.contains(&"Bash(git:*)".to_string()));
            }
            "simple-task" => {
                assert!(summary.description.contains("simple"));
                assert!(summary.allowed_tools.is_empty());
            }
            _ => panic!("Unexpected skill: {}", summary.name),
        }
    }
}

#[tokio::test]
async fn test_tpl_002_summaries_exclude_disabled() {
    let temp_dir = TempDir::new().unwrap();
    setup_test_skills(temp_dir.path());

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    registry.load_all().await.unwrap();

    // Initially all skills are in summaries
    let summaries = registry.get_summaries().await;
    assert_eq!(summaries.len(), 3);

    // Disable one skill
    registry.set_enabled("weather-query", false).await.unwrap();

    // Verify disabled skill is excluded from summaries
    let summaries = registry.get_summaries().await;
    assert_eq!(summaries.len(), 2);
    assert!(!summaries.iter().any(|s| s.name == "weather-query"));
}

// ============================================================================
// TPL-003: SkillInjector.phase1_injection()
// Verify Phase 1 injection generates correct skill table
// ============================================================================

#[tokio::test]
async fn test_tpl_003_phase1_injection() {
    let temp_dir = TempDir::new().unwrap();
    setup_test_skills(temp_dir.path());

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    registry.load_all().await.unwrap();

    let summaries = registry.get_summaries().await;
    let phase1_prompt = SkillInjector::phase1_injection(&summaries);

    // Verify header section
    assert!(
        phase1_prompt.contains("## Available Skills"),
        "Should contain Available Skills header"
    );

    // Verify table format
    assert!(
        phase1_prompt.contains("| Skill | Description |"),
        "Should contain table header"
    );
    assert!(
        phase1_prompt.contains("|-------|-------------|"),
        "Should contain table separator"
    );

    // Verify skills are in table
    assert!(
        phase1_prompt.contains("| weather-query |"),
        "Should contain weather-query in table"
    );
    assert!(
        phase1_prompt.contains("| code-review |"),
        "Should contain code-review in table"
    );
    assert!(
        phase1_prompt.contains("| simple-task |"),
        "Should contain simple-task in table"
    );

    // Verify usage instructions
    assert!(
        phase1_prompt.contains("<use_skill>skill-name</use_skill>"),
        "Should contain usage instructions"
    );
}

#[tokio::test]
async fn test_tpl_003_phase1_injection_empty() {
    let summaries: Vec<SkillSummary> = vec![];
    let phase1_prompt = SkillInjector::phase1_injection(&summaries);

    // Empty summaries should return empty string
    assert!(phase1_prompt.is_empty());
}

#[tokio::test]
async fn test_tpl_003_phase1_injection_preserves_order() {
    let summaries = vec![
        SkillSummary {
            name: "alpha".to_string(),
            description: "Alpha skill".to_string(),
            allowed_tools: vec![],
        },
        SkillSummary {
            name: "beta".to_string(),
            description: "Beta skill".to_string(),
            allowed_tools: vec![],
        },
        SkillSummary {
            name: "gamma".to_string(),
            description: "Gamma skill".to_string(),
            allowed_tools: vec![],
        },
    ];

    let phase1_prompt = SkillInjector::phase1_injection(&summaries);

    // Verify order is preserved
    let alpha_pos = phase1_prompt.find("alpha").unwrap();
    let beta_pos = phase1_prompt.find("beta").unwrap();
    let gamma_pos = phase1_prompt.find("gamma").unwrap();

    assert!(alpha_pos < beta_pos, "alpha should appear before beta");
    assert!(beta_pos < gamma_pos, "beta should appear before gamma");
}

// ============================================================================
// TPL-004: SkillDetector.detect()
// Verify correct parsing of <use_skill> tags from LLM responses
// ============================================================================

#[tokio::test]
async fn test_tpl_004_detect_single_skill() {
    let response = "I will use <use_skill>weather-query</use_skill> to get the weather.";

    let detected = SkillDetector::detect(response);
    assert_eq!(detected.len(), 1);
    assert_eq!(detected[0], "weather-query");
}

#[tokio::test]
async fn test_tpl_004_detect_multiple_skills() {
    let response = "<use_skill>weather-query</use_skill> and <use_skill>code-review</use_skill>";

    let detected = SkillDetector::detect(response);
    assert_eq!(detected.len(), 2);
    assert_eq!(detected[0], "weather-query");
    assert_eq!(detected[1], "code-review");
}

#[tokio::test]
async fn test_tpl_004_detect_with_whitespace() {
    let response = "<use_skill>  spaced-skill  </use_skill>";

    let detected = SkillDetector::detect(response);
    assert_eq!(detected.len(), 1);
    assert_eq!(detected[0], "spaced-skill");
}

#[tokio::test]
async fn test_tpl_004_detect_no_tags() {
    let response = "Just a regular response without any skill tags.";

    let detected = SkillDetector::detect(response);
    assert!(detected.is_empty());
}

#[tokio::test]
async fn test_tpl_004_detect_nested_text() {
    let response = r#"
Here is my analysis:

<use_skill>code-review</use_skill>

I'll review the code now.
"#;

    let detected = SkillDetector::detect(response);
    assert_eq!(detected.len(), 1);
    assert_eq!(detected[0], "code-review");
}

#[tokio::test]
async fn test_tpl_004_has_skill_request() {
    assert!(SkillDetector::has_skill_request(
        "<use_skill>test</use_skill>"
    ));
    assert!(!SkillDetector::has_skill_request("no skill here"));
    assert!(!SkillDetector::has_skill_request("<use_skill></use_skill>")); // Empty
}

#[tokio::test]
async fn test_tpl_004_detect_first() {
    let response = "<use_skill>first</use_skill> and <use_skill>second</use_skill>";

    let first = SkillDetector::detect_first(response);
    assert_eq!(first, Some("first".to_string()));
}

#[tokio::test]
async fn test_tpl_004_strip_tags() {
    let response = "I need <use_skill>weather-query</use_skill> for this task.";

    let cleaned = SkillDetector::strip_tags(response);

    assert!(!cleaned.contains("<use_skill>"));
    assert!(!cleaned.contains("</use_skill>"));
    assert!(cleaned.contains("I need"));
    assert!(cleaned.contains("for this task."));
}

#[tokio::test]
async fn test_tpl_004_extract_and_clean() {
    let response = "Using <use_skill>weather-query</use_skill> to get weather.";

    let (skills, cleaned) = SkillDetector::extract_and_clean(response);

    assert_eq!(skills, vec!["weather-query"]);
    assert!(!cleaned.contains("<use_skill>"));
    assert!(cleaned.contains("Using"));
    assert!(cleaned.contains("to get weather"));
}

// ============================================================================
// TPL-005: SkillRegistry.get()
// Verify full skill loading by name
// ============================================================================

#[tokio::test]
async fn test_tpl_005_get_existing_skill() {
    let temp_dir = TempDir::new().unwrap();
    setup_test_skills(temp_dir.path());

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    registry.load_all().await.unwrap();

    let skill = registry.get("weather-query").await;
    assert!(skill.is_some());

    let skill = skill.unwrap();
    assert_eq!(skill.metadata.name, "weather-query");
    assert!(skill.metadata.description.contains("weather"));
    assert!(skill.content.contains("# Weather Query Skill"));
    assert!(skill.enabled);
}

#[tokio::test]
async fn test_tpl_005_get_nonexistent_skill() {
    let temp_dir = TempDir::new().unwrap();
    setup_test_skills(temp_dir.path());

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    registry.load_all().await.unwrap();

    let skill = registry.get("nonexistent").await;
    assert!(skill.is_none());
}

#[tokio::test]
async fn test_tpl_005_get_disabled_skill() {
    let temp_dir = TempDir::new().unwrap();
    setup_test_skills(temp_dir.path());

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    registry.load_all().await.unwrap();

    // Disable skill
    registry.set_enabled("weather-query", false).await.unwrap();

    // Should still be able to get disabled skill
    let skill = registry.get("weather-query").await;
    assert!(skill.is_some());
    assert!(!skill.unwrap().enabled);
}

#[tokio::test]
async fn test_tpl_005_get_with_allowed_tools() {
    let temp_dir = TempDir::new().unwrap();
    setup_test_skills(temp_dir.path());

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    registry.load_all().await.unwrap();

    let skill = registry.get("weather-query").await.unwrap();
    let tools = skill.metadata.get_allowed_tools();

    assert_eq!(tools.len(), 2);
    assert!(tools.contains(&"weather".to_string()));
    assert!(tools.contains(&"forecast".to_string()));
}

// ============================================================================
// TPL-006: SkillInjector.phase2_injection()
// Verify Phase 2 injection generates complete skill content
// ============================================================================

#[tokio::test]
async fn test_tpl_006_phase2_injection() {
    let temp_dir = TempDir::new().unwrap();
    setup_test_skills(temp_dir.path());

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    registry.load_all().await.unwrap();

    let skill = registry.get("weather-query").await.unwrap();
    let phase2_prompt = SkillInjector::phase2_injection(&skill);

    // Verify header
    assert!(
        phase2_prompt.contains("## Active Skill: weather-query"),
        "Should contain active skill header"
    );

    // Verify content delimiters
    assert!(
        phase2_prompt.contains("---"),
        "Should contain content delimiters"
    );

    // Verify full content is present
    assert!(
        phase2_prompt.contains("# Weather Query Skill"),
        "Should contain skill content"
    );
    assert!(
        phase2_prompt.contains("## Purpose"),
        "Should contain Purpose section"
    );
    assert!(
        phase2_prompt.contains("## Usage"),
        "Should contain Usage section"
    );
}

#[tokio::test]
async fn test_tpl_006_phase2_injection_with_references() {
    let temp_dir = TempDir::new().unwrap();

    // Create skill with references
    create_test_skill(
        temp_dir.path(),
        "ref-skill",
        TestSkillConfig {
            description: "Skill with references".to_string(),
            content: "# Reference Skill\n\nMain content.".to_string(),
            with_references: true,
            ..Default::default()
        },
    );

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    registry.load_all().await.unwrap();

    let skill = registry.get("ref-skill").await.unwrap();

    // Load references
    let references = SkillLoader::load_references(&skill.skill_dir).await;
    let phase2_prompt = SkillInjector::phase2_injection_with_refs(&skill, &references);

    // Verify references are included
    assert!(phase2_prompt.contains("## Reference Materials"));
    assert!(phase2_prompt.contains("### Reference Document 1"));
    assert!(phase2_prompt.contains("API Documentation"));
}

#[tokio::test]
async fn test_tpl_006_inject_skill_into_prompt() {
    let temp_dir = TempDir::new().unwrap();
    setup_test_skills(temp_dir.path());

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    registry.load_all().await.unwrap();

    let base_prompt = "You are a helpful assistant.";
    let skill = registry.get("weather-query").await.unwrap();

    let full_prompt = SkillInjector::inject_skill(base_prompt, &skill);

    // Verify base prompt is preserved
    assert!(full_prompt.starts_with("You are a helpful assistant."));

    // Verify skill content is appended
    assert!(full_prompt.contains("## Active Skill: weather-query"));
}

// ============================================================================
// TPL-007: Skill Priority Resolution
// Verify skills with higher priority are resolved correctly
// ============================================================================

#[tokio::test]
async fn test_tpl_007_priority_resolution() {
    let temp_dir = TempDir::new().unwrap();
    setup_test_skills(temp_dir.path());

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    registry.load_all().await.unwrap();

    // Get skills and verify priorities
    let weather = registry.get("weather-query").await.unwrap();
    let code = registry.get("code-review").await.unwrap();
    let simple = registry.get("simple-task").await.unwrap();

    assert_eq!(weather.metadata.get_priority(), Some(10));
    assert_eq!(code.metadata.get_priority(), Some(5));
    assert_eq!(simple.metadata.get_priority(), Some(0));
}

#[tokio::test]
async fn test_tpl_007_get_by_priority() {
    let temp_dir = TempDir::new().unwrap();
    setup_test_skills(temp_dir.path());

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    registry.load_all().await.unwrap();

    // Get all skills and sort by priority
    let mut skills = vec![];
    for name in ["weather-query", "code-review", "simple-task"] {
        if let Some(skill) = registry.get(name).await {
            skills.push(skill);
        }
    }

    skills.sort_by(|a, b| {
        let pa = a.metadata.get_priority().unwrap_or(0);
        let pb = b.metadata.get_priority().unwrap_or(0);
        pb.cmp(&pa) // Descending order
    });

    // Verify high priority is first
    assert_eq!(skills[0].metadata.name, "weather-query");
    assert_eq!(skills[1].metadata.name, "code-review");
    assert_eq!(skills[2].metadata.name, "simple-task");
}

#[tokio::test]
async fn test_tpl_007_default_priority() {
    let temp_dir = TempDir::new().unwrap();

    // Create skill without priority
    create_test_skill(temp_dir.path(), "no-priority", TestSkillConfig::default());

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    registry.load_all().await.unwrap();

    let skill = registry.get("no-priority").await.unwrap();
    assert_eq!(skill.metadata.get_priority(), None);
}

// ============================================================================
// Integration: Complete Two-Phase Loading Workflow
// ============================================================================

#[tokio::test]
async fn test_complete_two_phase_workflow() {
    let temp_dir = TempDir::new().unwrap();
    setup_test_skills(temp_dir.path());

    // Step 1: Load all skills (TPL-001)
    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    let count = registry.load_all().await.unwrap();
    assert_eq!(count, 3);

    // Step 2: Get summaries for Phase 1 (TPL-002)
    let summaries = registry.get_summaries().await;
    assert_eq!(summaries.len(), 3);

    // Step 3: Generate Phase 1 prompt (TPL-003)
    let base_prompt = "You are a helpful assistant that can use skills.";
    let phase1_prompt = SkillInjector::inject_summaries(base_prompt, &summaries);
    assert!(phase1_prompt.contains("## Available Skills"));
    assert!(phase1_prompt.contains("weather-query"));

    // Step 4: Simulate LLM response with skill request
    let llm_response = r#"
Based on your request, I need to get weather information.
<use_skill>weather-query</use_skill>
Let me fetch the weather data.
"#;

    // Step 5: Detect skill request (TPL-004)
    let (detected_skills, cleaned_response) = SkillDetector::extract_and_clean(llm_response);
    assert_eq!(detected_skills, vec!["weather-query"]);
    assert!(!cleaned_response.contains("<use_skill>"));

    // Step 6: Load full skill (TPL-005)
    let skill = registry.get(&detected_skills[0]).await.unwrap();
    assert_eq!(skill.metadata.name, "weather-query");

    // Step 7: Generate Phase 2 prompt (TPL-006)
    let phase2_prompt = SkillInjector::inject_skill(base_prompt, &skill);
    assert!(phase2_prompt.contains("## Active Skill: weather-query"));
    assert!(phase2_prompt.contains("# Weather Query Skill"));

    // Verify tools are available
    let tools = skill.metadata.get_allowed_tools();
    assert!(tools.contains(&"weather".to_string()));
}

// ============================================================================
// Multi-Skill Injection Tests
// ============================================================================

#[tokio::test]
async fn test_multi_skill_injection() {
    let temp_dir = TempDir::new().unwrap();
    setup_test_skills(temp_dir.path());

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    registry.load_all().await.unwrap();

    // Load multiple skills
    let weather = registry.get("weather-query").await.unwrap();
    let code = registry.get("code-review").await.unwrap();

    let skills = vec![weather, code];
    let multi_prompt = SkillInjector::multi_skill_injection(&skills);

    // Verify header lists all skills
    assert!(multi_prompt.contains("## Active Skills: weather-query, code-review"));

    // Verify merged permissions
    assert!(multi_prompt.contains("### Merged Permissions"));
    assert!(multi_prompt.contains("**Allowed Tools:**"));

    // Verify individual skill sections
    assert!(multi_prompt.contains("### Skill: weather-query"));
    assert!(multi_prompt.contains("### Skill: code-review"));
}

#[tokio::test]
async fn test_merge_allowed_tools() {
    let temp_dir = TempDir::new().unwrap();
    setup_test_skills(temp_dir.path());

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    registry.load_all().await.unwrap();

    let weather = registry.get("weather-query").await.unwrap();
    let code = registry.get("code-review").await.unwrap();

    let skills = vec![weather, code];
    let merged_tools = SkillInjector::merge_allowed_tools(&skills);

    // Should contain tools from both skills (deduplicated)
    assert!(merged_tools.contains(&"weather".to_string()));
    assert!(merged_tools.contains(&"forecast".to_string()));
    assert!(merged_tools.contains(&"Bash(git:*)".to_string()));
    assert!(merged_tools.contains(&"Read".to_string()));
    assert!(merged_tools.contains(&"Write".to_string()));
}

// ============================================================================
// Performance Tests
// ============================================================================

#[tokio::test]
async fn test_performance_phase1_injection() {
    // Create many skill summaries
    let summaries: Vec<SkillSummary> = (0..50)
        .map(|i| SkillSummary {
            name: format!("skill-{}", i),
            description: format!("Description for skill {}", i),
            allowed_tools: vec![format!("tool-{}", i)],
        })
        .collect();

    let start = std::time::Instant::now();
    for _ in 0..100 {
        let _ = SkillInjector::phase1_injection(&summaries);
    }
    let duration = start.elapsed();

    // 100 iterations with 50 skills should be fast
    assert!(
        duration.as_millis() < 100,
        "Phase 1 injection too slow: {:?}",
        duration
    );
}

#[tokio::test]
async fn test_performance_skill_detection() {
    let response = "<use_skill>skill-1</use_skill>".repeat(10);

    let start = std::time::Instant::now();
    for _ in 0..1000 {
        let _ = SkillDetector::detect(&response);
    }
    let duration = start.elapsed();

    // 1000 detections should be very fast
    assert!(
        duration.as_millis() < 100,
        "Detection too slow: {:?}",
        duration
    );
}

// ============================================================================
// TSE-003: Multi Skill Activation
// Verify multiple skills can be activated simultaneously
// ============================================================================

#[tokio::test]
async fn test_tse_003_multi_skill_activation() {
    let temp_dir = TempDir::new().unwrap();
    setup_test_skills(temp_dir.path());

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    registry.load_all().await.unwrap();

    // Detect multiple Skills from LLM response
    let llm_response =
        "I'll use <use_skill>weather-query</use_skill> and <use_skill>code-review</use_skill>.";
    let detected = SkillDetector::detect(llm_response);

    assert_eq!(detected.len(), 2);
    assert!(detected.contains(&"weather-query".to_string()));
    assert!(detected.contains(&"code-review".to_string()));

    // Load multiple Skills
    let skill1 = registry.get("weather-query").await.unwrap();
    let skill2 = registry.get("code-review").await.unwrap();

    // Verify both Skills are correctly loaded
    assert_eq!(skill1.metadata.name, "weather-query");
    assert_eq!(skill2.metadata.name, "code-review");
}

#[tokio::test]
async fn test_tse_003_multi_skill_comma_separated() {
    let temp_dir = TempDir::new().unwrap();
    setup_test_skills(temp_dir.path());

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    registry.load_all().await.unwrap();

    // Comma-separated skills in single tag
    let llm_response = "Using <use_skill>weather-query, code-review</use_skill> for this task.";
    let detected = SkillDetector::detect(llm_response);

    assert_eq!(detected.len(), 2);
    assert!(detected.contains(&"weather-query".to_string()));
    assert!(detected.contains(&"code-review".to_string()));

    // Verify multi-skill injection works
    let skill1 = registry.get("weather-query").await.unwrap();
    let skill2 = registry.get("code-review").await.unwrap();
    let skills = vec![skill1, skill2];

    let multi_prompt = SkillInjector::multi_skill_injection(&skills);
    assert!(multi_prompt.contains("weather-query"));
    assert!(multi_prompt.contains("code-review"));
}

#[tokio::test]
async fn test_tse_003_multi_skill_content_merge() {
    let temp_dir = TempDir::new().unwrap();
    setup_test_skills(temp_dir.path());

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    registry.load_all().await.unwrap();

    let weather = registry.get("weather-query").await.unwrap();
    let code = registry.get("code-review").await.unwrap();
    let skills = vec![weather, code];

    let multi_prompt = SkillInjector::multi_skill_injection(&skills);

    // Verify header lists all skills
    assert!(multi_prompt.contains("## Active Skills: weather-query, code-review"));

    // Verify merged permissions section exists
    assert!(multi_prompt.contains("### Merged Permissions"));

    // Verify both skill contents are included
    assert!(multi_prompt.contains("### Skill: weather-query"));
    assert!(multi_prompt.contains("### Skill: code-review"));
    assert!(multi_prompt.contains("Weather Query Skill"));
    assert!(multi_prompt.contains("Code Review Skill"));
}

// ============================================================================
// TSE-004: Skill Tag Stripping
// Verify strip_tags() correctly removes skill tags from text
// ============================================================================

#[tokio::test]
async fn test_tse_004_skill_tag_stripping() {
    let llm_response = "I need to <use_skill>weather-query</use_skill> for this task.";

    // Clean tags
    let cleaned = SkillDetector::strip_tags(llm_response);

    assert!(!cleaned.contains("<use_skill>"));
    assert!(!cleaned.contains("</use_skill>"));
    assert!(!cleaned.contains("weather-query")); // skill name is also removed
    assert!(cleaned.contains("I need to"));
    assert!(cleaned.contains("for this task."));
}

#[tokio::test]
async fn test_tse_004_strip_multiple_tags() {
    let llm_response =
        "First <use_skill>skill-a</use_skill> then <use_skill>skill-b</use_skill> done.";
    let cleaned = SkillDetector::strip_tags(llm_response);

    assert!(!cleaned.contains("<use_skill>"));
    assert!(!cleaned.contains("</use_skill>"));
    assert!(cleaned.contains("First"));
    assert!(cleaned.contains("then"));
    assert!(cleaned.contains("done."));
}

#[tokio::test]
async fn test_tse_004_strip_comma_separated_tags() {
    let llm_response = "Using <use_skill>skill-a, skill-b</use_skill> for analysis.";
    let cleaned = SkillDetector::strip_tags(llm_response);

    assert!(!cleaned.contains("<use_skill>"));
    assert!(!cleaned.contains("skill-a"));
    assert!(!cleaned.contains("skill-b"));
    assert!(cleaned.contains("Using"));
    assert!(cleaned.contains("for analysis."));
}

#[tokio::test]
async fn test_tse_004_preserve_other_xml_tags() {
    let llm_response =
        "<thought>thinking</thought> <use_skill>my-skill</use_skill> <action>do</action>";
    let cleaned = SkillDetector::strip_tags(llm_response);

    // Other tags should be preserved
    assert!(cleaned.contains("<thought>thinking</thought>"));
    assert!(cleaned.contains("<action>do</action>"));
    // Skill tag should be removed
    assert!(!cleaned.contains("my-skill"));
}

#[tokio::test]
async fn test_tse_004_extract_and_clean() {
    let llm_response = "Using <use_skill>weather</use_skill> for forecast.";
    let (skills, cleaned) = SkillDetector::extract_and_clean(llm_response);

    assert_eq!(skills, vec!["weather"]);
    assert!(!cleaned.contains("<use_skill>"));
    assert!(cleaned.contains("Using"));
    assert!(cleaned.contains("for forecast."));
}

// ============================================================================
// TSE-006: Iteration Continue Mechanism
// Verify skill detection triggers next iteration in Plan mode
// ============================================================================

#[tokio::test]
async fn test_tse_006_skill_detection_triggers_iteration() {
    let llm_response = "I need to <use_skill>weather-query</use_skill> for this task.";

    // Detect Skill request
    assert!(SkillDetector::has_skill_request(llm_response));
    let detected = SkillDetector::detect(llm_response);
    assert_eq!(detected, vec!["weather-query"]);

    // In Plan mode logic: detecting Skill should trigger Phase 2
    let should_continue = !detected.is_empty();
    assert!(should_continue, "Detecting Skill should continue iteration");
}

#[tokio::test]
async fn test_tse_006_no_skill_stops_iteration() {
    let no_skill_response = "I will complete this task without any skill.";

    // No Skill request
    assert!(!SkillDetector::has_skill_request(no_skill_response));
    let detected = SkillDetector::detect(no_skill_response);
    assert!(detected.is_empty());

    // Should not trigger next iteration
    let should_continue = !detected.is_empty();
    assert!(
        !should_continue,
        "No Skill detected should not continue iteration"
    );
}

#[tokio::test]
async fn test_tse_006_phase_transition() {
    let temp_dir = TempDir::new().unwrap();
    setup_test_skills(temp_dir.path());

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    registry.load_all().await.unwrap();

    // Simulate Phase 1: LLM response with skill request
    let phase1_response = r#"
    Based on the user request, I need weather information.
    <use_skill>weather-query</use_skill>
    Let me proceed to get the data.
    "#;

    // Detect and verify Phase 2 should be triggered
    let detected = SkillDetector::detect(phase1_response);
    assert!(!detected.is_empty());
    assert_eq!(detected[0], "weather-query");

    // Load skill for Phase 2
    let skill = registry.get(&detected[0]).await.unwrap();
    assert!(skill.enabled);

    // Generate Phase 2 prompt
    let phase2_prompt = SkillInjector::phase2_injection(&skill);
    assert!(phase2_prompt.contains("## Active Skill: weather-query"));

    // Simulate Phase 2: LLM response without skill request (final answer)
    let phase2_response = r#"
    The weather in Tokyo is sunny with 22°C temperature.
    "#;

    // No more skill requests - iteration should stop
    assert!(!SkillDetector::has_skill_request(phase2_response));
    let final_detected = SkillDetector::detect(phase2_response);
    assert!(final_detected.is_empty());
}

#[tokio::test]
async fn test_tse_006_multiple_iterations() {
    let temp_dir = TempDir::new().unwrap();
    setup_test_skills(temp_dir.path());

    let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
    registry.load_all().await.unwrap();

    // Iteration 1: First skill request
    let response1 = "First I need <use_skill>weather-query</use_skill> for weather.";
    let skills1 = SkillDetector::detect(response1);
    assert_eq!(skills1, vec!["weather-query"]);
    let should_continue1 = !skills1.is_empty();
    assert!(should_continue1);

    // Iteration 2: Second skill request (after processing first)
    let response2 = "Now I need <use_skill>code-review</use_skill> for code analysis.";
    let skills2 = SkillDetector::detect(response2);
    assert_eq!(skills2, vec!["code-review"]);
    let should_continue2 = !skills2.is_empty();
    assert!(should_continue2);

    // Iteration 3: No more skill requests
    let response3 = "All tasks completed. Here is the final result.";
    let skills3 = SkillDetector::detect(response3);
    assert!(skills3.is_empty());
    let should_continue3 = !skills3.is_empty();
    assert!(!should_continue3);
}
