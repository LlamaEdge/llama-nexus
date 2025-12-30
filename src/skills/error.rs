//! Error types for Skills module

use thiserror::Error;

/// Errors that can occur when working with skills
#[derive(Error, Debug)]
pub enum SkillError {
    /// Invalid skill name format
    #[error("Invalid skill name '{name}': {reason}")]
    InvalidName { name: String, reason: String },

    /// Invalid description
    #[error("Invalid description: {0}")]
    InvalidDescription(String),

    /// Invalid compatibility field
    #[error("Invalid compatibility: {0}")]
    InvalidCompatibility(String),

    /// SKILL.md parsing error
    #[error("Failed to parse SKILL.md: {0}")]
    ParseError(String),

    /// YAML front matter error
    #[error("Invalid YAML front matter: {0}")]
    YamlError(String),

    /// File I/O error
    #[error("File error: {0}")]
    FileError(String),

    /// Skill not found
    #[error("Skill not found: {0}")]
    NotFound(String),

    /// Name doesn't match directory
    #[error("Skill name '{name}' must match directory name '{directory}'")]
    NameDirectoryMismatch { name: String, directory: String },

    /// Registry not initialized
    #[error("Skills registry not initialized")]
    RegistryNotInitialized,
}

impl From<std::io::Error> for SkillError {
    fn from(err: std::io::Error) -> Self {
        SkillError::FileError(err.to_string())
    }
}

impl From<serde_yaml::Error> for SkillError {
    fn from(err: serde_yaml::Error) -> Self {
        SkillError::YamlError(err.to_string())
    }
}

/// Result type for skill operations
pub type SkillResult<T> = Result<T, SkillError>;
