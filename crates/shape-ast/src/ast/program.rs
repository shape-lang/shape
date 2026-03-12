//! Program structure types for Shape AST

use serde::{Deserialize, Serialize};

use super::data_sources::{DataSourceDecl, QueryDecl};
use super::docs::DocComment;
use super::docs::ProgramDocs;
use super::expressions::Expr;
use super::functions::{AnnotationDef, ForeignFunctionDef, FunctionDef, FunctionParameter};
use super::modules::{ExportStmt, ImportStmt, ModuleDecl};
use super::patterns::DestructurePattern;
use super::queries::Query;
use super::span::Span;
use super::statements::Statement;
use super::streams::StreamDef;
use super::tests::TestDef;
use super::types::{
    EnumDef, ExtendStatement, ImplBlock, InterfaceDef, StructTypeDef, TraitDef, TypeAliasDef,
    TypeAnnotation, TypeParam,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Program {
    pub items: Vec<Item>,
    #[serde(default)]
    pub docs: ProgramDocs,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Item {
    /// Import statement
    Import(ImportStmt, Span),
    /// Export statement
    Export(ExportStmt, Span),
    /// Module definition
    Module(ModuleDecl, Span),
    /// Type alias definition
    TypeAlias(TypeAliasDef, Span),
    /// Interface definition
    Interface(InterfaceDef, Span),
    /// Trait definition (like interface but with `trait` keyword)
    Trait(TraitDef, Span),
    /// Enum definition
    Enum(EnumDef, Span),
    /// Type extension
    Extend(ExtendStatement, Span),
    /// Impl block (impl Trait for Type { ... })
    Impl(ImplBlock, Span),
    /// Function definition
    Function(FunctionDef, Span),
    /// Query
    Query(Query, Span),
    /// Variable declaration (let, var, const)
    VariableDecl(VariableDecl, Span),
    /// Variable assignment
    Assignment(Assignment, Span),
    /// Expression evaluation
    Expression(Expr, Span),
    /// Stream definition
    Stream(StreamDef, Span),
    /// Test definition
    Test(TestDef, Span),
    /// Optimize statement (Phase 3)
    Optimize(OptimizeStatement, Span),
    /// Annotation definition (annotation warmup(...) { ... })
    AnnotationDef(AnnotationDef, Span),
    /// Struct type definition (type Point { x: number, y: number })
    StructType(StructTypeDef, Span),
    /// Data source declaration (datasource Name: DataSource<T> = provider(...))
    DataSource(DataSourceDecl, Span),
    /// Query declaration (query Name: Query<T, Params> = sql(source, "..."))
    QueryDecl(QueryDecl, Span),
    /// Statement (treated as top-level code)
    Statement(Statement, Span),
    /// Compile-time block at top level: `comptime { stmts }`
    /// Executed during compilation; side effects only (result discarded).
    Comptime(Vec<Statement>, Span),
    /// Builtin type declaration (declaration-only intrinsic)
    BuiltinTypeDecl(BuiltinTypeDecl, Span),
    /// Builtin function declaration (declaration-only intrinsic)
    BuiltinFunctionDecl(BuiltinFunctionDecl, Span),
    /// Foreign function definition: `fn python analyze(data: DataTable) -> number { ... }`
    ForeignFunction(ForeignFunctionDef, Span),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VariableDecl {
    pub kind: VarKind,
    /// Explicit mutability: `let mut x = ...`
    /// When false with VarKind::Let, the binding is immutable (OwnedImmutable).
    /// When true with VarKind::Let, the binding is mutable (OwnedMutable).
    /// VarKind::Var always has flexible ownership: always mutable,
    /// function-scoped, with smart clone/move inference on initialization.
    #[serde(default)]
    pub is_mut: bool,
    pub pattern: DestructurePattern,
    pub type_annotation: Option<TypeAnnotation>,
    pub value: Option<Expr>,
    /// Explicit ownership modifier on the initializer: `let x = move y` or `let x = clone y`
    #[serde(default)]
    pub ownership: OwnershipModifier,
}

/// Explicit ownership transfer modifier on variable initialization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum OwnershipModifier {
    /// No explicit modifier — inferred from context.
    /// For `var`: smart inference (move if dead, clone if live).
    /// For `let`: always move.
    #[default]
    Inferred,
    /// `move` — explicitly force a move, invalidating the source.
    Move,
    /// `clone` — explicitly clone the source value.
    Clone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VarKind {
    Let,
    Var,
    Const,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Assignment {
    pub pattern: DestructurePattern,
    pub value: Expr,
}

/// Declaration-only intrinsic type in std/core metadata.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BuiltinTypeDecl {
    pub name: String,
    pub name_span: Span,
    #[serde(default)]
    pub doc_comment: Option<DocComment>,
    pub type_params: Option<Vec<TypeParam>>,
}

/// Declaration-only intrinsic function in std/core metadata.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BuiltinFunctionDecl {
    pub name: String,
    pub name_span: Span,
    #[serde(default)]
    pub doc_comment: Option<DocComment>,
    pub type_params: Option<Vec<TypeParam>>,
    pub params: Vec<FunctionParameter>,
    pub return_type: TypeAnnotation,
}

/// Optimization directive for parameter tuning
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OptimizeStatement {
    /// Parameter name to optimize
    pub parameter: String,
    /// Range for the parameter [min..max]
    pub range: (Box<Expr>, Box<Expr>),
    /// Metric to optimize for
    pub metric: OptimizationMetric,
}

/// Metrics that can be optimized
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum OptimizationMetric {
    Sharpe,
    Sortino,
    Return,
    Drawdown,
    WinRate,
    ProfitFactor,
    Custom(Box<Expr>),
}
