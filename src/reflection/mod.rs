//! Reflection and self-correction system for Plan mode.
//!
//! This module implements the reflection system that evaluates task execution
//! results and provides structured feedback for quality improvement.
//!
//! ## Overview
//!
//! The reflection system consists of four main components:
//!
//! 1. **Reflection Engine**: Evaluates subtask and plan results using LLM
//! 2. **Reflection Types**: Core types for representing reflection results
//! 3. **Reflection Prompts**: Templates for LLM-driven reflection
//! 4. **Validators**: Multi-layer result validation (structural, semantic, constraints)
//!
//! ## Usage
//!
//! ```rust,ignore
//! use crate::reflection::{ReflectionEngine, ReflectionConfig, ReflectionContext};
//!
//! // Create reflection engine
//! let engine = ReflectionEngine::new(server, ReflectionConfig::default());
//!
//! // Create context for reflection
//! let context = ReflectionContext::new("Calculate fibonacci(10)")
//!     .with_iterations(3)
//!     .with_tool_calls(vec!["calculate".to_string()]);
//!
//! // Reflect on subtask result
//! let result = engine.reflect_on_subtask(&subtask, "55", &trace, &context).await?;
//!
//! if !result.passed {
//!     // Handle issues
//!     for issue in &result.issues {
//!         println!("Issue: {} (severity: {})", issue.description, issue.severity);
//!     }
//! }
//! ```
//!
//! ## Validation
//!
//! ```rust,ignore
//! use crate::reflection::validator::{ValidationContext, Constraint};
//! use crate::reflection::validators::{JsonValidator, CodeValidator, SupportedLanguage};
//!
//! // JSON validation
//! let json_validator = JsonValidator::new();
//! let result = json_validator.validate(r#"{"key": "value"}"#, &context).await;
//!
//! // Code validation
//! let code_validator = CodeValidator::new(SupportedLanguage::Rust);
//! let result = code_validator.validate("fn main() {}", &context).await;
//! ```

// Allow dead code - will be fully integrated when Plan mode calls reflection
#![allow(dead_code)]

pub mod engine;
pub mod prompts;
pub mod types;
pub mod validator;
pub mod validators;

// Re-exports will be used when integrated into Plan mode
#[allow(unused_imports)]
pub use engine::{LlmServerInfo, ReflectionEngine};
#[allow(unused_imports)]
pub use types::{
    IssueType, RecommendedAction, ReflectionConfig, ReflectionContext, ReflectionIssue,
    ReflectionResult, ReplanRequest,
};
#[allow(unused_imports)]
pub use validator::{
    Constraint, ExpectedFormat, ResultValidator, ValidationContext, ValidationError,
    ValidationResult, ValidationWarning,
};
#[allow(unused_imports)]
pub use validators::{CodeValidator, JsonValidator, SemanticValidator, SupportedLanguage};
