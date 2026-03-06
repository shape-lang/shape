//! Query types for Shape AST

use serde::{Deserialize, Serialize};

use super::expressions::Expr;
use super::time::TimeWindow;
use super::windows::OrderByClause;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Query {
    Backtest(BacktestQuery),
    Alert(AlertQuery),
    /// Query with Common Table Expressions (CTEs)
    With(WithQuery),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BacktestQuery {
    pub strategy: String,
    pub window: TimeWindow,
    pub parameters: Vec<(String, Expr)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertQuery {
    pub condition: Expr,
    pub message: Option<String>,
    pub webhook: Option<String>,
}

/// A Common Table Expression (CTE) - named subquery
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cte {
    /// Name of the CTE (referenced in main query)
    pub name: String,
    /// Optional column names for the CTE
    pub columns: Option<Vec<String>>,
    /// The subquery that defines this CTE
    pub query: Box<Query>,
    /// Whether this CTE is recursive
    pub recursive: bool,
}

/// Query with CTEs (WITH clause)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WithQuery {
    /// List of CTEs defined in the WITH clause
    pub ctes: Vec<Cte>,
    /// The main query that uses the CTEs
    pub query: Box<Query>,
}

/// Common query modifiers (LIMIT, ORDER BY)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QueryModifiers {
    /// Maximum number of results to return
    pub limit: Option<usize>,
    /// Ordering for results
    pub order_by: Option<OrderByClause>,
}
