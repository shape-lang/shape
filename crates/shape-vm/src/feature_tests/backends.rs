//! Backend executor trait for legacy feature-test execution
//!
//! This in-process harness only has access to the VM-side executor. Real
//! VM-vs-JIT evidence is collected by the subprocess differential gate named
//! in `REAL_JIT_PARITY_GATE`; do not treat this module as JIT coverage.

use super::FeatureTest;
use super::parity::ExecutionResult;
use shape_runtime::engine::{ProgramExecutor, ShapeEngine};

/// Curated subprocess gate that runs real `shape run --mode jit` comparisons.
pub const REAL_JIT_PARITY_GATE: &str = "scripts/differential-gate.sh";

/// CI workflow that invokes `REAL_JIT_PARITY_GATE` on every push/PR.
pub const REAL_JIT_PARITY_CI: &str = ".github/workflows/ci.yml";

const JIT_FEATURE_TEST_LANE_RETIRED: &str =
    "feature-test JIT lane retired; use scripts/differential-gate.sh";

/// Trait for executing Shape code across different backends
pub trait BackendExecutor: Send + Sync {
    /// Human-readable name of this backend
    fn name(&self) -> &'static str;

    /// Execute a feature test and return the result
    fn execute(&self, test: &FeatureTest) -> ExecutionResult;

    /// Check if this backend is available/enabled
    fn is_available(&self) -> bool;

    /// Reason reported when this backend is intentionally unavailable.
    fn unavailable_reason(&self) -> &'static str {
        "backend not available"
    }

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
fn execute_with_executor<E: ProgramExecutor>(
    executor: &mut E,
    test: &FeatureTest,
) -> ExecutionResult {
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

/// Retired feature-test JIT lane.
///
/// `shape-vm` cannot instantiate the real `shape_jit::JITExecutor` without
/// introducing a crate cycle (`shape-jit` depends on `shape-vm`). The previous
/// implementation delegated to `BytecodeExecutor`, which made this harness look
/// like JIT parity evidence while exercising only the VM. Keep the type as a
/// compatibility marker, but make every use skip closed and point to the real
/// subprocess VM-vs-JIT gate.
pub struct JITBackend;

impl BackendExecutor for JITBackend {
    fn name(&self) -> &'static str {
        "JIT (external differential gate)"
    }

    fn execute(&self, _test: &FeatureTest) -> ExecutionResult {
        ExecutionResult::Skipped(JIT_FEATURE_TEST_LANE_RETIRED)
    }

    fn is_available(&self) -> bool {
        false
    }

    fn unavailable_reason(&self) -> &'static str {
        JIT_FEATURE_TEST_LANE_RETIRED
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
        assert!(!backend.is_available());
        assert_eq!(backend.name(), "JIT (external differential gate)");
    }

    #[test]
    fn test_jit_backend_skips_to_real_gate() {
        let backend = JITBackend;
        let test = FeatureTest {
            name: "jit_marker",
            covers: &[],
            code: "1 + 1",
            function: "",
            category: super::super::FeatureCategory::Operator,
            requires_data: false,
        };

        assert_eq!(
            backend.unavailable_reason(),
            "feature-test JIT lane retired; use scripts/differential-gate.sh"
        );
        assert!(matches!(
            backend.execute(&test),
            ExecutionResult::Skipped(_)
        ));
    }
}
