//! Shared helpers for MIR lowering.
//!
//! Contains:
//! - Generic store emission (`emit_container_store_if_needed`)
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

/// W12-enum-constructor-mir-lowering (Phase 3 cluster-0 Round 4):
/// Identify a bare-form built-in enum-variant constructor name.
///
/// The parser produces `Expr::FunctionCall { name: "Ok" / "Err" / "Some",
/// ... }` for the bare surface forms `Ok(x)` / `Err(x)` / `Some(x)` —
/// indistinguishable in AST shape from any other function call. These
/// names are NOT registered in the runtime function table; the VM
/// intercepts them at bytecode-compile time via `classify_builtin_function`
/// (`crates/shape-vm/src/compiler/helpers.rs:3194-3209`) and dispatches
/// them via `OpCode::BuiltinCall(OkCtor / ErrCtor / SomeCtor)` to the
/// executor's hand-written constructor bodies
/// (`executor/vm_impl/builtins.rs:529-586`).
///
/// MIR emission must perform the equivalent producer-side classification
/// at the MIR-lowering layer, otherwise it leaks
/// `MirConstant::Function("Ok")` operands into MIR — operands the JIT
/// cannot resolve through its function index table, producing
/// `iconst(I64, 0)` callee bits and segfaulting downstream of
/// `jit_call_value`'s graceful surface-and-stop.
///
/// The list here mirrors the enum-variant subset of
/// `classify_builtin_function`. The collection-constructor subset
/// (`HashMap`, `Set`, `Deque`, `PriorityQueue`, `Channel`, `Mutex`,
/// `Atomic`, `Lazy`) shares the same MIR-emission failure mode but
/// targets a different downstream JIT consumer (empty-collection FFI),
/// is not load-bearing for any current cluster-0 smoke, and is tracked
/// as the follow-up sub-cluster
/// `W12-collection-constructor-mir-lowering`. See
/// `docs/cluster-audits/w12-enum-constructor-audit.md` §5.3 for the
/// scope decision.
///
/// `None` is not listed: the parser emits it as `Literal::None`, which
/// lowers to `MirConstant::None` directly — not a constructor surface.
pub(super) fn is_bare_enum_variant_ctor(name: &str) -> bool {
    matches!(name, "Ok" | "Err" | "Some")
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
