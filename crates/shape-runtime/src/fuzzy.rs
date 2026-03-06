//! Fuzzy matching algorithms for pattern recognition

use std::f64;

/// Configuration for fuzzy matching
#[derive(Debug, Clone)]
pub struct FuzzyConfig {
    /// Default tolerance for fuzzy equality (as a percentage)
    pub default_tolerance: f64,
    /// Tolerance for price-based comparisons
    pub price_tolerance: f64,
    /// Tolerance for volume-based comparisons
    pub volume_tolerance: f64,
    /// Whether to use adaptive tolerance based on volatility
    pub adaptive_tolerance: bool,
}

impl Default for FuzzyConfig {
    fn default() -> Self {
        Self {
            default_tolerance: 0.02, // 2%
            price_tolerance: 0.02,   // 2%
            volume_tolerance: 0.05,  // 5%
            adaptive_tolerance: true,
        }
    }
}

/// Fuzzy comparison functions
#[derive(Debug, Clone)]
pub struct FuzzyMatcher {
    config: FuzzyConfig,
}

impl FuzzyMatcher {
    /// Create a new fuzzy matcher with default configuration
    pub fn new() -> Self {
        Self {
            config: FuzzyConfig::default(),
        }
    }

    /// Create a fuzzy matcher with custom configuration
    pub fn with_config(config: FuzzyConfig) -> Self {
        Self { config }
    }

    /// Fuzzy equality comparison
    pub fn fuzzy_equal(&self, a: f64, b: f64, tolerance: Option<f64>) -> bool {
        let tol = tolerance.unwrap_or(self.config.default_tolerance);
        let diff = (a - b).abs();
        let avg = (a.abs() + b.abs()) / 2.0;

        if avg == 0.0 {
            diff == 0.0
        } else {
            diff / avg <= tol
        }
    }

    /// Fuzzy greater than comparison
    pub fn fuzzy_greater(&self, a: f64, b: f64, tolerance: Option<f64>) -> bool {
        let tol = tolerance.unwrap_or(self.config.default_tolerance);
        a > b * (1.0 - tol)
    }

    /// Fuzzy less than comparison
    pub fn fuzzy_less(&self, a: f64, b: f64, tolerance: Option<f64>) -> bool {
        let tol = tolerance.unwrap_or(self.config.default_tolerance);
        a < b * (1.0 + tol)
    }

    /// Calculate adaptive tolerance based on volatility
    pub fn adaptive_tolerance(&self, values: &[f64]) -> f64 {
        if !self.config.adaptive_tolerance || values.len() < 2 {
            return self.config.default_tolerance;
        }

        // Calculate standard deviation
        let mean = values.iter().sum::<f64>() / values.len() as f64;
        let variance = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / values.len() as f64;
        let std_dev = variance.sqrt();

        // Adjust tolerance based on volatility
        let volatility = if mean != 0.0 {
            std_dev / mean.abs()
        } else {
            0.0
        };

        // Scale tolerance: higher volatility = higher tolerance
        let scaled_tolerance = self.config.default_tolerance * (1.0 + volatility).min(3.0);

        scaled_tolerance.min(0.1) // Cap at 10%
    }

    /// Fuzzy pattern matching with weighted conditions
    pub fn match_pattern(&self, conditions: &[(bool, f64)], threshold: f64) -> bool {
        if conditions.is_empty() {
            return false;
        }

        let total_weight: f64 = conditions.iter().map(|(_, w)| w).sum();
        let matched_weight: f64 = conditions
            .iter()
            .filter(|(matched, _)| *matched)
            .map(|(_, w)| w)
            .sum();

        if total_weight == 0.0 {
            // If no weights, use simple majority
            let matched_count = conditions.iter().filter(|(m, _)| *m).count();
            matched_count as f64 / conditions.len() as f64 >= threshold
        } else {
            matched_weight / total_weight >= threshold
        }
    }

    /// Calculate similarity score between two numeric sequences
    pub fn sequence_similarity(&self, seq1: &[f64], seq2: &[f64]) -> f64 {
        if seq1.is_empty() || seq2.is_empty() || seq1.len() != seq2.len() {
            return 0.0;
        }

        // Normalize sequences
        let norm1 = Self::normalize_sequence(seq1);
        let norm2 = Self::normalize_sequence(seq2);

        // Calculate correlation coefficient
        let n = norm1.len() as f64;
        let sum_xy: f64 = norm1.iter().zip(&norm2).map(|(x, y)| x * y).sum();
        let sum_x: f64 = norm1.iter().sum();
        let sum_y: f64 = norm2.iter().sum();
        let sum_x2: f64 = norm1.iter().map(|x| x * x).sum();
        let sum_y2: f64 = norm2.iter().map(|y| y * y).sum();

        let numerator = n * sum_xy - sum_x * sum_y;
        let denominator = ((n * sum_x2 - sum_x * sum_x) * (n * sum_y2 - sum_y * sum_y)).sqrt();

        if denominator == 0.0 {
            1.0 // Both sequences are constant
        } else {
            (numerator / denominator).abs() // Take absolute value for similarity
        }
    }

    /// Normalize a sequence to have mean 0 and std dev 1
    fn normalize_sequence(seq: &[f64]) -> Vec<f64> {
        let mean = seq.iter().sum::<f64>() / seq.len() as f64;
        let variance = seq.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / seq.len() as f64;
        let std_dev = variance.sqrt();

        if std_dev == 0.0 {
            vec![0.0; seq.len()]
        } else {
            seq.iter().map(|v| (v - mean) / std_dev).collect()
        }
    }
}

impl Default for FuzzyMatcher {
    fn default() -> Self {
        Self::new()
    }
}
