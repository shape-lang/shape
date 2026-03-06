//! Window function types for Shape AST

use serde::{Deserialize, Serialize};

use super::expressions::Expr;

/// SQL-style window function
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum WindowFunction {
    /// LAG(expr, offset, default) - access previous row value
    Lag {
        expr: Box<Expr>,
        offset: usize,
        default: Option<Box<Expr>>,
    },
    /// LEAD(expr, offset, default) - access next row value
    Lead {
        expr: Box<Expr>,
        offset: usize,
        default: Option<Box<Expr>>,
    },
    /// ROW_NUMBER() - sequential row number in partition
    RowNumber,
    /// RANK() - rank with gaps for ties
    Rank,
    /// DENSE_RANK() - rank without gaps
    DenseRank,
    /// NTILE(n) - divide rows into n buckets
    Ntile(usize),
    /// FIRST_VALUE(expr) - first value in window
    FirstValue(Box<Expr>),
    /// LAST_VALUE(expr) - last value in window
    LastValue(Box<Expr>),
    /// NTH_VALUE(expr, n) - nth value in window
    NthValue(Box<Expr>, usize),
    /// Running aggregate functions
    Sum(Box<Expr>),
    Avg(Box<Expr>),
    Min(Box<Expr>),
    Max(Box<Expr>),
    Count(Option<Box<Expr>>),
}

/// Sort direction for ORDER BY clause
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SortDirection {
    Ascending,
    Descending,
}

/// ORDER BY clause for query results
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OrderByClause {
    /// List of (expression, direction) pairs
    pub columns: Vec<(Expr, SortDirection)>,
}

/// Window specification for OVER clause
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WindowSpec {
    /// PARTITION BY expressions
    pub partition_by: Vec<Expr>,
    /// ORDER BY clause
    pub order_by: Option<OrderByClause>,
    /// Window frame (ROWS/RANGE BETWEEN)
    pub frame: Option<WindowFrame>,
}

/// Window frame definition
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WindowFrame {
    /// Frame type (ROWS or RANGE)
    pub frame_type: WindowFrameType,
    /// Start boundary
    pub start: WindowBound,
    /// End boundary (defaults to CURRENT ROW if not specified)
    pub end: WindowBound,
}

/// Window frame type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WindowFrameType {
    Rows,
    Range,
}

/// Window frame boundary
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WindowBound {
    /// UNBOUNDED PRECEDING
    UnboundedPreceding,
    /// UNBOUNDED FOLLOWING
    UnboundedFollowing,
    /// CURRENT ROW
    CurrentRow,
    /// n PRECEDING
    Preceding(usize),
    /// n FOLLOWING
    Following(usize),
}

/// Window function expression: func() OVER (window_spec)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WindowExpr {
    pub function: WindowFunction,
    pub over: WindowSpec,
}
