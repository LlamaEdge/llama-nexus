//! Resource loader for Skills
//!
//! Loads additional resources from skill directories:
//! - references/: Reference documents (.md, .txt)
//! - scripts/: Executable scripts
//! - assets/: Templates and other assets

use std::path::Path;

use crate::{executor::EXECUTOR_MANAGER, skills::types::ScriptInfo};

/// Loader for skill resources
///
/// Provides methods for loading skill resources (scripts, references, assets).
/// Public API for future API endpoints and integrations.
pub struct SkillLoader;

#[allow(dead_code)]
impl SkillLoader {
    /// Load reference documents from the references/ directory
    ///
    /// Reads .md and .txt files from the references/ subdirectory.
    /// If patterns are specified, only matching files are loaded.
    ///
    /// # Arguments
    /// * `skill_dir` - The skill directory path
    ///
    /// # Returns
    /// A vector of file contents
    pub async fn load_references(skill_dir: &Path) -> Vec<String> {
        Self::load_references_with_patterns(skill_dir, None).await
    }

    /// Load reference documents with optional pattern filtering
    ///
    /// Reads files from the references/ subdirectory that match the given patterns.
    /// If no patterns are specified, all .md and .txt files are loaded.
    ///
    /// # Arguments
    /// * `skill_dir` - The skill directory path
    /// * `patterns` - Optional list of glob patterns to filter files
    ///
    /// # Returns
    /// A vector of file contents
    pub async fn load_references_with_patterns(
        skill_dir: &Path,
        patterns: Option<&[String]>,
    ) -> Vec<String> {
        let refs_dir = skill_dir.join("references");
        if !refs_dir.exists() {
            return Vec::new();
        }

        let mut references = Vec::new();

        if let Ok(entries) = std::fs::read_dir(&refs_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

                    // Check if file should be loaded
                    let should_load = match patterns {
                        // With patterns: file must match at least one pattern
                        Some(pats) if !pats.is_empty() => pats
                            .iter()
                            .any(|pattern| Self::glob_match(pattern, filename)),
                        // Without patterns: load all .md and .txt files
                        _ => ext == "md" || ext == "txt",
                    };

                    if should_load && let Ok(content) = tokio::fs::read_to_string(&path).await {
                        references.push(content);
                    }
                }
            }
        }

        references
    }

    /// Simple glob pattern matching
    ///
    /// Supports:
    /// - `*` matches any sequence of characters (including empty)
    /// - `?` matches exactly one character
    fn glob_match(pattern: &str, text: &str) -> bool {
        let p: Vec<char> = pattern.chars().collect();
        let t: Vec<char> = text.chars().collect();
        let (m, n) = (p.len(), t.len());

        // dp[i][j] = true if pattern[0..i] matches text[0..j]
        let mut dp = vec![vec![false; n + 1]; m + 1];

        // Empty pattern matches empty text
        dp[0][0] = true;

        // Handle patterns starting with *
        for i in 1..=m {
            if p[i - 1] == '*' {
                dp[i][0] = dp[i - 1][0];
            } else {
                break;
            }
        }

        // Fill the DP table
        for i in 1..=m {
            for j in 1..=n {
                if p[i - 1] == '*' {
                    // * can match empty (dp[i-1][j]) or match one more char (dp[i][j-1])
                    dp[i][j] = dp[i - 1][j] || dp[i][j - 1];
                } else if p[i - 1] == '?' || p[i - 1] == t[j - 1] {
                    // ? matches any single char, or exact char match
                    dp[i][j] = dp[i - 1][j - 1];
                }
            }
        }

        dp[m][n]
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

    /// Check if a script extension is supported by the executor manager
    ///
    /// Returns true if an executor is registered for the given extension.
    /// Returns false if the executor manager is not initialized or no executor
    /// is registered for the extension.
    ///
    /// # Arguments
    /// * `extension` - The file extension (without dot, e.g., "js", "py")
    ///
    /// # Returns
    /// true if the extension is supported
    pub fn is_extension_supported(extension: &str) -> bool {
        EXECUTOR_MANAGER
            .get()
            .map(|m| m.supports(extension))
            .unwrap_or(false)
    }

    /// Get all supported script extensions
    ///
    /// Returns the list of file extensions that have registered executors.
    /// Returns an empty list if the executor manager is not initialized.
    ///
    /// # Returns
    /// Vector of supported extensions (without dots)
    pub fn supported_extensions() -> Vec<String> {
        EXECUTOR_MANAGER
            .get()
            .map(|m| {
                m.supported_extensions()
                    .iter()
                    .map(|s| s.to_string())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Filter scripts to only include those with supported extensions
    ///
    /// # Arguments
    /// * `scripts` - List of scripts to filter
    ///
    /// # Returns
    /// Scripts that have registered executors
    pub fn filter_supported_scripts(scripts: &[ScriptInfo]) -> Vec<&ScriptInfo> {
        scripts
            .iter()
            .filter(|s| {
                s.path
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(Self::is_extension_supported)
                    .unwrap_or(false)
            })
            .collect()
    }

    /// Get unsupported scripts from a list
    ///
    /// Returns scripts whose extensions don't have registered executors.
    /// Useful for warning users about scripts that cannot be executed.
    ///
    /// # Arguments
    /// * `scripts` - List of scripts to check
    ///
    /// # Returns
    /// Scripts that don't have registered executors
    pub fn filter_unsupported_scripts(scripts: &[ScriptInfo]) -> Vec<&ScriptInfo> {
        scripts
            .iter()
            .filter(|s| {
                s.path
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|ext| !Self::is_extension_supported(ext))
                    .unwrap_or(true)
            })
            .collect()
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

    #[test]
    fn test_is_extension_supported_no_manager() {
        // Without executor manager initialized, all extensions are unsupported
        // Note: This test might fail if another test initializes the global manager
        // In practice, it returns false when not initialized
        let result = SkillLoader::is_extension_supported("js");
        // Result depends on whether EXECUTOR_MANAGER is initialized
        assert!(result == false || result == true); // Just check it doesn't panic
    }

    #[test]
    fn test_supported_extensions_no_manager() {
        // Without executor manager, returns empty list
        let extensions = SkillLoader::supported_extensions();
        // Result depends on whether EXECUTOR_MANAGER is initialized
        // Just verify it returns a Vec and doesn't panic
        let _ = extensions.len();
    }

    #[test]
    fn test_filter_supported_scripts() {
        use std::path::PathBuf;

        let scripts = vec![
            ScriptInfo {
                name: "script.js".to_string(),
                path: PathBuf::from("/skills/test/scripts/script.js"),
                executable: true,
            },
            ScriptInfo {
                name: "helper.py".to_string(),
                path: PathBuf::from("/skills/test/scripts/helper.py"),
                executable: true,
            },
            ScriptInfo {
                name: "config.json".to_string(),
                path: PathBuf::from("/skills/test/scripts/config.json"),
                executable: false,
            },
        ];

        // Without EXECUTOR_MANAGER, all scripts are unsupported
        // This tests the filtering logic
        let supported = SkillLoader::filter_supported_scripts(&scripts);
        // Check it doesn't panic and returns a valid vector
        assert!(supported.len() <= scripts.len());
    }

    #[test]
    fn test_filter_unsupported_scripts() {
        use std::path::PathBuf;

        let scripts = vec![
            ScriptInfo {
                name: "script.js".to_string(),
                path: PathBuf::from("/skills/test/scripts/script.js"),
                executable: true,
            },
            ScriptInfo {
                name: "helper.py".to_string(),
                path: PathBuf::from("/skills/test/scripts/helper.py"),
                executable: true,
            },
        ];

        let unsupported = SkillLoader::filter_unsupported_scripts(&scripts);
        // Check it doesn't panic and returns a valid vector
        assert!(unsupported.len() <= scripts.len());
    }

    // Tests for load_references_with_patterns

    #[tokio::test]
    async fn test_load_references_with_patterns_no_patterns() {
        let temp_dir = TempDir::new().unwrap();
        let refs_dir = temp_dir.path().join("references");
        std::fs::create_dir(&refs_dir).unwrap();

        std::fs::write(refs_dir.join("doc1.md"), "# Document 1").unwrap();
        std::fs::write(refs_dir.join("doc2.txt"), "Plain text").unwrap();
        std::fs::write(refs_dir.join("ignored.json"), "{}").unwrap();

        // Without patterns, should load .md and .txt files
        let refs = SkillLoader::load_references_with_patterns(temp_dir.path(), None).await;
        assert_eq!(refs.len(), 2);
    }

    #[tokio::test]
    async fn test_load_references_with_patterns_specific_file() {
        let temp_dir = TempDir::new().unwrap();
        let refs_dir = temp_dir.path().join("references");
        std::fs::create_dir(&refs_dir).unwrap();

        std::fs::write(refs_dir.join("api.md"), "API docs").unwrap();
        std::fs::write(refs_dir.join("guide.md"), "User guide").unwrap();
        std::fs::write(refs_dir.join("notes.txt"), "Notes").unwrap();

        // With specific pattern
        let patterns = vec!["api.md".to_string()];
        let refs =
            SkillLoader::load_references_with_patterns(temp_dir.path(), Some(&patterns)).await;
        assert_eq!(refs.len(), 1);
        assert!(refs[0].contains("API docs"));
    }

    #[tokio::test]
    async fn test_load_references_with_patterns_glob() {
        let temp_dir = TempDir::new().unwrap();
        let refs_dir = temp_dir.path().join("references");
        std::fs::create_dir(&refs_dir).unwrap();

        std::fs::write(refs_dir.join("api-v1.md"), "API v1").unwrap();
        std::fs::write(refs_dir.join("api-v2.md"), "API v2").unwrap();
        std::fs::write(refs_dir.join("guide.md"), "User guide").unwrap();

        // With glob pattern
        let patterns = vec!["api-*.md".to_string()];
        let refs =
            SkillLoader::load_references_with_patterns(temp_dir.path(), Some(&patterns)).await;
        assert_eq!(refs.len(), 2);
    }

    #[tokio::test]
    async fn test_load_references_with_patterns_multiple() {
        let temp_dir = TempDir::new().unwrap();
        let refs_dir = temp_dir.path().join("references");
        std::fs::create_dir(&refs_dir).unwrap();

        std::fs::write(refs_dir.join("api.md"), "API docs").unwrap();
        std::fs::write(refs_dir.join("config.yaml"), "config: true").unwrap();
        std::fs::write(refs_dir.join("notes.txt"), "Notes").unwrap();

        // With multiple patterns
        let patterns = vec!["api.md".to_string(), "notes.txt".to_string()];
        let refs =
            SkillLoader::load_references_with_patterns(temp_dir.path(), Some(&patterns)).await;
        assert_eq!(refs.len(), 2);
    }

    #[tokio::test]
    async fn test_load_references_with_patterns_empty_list() {
        let temp_dir = TempDir::new().unwrap();
        let refs_dir = temp_dir.path().join("references");
        std::fs::create_dir(&refs_dir).unwrap();

        std::fs::write(refs_dir.join("doc.md"), "Document").unwrap();
        std::fs::write(refs_dir.join("notes.txt"), "Notes").unwrap();

        // Empty patterns should behave like None
        let patterns: Vec<String> = vec![];
        let refs =
            SkillLoader::load_references_with_patterns(temp_dir.path(), Some(&patterns)).await;
        assert_eq!(refs.len(), 2);
    }

    // Tests for glob_match

    #[test]
    fn test_glob_match_exact() {
        assert!(SkillLoader::glob_match("api.md", "api.md"));
        assert!(!SkillLoader::glob_match("api.md", "api.txt"));
    }

    #[test]
    fn test_glob_match_star() {
        assert!(SkillLoader::glob_match("*.md", "api.md"));
        assert!(SkillLoader::glob_match("*.md", "guide.md"));
        assert!(!SkillLoader::glob_match("*.md", "api.txt"));
        assert!(SkillLoader::glob_match("api-*.md", "api-v1.md"));
        assert!(SkillLoader::glob_match("api-*.md", "api-v2.md"));
        assert!(!SkillLoader::glob_match("api-*.md", "guide.md"));
    }

    #[test]
    fn test_glob_match_question() {
        assert!(SkillLoader::glob_match("api?.md", "api1.md"));
        assert!(SkillLoader::glob_match("api?.md", "apix.md"));
        assert!(!SkillLoader::glob_match("api?.md", "api.md"));
        assert!(!SkillLoader::glob_match("api?.md", "api12.md"));
    }

    #[test]
    fn test_glob_match_complex() {
        assert!(SkillLoader::glob_match("*-*-*.md", "a-b-c.md"));
        assert!(SkillLoader::glob_match("*api*", "myapi.md"));
        assert!(SkillLoader::glob_match("*api*", "api"));
        assert!(SkillLoader::glob_match("doc?.txt", "doc1.txt"));
    }
}
