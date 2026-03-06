//! Extensions to the AST for flexible query execution
//!
//! This module proposes enhanced query structures that support
//! Shape-defined rules for analysis and backtesting.

use serde::{Deserialize, Serialize};
use shape_ast::ast::{Block, Expr, TimeWindow, Timeframe};

/// Process statement for unified execution
/// This is the new primary way to execute analysis and backtesting
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessStatement {
    /// The points to process (variable name or expression)
    pub target: ProcessTarget,

    /// Execution rules defined in Shape
    pub rules: ProcessRules,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ProcessTarget {
    /// Process a variable containing points
    Variable(String),
    /// Process results of a find expression
    FindExpr(Box<Expr>),
    /// Process all rows in the data
    AllRows,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessRules {
    /// Initial state setup
    pub state: Option<StateBlock>,

    /// Rules evaluated at each point
    pub on_point: Option<Block>,

    /// Rules evaluated on each subsequent row
    pub on_bar: Option<Block>,

    /// Final aggregation/results
    pub finalize: Option<Block>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateBlock {
    /// State variable declarations
    pub declarations: Vec<StateDeclaration>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateDeclaration {
    pub name: String,
    pub value: Expr,
}

/// Pattern reference for AST extensions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PatternReference {
    /// Reference to a named pattern
    Named(String),
}

/// Find clause that identifies pattern matches
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FindClause {
    pub pattern: PatternReference,
    pub window: Option<TimeWindow>,
    pub where_conditions: Vec<Expr>,
    pub timeframe: Option<Timeframe>,
    pub as_name: Option<String>, // Store matches as variable
}

/// Analyze clause with Shape-defined rules
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalyzeClause {
    /// The target to analyze (usually the find results)
    pub target: Option<String>, // Variable name from find clause

    /// Analysis rules as a block of statements
    pub with_rules: Block,
}

/// Output clause for formatting results
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputClause {
    /// Output specification as object literal or block
    pub spec: Expr,
}

// DEPRECATED: AnalyzeQuery type removed - use method chaining instead
// /// Alternative: Extend existing queries with rule blocks
// #[derive(Debug, Clone, Serialize, Deserialize)]
// pub struct EnhancedAnalyzeQuery {
//     /// Target of analysis (pattern matches or expression)
//     pub target: AnalysisTarget,
//
//     /// Analysis rules defined in Shape
//     pub rules: Option<Block>,
//
//     /// Predefined metrics (for backward compatibility)
//     pub metrics: Vec<String>,
// }
//
// #[derive(Debug, Clone, Serialize, Deserialize)]
// pub enum AnalysisTarget {
//     /// Analyze pattern matches
//     Pattern(PatternReference),
//     /// Analyze a time window
//     Window(TimeWindow),
//     /// Analyze expression results
//     Expression(Expr),
//     /// Analyze a variable (from previous find)
//     Variable(String),
// }
