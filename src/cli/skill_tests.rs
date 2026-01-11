//! Integration tests for Skill CLI commands
//!
//! This module implements the test cases defined in docs/skills/skills-test-plan.md
//! under "CLI 命令" (CLI Commands).
//!
//! Test cases:
//! - TCL-001: skill list - List local skills
//! - TCL-002: skill info {name} - Show skill details
//! - TCL-003: skill install (local ZIP) - Install from local ZIP
//! - TCL-004: skill uninstall {name} - Uninstall a skill
//!
//! Note: These tests focus on CLI argument parsing and command structure.
//! Full integration tests with actual file system operations are performed
//! in the skill module's own tests.

use std::path::PathBuf;

use clap::Parser;

use super::{Cli, Command, skill::SkillCommand};

// ============================================================================
// TCL-001: skill list - List local skills
// ============================================================================

#[test]
fn test_tcl_001_skill_list_command_parsing() {
    // Test basic list command
    let cli = Cli::parse_from(["llama-nexus", "skill", "list"]);
    match cli.command {
        Some(Command::Skill(SkillCommand::List {
            remote,
            category,
            limit,
        })) => {
            assert!(!remote, "Default should be local listing");
            assert!(category.is_none(), "No category filter by default");
            assert_eq!(limit, 10, "Default limit should be 10");
        }
        _ => panic!("Expected Skill List command"),
    }
}

#[test]
fn test_tcl_001_skill_list_remote() {
    // Test list with --remote flag
    let cli = Cli::parse_from(["llama-nexus", "skill", "list", "--remote"]);
    match cli.command {
        Some(Command::Skill(SkillCommand::List { remote, .. })) => {
            assert!(remote, "Should be remote listing");
        }
        _ => panic!("Expected Skill List command"),
    }
}

#[test]
fn test_tcl_001_skill_list_with_category() {
    // Test list with category filter
    let cli = Cli::parse_from([
        "llama-nexus",
        "skill",
        "list",
        "--remote",
        "--category",
        "development",
    ]);
    match cli.command {
        Some(Command::Skill(SkillCommand::List { category, .. })) => {
            assert_eq!(category, Some("development".to_string()));
        }
        _ => panic!("Expected Skill List command"),
    }
}

#[test]
fn test_tcl_001_skill_list_with_limit() {
    // Test list with custom limit
    let cli = Cli::parse_from(["llama-nexus", "skill", "list", "-n", "20"]);
    match cli.command {
        Some(Command::Skill(SkillCommand::List { limit, .. })) => {
            assert_eq!(limit, 20);
        }
        _ => panic!("Expected Skill List command"),
    }
}

// ============================================================================
// TCL-002: skill info {name} - Show skill details
// ============================================================================

#[test]
fn test_tcl_002_skill_info_command_parsing() {
    // Test basic info command
    let cli = Cli::parse_from(["llama-nexus", "skill", "info", "code-review"]);
    match cli.command {
        Some(Command::Skill(SkillCommand::Info { name })) => {
            assert_eq!(name, "code-review");
        }
        _ => panic!("Expected Skill Info command"),
    }
}

#[test]
fn test_tcl_002_skill_info_with_remote_prefix() {
    // Test info with skillsmp: prefix
    let cli = Cli::parse_from(["llama-nexus", "skill", "info", "skillsmp:weather-query"]);
    match cli.command {
        Some(Command::Skill(SkillCommand::Info { name })) => {
            assert_eq!(name, "skillsmp:weather-query");
            assert!(name.starts_with("skillsmp:"));
        }
        _ => panic!("Expected Skill Info command"),
    }
}

#[test]
fn test_tcl_002_skill_info_requires_name() {
    // Test that info command requires a name argument
    let result = Cli::try_parse_from(["llama-nexus", "skill", "info"]);
    assert!(result.is_err(), "Should fail without skill name");
}

// ============================================================================
// TCL-003: skill install (local ZIP) - Install from local ZIP
// ============================================================================

#[test]
fn test_tcl_003_skill_install_from_skillsmp() {
    // Test install from skillsmp.com
    let cli = Cli::parse_from(["llama-nexus", "skill", "install", "skillsmp:code-review"]);
    match cli.command {
        Some(Command::Skill(SkillCommand::Install {
            source,
            dir,
            enable,
        })) => {
            assert_eq!(source, "skillsmp:code-review");
            assert!(dir.is_none(), "No custom directory by default");
            assert!(!enable, "Not enabled by default");
        }
        _ => panic!("Expected Skill Install command"),
    }
}

#[test]
fn test_tcl_003_skill_install_with_version() {
    // Test install with version specifier
    let cli = Cli::parse_from([
        "llama-nexus",
        "skill",
        "install",
        "skillsmp:code-review@2.0.0",
    ]);
    match cli.command {
        Some(Command::Skill(SkillCommand::Install { source, .. })) => {
            assert_eq!(source, "skillsmp:code-review@2.0.0");
            assert!(source.contains("@"));
        }
        _ => panic!("Expected Skill Install command"),
    }
}

#[test]
fn test_tcl_003_skill_install_with_custom_dir() {
    // Test install with custom directory
    let cli = Cli::parse_from([
        "llama-nexus",
        "skill",
        "install",
        "skillsmp:test",
        "--dir",
        "/custom/path",
    ]);
    match cli.command {
        Some(Command::Skill(SkillCommand::Install { dir, .. })) => {
            assert_eq!(dir, Some(PathBuf::from("/custom/path")));
        }
        _ => panic!("Expected Skill Install command"),
    }
}

#[test]
fn test_tcl_003_skill_install_with_enable() {
    // Test install with --enable flag
    let cli = Cli::parse_from([
        "llama-nexus",
        "skill",
        "install",
        "skillsmp:test",
        "--enable",
    ]);
    match cli.command {
        Some(Command::Skill(SkillCommand::Install { enable, .. })) => {
            assert!(enable, "Should be enabled");
        }
        _ => panic!("Expected Skill Install command"),
    }
}

#[test]
fn test_tcl_003_skill_install_from_github() {
    // Test install from GitHub
    let cli = Cli::parse_from(["llama-nexus", "skill", "install", "github:user/repo"]);
    match cli.command {
        Some(Command::Skill(SkillCommand::Install { source, .. })) => {
            assert_eq!(source, "github:user/repo");
            assert!(source.starts_with("github:"));
        }
        _ => panic!("Expected Skill Install command"),
    }
}

// ============================================================================
// TCL-004: skill uninstall {name} - Uninstall a skill
// ============================================================================

#[test]
fn test_tcl_004_skill_uninstall_command_parsing() {
    // Test basic uninstall command
    let cli = Cli::parse_from(["llama-nexus", "skill", "uninstall", "old-skill"]);
    match cli.command {
        Some(Command::Skill(SkillCommand::Uninstall { name, yes })) => {
            assert_eq!(name, "old-skill");
            assert!(!yes, "Should require confirmation by default");
        }
        _ => panic!("Expected Skill Uninstall command"),
    }
}

#[test]
fn test_tcl_004_skill_uninstall_with_yes() {
    // Test uninstall with --yes flag to skip confirmation
    let cli = Cli::parse_from(["llama-nexus", "skill", "uninstall", "old-skill", "--yes"]);
    match cli.command {
        Some(Command::Skill(SkillCommand::Uninstall { yes, .. })) => {
            assert!(yes, "Should skip confirmation");
        }
        _ => panic!("Expected Skill Uninstall command"),
    }
}

#[test]
fn test_tcl_004_skill_uninstall_short_flag() {
    // Test uninstall with -y short flag
    let cli = Cli::parse_from(["llama-nexus", "skill", "uninstall", "old-skill", "-y"]);
    match cli.command {
        Some(Command::Skill(SkillCommand::Uninstall { yes, .. })) => {
            assert!(yes, "Should skip confirmation with -y");
        }
        _ => panic!("Expected Skill Uninstall command"),
    }
}

#[test]
fn test_tcl_004_skill_uninstall_requires_name() {
    // Test that uninstall requires a skill name
    let result = Cli::try_parse_from(["llama-nexus", "skill", "uninstall"]);
    assert!(result.is_err(), "Should fail without skill name");
}

// ============================================================================
// Additional Tests: Search Command
// ============================================================================

#[test]
fn test_skill_search_command() {
    let cli = Cli::parse_from(["llama-nexus", "skill", "search", "code review"]);
    match cli.command {
        Some(Command::Skill(SkillCommand::Search {
            query,
            category,
            limit,
        })) => {
            assert_eq!(query, "code review");
            assert!(category.is_none());
            assert_eq!(limit, 10);
        }
        _ => panic!("Expected Skill Search command"),
    }
}

#[test]
fn test_skill_search_with_category() {
    let cli = Cli::parse_from([
        "llama-nexus",
        "skill",
        "search",
        "test",
        "--category",
        "security",
    ]);
    match cli.command {
        Some(Command::Skill(SkillCommand::Search { category, .. })) => {
            assert_eq!(category, Some("security".to_string()));
        }
        _ => panic!("Expected Skill Search command"),
    }
}

// ============================================================================
// Additional Tests: Update Command
// ============================================================================

#[test]
fn test_skill_update_single() {
    let cli = Cli::parse_from(["llama-nexus", "skill", "update", "my-skill"]);
    match cli.command {
        Some(Command::Skill(SkillCommand::Update { name, all })) => {
            assert_eq!(name, Some("my-skill".to_string()));
            assert!(!all);
        }
        _ => panic!("Expected Skill Update command"),
    }
}

#[test]
fn test_skill_update_all() {
    let cli = Cli::parse_from(["llama-nexus", "skill", "update", "--all"]);
    match cli.command {
        Some(Command::Skill(SkillCommand::Update { name, all })) => {
            assert!(name.is_none());
            assert!(all);
        }
        _ => panic!("Expected Skill Update command"),
    }
}

// ============================================================================
// Additional Tests: Outdated Command
// ============================================================================

#[test]
fn test_skill_outdated_command() {
    let cli = Cli::parse_from(["llama-nexus", "skill", "outdated"]);
    match cli.command {
        Some(Command::Skill(SkillCommand::Outdated)) => {
            // No arguments for outdated command
        }
        _ => panic!("Expected Skill Outdated command"),
    }
}

// ============================================================================
// Additional Tests: Global Config Option
// ============================================================================

#[test]
fn test_global_config_option() {
    let cli = Cli::parse_from([
        "llama-nexus",
        "--config",
        "/path/to/config.toml",
        "skill",
        "list",
    ]);
    assert_eq!(cli.config, PathBuf::from("/path/to/config.toml"));
}

#[test]
fn test_default_config_path() {
    let cli = Cli::parse_from(["llama-nexus", "skill", "list"]);
    assert_eq!(cli.config, PathBuf::from("config.toml"));
}

// ============================================================================
// Additional Tests: Server Options (No Subcommand)
// ============================================================================

#[test]
fn test_server_mode_default() {
    // When no subcommand is provided, server mode options are available
    let cli = Cli::parse_from(["llama-nexus"]);
    assert!(cli.command.is_none());
    assert!(!cli.check_health);
    assert_eq!(cli.check_health_interval, 60);
    assert_eq!(cli.log_destination, "stdout");
}

#[test]
fn test_server_mode_with_health_check() {
    let cli = Cli::parse_from([
        "llama-nexus",
        "--check-health",
        "--check-health-interval",
        "30",
    ]);
    assert!(cli.command.is_none());
    assert!(cli.check_health);
    assert_eq!(cli.check_health_interval, 30);
}

// ============================================================================
// Error Cases
// ============================================================================

#[test]
fn test_invalid_subcommand() {
    let result = Cli::try_parse_from(["llama-nexus", "invalid"]);
    assert!(result.is_err());
}

#[test]
fn test_missing_required_arg() {
    // skill install requires a source
    let result = Cli::try_parse_from(["llama-nexus", "skill", "install"]);
    assert!(result.is_err());
}

#[test]
fn test_invalid_limit_value() {
    // Limit should be a number
    let result = Cli::try_parse_from(["llama-nexus", "skill", "list", "-n", "abc"]);
    assert!(result.is_err());
}
