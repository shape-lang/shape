//! Backend executor trait for parity testing
//!
//! Provides a unified interface for executing code across different backends
//! (Interpreter, VM, JIT) to enable three-way parity testing.

use super::FeatureTest;
use super::parity::ExecutionResult;
use shape_runtime::engine::{ProgramExecutor, ShapeEngine};

/// Trait for executing Shape code across different backends
pub trait BackendExecutor: Send + Sync {
    /// Human-readable name of this backend
    fn name(&self) -> &'static str;

    /// Execute a feature test and return the result
    fn execute(&self, test: &FeatureTest) -> ExecutionResult;

    /// Check if this backend is available/enabled
    fn is_available(&self) -> bool;

    /// Get list of features this backend doesn't support yet
    fn unsupported_features(&self) -> &'static [&'static str] {
        &[]
    }
}

/// Helper to run async code in a blocking context
fn run_async<F, T>(future: F) -> T
where
    F: std::future::Future<Output = T>,
{
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}

/// Execute code with an executor
fn execute_with_executor<E: ProgramExecutor>(executor: &mut E, test: &FeatureTest) -> ExecutionResult {
    let mut engine = match ShapeEngine::new() {
        Ok(e) => e,
        Err(e) => return ExecutionResult::Error(format!("Engine init failed: {}", e)),
    };

    if let Err(e) = engine.load_stdlib() {
        return ExecutionResult::Error(format!("Stdlib load failed: {}", e));
    }

    run_async(async {
        // Execute the test code
        match engine.execute_repl(executor, test.code).await {
            Ok(result) => {
                if !test.function.is_empty() && test.function != "main" {
                    // Call the specific function
                    let call = format!("{}()", test.function);
                    match engine.execute_repl(executor, &call).await {
                        Ok(r) => ExecutionResult::Success(format!("{:?}", r.value)),
                        Err(e) => ExecutionResult::Error(format!("{}", e)),
                    }
                } else {
                    ExecutionResult::Success(format!("{:?}", result.value))
                }
            }
            Err(e) => ExecutionResult::Error(format!("{}", e)),
        }
    })
}

/// Legacy interpreter backend - now uses VM internally
/// Kept for parity testing infrastructure compatibility
pub struct InterpreterBackend;

impl BackendExecutor for InterpreterBackend {
    fn name(&self) -> &'static str {
        "Interpreter (VM)"
    }

    fn execute(&self, test: &FeatureTest) -> ExecutionResult {
        // Interpreter is retired - use VM for all execution
        execute_with_executor(&mut crate::BytecodeExecutor::new(), test)
    }

    fn is_available(&self) -> bool {
        true
    }
}

/// Bytecode VM backend
pub struct VMBackend;

impl BackendExecutor for VMBackend {
    fn name(&self) -> &'static str {
        "VM"
    }

    fn execute(&self, test: &FeatureTest) -> ExecutionResult {
        execute_with_executor(&mut crate::BytecodeExecutor::new(), test)
    }

    fn is_available(&self) -> bool {
        true
    }
}

/// JIT backend - works for math operations, returns NotSupported for others
pub struct JITBackend;

impl BackendExecutor for JITBackend {
    fn name(&self) -> &'static str {
        "JIT"
    }

    fn execute(&self, test: &FeatureTest) -> ExecutionResult {
        // Check if the test uses features JIT doesn't support
        for unsupported in self.unsupported_features() {
            if test.covers.contains(unsupported) {
                return ExecutionResult::NotSupported(unsupported);
            }
        }

        // JIT execution via the VM with JIT compilation
        // For now, delegate to VM since JIT is integrated there
        execute_with_executor(&mut crate::BytecodeExecutor::new(), test)
    }

    fn is_available(&self) -> bool {
        true // JIT is available
    }

    fn unsupported_features(&self) -> &'static [&'static str] {
        // Features that JIT doesn't support yet
        &["pattern_def", "stream_handler", "async_block", "try_catch"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_interpreter_available() {
        let backend = InterpreterBackend;
        assert!(backend.is_available());
        assert_eq!(backend.name(), "Interpreter (VM)");
    }

    #[test]
    fn test_vm_available() {
        let backend = VMBackend;
        assert!(backend.is_available());
        assert_eq!(backend.name(), "VM");
    }

    #[test]
    fn test_jit_available() {
        let backend = JITBackend;
        assert!(backend.is_available());
        assert_eq!(backend.name(), "JIT");
    }

    #[test]
    fn test_jit_unsupported_features() {
        let backend = JITBackend;
        let unsupported = backend.unsupported_features();
        assert!(unsupported.contains(&"pattern_def"));
        assert!(unsupported.contains(&"stream_handler"));
    }
}
