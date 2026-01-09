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

    /// Search for skills on skillsmp.com
    ///
    /// Examples:
    ///   llama-nexus skill search "code review"
    ///   llama-nexus skill search --category development
    Search {
        /// Search query
        query: String,

        /// Category filter (e.g., development, security, documentation)
        #[arg(long, short = 'c')]
        category: Option<String>,

        /// Number of results to show
        #[arg(long, short = 'n', default_value = "10")]
        limit: usize,
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

    /// Update installed skills
    ///
    /// Examples:
    ///   llama-nexus skill update code-review
    ///   llama-nexus skill update --all
    Update {
        /// Skill name to update (omit for --all)
        name: Option<String>,

        /// Update all installed skills
        #[arg(long, short = 'a')]
        all: bool,
    },

    /// Check for outdated skills
    ///
    /// Examples:
    ///   llama-nexus skill outdated
    Outdated,

    /// Uninstall a skill
    ///
    /// Examples:
    ///   llama-nexus skill uninstall code-review
    Uninstall {
        /// Skill name to uninstall
        name: String,

        /// Skip confirmation prompt
        #[arg(long, short = 'y')]
        yes: bool,
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
            SkillCommand::Search {
                query,
                category,
                limit,
            } => search_skills(&query, category.as_deref(), limit, config_path).await,
            SkillCommand::List {
                remote,
                category,
                limit,
            } => list_skills(remote, category.as_deref(), limit, config_path).await,
            SkillCommand::Info { name } => show_skill_info(&name, config_path).await,
            SkillCommand::Update { name, all } => {
                update_skills(name.as_deref(), all, config_path).await
            }
            SkillCommand::Outdated => check_outdated_skills(config_path).await,
            SkillCommand::Uninstall { name, yes } => uninstall_skill(&name, yes, config_path).await,
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

/// Search for skills on skillsmp.com
async fn search_skills(
    query: &str,
    category: Option<&str>,
    limit: usize,
    config_path: &PathBuf,
) -> ServerResult<()> {
    use crate::{cli::skill::marketplace::SkillsMarketplace, config::Config};

    let config = Config::load(config_path).await?;

    // Get API key from config or environment
    let api_key = config
        .skill
        .as_ref()
        .and_then(|c| c.market.as_ref())
        .and_then(|m| m.api_key.clone())
        .or_else(|| std::env::var("SKILLSMP_API_KEY").ok());

    let marketplace = SkillsMarketplace::new(api_key);

    // Construct search query with category filter if provided
    let search_query = if let Some(cat) = category {
        format!("{} category:{}", query, cat)
    } else {
        query.to_string()
    };

    println!("Searching skillsmp.com for '{}'...\n", query);

    let skills = marketplace.search(&search_query, limit).await?;

    if skills.is_empty() {
        println!("No skills found matching '{}'.", query);
        return Ok(());
    }

    println!("{:<30} {:<50}", "NAME", "DESCRIPTION");
    println!("{}", "-".repeat(80));

    for skill in &skills {
        let desc = if skill.description.len() > 47 {
            format!("{}...", &skill.description[..47])
        } else {
            skill.description.clone()
        };
        println!("{:<30} {:<50}", skill.name, desc);
    }

    println!("\nFound {} skill(s).", skills.len());
    println!("Install a skill with: llama-nexus skill install skillsmp:<name>");

    Ok(())
}

/// Update installed skills
async fn update_skills(name: Option<&str>, all: bool, config_path: &PathBuf) -> ServerResult<()> {
    use crate::{
        cli::skill::{installer::SkillInstaller, lockfile::SkillLockFile},
        config::Config,
    };

    let config = Config::load(config_path).await?;

    let skills_dir = if let Some(skill_config) = &config.skill {
        PathBuf::from(shellexpand::tilde(&skill_config.directory()).to_string())
    } else {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(format!("{}/.llama-nexus/skills", home))
    };

    if !skills_dir.exists() {
        println!("No skills directory found at: {}", skills_dir.display());
        return Ok(());
    }

    // Discover installed skills with lock files
    let skills_to_update = if all {
        discover_updatable_skills(&skills_dir).await
    } else if let Some(skill_name) = name {
        let skill_path = skills_dir.join(skill_name);
        if !skill_path.exists() {
            println!("Skill '{}' not found.", skill_name);
            return Ok(());
        }
        let lock_path = skill_path.join("skill.lock");
        if lock_path.exists() {
            if let Ok(lock) = SkillLockFile::load(&lock_path).await {
                vec![(skill_name.to_string(), lock)]
            } else {
                println!(
                    "Skill '{}' has no valid lock file, cannot update.",
                    skill_name
                );
                return Ok(());
            }
        } else {
            println!(
                "Skill '{}' was not installed from marketplace (no skill.lock).",
                skill_name
            );
            println!(
                "To reinstall from marketplace: llama-nexus skill install skillsmp:{}",
                skill_name
            );
            return Ok(());
        }
    } else {
        println!("Please specify a skill name or use --all to update all skills.");
        return Ok(());
    };

    if skills_to_update.is_empty() {
        println!("No updatable skills found.");
        return Ok(());
    }

    let installer = SkillInstaller::new(skills_dir.clone(), config.skill.as_ref());
    let mut updated_count = 0;

    for (skill_name, lock) in skills_to_update {
        println!("Checking '{}' for updates...", skill_name);

        // Get source from lock file
        if let Some(source) = &lock.source {
            // Remove old skill directory
            let skill_path = skills_dir.join(&skill_name);
            if let Err(e) = tokio::fs::remove_dir_all(&skill_path).await {
                println!("  Warning: Failed to remove old version: {}", e);
            }

            // Reinstall from source
            match crate::cli::skill::installer::SkillSource::parse(source) {
                Ok(skill_source) => match installer.install(&skill_source).await {
                    Ok(new_name) => {
                        println!("  Updated '{}' successfully.", new_name);
                        updated_count += 1;
                    }
                    Err(e) => {
                        println!("  Failed to update '{}': {}", skill_name, e);
                    }
                },
                Err(e) => {
                    println!("  Invalid source in lock file: {}", e);
                }
            }
        } else {
            println!("  No source information in lock file, skipping.");
        }
    }

    println!("\nUpdated {} skill(s).", updated_count);

    Ok(())
}

/// Check for outdated skills
async fn check_outdated_skills(config_path: &PathBuf) -> ServerResult<()> {
    use crate::config::Config;

    let config = Config::load(config_path).await?;

    let skills_dir = if let Some(skill_config) = &config.skill {
        PathBuf::from(shellexpand::tilde(&skill_config.directory()).to_string())
    } else {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(format!("{}/.llama-nexus/skills", home))
    };

    if !skills_dir.exists() {
        println!("No skills directory found at: {}", skills_dir.display());
        return Ok(());
    }

    // Discover installed skills with lock files
    let skills = discover_updatable_skills(&skills_dir).await;

    if skills.is_empty() {
        println!("No skills with version tracking found.");
        println!(
            "Skills installed from marketplace will have a skill.lock file for version tracking."
        );
        return Ok(());
    }

    println!("{:<25} {:<15} {:<40}", "SKILL", "VERSION", "SOURCE");
    println!("{}", "-".repeat(80));

    for (name, lock) in &skills {
        let version = lock.version.as_deref().unwrap_or("unknown");
        let source = lock.source.as_deref().unwrap_or("unknown");
        let source_display = if source.len() > 37 {
            format!("{}...", &source[..37])
        } else {
            source.to_string()
        };
        println!("{:<25} {:<15} {:<40}", name, version, source_display);
    }

    println!("\nTotal: {} skill(s) with version tracking.", skills.len());
    println!("\nNote: Version comparison with remote is not yet implemented.");
    println!("Use 'llama-nexus skill update <name>' to reinstall from the latest source.");

    Ok(())
}

/// Discover skills that have lock files (updatable)
async fn discover_updatable_skills(
    skills_dir: &PathBuf,
) -> Vec<(String, crate::cli::skill::lockfile::SkillLockFile)> {
    use crate::cli::skill::lockfile::SkillLockFile;

    let mut skills = Vec::new();

    if let Ok(entries) = std::fs::read_dir(skills_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let lock_path = path.join("skill.lock");
                if lock_path.exists()
                    && let Ok(lock) = SkillLockFile::load(&lock_path).await
                    && let Some(name) = path.file_name().and_then(|n| n.to_str())
                {
                    skills.push((name.to_string(), lock));
                }
            }
        }
    }

    skills.sort_by(|a, b| a.0.cmp(&b.0));
    skills
}

/// Uninstall a skill
async fn uninstall_skill(
    name: &str,
    skip_confirm: bool,
    config_path: &PathBuf,
) -> ServerResult<()> {
    use crate::config::Config;

    let config = Config::load(config_path).await?;

    let skills_dir = if let Some(skill_config) = &config.skill {
        PathBuf::from(shellexpand::tilde(&skill_config.directory()).to_string())
    } else {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(format!("{}/.llama-nexus/skills", home))
    };

    let skill_path = skills_dir.join(name);

    if !skill_path.exists() {
        println!("Skill '{}' not found in: {}", name, skills_dir.display());
        return Ok(());
    }

    // Confirmation prompt (unless --yes is provided)
    if !skip_confirm {
        println!("This will remove the skill '{}' from:", name);
        println!("  {}", skill_path.display());
        println!();
        print!("Are you sure? [y/N] ");

        use std::io::{self, Write};
        io::stdout().flush().ok();

        let mut input = String::new();
        if io::stdin().read_line(&mut input).is_ok() {
            let input = input.trim().to_lowercase();
            if input != "y" && input != "yes" {
                println!("Cancelled.");
                return Ok(());
            }
        } else {
            println!("Cancelled.");
            return Ok(());
        }
    }

    // Remove the skill directory
    match tokio::fs::remove_dir_all(&skill_path).await {
        Ok(_) => {
            println!("Skill '{}' uninstalled successfully.", name);
        }
        Err(e) => {
            return Err(crate::error::ServerError::Operation(format!(
                "Failed to remove skill '{}': {}",
                name, e
            )));
        }
    }

    Ok(())
}

// Submodules for skill management
pub mod installer;
pub mod lockfile;
pub mod marketplace;
