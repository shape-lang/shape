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
//!   elements share `Int64` / `Float64` / `Bool` / `String` kind, builds
//!   the matching `TypedArrayData::*` variant. Mixed-kind input still
//!   surfaces here (Round 11A landed the kinded reentry for `op_new_array`
//!   only — `op_new_typed_array`'s mixed-kind arm could route through
//!   the same `slot_to_heap_arc` + `TypedArrayData::build_specialized_from_heap_arcs`
//!   helpers but is left as a follow-up to keep the surface diff small).
//!
//! Round 11A (ADR-006 §2.7.24 Q25.A, 2026-05-13): `op_new_array` migrates
//! from the `NotImplemented(SURFACE)` shape to a kinded body. The
//! per-element-kind dispatch consults the §Q25.A monomorphic
//! `TypedArrayData::*` variant grid (I64 / F64 / Bool / String / Decimal
//! / BigInt / TypedObject / Char / etc.); heterogeneous-kind input
//! routes through `slot_to_heap_arc` +
//! `TypedArrayData::build_specialized_from_heap_arcs`. The deleted
//! `TypedArrayData::HeapValue` polymorphic catch-all stays deleted —
//! homogeneous-arm `HeapValue` input is the only catch-all path and it
//! goes through the build helper, not through a resurrected variant.
//!
//! One opcode body remains Phase-2c surface:
//!
//! - **`op_new_object`** still depends on the deleted
//!   `create_typed_object_from_pairs` (`vm_impl/schemas.rs` ValueWord-
//!   shaped helper, retired by Phase 2c per ADR-006 §2.7.4 / Q5).
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
    executor::builtins::array_ops::slot_to_heap_arc,
    executor::vm_impl::stack::drop_with_kind,
    executor::VirtualMachine,
};
use rust_decimal::prelude::ToPrimitive;
use shape_runtime::type_schema::FieldType;
use shape_value::heap_value::TypedArrayData;
use shape_value::{
    AlignedTypedBuffer, AlignedVec, HeapKind, KindedSlot, NativeKind, TypedBuffer,
    TypedObjectStorage, ValueSlot, VMError,
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

    /// Create a generic Array from N stack elements.
    ///
    /// ADR-006 §2.7.24 Q25.A (typed-carrier monomorphization bundle):
    /// the deleted `TypedArrayData::HeapValue(Arc<TypedBuffer<Arc<
    /// HeapValue>>>)` polymorphic catch-all is replaced by per-element-
    /// kind specialized variants (`I64`, `F64`, `Bool`, `String`,
    /// `Decimal`, `BigInt`, `DateTime`, `Timespan`, `Duration`,
    /// `Instant`, `Char`, `TypedObject`, `TraitObject`) plus the
    /// `Arc<HeapValue>` projection helper
    /// `TypedArrayData::build_specialized_from_heap_arcs` for cross-arm
    /// heterogeneous input (the only catch-all path is now homogeneous-
    /// `HeapValue`-arm input dispatched by the helper, NOT the deleted
    /// polymorphic carrier).
    ///
    /// Path discipline (ADR-006 §2.7.5 / §2.7.24 Q25.A):
    /// - **Empty** (`Count(0)`): default to `TypedArrayData::I64` with
    ///   an empty buffer. Matches `op_new_typed_array`'s stable empty
    ///   default; the compiler is responsible for emitting kind-specific
    ///   `NewTypedArray*` opcodes when the element type is statically
    ///   known.
    /// - **Homogeneous kind**: dispatch directly to the matching
    ///   specialized variant via per-kind construction (inline scalars
    ///   build a `TypedBuffer<T>` over the popped bits; heap-kinded
    ///   slots clone the typed `Arc<T>` shares out of
    ///   `slot.as_heap_value()` and assemble into the matching
    ///   `TypedArrayData::*(Arc<TypedBuffer<Arc<T>>>)` arm).
    /// - **Heterogeneous kind**: project each popped slot to
    ///   `Arc<HeapValue>` via `slot_to_heap_arc`, then route through
    ///   `TypedArrayData::build_specialized_from_heap_arcs`. Cross-arm
    ///   input surfaces as `VMError::RuntimeError` per Q25.A "Arrays do
    ///   not [admit heterogeneous slots]" — NOT `NotImplemented(SURFACE)`,
    ///   it is a user-facing kind-mismatch.
    ///
    /// Refcount discipline: every popped `(bits, kind)` share either
    /// transfers into the resulting `TypedArrayData::*` arm (heap-kinded
    /// elements) or is consumed inline (inline scalars are
    /// `drop_with_kind`-noop). The error paths drain remaining popped
    /// shares via `drop_with_kind` before returning.
    pub(in crate::executor) fn op_new_array(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let count = match instruction.operand {
            Some(Operand::Count(c)) => c as usize,
            _ => return Err(VMError::InvalidOperand),
        };

        // Pop in reverse-push order, then reverse to recover declared
        // source order. On any pop failure mid-way, retire the already-
        // popped shares to preserve refcount discipline.
        let mut popped: Vec<(u64, NativeKind)> = Vec::with_capacity(count);
        for _ in 0..count {
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

        // Empty-array stable default. ADR-006 §2.7.24 Q25.A: per-variant
        // uniform element kind; empty has no element kind so we pick a
        // stable default. Matches `op_new_typed_array`'s I64 default
        // (object_creation.rs:366-373). The compiler emits kind-specific
        // `NewTypedArray*` opcodes when the element type is known.
        if popped.is_empty() {
            let buf: TypedBuffer<i64> = TypedBuffer::from_vec(Vec::new());
            let arr = Arc::new(TypedArrayData::I64(Arc::new(buf)));
            let bits = Arc::into_raw(arr) as u64;
            return self.push_kinded(bits, NativeKind::Ptr(HeapKind::TypedArray));
        }

        // Classify homogeneity. Homogeneous-kind input dispatches to the
        // matching specialized variant directly without an Arc<HeapValue>
        // round-trip; heterogeneous input goes through
        // `build_specialized_from_heap_arcs`.
        let first_kind = popped[0].1;
        let all_match = popped.iter().all(|(_, k)| *k == first_kind);

        if all_match {
            return Self::build_homogeneous_typed_array(&mut popped, first_kind)
                .and_then(|arr| {
                    let bits = Arc::into_raw(Arc::new(arr)) as u64;
                    self.push_kinded(bits, NativeKind::Ptr(HeapKind::TypedArray))
                });
        }

        // Heterogeneous-kind path. Project each slot to `Arc<HeapValue>`
        // via `slot_to_heap_arc`, then route through
        // `build_specialized_from_heap_arcs`. The projection bumps
        // refcount shares on heap kinds (the popped shares remain
        // owned and are retired via drop_with_kind after).
        let mut elems: Vec<Arc<shape_value::HeapValue>> = Vec::with_capacity(popped.len());
        for (bits, kind) in popped.iter() {
            let slot = KindedSlot::new(ValueSlot::from_raw(*bits), *kind);
            // slot_to_heap_arc clones the underlying Arc on heap kinds
            // (its body uses Arc::increment_strong_count / Arc::clone)
            // — the popped share remains owned and is retired below.
            let arc_result = slot_to_heap_arc(&slot);
            // The slot carrier was constructed from raw bits; we did not
            // transfer ownership into it (popped owns the share). Forget
            // it so its Drop does not double-release. The popped slot's
            // share will be retired via drop_with_kind in the cleanup
            // loop after the elems vec is assembled.
            std::mem::forget(slot);
            match arc_result {
                Ok(arc) => elems.push(arc),
                Err(e) => {
                    // Cleanup: retire all popped shares and any
                    // already-projected arcs (those are clones; their
                    // own Drop retires the bumped shares).
                    for (b, k) in popped.drain(..) {
                        drop_with_kind(b, k);
                    }
                    return Err(e);
                }
            }
        }
        // The popped slots' original shares are independent from the
        // cloned shares in `elems`. Retire them now.
        for (b, k) in popped.drain(..) {
            drop_with_kind(b, k);
        }

        let arr = TypedArrayData::build_specialized_from_heap_arcs(elems)
            .map_err(VMError::RuntimeError)?;
        let bits = Arc::into_raw(Arc::new(arr)) as u64;
        self.push_kinded(bits, NativeKind::Ptr(HeapKind::TypedArray))
    }

    /// Build a `TypedArrayData` from a homogeneous-kind popped element
    /// vector (ADR-006 §2.7.24 Q25.A specialized variants). Per-kind
    /// dispatch mirrors `op_new_typed_array`'s body for inline scalars
    /// (Int64 / Float64 / Bool / String) and `Array.filled`'s heap-
    /// element handling (Decimal / BigInt / TypedObject / Char etc.).
    ///
    /// Element shares: inline scalars are inline (no Arc), heap-kinded
    /// slots transfer ownership via `Arc::from_raw` — caller MUST clear
    /// the popped vec (without `drop_with_kind`) on success so the
    /// shares are not double-released. On error, caller must
    /// `drop_with_kind` the remaining popped entries.
    fn build_homogeneous_typed_array(
        popped: &mut Vec<(u64, NativeKind)>,
        kind: NativeKind,
    ) -> Result<TypedArrayData, VMError> {
        let count = popped.len();
        match kind {
            NativeKind::Int64 => {
                let mut data: Vec<i64> = Vec::with_capacity(count);
                for (bits, _kind) in popped.iter() {
                    data.push(*bits as i64);
                }
                // Inline scalars: drop_with_kind is a no-op for Int64.
                for (b, k) in popped.drain(..) {
                    drop_with_kind(b, k);
                }
                Ok(TypedArrayData::I64(Arc::new(TypedBuffer::from_vec(data))))
            }
            NativeKind::Float64 => {
                let mut data = AlignedVec::<f64>::with_capacity(count);
                for (bits, _kind) in popped.iter() {
                    data.push(f64::from_bits(*bits));
                }
                for (b, k) in popped.drain(..) {
                    drop_with_kind(b, k);
                }
                Ok(TypedArrayData::F64(Arc::new(
                    AlignedTypedBuffer::from_aligned(data),
                )))
            }
            NativeKind::Bool => {
                let mut data: Vec<u8> = Vec::with_capacity(count);
                for (bits, _kind) in popped.iter() {
                    data.push(if *bits != 0 { 1u8 } else { 0u8 });
                }
                for (b, k) in popped.drain(..) {
                    drop_with_kind(b, k);
                }
                Ok(TypedArrayData::Bool(Arc::new(TypedBuffer::from_vec(data))))
            }
            NativeKind::String => {
                // Each popped slot's bits are `Arc::into_raw::<String>`.
                // Reconstruct each Arc<String>, consuming the strong-
                // count share, and assemble into TypedArrayData::String.
                // Same per-element-kind retain pattern as the W9
                // MR-string-misc fill in op_new_typed_array's String arm
                // (object_creation.rs:443-486).
                let mut data: Vec<Arc<String>> = Vec::with_capacity(count);
                for (bits, _kind) in popped.iter() {
                    if *bits == 0 {
                        // Defensive: a zero-bits String slot is a
                        // construction-site bug; surface for diagnosis.
                        // Release any successfully consumed shares is
                        // not possible here (Arc::from_raw above already
                        // consumed them); the partial data Vec drops
                        // them all on the return path.
                        return Err(VMError::RuntimeError(
                            "op_new_array: zero String bits — \
                             construction-side invariant violated"
                                .to_string(),
                        ));
                    }
                    // SAFETY: kind is `NativeKind::String`; bits are
                    // `Arc::into_raw::<String>`; the popped slot owns
                    // one strong-count share. `from_raw` transfers it
                    // into the new typed buffer.
                    let s: Arc<String> =
                        unsafe { Arc::from_raw(*bits as *const String) };
                    data.push(s);
                }
                // Shares were consumed by `Arc::from_raw` above; clear
                // popped without drop_with_kind.
                popped.clear();
                Ok(TypedArrayData::String(Arc::new(TypedBuffer::from_vec(data))))
            }
            // Heap-kinded specialized variants per ADR-006 §2.7.24 Q25.A.
            // Each popped slot's bits are `Arc::into_raw::<T>` for the
            // matching `T` (per the producing-call-site classification
            // discipline at §2.7.5); reconstruct, transfer into the
            // typed buffer, clear popped without drop_with_kind.
            NativeKind::Ptr(HeapKind::Decimal) => {
                let mut data: Vec<Arc<rust_decimal::Decimal>> =
                    Vec::with_capacity(count);
                for (bits, _kind) in popped.iter() {
                    if *bits == 0 {
                        return Err(VMError::RuntimeError(
                            "op_new_array: zero Decimal bits — \
                             construction-side invariant violated"
                                .to_string(),
                        ));
                    }
                    // SAFETY: bits are `Arc::into_raw::<Decimal>` per the
                    // §2.7.6/Q8 KindedSlot::from_decimal contract.
                    let d: Arc<rust_decimal::Decimal> = unsafe {
                        Arc::from_raw(*bits as *const rust_decimal::Decimal)
                    };
                    data.push(d);
                }
                popped.clear();
                Ok(TypedArrayData::Decimal(Arc::new(TypedBuffer::from_vec(data))))
            }
            NativeKind::Ptr(HeapKind::BigInt) => {
                let mut data: Vec<Arc<i64>> = Vec::with_capacity(count);
                for (bits, _kind) in popped.iter() {
                    if *bits == 0 {
                        return Err(VMError::RuntimeError(
                            "op_new_array: zero BigInt bits — \
                             construction-side invariant violated"
                                .to_string(),
                        ));
                    }
                    // SAFETY: bits are `Arc::into_raw::<i64>` per the
                    // §2.7.6/Q8 KindedSlot::from_bigint contract.
                    let b: Arc<i64> =
                        unsafe { Arc::from_raw(*bits as *const i64) };
                    data.push(b);
                }
                popped.clear();
                Ok(TypedArrayData::BigInt(Arc::new(TypedBuffer::from_vec(data))))
            }
            NativeKind::Ptr(HeapKind::TypedObject) => {
                let mut data: Vec<Arc<TypedObjectStorage>> =
                    Vec::with_capacity(count);
                for (bits, _kind) in popped.iter() {
                    if *bits == 0 {
                        return Err(VMError::RuntimeError(
                            "op_new_array: zero TypedObject bits — \
                             construction-side invariant violated"
                                .to_string(),
                        ));
                    }
                    // SAFETY: bits are `Arc::into_raw::<TypedObjectStorage>`
                    // per `KindedSlot::from_typed_object`.
                    let o: Arc<TypedObjectStorage> = unsafe {
                        Arc::from_raw(*bits as *const TypedObjectStorage)
                    };
                    data.push(o);
                }
                popped.clear();
                Ok(TypedArrayData::TypedObject(Arc::new(
                    TypedBuffer::from_vec(data),
                )))
            }
            NativeKind::Ptr(HeapKind::Char) => {
                let mut data: Vec<char> = Vec::with_capacity(count);
                for (bits, _kind) in popped.iter() {
                    // Char is inline (the bits ARE the codepoint per
                    // §2.7.6/Q8 KindedSlot::from_char). No Arc share.
                    let c = char::from_u32(*bits as u32).ok_or_else(|| {
                        VMError::RuntimeError(
                            "op_new_array: invalid char codepoint in slot bits"
                                .to_string(),
                        )
                    })?;
                    data.push(c);
                }
                // Inline kind; drop_with_kind is a no-op for
                // Ptr(HeapKind::Char) (per the kinded_slot drop dispatch
                // for the Char inline arm).
                for (b, k) in popped.drain(..) {
                    drop_with_kind(b, k);
                }
                Ok(TypedArrayData::Char(Arc::new(TypedBuffer::from_vec(data))))
            }
            // Other heap-kinded homogeneous arrays (Temporal-family,
            // Instant, TraitObject, etc.) — recover the typed Arc share
            // from each slot's `slot.as_heap_value()` projection and
            // route through `build_specialized_from_heap_arcs`. This is
            // a slow path (Arc<HeapValue> wrapper allocation per element)
            // but correctness-first; if it becomes hot, monomorphize
            // additional arms here.
            other => {
                // Project each slot to Arc<HeapValue> for the helper.
                let mut elems: Vec<Arc<shape_value::HeapValue>> =
                    Vec::with_capacity(count);
                for (bits, kind) in popped.iter() {
                    let slot = KindedSlot::new(ValueSlot::from_raw(*bits), *kind);
                    let arc_result = slot_to_heap_arc(&slot);
                    std::mem::forget(slot);
                    match arc_result {
                        Ok(arc) => elems.push(arc),
                        Err(e) => {
                            // Caller will drop_with_kind the remaining
                            // popped entries on Err return.
                            let _ = other; // suppress unused-binding lint
                            return Err(e);
                        }
                    }
                }
                for (b, k) in popped.drain(..) {
                    drop_with_kind(b, k);
                }
                TypedArrayData::build_specialized_from_heap_arcs(elems)
                    .map_err(VMError::RuntimeError)
            }
        }
    }

    /// Create a typed array (IntArray/FloatArray/BoolArray) from N elements on the stack.
    ///
    /// Wave-δ MR-string-misc: post-§2.7.7 the kind track tells us the
    /// element kind directly — no runtime tag-bit classifier. If all N
    /// popped elements share `Int64` / `Float64` / `Bool`, the op
    /// constructs the matching `TypedArrayData::*` variant. Mixed-kind
    /// arrays would require `the-deleted-heterogeneous-element-carrier` (heterogeneous
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
            // Heterogeneous — would require the-deleted-heterogeneous-element-carrier
            // projection. Surface per playbook §7.4: same gap as
            // op_new_array.
            for (b, k) in popped.drain(..) {
                drop_with_kind(b, k);
            }
            return Err(VMError::NotImplemented(format!(
                "op_new_typed_array({}): heterogeneous element kinds — \
                 needs the-deleted-heterogeneous-element-carrier projection from (bits, \
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
            // `the-deleted-heterogeneous-element-carrier` projection — same Phase-2c
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
