//! Lookahead bias and data leakage detection
//!
//! This module provides heuristic detection of common simulation mistakes:
//! - Using current element data for events and executing at same index
//! - Insufficient warmup periods for stateful functions
//! - Using future information in calculations
//!
//! These are "honest defaults" that warn users about potential simulation flaws.

use serde::{Deserialize, Serialize};

/// Leakage warning severity
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum LeakageSeverity {
    /// Informational - might be intentional
    Info,
    /// Warning - likely problematic
    Warning,
    /// Critical - almost certainly a bug
    Critical,
}

/// Types of leakage detected
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LeakageType {
    /// Using current index for event and executing at same index
    SameStepExecution,
    /// Function warmup period is insufficient
    InsufficientWarmup {
        function: String,
        required: usize,
        provided: usize,
    },
    /// Using future data in calculation
    FutureLookup { index: i32 },
    /// Potential peak into future via improper index
    SuspiciousIndex { context: String },
}

/// A single leakage warning
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeakageWarning {
    /// Type of leakage
    pub leak_type: LeakageType,
    /// Severity level
    pub severity: LeakageSeverity,
    /// Human-readable message
    pub message: String,
    /// Location in code (if available)
    pub location: Option<String>,
    /// Suggested fix
    pub suggestion: Option<String>,
}

impl LeakageWarning {
    /// Create a same-step execution warning
    pub fn same_step_execution(location: Option<&str>) -> Self {
        Self {
            leak_type: LeakageType::SameStepExecution,
            severity: LeakageSeverity::Critical,
            message: "Signal uses current index data and executes at same index. This is look-ahead bias - in live processing you cannot know current value until the step completes.".to_string(),
            location: location.map(|s| s.to_string()),
            suggestion: Some("Use execution_delay: 1 or execute at next step".to_string()),
        }
    }

    /// Create an insufficient warmup warning
    pub fn insufficient_warmup(function: &str, required: usize, provided: usize) -> Self {
        Self {
            leak_type: LeakageType::InsufficientWarmup {
                function: function.to_string(),
                required,
                provided,
            },
            severity: if provided == 0 {
                LeakageSeverity::Critical
            } else if provided < required / 2 {
                LeakageSeverity::Warning
            } else {
                LeakageSeverity::Info
            },
            message: format!(
                "Function '{}' requires {} elements to warm up, but only {} elements of warmup provided. Early signals may be unreliable.",
                function, required, provided
            ),
            location: None,
            suggestion: Some(format!(
                "Add warmup: {} to simulation config or skip first {} elements",
                required, required
            )),
        }
    }

    /// Create a future lookup warning
    pub fn future_lookup(index: i32, location: Option<&str>) -> Self {
        Self {
            leak_type: LeakageType::FutureLookup { index },
            severity: LeakageSeverity::Critical,
            message: format!(
                "Accessing future data with positive index [{}]. This data is not available at decision time.",
                index
            ),
            location: location.map(|s| s.to_string()),
            suggestion: Some("Use negative or zero indices for historical data".to_string()),
        }
    }
}

/// Detector for leakage in a simulation
#[derive(Debug, Default)]
pub struct LeakageDetector {
    pub warnings: Vec<LeakageWarning>,
}

impl LeakageDetector {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_warning(&mut self, warning: LeakageWarning) {
        self.warnings.push(warning);
    }

    pub fn report(&self) -> LeakageReport {
        LeakageReport {
            warnings: self.warnings.clone(),
            total_warnings: self.warnings.len(),
            max_severity: self
                .warnings
                .iter()
                .map(|w| w.severity)
                .max()
                .unwrap_or(LeakageSeverity::Info),
        }
    }

    pub fn check_row_index(&self, index: i32, context: &str) -> shape_ast::error::Result<()> {
        if index > 0 {
            return Err(shape_ast::error::ShapeError::RuntimeError {
                message: format!(
                    "Lookahead error: accessing future index {} in {}",
                    index, context
                ),
                location: None,
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeakageReport {
    pub warnings: Vec<LeakageWarning>,
    pub total_warnings: usize,
    pub max_severity: LeakageSeverity,
}
