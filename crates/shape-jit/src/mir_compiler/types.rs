//! Type mapping for MIR-to-Cranelift IR compilation.
//!
//! Maps MIR LocalTypeInfo and NativeKind to Cranelift types.
//! Includes MIR-level type inference for determining slot kinds
//! when the bytecode compiler doesn't provide them.

use cranelift::prelude::types;
use shape_value::v2::ConcreteType;
use shape_vm::mir::types::*;
use shape_vm::type_tracking::NativeKind;

/// Whether a local slot holds a heap value that needs reference counting.
pub(crate) fn is_heap_type(type_info: &LocalTypeInfo) -> bool {
    matches!(type_info, LocalTypeInfo::NonCopy)
}

/// Whether a local slot is known to be Copy (no refcounting needed).
pub(crate) fn is_copy_type(type_info: &LocalTypeInfo) -> bool {
    matches!(type_info, LocalTypeInfo::Copy)
}

/// Get the NativeKind for a local. Returns `None` when the slot
/// index is out of range OR the inference pass left the slot
/// undetermined.
///
/// Per ADR-006 §2.7.7, the deleted `NativeKind::Unknown` placeholder
/// is forbidden in the runtime parallel-kind track. This compile-time
/// helper is a different layer (compile-time inference metadata, not
/// the runtime track), but it adopts the same single-discriminator
/// discipline by returning `Option<NativeKind>` rather than papering
/// over the missing-kind case.
pub(crate) fn slot_kind_for_local(
    slot_kinds: &[Option<NativeKind>],
    slot_idx: u16,
) -> Option<NativeKind> {
    slot_kinds.get(slot_idx as usize).copied().flatten()
}

/// Whether a NativeKind is i32 (Int32 or UInt32).
pub(crate) fn is_i32_slot(kind: NativeKind) -> bool {
    matches!(kind, NativeKind::Int32 | NativeKind::UInt32)
}

/// Whether a NativeKind represents a native (non-NaN-boxed) Cranelift type.
#[allow(dead_code)]
pub(crate) fn is_native_slot(kind: NativeKind) -> bool {
    matches!(
        kind,
        NativeKind::Float64
            | NativeKind::Int32
            | NativeKind::UInt32
            | NativeKind::Bool
            | NativeKind::Int8
            | NativeKind::UInt8
            | NativeKind::Int16
            | NativeKind::UInt16
    )
}

/// Map a NativeKind to its Cranelift type.
/// Native numeric types get their natural width; everything else is I64.
pub(crate) fn cranelift_type_for_slot(kind: NativeKind) -> cranelift::prelude::Type {
    match kind {
        NativeKind::Float64 => types::F64,
        NativeKind::Int32 | NativeKind::UInt32 => types::I32,
        NativeKind::Int8 | NativeKind::UInt8 | NativeKind::Bool => types::I8,
        NativeKind::Int16 | NativeKind::UInt16 => types::I16,
        // Int64, UInt64, String, Ptr(_), Nullable*, IntSize, UIntSize:
        // 8-byte raw u64 (typed pointer for heap arms, scalar for ints).
        _ => types::I64,
    }
}

/// Whether a NativeKind is a v2 heap pointer type (TypedArray, TypedStruct, StringObj).
/// These use inline refcounting via HeapHeader at offset 0.
pub(crate) fn is_v2_heap_slot(kind: NativeKind) -> bool {
    let _ = kind;
    false
}

/// Map a `ConcreteType` element type to the matching `NativeKind` for the v2
/// typed-array codegen helpers (`v2_array_get`/`v2_array_set`).
pub(crate) fn elem_slot_kind_for_concrete(elem: &ConcreteType) -> Option<NativeKind> {
    match elem {
        ConcreteType::F64 => Some(NativeKind::Float64),
        ConcreteType::I64 => Some(NativeKind::Int64),
        ConcreteType::I32 => Some(NativeKind::Int32),
        ConcreteType::I16 => Some(NativeKind::Int16),
        ConcreteType::I8 => Some(NativeKind::Int8),
        ConcreteType::U64 => Some(NativeKind::UInt64),
        ConcreteType::U32 => Some(NativeKind::UInt32),
        ConcreteType::U16 => Some(NativeKind::UInt16),
        ConcreteType::U8 => Some(NativeKind::UInt8),
        ConcreteType::Bool => Some(NativeKind::Bool),
        _ => None,
    }
}

/// Inspect a slot's `ConcreteType` and report the v2 typed-array element kind
/// when the slot is known to hold an `Array<T>` whose element type maps to a
/// scalar Cranelift load/store. Returns `None` for unknown / non-array /
/// non-scalar slots — caller falls back to legacy NaN-boxed path.
pub(crate) fn is_v2_typed_array_slot(
    concrete_types: &[ConcreteType],
    slot_idx: u16,
) -> Option<NativeKind> {
    let ct = concrete_types.get(slot_idx as usize)?;
    match ct {
        ConcreteType::Array(elem) => elem_slot_kind_for_concrete(elem),
        _ => None,
    }
}

// ── MIR-level type inference ────────────────────────────────────────────

/// Infer SlotKinds from MIR constants and operations.
///
/// Scans all basic blocks forward and tracks what types flow into each slot.
/// When the bytecode compiler doesn't provide slot_kinds (empty vec),
/// this pass fills them in from MIR-observable information.
///
/// Returns a `Vec<Option<NativeKind>>`: `Some(k)` for slots whose kind
/// the inference proved, `None` for slots the inference left
/// undetermined (e.g. opaque field reads, or parameters with no
/// kind-source). Per ADR-006 §2.7.7 we use `None` rather than the
/// deleted `NativeKind::Unknown` placeholder — callers that need a
/// concrete kind for codegen surface-and-stop on `None`.
///
/// Rules:
/// - Assign(slot, Use(Constant(Float(_)))) → Float64
/// - Assign(slot, Use(Constant(Int(_)))) → Int64 (NaN-boxed int uses 48-bit payload)
/// - Assign(slot, Use(Constant(Bool(_)))) → Bool
/// - Assign(slot, BinaryOp(arith, lhs, rhs)) → inherits from operands if both agree
/// - Assign(slot, Use(Move/Copy(other_slot))) → inherits from other_slot
/// - Conflicting assignments → keep existing
pub(crate) fn infer_slot_kinds(
    mir: &MirFunction,
    existing: &[Option<NativeKind>],
) -> Vec<Option<NativeKind>> {
    let n = mir.num_locals as usize;
    let mut kinds: Vec<Option<NativeKind>> = vec![None; n];

    // Seed from existing slot_kinds (from bytecode compiler).
    for (i, &k) in existing.iter().enumerate() {
        if i < n && k.is_some() {
            kinds[i] = k;
        }
    }

    // Forward pass: infer from constants and operations.
    for block in &mir.blocks {
        for stmt in &block.statements {
            match &stmt.kind {
                StatementKind::Assign(place, rvalue) => {
                    if let Place::Local(slot) = place {
                        let idx = slot.0 as usize;
                        if idx < n && kinds[idx].is_none() {
                            if let Some(inferred) = infer_rvalue_kind(rvalue, &kinds) {
                                kinds[idx] = Some(inferred);
                            }
                        } else if idx < n {
                            // Slot already has a kind — check for conflicts.
                            if let Some(inferred) = infer_rvalue_kind(rvalue, &kinds) {
                                if Some(inferred) != kinds[idx] {
                                    // Conflict: different types on different paths.
                                    // Keep the existing kind (first write wins for
                                    // simple programs; SSA form means each slot is
                                    // typically written once in practice).
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    // F7.c — build the set of "opaque-source" slots: slots whose Rvalue
    // reads from a heap projection (`Field` / `Index`) or another
    // non-trivial source (calls, borrows, aggregates). The runtime value
    // of such a slot is determined by the projection — its Cranelift
    // width is not guaranteed to match anything derivable from later uses.
    //
    // Example: `for i in 0..arr.length { ... }` lowers the `arr.length`
    // read to `Assign(SlotId(4), Use(Copy(Field(Local(1), FieldIdx(0)))))`.
    // The backward pass below would otherwise see `SlotId(5) < SlotId(4)`
    // with `SlotId(5): Int64`, conclude `SlotId(4)` is also `Int64`, and
    // the `compile_binop_int64` fast path would then unpack the
    // `box_number(f64)` bits as a TAG_INT payload — silently reading 0
    // from an f64 `4.0` and making the loop skip every iteration.
    //
    // By excluding these slots from backward propagation, the comparison
    // falls back to `compile_binop_dynamic_cmp`, which traps on a true
    // mixed-tag operand pair (deopt) — but in the common case where the
    // field happens to carry a number (e.g. `arr.length` returns
    // `box_number(len as f64)`), the `both_num` path fires correctly by
    // inspecting the tag bits at runtime rather than trusting an
    // unsound compile-time inference.
    let mut opaque_slots: std::collections::HashSet<usize> = std::collections::HashSet::new();
    for block in &mir.blocks {
        for stmt in &block.statements {
            if let StatementKind::Assign(Place::Local(slot), rvalue) = &stmt.kind {
                let opaque = match rvalue {
                    Rvalue::Use(operand) => is_opaque_operand(operand),
                    // Binary / unary / clone / borrow / aggregate: their
                    // result type comes from the compiler's inference, not
                    // from the destination slot's later uses. We only care
                    // about bare projections here — `Use(Copy(Field))` is
                    // the canonical case.
                    _ => false,
                };
                if opaque {
                    opaque_slots.insert(slot.0 as usize);
                }
            }
        }
    }

    // Backward pass: propagate types from typed operands to Unknown slots
    // used as the other operand in a binop. This picks up closure-param slots
    // like `x` in `|x| x + 1`, where the forward pass leaves `x` Unknown because
    // closure params are registered without a type annotation, but the typed
    // constant `1` proves `x` is Int64.
    //
    // Iterate to a fixed point — at most `n` rounds — so chained inferences
    // propagate (e.g. `|x, y| x + y + 1` should flow Int64 from `1` through
    // both params).
    let mut changed = true;
    let mut rounds = 0;
    while changed && rounds < n {
        changed = false;
        rounds += 1;
        for block in &mir.blocks {
            for stmt in &block.statements {
                if let StatementKind::Assign(_, Rvalue::BinaryOp(op, lhs, rhs)) = &stmt.kind {
                    // Comparisons don't constrain the operands' kinds beyond
                    // "both must match" — and the producing slot becomes Bool,
                    // not the operand kind. Still useful for propagating
                    // operand kinds between each other.
                    let _ = op;
                    let lk = infer_operand_kind(lhs, &kinds);
                    let rk = infer_operand_kind(rhs, &kinds);
                    match (lk, rk) {
                        (Some(k), None) => {
                            if let Some(slot) = operand_local_slot(rhs) {
                                if !opaque_slots.contains(&slot)
                                    && set_kind_if_unknown(&mut kinds, slot, k)
                                {
                                    changed = true;
                                }
                            }
                        }
                        (None, Some(k)) => {
                            if let Some(slot) = operand_local_slot(lhs) {
                                if !opaque_slots.contains(&slot)
                                    && set_kind_if_unknown(&mut kinds, slot, k)
                                {
                                    changed = true;
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    // Parameters keep their existing-from-bytecode kind if any.
    // Otherwise they remain `None` — callers needing a concrete
    // kind for codegen surface-and-stop on the `None` per ADR-006
    // §2.7.7 (no deleted `NativeKind::Unknown` placeholder).
    for &param_slot in &mir.param_slots {
        let idx = param_slot.0 as usize;
        if idx < n {
            if let Some(Some(k)) = existing.get(idx).copied() {
                kinds[idx] = Some(k);
            }
        }
    }

    kinds
}

/// F7.c — `true` when `operand` reads through a heap projection
/// (`Place::Field` / `Place::Index` / `Place::Deref`). The runtime type
/// of such a read is opaque to the compiler; backward type propagation
/// must not invent a `NativeKind` for the destination slot from unrelated
/// uses of that slot in later binops.
fn is_opaque_operand(operand: &Operand) -> bool {
    match operand {
        Operand::Copy(place) | Operand::Move(place) | Operand::MoveExplicit(place) => {
            is_opaque_place(place)
        }
        Operand::Constant(_) => false,
    }
}

/// Walk a `Place` — `true` if any projection in the chain is a field
/// read, index read, or deref. Pure `Place::Local` chains stay typed.
fn is_opaque_place(place: &Place) -> bool {
    match place {
        Place::Local(_) => false,
        Place::Field(_, _) | Place::Index(_, _) | Place::Deref(_) => true,
    }
}

/// If `operand` is a direct `Copy`/`Move` of a local, return the slot's index.
/// Only handles the simple `Place::Local` form — projections (field/index) do
/// not participate in the backward type propagation.
fn operand_local_slot(operand: &Operand) -> Option<usize> {
    match operand {
        Operand::Copy(Place::Local(slot))
        | Operand::Move(Place::Local(slot))
        | Operand::MoveExplicit(Place::Local(slot)) => Some(slot.0 as usize),
        _ => None,
    }
}

/// Set `kinds[idx] = Some(kind)` if the slot was previously
/// undetermined (`None`), returning `true` when an update happened.
fn set_kind_if_unknown(kinds: &mut [Option<NativeKind>], idx: usize, kind: NativeKind) -> bool {
    if idx < kinds.len() && kinds[idx].is_none() {
        kinds[idx] = Some(kind);
        true
    } else {
        false
    }
}

/// Infer the NativeKind produced by an Rvalue.
fn infer_rvalue_kind(rvalue: &Rvalue, kinds: &[Option<NativeKind>]) -> Option<NativeKind> {
    match rvalue {
        Rvalue::Use(operand) => infer_operand_kind(operand, kinds),
        Rvalue::BinaryOp(op, lhs, rhs) => {
            let lk = infer_operand_kind(lhs, kinds);
            let rk = infer_operand_kind(rhs, kinds);
            match (lk, rk) {
                (Some(l), Some(r)) if l == r => {
                    // Both operands same type.
                    // Arithmetic on floats → float, on ints → int.
                    // Comparisons always → Bool.
                    if is_comparison_op(op) {
                        Some(NativeKind::Bool)
                    } else {
                        Some(l)
                    }
                }
                _ => {
                    // Mixed or unknown operands. Comparison still → Bool.
                    if is_comparison_op(op) {
                        Some(NativeKind::Bool)
                    } else {
                        None
                    }
                }
            }
        }
        Rvalue::UnaryOp(UnOp::Neg, operand) => infer_operand_kind(operand, kinds),
        Rvalue::UnaryOp(UnOp::Not, _) => Some(NativeKind::Bool),
        Rvalue::Clone(operand) => infer_operand_kind(operand, kinds),
        Rvalue::Borrow(_, _) => None,     // References are heap pointers
        Rvalue::Aggregate(_) => None,      // Arrays are heap objects
    }
}

/// Infer the NativeKind of an operand.
fn infer_operand_kind(operand: &Operand, kinds: &[Option<NativeKind>]) -> Option<NativeKind> {
    match operand {
        Operand::Constant(c) => infer_constant_kind(c),
        Operand::Copy(place) | Operand::Move(place) | Operand::MoveExplicit(place) => {
            let slot = place.root_local();
            let idx = slot.0 as usize;
            kinds.get(idx).copied().flatten()
        }
    }
}

/// Infer the NativeKind of a constant.
fn infer_constant_kind(constant: &MirConstant) -> Option<NativeKind> {
    match constant {
        MirConstant::Float(_) => Some(NativeKind::Float64),
        MirConstant::Int(_) => Some(NativeKind::Int64),
        MirConstant::Bool(_) => Some(NativeKind::Bool),
        MirConstant::None => None,
        MirConstant::StringId(_) | MirConstant::Str(_) => Some(NativeKind::String),
        MirConstant::Function(_) | MirConstant::Method(_) | MirConstant::ClosurePlaceholder => {
            None
        }
    }
}

fn is_comparison_op(op: &BinOp) -> bool {
    matches!(
        op,
        BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_vm::mir::types::*;

    fn make_mir(stmts: Vec<MirStatement>) -> MirFunction {
        MirFunction {
            name: "test".to_string(),
            blocks: vec![BasicBlock {
                id: BasicBlockId(0),
                statements: stmts,
                terminator: Terminator {
                    kind: TerminatorKind::Return,
                    span: shape_ast::Span::default(),
                },
            }],
            num_locals: 4,
            param_slots: vec![],
            param_reference_kinds: vec![],
            local_types: vec![],
            span: shape_ast::Span::default(),
            field_name_table: Default::default(),
        }
    }

    fn assign_const(slot: u16, constant: MirConstant) -> MirStatement {
        MirStatement {
            kind: StatementKind::Assign(
                Place::Local(SlotId(slot)),
                Rvalue::Use(Operand::Constant(constant)),
            ),
            span: shape_ast::Span::default(),
            point: Point(0),
        }
    }

    #[test]
    fn infer_float_from_constant() {
        let mir = make_mir(vec![assign_const(1, MirConstant::Float(0))]);
        let kinds = infer_slot_kinds(&mir, &[]);
        assert_eq!(kinds[1], Some(NativeKind::Float64));
    }

    #[test]
    fn infer_int_from_constant() {
        let mir = make_mir(vec![assign_const(1, MirConstant::Int(42))]);
        let kinds = infer_slot_kinds(&mir, &[]);
        assert_eq!(kinds[1], Some(NativeKind::Int64));
    }

    #[test]
    fn infer_bool_from_constant() {
        let mir = make_mir(vec![assign_const(1, MirConstant::Bool(true))]);
        let kinds = infer_slot_kinds(&mir, &[]);
        assert_eq!(kinds[1], Some(NativeKind::Bool));
    }

    #[test]
    fn infer_float_from_binop() {
        let mir = make_mir(vec![
            assign_const(1, MirConstant::Float(0)),
            assign_const(2, MirConstant::Float(0)),
            MirStatement {
                kind: StatementKind::Assign(
                    Place::Local(SlotId(3)),
                    Rvalue::BinaryOp(
                        BinOp::Add,
                        Operand::Copy(Place::Local(SlotId(1))),
                        Operand::Copy(Place::Local(SlotId(2))),
                    ),
                ),
                span: shape_ast::Span::default(),
                point: Point(0),
            },
        ]);
        let kinds = infer_slot_kinds(&mir, &[]);
        assert_eq!(kinds[3], Some(NativeKind::Float64));
    }

    #[test]
    fn infer_bool_from_comparison() {
        let mir = make_mir(vec![
            assign_const(1, MirConstant::Float(0)),
            assign_const(2, MirConstant::Float(0)),
            MirStatement {
                kind: StatementKind::Assign(
                    Place::Local(SlotId(3)),
                    Rvalue::BinaryOp(
                        BinOp::Lt,
                        Operand::Copy(Place::Local(SlotId(1))),
                        Operand::Copy(Place::Local(SlotId(2))),
                    ),
                ),
                span: shape_ast::Span::default(),
                point: Point(0),
            },
        ]);
        let kinds = infer_slot_kinds(&mir, &[]);
        assert_eq!(kinds[3], Some(NativeKind::Bool));
    }

    #[test]
    fn infer_backward_from_typed_sibling_on_binop() {
        // Regression: `|x| x + 1` leaves `x` (a param) Unknown after forward
        // inference because params are seeded from `existing`, not from uses.
        // The backward pass must propagate Int64 from the typed constant `1`
        // into `x`'s slot so the JIT binop picker routes through
        // `compile_binop_int64` instead of the dynamic-op error path.
        //
        // MIR shape:
        //   param(0) = x  (Unknown)
        //   _1 = x + Int(1)
        let mut mir = make_mir(vec![MirStatement {
            kind: StatementKind::Assign(
                Place::Local(SlotId(1)),
                Rvalue::BinaryOp(
                    BinOp::Add,
                    Operand::Copy(Place::Local(SlotId(0))),
                    Operand::Constant(MirConstant::Int(1)),
                ),
            ),
            span: shape_ast::Span::default(),
            point: Point(0),
        }]);
        mir.param_slots = vec![SlotId(0)];
        let kinds = infer_slot_kinds(&mir, &[]);
        assert_eq!(
            kinds[0],
            Some(NativeKind::Int64),
            "backward pass should infer x: Int64 from `x + Int(1)`"
        );
    }

    #[test]
    fn infer_backward_chains_across_params() {
        // `|x, y| x + y + 1` — typed constant `1` reaches both params via
        // two rounds of backward propagation. After round 1: `_1 = x + y`
        // stays Unknown (both sides Unknown); `_2 = _1 + Int(1)` makes `_1`
        // Int64. Round 2: `_1 = x + y` with lhs Unknown, rhs Unknown still
        // doesn't help — we need forward assignment of `_1` to come through
        // first. The forward pass already handles `_1` because both operands
        // are "Unknown" → rvalue kind returns None. So after backward makes
        // `_1` = Int64, the statement `_1 = x + y` would need ANOTHER pass
        // that uses the Assign's LHS kind to constrain RHS operands. That
        // is not implemented here — we only propagate within a single binop.
        //
        // This test pins the current (intentionally limited) behaviour:
        // the simpler case of `|x| x + 1` works; chained-binop backward
        // propagation through an intermediate local does NOT.
        let mut mir = make_mir(vec![
            MirStatement {
                kind: StatementKind::Assign(
                    Place::Local(SlotId(2)),
                    Rvalue::BinaryOp(
                        BinOp::Add,
                        Operand::Copy(Place::Local(SlotId(0))),
                        Operand::Copy(Place::Local(SlotId(1))),
                    ),
                ),
                span: shape_ast::Span::default(),
                point: Point(0),
            },
            MirStatement {
                kind: StatementKind::Assign(
                    Place::Local(SlotId(3)),
                    Rvalue::BinaryOp(
                        BinOp::Add,
                        Operand::Copy(Place::Local(SlotId(2))),
                        Operand::Constant(MirConstant::Int(1)),
                    ),
                ),
                span: shape_ast::Span::default(),
                point: Point(0),
            },
        ]);
        mir.param_slots = vec![SlotId(0), SlotId(1)];
        let kinds = infer_slot_kinds(&mir, &[]);
        // The inner binop picks up the type from `_2 + Int(1)` backwards.
        assert_eq!(kinds[2], Some(NativeKind::Int64));
    }

    #[test]
    fn existing_kinds_preserved() {
        let mir = make_mir(vec![assign_const(1, MirConstant::Float(0))]);
        let existing = vec![None, Some(NativeKind::Int32)];
        let kinds = infer_slot_kinds(&mir, &existing);
        // Existing Int32 is preserved (not overridden by Float64 inference)
        assert_eq!(kinds[1], Some(NativeKind::Int32));
    }

    #[test]
    fn cranelift_type_mapping() {
        assert_eq!(cranelift_type_for_slot(NativeKind::Float64), types::F64);
        assert_eq!(cranelift_type_for_slot(NativeKind::Int32), types::I32);
        assert_eq!(cranelift_type_for_slot(NativeKind::Bool), types::I8);
        assert_eq!(cranelift_type_for_slot(NativeKind::Int64), types::I64);
        assert_eq!(cranelift_type_for_slot(NativeKind::String), types::I64);
    }

    // -----------------------------------------------------------------------
    // R4.2F: borrow StackSlot sizing invariants
    //
    // `Rvalue::Borrow` creates a stack cell with
    //     size = cranelift_type_for_slot(kind).bytes()
    //     align = log2(size)
    // These tests pin the native widths across all slot kinds that flow into
    // borrow cells. Non-native kinds must collapse to 8 bytes / align=3 so the
    // widening is a no-op for the legacy heap/unknown path.
    // -----------------------------------------------------------------------

    #[test]
    fn r4_2f_borrow_cell_sizes() {
        // Native-typed slots get their natural width.
        assert_eq!(cranelift_type_for_slot(NativeKind::Float64).bytes(), 8);
        assert_eq!(cranelift_type_for_slot(NativeKind::Int64).bytes(), 8);
        assert_eq!(cranelift_type_for_slot(NativeKind::Int32).bytes(), 4);
        assert_eq!(cranelift_type_for_slot(NativeKind::UInt32).bytes(), 4);
        assert_eq!(cranelift_type_for_slot(NativeKind::Int16).bytes(), 2);
        assert_eq!(cranelift_type_for_slot(NativeKind::UInt16).bytes(), 2);
        assert_eq!(cranelift_type_for_slot(NativeKind::Int8).bytes(), 1);
        assert_eq!(cranelift_type_for_slot(NativeKind::UInt8).bytes(), 1);
        assert_eq!(cranelift_type_for_slot(NativeKind::Bool).bytes(), 1);
        // Non-native slots collapse to 8 bytes (legacy behaviour).
        assert_eq!(cranelift_type_for_slot(NativeKind::String).bytes(), 8);
    }

    #[test]
    fn r4_2f_borrow_cell_alignment_shifts() {
        // `align_shift = size.trailing_zeros()` — must match log2(size) for
        // every power-of-two native width. If this ever breaks, the
        // `StackSlotData::new` call in `Rvalue::Borrow` will assert.
        for kind in [
            NativeKind::Float64,
            NativeKind::Int64,
            NativeKind::Int32,
            NativeKind::UInt32,
            NativeKind::Int16,
            NativeKind::UInt16,
            NativeKind::Int8,
            NativeKind::UInt8,
            NativeKind::Bool,
            NativeKind::String,
        ] {
            let size = cranelift_type_for_slot(kind).bytes();
            assert!(
                size.is_power_of_two(),
                "slot kind {:?} has non-power-of-two size {}",
                kind,
                size
            );
            let shift = size.trailing_zeros() as u8;
            assert_eq!(
                1u32 << shift,
                size,
                "slot kind {:?}: shift {} does not reconstruct size {}",
                kind,
                shift,
                size
            );
        }
    }
}
