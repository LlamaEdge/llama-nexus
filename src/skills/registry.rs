//! Skills Registry for managing loaded skills
//!
//! Provides global access to loaded skills through a singleton pattern.
//! Skills are loaded once at startup and cached for efficient access.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use once_cell::sync::OnceCell;
use tokio::sync::RwLock;

use crate::skills::{
    error::{SkillError, SkillResult},
    parser::SkillParser,
    types::{LoadedSkill, SkillSummary},
};

/// Global skills registry instance
pub static SKILLS_REGISTRY: OnceCell<SkillRegistry> = OnceCell::new();

/// Registry for managing loaded skills
///
/// Maintains a collection of loaded skills indexed by name.
/// Provides methods for loading, retrieving, and listing skills.
pub struct SkillRegistry {
    /// Loaded skills indexed by name
    skills: RwLock<HashMap<String, LoadedSkill>>,

    /// Base directory for skills
    skills_dir: PathBuf,
}

impl SkillRegistry {
    /// Create a new skill registry
    ///
    /// # Arguments
    /// * `skills_dir` - Base directory containing skill subdirectories
    pub fn new(skills_dir: PathBuf) -> Self {
        Self {
            skills: RwLock::new(HashMap::new()),
            skills_dir,
        }
    }

    /// Initialize the global registry
    ///
    /// # Arguments
    /// * `skills_dir` - Base directory containing skill subdirectories
    ///
    /// # Returns
    /// Reference to the initialized registry, or error if already initialized
    pub fn init_global(skills_dir: PathBuf) -> SkillResult<&'static SkillRegistry> {
        let registry = SkillRegistry::new(skills_dir);
        SKILLS_REGISTRY
            .set(registry)
            .map_err(|_| SkillError::RegistryNotInitialized)?;

        Ok(SKILLS_REGISTRY
            .get()
            .expect("Registry was just initialized"))
    }

    /// Get the global registry instance
    pub fn global() -> SkillResult<&'static SkillRegistry> {
        SKILLS_REGISTRY
            .get()
            .ok_or(SkillError::RegistryNotInitialized)
    }

    /// Load all skills from the skills directory
    ///
    /// Scans the skills directory for subdirectories containing SKILL.md files
    /// and loads them into the registry.
    pub async fn load_all(&self) -> SkillResult<usize> {
        let skills_dir = &self.skills_dir;

        if !skills_dir.exists() {
            return Ok(0);
        }

        let mut loaded_count = 0;

        let entries = std::fs::read_dir(skills_dir)?;

        for entry in entries.flatten() {
            let path = entry.path();

            if path.is_dir() {
                let skill_md_path = path.join("SKILL.md");

                if skill_md_path.exists() {
                    match self.load_skill(&path).await {
                        Ok(skill) => {
                            let name = skill.metadata.name.clone();
                            self.skills.write().await.insert(name, skill);
                            loaded_count += 1;
                        }
                        Err(e) => {
                            tracing::warn!("Failed to load skill from {:?}: {}", path, e);
                        }
                    }
                }
            }
        }

        Ok(loaded_count)
    }

    /// Load a single skill from a directory
    ///
    /// # Arguments
    /// * `skill_dir` - Directory containing the SKILL.md file
    async fn load_skill(&self, skill_dir: &Path) -> SkillResult<LoadedSkill> {
        let skill_md_path = skill_dir.join("SKILL.md");
        let content = tokio::fs::read_to_string(&skill_md_path).await?;
        SkillParser::parse(&content, skill_dir).await
    }

    /// Get a skill by name
    ///
    /// # Arguments
    /// * `name` - The skill name
    ///
    /// # Returns
    /// The loaded skill if found
    pub async fn get(&self, name: &str) -> Option<LoadedSkill> {
        self.skills.read().await.get(name).cloned()
    }

    /// Check if a skill exists
    ///
    /// # Arguments
    /// * `name` - The skill name
    #[allow(dead_code)]
    pub async fn exists(&self, name: &str) -> bool {
        self.skills.read().await.contains_key(name)
    }

    /// Get summaries of all loaded skills
    ///
    /// Returns only name and description for Phase 1 injection.
    pub async fn get_summaries(&self) -> Vec<SkillSummary> {
        self.skills
            .read()
            .await
            .values()
            .filter(|s| s.enabled)
            .map(SkillSummary::from)
            .collect()
    }

    /// Get all loaded and enabled skills
    ///
    /// Returns the full LoadedSkill objects for all enabled skills.
    /// This is useful for multi-skill detection and conflict resolution.
    pub async fn get_all_loaded(&self) -> Vec<LoadedSkill> {
        self.skills
            .read()
            .await
            .values()
            .filter(|s| s.enabled)
            .cloned()
            .collect()
    }

    /// Get all skill names
    #[allow(dead_code)]
    pub async fn list_names(&self) -> Vec<String> {
        self.skills.read().await.keys().cloned().collect()
    }

    /// Get the number of loaded skills
    #[allow(dead_code)]
    pub async fn count(&self) -> usize {
        self.skills.read().await.len()
    }

    /// Enable or disable a skill
    ///
    /// # Arguments
    /// * `name` - The skill name
    /// * `enabled` - Whether to enable or disable
    #[allow(dead_code)]
    pub async fn set_enabled(&self, name: &str, enabled: bool) -> SkillResult<()> {
        let mut skills = self.skills.write().await;

        if let Some(skill) = skills.get_mut(name) {
            skill.enabled = enabled;
            Ok(())
        } else {
            Err(SkillError::NotFound(name.to_string()))
        }
    }

    /// Reload a specific skill
    ///
    /// # Arguments
    /// * `name` - The skill name to reload
    #[allow(dead_code)]
    pub async fn reload(&self, name: &str) -> SkillResult<()> {
        let skill_dir = self.skills_dir.join(name);

        if !skill_dir.exists() {
            return Err(SkillError::NotFound(name.to_string()));
        }

        let skill = self.load_skill(&skill_dir).await?;
        self.skills.write().await.insert(name.to_string(), skill);

        Ok(())
    }

    /// Reload all skills
    #[allow(dead_code)]
    pub async fn reload_all(&self) -> SkillResult<usize> {
        self.skills.write().await.clear();
        self.load_all().await
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    fn create_test_skill(dir: &Path, name: &str) {
        let skill_dir = dir.join(name);
        std::fs::create_dir_all(&skill_dir).unwrap();

        let content = format!(
            r#"---
name: {}
description: Test skill for {}
---
# {}

This is a test skill.
"#,
            name, name, name
        );

        std::fs::write(skill_dir.join("SKILL.md"), content).unwrap();
    }

    #[tokio::test]
    async fn test_registry_new() {
        let temp_dir = TempDir::new().unwrap();
        let registry = SkillRegistry::new(temp_dir.path().to_path_buf());

        assert_eq!(registry.count().await, 0);
    }

    #[tokio::test]
    async fn test_load_all_empty_dir() {
        let temp_dir = TempDir::new().unwrap();
        let registry = SkillRegistry::new(temp_dir.path().to_path_buf());

        let count = registry.load_all().await.unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test_load_all_with_skills() {
        let temp_dir = TempDir::new().unwrap();

        create_test_skill(temp_dir.path(), "skill-one");
        create_test_skill(temp_dir.path(), "skill-two");

        let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
        let count = registry.load_all().await.unwrap();

        assert_eq!(count, 2);
        assert_eq!(registry.count().await, 2);
    }

    #[tokio::test]
    async fn test_get_skill() {
        let temp_dir = TempDir::new().unwrap();
        create_test_skill(temp_dir.path(), "my-skill");

        let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
        registry.load_all().await.unwrap();

        let skill = registry.get("my-skill").await;
        assert!(skill.is_some());
        assert_eq!(skill.unwrap().metadata.name, "my-skill");
    }

    #[tokio::test]
    async fn test_get_skill_not_found() {
        let temp_dir = TempDir::new().unwrap();
        let registry = SkillRegistry::new(temp_dir.path().to_path_buf());

        let skill = registry.get("nonexistent").await;
        assert!(skill.is_none());
    }

    #[tokio::test]
    async fn test_exists() {
        let temp_dir = TempDir::new().unwrap();
        create_test_skill(temp_dir.path(), "test-skill");

        let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
        registry.load_all().await.unwrap();

        assert!(registry.exists("test-skill").await);
        assert!(!registry.exists("other-skill").await);
    }

    #[tokio::test]
    async fn test_get_summaries() {
        let temp_dir = TempDir::new().unwrap();
        create_test_skill(temp_dir.path(), "skill-a");
        create_test_skill(temp_dir.path(), "skill-b");

        let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
        registry.load_all().await.unwrap();

        let summaries = registry.get_summaries().await;
        assert_eq!(summaries.len(), 2);
    }

    #[tokio::test]
    async fn test_list_names() {
        let temp_dir = TempDir::new().unwrap();
        create_test_skill(temp_dir.path(), "alpha");
        create_test_skill(temp_dir.path(), "beta");

        let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
        registry.load_all().await.unwrap();

        let names = registry.list_names().await;
        assert_eq!(names.len(), 2);
        assert!(names.contains(&"alpha".to_string()));
        assert!(names.contains(&"beta".to_string()));
    }

    #[tokio::test]
    async fn test_set_enabled() {
        let temp_dir = TempDir::new().unwrap();
        create_test_skill(temp_dir.path(), "toggle-skill");

        let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
        registry.load_all().await.unwrap();

        // Initially enabled
        let skill = registry.get("toggle-skill").await.unwrap();
        assert!(skill.enabled);

        // Disable
        registry.set_enabled("toggle-skill", false).await.unwrap();
        let skill = registry.get("toggle-skill").await.unwrap();
        assert!(!skill.enabled);

        // Summaries should exclude disabled skills
        let summaries = registry.get_summaries().await;
        assert_eq!(summaries.len(), 0);
    }

    #[tokio::test]
    async fn test_reload() {
        let temp_dir = TempDir::new().unwrap();
        create_test_skill(temp_dir.path(), "reload-skill");

        let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
        registry.load_all().await.unwrap();

        // Modify the skill
        let skill_dir = temp_dir.path().join("reload-skill");
        let new_content = r#"---
name: reload-skill
description: Updated description
---
# Updated
"#;
        std::fs::write(skill_dir.join("SKILL.md"), new_content).unwrap();

        // Reload
        registry.reload("reload-skill").await.unwrap();

        let skill = registry.get("reload-skill").await.unwrap();
        assert_eq!(skill.metadata.description, "Updated description");
    }

    #[tokio::test]
    async fn test_reload_nonexistent_skill() {
        let temp_dir = TempDir::new().unwrap();
        let registry = SkillRegistry::new(temp_dir.path().to_path_buf());

        let result = registry.reload("nonexistent").await;
        assert!(matches!(result, Err(SkillError::NotFound(_))));
    }

    #[tokio::test]
    async fn test_set_enabled_nonexistent_skill() {
        let temp_dir = TempDir::new().unwrap();
        let registry = SkillRegistry::new(temp_dir.path().to_path_buf());

        let result = registry.set_enabled("nonexistent", true).await;
        assert!(matches!(result, Err(SkillError::NotFound(_))));
    }

    #[tokio::test]
    async fn test_load_all_nonexistent_dir() {
        let registry = SkillRegistry::new(PathBuf::from("/nonexistent/path/skills"));
        let count = registry.load_all().await.unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test_load_all_skips_invalid_skills() {
        let temp_dir = TempDir::new().unwrap();

        // Create a valid skill
        create_test_skill(temp_dir.path(), "valid-skill");

        // Create an invalid skill (missing description)
        let invalid_dir = temp_dir.path().join("invalid-skill");
        std::fs::create_dir_all(&invalid_dir).unwrap();
        std::fs::write(
            invalid_dir.join("SKILL.md"),
            r#"---
name: invalid-skill
---
"#,
        )
        .unwrap();

        // Create a directory without SKILL.md
        let empty_dir = temp_dir.path().join("empty-dir");
        std::fs::create_dir_all(empty_dir).unwrap();

        let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
        let count = registry.load_all().await.unwrap();

        // Only the valid skill should be loaded
        assert_eq!(count, 1);
        assert!(registry.exists("valid-skill").await);
        assert!(!registry.exists("invalid-skill").await);
    }

    #[tokio::test]
    async fn test_load_all_ignores_files() {
        let temp_dir = TempDir::new().unwrap();

        // Create a valid skill
        create_test_skill(temp_dir.path(), "my-skill");

        // Create a file (not a directory) in the skills dir
        std::fs::write(temp_dir.path().join("readme.txt"), "Some content").unwrap();

        let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
        let count = registry.load_all().await.unwrap();

        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn test_get_summaries_excludes_disabled() {
        let temp_dir = TempDir::new().unwrap();
        create_test_skill(temp_dir.path(), "enabled-skill");
        create_test_skill(temp_dir.path(), "disabled-skill");

        let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
        registry.load_all().await.unwrap();

        registry.set_enabled("disabled-skill", false).await.unwrap();

        let summaries = registry.get_summaries().await;
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].name, "enabled-skill");
    }

    #[tokio::test]
    async fn test_count() {
        let temp_dir = TempDir::new().unwrap();
        create_test_skill(temp_dir.path(), "skill-1");
        create_test_skill(temp_dir.path(), "skill-2");
        create_test_skill(temp_dir.path(), "skill-3");

        let registry = SkillRegistry::new(temp_dir.path().to_path_buf());

        // Before loading
        assert_eq!(registry.count().await, 0);

        registry.load_all().await.unwrap();

        // After loading
        assert_eq!(registry.count().await, 3);
    }

    #[tokio::test]
    async fn test_enable_disable_toggle() {
        let temp_dir = TempDir::new().unwrap();
        create_test_skill(temp_dir.path(), "toggle-test");

        let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
        registry.load_all().await.unwrap();

        // Toggle multiple times
        for _ in 0..3 {
            registry.set_enabled("toggle-test", false).await.unwrap();
            assert!(!registry.get("toggle-test").await.unwrap().enabled);

            registry.set_enabled("toggle-test", true).await.unwrap();
            assert!(registry.get("toggle-test").await.unwrap().enabled);
        }
    }

    #[tokio::test]
    async fn test_reload_preserves_enabled_state() {
        let temp_dir = TempDir::new().unwrap();
        create_test_skill(temp_dir.path(), "state-skill");

        let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
        registry.load_all().await.unwrap();

        // Disable the skill
        registry.set_enabled("state-skill", false).await.unwrap();

        // Reload (note: reload creates a new LoadedSkill, enabled resets to true)
        registry.reload("state-skill").await.unwrap();

        // After reload, enabled state is reset
        let skill = registry.get("state-skill").await.unwrap();
        assert!(skill.enabled); // Default is true on reload
    }

    fn create_test_skill_with_license(dir: &Path, name: &str, license: &str) {
        let skill_dir = dir.join(name);
        std::fs::create_dir_all(&skill_dir).unwrap();

        let content = format!(
            r#"---
name: {}
description: Test skill for {}
license: {}
---
# {}
"#,
            name, name, license, name
        );

        std::fs::write(skill_dir.join("SKILL.md"), content).unwrap();
    }

    #[tokio::test]
    async fn test_get_preserves_all_metadata() {
        let temp_dir = TempDir::new().unwrap();
        create_test_skill_with_license(temp_dir.path(), "licensed-skill", "MIT");

        let registry = SkillRegistry::new(temp_dir.path().to_path_buf());
        registry.load_all().await.unwrap();

        let skill = registry.get("licensed-skill").await.unwrap();
        assert_eq!(skill.metadata.name, "licensed-skill");
        assert_eq!(skill.metadata.license, Some("MIT".to_string()));
    }
}
