//! Property-specific fuzzy matching for Shape
//!
//! This module provides fine-grained fuzzy matching that allows different
//! tolerances for different row properties.

use shape_ast::error::{Result, ShapeError};
use std::collections::HashMap;

/// Property-specific tolerance configuration
#[derive(Debug, Clone)]
pub struct PropertyTolerances {
    /// Default tolerance for unspecified properties
    pub default: f64,
    /// Property-specific tolerances
    pub properties: HashMap<String, f64>,
}

impl Default for PropertyTolerances {
    fn default() -> Self {
        let mut properties = HashMap::new();

        // Default tolerances for common row properties
        properties.insert("body".to_string(), 0.02); // 2% for body size
        properties.insert("upper_wick".to_string(), 0.05); // 5% for upper wick
        properties.insert("lower_wick".to_string(), 0.05); // 5% for lower wick
        properties.insert("range".to_string(), 0.03); // 3% for high-low range
        properties.insert("open".to_string(), 0.01); // 1% for open price
        properties.insert("close".to_string(), 0.01); // 1% for close price
        properties.insert("high".to_string(), 0.01); // 1% for high price
        properties.insert("low".to_string(), 0.01); // 1% for low price
        properties.insert("volume".to_string(), 0.10); // 10% for volume

        Self {
            default: 0.02,
            properties,
        }
    }
}

impl PropertyTolerances {
    /// Get tolerance for a specific property
    pub fn get(&self, property: &str) -> f64 {
        self.properties
            .get(property)
            .copied()
            .unwrap_or(self.default)
    }

    /// Set tolerance for a specific property
    pub fn set(&mut self, property: String, tolerance: f64) -> Result<()> {
        if !(0.0..=1.0).contains(&tolerance) {
            return Err(ShapeError::RuntimeError {
                message: format!("Tolerance must be between 0.0 and 1.0, got {}", tolerance),
                location: None,
            });
        }
        self.properties.insert(property, tolerance);
        Ok(())
    }

    /// Parse from annotation arguments
    /// @fuzzy(0.02) - default tolerance
    /// @fuzzy(body: 0.02, wick: 0.05) - property-specific
    pub fn from_annotation_args(args: &[shape_ast::ast::Expr]) -> Result<Self> {
        let mut tolerances = Self::default();

        if args.is_empty() {
            return Ok(tolerances);
        }

        // Check if it's a single number (default tolerance)
        if args.len() == 1 {
            if let shape_ast::ast::Expr::Literal(shape_ast::ast::Literal::Number(n), _) = &args[0] {
                tolerances.default = *n;
                // Apply to all properties
                for (_, tol) in tolerances.properties.iter_mut() {
                    *tol = *n;
                }
                return Ok(tolerances);
            }
        }

        // Otherwise parse property: value pairs
        // This would need more sophisticated parsing, for now use defaults
        Ok(tolerances)
    }
}

/// Enhanced fuzzy matcher with property-specific tolerances
pub struct PropertyFuzzyMatcher {
    tolerances: PropertyTolerances,
}

impl PropertyFuzzyMatcher {
    pub fn new(tolerances: PropertyTolerances) -> Self {
        Self { tolerances }
    }

    /// Fuzzy compare with property context
    pub fn fuzzy_compare(&self, property: &str, actual: f64, expected: f64, op: FuzzyOp) -> bool {
        let tolerance = self.tolerances.get(property);

        match op {
            FuzzyOp::Equal => {
                let diff = (actual - expected).abs();
                let avg = (actual.abs() + expected.abs()) / 2.0;
                if avg == 0.0 {
                    diff == 0.0
                } else {
                    diff / avg <= tolerance
                }
            }
            FuzzyOp::Greater => actual > expected * (1.0 - tolerance),
            FuzzyOp::Less => actual < expected * (1.0 + tolerance),
            FuzzyOp::GreaterEq => actual >= expected * (1.0 - tolerance),
            FuzzyOp::LessEq => actual <= expected * (1.0 + tolerance),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum FuzzyOp {
    Equal,
    Greater,
    Less,
    GreaterEq,
    LessEq,
}
