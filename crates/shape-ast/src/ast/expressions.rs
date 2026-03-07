//! Core expression types for Shape AST

use serde::{Deserialize, Serialize};

use super::data_refs::{DataDateTimeRef, DataIndex, DataRef};
use super::literals::{Duration, Literal};
use super::operators::{BinaryOp, RangeKind, UnaryOp};
use super::span::{Span, Spanned};
use super::time::{DateTimeExpr, TimeReference, Timeframe};
use super::types::TypeAnnotation;

/// Entry in an object literal - either a field or a spread
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ObjectEntry {
    /// Regular field: key: value or key: Type = value
    Field {
        key: String,
        value: Expr,
        type_annotation: Option<TypeAnnotation>,
    },
    /// Spread: ...expr
    Spread(Expr),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum EnumConstructorPayload {
    Unit,
    Tuple(Vec<Expr>),
    Struct(Vec<(String, Expr)>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Expr {
    /// Literal values
    Literal(Literal, Span),
    /// Variable/identifier reference
    Identifier(String, Span),
    /// Data reference: data[0], data[-1], data[1:5]
    DataRef(DataRef, Span),
    /// DateTime-based data reference: data[@"2024-01-15"]
    DataDateTimeRef(DataDateTimeRef, Span),
    /// Relative access from a reference: ref[0], ref[-1]
    DataRelativeAccess {
        reference: Box<Expr>,
        index: DataIndex,
        span: Span,
    },
    /// Property access: expr.property or expr?.property
    PropertyAccess {
        object: Box<Expr>,
        property: String,
        optional: bool,
        span: Span,
    },
    /// Index access: expr[index] or expr[start:end]
    IndexAccess {
        object: Box<Expr>,
        index: Box<Expr>,
        end_index: Option<Box<Expr>>,
        span: Span,
    },
    /// Binary operations
    BinaryOp {
        left: Box<Expr>,
        op: BinaryOp,
        right: Box<Expr>,
        span: Span,
    },
    /// Fuzzy comparison with explicit tolerance: a ~= b within 0.02
    FuzzyComparison {
        left: Box<Expr>,
        op: super::operators::FuzzyOp,
        right: Box<Expr>,
        tolerance: super::operators::FuzzyTolerance,
        span: Span,
    },
    /// Unary operations
    UnaryOp {
        op: UnaryOp,
        operand: Box<Expr>,
        span: Span,
    },
    /// Function calls: sma(20), rsi(14), momentum(period: 10, threshold: 0.01)
    FunctionCall {
        name: String,
        args: Vec<Expr>,
        named_args: Vec<(String, Expr)>,
        span: Span,
    },
    /// Enum constructor: Enum::Variant, Enum::Variant(...), Enum::Variant { ... }
    EnumConstructor {
        enum_name: String,
        variant: String,
        payload: EnumConstructorPayload,
        span: Span,
    },
    /// Time reference: @today, @"2024-01-15"
    TimeRef(TimeReference, Span),
    /// DateTime expression: @"2024-01-15", @market_open, etc.
    DateTime(DateTimeExpr, Span),
    /// Pattern reference in expressions
    PatternRef(String, Span),
    /// Conditional expression: if cond then expr else expr
    Conditional {
        condition: Box<Expr>,
        then_expr: Box<Expr>,
        else_expr: Option<Box<Expr>>,
        span: Span,
    },
    /// Object literal: { field1: expr1, field2: expr2, ...spread }
    Object(Vec<ObjectEntry>, Span),
    /// Array literal: [1, 2, 3]
    Array(Vec<Expr>, Span),
    /// List comprehension: [expr for var in iter if cond]
    ListComprehension(Box<super::expr_helpers::ListComprehension>, Span),
    /// Block expression: { let x = 10; x + 5 }
    Block(super::expr_helpers::BlockExpr, Span),
    /// Type assertion: expr as Type or expr as Type { param: value }
    TypeAssertion {
        expr: Box<Expr>,
        type_annotation: TypeAnnotation,
        /// Meta parameter overrides: as Percent { decimals: 4 }
        meta_param_overrides: Option<std::collections::HashMap<String, Expr>>,
        span: Span,
    },
    /// Instance check: expr instanceof Type
    InstanceOf {
        expr: Box<Expr>,
        type_annotation: TypeAnnotation,
        span: Span,
    },
    /// Function expression: function(x, y) { return x + y }
    FunctionExpr {
        params: Vec<super::functions::FunctionParameter>,
        return_type: Option<TypeAnnotation>,
        body: Vec<super::statements::Statement>,
        span: Span,
    },
    /// Duration literal: 30d, 1h, 15m
    Duration(Duration, Span),
    /// Spread expression: ...expr (used in arrays and objects)
    Spread(Box<Expr>, Span),

    // ===== Expression-based Control Flow =====
    /// If expression: if condition { expr } else { expr }
    If(Box<super::expr_helpers::IfExpr>, Span),

    /// While expression: while condition { expr }
    While(Box<super::expr_helpers::WhileExpr>, Span),

    /// For expression: for x in iter { expr }
    For(Box<super::expr_helpers::ForExpr>, Span),

    /// Loop expression: loop { expr }
    Loop(Box<super::expr_helpers::LoopExpr>, Span),

    /// Let binding expression: let x = value; expr
    Let(Box<super::expr_helpers::LetExpr>, Span),

    /// Assignment expression: x = value (returns value)
    Assign(Box<super::expr_helpers::AssignExpr>, Span),

    /// Break with optional value
    Break(Option<Box<Expr>>, Span),

    /// Continue
    Continue(Span),

    /// Return with optional value
    Return(Option<Box<Expr>>, Span),

    /// Method call: expr.method(args) or expr.method(name: value)
    MethodCall {
        receiver: Box<Expr>,
        method: String,
        args: Vec<Expr>,
        named_args: Vec<(String, Expr)>,
        span: Span,
    },

    /// Match expression
    Match(Box<super::expr_helpers::MatchExpr>, Span),

    /// Unit value (void)
    Unit(Span),

    /// Range expression with Rust-style syntax
    Range {
        start: Option<Box<Expr>>,
        end: Option<Box<Expr>>,
        kind: RangeKind,
        span: Span,
    },

    /// Timeframe context expression: on(1h) { expr }
    TimeframeContext {
        timeframe: Timeframe,
        expr: Box<Expr>,
        span: Span,
    },

    /// Try operator for fallible propagation (Result/Option): expr?
    TryOperator(Box<Expr>, Span),

    /// Named implementation selector: `expr using ImplName`
    UsingImpl {
        expr: Box<Expr>,
        impl_name: String,
        span: Span,
    },

    /// Simulation call with inline parameters
    SimulationCall {
        name: String,
        params: Vec<(String, Expr)>,
        span: Span,
    },

    /// Window function expression: expr OVER (partition by ... order by ...)
    WindowExpr(Box<super::windows::WindowExpr>, Span),

    /// LINQ-style from query expression
    /// Syntax: from var in source [clauses...] select expr
    FromQuery(Box<super::expr_helpers::FromQueryExpr>, Span),

    /// Struct literal: TypeName { field: value, ... }
    StructLiteral {
        type_name: String,
        fields: Vec<(String, Expr)>,
        span: Span,
    },

    /// Await expression: await expr
    Await(Box<Expr>, Span),

    /// Join expression: await join all|race|any|settle { branch, ... }
    Join(Box<super::expr_helpers::JoinExpr>, Span),

    /// Annotated expression: @annotation expr
    /// Used for expression-level annotations like `@timeout(5s) fetch()` or `@timed computation()`
    /// Multiple annotations nest left-to-right: `@retry(3) @timeout(5s) fetch()` becomes
    /// `Annotated { @retry(3), target: Annotated { @timeout(5s), target: fetch() } }`
    Annotated {
        annotation: super::functions::Annotation,
        target: Box<Expr>,
        span: Span,
    },

    /// Async let expression: `async let name = expr`
    /// Spawns a task and binds a future handle to a local variable.
    AsyncLet(Box<super::expr_helpers::AsyncLetExpr>, Span),

    /// Async scope expression: `async scope { ... }`
    /// Cancellation boundary — on scope exit, all pending tasks are cancelled in reverse order.
    AsyncScope(Box<Expr>, Span),

    /// Compile-time block expression: `comptime { stmts }`
    /// Evaluated at compile time via the mini-VM. The result replaces this node with a literal.
    Comptime(Vec<super::statements::Statement>, Span),

    /// Compile-time for loop: `comptime for field in target.fields { ... }`
    /// Unrolled at compile time. Each iteration is compiled with the loop variable
    /// bound to the concrete field descriptor. Used inside comptime annotation handlers.
    ComptimeFor(Box<super::expr_helpers::ComptimeForExpr>, Span),

    /// Reference expression: `&expr` or `&mut expr`.
    /// Creates a shared or exclusive reference to a local variable.
    Reference {
        expr: Box<Expr>,
        /// True for `&mut expr` (exclusive/mutable reference).
        is_mutable: bool,
        span: Span,
    },

    /// Table row literal: `[a, b, c], [d, e, f]`
    /// Used with `let t: Table<T> = [row1], [row2], ...` syntax.
    /// Each inner Vec<Expr> is one row's positional field values.
    TableRows(Vec<Vec<Expr>>, Span),
}

impl Expr {
    /// Convert expression to a JSON value (for literals used in metadata)
    pub fn to_json_value(&self) -> serde_json::Value {
        match self {
            Expr::Literal(lit, _) => lit.to_json_value(),
            Expr::Array(elements, _) => {
                serde_json::Value::Array(elements.iter().map(|e| e.to_json_value()).collect())
            }
            Expr::Object(entries, _) => {
                let mut map = serde_json::Map::new();
                for entry in entries {
                    if let ObjectEntry::Field { key, value, .. } = entry {
                        map.insert(key.clone(), value.to_json_value());
                    }
                }
                serde_json::Value::Object(map)
            }
            _ => serde_json::Value::Null, // Fallback for non-literals
        }
    }
}

impl Spanned for Expr {
    fn span(&self) -> Span {
        match self {
            Expr::Literal(_, span) => *span,
            Expr::Identifier(_, span) => *span,
            Expr::DataRef(_, span) => *span,
            Expr::DataDateTimeRef(_, span) => *span,
            Expr::DataRelativeAccess { span, .. } => *span,
            Expr::PropertyAccess { span, .. } => *span,
            Expr::IndexAccess { span, .. } => *span,
            Expr::BinaryOp { span, .. } => *span,
            Expr::FuzzyComparison { span, .. } => *span,
            Expr::UnaryOp { span, .. } => *span,
            Expr::FunctionCall { span, .. } => *span,
            Expr::EnumConstructor { span, .. } => *span,
            Expr::TimeRef(_, span) => *span,
            Expr::DateTime(_, span) => *span,
            Expr::PatternRef(_, span) => *span,
            Expr::Conditional { span, .. } => *span,
            Expr::Object(_, span) => *span,
            Expr::Array(_, span) => *span,
            Expr::ListComprehension(_, span) => *span,
            Expr::Block(_, span) => *span,
            Expr::TypeAssertion { span, .. } => *span,
            Expr::InstanceOf { span, .. } => *span,
            Expr::FunctionExpr { span, .. } => *span,
            Expr::Duration(_, span) => *span,
            Expr::Spread(_, span) => *span,
            Expr::If(_, span) => *span,
            Expr::While(_, span) => *span,
            Expr::For(_, span) => *span,
            Expr::Loop(_, span) => *span,
            Expr::Let(_, span) => *span,
            Expr::Assign(_, span) => *span,
            Expr::Break(_, span) => *span,
            Expr::Continue(span) => *span,
            Expr::Return(_, span) => *span,
            Expr::MethodCall { span, .. } => *span,
            Expr::Match(_, span) => *span,
            Expr::Unit(span) => *span,
            Expr::Range { span, .. } => *span,
            Expr::TimeframeContext { span, .. } => *span,
            Expr::TryOperator(_, span) => *span,
            Expr::UsingImpl { span, .. } => *span,
            Expr::SimulationCall { span, .. } => *span,
            Expr::WindowExpr(_, span) => *span,
            Expr::FromQuery(_, span) => *span,
            Expr::StructLiteral { span, .. } => *span,
            Expr::Await(_, span) => *span,
            Expr::Join(_, span) => *span,
            Expr::Annotated { span, .. } => *span,
            Expr::AsyncLet(_, span) => *span,
            Expr::AsyncScope(_, span) => *span,
            Expr::Comptime(_, span) => *span,
            Expr::ComptimeFor(_, span) => *span,
            Expr::Reference { span, .. } => *span,
            Expr::TableRows(_, span) => *span,
        }
    }
}
