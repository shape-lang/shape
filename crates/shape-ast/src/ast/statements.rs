//! Statement types for Shape AST

use serde::{Deserialize, Serialize};

use super::expressions::Expr;
use super::program::{Assignment, VariableDecl};
use super::span::Span;
use super::types::{ExtendStatement, TypeAnnotation};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Statement {
    /// Return statement
    Return(Option<Expr>, Span),
    /// Break statement
    Break(Span),
    /// Continue statement
    Continue(Span),
    /// Variable declaration
    VariableDecl(VariableDecl, Span),
    /// Assignment
    Assignment(Assignment, Span),
    /// Expression statement
    Expression(Expr, Span),
    /// For loop
    For(ForLoop, Span),
    /// While loop
    While(WhileLoop, Span),
    /// If statement
    If(IfStatement, Span),
    /// Comptime-only type extension directive inside comptime handlers/blocks.
    Extend(ExtendStatement, Span),
    /// Comptime-only directive to remove the current annotation target.
    RemoveTarget(Span),
    /// Comptime-only directive to set a function parameter type.
    SetParamType {
        param_name: String,
        type_annotation: TypeAnnotation,
        span: Span,
    },
    /// Comptime-only directive to set a function return type.
    SetReturnType {
        type_annotation: TypeAnnotation,
        span: Span,
    },
    /// Comptime-only directive to set a function return type from an expression
    /// evaluated in comptime context.
    SetReturnExpr { expression: Expr, span: Span },
    /// Comptime-only directive to replace a function body.
    ReplaceBody { body: Vec<Statement>, span: Span },
    /// Comptime-only directive to replace a function body from an expression
    /// evaluated in comptime context.
    ReplaceBodyExpr { expression: Expr, span: Span },
    /// Comptime-only directive to replace a module body from an expression
    /// evaluated in comptime context.
    ReplaceModuleExpr { expression: Expr, span: Span },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ForLoop {
    /// Loop variable or initialization
    pub init: ForInit,
    /// Loop body
    pub body: Vec<Statement>,
    /// Whether this is an async for-await: `for await x in stream { ... }`
    pub is_async: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ForInit {
    /// for x in expr (or destructuring: for {x, y} in expr)
    ForIn {
        pattern: super::patterns::DestructurePattern,
        iter: Expr,
    },
    /// for (let i = 0; i < 10; i++)
    ForC {
        init: Box<Statement>,
        condition: Expr,
        update: Expr,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WhileLoop {
    pub condition: Expr,
    pub body: Vec<Statement>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IfStatement {
    pub condition: Expr,
    pub then_body: Vec<Statement>,
    pub else_body: Option<Vec<Statement>>,
}

/// Block is a sequence of statements (used in AST extensions)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Block {
    pub statements: Vec<Statement>,
}
