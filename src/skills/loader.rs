//! Resource loader for Skills
//!
//! Loads additional resources from skill directories:
//! - references/: Reference documents (.md, .txt)
//! - scripts/: Executable scripts
//! - assets/: Templates and other assets

use std::path::Path;

use crate::skills::types::ScriptInfo;

/// Loader for skill resources
pub struct SkillLoader;

impl SkillLoader {
    /// Load reference documents from the references/ directory
    ///
    /// Reads all .md and .txt files from the references/ subdirectory
    ///
    /// # Arguments
    /// * `skill_dir` - The skill directory path
    ///
    /// # Returns
    /// A vector of file contents
    pub async fn load_references(skill_dir: &Path) -> Vec<String> {
        let refs_dir = skill_dir.join("references");
        if !refs_dir.exists() {
            return Vec::new();
        }

        let mut references = Vec::new();

        if let Ok(entries) = std::fs::read_dir(&refs_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

                    if (ext == "md" || ext == "txt")
                        && let Ok(content) = tokio::fs::read_to_string(&path).await
                    {
                        references.push(content);
                    }
                }
            }
        }

        references
    }

    /// List available scripts from the scripts/ directory
    ///
    /// # Arguments
    /// * `skill_dir` - The skill directory path
    ///
    /// # Returns
    /// A vector of script information
    pub async fn list_scripts(skill_dir: &Path) -> Vec<ScriptInfo> {
        let scripts_dir = skill_dir.join("scripts");
        if !scripts_dir.exists() {
            return Vec::new();
        }

        let mut scripts = Vec::new();

        if let Ok(entries) = std::fs::read_dir(&scripts_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    let name = path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("")
                        .to_string();

                    let executable = Self::is_executable(&path);

                    scripts.push(ScriptInfo {
                        name,
                        path,
                        executable,
                    });
                }
            }
        }

        scripts
    }

    /// Load an asset file from the assets/ directory
    ///
    /// # Arguments
    /// * `skill_dir` - The skill directory path
    /// * `asset_name` - The name of the asset file
    ///
    /// # Returns
    /// The file contents as bytes, or None if not found
    pub async fn load_asset(skill_dir: &Path, asset_name: &str) -> Option<Vec<u8>> {
        let asset_path = skill_dir.join("assets").join(asset_name);
        tokio::fs::read(&asset_path).await.ok()
    }

    /// Load an asset file as a string
    ///
    /// # Arguments
    /// * `skill_dir` - The skill directory path
    /// * `asset_name` - The name of the asset file
    ///
    /// # Returns
    /// The file contents as a string, or None if not found
    pub async fn load_asset_string(skill_dir: &Path, asset_name: &str) -> Option<String> {
        let asset_path = skill_dir.join("assets").join(asset_name);
        tokio::fs::read_to_string(&asset_path).await.ok()
    }

    /// Check if the skill has additional resources
    ///
    /// # Arguments
    /// * `skill_dir` - The skill directory path
    ///
    /// # Returns
    /// true if the skill has scripts/, references/, or assets/ directories
    pub fn has_resources(skill_dir: &Path) -> bool {
        skill_dir.join("scripts").exists()
            || skill_dir.join("references").exists()
            || skill_dir.join("assets").exists()
    }

    /// Check if a file is executable
    #[cfg(unix)]
    fn is_executable(path: &Path) -> bool {
        use std::os::unix::fs::PermissionsExt;

        if let Ok(metadata) = std::fs::metadata(path) {
            let permissions = metadata.permissions();
            permissions.mode() & 0o111 != 0
        } else {
            false
        }
    }

    /// Check if a file is executable (Windows)
    #[cfg(windows)]
    fn is_executable(path: &Path) -> bool {
        // On Windows, check for common executable extensions
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        matches!(ext.as_str(), "exe" | "bat" | "cmd" | "ps1")
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[tokio::test]
    async fn test_load_references_empty() {
        let temp_dir = TempDir::new().unwrap();
        let refs = SkillLoader::load_references(temp_dir.path()).await;
        assert!(refs.is_empty());
    }

    #[tokio::test]
    async fn test_load_references_with_files() {
        let temp_dir = TempDir::new().unwrap();
        let refs_dir = temp_dir.path().join("references");
        std::fs::create_dir(&refs_dir).unwrap();

        std::fs::write(refs_dir.join("doc1.md"), "# Document 1").unwrap();
        std::fs::write(refs_dir.join("doc2.txt"), "Plain text").unwrap();
        std::fs::write(refs_dir.join("ignored.json"), "{}").unwrap();

        let refs = SkillLoader::load_references(temp_dir.path()).await;
        assert_eq!(refs.len(), 2);
    }

    #[tokio::test]
    async fn test_list_scripts_empty() {
        let temp_dir = TempDir::new().unwrap();
        let scripts = SkillLoader::list_scripts(temp_dir.path()).await;
        assert!(scripts.is_empty());
    }

    #[tokio::test]
    async fn test_list_scripts_with_files() {
        let temp_dir = TempDir::new().unwrap();
        let scripts_dir = temp_dir.path().join("scripts");
        std::fs::create_dir(&scripts_dir).unwrap();

        std::fs::write(scripts_dir.join("script1.sh"), "#!/bin/bash").unwrap();
        std::fs::write(scripts_dir.join("script2.py"), "#!/usr/bin/env python").unwrap();

        let scripts = SkillLoader::list_scripts(temp_dir.path()).await;
        assert_eq!(scripts.len(), 2);
    }

    #[tokio::test]
    async fn test_load_asset() {
        let temp_dir = TempDir::new().unwrap();
        let assets_dir = temp_dir.path().join("assets");
        std::fs::create_dir(&assets_dir).unwrap();

        std::fs::write(assets_dir.join("template.md"), "# Template").unwrap();

        let content = SkillLoader::load_asset(temp_dir.path(), "template.md").await;
        assert!(content.is_some());
        assert_eq!(content.unwrap(), b"# Template");
    }

    #[tokio::test]
    async fn test_load_asset_not_found() {
        let temp_dir = TempDir::new().unwrap();
        let content = SkillLoader::load_asset(temp_dir.path(), "nonexistent.md").await;
        assert!(content.is_none());
    }

    #[test]
    fn test_has_resources() {
        let temp_dir = TempDir::new().unwrap();

        // No resources
        assert!(!SkillLoader::has_resources(temp_dir.path()));

        // With scripts/
        std::fs::create_dir(temp_dir.path().join("scripts")).unwrap();
        assert!(SkillLoader::has_resources(temp_dir.path()));
    }
}
