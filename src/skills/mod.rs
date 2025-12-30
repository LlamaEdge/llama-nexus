//! Skills module for Plan Mode
//!
//! This module implements Agent Skills support following the
//! [Agent Skills Standard](https://agentskills.io/specification).
//!
//! Skills are prompt-driven extensions that enhance LLM capabilities
//! for specific tasks through two-stage loading:
//! 1. Discovery: Load skill summaries (name + description)
//! 2. Activation: Load full SKILL.md content when selected

pub mod detector;
pub mod error;
pub mod injector;
pub mod loader;
pub mod parser;
pub mod registry;
pub mod types;
pub mod validator;

pub use detector::SkillDetector;
pub use error::SkillError;
pub use injector::SkillInjector;
pub use loader::SkillLoader;
pub use parser::SkillParser;
pub use registry::{SKILLS_REGISTRY, SkillRegistry};
pub use types::{LoadedSkill, ScriptInfo, SkillMetadata, SkillSummary};
pub use validator::validate_skill_name;
