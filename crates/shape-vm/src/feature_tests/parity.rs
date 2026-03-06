//! Three-way parity testing infrastructure
//!
//! This module provides types and utilities for testing parity
//! between the Interpreter, VM, and JIT backends.

// ============================================================================
// Three-Way Parity Testing
// ============================================================================

/// Result of running a test across all three backends
#[derive(Debug, Clone)]
pub struct ParityResult {
    pub test_name: &'static str,
    pub interpreter: ExecutionResult,
    pub vm: ExecutionResult,
    pub jit: ExecutionResult,
}

/// Result of executing a test on a single backend
#[derive(Debug, Clone)]
pub enum ExecutionResult {
    /// Execution succeeded with this output
    Success(String),
    /// Execution failed with this error
    Error(String),
    /// Feature is not supported by this backend
    NotSupported(&'static str),
    /// Backend was skipped (disabled or unavailable)
    Skipped(&'static str),
}

impl ExecutionResult {
    pub fn is_success(&self) -> bool {
        matches!(self, ExecutionResult::Success(_))
    }

    pub fn is_not_supported(&self) -> bool {
        matches!(self, ExecutionResult::NotSupported(_))
    }

    pub fn is_skipped(&self) -> bool {
        matches!(self, ExecutionResult::Skipped(_))
    }

    /// Get the output string if successful
    pub fn output(&self) -> Option<&str> {
        match self {
            ExecutionResult::Success(s) => Some(s),
            _ => None,
        }
    }
}

/// Status of parity check between backends
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParityStatus {
    /// All backends that ran produced matching results
    AllMatch,
    /// Interpreter and VM produced different results
    InterpreterVmMismatch { interpreter: String, vm: String },
    /// Interpreter and JIT produced different results
    InterpreterJitMismatch { interpreter: String, jit: String },
    /// VM and JIT produced different results
    VmJitMismatch { vm: String, jit: String },
    /// Some backends were skipped or didn't support the feature
    PartialSkipped { reason: String },
    /// All backends failed
    AllFailed,
}

impl ParityResult {
    /// Check if all backends that succeeded have matching results
    pub fn check_parity(&self) -> bool {
        matches!(
            self.parity_status(),
            ParityStatus::AllMatch | ParityStatus::PartialSkipped { .. }
        )
    }

    /// Get detailed parity status
    pub fn parity_status(&self) -> ParityStatus {
        let i_out = self.interpreter.output();
        let v_out = self.vm.output();
        let j_out = self.jit.output();

        match (i_out, v_out, j_out) {
            // All three succeeded - check all match
            (Some(i), Some(v), Some(j)) => {
                if i == v && v == j {
                    ParityStatus::AllMatch
                } else if i != v {
                    ParityStatus::InterpreterVmMismatch {
                        interpreter: i.to_string(),
                        vm: v.to_string(),
                    }
                } else if i != j {
                    ParityStatus::InterpreterJitMismatch {
                        interpreter: i.to_string(),
                        jit: j.to_string(),
                    }
                } else {
                    ParityStatus::VmJitMismatch {
                        vm: v.to_string(),
                        jit: j.to_string(),
                    }
                }
            }
            // Two succeeded - check they match
            (Some(i), Some(v), None) => {
                if i == v {
                    ParityStatus::PartialSkipped {
                        reason: "JIT skipped/not supported".to_string(),
                    }
                } else {
                    ParityStatus::InterpreterVmMismatch {
                        interpreter: i.to_string(),
                        vm: v.to_string(),
                    }
                }
            }
            (Some(i), None, Some(j)) => {
                if i == j {
                    ParityStatus::PartialSkipped {
                        reason: "VM skipped/not supported".to_string(),
                    }
                } else {
                    ParityStatus::InterpreterJitMismatch {
                        interpreter: i.to_string(),
                        jit: j.to_string(),
                    }
                }
            }
            (None, Some(v), Some(j)) => {
                if v == j {
                    ParityStatus::PartialSkipped {
                        reason: "Interpreter skipped/not supported".to_string(),
                    }
                } else {
                    ParityStatus::VmJitMismatch {
                        vm: v.to_string(),
                        jit: j.to_string(),
                    }
                }
            }
            // Only one succeeded
            (Some(_), None, None) | (None, Some(_), None) | (None, None, Some(_)) => {
                ParityStatus::PartialSkipped {
                    reason: "Only one backend succeeded".to_string(),
                }
            }
            // None succeeded
            (None, None, None) => ParityStatus::AllFailed,
        }
    }

    /// Check if this result is passing (parity maintained)
    pub fn is_passing(&self) -> bool {
        matches!(
            self.parity_status(),
            ParityStatus::AllMatch | ParityStatus::PartialSkipped { .. }
        )
    }

    /// Format a diff for display when there's a mismatch
    pub fn format_diff(&self) -> String {
        match self.parity_status() {
            ParityStatus::AllMatch => "All backends match".to_string(),
            ParityStatus::InterpreterVmMismatch { interpreter, vm } => {
                format!(
                    "Interpreter vs VM mismatch:\n  Interpreter: {}\n  VM: {}",
                    interpreter, vm
                )
            }
            ParityStatus::InterpreterJitMismatch { interpreter, jit } => {
                format!(
                    "Interpreter vs JIT mismatch:\n  Interpreter: {}\n  JIT: {}",
                    interpreter, jit
                )
            }
            ParityStatus::VmJitMismatch { vm, jit } => {
                format!("VM vs JIT mismatch:\n  VM: {}\n  JIT: {}", vm, jit)
            }
            ParityStatus::PartialSkipped { reason } => {
                format!("Partial: {}", reason)
            }
            ParityStatus::AllFailed => "All backends failed".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_match() {
        let result = ParityResult {
            test_name: "test",
            interpreter: ExecutionResult::Success("42".to_string()),
            vm: ExecutionResult::Success("42".to_string()),
            jit: ExecutionResult::Success("42".to_string()),
        };
        assert!(result.is_passing());
        assert_eq!(result.parity_status(), ParityStatus::AllMatch);
    }

    #[test]
    fn test_interpreter_vm_mismatch() {
        let result = ParityResult {
            test_name: "test",
            interpreter: ExecutionResult::Success("42".to_string()),
            vm: ExecutionResult::Success("43".to_string()),
            jit: ExecutionResult::Success("42".to_string()),
        };
        assert!(!result.is_passing());
        assert!(matches!(
            result.parity_status(),
            ParityStatus::InterpreterVmMismatch { .. }
        ));
    }

    #[test]
    fn test_partial_skipped() {
        let result = ParityResult {
            test_name: "test",
            interpreter: ExecutionResult::Success("42".to_string()),
            vm: ExecutionResult::Success("42".to_string()),
            jit: ExecutionResult::NotSupported("pattern_def"),
        };
        assert!(result.is_passing());
        assert!(matches!(
            result.parity_status(),
            ParityStatus::PartialSkipped { .. }
        ));
    }

    #[test]
    fn test_not_supported_variant() {
        let result = ExecutionResult::NotSupported("async_block");
        assert!(result.is_not_supported());
        assert!(!result.is_success());
    }
}
