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
//! Wave-δ MR-string-misc body migration (2026-05-09): `op_new_matrix` and
//! `op_new_typed_array` migrate to real bodies.
//!
//! - **`op_new_matrix`**: pops `rows * cols` numeric (`Float64`) bits per
//!   the operand's `MatrixDims`; constructs
//!   `Arc<TypedArrayData::Matrix(Arc<MatrixData>)>` via
//!   `MatrixData::from_flat`; pushes via `Arc::into_raw + push_kinded(_,
//!   NativeKind::Ptr(HeapKind::TypedArray))`.
//! - **`op_new_typed_array`**: pops N elements with their kinds; if all
//!   elements share `Int64` / `Float64` / `Bool` kind, builds the matching
//!   `TypedArrayData::*` variant. Mixed-kind arrays would require
//!   `TypedArrayData::HeapValue` (heterogeneous, see `op_new_array`) and
//!   are surfaced.
//!
//! Two opcode bodies remain Phase-2c surfaces:
//!
//! - **`op_new_object`** still depends on the deleted
//!   `create_typed_object_from_pairs` (`vm_impl/schemas.rs` ValueWord-
//!   shaped helper, retired by Phase 2c per ADR-006 §2.7.4 / Q5).
//! - **`op_new_array`** (untyped heterogeneous) needs a kinded projection
//!   from `(bits, kind)` into `Arc<HeapValue>` for the
//!   `TypedArrayData::HeapValue` variant. There is no such projection
//!   helper today; the natural site is in `shape-value` next to the
//!   `TypedArrayData` variants, and adding it requires a per-kind
//!   `Arc::from_raw` + `HeapValue::*` arm wrapper that is itself a
//!   Phase-2c constructor surface.
//!
//! Wave-ε E-object-creation-helpers cleanup (2026-05-09): the legacy helper
//! functions at the bottom of this file (`nb_to_slot_with_field_type`,
//! `decode_field_bits_for_type`, `read_slot_nb`, `read_slot_value_typed`,
//! `clone_slots_with_update`) were pre-existing forbidden-pattern carriers
//! (took/returned `&ValueWord`, decoded via `tag_bits::is_tagged`, called
//! the deleted `ValueSlot::from_value_word` / `as_heap_nb` / `as_value_word`
//! methods). Their consumer clusters (`typed_object_ops.rs` D-typed-obj-ops,
//! `objects/mod.rs` D-objects-mod, `variables/mod.rs` B6-round-2,
//! `vm_impl/{modules,schemas}.rs` E-vm-impl-tail, `foreign_marshal.rs`
//! B-control-flow-heap) all migrated off the helpers in earlier waves —
//! either to the kinded API (KindedSlot / pop_kinded / push_kinded) or to
//! `VMError::NotImplemented(SURFACE: phase-2c)` stubs. The helpers are
//! deleted; consumers carrying the rebuilt write path live in their own
//! cluster territories per ADR-006 §2.4 (typed-Arc HeapValue payloads) and
//! §2.7.4 (Phase-2c deferral).

use crate::{
    bytecode::{Instruction, Operand},
    executor::vm_impl::stack::drop_with_kind,
    executor::VirtualMachine,
};
use rust_decimal::prelude::ToPrimitive;
use shape_runtime::type_schema::FieldType;
use shape_value::{HeapKind, NativeKind, TypedObjectStorage, ValueSlot, VMError};
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
    /// Wave-δ MR-string-misc: `Arc::new(TypedArrayData::Matrix(Arc::new(
    /// MatrixData::from_flat(...))))` + `push_kinded(_, NativeKind::Ptr(
    /// HeapKind::TypedArray))`. Element kinds are expected to be `Float64`
    /// per the compiler's matrix-emit contract; `Int64` is widened to
    /// `f64` for backward compatibility (existing emit paths sometimes
    /// push `int` literals through this op).
    pub(in crate::executor) fn op_new_matrix(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let (rows, cols) = match instruction.operand {
            Some(Operand::MatrixDims { rows, cols }) => (rows as u32, cols as u32),
            _ => return Err(VMError::InvalidOperand),
        };

        let total = (rows as usize) * (cols as usize);
        // Pop in reverse, then reverse to recover row-major source order.
        let mut popped: Vec<(u64, NativeKind)> = Vec::with_capacity(total);
        for _ in 0..total {
            match self.pop_kinded() {
                Ok(pair) => popped.push(pair),
                Err(_) => {
                    for (b, k) in popped.drain(..) {
                        drop_with_kind(b, k);
                    }
                    return Err(VMError::StackUnderflow);
                }
            }
        }
        popped.reverse();

        let mut data =
            shape_value::aligned_vec::AlignedVec::<f64>::with_capacity(total);
        for (bits, kind) in popped.iter() {
            let v = match kind {
                NativeKind::Float64 => f64::from_bits(*bits),
                NativeKind::Int64 => (*bits as i64) as f64,
                NativeKind::Bool => {
                    if *bits != 0 {
                        1.0
                    } else {
                        0.0
                    }
                }
                _ => {
                    // Heap kinds shouldn't reach a numeric matrix cell;
                    // retire all popped shares and surface a TypeError.
                    for (b, k) in popped.iter() {
                        drop_with_kind(*b, *k);
                    }
                    return Err(VMError::TypeError {
                        expected: "numeric matrix element",
                        got: "non-numeric kind",
                    });
                }
            };
            data.push(v);
            // Inline-scalar kinds — `drop_with_kind` is a no-op; explicit
            // call documents the discipline.
            drop_with_kind(*bits, *kind);
        }

        let matrix = shape_value::heap_value::MatrixData::from_flat(data, rows, cols);
        let arr = Arc::new(
            shape_value::heap_value::TypedArrayData::Matrix(Arc::new(matrix)),
        );
        let bits = Arc::into_raw(arr) as u64;
        self.push_kinded(bits, NativeKind::Ptr(HeapKind::TypedArray))
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
    /// Wave-δ MR-string-misc: post-§2.7.7 the kind track tells us the
    /// element kind directly — no runtime tag-bit classifier. If all N
    /// popped elements share `Int64` / `Float64` / `Bool`, the op
    /// constructs the matching `TypedArrayData::*` variant. Mixed-kind
    /// arrays would require `TypedArrayData::HeapValue` (heterogeneous
    /// `Arc<HeapValue>` payload — same Phase-2c surface as `op_new_array`).
    ///
    /// Empty arrays default to `TypedArrayData::I64` (an arbitrary but
    /// stable choice; the compiler is responsible for emitting the
    /// kind-specific `NewTypedArray{I64,F64,Bool}` opcodes when the
    /// element type is known at compile time).
    pub(in crate::executor) fn op_new_typed_array(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let count = match instruction.operand {
            Some(Operand::Count(c)) => c as usize,
            _ => return Err(VMError::InvalidOperand),
        };

        // Pop in reverse, then reverse to recover declared element order.
        let mut popped: Vec<(u64, NativeKind)> = Vec::with_capacity(count);
        for _ in 0..count {
            match self.pop_kinded() {
                Ok(pair) => popped.push(pair),
                Err(_) => {
                    for (b, k) in popped.drain(..) {
                        drop_with_kind(b, k);
                    }
                    return Err(VMError::StackUnderflow);
                }
            }
        }
        popped.reverse();

        // Empty array: arbitrary stable variant (I64). The compiler's
        // kind-specific NewTypedArray* opcodes pick the right variant
        // when the element type is statically known.
        if popped.is_empty() {
            let buf = shape_value::typed_buffer::TypedBuffer::<i64>::from_vec(Vec::new());
            let arr = Arc::new(
                shape_value::heap_value::TypedArrayData::I64(Arc::new(buf)),
            );
            let bits = Arc::into_raw(arr) as u64;
            return self.push_kinded(bits, NativeKind::Ptr(HeapKind::TypedArray));
        }

        // Inspect first element's kind; classify the homogeneity.
        let first_kind = popped[0].1;
        let all_match = popped.iter().all(|(_, k)| *k == first_kind);

        if !all_match {
            // Heterogeneous — would require TypedArrayData::HeapValue
            // projection. Surface per playbook §7.4: same gap as
            // op_new_array.
            for (b, k) in popped.drain(..) {
                drop_with_kind(b, k);
            }
            return Err(VMError::NotImplemented(format!(
                "op_new_typed_array({}): heterogeneous element kinds — \
                 needs TypedArrayData::HeapValue projection from (bits, \
                 kind) into Arc<HeapValue> (Phase-2c — see ADR-006 §2.7.4)",
                count
            )));
        }

        match first_kind {
            NativeKind::Int64 => {
                let mut data: Vec<i64> = Vec::with_capacity(count);
                for (bits, _kind) in popped.iter() {
                    data.push(*bits as i64);
                }
                // Inline scalars; `drop_with_kind` is a no-op for Int64.
                for (b, k) in popped.drain(..) {
                    drop_with_kind(b, k);
                }
                let buf = shape_value::typed_buffer::TypedBuffer::from_vec(data);
                let arr = Arc::new(
                    shape_value::heap_value::TypedArrayData::I64(Arc::new(buf)),
                );
                let bits = Arc::into_raw(arr) as u64;
                self.push_kinded(bits, NativeKind::Ptr(HeapKind::TypedArray))
            }
            NativeKind::Float64 => {
                let mut data =
                    shape_value::aligned_vec::AlignedVec::<f64>::with_capacity(count);
                for (bits, _kind) in popped.iter() {
                    data.push(f64::from_bits(*bits));
                }
                for (b, k) in popped.drain(..) {
                    drop_with_kind(b, k);
                }
                let buf =
                    shape_value::typed_buffer::AlignedTypedBuffer::from_aligned(data);
                let arr = Arc::new(
                    shape_value::heap_value::TypedArrayData::F64(Arc::new(buf)),
                );
                let bits = Arc::into_raw(arr) as u64;
                self.push_kinded(bits, NativeKind::Ptr(HeapKind::TypedArray))
            }
            NativeKind::Bool => {
                let mut data: Vec<u8> = Vec::with_capacity(count);
                for (bits, _kind) in popped.iter() {
                    data.push(if *bits != 0 { 1u8 } else { 0u8 });
                }
                for (b, k) in popped.drain(..) {
                    drop_with_kind(b, k);
                }
                let buf = shape_value::typed_buffer::TypedBuffer::from_vec(data);
                let arr = Arc::new(
                    shape_value::heap_value::TypedArrayData::Bool(Arc::new(buf)),
                );
                let bits = Arc::into_raw(arr) as u64;
                self.push_kinded(bits, NativeKind::Ptr(HeapKind::TypedArray))
            }
            // String element kind: each popped slot's bits are
            // `Arc::into_raw::<String>` per `pop_kinded` + the
            // `NativeKind::String` shape. Reconstruct each `Arc<String>`
            // (consuming the strong-count share) and assemble into
            // `TypedArrayData::String`. W9 MR-string-misc fill (mirrors
            // the per-element-kind retain pattern in
            // `concat.rs::concat_typed_arrays` for the `String` arm).
            NativeKind::String => {
                let mut data: Vec<Arc<String>> = Vec::with_capacity(count);
                for (bits, _kind) in popped.iter() {
                    if *bits == 0 {
                        // Defensive: a zero-bits String slot would mean a
                        // construction-side bug. Release any successful
                        // shares + surface.
                        for (b, k) in popped.drain(..) {
                            drop_with_kind(b, k);
                        }
                        return Err(VMError::RuntimeError(
                            "op_new_typed_array: zero String bits — \
                             construction-side invariant violated"
                                .to_string(),
                        ));
                    }
                    // SAFETY: kind is `NativeKind::String`; bits are
                    // `Arc::into_raw::<String>`; popped slot owns one
                    // strong-count share. `from_raw` transfers that
                    // share into the new typed buffer (where the
                    // resulting `TypedArrayData::String` Arc owns it).
                    let s: Arc<String> =
                        unsafe { Arc::from_raw(*bits as *const String) };
                    data.push(s);
                }
                // The popped shares were consumed by `Arc::from_raw`
                // above; clear `popped` without `drop_with_kind` (the
                // slots are now owned by the typed buffer).
                popped.clear();
                let buf =
                    shape_value::typed_buffer::TypedBuffer::from_vec(data);
                let arr = Arc::new(
                    shape_value::heap_value::TypedArrayData::String(Arc::new(buf)),
                );
                let bits = Arc::into_raw(arr) as u64;
                self.push_kinded(bits, NativeKind::Ptr(HeapKind::TypedArray))
            }
            // Other heap-kind / scalar-kind element-arrays (Char,
            // Decimal, BigInt, TypedObject, …) require the
            // `TypedArrayData::HeapValue` projection — same Phase-2c
            // dependency as `op_new_array`'s heterogeneous case (the
            // `Arc<HeapValue>`-arm wrapper that today's emit path
            // doesn't supply per-`(bits, kind)`).
            other => {
                for (b, k) in popped.drain(..) {
                    drop_with_kind(b, k);
                }
                Err(VMError::NotImplemented(format!(
                    "op_new_typed_array({}): element kind {:?} — needs \
                     per-kind TypedArrayData variant construction (Phase-\
                     2c — see ADR-006 §2.7.4); the per-kind \
                     NewTypedArray* opcodes already cover I64/F64/Bool \
                     in `dispatch.rs`.",
                    count, other
                )))
            }
        }
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
    // FieldType::F64 / Decimal: schema demands inline f64 storage.
    // Pre-bulldozer behaviour is lossy for Arc<Decimal> inputs; preserve
    // that here so existing read-back consumers still work. The popped
    // Decimal Arc share is released after we materialise its f64
    // projection.
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
    // FieldType. The popped `bits` carry the raw native payload
    // directly; we rewrap via `ValueSlot::from_*`.
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
        // read path reconstructs the appropriate shape; preserving the
        // raw bits is the lossless round-trip per the existing
        // pre-bulldozer behaviour. heap_mask remains 0 — the value is
        // inline.
        Some(FieldType::Any) | None | Some(_) => (ValueSlot::from_raw(bits), false),
    }
}
