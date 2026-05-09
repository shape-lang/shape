//! Object creation operations (NewArray, NewObject, NewTypedObject)
//!
//! Handles allocation and initialization of arrays, objects, and typed objects.
//!
//! Wave 6.5 substep-2 Wave-α `D-obj-create` (ADR-006 §2.7.7 / §2.7.8 / Q9-Q10,
//! playbook §10 D-obj-create row): the 19 mandatory shim caller sites in this
//! file's `op_*` factory methods migrate from the deleted shim layer
//! (`push_raw_u64` / `pop_raw_u64`) to the kept kinded API
//! (`push_kinded` / `pop_kinded` + `drop_with_kind`). `op_new_typed_object`
//! constructs `Arc<TypedObjectStorage>` directly per playbook §3's
//! per-`HeapKind` push pattern, then pushes the raw `Arc::into_raw` pointer
//! bits with `NativeKind::Ptr(HeapKind::TypedObject)`.
//!
//! `op_new_object` / `op_new_matrix` / `op_new_array` / `op_new_typed_array`
//! depend on `shape_value` constructors that were deleted by the strict-typing
//! bulldozer (`vmarray_from_vec`, `ValueWord::from_array`,
//! `ValueWord::from_matrix`, `ValueWord::from_int_array`,
//! `ValueWord::from_float_array`, `ValueWord::from_bool_array`). The proper
//! reentry shape for those is the per-`HeapKind` `Arc<T>` construction path
//! plus the kind-aware `op_new_typed_array_*` opcodes; that's Phase 2c
//! territory per ADR-006 §2.7.4. Until then their bodies drain stack
//! arguments via `pop_kinded` + `drop_with_kind` (preserves the stack ABI
//! `data.len() == kinds.len()` invariant) and surface
//! `VMError::NotImplemented` — the canonical playbook §7 #4 "no clean
//! migration this round" shape.
//!
//! Cross-cluster cascade (playbook §8 surface): the helper functions at
//! the bottom of this file (`nb_to_slot_with_field_type`,
//! `decode_field_bits_for_type`, `read_slot_nb`, `read_slot_value_typed`,
//! `clone_slots_with_update`) are pre-existing forbidden-pattern carriers
//! (they take/return `&ValueWord`, decode via `tag_bits::is_tagged`, etc.).
//! They are imported by 7+ files in five OTHER cluster territories
//! (`typed_object_ops.rs` D-typed-obj-ops; `objects/mod.rs` D-objects-mod;
//! `objects/datatable_methods/*.rs` D-objects-mod tail;
//! `control_flow/foreign_marshal.rs` B-control-flow-heap;
//! `variables/mod.rs` B-variables-loadptr; `vm_impl/modules.rs` and
//! `vm_impl/schemas.rs` E-vm-impl-tail). Migrating them off `ValueWord`
//! requires coordinated edits across those territories, which is exactly
//! the playbook §8 cross-cluster-cascade surface-and-stop trigger. The
//! helpers stay as-is for this cluster; the supervisor coordinates the
//! cleanup once all consumers' clusters have landed.

use crate::{
    bytecode::{Instruction, Operand},
    executor::vm_impl::stack::drop_with_kind,
    executor::VirtualMachine,
};
use rust_decimal::prelude::ToPrimitive;
use shape_runtime::type_schema::FieldType;
use shape_value::{
    HeapKind, NativeKind, TypedObjectStorage, VMError, ValueSlot, ValueWord, ValueWordExt,
};
use std::sync::Arc;

fn field_type_to_int_width(ft: &FieldType) -> Option<shape_ast::IntWidth> {
    match ft {
        FieldType::I8 => Some(shape_ast::IntWidth::I8),
        FieldType::U8 => Some(shape_ast::IntWidth::U8),
        FieldType::I16 => Some(shape_ast::IntWidth::I16),
        FieldType::U16 => Some(shape_ast::IntWidth::U16),
        FieldType::I32 => Some(shape_ast::IntWidth::I32),
        FieldType::U32 => Some(shape_ast::IntWidth::U32),
        FieldType::U64 => Some(shape_ast::IntWidth::U64),
        _ => None,
    }
}

impl VirtualMachine {
    /// Create a new TypedObject with fields from stack
    ///
    /// Stack: [...field_values] -> [typed_object]
    /// Operand: TypedObjectAlloc { schema_id, field_count }
    ///
    /// ADR-006 §2.7.7 / playbook §3: pop fields via `pop_kinded` (each
    /// slot's `NativeKind` matches its producing opcode's emitted kind),
    /// build per-field `ValueSlot`s via the kind+FieldType dispatch in
    /// `kinded_to_slot`, then construct `Arc<TypedObjectStorage>` per the
    /// playbook §3 TypedObject pattern and push the raw `Arc::into_raw`
    /// pointer bits with `NativeKind::Ptr(HeapKind::TypedObject)`. No
    /// ValueWord round-trip; the deleted `decode_field_bits_for_type`
    /// does not run. The popped shares' ownership transfers into the new
    /// TypedObject (each heap slot's strong-count remains at 1; Drop on
    /// the final TypedObject decrements via `field_kinds`-driven
    /// dispatch — same pattern as `executor/builtins/object_ops.rs`).
    pub(in crate::executor) fn op_new_typed_object(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let (schema_id, field_count) = match instruction.operand {
            Some(Operand::TypedObjectAlloc {
                schema_id,
                field_count,
            }) => (schema_id, field_count),
            _ => return Err(VMError::InvalidOperand),
        };

        // Look up the schema's per-field FieldType list before popping —
        // we need it to dispatch each slot's kind+payload through
        // `kinded_to_slot` once it leaves the stack.
        let field_types: Option<Vec<FieldType>> = self
            .lookup_schema(schema_id as u32)
            .map(|schema| schema.fields.iter().map(|f| f.field_type.clone()).collect());

        // Pop kinded fields (LIFO from the stack — last argument is
        // popped first), then reverse to recover declared field order.
        // On any pop failure mid-way, the already-popped shares are
        // released via `drop_with_kind` to keep refcount discipline.
        let mut popped: Vec<(u64, NativeKind)> = Vec::with_capacity(field_count as usize);
        for _ in 0..field_count {
            match self.pop_kinded() {
                Ok(pair) => popped.push(pair),
                Err(e) => {
                    for (b, k) in popped.drain(..) {
                        drop_with_kind(b, k);
                    }
                    return Err(e);
                }
            }
        }
        popped.reverse();

        // Allocate slots + heap_mask. Each popped (bits, kind) pair
        // transfers its strong-count share into the slot list — the new
        // TypedObjectStorage's Drop releases it via per-`field_kinds[i]`
        // dispatch (ADR-006 §2.5).
        let mut slots: Vec<ValueSlot> = Vec::with_capacity(field_count as usize);
        let mut heap_mask: u64 = 0;
        for (i, (bits, kind)) in popped.iter().enumerate() {
            let field_type = field_types.as_ref().and_then(|types| types.get(i));
            let (slot, is_heap) = kinded_to_slot(*bits, *kind, field_type);
            if is_heap {
                heap_mask |= 1u64 << i;
            }
            slots.push(slot);
        }

        // Build the per-slot `field_kinds` table from the popped slot
        // kinds. Lockstep with `slots` per the §2.5 invariant
        // (`slots.len() == field_kinds.len()`). Drop walks this table to
        // dispatch per-slot `Arc::decrement_strong_count`.
        let field_kinds: Vec<NativeKind> = popped.iter().map(|(_, k)| *k).collect();

        // Construct the storage, transfer ownership to the stack via
        // `Arc::into_raw` + `push_kinded(NativeKind::Ptr(HeapKind::TypedObject))`.
        let storage = Arc::new(TypedObjectStorage::new(
            schema_id as u64,
            slots.into_boxed_slice(),
            heap_mask,
            Arc::from(field_kinds.into_boxed_slice()),
        ));
        let bits = Arc::into_raw(storage) as u64;
        self.push_kinded(bits, NativeKind::Ptr(HeapKind::TypedObject))
    }

    /// Phase 2c (ADR-006 §2.7.4): `op_new_object` builds an ad-hoc
    /// TypedObject from key/value stack pairs via
    /// `create_typed_object_from_pairs`, which is itself a forbidden-
    /// pattern carrier in `vm_impl/schemas.rs` (returns `ValueWord` /
    /// dispatches via `ValueWordExt::as_str`) — that helper is
    /// `E-vm-impl-tail` cluster territory.
    ///
    /// Until that helper is migrated to a kinded `KindedSlot`-returning
    /// shape (Phase 2c), this opcode body drains the popped pairs via
    /// `pop_kinded` + `drop_with_kind` (preserving the stack ABI
    /// `data.len() == kinds.len()` invariant — playbook §7 #4) and
    /// surfaces `VMError::NotImplemented`. The drain is required even on
    /// the error path: the stack must be left consistent.
    pub(in crate::executor) fn op_new_object(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        if let Some(Operand::Count(count)) = instruction.operand {
            // Drain 2*count slots (alternating key, value) per the
            // pre-§2.7.7 emission pattern. `pop_kinded` short-circuits on
            // underflow; release any successfully popped shares.
            for _ in 0..count {
                // Pop value, then key (LIFO).
                if let Ok((vb, vk)) = self.pop_kinded() {
                    drop_with_kind(vb, vk);
                } else {
                    return Err(VMError::StackUnderflow);
                }
                if let Ok((kb, kk)) = self.pop_kinded() {
                    drop_with_kind(kb, kk);
                } else {
                    return Err(VMError::StackUnderflow);
                }
            }
            Err(VMError::NotImplemented(
                "op_new_object: ad-hoc TypedObject construction depends on \
                 `create_typed_object_from_pairs` (E-vm-impl-tail territory) \
                 being migrated off ValueWord — phase-2c, see ADR-006 §2.7.4"
                    .to_string(),
            ))
        } else {
            Err(VMError::InvalidOperand)
        }
    }

    /// Create a new Matrix from values on the stack.
    ///
    /// Stack: [...f64_values (rows*cols)] -> [matrix]
    /// Operand: MatrixDims { rows, cols }
    ///
    /// Phase 2c (ADR-006 §2.7.4): `MatrixData` is held by
    /// `TypedArrayData::Matrix(Arc<MatrixData>)` — a `HeapKind::TypedArray`
    /// arm. The emit-side opcode emission pattern (one popped numeric per
    /// matrix cell) is fine, but pushing via the deleted
    /// `ValueWord::from_matrix` helper is no longer valid. The kinded
    /// reentry shape is `Arc::into_raw(Arc::new(TypedArrayData::Matrix(
    /// Arc::new(MatrixData::from_flat(...)))))` + `push_kinded(bits,
    /// NativeKind::Ptr(HeapKind::TypedArray))`, but the matrix builtin
    /// frontier has additional consumers (matrix-typed methods on
    /// `TypedArrayData::Matrix`) that depend on the same migration. Until
    /// those land, this op drains the popped slots via `pop_kinded` +
    /// `drop_with_kind` (stack ABI invariant) and surfaces
    /// `VMError::NotImplemented`.
    pub(in crate::executor) fn op_new_matrix(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let (rows, cols) = match instruction.operand {
            Some(Operand::MatrixDims { rows, cols }) => (rows as u32, cols as u32),
            _ => return Err(VMError::InvalidOperand),
        };

        let total = (rows as usize) * (cols as usize);
        for _ in 0..total {
            match self.pop_kinded() {
                Ok((bits, kind)) => drop_with_kind(bits, kind),
                Err(_) => return Err(VMError::StackUnderflow),
            }
        }
        Err(VMError::NotImplemented(format!(
            "op_new_matrix({}×{}): MatrixData construction depends on the \
             kinded TypedArray emit path (Phase 2c reentry — see ADR-006 §2.7.4)",
            rows, cols
        )))
    }

    /// Create a generic untyped Array from N stack elements.
    ///
    /// Phase 2c (ADR-006 §2.7.4): the legacy `ValueWord::from_array` /
    /// `shape_value::vmarray_from_vec` constructors were deleted by the
    /// strict-typing bulldozer (CLAUDE.md "Forbidden Patterns" lists
    /// `vmarray_from_vec` as a deleted name). The kinded reentry shape
    /// is per-kind dispatch into a `TypedArrayData::*` variant matching
    /// the elements' actual `NativeKind` — but a truly heterogeneous-
    /// array constructor (mixed kinds) requires
    /// `TypedArrayData::HeapValue(Arc<TypedBuffer<Arc<HeapValue>>>)` on
    /// the back end, plus rebuilding the per-element `Arc<HeapValue>`
    /// projection from `(bits, kind)` pairs (a `HeapValue::*`-arm match
    /// that today's emit path doesn't yet supply). This is Phase 2c
    /// territory.
    ///
    /// Until then the op drains the popped slots via `pop_kinded` +
    /// `drop_with_kind` and surfaces `VMError::NotImplemented`.
    pub(in crate::executor) fn op_new_array(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        if let Some(Operand::Count(count)) = instruction.operand {
            for _ in 0..count {
                match self.pop_kinded() {
                    Ok((bits, kind)) => drop_with_kind(bits, kind),
                    Err(_) => return Err(VMError::StackUnderflow),
                }
            }
            Err(VMError::NotImplemented(
                "op_new_array: generic untyped-array construction depends \
                 on the kinded TypedArrayData::HeapValue emit path \
                 (Phase 2c reentry — see ADR-006 §2.7.4)"
                    .to_string(),
            ))
        } else {
            Err(VMError::InvalidOperand)
        }
    }

    /// Create a typed array (IntArray/FloatArray/BoolArray) from N elements on the stack.
    ///
    /// Phase 2c (ADR-006 §2.7.4): the legacy
    /// `ValueWord::from_int_array` / `from_float_array` / `from_bool_array`
    /// / `from_array` constructors were deleted by the strict-typing
    /// bulldozer; their replacement is per-kind `Arc<TypedArrayData::*>`
    /// construction + `push_kinded(NativeKind::Ptr(HeapKind::TypedArray))`.
    /// The bytecode compiler is also being migrated toward emitting the
    /// kind-specific `NewTypedArrayI64` / `NewTypedArrayF64` /
    /// `NewTypedArrayBool` opcodes (already wired in `dispatch.rs`)
    /// instead of this dynamic-classifier shape, which makes the op
    /// itself a Phase 2c retire-or-refactor candidate.
    ///
    /// Until that lands, the op drains the popped slots via `pop_kinded`
    /// + `drop_with_kind` and surfaces `VMError::NotImplemented`. The
    /// per-kind opcodes already in dispatch.rs continue to work
    /// independently.
    pub(in crate::executor) fn op_new_typed_array(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let count = match instruction.operand {
            Some(Operand::Count(c)) => c as usize,
            _ => return Err(VMError::InvalidOperand),
        };

        for _ in 0..count {
            match self.pop_kinded() {
                Ok((bits, kind)) => drop_with_kind(bits, kind),
                Err(_) => return Err(VMError::StackUnderflow),
            }
        }
        Err(VMError::NotImplemented(format!(
            "op_new_typed_array({}): dynamic-classifier path retires in \
             favour of the per-kind `NewTypedArray{{I64,F64,Bool}}` opcodes; \
             phase-2c reentry — see ADR-006 §2.7.4",
            count
        )))
    }
}

/// Build a single TypedObject slot from a popped `(bits, kind)` pair plus
/// the schema's declared `FieldType` for that slot. Returns
/// `(slot, is_heap)` where `is_heap` is the bit to set in `heap_mask`.
///
/// ADR-006 §2.4 / §2.5: the kind is the source of truth for slot shape.
/// Width-truncation for sub-i64 schemas happens against the popped i64
/// payload before storing. For heap-kind slots, the popped `bits` are
/// already an `Arc::into_raw` raw pointer — we move it into a typed
/// `ValueSlot::from_raw(bits)` and set the `heap_mask` bit; the new
/// TypedObjectStorage's `Drop` retires that share via per-`field_kinds[i]`
/// dispatch.
///
/// Heterogeneous-kind schema/value combinations (e.g. schema says I64,
/// producer pushed Float64) are lossily coerced where the existing
/// pre-bulldozer behaviour did so (int<->float widening, decimal stored
/// lossy as f64) and stored zero where the kind cannot represent the
/// schema type.
fn kinded_to_slot(
    bits: u64,
    kind: NativeKind,
    field_type: Option<&FieldType>,
) -> (ValueSlot, bool) {
    // FieldType::F64 / Decimal: schema demands inline f64 storage
    // (matching `read_slot_nb`'s FieldType::F64 / FieldType::Decimal
    // arms which read `slots[index].as_f64()`). Pre-bulldozer behaviour
    // is lossy for Arc<Decimal> inputs; preserve that here so existing
    // read-back consumers still work. The popped Decimal Arc share is
    // released after we materialise its f64 projection.
    if matches!(field_type, Some(FieldType::F64) | Some(FieldType::Decimal)) {
        let n = match kind {
            NativeKind::Float64 => f64::from_bits(bits),
            NativeKind::Int64 => (bits as i64) as f64,
            NativeKind::Bool => {
                if bits != 0 {
                    1.0
                } else {
                    0.0
                }
            }
            NativeKind::Ptr(HeapKind::Decimal) if bits != 0 => {
                // SAFETY: pop_kinded transferred ownership of one
                // Arc<rust_decimal::Decimal> strong-count share via raw
                // pointer bits; reconstruct, read, and let it drop
                // (releases the share) — the slot stores the lossy f64
                // projection per the schema's existing inline-storage
                // contract.
                let arc: Arc<rust_decimal::Decimal> =
                    unsafe { Arc::from_raw(bits as *const rust_decimal::Decimal) };
                let f = arc.to_f64().unwrap_or(0.0);
                drop(arc);
                f
            }
            _ => 0.0,
        };
        return (ValueSlot::from_number(n), false);
    }

    // Heap-kind slots (other than the FieldType::F64/Decimal lossy
    // case above): the popped bits are an Arc raw pointer. Move them
    // into the slot via `ValueSlot::from_raw(bits)`; setting the
    // heap_mask bit makes the new TypedObjectStorage's Drop retire the
    // share through the matching `field_kinds[i]` arm.
    let is_heap = matches!(kind, NativeKind::String | NativeKind::Ptr(_));
    if is_heap {
        return (ValueSlot::from_raw(bits), true);
    }

    // Inline-scalar kinds: rebuild the typed slot per the schema's
    // FieldType so the slot's read-back semantics match the existing
    // `read_slot_nb` shape. The popped `bits` carry the raw native
    // payload directly; we rewrap via `ValueSlot::from_*`.
    match field_type {
        Some(FieldType::I64) | Some(FieldType::Timestamp) => {
            let i = match kind {
                NativeKind::Int64 => bits as i64,
                NativeKind::Float64 => f64::from_bits(bits) as i64,
                NativeKind::Bool => (bits != 0) as i64,
                _ => 0,
            };
            (ValueSlot::from_int(i), false)
        }
        Some(ft) if ft.is_width_integer() => {
            let raw = match kind {
                NativeKind::Int64 => bits as i64,
                NativeKind::Float64 => f64::from_bits(bits) as i64,
                NativeKind::Bool => (bits != 0) as i64,
                k if matches!(
                    k,
                    NativeKind::Int8
                        | NativeKind::Int16
                        | NativeKind::Int32
                        | NativeKind::UInt8
                        | NativeKind::UInt16
                        | NativeKind::UInt32
                        | NativeKind::UInt64
                ) =>
                {
                    bits as i64
                }
                _ => 0,
            };
            if matches!(ft, FieldType::U64) {
                // U64 stored as i64 bits; preserve the high bit
                // pattern losslessly.
                (ValueSlot::from_int(raw), false)
            } else {
                let truncated = if let Some(w) = field_type_to_int_width(ft) {
                    w.truncate(raw)
                } else {
                    raw
                };
                (ValueSlot::from_int(truncated), false)
            }
        }
        Some(FieldType::Bool) => {
            let b = match kind {
                NativeKind::Bool => bits != 0,
                NativeKind::Int64 => (bits as i64) != 0,
                NativeKind::Float64 => f64::from_bits(bits) != 0.0,
                _ => false,
            };
            (ValueSlot::from_bool(b), false)
        }
        // `Any` and non-primitive schema types with an inline-scalar
        // popped value: store the raw bits as-is. The schema-driven
        // read path (`read_slot_nb`) reconstructs the appropriate
        // shape; preserving the raw bits is the lossless round-trip
        // per the existing pre-bulldozer behaviour. heap_mask remains
        // 0 — the value is inline.
        Some(FieldType::Any) | None | Some(_) => (ValueSlot::from_raw(bits), false),
    }
}

/// Wave E+5 / task #98: decode raw stack bits into a tagged `ValueWord`,
/// disambiguating per the declared field type so that fields whose producer
/// pushed *raw native bits* (e.g. Unit-B `op_push_const Int(42)` →
/// `0x000000000000002A`) are correctly recovered as ValueWord ints, while
/// fields whose producer pushed *tagged ValueWord bits* (the legacy
/// polymorphic path, `0xFFF9_0000_0000_002A`) round-trip unchanged.
///
/// The disambiguation key is `is_tagged(bits)`: tagged ValueWords have the
/// canonical NaN-box prefix `0xFFF8`; raw native i64 / bool values do not.
///
/// For `FieldType::Any` (enum tuple payloads etc.), small untagged bits are
/// most likely raw-native-int output from a post-Unit-B `PushConst Int(...)`
/// since real `Any` slots encode their value as a tagged ValueWord (heap
/// pointer or NaN-boxed inline). We use the i48 range as the cutoff;
/// out-of-range untagged bits stay on the tagged-passthrough path so larger
/// raw f64 / heap-pointer values aren't misclassified.
fn decode_field_bits_for_type(bits: u64, field_type: Option<&FieldType>) -> ValueWord {
    use shape_value::tag_bits::is_tagged;
    let is_int_field = matches!(
        field_type,
        Some(
            FieldType::I64
                | FieldType::I8
                | FieldType::I16
                | FieldType::I32
                | FieldType::U8
                | FieldType::U16
                | FieldType::U32
                | FieldType::U64
        )
    );
    if is_int_field && !is_tagged(bits) {
        // Raw native i64 / sub-i64 bits — re-tag as a ValueWord int so
        // `nb_to_slot_with_field_type` can extract via `as_i64()`. Out of
        // i48 range falls back to a heap-boxed BigInt.
        return ValueWord::from_i64(bits as i64);
    }
    if matches!(field_type, Some(FieldType::Bool)) && !is_tagged(bits) {
        // Raw native bool: 0 → false, anything else → true.
        return ValueWord::from_bool(bits != 0);
    }
    // For `Any` fields (e.g. enum tuple payloads), an untagged scalar
    // whose magnitude fits in i48 is most likely raw-native-int output
    // from a post-Unit-B `PushConst Int(...)`.
    if matches!(field_type, Some(FieldType::Any)) && !is_tagged(bits) {
        let signed = bits as i64;
        if signed >= shape_value::tag_bits::I48_MIN
            && signed <= shape_value::tag_bits::I48_MAX
        {
            return ValueWord::from_i64(signed);
        }
    }
    ValueWord::from_raw_bits(bits)
}

/// Convert a ValueWord to a ValueSlot using schema field type when available.
/// This avoids ambiguous non-heap encodings for `FieldType::Any`.
pub(in crate::executor) fn nb_to_slot_with_field_type(
    nb: &ValueWord,
    field_type: Option<&FieldType>,
) -> (ValueSlot, bool) {
    match field_type {
        Some(FieldType::I64) => (
            ValueSlot::from_int(
                nb.as_i64()
                    .or_else(|| nb.as_f64().map(|n| n as i64))
                    .unwrap_or(0),
            ),
            false,
        ),
        Some(ft) if ft.is_width_integer() => {
            if matches!(ft, FieldType::U64) {
                // U64 may exceed i64::MAX — extract via as_u64() for lossless storage
                let val = nb
                    .as_u64_value()
                    .or_else(|| nb.as_i64().map(|i| i as u64))
                    .or_else(|| nb.as_f64().map(|n| n as u64))
                    .unwrap_or(0);
                (ValueSlot::from_int(val as i64), false)
            } else {
                let raw = nb
                    .as_i64()
                    .or_else(|| nb.as_f64().map(|n| n as i64))
                    .unwrap_or(0);
                let truncated = if let Some(w) = field_type_to_int_width(ft) {
                    w.truncate(raw)
                } else {
                    raw
                };
                (ValueSlot::from_int(truncated), false)
            }
        }
        Some(FieldType::Bool) => (
            ValueSlot::from_bool(nb.as_bool().unwrap_or(nb.is_truthy())),
            false,
        ),
        Some(FieldType::F64) | Some(FieldType::Decimal) => (
            ValueSlot::from_number(
                nb.as_number_coerce()
                    .or_else(|| nb.as_decimal().and_then(|d| d.to_f64()))
                    .unwrap_or(0.0),
            ),
            false,
        ),
        // `Any` must preserve dynamic type losslessly — including inline inline tag
        // variants like Function, ModuleFunction, I48, etc.  `from_value_word`
        // stores raw NaN-boxed bits for inline tags and clones HeapValues for
        // heap tags, so the exact tag round-trips through `as_value_word`.
        Some(FieldType::Any) | None => ValueSlot::from_value_word(nb),
        // For non-primitive schema field types, preserve full value via from_value_word.
        Some(_) => {
            if nb.is_none() {
                (ValueSlot::none(), false)
            } else {
                ValueSlot::from_value_word(nb)
            }
        }
    }
}

/// Read a ValueWord value from a TypedObject slot.
///
/// `field_type` is optional and lets callers preserve i64/bool semantics for non-heap slots.
pub(in crate::executor) fn read_slot_nb(
    slots: &[ValueSlot],
    index: usize,
    heap_mask: u64,
    field_type: Option<&shape_runtime::type_schema::FieldType>,
) -> ValueWord {
    if index >= slots.len() {
        return ValueWord::none();
    }

    if heap_mask & (1u64 << index) != 0 {
        return slots[index].as_heap_nb();
    }

    match field_type {
        Some(shape_runtime::type_schema::FieldType::I64) => {
            ValueWord::from_i64(slots[index].as_i64())
        }
        Some(shape_runtime::type_schema::FieldType::Bool) => {
            ValueWord::from_bool(slots[index].as_bool())
        }
        Some(shape_runtime::type_schema::FieldType::F64) => {
            ValueWord::from_f64(slots[index].as_f64())
        }
        Some(shape_runtime::type_schema::FieldType::Decimal) => ValueWord::from_decimal(
            rust_decimal::Decimal::from_f64_retain(slots[index].as_f64()).unwrap_or_default(),
        ),
        // Width integer types: stored via from_int(), read back via as_i64()
        Some(ft) if ft.is_width_integer() => {
            let raw_bits = slots[index].as_i64() as u64;
            if matches!(ft, shape_runtime::type_schema::FieldType::U64)
                && raw_bits > i64::MAX as u64
            {
                ValueWord::from_native_u64(raw_bits)
            } else {
                ValueWord::from_i64(slots[index].as_i64())
            }
        }
        // Any and non-primitive types: reconstruct via as_value_word to preserve
        // all inline inline tag variants (Function, ModuleFunction, I48, etc.)
        Some(_) | None => slots[index].as_value_word(false),
    }
}

/// Read a ValueWord from a TypedObject slot with optional schema field type.
#[cfg(test)]
pub(in crate::executor) fn read_slot_value_typed(
    slots: &[ValueSlot],
    index: usize,
    heap_mask: u64,
    field_type: Option<&FieldType>,
) -> ValueWord {
    if index >= slots.len() {
        return ValueWord::none();
    }
    if heap_mask & (1u64 << index) != 0 {
        return slots[index].as_heap_nb();
    }

    match field_type {
        Some(FieldType::I64) => ValueWord::from_i64(slots[index].as_i64()),
        Some(FieldType::Bool) => ValueWord::from_bool(slots[index].as_bool()),
        Some(FieldType::F64) => ValueWord::from_f64(slots[index].as_f64()),
        Some(FieldType::Decimal) => ValueWord::from_decimal(
            rust_decimal::Decimal::from_f64_retain(slots[index].as_f64()).unwrap_or_default(),
        ),
        // Width integer types: stored via from_int(), read back via as_i64()
        Some(ft) if ft.is_width_integer() => {
            let raw_bits = slots[index].as_i64() as u64;
            if matches!(ft, FieldType::U64) && raw_bits > i64::MAX as u64 {
                ValueWord::from_native_u64(raw_bits)
            } else {
                ValueWord::from_i64(slots[index].as_i64())
            }
        }
        // Any and non-primitive types: reconstruct via as_value_word to preserve
        // all inline inline tag variants (Function, ModuleFunction, I48, etc.)
        Some(_) | None => slots[index].as_value_word(false),
    }
}

/// Clone slots and overwrite one index with a new ValueWord value.
pub(in crate::executor) fn clone_slots_with_update(
    slots: &[ValueSlot],
    heap_mask: u64,
    update_index: usize,
    update_value: &ValueWord,
    field_type: Option<&FieldType>,
) -> (Vec<ValueSlot>, u64) {
    let mut new_slots = Vec::with_capacity(slots.len());
    let mut new_mask: u64 = 0;

    for (index, slot) in slots.iter().enumerate() {
        if index == update_index {
            let (updated_slot, is_heap) = nb_to_slot_with_field_type(update_value, field_type);
            if is_heap {
                new_mask |= 1u64 << index;
            }
            new_slots.push(updated_slot);
            continue;
        }

        if heap_mask & (1u64 << index) != 0 {
            new_slots.push(unsafe { slot.clone_heap() });
            new_mask |= 1u64 << index;
        } else {
            new_slots.push(*slot);
        }
    }

    (new_slots, new_mask)
}
