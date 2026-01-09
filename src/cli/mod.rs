//! CLI subcommands for llama-nexus
//!
//! Provides command-line interface for:
//! - Server mode (default): Start the API server
//! - Skill management: Install, list, info for skills

pub mod skill;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

/// LlamaEdge Nexus - A gateway service for LLM backends
#[derive(Debug, Parser)]
#[command(version = env!("CARGO_PKG_VERSION"), about = "LlamaEdge Nexus - A gateway service for LLM backends")]
pub struct Cli {
    /// Path to the config file
    #[arg(long, default_value = "config.toml", value_parser = clap::value_parser!(PathBuf), global = true)]
    pub config: PathBuf,

    /// Subcommand to execute
    #[command(subcommand)]
    pub command: Option<Command>,

    // Server-specific options (used when no subcommand is provided)
    /// Enable health check for downstream servers
    #[arg(long, default_value = "false")]
    pub check_health: bool,

    /// Health check interval for downstream servers in seconds
    #[arg(long, default_value = "60")]
    pub check_health_interval: u64,

    /// Root path for the Web UI files
    #[arg(long, default_value = "chatbot-ui")]
    pub web_ui: PathBuf,

    /// Log destination: "stdout", "file", or "both"
    #[arg(long, default_value = "stdout")]
    pub log_destination: String,

    /// Log file path (required when log_destination is "file" or "both")
    #[arg(long)]
    pub log_file: Option<String>,
}

/// Available commands
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Manage skills (install, list, info)
    #[command(subcommand)]
    Skill(skill::SkillCommand),
}
