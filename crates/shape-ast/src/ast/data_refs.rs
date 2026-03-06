//! Data reference types for Shape AST

use serde::{Deserialize, Serialize};

use super::time::{DateTimeExpr, Timeframe};

/// Data reference: data[0], data[-1], data[1:5]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DataRef {
    pub index: DataIndex,
    /// Optional timeframe for multi-timeframe access: data[0, 5m] or data(5m)[0]
    pub timeframe: Option<Timeframe>,
}

/// DateTime-based data reference: data[@"2024-01-15"]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DataDateTimeRef {
    pub datetime: DateTimeExpr,
    pub timezone: Option<String>,
    pub timeframe: Option<Timeframe>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DataIndex {
    /// Single data row with static index: data[0]
    Single(i32),
    /// Range of data rows with static indices: data[1:5]
    Range(i32, i32),
    /// Single data row with expression index: data[variable_name]
    Expression(Box<super::expressions::Expr>),
    /// Range with expression indices: data[start_expr:end_expr]
    ExpressionRange(Box<super::expressions::Expr>, Box<super::expressions::Expr>),
}
