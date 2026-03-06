//! Helper types for expressions in Shape AST

use serde::{Deserialize, Serialize};

use super::expressions::Expr;
use super::functions::Annotation;
use super::patterns::{DestructurePattern, Pattern};
use super::program::VariableDecl;
use super::span::Span;
use super::types::TypeAnnotation;

/// Block expression containing multiple statements
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlockExpr {
    /// The statements in the block
    pub items: Vec<BlockItem>,
}

/// An item in a block expression
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum BlockItem {
    /// Variable declaration
    VariableDecl(VariableDecl),
    /// Assignment
    Assignment(super::program::Assignment),
    /// Statement
    Statement(super::statements::Statement),
    /// Expression (the last expression's value is the block's value)
    Expression(Expr),
}

/// If expression that returns a value
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IfExpr {
    pub condition: Box<Expr>,
    pub then_branch: Box<Expr>,
    pub else_branch: Option<Box<Expr>>, // Defaults to Unit if missing
}

/// While expression that returns a value
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WhileExpr {
    pub condition: Box<Expr>,
    pub body: Box<Expr>,
}

/// For expression that returns a value
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ForExpr {
    pub pattern: Pattern,
    pub iterable: Box<Expr>,
    pub body: Box<Expr>,
    /// Whether this is an async for-await: `for await x in stream { ... }`
    pub is_async: bool,
}

/// List comprehension expression
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListComprehension {
    /// The expression to evaluate for each element
    pub element: Box<Expr>,
    /// The comprehension clauses (for loops and filters)
    pub clauses: Vec<ComprehensionClause>,
}

/// A clause in a list comprehension
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComprehensionClause {
    /// The pattern to bind values to
    pub pattern: DestructurePattern,
    /// The iterable expression
    pub iterable: Box<Expr>,
    /// Optional filter expression (if clause)
    pub filter: Option<Box<Expr>>,
}

/// Loop expression (infinite loop with break)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoopExpr {
    pub body: Box<Expr>,
}

/// Let binding expression
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LetExpr {
    pub pattern: Pattern,
    pub type_annotation: Option<TypeAnnotation>,
    pub value: Option<Box<Expr>>,
    pub body: Box<Expr>, // The scope where the binding is valid
}

/// Assignment expression that returns the assigned value
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssignExpr {
    pub target: Box<Expr>, // Can be identifier, property access, index
    pub value: Box<Expr>,
}

/// Match expression
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MatchExpr {
    pub scrutinee: Box<Expr>,
    pub arms: Vec<MatchArm>,
}

/// Match arm
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MatchArm {
    pub pattern: Pattern,
    pub guard: Option<Box<Expr>>,
    pub body: Box<Expr>,
    /// Span of the pattern portion (for error reporting)
    pub pattern_span: Option<super::Span>,
}

/// LINQ-style from query expression
/// Syntax: from var in source [clauses...] select expr
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FromQueryExpr {
    /// Loop variable name (e.g., "t" in "from t in trades")
    pub variable: String,
    /// Source expression (e.g., "trades")
    pub source: Box<Expr>,
    /// Query clauses (where, order by, group by, join, let)
    pub clauses: Vec<QueryClause>,
    /// Final select expression
    pub select: Box<Expr>,
}

/// Query clause in a from expression
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum QueryClause {
    /// where condition
    Where(Box<Expr>),

    /// order by key asc/desc, key2 asc/desc, ...
    OrderBy(Vec<OrderBySpec>),

    /// group element by key into variable
    GroupBy {
        element: Box<Expr>,
        key: Box<Expr>,
        into_var: Option<String>,
    },

    /// join var in source on leftKey equals rightKey into var
    Join {
        variable: String,
        source: Box<Expr>,
        left_key: Box<Expr>,
        right_key: Box<Expr>,
        into_var: Option<String>,
    },

    /// let var = expr
    Let { variable: String, value: Box<Expr> },
}

/// Order specification for order by clause
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OrderBySpec {
    pub key: Box<Expr>,
    pub descending: bool,
}

/// Join strategy for async join expressions
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum JoinKind {
    /// Wait for all branches to complete, return tuple of results
    All = 0,
    /// Return the first branch to complete, cancel the rest
    Race = 1,
    /// Return the first branch to succeed (non-error), cancel the rest
    Any = 2,
    /// Wait for all branches, preserve individual success/error results
    Settle = 3,
}

/// A branch in a join expression
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JoinBranch {
    /// Optional label for named branches: `prices: fetch_prices("AAPL")`
    pub label: Option<String>,
    /// The expression to evaluate in this branch
    pub expr: Expr,
    /// Per-branch annotations: `@node(find_node("us-east")) compute_a()`
    pub annotations: Vec<Annotation>,
}

/// Async let expression: `async let name = expr`
/// Spawns a task and binds a future handle to a local variable.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AsyncLetExpr {
    /// The variable name to bind the future handle to
    pub name: String,
    /// The expression to spawn as an async task
    pub expr: Box<Expr>,
    /// Span covering the entire async let expression
    pub span: Span,
}

/// Join expression: `join all|race|any|settle { branch, ... }`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JoinExpr {
    /// The join strategy
    pub kind: JoinKind,
    /// The branches to execute concurrently
    pub branches: Vec<JoinBranch>,
    /// Span covering the entire join expression
    pub span: Span,
}

/// Compile-time for loop: `comptime for field in target.fields { ... }`
/// Unrolled at compile time — each iteration generates code with the loop variable
/// substituted for the concrete field descriptor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComptimeForExpr {
    /// Loop variable name (e.g., "field")
    pub variable: String,
    /// The iterable expression (e.g., `target.fields`)
    pub iterable: Box<Expr>,
    /// Body statements to unroll for each iteration
    pub body: Vec<super::statements::Statement>,
}
