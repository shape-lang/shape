//! Data access validation for time-series processing
//!
//! This module provides runtime guards to prevent accessing future data
//! during restricted execution modes (useful for any time-series domain).

use chrono::{DateTime, Utc};
use shape_ast::error::{Result, ShapeError};
use std::sync::RwLock;

/// Data access mode - controls which data can be accessed during evaluation
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DataAccessMode {
    /// Unrestricted - can access all data (historical analysis)
    Unrestricted,
    /// Restricted - no future data access (simulation, backtesting, validation)
    Restricted,
    /// Forward-only - only current and future data (real-time streaming)
    ForwardOnly,
}

/// Guards against accessing future data
#[derive(Debug)]
pub struct LookAheadGuard {
    mode: DataAccessMode,
    current_time: RwLock<Option<DateTime<Utc>>>,
    strict_mode: bool,
    access_log: RwLock<Vec<DataAccess>>,
}

/// Record of data access for auditing
#[derive(Debug, Clone)]
pub struct DataAccess {
    pub timestamp: DateTime<Utc>,
    pub accessed_time: DateTime<Utc>,
    pub access_type: String,
    pub allowed: bool,
}

impl LookAheadGuard {
    pub fn new(mode: DataAccessMode, strict_mode: bool) -> Self {
        Self {
            mode,
            current_time: RwLock::new(None),
            strict_mode,
            access_log: RwLock::new(Vec::new()),
        }
    }

    /// Set the current processing time
    pub fn set_current_time(&self, time: DateTime<Utc>) {
        *self.current_time.write().unwrap() = Some(time);
    }

    /// Check if accessing data at a specific time is allowed
    pub fn check_access(&self, access_time: DateTime<Utc>, access_type: &str) -> Result<()> {
        let current =
            self.current_time
                .read()
                .unwrap()
                .ok_or_else(|| ShapeError::RuntimeError {
                    message: "Current time not set in LookAheadGuard".to_string(),
                    location: None,
                })?;

        let allowed = match self.mode {
            DataAccessMode::Unrestricted => true, // Can access any data
            DataAccessMode::Restricted | DataAccessMode::ForwardOnly => access_time <= current,
        };

        // Log the access
        self.access_log.write().unwrap().push(DataAccess {
            timestamp: current,
            accessed_time: access_time,
            access_type: access_type.to_string(),
            allowed,
        });

        if !allowed {
            if self.strict_mode {
                return Err(ShapeError::RuntimeError {
                    message: format!(
                        "Future data access violation: Attempted to access data at {} while current time is {}",
                        access_time, current
                    ),
                    location: None,
                });
            } else {
                // In non-strict mode, log warning but continue
                eprintln!(
                    "WARNING: Future data access - accessing {} at current time {}",
                    access_time, current
                );
            }
        }

        Ok(())
    }

    /// Check if accessing a row index is allowed
    pub fn check_row_index(&self, index: i32, _access_type: &str) -> Result<()> {
        match self.mode {
            DataAccessMode::Unrestricted => Ok(()), // Can access any index
            DataAccessMode::Restricted | DataAccessMode::ForwardOnly => {
                if index > 0 {
                    let msg = format!(
                        "Future data access violation: Attempted to access data[{}] in restricted mode",
                        index
                    );

                    if self.strict_mode {
                        return Err(ShapeError::RuntimeError {
                            message: msg,
                            location: None,
                        });
                    } else {
                        eprintln!("WARNING: {}", msg);
                    }
                }
                Ok(())
            }
        }
    }

    /// Get access log for auditing
    pub fn get_access_log(&self) -> Vec<DataAccess> {
        self.access_log.read().unwrap().clone()
    }

    /// Clear access log
    pub fn clear_log(&self) {
        self.access_log.write().unwrap().clear();
    }

    /// Get summary of violations
    pub fn get_violation_summary(&self) -> LookAheadSummary {
        let log = self.access_log.read().unwrap();
        let violations: Vec<_> = log
            .iter()
            .filter(|access| !access.allowed)
            .cloned()
            .collect();

        LookAheadSummary {
            total_accesses: log.len(),
            violations: violations.len(),
            violation_details: violations,
        }
    }
}

impl Clone for LookAheadGuard {
    fn clone(&self) -> Self {
        Self {
            mode: self.mode,
            current_time: RwLock::new(*self.current_time.read().unwrap()),
            strict_mode: self.strict_mode,
            access_log: RwLock::new(self.access_log.read().unwrap().clone()),
        }
    }
}

/// Summary of look-ahead violations
#[derive(Debug, Clone)]
pub struct LookAheadSummary {
    pub total_accesses: usize,
    pub violations: usize,
    pub violation_details: Vec<DataAccess>,
}

impl LookAheadSummary {
    pub fn print_report(&self) {
        println!("=== Data Access Validation Report ===");
        println!("Total data accesses: {}", self.total_accesses);
        println!("Violations found: {}", self.violations);

        if self.violations > 0 {
            println!("\nViolation Details:");
            for (i, violation) in self.violation_details.iter().enumerate() {
                println!(
                    "  {}. At {}: Tried to access {} (type: {})",
                    i + 1,
                    violation.timestamp.format("%Y-%m-%d %H:%M:%S"),
                    violation.accessed_time.format("%Y-%m-%d %H:%M:%S"),
                    violation.access_type
                );
            }
        }
    }
}
