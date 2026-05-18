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
    emit_container_store_full(
        builder,
        kind,
        container_slot,
        operands,
        field_names,
        None,
        span,
    );
}

/// Emit a container-store statement carrying both optional field names
/// (for ObjectStore) and an optional variant name (for EnumStore — required
/// per ADR-006 §2.7.5 for the JIT EnumStore consumer to dispatch to the
/// right typed-Arc producer). Non-applicable kinds ignore the extra
/// metadata.
pub(super) fn emit_container_store_full(
    builder: &mut MirBuilder,
    kind: ContainerStoreKind,
    container_slot: SlotId,
    operands: Vec<Operand>,
    field_names: Vec<String>,
    variant_name: Option<String>,
    span: Span,
) {
    if operands.is_empty() {
        // Empty-operands container stores: most kinds short-circuit
        // because there's no per-element work to record (no array
        // element to push, no enum payload to wire, no closure capture
        // to box). The exception is `Object` for empty struct literals
        // (`let t = X {}`): the JIT-MIR consumer at
        // `crates/shape-jit/src/mir_compiler/statements.rs` relies on
        // the `StatementKind::ObjectStore` site to allocate the empty
        // TypedObject via `typed_object_alloc(schema_id, 0)`, and the
        // producer-side conduit at
        // `crates/shape-vm/src/compiler/helpers.rs::infer_top_level_
        // concrete_types_from_mir` keys its `Struct(StructLayoutId(0))`
        // stamp on `ObjectStore { container_slot, .. }` — without the
        // empty-operands ObjectStore, the slot's `ConcreteType` stays
        // `Void`, the JIT's `is_typed_object_slot` short-circuit at
        // `statements.rs:61` doesn't fire, and Aggregate falls through
        // to the kind-blind fallback (Phase 3 cluster-0 Round 11-trinity
        // Part c, 2026-05-13 — closes the kickoff Smoke 3 JIT-side
        // regression: `type X {}; let t = X {}` surfacing with
        // `Rvalue::Aggregate reached the kind-blind fallback`).
        //
        // ADR-006 §2.7.5 producing-site classification: the empty
        // ObjectStore IS the proof of struct-construction at MIR-emit
        // time. The downstream conduit walks ObjectStore unconditionally
        // (operand-count-independent), and the JIT consumer's empty-
        // operand `typed_object_alloc(schema_id, 0)` path already
        // handles the zero-field case correctly (loop body executes
        // zero times). No new MIR statement kind, no new dispatch shape,
        // no Bool-default fallback.
        //
        // Array / Enum / Closure containers preserve the short-circuit:
        // - `ArrayStore { ops: [] }` would have no v2 typed-array
        //   element kind source (the Round 5C `concrete_types[arr] =
        //   Array<scalar>` seed flows from element constant kinds, and
        //   empty arrays have no elements to classify) — the JIT's v2
        //   `emit_v2_array_aggregate` requires element kind, so empty
        //   arrays would surface anyway. The bytecode compiler emits
        //   `op_new_array` directly for empty literal arrays.
        // - `EnumStore { ops: [] }` is the unit-variant pattern
        //   (`None`, `Ok` of unit, user-enum unit variants) — handled
        //   by the Round 7A / Round 10 `EnumStore` consumer through the
        //   `variant_name` discriminator, no operand storage needed.
        //   The conduit walks EnumStore container slot regardless of
        //   operand count, so the stamp lands without the early-return
        //   guard.
        // - `ClosureCapture { ops: [] }` is a no-capture closure — no
        //   shared cell payload to wire.
        if !matches!(kind, ContainerStoreKind::Object) {
            return;
        }
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
            // ADR-006 §2.7.5 stamp-at-compile-time, Phase 3 cluster-0
            // Round 16 W17-narrow-follow-up-A: MIR lowering does not
            // have direct access to the bytecode compiler's
            // `type_tracker.schema_registry`. The schema_id is
            // back-patched by `crate::compiler::mir_schema_threading::
            // back_patch_schema_ids` after MIR lowering completes,
            // using the user-declared (or inline-anonymous) schema id
            // that matches the parallel bytecode-side
            // `OpCode::NewTypedObject` operand.
            schema_id: None,
        },
        ContainerStoreKind::Enum => StatementKind::EnumStore {
            container_slot,
            operands,
            variant_name,
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
        // W11-fup-A (Phase 3d, 2026-05-18): Pow + bitwise variants land in
        // MIR per the W11-jit-new-array close §4 Class A disposition. The
        // bytecode compiler emits the typed opcode flavours directly
        // (`PowInt`/`PowNumber`/`BitAndInt`/etc.) — the MIR `BinOp` here
        // mirrors the AST shape and lets the JIT consumer emit
        // Cranelift-native code (`compile_binop_int64` for i64 bitwise,
        // `jit_pow_f64` FFI for f64 Pow) instead of falling through to
        // the kind-blind `Rvalue::Aggregate(vec![l, r])` Route A SURFACE.
        ast::BinaryOp::Pow => Some(BinOp::Pow),
        ast::BinaryOp::BitAnd => Some(BinOp::BitAnd),
        ast::BinaryOp::BitOr => Some(BinOp::BitOr),
        ast::BinaryOp::BitXor => Some(BinOp::BitXor),
        ast::BinaryOp::BitShl => Some(BinOp::BitShl),
        ast::BinaryOp::BitShr => Some(BinOp::BitShr),
        ast::BinaryOp::Greater => Some(BinOp::Gt),
        ast::BinaryOp::Less => Some(BinOp::Lt),
        ast::BinaryOp::GreaterEq => Some(BinOp::Ge),
        ast::BinaryOp::LessEq => Some(BinOp::Le),
        ast::BinaryOp::Equal => Some(BinOp::Eq),
        ast::BinaryOp::NotEqual => Some(BinOp::Ne),
        ast::BinaryOp::And => Some(BinOp::And),
        ast::BinaryOp::Or => Some(BinOp::Or),
        ast::BinaryOp::FuzzyEqual
        | ast::BinaryOp::FuzzyGreater
        | ast::BinaryOp::FuzzyLess
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

/// W12-collection-constructor-mir-lowering (Phase 3 cluster-0 Round 6C):
/// Identify a bare-form built-in primitive-collection constructor name.
///
/// The parser produces `Expr::FunctionCall { name: "Set" / "HashMap" /
/// ..., ... }` for the bare surface forms `Set()` / `HashMap()` /
/// `Deque()` / `PriorityQueue()` / `Channel()` / `Mutex(x)` /
/// `Atomic(x)` / `Lazy(x)` — indistinguishable in AST shape from any
/// other function call. These names are NOT registered in the runtime
/// function table; the VM intercepts them at bytecode-compile time via
/// `classify_builtin_function` (`crates/shape-vm/src/compiler/helpers.rs:3427-3440`)
/// and dispatches them via `OpCode::BuiltinCall(HashMapCtor / SetCtor / ...)`
/// to the executor's hand-written constructor bodies
/// (`executor/vm_impl/builtins.rs:587-749`).
///
/// MIR emission must perform the equivalent producer-side classification
/// at the MIR-lowering layer, otherwise it leaks
/// `MirConstant::Function("Set")` operands into MIR — operands the JIT
/// cannot resolve through its function index table, producing
/// `iconst(I64, 0)` callee bits and downstream garbage values (the JIT-
/// side reads the null bits as int/float and prints garbage; the segfault
/// the §2.1 enum-variant family produced is replaced with a
/// silent-wrong-answer here because `.add()` etc. on null bits dispatches
/// to method handlers via PHF that interpret the null as a valid Arc).
///
/// The list here mirrors the collection-constructor subset of
/// `classify_builtin_function`. The bare enum-variant subset
/// (`Ok` / `Err` / `Some`) is handled separately by
/// `is_bare_enum_variant_ctor` — same producer-side classification, same
/// `EnumStore` MIR shape, different downstream JIT consumer surface
/// message (enum vs collection ctor).
///
/// All eight names share the same MIR shape — `Assign(Aggregate(operands))`
/// + `EnumStore { container_slot, operands, variant_name: Some(name) }` —
/// per the W12-enum-constructor audit's §5.3 "reuse `EnumStore` with
/// `kind`-on-the-slot threading" recommendation. The empty-args ctors
/// (Set/HashMap/Deque/PriorityQueue/Channel) emit `operands: []`; the
/// with-arg ctors (Mutex/Atomic/Lazy) emit `operands: [arg]`. The JIT
/// EnumStore consumer disambiguates by `variant_name` to surface a
/// collection-specific message (`docs/cluster-audits/w12-enum-
/// constructor-audit.md` §5.3, ADR-006 §2.7.5 producing-site
/// classification).
pub(super) fn is_bare_collection_ctor(name: &str) -> bool {
    matches!(
        name,
        "HashMap" | "Set" | "Deque" | "PriorityQueue" | "Channel" | "Mutex" | "Atomic" | "Lazy"
    )
}

/// True iff `name` is a bare-form collection ctor that takes one
/// argument (`Mutex(x)` / `Atomic(x)` / `Lazy(x)`). The remaining
/// collection ctors (`Set` / `HashMap` / `Deque` / `PriorityQueue` /
/// `Channel`) take zero arguments at landing; passing args produces a
/// VM-side argument-count error from the executor's ctor body
/// (`executor/vm_impl/builtins.rs:592` etc.), so the MIR rewrite must
/// not silently accept `Set(extra_arg)` via the pipe-operator path.
pub(super) fn is_bare_collection_ctor_with_arg(name: &str) -> bool {
    matches!(name, "Mutex" | "Atomic" | "Lazy")
}

/// W12-collection-constructor-mir-lowering: lower a bare-form collection
/// constructor to the `Aggregate` + `EnumStore` MIR shape per the
/// W12-enum-constructor audit's §5.3 recommendation.
///
/// Unlike the `emit_container_store_full` helper (which guards
/// `if operands.is_empty() { return; }` for the existing
/// Array/Object/Enum/Closure-store callers), the empty-args collection
/// ctors (`Set()` / `HashMap()` / `Deque()` / `PriorityQueue()` /
/// `Channel()`) must still emit the `EnumStore` so the JIT consumer
/// observes the collection-ctor surface (and bytecode-side borrow /
/// storage planning has a kind-source statement for the container slot).
/// The helper is bypassed here by emitting `StatementKind::EnumStore`
/// directly with the variant_name.
///
/// The `Assign(Aggregate(...))` is unconditional: with `operands: []`
/// it produces an empty aggregate that the JIT side recognizes as the
/// container-init form. The MIR borrow solver / liveness / storage
/// planner all treat empty-operands and non-empty-operands EnumStore
/// statements uniformly (`crates/shape-vm/src/mir/{solver,liveness,
/// storage_planning,field_analysis}.rs` — every match arm is on
/// `EnumStore { operands, .. }` with operand iteration that no-ops on
/// empty slices).
pub(super) fn emit_collection_ctor_store(
    builder: &mut MirBuilder,
    container_slot: SlotId,
    operands: Vec<Operand>,
    variant_name: String,
    span: Span,
) {
    builder.push_stmt(
        StatementKind::Assign(
            Place::Local(container_slot),
            Rvalue::Aggregate(operands.clone()),
        ),
        span,
    );
    builder.push_stmt(
        StatementKind::EnumStore {
            container_slot,
            operands,
            variant_name: Some(variant_name),
        },
        span,
    );
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
