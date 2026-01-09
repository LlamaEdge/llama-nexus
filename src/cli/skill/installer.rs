//! Skill installer for downloading and installing skills
//!
//! Supports installing from:
//! - skillsmp.com (skillsmp:name format)
//! - Direct URLs (https://...)

use std::{
    io::{Read, Write},
    path::PathBuf,
};

use super::marketplace::SkillsMarketplace;
use crate::error::{ServerError, ServerResult};

/// Skill source specification
#[derive(Debug, Clone)]
pub enum SkillSource {
    /// From skillsmp.com marketplace
    Skillsmp {
        name: String,
        version: Option<String>,
    },
    /// From a direct URL
    Url(String),
}

impl SkillSource {
    /// Parse a source string into a SkillSource
    ///
    /// Supported formats:
    /// - `skillsmp:name` - Install from skillsmp.com
    /// - `skillsmp:name@version` - Install specific version from skillsmp.com
    /// - `https://...` - Install from direct URL
    pub fn parse(source: &str) -> ServerResult<Self> {
        if let Some(rest) = source.strip_prefix("skillsmp:") {
            if let Some((name, version)) = rest.split_once('@') {
                Ok(SkillSource::Skillsmp {
                    name: name.to_string(),
                    version: Some(version.to_string()),
                })
            } else {
                Ok(SkillSource::Skillsmp {
                    name: rest.to_string(),
                    version: None,
                })
            }
        } else if source.starts_with("https://") || source.starts_with("http://") {
            Ok(SkillSource::Url(source.to_string()))
        } else {
            Err(ServerError::Operation(format!(
                "Invalid skill source: '{}'. Use 'skillsmp:name' or a URL.",
                source
            )))
        }
    }

    /// Get a display name for the source
    pub fn display_name(&self) -> String {
        match self {
            SkillSource::Skillsmp { name, version } => {
                if let Some(v) = version {
                    format!("skillsmp:{}@{}", name, v)
                } else {
                    format!("skillsmp:{}", name)
                }
            }
            SkillSource::Url(url) => url.clone(),
        }
    }
}

/// Skill installer
pub struct SkillInstaller {
    /// Target installation directory
    install_dir: PathBuf,
    /// Marketplace client
    marketplace: SkillsMarketplace,
}

impl SkillInstaller {
    /// Create a new skill installer
    pub fn new(install_dir: PathBuf, skill_config: Option<&crate::config::SkillConfig>) -> Self {
        // Get API key from config or environment
        let api_key = skill_config
            .and_then(|c| c.market.as_ref())
            .and_then(|m| m.api_key.clone())
            .or_else(|| std::env::var("SKILLSMP_API_KEY").ok());

        Self {
            install_dir,
            marketplace: SkillsMarketplace::new(api_key),
        }
    }

    /// Install a skill from the given source
    ///
    /// Returns the installed skill name
    pub async fn install(&self, source: &SkillSource) -> ServerResult<String> {
        use super::lockfile::SkillLockFile;

        // Ensure installation directory exists
        tokio::fs::create_dir_all(&self.install_dir)
            .await
            .map_err(|e| {
                ServerError::Operation(format!(
                    "Failed to create skills directory '{}': {}",
                    self.install_dir.display(),
                    e
                ))
            })?;

        let (skill_name, version) = match source {
            SkillSource::Skillsmp { name, version } => {
                let skill_name = self.install_from_skillsmp(name, version.as_deref()).await?;
                (skill_name, version.clone())
            }
            SkillSource::Url(url) => {
                let skill_name = self.install_from_url(url).await?;
                (skill_name, None)
            }
        };

        // Create skill.lock file for version tracking
        let lock_file = SkillLockFile::new(skill_name.clone(), source.display_name());
        let lock_file = if let Some(ver) = version {
            lock_file.with_version(ver)
        } else {
            lock_file
        };

        let lock_path = self.install_dir.join(&skill_name).join("skill.lock");
        if let Err(e) = lock_file.save(&lock_path).await {
            println!("  Warning: Failed to create skill.lock: {}", e);
        } else {
            println!("  Created skill.lock for version tracking");
        }

        Ok(skill_name)
    }

    /// Install a skill from skillsmp.com
    async fn install_from_skillsmp(
        &self,
        name: &str,
        version: Option<&str>,
    ) -> ServerResult<String> {
        if let Some(ver) = version {
            println!("  Resolving skill ID for '{}@{}'...", name, ver);
        } else {
            println!("  Resolving skill ID for '{}'...", name);
        }

        // Resolve the skill name to an ID (with optional version)
        let skill_id = self.marketplace.resolve_skill_id(name, version).await?;
        println!("  Found: {}", skill_id);

        println!("  Downloading skill package...");

        // Download the skill package
        let package_data = self.marketplace.download_skill(&skill_id).await?;
        println!("  Downloaded {} bytes", package_data.len());

        // Extract the package
        let skill_name = self.extract_package(&package_data, name)?;

        Ok(skill_name)
    }

    /// Install a skill from a direct URL
    async fn install_from_url(&self, url: &str) -> ServerResult<String> {
        println!("  Downloading from URL...");

        let response = reqwest::get(url)
            .await
            .map_err(|e| ServerError::Operation(format!("Failed to download skill: {}", e)))?;

        if !response.status().is_success() {
            return Err(ServerError::Operation(format!(
                "Failed to download skill: HTTP {}",
                response.status()
            )));
        }

        let package_data = response
            .bytes()
            .await
            .map_err(|e| ServerError::Operation(format!("Failed to read skill package: {}", e)))?;

        println!("  Downloaded {} bytes", package_data.len());

        // Try to extract skill name from URL
        let name_hint = url
            .rsplit('/')
            .next()
            .and_then(|s| s.strip_suffix(".zip"))
            .unwrap_or("downloaded-skill");

        let skill_name = self.extract_package(&package_data, name_hint)?;

        Ok(skill_name)
    }

    /// Extract a skill package (zip format) to the installation directory
    fn extract_package(&self, data: &[u8], name_hint: &str) -> ServerResult<String> {
        use std::io::Cursor;

        println!("  Extracting skill package...");

        let reader = Cursor::new(data);
        let mut archive = zip::ZipArchive::new(reader).map_err(|e| {
            ServerError::Operation(format!("Invalid skill package (not a valid zip): {}", e))
        })?;

        // Find the skill name from the archive
        // Usually the root directory name or from SKILL.md
        let mut skill_name: Option<String> = None;
        let mut root_prefix: Option<String> = None;

        // First pass: determine the structure
        for i in 0..archive.len() {
            let file = archive.by_index(i).map_err(|e| {
                ServerError::Operation(format!("Failed to read archive entry: {}", e))
            })?;

            let path = file.name();

            // Check if this is a root directory
            if root_prefix.is_none()
                && let Some(first_component) = path.split('/').next()
                && !first_component.is_empty()
            {
                root_prefix = Some(format!("{}/", first_component));
            }

            // Look for SKILL.md to determine skill name
            if path.ends_with("SKILL.md") || path.ends_with("/SKILL.md") {
                // Extract skill name from the directory containing SKILL.md
                let skill_dir = std::path::Path::new(path).parent();
                if let Some(dir) = skill_dir
                    && let Some(name) = dir.file_name().and_then(|n| n.to_str())
                    && !name.is_empty()
                    && name != "."
                {
                    skill_name = Some(name.to_string());
                }
            }
        }

        // Use name hint if we couldn't determine the skill name
        let final_name = skill_name.unwrap_or_else(|| name_hint.to_string());
        let target_dir = self.install_dir.join(&final_name);

        // Check if skill already exists
        if target_dir.exists() {
            return Err(ServerError::Operation(format!(
                "Skill '{}' already exists at '{}'. Remove it first to reinstall.",
                final_name,
                target_dir.display()
            )));
        }

        // Create target directory
        std::fs::create_dir_all(&target_dir).map_err(|e| {
            ServerError::Operation(format!(
                "Failed to create skill directory '{}': {}",
                target_dir.display(),
                e
            ))
        })?;

        // Second pass: extract files
        for i in 0..archive.len() {
            let mut file = archive.by_index(i).map_err(|e| {
                ServerError::Operation(format!("Failed to read archive entry: {}", e))
            })?;

            let raw_path = file.name().to_string();

            // Strip the root prefix if present
            let relative_path = if let Some(ref prefix) = root_prefix {
                raw_path.strip_prefix(prefix).unwrap_or(&raw_path)
            } else {
                &raw_path
            };

            // Skip empty paths or root directory
            if relative_path.is_empty() || relative_path == "/" {
                continue;
            }

            let target_path = target_dir.join(relative_path);

            if file.is_dir() {
                std::fs::create_dir_all(&target_path).map_err(|e| {
                    ServerError::Operation(format!(
                        "Failed to create directory '{}': {}",
                        target_path.display(),
                        e
                    ))
                })?;
            } else {
                // Ensure parent directory exists
                if let Some(parent) = target_path.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| {
                        ServerError::Operation(format!(
                            "Failed to create directory '{}': {}",
                            parent.display(),
                            e
                        ))
                    })?;
                }

                // Extract file
                let mut outfile = std::fs::File::create(&target_path).map_err(|e| {
                    ServerError::Operation(format!(
                        "Failed to create file '{}': {}",
                        target_path.display(),
                        e
                    ))
                })?;

                let mut buffer = Vec::new();
                file.read_to_end(&mut buffer).map_err(|e| {
                    ServerError::Operation(format!("Failed to read archive content: {}", e))
                })?;

                outfile.write_all(&buffer).map_err(|e| {
                    ServerError::Operation(format!(
                        "Failed to write file '{}': {}",
                        target_path.display(),
                        e
                    ))
                })?;
            }
        }

        println!("  Extracted to: {}", target_dir.display());

        Ok(final_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_skillsmp_source() {
        let source = SkillSource::parse("skillsmp:code-review").unwrap();
        match source {
            SkillSource::Skillsmp { name, version } => {
                assert_eq!(name, "code-review");
                assert!(version.is_none());
            }
            _ => panic!("Expected Skillsmp source"),
        }
    }

    #[test]
    fn test_parse_skillsmp_source_with_version() {
        let source = SkillSource::parse("skillsmp:code-review@2.0.0").unwrap();
        match source {
            SkillSource::Skillsmp { name, version } => {
                assert_eq!(name, "code-review");
                assert_eq!(version, Some("2.0.0".to_string()));
            }
            _ => panic!("Expected Skillsmp source"),
        }
    }

    #[test]
    fn test_parse_url_source() {
        let source = SkillSource::parse("https://example.com/skill.zip").unwrap();
        match source {
            SkillSource::Url(url) => {
                assert_eq!(url, "https://example.com/skill.zip");
            }
            _ => panic!("Expected Url source"),
        }
    }

    #[test]
    fn test_parse_invalid_source() {
        let result = SkillSource::parse("invalid");
        assert!(result.is_err());
    }

    #[test]
    fn test_display_name() {
        let source = SkillSource::Skillsmp {
            name: "test".to_string(),
            version: None,
        };
        assert_eq!(source.display_name(), "skillsmp:test");

        let source = SkillSource::Skillsmp {
            name: "test".to_string(),
            version: Some("1.0.0".to_string()),
        };
        assert_eq!(source.display_name(), "skillsmp:test@1.0.0");
    }
}
