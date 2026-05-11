//! TypedObject operations for the VM.
//!
//! ADR-006 §2.7.6 / §2.7.7 / §2.7.8 + Q7-Q10 — Wave 6.5 substep-2 cluster
//! `D-typed-obj-ops`. Receiver kind is `NativeKind::Ptr(HeapKind::TypedObject)`;
//! heap dispatch goes through `ValueSlot::as_heap_value()` +
//! `HeapValue::TypedObject(arc)` match (Q8 — no per-heap-variant accessors on
//! `KindedSlot`). Field-access opcodes carry per-field FieldType operands
//! that supply the loaded value's `NativeKind` via the `field_type_tag` →
//! `NativeKind` mapping in [`field_tag_to_heap_native_kind`] / inline
//! match arms in [`push_field_value`].
//!
//! Forbidden patterns (CLAUDE.md "Forbidden Patterns" + playbook §4):
//! the deleted dynamic-word runtime construction, the deleted raw-helper
//! tag_bits dispatch, and the deleted Wave 6.0 transitional shim layer
//! have all been migrated off this file.
//!
//! `op_set_field_typed` write-path was rebuilt by
//! W17-typed-object-mutation (2026-05-11) on top of
//! `TypedObjectStorage::write_slot_in_place` (the kinded in-place
//! projection writer landed by W17-references-mutation `30b9ebf`,
//! ADR-006 §2.7.13 / Q14). The legacy `clone_slots_with_update` shape
//! is intentionally NOT resurrected — `Arc::make_mut` on a shared
//! `Arc<TypedObjectStorage>` is fundamentally incompatible with the
//! ref-projection invariant that prompted the kinded in-place writer
//! (refcount > 1 by construction; the struct is intentionally not
//! `Clone`). Remaining `NotImplemented(SURFACE)` sites in this file
//! (`push_field_value` arms for impossible non-heap/heap-with-Any
//! shapes; soundness gap when receiver kind says TypedObject but the
//! HeapValue arm disagrees) are defensive surfaces for construction-
//! side bugs, not migration cascades.

use crate::bytecode::{Instruction, Operand};
use crate::executor::vm_impl::stack::{clone_with_kind, drop_with_kind};
use shape_runtime::type_schema::FieldType;
use shape_value::heap_value::HeapValue;
use shape_value::{HeapKind, NativeKind, VMError, ValueSlot};

/// Compile-time field type tags for zero-cost field access.
/// Stored in `Operand::TypedField::field_type_tag` so the executor
/// can interpret slot bits without a runtime schema lookup.
pub const FIELD_TAG_F64: u16 = 0;
pub const FIELD_TAG_I64: u16 = 1;
pub const FIELD_TAG_BOOL: u16 = 2;
pub const FIELD_TAG_STRING: u16 = 3;
pub const FIELD_TAG_TIMESTAMP: u16 = 4;
pub const FIELD_TAG_ARRAY: u16 = 5;
pub const FIELD_TAG_OBJECT: u16 = 6;
pub const FIELD_TAG_DECIMAL: u16 = 7;
pub const FIELD_TAG_ANY: u16 = 8;
pub const FIELD_TAG_UNKNOWN: u16 = 255;

/// Encode a FieldType as a compact u16 tag for the operand.
pub fn field_type_to_tag(ft: &FieldType) -> u16 {
    match ft {
        FieldType::F64 => FIELD_TAG_F64,
        FieldType::I64 => FIELD_TAG_I64,
        FieldType::Bool => FIELD_TAG_BOOL,
        FieldType::String => FIELD_TAG_STRING,
        FieldType::Timestamp => FIELD_TAG_TIMESTAMP,
        FieldType::Array(_) => FIELD_TAG_ARRAY,
        FieldType::Object(_) => FIELD_TAG_OBJECT,
        FieldType::Decimal => FIELD_TAG_DECIMAL,
        FieldType::Any => FIELD_TAG_ANY,
        // Width integer types stored as I64 in the slot bits.
        FieldType::I8
        | FieldType::U8
        | FieldType::I16
        | FieldType::U16
        | FieldType::I32
        | FieldType::U32
        | FieldType::U64 => FIELD_TAG_I64,
    }
}

/// Convert a `field_type_tag` back to a `FieldType` (used by write-path
/// `clone_slots_with_update`, which is owned by sibling cluster
/// `D-obj-create`). Pure schema-tag mapping — no runtime dynamic-word shape.
pub(in crate::executor) fn tag_to_field_type(tag: u16) -> Option<FieldType> {
    match tag {
        FIELD_TAG_F64 => Some(FieldType::F64),
        FIELD_TAG_I64 => Some(FieldType::I64),
        FIELD_TAG_BOOL => Some(FieldType::Bool),
        FIELD_TAG_STRING => Some(FieldType::String),
        FIELD_TAG_TIMESTAMP => Some(FieldType::Timestamp),
        FIELD_TAG_ARRAY => Some(FieldType::Array(Box::new(FieldType::Any))),
        FIELD_TAG_OBJECT => Some(FieldType::Object(String::new())),
        FIELD_TAG_DECIMAL => Some(FieldType::Decimal),
        FIELD_TAG_ANY => Some(FieldType::Any),
        _ => None,
    }
}

/// Map a heap-backed `field_type_tag` to its `NativeKind` (ADR-006 §2.7.7
/// — kinded API receives the kind alongside the bits; the deleted
/// tag_bits dispatch never runs at the consumer). For tags whose heap arm
/// is unambiguous, returns `Some(kind)`; for `FIELD_TAG_ANY` /
/// `FIELD_TAG_UNKNOWN` (dynamic), returns `None`.
#[inline]
fn field_tag_to_heap_native_kind(tag: u16) -> Option<NativeKind> {
    match tag {
        FIELD_TAG_STRING => Some(NativeKind::String),
        FIELD_TAG_ARRAY => Some(NativeKind::Ptr(HeapKind::TypedArray)),
        FIELD_TAG_OBJECT => Some(NativeKind::Ptr(HeapKind::TypedObject)),
        FIELD_TAG_DECIMAL => Some(NativeKind::Ptr(HeapKind::Decimal)),
        // FIELD_TAG_TIMESTAMP heap-backed → Temporal payload.
        FIELD_TAG_TIMESTAMP => Some(NativeKind::Ptr(HeapKind::Temporal)),
        // Tags below are non-heap (inline scalar); exposing them here is a
        // construction-side bug — caller must check `is_heap` first.
        FIELD_TAG_F64 | FIELD_TAG_I64 | FIELD_TAG_BOOL => None,
        // Dynamic / unknown — ADR-006 §2.7.7: no statically-sourceable kind.
        _ => None,
    }
}

/// Map any `field_type_tag` (heap-backed or inline scalar) to its
/// `NativeKind`. ADR-006 §2.7.13 / Q14 (Wave 8 W8-T26): the
/// `MakeFieldRef` projection captures the projected slot's kind on the
/// `RefTarget::TypedField` variant; the kind-source is the operand-
/// encoded `field_type_tag`, and the `MakeFieldRef` path needs the
/// inline-scalar arms too (a `&obj.bool_field` ref carries
/// `kind = NativeKind::Bool`, etc.).
///
/// Returns `None` for `FIELD_TAG_ANY` / `FIELD_TAG_UNKNOWN` (no
/// statically-sourceable kind — caller surfaces per playbook §7
/// REVISED #4, no Bool-default fallback per §2.7.7 #9).
#[inline]
pub(in crate::executor) fn field_tag_to_native_kind(tag: u16) -> Option<NativeKind> {
    match tag {
        FIELD_TAG_F64 => Some(NativeKind::Float64),
        FIELD_TAG_I64 => Some(NativeKind::Int64),
        FIELD_TAG_BOOL => Some(NativeKind::Bool),
        FIELD_TAG_STRING => Some(NativeKind::String),
        FIELD_TAG_ARRAY => Some(NativeKind::Ptr(HeapKind::TypedArray)),
        FIELD_TAG_OBJECT => Some(NativeKind::Ptr(HeapKind::TypedObject)),
        FIELD_TAG_DECIMAL => Some(NativeKind::Ptr(HeapKind::Decimal)),
        FIELD_TAG_TIMESTAMP => Some(NativeKind::Ptr(HeapKind::Temporal)),
        // Dynamic / unknown — ADR-006 §2.7.7: no statically-sourceable kind.
        _ => None,
    }
}

/// Push a TypedObject field onto the kinded VM stack.
///
/// ADR-006 §2.7.7 — kind is sourced from the operand-encoded `field_type_tag`
/// (per-field FieldType supplies the `NativeKind`); no dynamic-word
/// construction, no NaN-tag decoding, no transitional-shim push/pop. Heap-
/// backed slots have their `Arc<T>` strong-count bumped via
/// `clone_with_kind` because the slot is borrowed (the underlying
/// `TypedObjectStorage` retains the original share).
///
/// Cross-cluster note: the signature is preserved because cluster B
/// (`variables/mod.rs:3476`) and cluster D-prop-access
/// (`property_access.rs:463`) call this helper. Migrating those call sites
/// is their own cluster's responsibility per playbook §10 — this body
/// uses only kinded-API primitives.
#[inline(always)]
pub(in crate::executor) fn push_field_value(
    vm: &mut super::VirtualMachine,
    slot: &ValueSlot,
    is_heap: bool,
    field_type_tag: u16,
) -> Result<(), VMError> {
    if !is_heap {
        // Inline scalar field. Kind sourced from the operand tag per
        // playbook §10 D-typed-obj-ops row.
        return match field_type_tag {
            FIELD_TAG_I64 | FIELD_TAG_TIMESTAMP => {
                vm.push_kinded(slot.as_i64() as u64, NativeKind::Int64)
            }
            FIELD_TAG_F64 => vm.push_kinded(slot.as_f64().to_bits(), NativeKind::Float64),
            FIELD_TAG_BOOL => vm.push_kinded(slot.as_bool() as u64, NativeKind::Bool),
            // Non-heap slot tagged ANY / UNKNOWN / STRING / OBJECT / ARRAY /
            // DECIMAL: no statically-sourceable NativeKind in the §2.7.7
            // model. Surface per playbook §7 REVISED #4 — the right shape
            // is a NotImplemented(SURFACE) marker, never a Bool-default
            // fallback (W-series defection-attractor §2.7.7).
            _ => Err(VMError::NotImplemented(format!(
                "push_field_value SURFACE: non-heap slot with field_type_tag {} \
                 has no statically-sourceable NativeKind — \
                 ADR-006 §2.7.7 / playbook §10 D-typed-obj-ops",
                field_type_tag
            ))),
        };
    }

    // Heap-backed field: slot bits are an `Arc::into_raw`'d typed pointer
    // (per ADR-006 §2.4 / TypedObjectStorage construction-side contract).
    // Source the kind from the operand tag, then bump the underlying
    // refcount via `clone_with_kind` because we are borrowing the slot
    // — the enclosing `TypedObjectStorage` keeps the original share.
    let Some(kind) = field_tag_to_heap_native_kind(field_type_tag) else {
        return Err(VMError::NotImplemented(format!(
            "push_field_value SURFACE: heap-backed slot with dynamic \
             field_type_tag {} (FIELD_TAG_ANY / UNKNOWN) — \
             ADR-006 §2.7.7 / playbook §10 D-typed-obj-ops",
            field_type_tag
        )));
    };
    let bits = slot.raw();
    clone_with_kind(bits, kind);
    vm.push_kinded(bits, kind)
}

/// TypedObject operations for VirtualMachine
pub trait TypedObjectOps {
    /// Get field from typed object using precomputed offset (JIT optimization)
    fn op_get_field_typed(&mut self, instruction: &Instruction) -> Result<(), VMError>;

    /// Set field on typed object using precomputed offset (JIT optimization)
    fn op_set_field_typed(&mut self, instruction: &Instruction) -> Result<(), VMError>;
}

impl TypedObjectOps for super::VirtualMachine {
    /// Get field from typed object using precomputed field type tag.
    ///
    /// ADR-006 §2.7.7 / Wave 6.5 cluster `D-typed-obj-ops` — receiver pop
    /// uses the kinded API; heap dispatch is `slot.as_heap_value()` +
    /// `HeapValue::TypedObject(arc)` match (Q8 single-discriminator).
    /// The receiver share is dropped via `drop_with_kind` after the field
    /// load completes (the loaded value owns its own retained share via
    /// `clone_with_kind` inside `push_field_value`).
    #[inline(always)]
    fn op_get_field_typed(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let operand = instruction
            .operand
            .as_ref()
            .ok_or(VMError::InvalidOperand)?;

        let Operand::TypedField {
            type_id,
            field_idx,
            field_type_tag,
        } = operand
        else {
            return Err(VMError::InvalidOperand);
        };

        // Pop receiver via kinded API. The §2.7.7 invariant: bits + kind
        // come together — no transitional-shim pop, no the deleted tag_bits dispatch.
        let (recv_bits, recv_kind) = self.pop_kinded()?;

        // Validate receiver kind. Non-TypedObject receivers fall back to the
        // post-§2.7.7 "Bool sentinel" None push (drop the receiver share so
        // refcount stays balanced).
        if recv_kind != NativeKind::Ptr(HeapKind::TypedObject) {
            drop_with_kind(recv_bits, recv_kind);
            return self.push_kinded(0u64, NativeKind::Bool);
        }

        // Single-discriminator dispatch (ADR-005 §1, Q8): build a borrow-
        // only ValueSlot view of the bits, then `as_heap_value()` +
        // `HeapValue::TypedObject(arc)` match.
        let recv_slot = ValueSlot::from_raw(recv_bits);
        let recv_hv = recv_slot.as_heap_value();
        let HeapValue::TypedObject(storage) = recv_hv else {
            // Kind said TypedObject but heap arm disagrees — surface the
            // soundness gap rather than papering over with a Bool-default
            // sentinel (the W-series defection-attractor §2.7.7 names
            // verbatim). Drop the receiver share before erroring.
            drop_with_kind(recv_bits, recv_kind);
            return Err(VMError::NotImplemented(format!(
                "op_get_field_typed SURFACE: receiver kind says \
                 Ptr(TypedObject) but HeapValue arm is {:?} — \
                 ADR-005 §1 single-discriminator violation",
                recv_hv.kind(),
            )));
        };

        let schema_id = storage.schema_id;
        let field_count = storage.slots.len();

        // Schema mismatch: the operand's `type_id` doesn't match the
        // receiver's `schema_id`. Falls back to name-based field lookup
        // through the registry + property IC + megamorphic cache. Same
        // shape as the pre-Wave-6.5 path; only the push/pop primitives
        // change.
        if schema_id != *type_id as u64 {
            let ic_ip = self.ip;
            let sid = schema_id;

            // IC fast path: monomorphic per-schema cache hit.
            if let Some(hit) =
                crate::executor::ic_fast_paths::property_ic_check(self, ic_ip, sid)
            {
                let src_idx = hit.field_idx as usize;
                if src_idx < field_count {
                    let is_heap = (storage.heap_mask & (1u64 << src_idx)) != 0;
                    let result =
                        push_field_value(self, &storage.slots[src_idx], is_heap, hit.field_type_tag);
                    drop_with_kind(recv_bits, recv_kind);
                    return result;
                }
            }

            // Resolve target field name + source-side index from the
            // registry (immutable borrow scope). Extract before any
            // mutable borrows.
            let resolved = {
                let target_schema =
                    self.program.type_schema_registry.get_by_id(*type_id as u32);
                let source_schema = self
                    .program
                    .type_schema_registry
                    .get_by_id(schema_id as u32);
                match (target_schema, source_schema) {
                    (Some(target), Some(source)) => {
                        if let Some(target_field) = target.field_by_index(*field_idx) {
                            let field_name = target_field.name.clone();
                            if let Some(src_field_idx) = source.field_index(&field_name) {
                                let tag = source
                                    .field_by_index(src_field_idx)
                                    .map(|f| field_type_to_tag(&f.field_type))
                                    .unwrap_or(0);
                                Some((field_name, src_field_idx, tag))
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    }
                    _ => None,
                }
            };

            // Megamorphic cache fast path: when >4 schemas observed,
            // check the direct-mapped global cache before name-based
            // lookup.
            if let Some((ref fname, _, _)) = resolved {
                if let Some(hit) = crate::executor::ic_fast_paths::megamorphic_property_check(
                    self, ic_ip, sid, fname,
                ) {
                    let src_idx = hit.field_idx as usize;
                    if src_idx < field_count {
                        let is_heap = (storage.heap_mask & (1u64 << src_idx)) != 0;
                        let result = push_field_value(
                            self,
                            &storage.slots[src_idx],
                            is_heap,
                            hit.field_type_tag,
                        );
                        drop_with_kind(recv_bits, recv_kind);
                        return result;
                    }
                }
            }

            // Full name-based fallback: use pre-resolved field mapping.
            if let Some((field_name, src_field_idx, tag)) = resolved {
                let src_idx = src_field_idx as usize;
                if src_idx < field_count {
                    let is_heap = (storage.heap_mask & (1u64 << src_idx)) != 0;
                    // Record IC + megamorphic cache (mutable borrows safe
                    // here — we already cloned the strings we need).
                    if let Some(fv) = self.current_feedback_vector() {
                        fv.record_property(
                            ic_ip,
                            sid,
                            src_field_idx,
                            tag,
                            crate::feedback::RECEIVER_TYPED_OBJECT,
                        );
                    }
                    crate::executor::ic_fast_paths::megamorphic_property_insert(
                        self,
                        sid,
                        &field_name,
                        src_field_idx,
                        tag,
                    );
                    let result = push_field_value(self, &storage.slots[src_idx], is_heap, tag);
                    drop_with_kind(recv_bits, recv_kind);
                    return result;
                }
            }

            // Field not found on either side: push None sentinel, drop
            // receiver share.
            drop_with_kind(recv_bits, recv_kind);
            return self.push_kinded(0u64, NativeKind::Bool);
        }

        // Schema match: direct field index lookup using the operand's
        // pre-baked offset.
        let field_index = *field_idx as usize;
        debug_assert!(
            field_index < field_count,
            "GetFieldTyped field_idx {} out of bounds (field_count = {})",
            field_index,
            field_count
        );

        if field_index < field_count {
            let is_heap = (storage.heap_mask & (1u64 << field_index)) != 0;
            let result = push_field_value(
                self,
                &storage.slots[field_index],
                is_heap,
                *field_type_tag,
            );
            drop_with_kind(recv_bits, recv_kind);
            return result;
        }

        // Out-of-bounds: push None sentinel.
        drop_with_kind(recv_bits, recv_kind);
        self.push_kinded(0u64, NativeKind::Bool)
    }

    /// Set field on typed object using precomputed field type tag.
    ///
    /// W17-typed-object-mutation (2026-05-11) — write-path rebuild on
    /// top of `TypedObjectStorage::write_slot_in_place` (the kinded
    /// in-place projection writer added by W17-references-mutation close
    /// `30b9ebf`, ADR-006 §2.7.13 / Q14). Mirror of the
    /// `RefTarget::TypedField` arm in `write_ref_target`
    /// (`variables/mod.rs:3100`) — same single-threaded VM contract,
    /// same kind-invariance debug_assert, same heap_mask-driven
    /// drop_with_kind on the prior occupant.
    ///
    /// Stack contract (per `assignment.rs:611-625` emit pattern):
    /// pop value, pop receiver; mutate the receiver's slot in place;
    /// push the (now-mutated) receiver back so `emit_nested_store_back`
    /// can either store it back to a local/binding identifier or `Pop`
    /// it for non-identifier roots. Schema-mismatch falls back through
    /// the same name-based + IC lookup chain as `op_get_field_typed`.
    fn op_set_field_typed(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let operand = instruction
            .operand
            .as_ref()
            .ok_or(VMError::InvalidOperand)?;

        let Operand::TypedField {
            type_id,
            field_idx,
            field_type_tag,
        } = operand
        else {
            return Err(VMError::InvalidOperand);
        };

        // Pop value then receiver (LIFO; the compiler pushes receiver
        // first per `assignment.rs:566`).
        let (value_bits, value_kind) = self.pop_kinded()?;
        let (recv_bits, recv_kind) = self.pop_kinded()?;

        // Validate receiver kind. Non-TypedObject receivers: drain shares
        // and surface a TypeError. The Bool-sentinel fallback shape of
        // `op_get_field_typed` is read-side only — write-side type
        // confusion must error rather than silently no-op.
        if recv_kind != NativeKind::Ptr(HeapKind::TypedObject) {
            drop_with_kind(value_bits, value_kind);
            drop_with_kind(recv_bits, recv_kind);
            return Err(VMError::TypeError {
                expected: "TypedObject receiver",
                got: "non-TypedObject kind",
            });
        }

        if recv_bits == 0 {
            drop_with_kind(value_bits, value_kind);
            return Err(VMError::RuntimeError(
                "op_set_field_typed: null TypedObject receiver".to_string(),
            ));
        }

        // SAFETY: kind says `Ptr(HeapKind::TypedObject)`, so `recv_bits`
        // is `Arc::into_raw::<TypedObjectStorage>` and the popped slot
        // owns one strong-count share. Reconstruct, mutate in place,
        // re-into_raw to transfer the same share onto the result stack
        // slot (no refcount change).
        let storage_arc: std::sync::Arc<shape_value::heap_value::TypedObjectStorage> =
            unsafe { std::sync::Arc::from_raw(recv_bits as *const _) };

        let result = self.write_typed_object_field(
            &storage_arc,
            *type_id,
            *field_idx,
            *field_type_tag,
            value_bits,
            value_kind,
        );

        // Re-into_raw before result handling so the stack push transfers
        // the receiver share back regardless of which branch fired.
        let recv_bits_back = std::sync::Arc::into_raw(storage_arc) as u64;
        match result {
            Ok(()) => self.push_kinded(recv_bits_back, recv_kind),
            Err(e) => {
                // On error the value was already dropped inside
                // write_typed_object_field; release the receiver share.
                drop_with_kind(recv_bits_back, recv_kind);
                Err(e)
            }
        }
    }
}

impl super::VirtualMachine {
    /// Write `value_bits`/`value_kind` into `storage`'s field at the
    /// operand-specified location. Mirrors the schema-match / IC /
    /// name-fallback chain in `op_get_field_typed`. On success the
    /// `value_bits` share is transferred to the storage's slot and the
    /// prior occupant's share is released via `drop_with_kind`. On error
    /// the `value_bits` share is dropped before return.
    ///
    /// ADR-006 §2.7.13 / Q14 in-place write via
    /// `TypedObjectStorage::write_slot_in_place`.
    fn write_typed_object_field(
        &mut self,
        storage: &std::sync::Arc<shape_value::heap_value::TypedObjectStorage>,
        type_id: u16,
        field_idx: u16,
        field_type_tag: u16,
        value_bits: u64,
        value_kind: NativeKind,
    ) -> Result<(), VMError> {
        let schema_id = storage.schema_id;
        let field_count = storage.slots.len();

        // Schema-match path: direct field index from the operand's
        // pre-baked offset.
        if schema_id == type_id as u64 {
            let idx = field_idx as usize;
            if idx >= field_count {
                drop_with_kind(value_bits, value_kind);
                return Err(VMError::RuntimeError(format!(
                    "op_set_field_typed: field_idx {} out of bounds \
                     (slot count {})",
                    idx, field_count
                )));
            }
            return write_field_at_idx(storage, idx, field_type_tag, value_bits, value_kind);
        }

        // Schema-mismatch path: name-based lookup via IC + megamorphic
        // cache + registry. Mirror of `op_get_field_typed`'s structure.
        let ic_ip = self.ip;

        // IC fast path: monomorphic per-schema cache hit.
        if let Some(hit) =
            crate::executor::ic_fast_paths::property_ic_check(self, ic_ip, schema_id)
        {
            let src_idx = hit.field_idx as usize;
            if src_idx < field_count {
                return write_field_at_idx(
                    storage,
                    src_idx,
                    hit.field_type_tag,
                    value_bits,
                    value_kind,
                );
            }
        }

        // Resolve target field name + source-side index from the registry.
        let resolved = {
            let target_schema = self.program.type_schema_registry.get_by_id(type_id as u32);
            let source_schema = self.program.type_schema_registry.get_by_id(schema_id as u32);
            match (target_schema, source_schema) {
                (Some(target), Some(source)) => {
                    if let Some(target_field) = target.field_by_index(field_idx) {
                        let field_name = target_field.name.clone();
                        if let Some(src_field_idx) = source.field_index(&field_name) {
                            let tag = source
                                .field_by_index(src_field_idx)
                                .map(|f| field_type_to_tag(&f.field_type))
                                .unwrap_or(0);
                            Some((field_name, src_field_idx, tag))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }
                _ => None,
            }
        };

        // Megamorphic cache fast path.
        if let Some((ref fname, _, _)) = resolved {
            if let Some(hit) = crate::executor::ic_fast_paths::megamorphic_property_check(
                self, ic_ip, schema_id, fname,
            ) {
                let src_idx = hit.field_idx as usize;
                if src_idx < field_count {
                    return write_field_at_idx(
                        storage,
                        src_idx,
                        hit.field_type_tag,
                        value_bits,
                        value_kind,
                    );
                }
            }
        }

        // Full name-based fallback.
        if let Some((field_name, src_field_idx, tag)) = resolved {
            let src_idx = src_field_idx as usize;
            if src_idx < field_count {
                if let Some(fv) = self.current_feedback_vector() {
                    fv.record_property(
                        ic_ip,
                        schema_id,
                        src_field_idx,
                        tag,
                        crate::feedback::RECEIVER_TYPED_OBJECT,
                    );
                }
                crate::executor::ic_fast_paths::megamorphic_property_insert(
                    self,
                    schema_id,
                    &field_name,
                    src_field_idx,
                    tag,
                );
                return write_field_at_idx(storage, src_idx, tag, value_bits, value_kind);
            }
        }

        // Field not found on either side: drop the value share and
        // surface UndefinedProperty rather than silently no-op'ing.
        drop_with_kind(value_bits, value_kind);
        Err(VMError::RuntimeError(format!(
            "op_set_field_typed: field index {} on schema {} not found \
             on receiver schema {}",
            field_idx, type_id, schema_id,
        )))
    }
}

/// Common in-place writer: validate the field's kind matches the popped
/// value's kind (post-proof §2.7.5.1 contract), write through
/// `write_slot_in_place`, drop the prior occupant's share.
fn write_field_at_idx(
    storage: &shape_value::heap_value::TypedObjectStorage,
    idx: usize,
    field_type_tag: u16,
    value_bits: u64,
    value_kind: NativeKind,
) -> Result<(), VMError> {
    debug_assert!(idx < storage.slots.len());
    debug_assert!(idx < storage.field_kinds.len());

    let stored_kind = storage.field_kinds[idx];

    // Kind invariance check (release form). The post-proof contract
    // forbids mid-life kind changes for typed fields; if a divergent
    // kind reaches here it's a compiler-emit bug worth surfacing.
    // FIELD_TAG_ANY / UNKNOWN are the only operand tags that may
    // legitimately carry a kind not statically resolvable in the
    // operand — for those we accept the stored kind as canonical.
    if value_kind != stored_kind
        && field_type_tag != FIELD_TAG_ANY
        && field_type_tag != FIELD_TAG_UNKNOWN
    {
        // Tag-based equivalence: width-integer fields all store as
        // Int64; FIELD_TAG_TIMESTAMP also routes through Int64. Accept
        // those equivalences without surfacing.
        let kind_compatible_with_tag = match field_type_tag {
            FIELD_TAG_I64 | FIELD_TAG_TIMESTAMP => matches!(
                value_kind,
                NativeKind::Int64
                    | NativeKind::Int8
                    | NativeKind::Int16
                    | NativeKind::Int32
                    | NativeKind::UInt8
                    | NativeKind::UInt16
                    | NativeKind::UInt32
                    | NativeKind::UInt64
            ),
            FIELD_TAG_F64 => value_kind == NativeKind::Float64,
            FIELD_TAG_BOOL => value_kind == NativeKind::Bool,
            FIELD_TAG_STRING => matches!(
                value_kind,
                NativeKind::String | NativeKind::Ptr(HeapKind::String)
            ),
            _ => value_kind == stored_kind,
        };
        if !kind_compatible_with_tag {
            drop_with_kind(value_bits, value_kind);
            return Err(VMError::TypeError {
                expected: "value kind matching field schema",
                got: "mismatched kind",
            });
        }
    }

    // Pre-read prior bits for the write barrier; the in-place writer
    // returns the same value so we record it before the call.
    let prior_bits = storage.slots[idx].raw();
    crate::memory::write_barrier_slot(prior_bits, value_bits);

    // SAFETY: per `TypedObjectStorage::write_slot_in_place` contract —
    // single-threaded VM, no aliased `&mut ValueSlot` outstanding (this
    // function holds only `&storage`; the in-place writer reaches the
    // slot through `*const ValueSlot` cast), kind invariance verified
    // above against the storage's `field_kinds` track. `value_bits`
    // ownership (one strong-count share for heap kinds) transfers to
    // the slot; the returned `_returned_prior` is the same bits we
    // pre-read.
    let _returned_prior = unsafe { storage.write_slot_in_place(idx, value_bits) };
    debug_assert_eq!(
        _returned_prior, prior_bits,
        "op_set_field_typed: write_slot_in_place prior_bits mismatch — \
         concurrent write detected? ADR-006 §2.7.13 / Q14",
    );

    // Release the prior occupant's share via the kind-aware dispatch
    // table (§2.7.7 WB2.4). For inline scalar fields this is a no-op.
    drop_with_kind(prior_bits, stored_kind);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytecode::{Instruction, OpCode};
    use crate::executor::{VMConfig, VirtualMachine};
    use shape_runtime::type_schema::{FieldType, TypeSchema};
    use shape_value::heap_value::TypedObjectStorage;
    use std::sync::Arc;

    /// `op_set_field_typed` on a schema-match path rotates the field's
    /// slot through `write_slot_in_place` and pushes the (mutated)
    /// receiver back. W17-typed-object-mutation fill (2026-05-11).
    #[test]
    fn set_field_typed_schema_match_writes_int_field() {
        let mut vm = VirtualMachine::new(VMConfig::default());

        let schema = TypeSchema::new(
            "Probe".to_string(),
            vec![
                ("x".to_string(), FieldType::I64),
                ("y".to_string(), FieldType::I64),
            ],
        );
        let schema_id = schema.id;
        vm.program.type_schema_registry.register(schema);

        let slots = vec![ValueSlot::from_raw(1u64), ValueSlot::from_raw(2u64)];
        let storage = TypedObjectStorage::new(
            schema_id as u64,
            slots.into_boxed_slice(),
            0,
            Arc::from(vec![NativeKind::Int64, NativeKind::Int64].into_boxed_slice()),
        );
        let storage_arc = Arc::new(storage);
        let recv_bits = Arc::into_raw(storage_arc) as u64;

        // Stack: [recv, value]; operand: TypedField { type_id = schema_id, field_idx = 1, tag = I64 }
        vm.push_kinded(recv_bits, NativeKind::Ptr(HeapKind::TypedObject))
            .unwrap();
        vm.push_kinded(99u64, NativeKind::Int64).unwrap();

        let operand = Operand::TypedField {
            type_id: schema_id as u16,
            field_idx: 1,
            field_type_tag: FIELD_TAG_I64,
        };
        let instr = Instruction::new(OpCode::SetFieldTyped, Some(operand));
        vm.op_set_field_typed(&instr).unwrap();

        // op_set_field_typed pushes the (mutated) receiver back.
        let (obj_bits_back, obj_kind_back) = vm.pop_kinded().unwrap();
        assert_eq!(obj_kind_back, NativeKind::Ptr(HeapKind::TypedObject));
        // Recover and verify field y now reads 99.
        let storage_back: Arc<TypedObjectStorage> =
            unsafe { Arc::from_raw(obj_bits_back as *const _) };
        assert_eq!(storage_back.slots[0].raw(), 1u64);
        assert_eq!(storage_back.slots[1].raw(), 99u64);
        drop(storage_back);
    }

    /// `op_set_field_typed` on a non-TypedObject receiver returns a
    /// TypeError after draining shares. W17-typed-object-mutation
    /// (2026-05-11).
    #[test]
    fn set_field_typed_non_typed_object_receiver_errors() {
        let mut vm = VirtualMachine::new(VMConfig::default());

        // Push an Int64 "receiver" — wrong kind for SetFieldTyped.
        vm.push_kinded(0u64, NativeKind::Int64).unwrap();
        vm.push_kinded(1u64, NativeKind::Int64).unwrap();

        let operand = Operand::TypedField {
            type_id: 0,
            field_idx: 0,
            field_type_tag: FIELD_TAG_I64,
        };
        let instr = Instruction::new(OpCode::SetFieldTyped, Some(operand));
        let err = vm.op_set_field_typed(&instr).unwrap_err();
        assert!(matches!(err, VMError::TypeError { .. }));
    }
}
