//! Skill management CLI commands
//!
//! Provides commands for managing skills:
//! - install: Install skills from skillsmp.com or other sources
//! - list: List installed skills
//! - info: Show skill details

use std::path::PathBuf;

use clap::Subcommand;

use crate::error::ServerResult;

/// Skill management subcommands
#[derive(Debug, Subcommand)]
pub enum SkillCommand {
    /// Install a skill from skillsmp.com or other sources
    ///
    /// Examples:
    ///   llama-nexus skill install skillsmp:code-review
    ///   llama-nexus skill install skillsmp:code-review@2.0.0
    Install {
        /// Skill source (e.g., skillsmp:code-review, github:user/repo)
        source: String,

        /// Installation directory (default: ~/.llama-nexus/skills/)
        #[arg(long, short = 'd')]
        dir: Option<PathBuf>,

        /// Enable the skill after installation
        #[arg(long, short = 'e')]
        enable: bool,
    },

    /// List installed skills
    ///
    /// Examples:
    ///   llama-nexus skill list
    ///   llama-nexus skill list --remote
    List {
        /// Show skills from remote marketplace instead of local
        #[arg(long, short = 'r')]
        remote: bool,

        /// Category filter for remote skills
        #[arg(long, short = 'c')]
        category: Option<String>,

        /// Number of skills to show (for remote listing)
        #[arg(long, short = 'n', default_value = "10")]
        limit: usize,
    },

    /// Show detailed information about a skill
    ///
    /// Examples:
    ///   llama-nexus skill info code-review
    ///   llama-nexus skill info skillsmp:code-review
    Info {
        /// Skill name or source (e.g., code-review, skillsmp:code-review)
        name: String,
    },
}

impl SkillCommand {
    /// Execute the skill command
    pub async fn execute(self, config_path: &PathBuf) -> ServerResult<()> {
        match self {
            SkillCommand::Install {
                source,
                dir,
                enable,
            } => install_skill(&source, dir.as_ref(), enable, config_path).await,
            SkillCommand::List {
                remote,
                category,
                limit,
            } => list_skills(remote, category.as_deref(), limit, config_path).await,
            SkillCommand::Info { name } => show_skill_info(&name, config_path).await,
        }
    }
}

/// Install a skill from a source
async fn install_skill(
    source: &str,
    dir: Option<&PathBuf>,
    enable: bool,
    config_path: &PathBuf,
) -> ServerResult<()> {
    use crate::{
        cli::skill::installer::{SkillInstaller, SkillSource},
        config::Config,
    };

    // Load config to get default skills directory
    let config = Config::load(config_path).await?;

    // Determine installation directory
    let install_dir = if let Some(d) = dir {
        d.clone()
    } else if let Some(skill_config) = &config.skill {
        PathBuf::from(shellexpand::tilde(&skill_config.directory()).to_string())
    } else {
        // Default to ~/.llama-nexus/skills/
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(format!("{}/.llama-nexus/skills", home))
    };

    // Parse the source
    let skill_source = SkillSource::parse(source)?;

    println!("Installing skill from: {}", skill_source.display_name());
    println!("Target directory: {}", install_dir.display());

    // Create installer and install
    let installer = SkillInstaller::new(install_dir, config.skill.as_ref());
    let skill_name = installer.install(&skill_source).await?;

    println!("\n✓ Skill '{}' installed successfully!", skill_name);

    if enable {
        println!("  Skill enabled for use.");
    } else {
        println!("  Use 'llama-nexus skill list' to see installed skills.");
    }

    Ok(())
}

/// List installed or remote skills
async fn list_skills(
    remote: bool,
    category: Option<&str>,
    limit: usize,
    config_path: &PathBuf,
) -> ServerResult<()> {
    use crate::config::Config;

    let config = Config::load(config_path).await?;

    if remote {
        // List from skillsmp.com
        list_remote_skills(category, limit, config.skill.as_ref()).await
    } else {
        // List local skills
        list_local_skills(&config).await
    }
}

/// List skills from skillsmp.com
async fn list_remote_skills(
    _category: Option<&str>,
    limit: usize,
    _skill_config: Option<&crate::config::SkillConfig>,
) -> ServerResult<()> {
    use crate::cli::skill::marketplace::SkillsMarketplace;

    println!("Fetching popular skills from skillsmp.com...\n");

    let marketplace = SkillsMarketplace::new(None);
    let skills = marketplace.list_popular(limit).await?;

    if skills.is_empty() {
        println!("No skills found.");
        return Ok(());
    }

    println!("{:<30} {:<50}", "NAME", "DESCRIPTION");
    println!("{}", "-".repeat(80));

    for skill in skills {
        let desc = if skill.description.len() > 47 {
            format!("{}...", &skill.description[..47])
        } else {
            skill.description.clone()
        };
        println!("{:<30} {:<50}", skill.name, desc);
    }

    println!("\nInstall a skill with: llama-nexus skill install skillsmp:<name>");

    Ok(())
}

/// List locally installed skills
async fn list_local_skills(config: &crate::config::Config) -> ServerResult<()> {
    let skills_dir = if let Some(skill_config) = &config.skill {
        PathBuf::from(shellexpand::tilde(&skill_config.directory()).to_string())
    } else {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(format!("{}/.llama-nexus/skills", home))
    };

    if !skills_dir.exists() {
        println!("No skills directory found at: {}", skills_dir.display());
        println!("\nInstall skills with: llama-nexus skill install skillsmp:<name>");
        return Ok(());
    }

    // Discover skills by scanning subdirectories for SKILL.md
    let skills = discover_local_skills(&skills_dir).await;

    if skills.is_empty() {
        println!("No skills installed in: {}", skills_dir.display());
        println!("\nInstall skills with: llama-nexus skill install skillsmp:<name>");
        return Ok(());
    }

    println!("Installed skills in: {}\n", skills_dir.display());
    println!("{:<25} {:<50}", "NAME", "DESCRIPTION");
    println!("{}", "-".repeat(75));

    for (name, description) in &skills {
        let desc = if description.len() > 47 {
            format!("{}...", &description[..47])
        } else {
            description.clone()
        };
        println!("{:<25} {:<50}", name, desc);
    }

    println!("\nTotal: {} skill(s)", skills.len());

    Ok(())
}

/// Discover locally installed skills by scanning for SKILL.md files
async fn discover_local_skills(skills_dir: &PathBuf) -> Vec<(String, String)> {
    use crate::skills::SkillParser;

    let mut skills = Vec::new();

    if let Ok(entries) = std::fs::read_dir(skills_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let skill_md = path.join("SKILL.md");
                if skill_md.exists()
                    && let Ok(content) = tokio::fs::read_to_string(&skill_md).await
                    && let Ok(skill) = SkillParser::parse(&content, &path).await
                {
                    skills.push((
                        skill.metadata.name.clone(),
                        skill.metadata.description.clone(),
                    ));
                }
            }
        }
    }

    skills.sort_by(|a, b| a.0.cmp(&b.0));
    skills
}

/// Show detailed information about a skill
async fn show_skill_info(name: &str, config_path: &PathBuf) -> ServerResult<()> {
    use crate::config::Config;

    let config = Config::load(config_path).await?;

    // Check if it's a remote skill reference
    // if name.starts_with("skillsmp:") {
    if let Some(stripped) = name.strip_prefix("skillsmp:") {
        show_remote_skill_info(stripped, config.skill.as_ref()).await
    } else {
        show_local_skill_info(name, &config).await
    }
}

/// Show information about a remote skill from skillsmp.com
async fn show_remote_skill_info(
    skill_name: &str,
    _skill_config: Option<&crate::config::SkillConfig>,
) -> ServerResult<()> {
    use crate::cli::skill::marketplace::SkillsMarketplace;

    println!("Fetching skill info from skillsmp.com...\n");

    let marketplace = SkillsMarketplace::new(None);
    let skill = marketplace.get_skill_info(skill_name).await?;

    println!("Name:        {}", skill.name);
    println!("Description: {}", skill.description);
    if let Some(version) = &skill.version {
        println!("Version:     {}", version);
    }
    if let Some(author) = &skill.author {
        println!("Author:      {}", author);
    }
    if let Some(license) = &skill.license {
        println!("License:     {}", license);
    }
    if !skill.allowed_tools.is_empty() {
        println!("Tools:       {}", skill.allowed_tools.join(", "));
    }
    println!(
        "\nInstall with: llama-nexus skill install skillsmp:{}",
        skill_name
    );

    Ok(())
}

/// Show information about a locally installed skill
async fn show_local_skill_info(
    skill_name: &str,
    config: &crate::config::Config,
) -> ServerResult<()> {
    use crate::skills::SkillParser;

    let skills_dir = if let Some(skill_config) = &config.skill {
        PathBuf::from(shellexpand::tilde(&skill_config.directory()).to_string())
    } else {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(format!("{}/.llama-nexus/skills", home))
    };

    let skill_path = skills_dir.join(skill_name);
    if !skill_path.exists() {
        println!(
            "Skill '{}' not found in: {}",
            skill_name,
            skills_dir.display()
        );
        println!("\nTry: llama-nexus skill info skillsmp:{}", skill_name);
        return Ok(());
    }

    let skill_md = skill_path.join("SKILL.md");
    if !skill_md.exists() {
        println!("Skill '{}' is missing SKILL.md file", skill_name);
        return Ok(());
    }

    let content = tokio::fs::read_to_string(&skill_md).await.map_err(|e| {
        crate::error::ServerError::Operation(format!("Failed to read SKILL.md: {}", e))
    })?;

    match SkillParser::parse(&content, &skill_path).await {
        Ok(skill) => {
            println!("Name:        {}", skill.metadata.name);
            println!("Description: {}", skill.metadata.description);
            println!("Enabled:     {}", skill.enabled);
            if let Some(license) = &skill.metadata.license {
                println!("License:     {}", license);
            }
            let tools = skill.metadata.get_allowed_tools();
            if !tools.is_empty() {
                println!("Tools:       {}", tools.join(", "));
            }
            if let Some(scripts) = skill.metadata.get_allowed_scripts() {
                println!("Scripts:     {}", scripts.join(", "));
            }
            if !skill.scripts.is_empty() {
                let script_names: Vec<_> = skill.scripts.iter().map(|s| s.name.as_str()).collect();
                println!("Available:   {}", script_names.join(", "));
            }
            println!("Path:        {}", skill_path.display());
        }
        Err(e) => {
            println!("Error parsing skill '{}': {}", skill_name, e);
        }
    }

    Ok(())
}

// Submodules for skill management
pub mod installer;
pub mod marketplace;
