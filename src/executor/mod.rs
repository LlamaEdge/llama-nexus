//! Script Executor Module
//!
//! Provides sandboxed execution environments for Skills scripts.
//! Supports multiple runtimes: WasmEdge, Docker, and Deno.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                  ScriptExecutorManager                       │
//! │  ┌─────────────┬─────────────┬─────────────┐                │
//! │  │ WasmEdge    │   Docker    │    Deno     │  ... more      │
//! │  │ Executor    │  Executor   │  Executor   │  executors     │
//! │  └─────────────┴─────────────┴─────────────┘                │
//! │         │              │             │                       │
//! │         ▼              ▼             ▼                       │
//! │     .wasm          .py/.sh      .js/.ts                     │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Usage
//!
//! ```rust,ignore
//! use executor::{ScriptExecutorManager, ResourceLimits};
//!
//! // Create manager with default limits
//! let mut manager = ScriptExecutorManager::new(ResourceLimits::default());
//!
//! // Register executors
//! manager.register(Box::new(WasmEdgeExecutor::new(config)?));
//! manager.register(Box::new(DockerExecutor::new(config)?));
//! manager.register(Box::new(DenoExecutor::new(config)?));
//!
//! // Execute a script (executor auto-selected by extension)
//! let output = manager.execute(&script, args, env, None).await?;
//! ```

mod error;
mod manager;
mod traits;
mod types;

// Executor implementations
pub mod docker;

// Future executor implementations (feature-gated)
// #[cfg(feature = "executor-wasmedge")]
// mod wasmedge;
// #[cfg(feature = "executor-deno")]
// mod deno;

pub use docker::{DockerConfig, DockerExecutor};
pub use error::{ExecutionError, ExecutionResult};
pub use manager::ScriptExecutorManager;
pub use traits::{Executor, IsolationLevel};
pub use types::{ExecuteRequest, FilesystemPolicy, ResourceLimits, ResourceUsage, ScriptOutput};
