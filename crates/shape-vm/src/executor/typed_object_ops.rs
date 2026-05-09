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
//! have all been migrated off this file. Cross-cluster cascades (write-path
//! `clone_slots_with_update` / `op_set_field_typed`) are surfaced as
//! `VMError::NotImplemented` with a SURFACE marker per playbook §7 #4.

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
    /// SURFACE per playbook §7 REVISED #4 / §8 — write-path requires
    /// rewriting `clone_slots_with_update` (`object_creation.rs`,
    /// territory `D-obj-create`) to take kinded `(bits, kind)` instead
    /// of the deleted dynamic-word reference, plus a kinded
    /// `HeapValue::TypedObject` rebuild. Both are cross-cluster cascade
    /// per playbook §8 surface-and-stop. Until those land, the write-
    /// path is `NotImplemented(SURFACE)` rather than a Bool-default
    /// forbidden-pattern workaround (CLAUDE.md "Forbidden Patterns").
    fn op_set_field_typed(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let operand = instruction
            .operand
            .as_ref()
            .ok_or(VMError::InvalidOperand)?;

        let Operand::TypedField {
            type_id: _,
            field_idx: _,
            field_type_tag: _,
        } = operand
        else {
            return Err(VMError::InvalidOperand);
        };

        // Drain the value + receiver shares before surfacing so the kind
        // track stays balanced (refcount discipline ADR-006 §2.7.7 WB2.4).
        let (value_bits, value_kind) = self.pop_kinded()?;
        let (recv_bits, recv_kind) = self.pop_kinded()?;
        drop_with_kind(value_bits, value_kind);
        drop_with_kind(recv_bits, recv_kind);

        Err(VMError::NotImplemented(
            "op_set_field_typed SURFACE: write-path requires kinded \
             clone_slots_with_update + HeapValue::TypedObject(Arc<TypedObjectStorage>) \
             rebuild — cross-cluster cascade with D-obj-create \
             (playbook §8 surface-and-stop / §10 D-typed-obj-ops). \
             ADR-006 §2.7.7 / Q10 — no Bool-default fallback per W-series \
             defection-attractor."
                .into(),
        ))
    }
}
