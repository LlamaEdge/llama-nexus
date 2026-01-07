//! Executor trait definition
//!
//! All script executors (WasmEdge, Docker, Deno, etc.) must implement this trait.

use async_trait::async_trait;

use super::{
    error::ExecutionError,
    types::{ExecuteRequest, ScriptOutput},
};
use crate::skills::types::ScriptInfo;

/// Script executor trait
///
/// Defines the interface for all script execution backends.
/// New executor types can be added by implementing this trait.
///
/// # Example
///
/// ```rust,ignore
/// pub struct MyExecutor { /* ... */ }
///
/// #[async_trait]
/// impl Executor for MyExecutor {
///     fn name(&self) -> &str { "my-executor" }
///
///     fn supported_extensions(&self) -> Vec<&str> {
///         vec!["myext"]
///     }
///
///     async fn execute(&self, request: ExecuteRequest) -> Result<ScriptOutput, ExecutionError> {
///         // Execute the script
///         todo!()
///     }
/// }
/// ```
#[async_trait]
pub trait Executor: Send + Sync {
    /// Returns the executor name (used for logging and configuration)
    fn name(&self) -> &str;

    /// Returns the list of supported file extensions
    ///
    /// Extensions should be lowercase without the leading dot.
    /// Example: `vec!["py", "sh"]` for Python and Shell scripts.
    fn supported_extensions(&self) -> Vec<&str>;

    /// Checks if this executor supports the given script
    ///
    /// Default implementation checks the file extension.
    /// Override for more complex matching logic.
    #[allow(dead_code)]
    fn supports(&self, script: &ScriptInfo) -> bool {
        let ext = script
            .path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        self.supported_extensions()
            .iter()
            .any(|e| e.to_lowercase() == ext)
    }

    /// Executes a script and returns the output
    ///
    /// # Arguments
    ///
    /// * `request` - The execution request containing script, args, env, and limits
    ///
    /// # Returns
    ///
    /// * `Ok(ScriptOutput)` - The script output (stdout, stderr, exit code, etc.)
    /// * `Err(ExecutionError)` - If execution fails
    async fn execute(&self, request: ExecuteRequest) -> Result<ScriptOutput, ExecutionError>;

    /// Performs a health check on the executor
    ///
    /// Default implementation returns Ok(()).
    /// Override to check if the runtime is available (e.g., Docker daemon running).
    async fn health_check(&self) -> Result<(), ExecutionError> {
        Ok(())
    }

    /// Returns the isolation level of this executor
    ///
    /// Used for informational purposes and executor selection.
    #[allow(dead_code)]
    fn isolation_level(&self) -> IsolationLevel {
        IsolationLevel::Runtime
    }
}

/// Isolation level of an executor
///
/// Note: Some variants are used in tests and executor implementations
/// but clippy reports them as dead code because test usage is ignored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[allow(dead_code)]
pub enum IsolationLevel {
    /// No isolation (direct process execution)
    None,
    /// Runtime-level isolation (Deno permissions, WASM sandbox)
    Runtime,
    /// OS-level isolation (Docker containers, namespaces)
    Container,
    /// VM-level isolation (Firecracker, Hyperlight)
    VirtualMachine,
}

impl std::fmt::Display for IsolationLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IsolationLevel::None => write!(f, "none"),
            IsolationLevel::Runtime => write!(f, "runtime"),
            IsolationLevel::Container => write!(f, "container"),
            IsolationLevel::VirtualMachine => write!(f, "vm"),
        }
    }
}
