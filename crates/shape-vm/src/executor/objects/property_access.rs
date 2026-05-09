//! Property access operations (GetProp, SetProp, Length).
//!
//! Wave 6.5 cluster D sub-cluster D-prop-access (ADR-006 §2.7.6, §2.7.7,
//! §2.7.8). Heap dispatch uses `slot.as_heap_value()` + `HeapValue::*`
//! match per Q8 — no per-heap-variant accessors on `KindedSlot`. Result
//! kinds are sourced from the schema's per-slot `field_kinds` track
//! (ADR-006 §2.5) for TypedObject reads, never defaulted to Bool.
//!
//! See `docs/cluster-audits/phase-1b-vm-wave-6-5-playbook.md` §10
//! D-prop-access row.

use crate::bytecode::{Instruction, Operand};
use crate::executor::VirtualMachine;
use crate::executor::vm_impl::stack::{clone_with_kind, drop_with_kind};
use shape_value::{NativeKind, VMError, heap_value::HeapKind};
use std::sync::Arc;

impl VirtualMachine {
    /// `GetProp`: read a property from a heap object.
    ///
    /// **Migrated path (Wave 6.5 cluster D-prop-access):** TypedObject
    /// property dispatch via the TypedObjectStorage's per-slot
    /// `field_kinds` track (ADR-006 §2.5). The slot's `NativeKind`
    /// determines the push kind; heap-bearing slots are shared via
    /// `clone_with_kind` (WB2.4 retain-on-read).
    ///
    /// **Surfaced (`NotImplemented(SURFACE)`):** every other receiver
    /// shape — typed arrays, strings, hashmaps, table views, native
    /// views, matrices, etc. The legacy code paths used the deleted
    /// dynamic-word carrier, the deleted `raw_helpers` tag_bits
    /// dispatch, and the deleted `is_tagged()` index call — all
    /// forbidden by §2.7.7. Those paths need first-class kinded
    /// handlers that consume `(bits, kind)` pairs and never round-trip
    /// through a tagged encoding; that work belongs to the matching
    /// cluster (typed-array element ops, hashmap, table-view) per
    /// playbook §10.
    pub(in crate::executor) fn op_get_prop(
        &mut self,
        _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
    ) -> Result<(), VMError> {
        let (key_bits, key_kind) = self.pop_kinded()?;
        let (obj_bits, obj_kind) = self.pop_kinded()?;

        // Borrow the key as &str when its kind is `NativeKind::String`. The
        // key carries one strong-count share per WB2.4; we drop it after
        // the dispatch completes.
        let key_str: Option<&str> = match key_kind {
            NativeKind::String
            | NativeKind::Ptr(HeapKind::String) => {
                if key_bits == 0 {
                    None
                } else {
                    // SAFETY: `NativeKind::String` means `key_bits` is
                    // `Arc::into_raw::<String>` and the slot owns one
                    // strong-count share. The borrow is valid for the
                    // remainder of this scope (we only release the share
                    // via `drop_with_kind` at the end).
                    let s: &String = unsafe { &*(key_bits as *const String) };
                    Some(s.as_str())
                }
            }
            _ => None,
        };

        let result = self.dispatch_get_prop(obj_bits, obj_kind, key_bits, key_kind, key_str);

        // Retire the popped key + object shares per WB2.4 drop discipline.
        // Heap-bearing arms are no-ops on zero bits; inline scalars are
        // always no-ops.
        drop_with_kind(key_bits, key_kind);
        drop_with_kind(obj_bits, obj_kind);

        result
    }

    /// Inner dispatch for `op_get_prop`. Borrows are valid per the
    /// outer pop's WB2.4 retain — the shares are released at the end
    /// of `op_get_prop` regardless of which arm fires.
    #[inline]
    fn dispatch_get_prop(
        &mut self,
        obj_bits: u64,
        obj_kind: NativeKind,
        _key_bits: u64,
        key_kind: NativeKind,
        key_str: Option<&str>,
    ) -> Result<(), VMError> {
        match obj_kind {
            // ── TypedObject: schema-driven field read ────────────────────
            //
            // The slot bits are `Arc::into_raw::<TypedObjectStorage>`.
            // `as_heap_value()` is the Q8 heap-dispatch entry point; we
            // pattern-match `HeapValue::TypedObject(arc)` and read the
            // field via the schema registry, sourcing the push kind from
            // `field_kinds[index]` (ADR-006 §2.5).
            NativeKind::Ptr(HeapKind::TypedObject) => {
                let ks = key_str.ok_or_else(|| VMError::TypeError {
                    expected: "string property name",
                    got: "non-string key",
                })?;
                if obj_bits == 0 {
                    return Err(VMError::RuntimeError(
                        "GetProp on null TypedObject".to_string(),
                    ));
                }
                // SAFETY: kind says `Ptr(HeapKind::TypedObject)`, so
                // `obj_bits` is `Arc::into_raw::<TypedObjectStorage>` and
                // the popped slot owns one strong-count share. Borrow via
                // a transient `Arc` (does NOT add a refcount because we
                // pair `Arc::from_raw` with `Arc::into_raw` immediately).
                //
                // The schema's per-slot `NativeKind` table is the canonical
                // kind source per ADR-006 §2.5.
                let storage_arc: Arc<shape_value::heap_value::TypedObjectStorage> =
                    unsafe { Arc::from_raw(obj_bits as *const _) };
                let result = self.read_typed_object_field(&storage_arc, ks);
                // Release our transient borrow without dropping a share —
                // the original popped slot still owns it (it's released
                // by the caller's `drop_with_kind`).
                let _ = Arc::into_raw(storage_arc);
                result
            }

            // ── String, TypedArray, HashMap, NativeView, Temporal,
            //    TableView, DataTable, Decimal, BigInt, etc. ─────────────
            //
            // SURFACE: these need first-class kinded handlers that consume
            // `(bits, kind)` and dispatch on the matching HeapKind without
            // routing through the deleted dynamic-word carrier or the
            // deleted raw-helpers shim layer. The previous code shapes
            // used `extract_heap_ref`, `unwrap_annotated_bits`,
            // `ValueBits::is_unified_heap`, and per-FieldKind tag
            // unpackers — all forbidden by ADR-006 §2.7.7. Migrating
            // each receiver shape is per-cluster work (typed-array
            // element access, hashmap, table-view) per playbook §10.
            NativeKind::String
            | NativeKind::Ptr(_) => Err(VMError::NotImplemented(format!(
                "SURFACE: GetProp on {:?} not yet kinded — see playbook §10 \
                 D-prop-access; legacy path used the deleted dynamic-word \
                 carrier and the deleted `raw_helpers` hops (key kind {:?})",
                obj_kind, key_kind
            ))),

            // ── Inline scalars: no property access semantics ────────────
            _ => Err(VMError::TypeError {
                expected: "object, array, string, or other heap value",
                got: "scalar",
            }),
        }
    }

    /// Read a named field from a `TypedObjectStorage`, sourcing the push
    /// kind from `field_kinds[index]` per ADR-006 §2.5. Heap-bearing
    /// slots are shared via `clone_with_kind` (WB2.4 retain-on-read) so
    /// the caller's slot owns an independent strong-count share.
    fn read_typed_object_field(
        &mut self,
        storage: &shape_value::heap_value::TypedObjectStorage,
        key: &str,
    ) -> Result<(), VMError> {
        let schema = self
            .program
            .type_schema_registry
            .get_by_id(storage.schema_id as u32)
            .ok_or_else(|| {
                VMError::RuntimeError(format!(
                    "Schema {} not found in registry",
                    storage.schema_id
                ))
            })?;
        let field = schema
            .get_field(key)
            .ok_or_else(|| VMError::UndefinedProperty(key.to_string()))?;
        let idx = field.index as usize;
        if idx >= storage.slots.len() {
            return Err(VMError::RuntimeError(format!(
                "Field '{}' index {} exceeds slot count {}",
                key,
                idx,
                storage.slots.len()
            )));
        }
        if idx >= storage.field_kinds.len() {
            return Err(VMError::RuntimeError(format!(
                "Field '{}' index {} exceeds field_kinds length {}",
                key,
                idx,
                storage.field_kinds.len()
            )));
        }

        let bits = storage.slots[idx].raw();
        let kind = storage.field_kinds[idx];

        // WB2.4 retain-on-read: the storage owns the strong-count share;
        // the pushed slot must own an independent share. `clone_with_kind`
        // is a no-op on inline scalars and zero bits.
        clone_with_kind(bits, kind);
        self.push_kinded(bits, kind)
    }

    /// `SetProp`: write a property on a heap object. Pops value, key,
    /// object; mutates object; pushes object back.
    ///
    /// **SURFACE.** The legacy implementation took a `&mut` borrow on
    /// the dynamic-word carrier and used `Arc::make_mut`, plus the
    /// deleted heap-mut accessor, the deleted unified-array
    /// from-heap-bits-mut hop, and the deleted refcount-release
    /// helper, for in-place mutation through tag-encoded slots —
    /// every one of which is forbidden by ADR-006 §2.7.7 / §2.7.8.
    ///
    /// Migrating SetProp requires structural choices that are out of
    /// scope for D-prop-access:
    ///
    /// 1. The cell-storage parallel-kind invariant (§2.7.8 / Q10) for
    ///    binding writes — `Wave-α B7-B9` territory.
    /// 2. Mutable-receiver handling for typed arrays / hashmaps —
    ///    `D-array-ops` and `D-typed-access` territory.
    /// 3. NativeView field write — kept inline below (the helpers are
    ///    in this file) but driven by a TypedObject SURFACE today.
    ///
    /// Per playbook §7 REVISED #3 / §10 D-prop-access, surface-and-stop
    /// is the correct response when the call site cannot be migrated
    /// without reaching into another cluster's territory.
    pub(in crate::executor) fn op_set_prop(&mut self) -> Result<(), VMError> {
        // Drain the three operands so the stack stays balanced; release
        // their shares via `drop_with_kind` to avoid leaks while the
        // surface placeholder is in place.
        let (val_bits, val_kind) = self.pop_kinded()?;
        let (key_bits, key_kind) = self.pop_kinded()?;
        let (obj_bits, obj_kind) = self.pop_kinded()?;
        drop_with_kind(val_bits, val_kind);
        drop_with_kind(key_bits, key_kind);
        drop_with_kind(obj_bits, obj_kind);
        Err(VMError::NotImplemented(
            "SURFACE: SetProp not yet kinded (see playbook §10 D-prop-access; \
             legacy path used the deleted dynamic-word receiver-mutation \
             surface — forbidden by ADR-006 §2.7.7/§2.7.8)"
                .to_string(),
        ))
    }

    /// `SetLocalIndex`: in-place index assignment on a local. SURFACE
    /// for the same reason as `op_set_prop` (legacy receiver mutation
    /// path used the deleted local-take shim + `Arc::make_mut` + the
    /// dynamic-word carrier). Local-frame interaction with the §2.7.8
    /// cell-storage parallel-kind invariant is `B6-variables-loadptr`
    /// / `B7-closure-cells` territory.
    pub(in crate::executor) fn op_set_local_index(
        &mut self,
        _instruction: &Instruction,
    ) -> Result<(), VMError> {
        let (val_bits, val_kind) = self.pop_kinded()?;
        let (key_bits, key_kind) = self.pop_kinded()?;
        drop_with_kind(val_bits, val_kind);
        drop_with_kind(key_bits, key_kind);
        Err(VMError::NotImplemented(
            "SURFACE: SetLocalIndex not yet kinded (see playbook §10 \
             D-prop-access + B6-variables-loadptr; legacy path used the \
             deleted local-take shim + the dynamic-word carrier)"
                .to_string(),
        ))
    }

    /// `SetModuleBindingIndex`: in-place index assignment on a module
    /// binding. SURFACE — depends on the §2.7.8 cell-storage
    /// parallel-kind invariant for module bindings (`B8-shared-cell` /
    /// `B9-callframe-kind` territory).
    pub(in crate::executor) fn op_set_module_binding_index(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let (val_bits, val_kind) = self.pop_kinded()?;
        let (key_bits, key_kind) = self.pop_kinded()?;
        drop_with_kind(val_bits, val_kind);
        drop_with_kind(key_bits, key_kind);
        // Decode the operand only to validate the bytecode shape; the
        // actual binding mutation is surfaced.
        let _binding_idx = match instruction.operand {
            Some(Operand::ModuleBinding(idx)) => idx as usize,
            _ => return Err(VMError::InvalidOperand),
        };
        Err(VMError::NotImplemented(
            "SURFACE: SetModuleBindingIndex not yet kinded (see playbook \
             §10 B8-shared-cell + B9-callframe-kind; legacy path used the \
             deleted module-binding-take shim + the dynamic-word carrier)"
                .to_string(),
        ))
    }

    /// `Length`: read the length of an array, string, hashmap, etc.
    ///
    /// **Migrated path (Wave 6.5 cluster D-prop-access):** TypedObject
    /// length is the slot count.
    ///
    /// **SURFACE:** every other receiver — typed arrays, strings,
    /// hashmaps, native views, matrices — needs first-class kinded
    /// handlers per the same per-cluster split as `op_get_prop`.
    pub(in crate::executor) fn op_length(&mut self) -> Result<(), VMError> {
        let (bits, kind) = self.pop_kinded()?;
        let result = match kind {
            NativeKind::Ptr(HeapKind::TypedObject) => {
                if bits == 0 {
                    Err(VMError::RuntimeError(
                        "length() on null TypedObject".to_string(),
                    ))
                } else {
                    // SAFETY: kind says `Ptr(HeapKind::TypedObject)`; bits
                    // are `Arc::into_raw::<TypedObjectStorage>` per the
                    // construction-side contract. Borrow transiently.
                    let storage: Arc<shape_value::heap_value::TypedObjectStorage> =
                        unsafe { Arc::from_raw(bits as *const _) };
                    let len = storage.slots.len() as i64;
                    let _ = Arc::into_raw(storage);
                    self.push_kinded(len as u64, NativeKind::Int64)
                }
            }
            // SURFACE for every non-TypedObject heap kind: the legacy
            // path used `extract_heap_ref` + `HeapValue::*` exhaustive
            // match, which assumed `Box<HeapValue>` slot layout
            // incompatible with the typed-Arc shape (ADR-006 §2.4).
            NativeKind::String | NativeKind::Ptr(_) => {
                Err(VMError::NotImplemented(format!(
                    "SURFACE: length() on {:?} not yet kinded — see playbook \
                     §10 D-prop-access + per-cluster receiver migrations",
                    kind
                )))
            }
            _ => Err(VMError::TypeError {
                expected: "array, object, or string",
                got: "scalar",
            }),
        };
        // Retire the popped object's share regardless of which arm fired.
        drop_with_kind(bits, kind);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::VMConfig;
    use shape_value::ValueSlot;
    use shape_value::heap_value::TypedObjectStorage;

    /// A standalone `op_length` call on a TypedObject built with empty
    /// slots returns 0 + `NativeKind::Int64`.
    #[test]
    fn length_typed_object_empty() {
        let mut vm = VirtualMachine::new(VMConfig::default());
        let storage = TypedObjectStorage::new(
            0,
            Vec::<ValueSlot>::new().into_boxed_slice(),
            0,
            Arc::from(Vec::<NativeKind>::new().into_boxed_slice()),
        );
        let arc = Arc::new(storage);
        let bits = Arc::into_raw(arc) as u64;
        vm.push_kinded(bits, NativeKind::Ptr(HeapKind::TypedObject))
            .unwrap();
        vm.op_length().unwrap();
        let (len_bits, len_kind) = vm.pop_kinded().unwrap();
        assert_eq!(len_bits, 0);
        assert_eq!(len_kind, NativeKind::Int64);
    }

    /// `op_length` surfaces for non-TypedObject heap kinds (legacy paths
    /// removed; per-cluster migration owns each receiver).
    #[test]
    fn length_string_surfaces() {
        let mut vm = VirtualMachine::new(VMConfig::default());
        let s: Arc<String> = Arc::new("hello".to_string());
        let bits = Arc::into_raw(s) as u64;
        vm.push_kinded(bits, NativeKind::String).unwrap();
        let err = vm.op_length().unwrap_err();
        match err {
            VMError::NotImplemented(msg) => assert!(msg.contains("SURFACE")),
            other => panic!("expected NotImplemented(SURFACE), got {:?}", other),
        }
    }
}
