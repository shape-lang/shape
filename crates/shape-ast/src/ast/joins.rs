//! JOIN operation types for Shape AST

use serde::{Deserialize, Serialize};

use super::expressions::Expr;

/// JOIN type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum JoinType {
    Inner,
    Left,
    Right,
    Full,
    Cross,
}

/// JOIN condition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum JoinCondition {
    /// ON expression (e.g., ON a.id = b.id)
    On(Expr),
    /// USING columns (e.g., USING (id, name))
    Using(Vec<String>),
    /// Temporal join (within a time duration)
    Temporal {
        /// Time column on left side
        left_time: String,
        /// Time column on right side
        right_time: String,
        /// Maximum time difference
        within: crate::data::Timeframe,
    },
    /// Natural join (implicit matching columns)
    Natural,
}

/// A JOIN clause
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JoinClause {
    /// Type of join
    pub join_type: JoinType,
    /// Right side of the join
    pub right: JoinSource,
    /// Join condition
    pub condition: JoinCondition,
}

/// Source for a JOIN
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum JoinSource {
    /// Named data source (table/symbol)
    Named(String),
    /// Subquery
    Subquery(Box<super::queries::Query>),
    /// CTE reference
    Cte(String),
}
