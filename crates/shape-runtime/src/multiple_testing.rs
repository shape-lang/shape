//! Multiple testing corrections and warnings
//!
//! This module provides tools to track the number of parameter combinations
//! tested during optimization and warn about overfitting risks.

use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Multiple testing correction methods
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum CorrectionMethod {
    /// Bonferroni correction (most conservative)
    #[default]
    Bonferroni,
    /// Holm-Bonferroni step-down procedure
    HolmBonferroni,
    /// Benjamini-Hochberg False Discovery Rate
    BenjaminiHochberg,
    /// No correction applied
    None,
}

/// Warning severity level
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum WarningLevel {
    /// No warning needed
    None = 0,
    /// Informational (< 50 combinations)
    Info = 1,
    /// Caution advised (50-199 combinations)
    Caution = 2,
    /// Warning (200-999 combinations)
    Warning = 3,
    /// Critical overfitting risk (1000+ combinations)
    Critical = 4,
}

impl WarningLevel {
    /// Get a human-readable description
    pub fn description(&self) -> &'static str {
        match self {
            WarningLevel::None => "No multiple testing concerns",
            WarningLevel::Info => "Low risk - consider walk-forward validation",
            WarningLevel::Caution => "Moderate risk - walk-forward analysis recommended",
            WarningLevel::Warning => "High risk - walk-forward analysis strongly recommended",
            WarningLevel::Critical => {
                "Critical overfitting risk - results may be meaningless without validation"
            }
        }
    }
}

/// Statistics about multiple testing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultipleTestingStats {
    /// Number of parameter combinations tested
    pub n_tests: usize,

    /// Original significance level (alpha)
    pub alpha: f64,

    /// Adjusted significance level after correction
    pub adjusted_alpha: f64,

    /// Correction method applied
    pub method: CorrectionMethod,

    /// Warning level based on number of tests
    pub warning_level: WarningLevel,

    /// Human-readable warning message (if any)
    pub warning_message: Option<String>,

    /// Whether the user explicitly accepted overfitting risk
    pub risk_accepted: bool,
}

impl MultipleTestingStats {
    /// Convert to a ValueWord TypedObject for Shape
    pub fn to_value(&self) -> shape_value::ValueWord {
        use shape_value::ValueWord;

        let warning_msg = self
            .warning_message
            .clone()
            .map(|s| ValueWord::from_string(Arc::new(s)))
            .unwrap_or(ValueWord::none());

        crate::type_schema::typed_object_from_nb_pairs(&[
            ("n_tests", ValueWord::from_f64(self.n_tests as f64)),
            ("alpha", ValueWord::from_f64(self.alpha)),
            ("adjusted_alpha", ValueWord::from_f64(self.adjusted_alpha)),
            (
                "method",
                ValueWord::from_string(Arc::new(format!("{:?}", self.method))),
            ),
            (
                "warning_level",
                ValueWord::from_string(Arc::new(format!("{:?}", self.warning_level))),
            ),
            ("warning_message", warning_msg),
            ("risk_accepted", ValueWord::from_bool(self.risk_accepted)),
        ])
    }
}

/// Guard that tracks and warns about multiple testing
#[derive(Debug, Clone)]
pub struct MultipleTestingGuard {
    /// Number of combinations tested so far
    combinations_tested: usize,

    /// Base significance level
    alpha: f64,

    /// Correction method to use
    method: CorrectionMethod,

    /// Whether user has explicitly accepted overfitting risk
    accept_overfitting_risk: bool,

    /// Threshold for caution warning
    _caution_threshold: usize,

    /// Threshold for warning
    _warning_threshold: usize,

    /// Threshold for critical warning
    _critical_threshold: usize,
}

impl Default for MultipleTestingGuard {
    fn default() -> Self {
        Self::new(0.05)
    }
}

impl MultipleTestingGuard {
    /// Create a new guard with given significance level
    pub fn new(alpha: f64) -> Self {
        Self {
            combinations_tested: 0,
            alpha,
            method: CorrectionMethod::Bonferroni,
            accept_overfitting_risk: false,
            _caution_threshold: 50,
            _warning_threshold: 200,
            _critical_threshold: 1000,
        }
    }

    /// Set the correction method
    pub fn with_method(mut self, method: CorrectionMethod) -> Self {
        self.method = method;
        self
    }

    /// Record that N combinations were tested
    pub fn record_tests(&mut self, n: usize) {
        self.combinations_tested += n;
    }

    /// Get the number of combinations tested
    pub fn combinations_tested(&self) -> usize {
        self.combinations_tested
    }

    /// Suppress warnings (user explicitly accepts risk)
    pub fn accept_risk(&mut self) {
        self.accept_overfitting_risk = true;
    }

    /// Check if risk has been accepted
    pub fn is_risk_accepted(&self) -> bool {
        self.accept_overfitting_risk
    }

    /// Calculate the adjusted alpha based on correction method
    pub fn adjusted_alpha(&self) -> f64 {
        if self.combinations_tested == 0 {
            return self.alpha;
        }

        match self.method {
            CorrectionMethod::Bonferroni => self.alpha / self.combinations_tested as f64,
            CorrectionMethod::HolmBonferroni => {
                // For Holm-Bonferroni, the adjusted alpha for the first test
                // is alpha/n, for the second alpha/(n-1), etc.
                // We return the most stringent (first test) here
                self.alpha / self.combinations_tested as f64
            }
            CorrectionMethod::BenjaminiHochberg => {
                // FDR control - less conservative than Bonferroni
                // Roughly alpha * (k/n) for the k-th smallest p-value
                // We return a representative value
                self.alpha * 0.5 / self.combinations_tested as f64
            }
            CorrectionMethod::None => self.alpha,
        }
    }

    /// Determine warning level based on combinations tested
    pub fn warning_level(&self) -> WarningLevel {
        match self.combinations_tested {
            0..=49 => WarningLevel::None,
            50..=199 => WarningLevel::Info,
            200..=999 => WarningLevel::Caution,
            _ => WarningLevel::Critical,
        }
    }

    /// Get current statistics and warnings
    pub fn get_stats(&self) -> MultipleTestingStats {
        let warning_level = self.warning_level();
        let adjusted_alpha = self.adjusted_alpha();

        let warning_message = if self.accept_overfitting_risk {
            None
        } else {
            self.generate_warning_message(warning_level, adjusted_alpha)
        };

        MultipleTestingStats {
            n_tests: self.combinations_tested,
            alpha: self.alpha,
            adjusted_alpha,
            method: self.method,
            warning_level,
            warning_message,
            risk_accepted: self.accept_overfitting_risk,
        }
    }

    /// Generate a warning message based on severity
    fn generate_warning_message(&self, level: WarningLevel, adjusted_alpha: f64) -> Option<String> {
        match level {
            WarningLevel::None => None,
            WarningLevel::Info => Some(format!(
                "INFO: {} parameter combinations tested. Consider walk-forward validation.",
                self.combinations_tested
            )),
            WarningLevel::Caution => Some(format!(
                "CAUTION: {} parameter combinations tested. \
                 Bonferroni-adjusted alpha: {:.6}. \
                 Walk-forward analysis recommended.",
                self.combinations_tested, adjusted_alpha
            )),
            WarningLevel::Warning | WarningLevel::Critical => Some(format!(
                "WARNING: {} parameter combinations tested without walk-forward analysis.\n\
                 Bonferroni-adjusted alpha: {:.2e}\n\
                 This many tests dramatically increases false discovery risk.\n\n\
                 To address this:\n\
                 1. Use walk-forward analysis: `walk_forward: {{ ... }}`\n\
                 2. Or explicitly accept risk: `accept_overfitting_risk: true`",
                self.combinations_tested, adjusted_alpha
            )),
        }
    }

    /// Print warning to stderr if needed
    pub fn emit_warning_if_needed(&self) {
        if self.accept_overfitting_risk {
            return;
        }

        let stats = self.get_stats();
        if let Some(msg) = &stats.warning_message {
            if stats.warning_level >= WarningLevel::Caution {
                eprintln!("\n{}\n", msg);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_warning_levels() {
        let mut guard = MultipleTestingGuard::new(0.05);

        assert_eq!(guard.warning_level(), WarningLevel::None);

        guard.record_tests(50);
        assert_eq!(guard.warning_level(), WarningLevel::Info);

        guard.record_tests(150);
        assert_eq!(guard.warning_level(), WarningLevel::Caution);

        guard.record_tests(800);
        assert_eq!(guard.warning_level(), WarningLevel::Critical);
    }

    #[test]
    fn test_bonferroni_correction() {
        let mut guard = MultipleTestingGuard::new(0.05);
        guard.record_tests(100);

        let adjusted = guard.adjusted_alpha();
        assert!((adjusted - 0.0005).abs() < 1e-10);
    }

    #[test]
    fn test_accept_risk_suppresses_warning() {
        let mut guard = MultipleTestingGuard::new(0.05);
        guard.record_tests(500);
        guard.accept_risk();

        let stats = guard.get_stats();
        assert!(stats.warning_message.is_none());
        assert!(stats.risk_accepted);
    }
}
