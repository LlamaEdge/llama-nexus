//! Reflection and self-correction system for Plan mode.
//!
//! This module implements the reflection system that evaluates task execution
//! results and provides structured feedback for quality improvement.
//!
//! ## Overview
//!
//! The reflection system consists of the following main components:
//!
//! 1. **Reflection Engine**: Evaluates subtask and plan results using LLM
//! 2. **Reflection Types**: Core types for representing reflection results
//! 3. **Reflection Prompts**: Templates for LLM-driven reflection
//! 4. **Validators**: Multi-layer result validation (structural, semantic, constraints)
//! 5. **Dynamic Replanner**: Automatic plan revision when failures occur
//! 6. **Reflection Cache**: Caching for similar task reflections
//! 7. **Adaptive Strategy**: Learning-based reflection parameter adjustment
//! 8. **Reflection Reports**: Structured reports for API responses
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

pub mod cache;
pub mod engine;
pub mod prompts;
pub mod replanner;
pub mod report;
pub mod strategy;
pub mod types;
pub mod validator;
pub mod validators;

// Re-exports for Plan mode integration
// Core reflection types (currently used in plan.rs)
pub use cache::{CacheConfig, ReflectionCache};
// Additional exports for future use and external API
#[allow(unused_imports)]
pub use cache::{CacheEntry, CacheKey, CacheStats};
pub use engine::{LlmServerInfo, ReflectionEngine};
pub use replanner::{
    DependencyGraph, DynamicReplanner, FailedSubtaskInfo, ReplanContext, ReplanTrigger, SubtaskInfo,
};
#[allow(unused_imports)]
pub use replanner::{NewSubtask, PlanDiff, ReplanConfig, ReplanResult, TimeBudget};
#[allow(unused_imports)]
pub use report::{
    ActionReport, IssueReport, PlanChangeSummary, PlanReflectionReport, ReflectionReport,
    ReflectionSummary, ReplanSummary, ReportMetadata, SubtaskReflectionReport, ValidationReport,
};
pub use strategy::AdaptiveStrategy;
#[allow(unused_imports)]
pub use strategy::{AdaptedParams, AdaptiveConfig, CategoryStats, ReflectionStats, TaskCategory};
#[allow(unused_imports)]
pub use types::{IssueType, ReflectionIssue, ReflectionResult, ReplanRequest};
pub use types::{RecommendedAction, ReflectionConfig, ReflectionContext};
#[allow(unused_imports)]
pub use validator::{
    Constraint, ExpectedFormat, ResultValidator, ValidationContext, ValidationError,
    ValidationResult, ValidationWarning,
};
#[allow(unused_imports)]
pub use validators::{CodeValidator, JsonValidator, SemanticValidator, SupportedLanguage};
