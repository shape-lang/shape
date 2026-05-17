//! Property access operations (GetProp, SetProp, Length).
//!
//! Wave 6.5 cluster D sub-cluster D-prop-access (ADR-006 §2.7.6, §2.7.7,
//! §2.7.8). Heap dispatch uses per-`HeapKind` `Arc::from_raw` recovery
//! per ADR-006 §2.4 (typed-Arc slots).
//!
//! ## V3-S5 ckpt-3 consumer-cascade tier 2 surface (2026-05-15)
//!
//! Per V3-S5 ckpt-1 close (commit `aac8495e`, 2026-05-15), the
//! `TypedArrayData` enum + impl blocks + `Display for TypedArrayData` +
//! `typed_array_structural_eq` fn were DELETED at
//! `crates/shape-value/src/heap_value.rs` per W12-typed-array-data-deletion
//! audit §3.5 + ADR-006 §2.7.24 Q25.A SUPERSEDED. This file's previous
//! consumer-shape — TypedArray-receiver paths in `dispatch_get_prop`
//! (`read_typed_array_index` per-variant element read), `op_length`'s
//! `Ptr(HeapKind::TypedArray)` arm using `typed_array_len` over
//! `TypedArrayData::I64 / F64 / Bool / I8 / I16 / I32 / U8 / U16 / U32 /
//! U64 / F32 / String / Decimal / BigInt / Char / TypedObject` arms —
//! cascade-breaks here as the deletion's consumer cascade tier 2.
//!
//! TypedArray-receiver arms in `dispatch_get_prop` and `op_length` are
//! replaced with structured surface-and-stop returning
//! `VMError::NotImplemented`. Local helpers `read_typed_array_index` and
//! `typed_array_len` are DELETED. Tests `length_typed_array_i64` and
//! `get_prop_typed_array_i64_index` (which constructed
//! `TypedArrayData::I64` buffers) are DELETED.
//!
//! PRESERVED INTACT (no `TypedArrayData` dependency):
//! - `op_get_prop` outer / `dispatch_get_prop` non-TypedArray arms
//!   (TypedObject, HashMap/String SURFACE).
//! - `op_set_prop` full path: TypedObject mutation via
//!   `TypedObjectStorage::write_slot_in_place`; non-TypedObject SURFACE.
//! - `write_typed_object_field_by_name` — schema-driven kinded writer.
//! - `op_set_local_index` / `op_set_module_binding_index` — already SURFACE.
//! - `op_length`'s `TypedObject` / `String` / `HashMap` arms.
//! - `numeric_index_from_kinded` — pure `(bits, kind) → usize` projection.
//! - Tests `length_typed_object_empty`, `length_string_returns_chars_count`,
//!   `set_prop_typed_object_int_field`, `set_prop_typed_object_non_string_key_errors`.
//!
//! ## Cascade migration target (post-ckpt-6 STRICT close)
//!
//! Per W12-typed-array-data-deletion audit §A.3 + §2.1 scalar recipe +
//! §2.2 heap-element variants, every previous `TypedArrayData::X(buf)`
//! match arm in `read_typed_array_index` / `typed_array_len` migrates to
//! the v2-raw `TypedArray<T>` flat-struct carrier — per-T direct
//! `*buf.data.add(idx)` reads + per-T `data.len()` length.
//!
//! Bodies REFUSED ON SIGHT under Refusal #1 (resurrection under rename
//! per ckpt-1 close-marker at `heap_value.rs:3956`).

use crate::bytecode::{Instruction, Operand};
use crate::executor::VirtualMachine;
use crate::executor::vm_impl::stack::{clone_with_kind, drop_with_kind};
use shape_value::{
    NativeKind, VMError,
    heap_value::{HashMapKindedRef, HeapKind},
};
use std::sync::Arc;

// ═══════════════════════════════════════════════════════════════════════════
// V3-S5 ckpt-3 surface-and-stop builder
// ═══════════════════════════════════════════════════════════════════════════

/// Common surface-and-stop body for TypedArray-receiver arms in
/// `dispatch_get_prop` and `op_length`.
#[cold]
#[inline(never)]
fn ckpt3_surface(op: &'static str, key_kind: NativeKind) -> VMError {
    VMError::NotImplemented(format!(
        "{op}: SURFACE — V3-S5 ckpt-3 consumer-cascade tier 2 surface. \
         `TypedArrayData` enum DELETED at ckpt-1 (2026-05-15) per W12-\
         typed-array-data-deletion audit §3.5 + ADR-006 §2.7.24 Q25.A \
         SUPERSEDED. The previous `Arc<TypedArrayData>` receiver-recovery \
         + per-variant element-read / length-read dispatch path (~34 \
         references across `read_typed_array_index` 16 arms + \
         `typed_array_len` 16 arms) cascade-broke at the enum deletion \
         site (`crates/shape-value/src/heap_value.rs:3944`). Post-deletion \
         target is the v2-raw `TypedArray<T>` flat-struct carrier per \
         audit §1.2 + §A.3 + §3.1 scalar recipe + §2.2 heap-element \
         variants — per-T `*buf.data.add(idx)` element read + per-T \
         `data.len()` length; landing across ckpt-3 (this file plus \
         array_ops/typed_array_methods/iterator_methods/array_sort/concat/\
         array_query) + ckpt-4 (Buf<T> / HeapValue::TypedArray \
         arm / HeapKind::TypedArray ordinal) + ckpt-5 (wire/json/marshal \
         + 4-table lockstep) + ckpt-6 (JIT FFI). Key kind: {key_kind:?}. \
         UNREACHABLE until ckpt-6 STRICT close. REFUSED ON SIGHT: \
         TypedArrayData resurrection under any rename (Refusal #1, W12 \
         audit §7).",
        op = op,
        key_kind = key_kind,
    ))
}

impl VirtualMachine {
    /// `GetProp`: read a property from a heap object.
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
        drop_with_kind(key_bits, key_kind);
        drop_with_kind(obj_bits, obj_kind);

        result
    }

    /// Inner dispatch for `op_get_prop`.
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
                let storage_arc: Arc<shape_value::heap_value::TypedObjectStorage> =
                    unsafe { Arc::from_raw(obj_bits as *const _) };
                let result = self.read_typed_object_field(&storage_arc, ks);
                let _ = Arc::into_raw(storage_arc);
                result
            }

            // ── TypedArray: V3-S5 ckpt-3 surface-and-stop ────────────────
            //
            // Previous body: recover `Arc<TypedArrayData>` via
            // `Arc::from_raw`, dispatch on `TypedArrayData` variant for
            // raw-bits read + element kind per `read_typed_array_index`.
            // The 16-arm dispatch cascade-broke at ckpt-1.
            NativeKind::Ptr(HeapKind::TypedArray) => {
                Err(ckpt3_surface("GetProp(TypedArray)", key_kind))
            }

            // ── HashMap, String index, NativeView, Temporal, TableView,
            //    DataTable, Decimal, BigInt, etc. ─────────────────────────
            NativeKind::String
            | NativeKind::Ptr(_) => Err(VMError::NotImplemented(format!(
                "SURFACE: GetProp on {:?} not yet kinded — requires the \
                 W17-typed-carrier-monomorphization replacement for the \
                 deleted HashMapData::values: `Arc<Buf<Arc<HeapValue>>>` \
                 carrier (ADR-006 §2.7.24 Q25.B) or the per-receiver \
                 heterogeneous-kind body. Key kind observed: {:?}.",
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
            .cloned()
            .or_else(|| {
                shape_runtime::type_schema::lookup_schema_by_id_public(
                    storage.schema_id as u32,
                )
            })
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

        // WB2.4 retain-on-read.
        clone_with_kind(bits, kind);
        self.push_kinded(bits, kind)
    }

    /// `SetProp`: write a property on a heap object. Pops value, key,
    /// object; mutates object; pushes object back.
    pub(in crate::executor) fn op_set_prop(&mut self) -> Result<(), VMError> {
        let (val_bits, val_kind) = self.pop_kinded()?;
        let (key_bits, key_kind) = self.pop_kinded()?;
        let (obj_bits, obj_kind) = self.pop_kinded()?;

        if obj_kind == NativeKind::Ptr(HeapKind::TypedObject) {
            let key_str: Option<&str> = match key_kind {
                NativeKind::String | NativeKind::Ptr(HeapKind::String) => {
                    if key_bits == 0 {
                        None
                    } else {
                        // SAFETY: kind is `String`; bits are
                        // `Arc::into_raw::<String>` with one share owned
                        // by the popped slot.
                        let s: &String = unsafe { &*(key_bits as *const String) };
                        Some(s.as_str())
                    }
                }
                _ => None,
            };

            let Some(ks) = key_str else {
                drop_with_kind(val_bits, val_kind);
                drop_with_kind(key_bits, key_kind);
                drop_with_kind(obj_bits, obj_kind);
                return Err(VMError::TypeError {
                    expected: "string property name",
                    got: "non-string key",
                });
            };

            if obj_bits == 0 {
                drop_with_kind(val_bits, val_kind);
                drop_with_kind(key_bits, key_kind);
                return Err(VMError::RuntimeError(
                    "SetProp on null TypedObject".to_string(),
                ));
            }

            // SAFETY: kind says `Ptr(HeapKind::TypedObject)`; obj_bits is
            // `Arc::into_raw::<TypedObjectStorage>` with one share owned
            // by the popped slot.
            let storage_arc: Arc<shape_value::heap_value::TypedObjectStorage> =
                unsafe { Arc::from_raw(obj_bits as *const _) };

            let write_result = self.write_typed_object_field_by_name(
                &storage_arc,
                ks,
                val_bits,
                val_kind,
            );

            let obj_bits_back = Arc::into_raw(storage_arc) as u64;

            drop_with_kind(key_bits, key_kind);

            return match write_result {
                Ok(()) => self.push_kinded(obj_bits_back, obj_kind),
                Err(e) => {
                    drop_with_kind(obj_bits_back, obj_kind);
                    Err(e)
                }
            };
        }

        // Non-TypedObject receivers: drain and surface.
        drop_with_kind(val_bits, val_kind);
        drop_with_kind(key_bits, key_kind);
        drop_with_kind(obj_bits, obj_kind);
        Err(VMError::NotImplemented(format!(
            "SURFACE: SetProp on {:?} not yet kinded — Phase-2c reentry \
             (ADR-006 §2.7.4 + §2.7.24 Q25.A). TypedObject receivers are \
             filled in W17-typed-object-mutation; other heap receivers \
             (HashMap, etc.) require the W17-typed-carrier-monomorphization \
             sub-cluster's per-receiver heterogeneous-kind body. Key kind \
             observed: {:?}.",
            obj_kind, key_kind,
        )))
    }

    /// Write a named field on a `TypedObjectStorage`.
    fn write_typed_object_field_by_name(
        &mut self,
        storage: &Arc<shape_value::heap_value::TypedObjectStorage>,
        key: &str,
        val_bits: u64,
        val_kind: NativeKind,
    ) -> Result<(), VMError> {
        let schema_owned = self
            .program
            .type_schema_registry
            .get_by_id(storage.schema_id as u32)
            .cloned()
            .or_else(|| {
                shape_runtime::type_schema::lookup_schema_by_id_public(
                    storage.schema_id as u32,
                )
            });
        let Some(schema) = schema_owned.as_ref()
        else {
            drop_with_kind(val_bits, val_kind);
            return Err(VMError::RuntimeError(format!(
                "Schema {} not found in registry",
                storage.schema_id
            )));
        };
        let Some(field) = schema.get_field(key) else {
            drop_with_kind(val_bits, val_kind);
            return Err(VMError::UndefinedProperty(key.to_string()));
        };
        let idx = field.index as usize;
        if idx >= storage.slots.len() {
            drop_with_kind(val_bits, val_kind);
            return Err(VMError::RuntimeError(format!(
                "Field '{}' index {} exceeds slot count {}",
                key,
                idx,
                storage.slots.len()
            )));
        }
        if idx >= storage.field_kinds.len() {
            drop_with_kind(val_bits, val_kind);
            return Err(VMError::RuntimeError(format!(
                "Field '{}' index {} exceeds field_kinds length {}",
                key,
                idx,
                storage.field_kinds.len()
            )));
        }

        let stored_kind = storage.field_kinds[idx];
        let kind_compatible = val_kind == stored_kind
            || matches!(
                (stored_kind, val_kind),
                (
                    NativeKind::Int64,
                    NativeKind::Int8
                        | NativeKind::Int16
                        | NativeKind::Int32
                        | NativeKind::UInt8
                        | NativeKind::UInt16
                        | NativeKind::UInt32
                        | NativeKind::UInt64,
                ) | (
                    NativeKind::String,
                    NativeKind::Ptr(HeapKind::String),
                ) | (
                    NativeKind::Ptr(HeapKind::String),
                    NativeKind::String,
                )
            );
        if !kind_compatible {
            drop_with_kind(val_bits, val_kind);
            return Err(VMError::TypeError {
                expected: "value kind matching field schema",
                got: "mismatched kind",
            });
        }

        let prior_bits = storage.slots[idx].raw();
        crate::memory::write_barrier_slot(prior_bits, val_bits);

        // SAFETY: per `TypedObjectStorage::write_slot_in_place` contract.
        let _returned_prior = unsafe { storage.write_slot_in_place(idx, val_bits) };
        debug_assert_eq!(
            _returned_prior, prior_bits,
            "SetProp: write_slot_in_place prior_bits mismatch — \
             concurrent write detected? ADR-006 §2.7.13 / Q14",
        );

        drop_with_kind(prior_bits, stored_kind);
        Ok(())
    }

    /// `SetLocalIndex`: in-place index assignment on a local. SURFACE.
    pub(in crate::executor) fn op_set_local_index(
        &mut self,
        _instruction: &Instruction,
    ) -> Result<(), VMError> {
        let (val_bits, val_kind) = self.pop_kinded()?;
        let (key_bits, key_kind) = self.pop_kinded()?;
        drop_with_kind(val_bits, val_kind);
        drop_with_kind(key_bits, key_kind);
        Err(VMError::NotImplemented(format!(
            "SURFACE: SetLocalIndex requires the W17-typed-carrier-\
             monomorphization replacement for the deleted \
             the-deleted-heterogeneous-element-carrier heterogeneous-element carrier \
             (ADR-006 §2.7.24 Q25.A). Typed-array fast path \
             (TypedArraySet{{I64,F64,Bool,...}}) is the supported \
             surface today; this opcode covers the fallback shapes \
             that need the carrier-monomorphization rebuild. Key \
             kind observed: {:?}.",
            key_kind,
        )))
    }

    /// `SetModuleBindingIndex`: in-place index assignment on a module
    /// binding. SURFACE.
    pub(in crate::executor) fn op_set_module_binding_index(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let (val_bits, val_kind) = self.pop_kinded()?;
        let (key_bits, key_kind) = self.pop_kinded()?;
        drop_with_kind(val_bits, val_kind);
        drop_with_kind(key_bits, key_kind);
        let _binding_idx = match instruction.operand {
            Some(Operand::ModuleBinding(idx)) => idx as usize,
            _ => return Err(VMError::InvalidOperand),
        };
        Err(VMError::NotImplemented(format!(
            "SURFACE: SetModuleBindingIndex requires the W17-typed-\
             carrier-monomorphization replacement for the deleted \
             the-deleted-heterogeneous-element-carrier heterogeneous-element carrier \
             (ADR-006 §2.7.24 Q25.A). Key kind observed: {:?}.",
            key_kind,
        )))
    }

    /// `Length`: read the length of an array, string, hashmap, etc.
    ///
    /// **Migrated paths (preserved through V3-S5 ckpt-3):**
    /// - `TypedObject`: slot count.
    /// - `String` / `Ptr(String)`: `chars().count()` per the
    ///   `v2_string_len` precedent in `string_methods.rs`.
    /// - `HashMap`: `HashMapKindedRef::len()`.
    ///
    /// **V3-S5 ckpt-3 surface:**
    /// - `Ptr(HeapKind::TypedArray)` arm — `TypedArrayData` enum gone;
    ///   per-variant `typed_array_len` cascade-broke at ckpt-1.
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
                // V3-S5 ckpt-3 surface-and-stop — TypedArrayData enum gone.
                if bits == 0 {
                    Err(VMError::RuntimeError(
                        "length() on null TypedArray".to_string(),
                    ))
                } else {
                    Err(ckpt3_surface("Length(TypedArray)", kind))
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
                    // Wave 2 Round 3b C2-joint ckpt-2 (2026-05-14): bits are
                    // `Arc::into_raw(Arc<HashMapKindedRef>)`. Transient
                    // borrow to read `len()` via the kinded ref accessor.
                    let map: Arc<HashMapKindedRef> = unsafe {
                        Arc::from_raw(bits as *const HashMapKindedRef)
                    };
                    let len = map.len() as i64;
                    let _ = Arc::into_raw(map);
                    self.push_kinded(len as u64, NativeKind::Int64)
                }
            }
            // Other heap kinds — no semantic length.
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

/// Convert a kinded `(bits, kind)` pair into a `usize` index. Accepts
/// `Int64` / `UInt64` / `Float64` (truncating to integer) — matches the
/// constants the compiler emits for `arr[i]`.
///
/// Preserved through V3-S5 ckpt-3: no `TypedArrayData` dependency.
#[inline]
#[allow(dead_code)]
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
    ///
    /// W5 v0.3 fix (2026-05-17): constructed via `TypedObjectStorage::_new`
    /// to match production carrier shape. The previous shape used the
    /// legacy `Arc::new(...)` + `Arc::into_raw` pattern whose bits flowed
    /// into the v2-raw `drop_with_kind(Ptr(HeapKind::TypedObject))`
    /// dispatch — which calls `release_elem` → `_drop` → `std::alloc::
    /// dealloc(ptr, Layout::new::<Self>())`. Arc-allocated memory has
    /// the `ArcInner` header before `T` and a different layout, so the
    /// dealloc with `Layout::new::<TypedObjectStorage>` on an
    /// `Arc::into_raw`'d pointer is a wrong-allocator-pair free →
    /// `free(): invalid size` SIGABRT.
    #[test]
    fn length_typed_object_empty() {
        let mut vm = VirtualMachine::new(VMConfig::default());
        let ptr = TypedObjectStorage::_new(
            0,
            Vec::<ValueSlot>::new().into_boxed_slice(),
            0,
            Arc::from(Vec::<NativeKind>::new().into_boxed_slice()),
        );
        let bits = ptr as u64;
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

    /// `op_set_prop` on a TypedObject with a string key writes the
    /// matching field in place. W17-typed-object-mutation fill
    /// (2026-05-11).
    ///
    /// W5 v0.3 fix (2026-05-17): migrated to `_new` carrier per
    /// `length_typed_object_empty` rationale.
    #[test]
    fn set_prop_typed_object_int_field() {
        use shape_runtime::type_schema::{FieldType, TypeSchema};
        let mut vm = VirtualMachine::new(VMConfig::default());

        // Build a single-field schema (`x: int`) and register it.
        let schema = TypeSchema::new(
            "Probe".to_string(),
            vec![("x".to_string(), FieldType::I64)],
        );
        let schema_id = schema.id;
        vm.program.type_schema_registry.register(schema);

        // Construct a storage with x = 7.
        let slot = ValueSlot::from_raw(7u64);
        let ptr = TypedObjectStorage::_new(
            schema_id as u64,
            vec![slot].into_boxed_slice(),
            0, // heap_mask: no heap fields
            Arc::from(vec![NativeKind::Int64].into_boxed_slice()),
        );
        let recv_bits = ptr as u64;

        // Push (recv, key, val) to match `op_set_prop`'s pop order.
        vm.push_kinded(recv_bits, NativeKind::Ptr(HeapKind::TypedObject))
            .unwrap();
        let key_arc: Arc<String> = Arc::new("x".to_string());
        let key_bits = Arc::into_raw(key_arc) as u64;
        vm.push_kinded(key_bits, NativeKind::String).unwrap();
        vm.push_kinded(42u64, NativeKind::Int64).unwrap();

        vm.op_set_prop().unwrap();

        // op_set_prop pushes the (mutated) receiver back.
        let (obj_bits_back, obj_kind_back) = vm.pop_kinded().unwrap();
        assert_eq!(obj_kind_back, NativeKind::Ptr(HeapKind::TypedObject));
        // Recover via raw-pointer borrow (matches v2-raw carrier shape);
        // assert the slot's new value, then retire the popped share via
        // drop_with_kind.
        // SAFETY: `obj_bits_back` came from the v2-raw `_new` allocator
        // and op_set_prop pushed the (mutated) receiver back without
        // changing its allocator provenance.
        let storage_back: &TypedObjectStorage =
            unsafe { &*(obj_bits_back as *const TypedObjectStorage) };
        assert_eq!(storage_back.slots[0].raw(), 42u64);
        // Release the popped share through the v2-raw drop dispatch.
        crate::executor::vm_impl::stack::drop_with_kind(obj_bits_back, obj_kind_back);
    }

    /// `op_set_prop` on a TypedObject with a non-string key returns a
    /// TypeError and balances the kind track via the drain branch.
    ///
    /// W5 v0.3 fix (2026-05-17): migrated to `_new` carrier per
    /// `length_typed_object_empty` rationale.
    #[test]
    fn set_prop_typed_object_non_string_key_errors() {
        let mut vm = VirtualMachine::new(VMConfig::default());
        let ptr = TypedObjectStorage::_new(
            0,
            Vec::<ValueSlot>::new().into_boxed_slice(),
            0,
            Arc::from(Vec::<NativeKind>::new().into_boxed_slice()),
        );
        let recv_bits = ptr as u64;

        vm.push_kinded(recv_bits, NativeKind::Ptr(HeapKind::TypedObject))
            .unwrap();
        vm.push_kinded(0u64, NativeKind::Int64).unwrap(); // non-string key
        vm.push_kinded(1u64, NativeKind::Int64).unwrap(); // value

        let err = vm.op_set_prop().unwrap_err();
        assert!(matches!(err, VMError::TypeError { .. }));
    }
}
