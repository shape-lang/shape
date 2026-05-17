//! Object creation operations (NewArray, NewObject, NewTypedObject)
//!
//! Handles allocation and initialization of arrays, objects, and typed objects.
//!
//! ## V3-S5 ckpt-5 consumer-cascade tier 3 surface (2026-05-15)
//!
//! Per V3-S5 ckpt-1..ckpt-4 cascade (commits `aac8495e` /
//! `b38fbd3c` / `30c40f51` / `654c7202`, 2026-05-15) the
//! `TypedArrayData` enum + impl blocks + `Display for TypedArrayData` +
//! `typed_array_structural_eq` fn + `HeapValue::TypedArray(Arc<TypedArrayData>)`
//! outer arm + `HeapKind::TypedArray = 8` ordinal + `TypedBuffer<T>` /
//! `AlignedTypedBuffer` wrapper layer were DELETED at
//! `crates/shape-value/src/heap_value.rs` + `heap_variants.rs` +
//! `typed_buffer.rs` per W12-typed-array-data-deletion audit §3.5 + §3.6
//! + ADR-006 §2.7.24 Q25.A SUPERSEDED.
//!
//! This file's `op_new_array` + `op_new_typed_array` constructors previously
//! built `Arc<TypedArrayData>` carriers and pushed them with
//! `NativeKind::Ptr(HeapKind::TypedArray)`. Both carriers are gone.
//! Bodies are replaced with structured surface-and-stop via
//! `ckpt5_surface(op, args)`; the `build_homogeneous_typed_array` helper
//! (a `TypedArrayData` producer) is DELETED. The `TypedArrayData` /
//! `TypedBuffer` / `AlignedTypedBuffer` imports are removed.
//!
//! ## Preserved entry-points (no `TypedArrayData` dependency)
//!
//! - `op_new_typed_object` — constructs `Arc<TypedObjectStorage>` via
//!   `TypedObjectStorage::_new`, pushes `Ptr(HeapKind::TypedObject)`.
//!   Independent of the array carrier hierarchy.
//! - `op_new_object` — surfaces `NotImplemented` (Phase-2c, depends on
//!   deleted ValueWord-shaped `create_typed_object_from_pairs`).
//! - `op_new_matrix` — constructs `Arc<MatrixData>` per Round 18 S3
//!   ADR-006 §2.7.22 amendment, pushes `Ptr(HeapKind::Matrix)`.
//!   Independent of the array carrier hierarchy.
//! - `kinded_to_slot` / `field_type_to_int_width` — TypedObject field
//!   construction helpers. No `TypedArrayData` dependency.
//!
//! ## Cascade migration target (post-ckpt-6 STRICT close)
//!
//! Per W12-typed-array-data-deletion audit §A.3 + §1.2 + §2.2 + §3.1
//! scalar recipe: every previous `TypedArrayData::X(buf)` match arm
//! migrates to the v2-raw `TypedArray<T>` flat-struct carrier:
//!
//! | Previous arm | Post-deletion target |
//! |---|---|
//! | `TypedArrayData::I64(buf)` | `*mut TypedArray<i64>` direct access |
//! | `TypedArrayData::F64(buf)` | `*mut TypedArray<f64>` direct access |
//! | `TypedArrayData::Bool(buf)` | `*mut TypedArray<u8>` direct access |
//! | `TypedArrayData::String(buf)` | `*mut TypedArray<*const StringObj>` |
//! | `TypedArrayData::Decimal(buf)` | `*mut TypedArray<*const DecimalObj>` |
//! | `TypedArrayData::TypedObject(buf)` | `TypedArray<TypedObjectPtr>` (D4 Path B) |
//! | `TypedArrayData::TraitObject(buf)` | `TypedArray<TraitObjectPtr>` (D4 Path B) |
//! | `TypedArrayData::Char(buf)` | `TypedArray<char>` direct |
//! | `TypedArrayData::I8/I16/I32/U8/U16/U32/U64/F32(buf)` | new `TypedArray<T>` monomorphizations |
//! | `TypedArrayData::BigInt(buf)` | DEFERRED to cluster-1+ (audit Obstacle 3 R19 defer) |
//!
//! Refusal #1 binding: TypedArrayData resurrection under any rename
//! (`TypedArrayKind` / `TypedArrayCarrier` / `TypedBuffer<T>` wrapper) is
//! refused on sight.
//!
//! Cascade-broken cross-module helpers picked up at ckpt-1..ckpt-4 close.
//! Production cascade lands at ckpt-6 STRICT close per the multi-session
//! chain pattern step 5.

use crate::{
    bytecode::{Instruction, Operand},
    executor::vm_impl::stack::drop_with_kind,
    executor::VirtualMachine,
};
use rust_decimal::prelude::ToPrimitive;
use shape_runtime::type_schema::FieldType;
use shape_value::{
    HeapKind, KindedSlot, NativeKind, TypedObjectStorage, ValueSlot, VMError,
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

// ═══════════════════════════════════════════════════════════════════════════
// V3-S5 ckpt-5 surface-and-stop builder
// ═══════════════════════════════════════════════════════════════════════════

/// Common surface-and-stop error for the two array constructors in this file.
/// Returns a structured `VMError::NotImplemented` citing the V3-S5 ckpt-5
/// consumer-cascade tier 3 state: the previous `TypedArrayData` /
/// `TypedBuffer<T>` / `AlignedTypedBuffer` carriers + the outer
/// `HeapValue::TypedArray(Arc<TypedArrayData>)` arm + the
/// `HeapKind::TypedArray=8` ordinal are GONE; the v2-raw `TypedArray<T>`
/// flat-struct migration lands across ckpt-5-prime (wire/marshal/json +
/// 4-table lockstep + U64 relabel) + ckpt-5-prime² (storage migration +
/// 10 intrinsics marshal-parameter migration) + ckpt-6 (JIT FFI +
/// STRICT close gate).
#[cold]
#[inline(never)]
fn ckpt5_surface(op: &'static str, count: usize) -> VMError {
    VMError::NotImplemented(format!(
        "{op}({count}): SURFACE — V3-S5 ckpt-5 consumer-cascade tier 3 \
         surface. The deleted typed-array-data enum + `Buf<T>` / \
         aligned-typed-buf wrapper layer + outer `HeapValue::TypedArray(\
         Arc<_>)` arm + `HeapKind::TypedArray=8` ordinal \
         DELETED across V3-S5 ckpt-1..ckpt-4 per W12-typed-array-data-\
         deletion audit §3.5 + §3.6 + ADR-006 §2.7.24 Q25.A SUPERSEDED. \
         Post-deletion target is per-T v2-raw `TypedArray<T>` flat-struct \
         monomorphization per audit §A.3 + §3.1 scalar recipe + §2.2 \
         heap-element variants. Construction-site rebuild lands at ckpt-6 \
         STRICT close after ckpt-5-prime (wire/marshal/json + 4-table \
         lockstep) + ckpt-5-prime² (storage migration + 10 intrinsics \
         marshal-parameter migration). REFUSED ON SIGHT: TypedArrayData \
         resurrection under any rename (Refusal #1).",
        op = op,
        count = count,
    ))
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

        // Wave 2 Round 4 D4 ckpt-1: migrated to v2-raw `_new`. The raw
        // pointer is directly the stack carrier bits per ADR-006 §2.4 /
        // D1's `from_typed_object_raw` contract. No variant signature
        // dependency at this site.
        let ptr = TypedObjectStorage::_new(
            schema_id as u64,
            slots.into_boxed_slice(),
            heap_mask,
            Arc::from(field_kinds.into_boxed_slice()),
        );
        let bits = ptr as u64;
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
    /// ADR-006 §2.7.22 amendment (Round 18 S3 W12-matrix-floatslice-heapkind
    /// -exit, 2026-05-13): the construction site pushes
    /// `Arc<MatrixData>` directly under kind `Ptr(HeapKind::Matrix)`. The
    /// pre-amendment shape (`Arc<TypedArrayData::Matrix(Arc<MatrixData>)>`
    /// under `Ptr(HeapKind::TypedArray)`) is retired. Element kinds are
    /// expected to be `Float64` per the compiler's matrix-emit contract;
    /// `Int64` is widened to `f64` for backward compatibility (existing
    /// emit paths sometimes push `int` literals through this op).
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
        let arc = Arc::new(matrix);
        let bits = Arc::into_raw(arc) as u64;
        self.push_kinded(bits, NativeKind::Ptr(HeapKind::Matrix))
    }

    /// Create a generic Array from N stack elements.
    ///
    /// ## V3-S5 ckpt-5 surface (2026-05-15)
    ///
    /// The pre-ckpt-1 body built `Arc<TypedArrayData>` via
    /// per-element-kind specialized variants (`I64` / `F64` / `Bool` /
    /// `String` / `Decimal` / `BigInt` / `DateTime` / `Timespan` /
    /// `Duration` / `Instant` / `Char` / `TypedObject` / `TraitObject`)
    /// and pushed `Ptr(HeapKind::TypedArray)`. Both the variant grid and
    /// the outer `HeapValue::TypedArray` arm + `HeapKind::TypedArray`
    /// ordinal are DELETED across V3-S5 ckpt-1..ckpt-4. Construction-site
    /// rebuild lands at ckpt-6 STRICT close per the per-T v2-raw
    /// `TypedArray<T>` monomorphization migration target.
    ///
    /// Refcount discipline: every popped `(bits, kind)` share is retired
    /// via `drop_with_kind` before the surface returns — the stack ABI
    /// `data.len() == kinds.len()` invariant + per-slot share-release
    /// discipline (playbook §7 #4) is preserved.
    pub(in crate::executor) fn op_new_array(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let count = match instruction.operand {
            Some(Operand::Count(c)) => c as usize,
            _ => return Err(VMError::InvalidOperand),
        };

        // Drain the popped shares to preserve stack discipline before
        // surfacing. drop_with_kind retires each share through the
        // matching kinded path; inline scalars are no-ops.
        for _ in 0..count {
            if let Ok((b, k)) = self.pop_kinded() {
                drop_with_kind(b, k);
            } else {
                return Err(VMError::StackUnderflow);
            }
        }

        Err(ckpt5_surface("op_new_array", count))
    }

    /// Create a typed array (IntArray/FloatArray/BoolArray) from N elements
    /// on the stack.
    ///
    /// ## V3-S5 ckpt-5 surface (2026-05-15)
    ///
    /// The pre-ckpt-1 body built `Arc<TypedArrayData::{I64,F64,Bool,
    /// String}>` from homogeneous-kind popped elements and pushed
    /// `Ptr(HeapKind::TypedArray)`. Both the variants and the outer
    /// `HeapValue::TypedArray` arm + `HeapKind::TypedArray=8` ordinal are
    /// DELETED across V3-S5 ckpt-1..ckpt-4. Construction-site rebuild
    /// lands at ckpt-6 STRICT close per the per-T v2-raw `TypedArray<T>`
    /// monomorphization migration target — the compiler's kind-specific
    /// `NewTypedArray{I64,F64,Bool,String}` opcodes pick the right variant
    /// when the element type is statically known.
    ///
    /// Refcount discipline: every popped `(bits, kind)` share is retired
    /// via `drop_with_kind` before the surface returns.
    pub(in crate::executor) fn op_new_typed_array(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let count = match instruction.operand {
            Some(Operand::Count(c)) => c as usize,
            _ => return Err(VMError::InvalidOperand),
        };

        for _ in 0..count {
            if let Ok((b, k)) = self.pop_kinded() {
                drop_with_kind(b, k);
            } else {
                return Err(VMError::StackUnderflow);
            }
        }

        Err(ckpt5_surface("op_new_typed_array", count))
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

// Suppress unused import lint — KindedSlot is reserved for forward-port of
// the v2-raw rebuilt array constructors at ckpt-6 STRICT close.
#[allow(dead_code)]
fn _ckpt5_reserved_kinded_slot(_: KindedSlot) {}
