//! Abstract Syntax Tree definitions for Shape
//!
//! This module defines the complete AST for the Shape language,
//! supporting all features from the language specification.

// Declare submodules
pub mod data_refs;
pub mod data_sources;
pub mod docs;
pub mod expr_helpers;
pub mod expressions;
pub mod functions;
pub mod joins;
pub mod literals;
pub mod modules;
pub mod operators;
pub mod patterns;
pub mod program;
pub mod queries;
pub mod span;
pub mod statements;
pub mod streams;
pub mod tests;
pub mod time;
pub mod type_path;
pub mod types;
pub mod windows;

// Re-export all public types for backwards compatibility
// This ensures that code using `use crate::ast::TypeName` continues to work

// From span.rs
pub use span::{Span, Spanned};

// From literals.rs
pub use literals::{Duration, DurationUnit, InterpolationMode, Literal};

// From operators.rs
pub use operators::{BinaryOp, RangeKind, UnaryOp};

// From time.rs
pub use time::{
    DateTimeExpr, NamedTime, RelativeTime, TimeDirection, TimeReference, TimeUnit, TimeWindow,
    Timeframe, TimeframeUnit,
};

// From data_refs.rs
pub use data_refs::{DataDateTimeRef, DataIndex, DataRef};

// From docs.rs
pub use docs::{
    DocComment, DocEntry, DocLink, DocTag, DocTagKind, DocTarget, DocTargetKind, ProgramDocs,
    extend_method_doc_path, impl_method_doc_path, qualify_doc_owner_path, type_annotation_doc_path,
    type_name_doc_path,
};

// From type_path.rs
pub use type_path::TypePath;

// From types.rs
pub use types::{
    EnumDef, EnumMember, EnumMemberKind, EnumValue, ExtendStatement, FunctionParam, ImplBlock,
    InterfaceDef, InterfaceMember, MethodDef, NativeLayoutBinding, ObjectTypeField, StructField,
    StructTypeDef, TraitDef, TraitMember, TypeAliasDef, TypeAnnotation, TypeName, TypeParam,
};

// From patterns.rs
pub use patterns::{
    DecompositionBinding, DestructurePattern, ObjectPatternField, Pattern,
    PatternConstructorFields, SweepParam,
};

// From expressions.rs
pub use expressions::{EnumConstructorPayload, Expr, ObjectEntry};

// From expr_helpers.rs
pub use expr_helpers::{
    AssignExpr, AsyncLetExpr, BlockExpr, BlockItem, ComprehensionClause, ComptimeForExpr, ForExpr,
    FromQueryExpr, IfExpr, JoinBranch, JoinExpr, JoinKind, LetExpr, ListComprehension, LoopExpr,
    MatchArm, MatchExpr, OrderBySpec, QueryClause, WhileExpr,
};

// From statements.rs
pub use statements::{Block, ForInit, ForLoop, IfStatement, Statement, WhileLoop};

// From functions.rs
pub use functions::{
    Annotation, AnnotationDef, AnnotationHandler, AnnotationHandlerParam, AnnotationHandlerType,
    AnnotationTargetKind, ForeignFunctionDef, FunctionDef, FunctionParameter, NativeAbiBinding,
};

// From modules.rs
pub use modules::{
    ExportItem, ExportSpec, ExportStmt, ImportItems, ImportSpec, ImportStmt, ModuleDecl,
};

// From queries.rs
pub use queries::{AlertQuery, BacktestQuery, Cte, Query, QueryModifiers, WithQuery};

// From joins.rs
pub use joins::{JoinClause, JoinCondition, JoinSource, JoinType};

// From windows.rs
pub use windows::{
    OrderByClause, SortDirection, WindowBound, WindowExpr, WindowFrame, WindowFrameType,
    WindowFunction, WindowSpec,
};

// From streams.rs
pub use streams::{StreamConfig, StreamDef, StreamOnError, StreamOnEvent, StreamOnWindow};

// From tests.rs
pub use tests::{
    AssertStatement, ExpectStatement, ExpectationMatcher, ShouldMatcher, ShouldStatement, TestCase,
    TestDef, TestFixture, TestMatchOptions, TestStatement,
};

// From data_sources.rs
pub use data_sources::{DataSourceDecl, QueryDecl};

// From program.rs
pub use program::{
    Assignment, BuiltinFunctionDecl, BuiltinTypeDecl, Item, OptimizationMetric, OptimizeStatement,
    OwnershipModifier, Program, VarKind, VariableDecl,
};
