//! Shared helpers for MIR lowering.
//!
//! Contains:
//! - Generic store emission (`emit_container_store_if_needed`)
//! - Operand collection helpers (`collect_operands`, `collect_named_operands`)
//! - Task boundary emission
//! - Place projection utilities
//! - Type inference from expressions

use super::MirBuilder;
use crate::mir::types::*;
use shape_ast::ast::{self, Expr, Span};

// ---------------------------------------------------------------------------
// Generic container store emission
// ---------------------------------------------------------------------------

/// The kind of container a store is being emitted for.
///
/// Each variant maps to the corresponding `StatementKind`:
/// - `Array`   -> `StatementKind::ArrayStore`
/// - `Object`  -> `StatementKind::ObjectStore`
/// - `Enum`    -> `StatementKind::EnumStore`
/// - `Closure` -> `StatementKind::ClosureCapture`
#[derive(Debug, Clone, Copy)]
pub(super) enum ContainerStoreKind {
    Array,
    Object,
    Enum,
    Closure,
}

/// Emit a container-store statement if `operands` is non-empty.
///
/// This replaces the four near-identical `emit_*_store_if_needed()` helpers
/// that previously existed for arrays, objects, enums, and closures.
pub(super) fn emit_container_store_if_needed(
    builder: &mut MirBuilder,
    kind: ContainerStoreKind,
    container_slot: SlotId,
    operands: Vec<Operand>,
    span: Span,
) {
    emit_container_store_with_names(builder, kind, container_slot, operands, Vec::new(), span);
}

/// Like `emit_container_store_if_needed` but carries optional field names
/// for ObjectStore. `field_names` is ignored for non-Object kinds.
pub(super) fn emit_container_store_with_names(
    builder: &mut MirBuilder,
    kind: ContainerStoreKind,
    container_slot: SlotId,
    operands: Vec<Operand>,
    field_names: Vec<String>,
    span: Span,
) {
    if operands.is_empty() {
        return;
    }
    let stmt_kind = match kind {
        ContainerStoreKind::Array => StatementKind::ArrayStore {
            container_slot,
            operands,
        },
        ContainerStoreKind::Object => StatementKind::ObjectStore {
            container_slot,
            operands,
            field_names,
        },
        ContainerStoreKind::Enum => StatementKind::EnumStore {
            container_slot,
            operands,
        },
        ContainerStoreKind::Closure => StatementKind::ClosureCapture {
            closure_slot: container_slot,
            operands,
            function_id: None, // Patched after bytecode compilation
        },
    };
    builder.push_stmt(stmt_kind, span);
}

// ---------------------------------------------------------------------------
// Task boundary emission
// ---------------------------------------------------------------------------

pub(super) fn emit_task_boundary_if_needed(
    builder: &mut MirBuilder,
    operands: Vec<Operand>,
    span: Span,
) {
    if operands.is_empty() {
        return;
    }
    let kind = if builder.async_scope_depth > 0 {
        TaskBoundaryKind::Structured
    } else {
        TaskBoundaryKind::Detached
    };
    builder.push_stmt(StatementKind::TaskBoundary(operands, kind), span);
}

// ---------------------------------------------------------------------------
// Operand collection
// ---------------------------------------------------------------------------

/// Collect operands by lowering each expression through `lower_fn`.
///
/// This consolidates the repeated pattern of:
/// ```ignore
/// let operands: Vec<_> = exprs.iter().map(|e| lower_as_moved(builder, e)).collect();
/// ```
#[allow(dead_code)]
pub(super) fn collect_operands<'a>(
    builder: &mut MirBuilder,
    exprs: impl IntoIterator<Item = &'a Expr>,
    lower_fn: fn(&mut MirBuilder, &Expr) -> Operand,
) -> Vec<Operand> {
    exprs.into_iter().map(|e| lower_fn(builder, e)).collect()
}

/// Collect operands from named (key, expr) pairs by lowering only the expr.
#[allow(dead_code)]
pub(super) fn collect_named_operands<'a>(
    builder: &mut MirBuilder,
    named: impl IntoIterator<Item = &'a (String, Expr)>,
    lower_fn: fn(&mut MirBuilder, &Expr) -> Operand,
) -> Vec<Operand> {
    named.into_iter().map(|(_, e)| lower_fn(builder, e)).collect()
}

// ---------------------------------------------------------------------------
// Place projection utilities
// ---------------------------------------------------------------------------

pub(super) fn projected_field_place(
    builder: &mut MirBuilder,
    base: &Place,
    property: &str,
) -> Place {
    Place::Field(Box::new(base.clone()), builder.field_idx(property))
}

pub(super) fn projected_index_place(base: &Place, index: usize) -> Place {
    Place::Index(
        Box::new(base.clone()),
        Box::new(Operand::Constant(MirConstant::Int(index as i64))),
    )
}

// ---------------------------------------------------------------------------
// Common lowering utilities
// ---------------------------------------------------------------------------

pub(super) fn assign_none(builder: &mut MirBuilder, destination: SlotId, span: Span) {
    builder.push_stmt(
        StatementKind::Assign(
            Place::Local(destination),
            Rvalue::Use(Operand::Constant(MirConstant::None)),
        ),
        span,
    );
}

/// Assign a closure placeholder to a slot. Patched to Function(name) after bytecode compilation.
pub(super) fn assign_closure_placeholder(
    builder: &mut MirBuilder,
    destination: SlotId,
    span: Span,
) {
    builder.push_stmt(
        StatementKind::Assign(
            Place::Local(destination),
            Rvalue::Use(Operand::Constant(MirConstant::ClosurePlaceholder)),
        ),
        span,
    );
}

pub(super) fn assign_copy_from_place(
    builder: &mut MirBuilder,
    destination: SlotId,
    place: Place,
    span: Span,
) {
    builder.push_stmt(
        StatementKind::Assign(Place::Local(destination), Rvalue::Use(Operand::Copy(place))),
        span,
    );
}

pub(super) fn assign_copy_from_slot(
    builder: &mut MirBuilder,
    destination: SlotId,
    source: SlotId,
    span: Span,
) {
    assign_copy_from_place(builder, destination, Place::Local(source), span);
}

pub(super) fn start_dead_block(builder: &mut MirBuilder) {
    let dead_block = builder.new_block();
    builder.start_block(dead_block);
}

pub(super) fn infer_local_type_from_expr(expr: &Expr) -> LocalTypeInfo {
    match expr {
        Expr::Literal(literal, _) => match literal {
            ast::Literal::Int(_)
            | ast::Literal::UInt(_)
            | ast::Literal::TypedInt(_, _)
            | ast::Literal::Number(_)
            | ast::Literal::Decimal(_)
            | ast::Literal::Bool(_)
            | ast::Literal::Char(_)
            | ast::Literal::None
            | ast::Literal::Unit
            | ast::Literal::Timeframe(_) => LocalTypeInfo::Copy,
            ast::Literal::String(_)
            | ast::Literal::FormattedString { .. }
            | ast::Literal::ContentString { .. } => LocalTypeInfo::NonCopy,
        },
        Expr::Reference { .. } => LocalTypeInfo::NonCopy,
        _ => LocalTypeInfo::Unknown,
    }
}

pub(super) fn lower_binary_op(op: ast::BinaryOp) -> Option<BinOp> {
    match op {
        ast::BinaryOp::Add => Some(BinOp::Add),
        ast::BinaryOp::Sub => Some(BinOp::Sub),
        ast::BinaryOp::Mul => Some(BinOp::Mul),
        ast::BinaryOp::Div => Some(BinOp::Div),
        ast::BinaryOp::Mod => Some(BinOp::Mod),
        ast::BinaryOp::Greater => Some(BinOp::Gt),
        ast::BinaryOp::Less => Some(BinOp::Lt),
        ast::BinaryOp::GreaterEq => Some(BinOp::Ge),
        ast::BinaryOp::LessEq => Some(BinOp::Le),
        ast::BinaryOp::Equal => Some(BinOp::Eq),
        ast::BinaryOp::NotEqual => Some(BinOp::Ne),
        ast::BinaryOp::And => Some(BinOp::And),
        ast::BinaryOp::Or => Some(BinOp::Or),
        ast::BinaryOp::Pow
        | ast::BinaryOp::FuzzyEqual
        | ast::BinaryOp::FuzzyGreater
        | ast::BinaryOp::FuzzyLess
        | ast::BinaryOp::BitAnd
        | ast::BinaryOp::BitOr
        | ast::BinaryOp::BitXor
        | ast::BinaryOp::BitShl
        | ast::BinaryOp::BitShr
        | ast::BinaryOp::NullCoalesce
        | ast::BinaryOp::ErrorContext
        | ast::BinaryOp::Pipe => None,
    }
}

pub(super) fn lower_unary_op(op: ast::UnaryOp) -> Option<UnOp> {
    match op {
        ast::UnaryOp::Neg => Some(UnOp::Neg),
        ast::UnaryOp::Not => Some(UnOp::Not),
        ast::UnaryOp::BitNot => None,
    }
}

pub(super) fn operand_crosses_task_boundary(
    outer_locals_cutoff: u16,
    operand: &Operand,
) -> bool {
    match operand {
        Operand::Copy(place) | Operand::Move(place) | Operand::MoveExplicit(place) => {
            place.root_local().0 < outer_locals_cutoff
        }
        Operand::Constant(_) => false,
    }
}
