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

#[cfg(test)]
mod e2e_tests;

pub use detector::SkillDetector;
#[allow(unused_imports)]
pub use error::SkillError;
pub use injector::SkillInjector;
#[allow(unused_imports)]
pub use loader::SkillLoader;
#[allow(unused_imports)]
pub use parser::SkillParser;
#[allow(unused_imports)]
pub use registry::SKILLS_REGISTRY;
pub use registry::SkillRegistry;
pub use types::{LoadedSkill, SkillSummary};
#[allow(unused_imports)]
pub use types::{ScriptInfo, SkillMetadata};
#[allow(unused_imports)]
pub use validator::validate_skill_name;
