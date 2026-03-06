//! Operator types for Shape AST

use serde::{Deserialize, Serialize};

/// Kind of range: exclusive (..) or inclusive (..=)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RangeKind {
    /// Exclusive range: start..end (excludes end)
    Exclusive,
    /// Inclusive range: start..=end (includes end)
    Inclusive,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum BinaryOp {
    // Arithmetic
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,

    // Comparison
    Greater,
    Less,
    GreaterEq,
    LessEq,
    Equal,
    NotEqual,

    // Fuzzy comparison
    FuzzyEqual,   // ~=
    FuzzyGreater, // ~>
    FuzzyLess,    // ~<

    // Bitwise
    BitAnd,
    BitOr,
    BitXor,
    BitShl,
    BitShr,

    // Logical
    And,
    Or,

    // Null handling
    NullCoalesce, // ??
    ErrorContext, // !!

    // Pipeline
    Pipe, // |>
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum UnaryOp {
    Not,
    Neg,
    BitNot,
}

/// Tolerance specification for fuzzy comparisons
/// Used with `within` syntax: `a ~= b within 0.02` or `a ~= b within 2%`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FuzzyTolerance {
    /// Absolute tolerance: `within 5` means |a - b| <= 5
    Absolute(f64),
    /// Percentage tolerance: `within 2%` means |a - b| / avg(|a|, |b|) <= 0.02
    Percentage(f64),
}

impl FuzzyTolerance {
    /// Check if two values are within tolerance
    pub fn is_within(&self, a: f64, b: f64) -> bool {
        let diff = (a - b).abs();
        match self {
            FuzzyTolerance::Absolute(tol) => diff <= *tol,
            FuzzyTolerance::Percentage(pct) => {
                let avg = (a.abs() + b.abs()) / 2.0;
                if avg == 0.0 {
                    diff == 0.0
                } else {
                    diff / avg <= *pct
                }
            }
        }
    }

    /// Get the tolerance value (percentage in 0-1 form, or absolute)
    pub fn value(&self) -> f64 {
        match self {
            FuzzyTolerance::Absolute(v) | FuzzyTolerance::Percentage(v) => *v,
        }
    }
}

/// Fuzzy comparison operator type
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum FuzzyOp {
    /// Fuzzy equal: ~=
    Equal,
    /// Fuzzy greater: ~>
    Greater,
    /// Fuzzy less: ~<
    Less,
}
