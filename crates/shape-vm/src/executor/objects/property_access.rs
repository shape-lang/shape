//! Property access operations (GetProp, SetProp, Length).
//!
//! Wave 6.5 cluster D sub-cluster D-prop-access (ADR-006 §2.7.6, §2.7.7,
//! §2.7.8). Heap dispatch uses per-`HeapKind` `Arc::from_raw` recovery
//! per ADR-006 §2.4 (typed-Arc slots — `as_heap_value()` is the legacy
//! `Box<HeapValue>` shape and is unsound on `Arc::into_raw`-stored slot
//! bits for everything except `TypedObject` and the Box-wrapped legacy
//! arms). Result kinds are sourced from the schema's per-slot
//! `field_kinds` track (ADR-006 §2.5) for TypedObject reads, never
//! defaulted to Bool.
//!
//! ## Wave 9 W9-property-access body re-fill (2026-05-10)
//!
//! Read-only receiver paths previously left as `NotImplemented(SURFACE)`
//! are filled per the wave-9 playbook §1 recipe + §2 row:
//!
//! - **`op_length`**: `Ptr(HeapKind::TypedArray)` walks `TypedArrayData`
//!   variants for element count; `NativeKind::String` /
//!   `Ptr(HeapKind::String)` returns `chars().count()` per the
//!   `v2_string_len` precedent in `string_methods.rs`;
//!   `Ptr(HeapKind::HashMap)` returns `HashMapData::len()`.
//! - **`op_get_prop`** with numeric key on `Ptr(HeapKind::TypedArray)`:
//!   reads the indexed element, retains heap-bearing payloads via
//!   `clone_with_kind` per WB2.4, pushes `(bits, element_kind)` where
//!   the element kind is sourced from the variant arm directly.
//!
//! Mutation paths (`op_set_prop`, `op_set_local_index`,
//! `op_set_module_binding_index`) and the HeapValue-projection
//! receivers (HashMap key dispatch into `Arc<HeapValue>` payload, String
//! per-codepoint indexing) remain `NotImplemented(SURFACE)` per the
//! W8-T26 close note + ADR-006 §2.7.4 Phase-2c deferral discipline.
//!
//! See `docs/cluster-audits/phase-1b-vm-wave-6-5-playbook.md` §10
//! D-prop-access row + `docs/cluster-audits/wave-9-method-refill-playbook.md`.

use crate::bytecode::{Instruction, Operand};
use crate::executor::VirtualMachine;
use crate::executor::vm_impl::stack::{clone_with_kind, drop_with_kind};
use shape_value::{
    NativeKind, VMError,
    heap_value::{HashMapData, HeapKind, TypedArrayData},
};
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
        key_bits: u64,
        key_kind: NativeKind,
        key_str: Option<&str>,
    ) -> Result<(), VMError> {
        match obj_kind {
            // ── TypedObject: schema-driven field read ────────────────────
            //
            // The slot bits are `Arc::into_raw::<TypedObjectStorage>`.
            // Recover via `Arc::from_raw` per the typed-Arc slot contract
            // (ADR-006 §2.4); pattern-match the schema and read the
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

            // ── TypedArray: numeric index access (W9 fill) ───────────────
            //
            // `arr[i]` lowers to `PushConst Int|Number i` + `GetProp`
            // (see `compiler/expressions/data_access.rs`,
            // `compiler/loops.rs` array-destructuring). Recover the
            // typed-Arc array, dispatch on `TypedArrayData` variant for
            // raw-bits read + element kind. Heap-bearing arms
            // (`String` / `HeapValue` / `FloatSlice` / `Matrix`) are
            // partially supported: scalar variants + `String` flow
            // through the kinded retain path; `HeapValue`-payload arrays
            // surface (HeapValue→KindedSlot projection is Phase-2c — see
            // ADR-006 §2.7.4).
            NativeKind::Ptr(HeapKind::TypedArray) => {
                if obj_bits == 0 {
                    return Err(VMError::RuntimeError(
                        "GetProp on null TypedArray".to_string(),
                    ));
                }
                let index = numeric_index_from_kinded(key_bits, key_kind)?;
                // SAFETY: kind says `Ptr(HeapKind::TypedArray)`; bits are
                // `Arc::into_raw::<TypedArrayData>`; popped slot owns one
                // strong-count share. Transient borrow — re-into_raw
                // before return so caller's `drop_with_kind` still
                // releases the original share.
                let arr_arc: Arc<TypedArrayData> =
                    unsafe { Arc::from_raw(obj_bits as *const _) };
                let result = self.read_typed_array_index(&arr_arc, index);
                let _ = Arc::into_raw(arr_arc);
                result
            }

            // ── HashMap, String index, NativeView, Temporal, TableView,
            //    DataTable, Decimal, BigInt, etc. ─────────────────────────
            //
            // SURFACE: HashMap value reads project through `Arc<HeapValue>`
            // payloads (`HashMapData::values: Arc<TypedBuffer<Arc<HeapValue>>>`)
            // and require the same `Arc<HeapValue>`→`KindedSlot` projection
            // as `op_new_array`'s heterogeneous path. String per-codepoint
            // indexing, table-view row dispatch, etc. each need their own
            // §2.7.6/Q8 heterogeneous-kind body. Phase-2c reentry per
            // ADR-006 §2.7.4.
            NativeKind::String
            | NativeKind::Ptr(_) => Err(VMError::NotImplemented(format!(
                "SURFACE: GetProp on {:?} not yet kinded — Phase-2c reentry \
                 (ADR-006 §2.7.4); requires Arc<HeapValue>→KindedSlot \
                 projection (HashMap values / heterogeneous arrays) or a \
                 per-receiver heterogeneous-kind body (key kind {:?})",
                obj_kind, key_kind
            ))),

            // ── Inline scalars: no property access semantics ────────────
            _ => Err(VMError::TypeError {
                expected: "object, array, string, or other heap value",
                got: "scalar",
            }),
        }
    }

    /// Read the `index`-th element of a `TypedArrayData` and push it onto
    /// the kinded stack with the matching element kind.
    ///
    /// Scalar variants push raw native bits + their `NativeKind`. The
    /// `String` variant retains an independent `Arc<String>` strong-count
    /// share for the pushed slot (the buffer keeps its share). Variants
    /// without a single statically-sourceable scalar element kind
    /// (`HeapValue` / `FloatSlice` / `Matrix`) surface — the
    /// `Arc<HeapValue>` projection is Phase-2c reentry territory per
    /// ADR-006 §2.7.4.
    fn read_typed_array_index(
        &mut self,
        arr: &TypedArrayData,
        index: usize,
    ) -> Result<(), VMError> {
        let oob = |len: usize| VMError::IndexOutOfBounds {
            index: index as i32,
            length: len,
        };
        match arr {
            TypedArrayData::I64(buf) => {
                let v = *buf.data.get(index).ok_or_else(|| oob(buf.data.len()))?;
                self.push_kinded(v as u64, NativeKind::Int64)
            }
            TypedArrayData::F64(buf) => {
                let v = *buf.data.get(index).ok_or_else(|| oob(buf.data.len()))?;
                self.push_kinded(v.to_bits(), NativeKind::Float64)
            }
            TypedArrayData::Bool(buf) => {
                let v = *buf.data.get(index).ok_or_else(|| oob(buf.data.len()))?;
                self.push_kinded(v as u64, NativeKind::Bool)
            }
            TypedArrayData::I8(buf) => {
                let v = *buf.data.get(index).ok_or_else(|| oob(buf.data.len()))?;
                self.push_kinded(v as i64 as u64, NativeKind::Int8)
            }
            TypedArrayData::I16(buf) => {
                let v = *buf.data.get(index).ok_or_else(|| oob(buf.data.len()))?;
                self.push_kinded(v as i64 as u64, NativeKind::Int16)
            }
            TypedArrayData::I32(buf) => {
                let v = *buf.data.get(index).ok_or_else(|| oob(buf.data.len()))?;
                self.push_kinded(v as i64 as u64, NativeKind::Int32)
            }
            TypedArrayData::U8(buf) => {
                let v = *buf.data.get(index).ok_or_else(|| oob(buf.data.len()))?;
                self.push_kinded(v as u64, NativeKind::UInt8)
            }
            TypedArrayData::U16(buf) => {
                let v = *buf.data.get(index).ok_or_else(|| oob(buf.data.len()))?;
                self.push_kinded(v as u64, NativeKind::UInt16)
            }
            TypedArrayData::U32(buf) => {
                let v = *buf.data.get(index).ok_or_else(|| oob(buf.data.len()))?;
                self.push_kinded(v as u64, NativeKind::UInt32)
            }
            TypedArrayData::U64(buf) => {
                let v = *buf.data.get(index).ok_or_else(|| oob(buf.data.len()))?;
                self.push_kinded(v, NativeKind::UInt64)
            }
            TypedArrayData::F32(buf) => {
                // F32 read widens to f64 to match the cross-crate
                // numeric-element contract (`typed_array_read_index_raw`
                // in `executor/variables/mod.rs`).
                let v = *buf.data.get(index).ok_or_else(|| oob(buf.data.len()))?;
                self.push_kinded((v as f64).to_bits(), NativeKind::Float64)
            }
            TypedArrayData::String(buf) => {
                let s_arc = buf.data.get(index).ok_or_else(|| oob(buf.data.len()))?;
                // The buffer holds the canonical `Arc<String>` share;
                // clone it for the pushed slot so the buffer's share
                // remains intact.
                let pushed: Arc<String> = Arc::clone(s_arc);
                let bits = Arc::into_raw(pushed) as u64;
                // `clone_with_kind` here would double-bump because
                // `Arc::clone` already added one share above. Push the
                // raw pointer bits directly with the matching kind.
                self.push_kinded(bits, NativeKind::String)
            }
            TypedArrayData::FloatSlice {
                parent,
                offset,
                len,
            } => {
                let len = *len as usize;
                let offset = *offset as usize;
                if index >= len {
                    return Err(oob(len));
                }
                let v = parent.data[offset + index];
                self.push_kinded(v.to_bits(), NativeKind::Float64)
            }
            TypedArrayData::HeapValue(_) | TypedArrayData::Matrix(_) => {
                Err(VMError::NotImplemented(format!(
                    "SURFACE: GetProp on TypedArray::{} variant — \
                     Phase-2c reentry (ADR-006 §2.7.4); needs the \
                     Arc<HeapValue>→KindedSlot projection (HeapValue \
                     payload) or a Matrix 2D-index opcode",
                    arr.type_name()
                )))
            }
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
    /// **Migrated paths:**
    /// - `TypedObject` (Wave 6.5): slot count.
    /// - `TypedArray` (W9): per-variant element count.
    /// - `String` / `Ptr(String)` (W9): `chars().count()` per the
    ///   `v2_string_len` precedent in `string_methods.rs`.
    /// - `HashMap` (W9): `HashMapData::len()`.
    ///
    /// Other heap receivers (NativeView, Temporal, TableView, DataTable,
    /// FilterExpr, SharedCell, Reference, etc.) have no semantic length
    /// — they remain TypeError. The remaining SURFACE arm is reserved
    /// for Phase-2c additions if a new length-bearing HeapKind is added.
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
            NativeKind::Ptr(HeapKind::TypedArray) => {
                if bits == 0 {
                    Err(VMError::RuntimeError(
                        "length() on null TypedArray".to_string(),
                    ))
                } else {
                    // SAFETY: kind says `Ptr(HeapKind::TypedArray)`; bits
                    // are `Arc::into_raw::<TypedArrayData>`. Transient borrow.
                    let arr: Arc<TypedArrayData> =
                        unsafe { Arc::from_raw(bits as *const _) };
                    let len = typed_array_len(&arr) as i64;
                    let _ = Arc::into_raw(arr);
                    self.push_kinded(len as u64, NativeKind::Int64)
                }
            }
            NativeKind::String | NativeKind::Ptr(HeapKind::String) => {
                if bits == 0 {
                    Err(VMError::RuntimeError(
                        "length() on null string".to_string(),
                    ))
                } else {
                    // SAFETY: kind is `String` / `Ptr(HeapKind::String)`;
                    // bits are `Arc::into_raw::<String>`. Transient borrow.
                    let s: Arc<String> = unsafe { Arc::from_raw(bits as *const String) };
                    let len = s.chars().count() as i64;
                    let _ = Arc::into_raw(s);
                    self.push_kinded(len as u64, NativeKind::Int64)
                }
            }
            NativeKind::Ptr(HeapKind::HashMap) => {
                if bits == 0 {
                    Err(VMError::RuntimeError(
                        "length() on null HashMap".to_string(),
                    ))
                } else {
                    // SAFETY: kind says `Ptr(HeapKind::HashMap)`; bits are
                    // `Arc::into_raw::<HashMapData>`. Transient borrow.
                    let map: Arc<HashMapData> =
                        unsafe { Arc::from_raw(bits as *const HashMapData) };
                    let len = map.len() as i64;
                    let _ = Arc::into_raw(map);
                    self.push_kinded(len as u64, NativeKind::Int64)
                }
            }
            // Other heap kinds — no semantic length. (BigInt, Decimal,
            // Char, Future, Closure, Instant, IoHandle, NativeScalar,
            // NativeView, TableView, Temporal, DataTable, TaskGroup,
            // Content, FilterExpr, Reference, SharedCell.)
            NativeKind::Ptr(_) => Err(VMError::TypeError {
                expected: "array, object, string, or hashmap",
                got: "heap value without length semantics",
            }),
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

/// Element count for a `TypedArrayData`, dispatching on the variant.
/// Mirrors the local helper in `executor/objects/array_basic.rs` (kept
/// duplicated to avoid forcing a shared-helpers cluster cascade in this
/// sub-cluster).
#[inline]
fn typed_array_len(arr: &TypedArrayData) -> usize {
    match arr {
        TypedArrayData::I64(b) => b.data.len(),
        TypedArrayData::F64(b) => b.data.len(),
        TypedArrayData::Bool(b) => b.data.len(),
        TypedArrayData::I8(b) => b.data.len(),
        TypedArrayData::I16(b) => b.data.len(),
        TypedArrayData::I32(b) => b.data.len(),
        TypedArrayData::U8(b) => b.data.len(),
        TypedArrayData::U16(b) => b.data.len(),
        TypedArrayData::U32(b) => b.data.len(),
        TypedArrayData::U64(b) => b.data.len(),
        TypedArrayData::F32(b) => b.data.len(),
        TypedArrayData::String(b) => b.data.len(),
        TypedArrayData::HeapValue(b) => b.data.len(),
        TypedArrayData::Matrix(m) => m.data.len(),
        TypedArrayData::FloatSlice { len, .. } => *len as usize,
    }
}

/// Convert a kinded `(bits, kind)` pair into a `usize` index. Accepts
/// `Int64` / `UInt64` / `Float64` (truncating to integer) — matches the
/// constants the compiler emits for `arr[i]` (`Constant::Int` /
/// `Constant::Number`). Negative or non-finite values are rejected.
#[inline]
fn numeric_index_from_kinded(bits: u64, kind: NativeKind) -> Result<usize, VMError> {
    let i = match kind {
        NativeKind::Int64 => bits as i64,
        NativeKind::Int8 => (bits as i8) as i64,
        NativeKind::Int16 => (bits as i16) as i64,
        NativeKind::Int32 => (bits as i32) as i64,
        NativeKind::UInt8 => (bits as u8) as i64,
        NativeKind::UInt16 => (bits as u16) as i64,
        NativeKind::UInt32 => (bits as u32) as i64,
        NativeKind::UInt64 => bits as i64,
        NativeKind::Float64 => {
            let f = f64::from_bits(bits);
            if !f.is_finite() {
                return Err(VMError::TypeError {
                    expected: "finite numeric index",
                    got: "non-finite float",
                });
            }
            f as i64
        }
        NativeKind::Bool => {
            if bits != 0 {
                1
            } else {
                0
            }
        }
        _ => {
            return Err(VMError::TypeError {
                expected: "numeric array index",
                got: "non-numeric key kind",
            });
        }
    };
    if i < 0 {
        return Err(VMError::IndexOutOfBounds {
            index: i as i32,
            length: 0,
        });
    }
    Ok(i as usize)
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

    /// `op_length` on `NativeKind::String` returns the codepoint count
    /// (`chars().count()`). Migrated path (W9 fill).
    #[test]
    fn length_string_returns_chars_count() {
        let mut vm = VirtualMachine::new(VMConfig::default());
        let s: Arc<String> = Arc::new("hello".to_string());
        let bits = Arc::into_raw(s) as u64;
        vm.push_kinded(bits, NativeKind::String).unwrap();
        vm.op_length().unwrap();
        let (len_bits, len_kind) = vm.pop_kinded().unwrap();
        assert_eq!(len_bits, 5);
        assert_eq!(len_kind, NativeKind::Int64);
    }

    /// `op_length` on a TypedArray scalar variant returns the element
    /// count as Int64 (W9 fill).
    #[test]
    fn length_typed_array_i64() {
        use shape_value::heap_value::TypedArrayData;
        use shape_value::typed_buffer::TypedBuffer;
        let mut vm = VirtualMachine::new(VMConfig::default());
        let buf = TypedBuffer::from_vec(vec![10i64, 20, 30, 40]);
        let arr = Arc::new(TypedArrayData::I64(Arc::new(buf)));
        let bits = Arc::into_raw(arr) as u64;
        vm.push_kinded(bits, NativeKind::Ptr(HeapKind::TypedArray))
            .unwrap();
        vm.op_length().unwrap();
        let (len_bits, len_kind) = vm.pop_kinded().unwrap();
        assert_eq!(len_bits, 4);
        assert_eq!(len_kind, NativeKind::Int64);
    }

    /// `op_get_prop` on a TypedArray with `Int64` key reads the indexed
    /// element with the variant's element kind (W9 fill).
    #[test]
    fn get_prop_typed_array_i64_index() {
        use shape_value::heap_value::TypedArrayData;
        use shape_value::typed_buffer::TypedBuffer;
        let mut vm = VirtualMachine::new(VMConfig::default());
        let buf = TypedBuffer::from_vec(vec![10i64, 20, 30, 40]);
        let arr = Arc::new(TypedArrayData::I64(Arc::new(buf)));
        let bits = Arc::into_raw(arr) as u64;
        vm.push_kinded(bits, NativeKind::Ptr(HeapKind::TypedArray))
            .unwrap();
        vm.push_kinded(2u64, NativeKind::Int64).unwrap();
        vm.op_get_prop(None).unwrap();
        let (v_bits, v_kind) = vm.pop_kinded().unwrap();
        assert_eq!(v_bits as i64, 30);
        assert_eq!(v_kind, NativeKind::Int64);
    }
}
