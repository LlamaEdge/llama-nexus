//! Validator implementations for the reflection system.
//!
//! This module provides concrete validator implementations:
//! - Structural validators (JSON, code syntax)
//! - Semantic validators (LLM-driven content evaluation)

// Validators are reserved for future validation features
#![allow(dead_code)]

pub mod semantic;
pub mod structural;

pub use semantic::SemanticValidator;
pub use structural::{CodeValidator, JsonValidator, SupportedLanguage};
