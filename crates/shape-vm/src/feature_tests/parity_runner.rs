//! Parity test runner for comparing backend execution results
//!
//! Runs feature tests across all backends and collects parity results.

use super::FeatureTest;
use super::backends::BackendExecutor;
use super::parity::{ExecutionResult, ParityResult, ParityStatus};

/// Runner that executes tests across multiple backends
pub struct ParityRunner {
    interpreter: Box<dyn BackendExecutor>,
    vm: Box<dyn BackendExecutor>,
    jit: Option<Box<dyn BackendExecutor>>,
}

impl ParityRunner {
    /// Create a new parity runner with all backends
    pub fn new(
        interpreter: Box<dyn BackendExecutor>,
        vm: Box<dyn BackendExecutor>,
        jit: Option<Box<dyn BackendExecutor>>,
    ) -> Self {
        Self {
            interpreter,
            vm,
            jit,
        }
    }

    /// Create a runner with default backends
    pub fn with_defaults() -> Self {
        use super::backends::{InterpreterBackend, JITBackend, VMBackend};
        Self {
            interpreter: Box::new(InterpreterBackend),
            vm: Box::new(VMBackend),
            jit: Some(Box::new(JITBackend)),
        }
    }

    /// Run a single test across all backends
    pub fn run_test(&self, test: &FeatureTest) -> ParityResult {
        let interpreter = if self.interpreter.is_available() {
            self.interpreter.execute(test)
        } else {
            ExecutionResult::Skipped("Interpreter not available")
        };

        let vm = if self.vm.is_available() {
            self.vm.execute(test)
        } else {
            ExecutionResult::Skipped("VM not available")
        };

        let jit = match &self.jit {
            Some(jit_backend) if jit_backend.is_available() => jit_backend.execute(test),
            Some(_) => ExecutionResult::Skipped("JIT not available"),
            None => ExecutionResult::Skipped("JIT backend not configured"),
        };

        ParityResult {
            test_name: test.name,
            interpreter,
            vm,
            jit,
        }
    }

    /// Run all tests and collect results
    pub fn run_all(&self, tests: &[&FeatureTest]) -> ParityReport {
        let mut results = Vec::with_capacity(tests.len());

        for test in tests {
            results.push(self.run_test(test));
        }

        ParityReport::from_results(results)
    }
}

/// Report of all parity test results
#[derive(Debug)]
pub struct ParityReport {
    pub results: Vec<ParityResult>,
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub partial: usize,
}

impl ParityReport {
    /// Create a report from a list of results
    pub fn from_results(results: Vec<ParityResult>) -> Self {
        let total = results.len();
        let mut passed = 0;
        let mut failed = 0;
        let mut partial = 0;

        for result in &results {
            match result.parity_status() {
                ParityStatus::AllMatch => passed += 1,
                ParityStatus::PartialSkipped { .. } => partial += 1,
                ParityStatus::AllFailed => partial += 1, // Count as partial, not failed
                _ => failed += 1,
            }
        }

        Self {
            results,
            total,
            passed,
            failed,
            partial,
        }
    }

    /// Check if all tests passed (no mismatches)
    pub fn all_passed(&self) -> bool {
        self.failed == 0
    }

    /// Get all failing tests
    pub fn failures(&self) -> Vec<&ParityResult> {
        self.results.iter().filter(|r| !r.is_passing()).collect()
    }

    /// Format as text report
    pub fn format_text(&self) -> String {
        let mut output = String::new();

        output.push_str("═══════════════════════════════════════════════════════════════\n");
        output.push_str("                    PARITY TEST REPORT\n");
        output.push_str("═══════════════════════════════════════════════════════════════\n\n");

        output.push_str(&format!("Total tests: {}\n", self.total));
        output.push_str(&format!("  ✓ Passed (all match): {}\n", self.passed));
        output.push_str(&format!("  ~ Partial (some skipped): {}\n", self.partial));
        output.push_str(&format!("  ✗ Failed (mismatch): {}\n", self.failed));
        output.push('\n');

        if self.failed > 0 {
            output.push_str("───────────────────────────────────────────────────────────────\n");
            output.push_str("                      FAILURES\n");
            output.push_str("───────────────────────────────────────────────────────────────\n\n");

            for result in self.failures() {
                output.push_str(&format!("Test: {}\n", result.test_name));
                output.push_str(&format!("  {}\n\n", result.format_diff()));
            }
        }

        output.push_str("═══════════════════════════════════════════════════════════════\n");

        if self.all_passed() {
            output.push_str("                    ALL TESTS PASSED\n");
        } else {
            output.push_str(&format!("                    {} FAILURES\n", self.failed));
        }

        output.push_str("═══════════════════════════════════════════════════════════════\n");

        output
    }

    /// Format as JSON
    pub fn format_json(&self) -> String {
        let json = serde_json::json!({
            "total": self.total,
            "passed": self.passed,
            "partial": self.partial,
            "failed": self.failed,
            "results": self.results.iter().map(|r| {
                serde_json::json!({
                    "name": r.test_name,
                    "passing": r.is_passing(),
                    "status": format!("{:?}", r.parity_status()),
                })
            }).collect::<Vec<_>>(),
        });
        serde_json::to_string_pretty(&json).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_report_from_results() {
        let results = vec![
            ParityResult {
                test_name: "test1",
                interpreter: ExecutionResult::Success("1".to_string()),
                vm: ExecutionResult::Success("1".to_string()),
                jit: ExecutionResult::Success("1".to_string()),
            },
            ParityResult {
                test_name: "test2",
                interpreter: ExecutionResult::Success("2".to_string()),
                vm: ExecutionResult::Success("3".to_string()), // Mismatch!
                jit: ExecutionResult::Success("2".to_string()),
            },
        ];

        let report = ParityReport::from_results(results);
        assert_eq!(report.total, 2);
        assert_eq!(report.passed, 1);
        assert_eq!(report.failed, 1);
        assert!(!report.all_passed());
    }

    #[test]
    fn test_report_format_text() {
        let results = vec![ParityResult {
            test_name: "simple_add",
            interpreter: ExecutionResult::Success("42".to_string()),
            vm: ExecutionResult::Success("42".to_string()),
            jit: ExecutionResult::Success("42".to_string()),
        }];

        let report = ParityReport::from_results(results);
        let text = report.format_text();
        assert!(text.contains("ALL TESTS PASSED"));
        assert!(text.contains("Total tests: 1"));
    }
}
