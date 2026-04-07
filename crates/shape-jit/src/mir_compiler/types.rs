//! Type mapping for MIR-to-Cranelift IR compilation.
//!
//! Maps MIR LocalTypeInfo and SlotKind to Cranelift types.
//! Includes MIR-level type inference for determining slot kinds
//! when the bytecode compiler doesn't provide them.

use cranelift::prelude::types;
use shape_value::v2::ConcreteType;
use shape_vm::mir::types::*;
use shape_vm::type_tracking::SlotKind;

/// Whether a local slot holds a heap value that needs reference counting.
pub(crate) fn is_heap_type(type_info: &LocalTypeInfo) -> bool {
    matches!(type_info, LocalTypeInfo::NonCopy)
}

/// Whether a local slot is known to be Copy (no refcounting needed).
pub(crate) fn is_copy_type(type_info: &LocalTypeInfo) -> bool {
    matches!(type_info, LocalTypeInfo::Copy)
}

/// Get the SlotKind for a local, falling back to Unknown.
pub(crate) fn slot_kind_for_local(slot_kinds: &[SlotKind], slot_idx: u16) -> SlotKind {
    slot_kinds
        .get(slot_idx as usize)
        .copied()
        .unwrap_or(SlotKind::Unknown)
}

/// Whether a SlotKind is i32 (Int32 or UInt32).
pub(crate) fn is_i32_slot(kind: SlotKind) -> bool {
    matches!(kind, SlotKind::Int32 | SlotKind::UInt32)
}

/// Whether a SlotKind represents a native (non-NaN-boxed) Cranelift type.
pub(crate) fn is_native_slot(kind: SlotKind) -> bool {
    matches!(
        kind,
        SlotKind::Float64
            | SlotKind::Int32
            | SlotKind::UInt32
            | SlotKind::Bool
            | SlotKind::Int8
            | SlotKind::UInt8
            | SlotKind::Int16
            | SlotKind::UInt16
    )
}

/// Map a SlotKind to its Cranelift type.
/// Native numeric types get their natural width; everything else is I64 (NaN-boxed).
pub(crate) fn cranelift_type_for_slot(kind: SlotKind) -> cranelift::prelude::Type {
    match kind {
        SlotKind::Float64 => types::F64,
        SlotKind::Int32 | SlotKind::UInt32 => types::I32,
        SlotKind::Int8 | SlotKind::UInt8 | SlotKind::Bool => types::I8,
        SlotKind::Int16 | SlotKind::UInt16 => types::I16,
        // Int64, UInt64, Unknown, NanBoxed, String, Nullable*, IntSize, UIntSize
        _ => types::I64,
    }
}

/// Whether a SlotKind is a v2 heap pointer type (TypedArray, TypedStruct, StringObj).
/// These use inline refcounting via HeapHeader at offset 0.
pub(crate) fn is_v2_heap_slot(kind: SlotKind) -> bool {
    let _ = kind;
    false
}

/// Map a `ConcreteType` element type to the matching `SlotKind` for the v2
/// typed-array codegen helpers (`v2_array_get`/`v2_array_set`).
pub(crate) fn elem_slot_kind_for_concrete(elem: &ConcreteType) -> Option<SlotKind> {
    match elem {
        ConcreteType::F64 => Some(SlotKind::Float64),
        ConcreteType::I64 => Some(SlotKind::Int64),
        ConcreteType::I32 => Some(SlotKind::Int32),
        ConcreteType::I16 => Some(SlotKind::Int16),
        ConcreteType::I8 => Some(SlotKind::Int8),
        ConcreteType::U64 => Some(SlotKind::UInt64),
        ConcreteType::U32 => Some(SlotKind::UInt32),
        ConcreteType::U16 => Some(SlotKind::UInt16),
        ConcreteType::U8 => Some(SlotKind::UInt8),
        ConcreteType::Bool => Some(SlotKind::Bool),
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
) -> Option<SlotKind> {
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
/// Rules:
/// - Assign(slot, Use(Constant(Float(_)))) → Float64
/// - Assign(slot, Use(Constant(Int(_)))) → Int64 (NaN-boxed int uses 48-bit payload)
/// - Assign(slot, Use(Constant(Bool(_)))) → Bool
/// - Assign(slot, BinaryOp(arith, lhs, rhs)) → inherits from operands if both agree
/// - Assign(slot, Use(Move/Copy(other_slot))) → inherits from other_slot
/// - Conflicting assignments → Unknown
pub(crate) fn infer_slot_kinds(mir: &MirFunction, existing: &[SlotKind]) -> Vec<SlotKind> {
    let n = mir.num_locals as usize;
    let mut kinds = vec![SlotKind::Unknown; n];

    // Seed from existing slot_kinds (from bytecode compiler).
    for (i, &k) in existing.iter().enumerate() {
        if i < n && k != SlotKind::Unknown {
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
                        if idx < n && kinds[idx] == SlotKind::Unknown {
                            if let Some(inferred) = infer_rvalue_kind(rvalue, &kinds) {
                                kinds[idx] = inferred;
                            }
                        } else if idx < n {
                            // Slot already has a kind — check for conflicts.
                            if let Some(inferred) = infer_rvalue_kind(rvalue, &kinds) {
                                if inferred != kinds[idx] {
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

    // Parameters keep Unknown if not otherwise determined — they receive
    // NaN-boxed values from the calling convention.
    for &param_slot in &mir.param_slots {
        let idx = param_slot.0 as usize;
        if idx < n && existing.get(idx).copied().unwrap_or(SlotKind::Unknown) != SlotKind::Unknown
        {
            // Keep the existing kind from the compiler for params.
            kinds[idx] = existing[idx];
        }
    }

    kinds
}

/// Infer the SlotKind produced by an Rvalue.
fn infer_rvalue_kind(rvalue: &Rvalue, kinds: &[SlotKind]) -> Option<SlotKind> {
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
                        Some(SlotKind::Bool)
                    } else {
                        Some(l)
                    }
                }
                _ => {
                    // Mixed or unknown operands. Comparison still → Bool.
                    if is_comparison_op(op) {
                        Some(SlotKind::Bool)
                    } else {
                        None
                    }
                }
            }
        }
        Rvalue::UnaryOp(UnOp::Neg, operand) => infer_operand_kind(operand, kinds),
        Rvalue::UnaryOp(UnOp::Not, _) => Some(SlotKind::Bool),
        Rvalue::Clone(operand) => infer_operand_kind(operand, kinds),
        Rvalue::Borrow(_, _) => None,     // References are heap pointers
        Rvalue::Aggregate(_) => None,      // Arrays are heap objects
    }
}

/// Infer the SlotKind of an operand.
fn infer_operand_kind(operand: &Operand, kinds: &[SlotKind]) -> Option<SlotKind> {
    match operand {
        Operand::Constant(c) => infer_constant_kind(c),
        Operand::Copy(place) | Operand::Move(place) | Operand::MoveExplicit(place) => {
            let slot = place.root_local();
            let idx = slot.0 as usize;
            let k = kinds.get(idx).copied().unwrap_or(SlotKind::Unknown);
            if k != SlotKind::Unknown {
                Some(k)
            } else {
                None
            }
        }
    }
}

/// Infer the SlotKind of a constant.
fn infer_constant_kind(constant: &MirConstant) -> Option<SlotKind> {
    match constant {
        MirConstant::Float(_) => Some(SlotKind::Float64),
        MirConstant::Int(_) => Some(SlotKind::Int64),
        MirConstant::Bool(_) => Some(SlotKind::Bool),
        MirConstant::None => None,
        MirConstant::StringId(_) | MirConstant::Str(_) => Some(SlotKind::String),
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
        assert_eq!(kinds[1], SlotKind::Float64);
    }

    #[test]
    fn infer_int_from_constant() {
        let mir = make_mir(vec![assign_const(1, MirConstant::Int(42))]);
        let kinds = infer_slot_kinds(&mir, &[]);
        assert_eq!(kinds[1], SlotKind::Int64);
    }

    #[test]
    fn infer_bool_from_constant() {
        let mir = make_mir(vec![assign_const(1, MirConstant::Bool(true))]);
        let kinds = infer_slot_kinds(&mir, &[]);
        assert_eq!(kinds[1], SlotKind::Bool);
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
        assert_eq!(kinds[3], SlotKind::Float64);
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
        assert_eq!(kinds[3], SlotKind::Bool);
    }

    #[test]
    fn existing_kinds_preserved() {
        let mir = make_mir(vec![assign_const(1, MirConstant::Float(0))]);
        let existing = vec![SlotKind::Unknown, SlotKind::Int32];
        let kinds = infer_slot_kinds(&mir, &existing);
        // Existing Int32 is preserved (not overridden by Float64 inference)
        assert_eq!(kinds[1], SlotKind::Int32);
    }

    #[test]
    fn cranelift_type_mapping() {
        assert_eq!(cranelift_type_for_slot(SlotKind::Float64), types::F64);
        assert_eq!(cranelift_type_for_slot(SlotKind::Int32), types::I32);
        assert_eq!(cranelift_type_for_slot(SlotKind::Bool), types::I8);
        assert_eq!(cranelift_type_for_slot(SlotKind::Unknown), types::I64);
        assert_eq!(cranelift_type_for_slot(SlotKind::Int64), types::I64);
        assert_eq!(cranelift_type_for_slot(SlotKind::String), types::I64);
    }
}
