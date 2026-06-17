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
            NativeKind::String | NativeKind::Ptr(HeapKind::String) => {
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
        key_bits: u64,
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

            // ── v2 typed array (raw `*mut TypedArray<T>` pointer) ────────
            //
            // Phase 4b Round 4 W15 LANG-9-spin-3-first VM fix. ADR-006
            // §2.7.5 producer-side stamp: v2 typed-array allocators stamp
            // `HEAP_KIND_V2_TYPED_ARRAY` (kind=4) + `ELEM_TYPE_*` byte in
            // the header at `v2_handlers/array.rs` allocation; consumer
            // recovers via `as_v2_typed_array(bits, kind)` + per-T
            // `read_element` from `v2_handlers/v2_array_detect.rs`.
            //
            // r5c-2-β-CKPT-C u64-carrier-disambiguation (2026-05-20): the
            // v2-typed-array carrier kind is `NativeKind::Ptr(HeapKind::
            // TypedArray)` — the single canonical carrier for every v2-raw
            // `*mut TypedArray<T>` pointer (direct `NewTypedArray*`
            // allocation + refcounted struct-field / closure-capture read).
            // A genuine scalar `u64` (`NativeKind::UInt64`) is NOT an array
            // receiver — it falls through to the inline-scalar `_` arm
            // below as a TypeError, never dereferenced. The pre-fix
            // `UInt64 | Ptr(HeapKind::TypedArray)` arm dereferenced an
            // arbitrary scalar `u64` value as a header pointer → SIGSEGV.
            //
            // This arm closes the dispatch path for `arr[i]` (parsed as
            // `IndexAccess`, lowered to `GetProp` at
            // `compiler/expressions/property_access.rs:642` when the
            // receiver isn't a tracked typed-array local) for receivers
            // flowing through function params (e.g. `self` in stdlib
            // `Vec.first()` body `self[0]` at
            // `crates/shape-runtime/stdlib-src/core/vec.shape:23`). The
            // JIT-side `mir_compiler/v2_array.rs::v2_array_get` is the
            // sibling consumer for the same receiver shape — VM and JIT
            // share the producer-stamped header byte; no parallel-carrier
            // duality.
            NativeKind::Ptr(HeapKind::TypedArray) => {
                use crate::executor::v2_handlers::v2_array_detect::{
                    as_v2_typed_array, read_element,
                };
                let view = match as_v2_typed_array(obj_bits, obj_kind) {
                    Some(v) => v,
                    None => {
                        // Kind says TypedArray but the bits aren't a v2
                        // typed-array pointer (missing header) — surface.
                        return Err(VMError::TypeError {
                            expected: "object, array, string, or other heap value",
                            got: "scalar",
                        });
                    }
                };
                let idx = numeric_index_from_kinded(key_bits, key_kind)?;
                if (idx as u64) >= view.len as u64 {
                    return Err(VMError::IndexOutOfBounds {
                        index: idx as i32,
                        length: view.len as usize,
                    });
                }
                match read_element(&view, idx as u32) {
                    Some((bits, kind)) => self.push_kinded(bits, kind),
                    None => Err(VMError::IndexOutOfBounds {
                        index: idx as i32,
                        length: view.len as usize,
                    }),
                }
            }

            // ── String index `s[i]` — the i-th character ────────────────
            //
            // Book model (`fundamentals/strings.mdx` llm_summary + operators.mdx
            // §Indexing): `s[i]` reads the i-th character of a `string`. Shape
            // has NO first-class `char` type (STAGE-S4) — a single character is
            // a real 1-char `NativeKind::String`. This makes `s[i]` exact sugar
            // for `s.charAt(i)`: same producer (`op_string_char_at` /
            // `v2_string_char_at`), same Unicode `chars().nth(i)` codepoint
            // semantics, same out-of-range neutral (empty string — incl. a
            // negative `int` index, which the string char-model treats as a
            // miss rather than an array-style `IndexOutOfBounds`). Materializes
            // a fresh `Arc<String>` — NO bit-reinterpret of the codepoint into
            // a pointer slot (the deleted `NativeKind::Char`-into-`Array<string>`
            // SIGSEGV the S4 fix retired). Accepts all three string carriers
            // (`String` / `Ptr(HeapKind::String)` Arc carriers + the v2-raw
            // `StringV2` `*const StringObj`) via `borrow_string_for_index`,
            // which copies bytes and does NOT consume the share the popped
            // slot owns (the caller's `drop_with_kind` releases it).
            NativeKind::String
            | NativeKind::Ptr(HeapKind::String)
            | NativeKind::StringV2 => {
                let s = match borrow_string_for_index(obj_bits, obj_kind) {
                    Some(s) => s,
                    None => {
                        // Kind says a string carrier but bits are null —
                        // an empty string indexes to the empty neutral.
                        String::new()
                    }
                };
                let idx = string_index_from_kinded(key_bits, key_kind)?;
                // Negative `idx` (sign-preserved as i64) and past-the-end both
                // miss `chars().nth(_)` → empty string, identical to charAt.
                let ch_str = if idx < 0 {
                    String::new()
                } else {
                    match s.chars().nth(idx as usize) {
                        Some(ch) => ch.to_string(),
                        None => String::new(),
                    }
                };
                let bits = Arc::into_raw(Arc::new(ch_str)) as u64;
                self.push_kinded(bits, NativeKind::String)
            }

            // ── HashMap, NativeView, Temporal, TableView,
            //    DataTable, Decimal, BigInt, etc. ─────────────────────────
            NativeKind::Ptr(_) => Err(VMError::NotImplemented(format!(
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
                shape_runtime::type_schema::lookup_schema_by_id_public(storage.schema_id as u32)
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

            let write_result =
                self.write_typed_object_field_by_name(&storage_arc, ks, val_bits, val_kind);

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
                shape_runtime::type_schema::lookup_schema_by_id_public(storage.schema_id as u32)
            });
        let Some(schema) = schema_owned.as_ref() else {
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
                ) | (NativeKind::String, NativeKind::Ptr(HeapKind::String),)
                    | (NativeKind::Ptr(HeapKind::String), NativeKind::String,)
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

    /// `SetLocalIndex { Operand::Local(idx) }`: `arr[i] = value` where the
    /// local at `idx` holds the array directly (`*mut TypedArray<T>`,
    /// kind `Ptr(HeapKind::TypedArray)`).
    ///
    /// ## V3-S5 Seam #2 (2026-06-05)
    ///
    /// Routes through the live flat-struct `TypedArray<T>` carrier — the
    /// `write_element` per-element-kind dispatch (NOT a resurrected
    /// `Arc<TypedArrayData>` heterogeneous-element carrier; REFUSED ON
    /// SIGHT, Refusal #1). Mirrors `op_set_index_ref` but reads the array
    /// pointer straight from the local slot instead of through a `RefTarget`.
    ///
    /// Stack discipline: pops [index, value] (value on top). The value
    /// share is transferred to the array element by `write_element`; the
    /// index share is retired. The local retains its own array share (we
    /// only borrow the pointer).
    pub(in crate::executor) fn op_set_local_index(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use crate::executor::v2_handlers::v2_array_detect::{as_v2_typed_array, write_element};
        let Some(Operand::Local(local_idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        // Pop value (top) then index — we own both shares.
        let (val_bits, val_kind) = self.pop_kinded()?;
        let (key_bits, key_kind) = self.pop_kinded()?;
        let index = match numeric_index_from_kinded(key_bits, key_kind) {
            Ok(i) => i as u32,
            Err(e) => {
                drop_with_kind(key_bits, key_kind);
                drop_with_kind(val_bits, val_kind);
                return Err(e);
            }
        };
        drop_with_kind(key_bits, key_kind);

        // Borrow the array pointer from the local slot — the local retains
        // its own share.
        let bp = self.current_locals_base();
        let slot = bp + local_idx as usize;
        if slot >= self.stack.len() {
            drop_with_kind(val_bits, val_kind);
            return Err(VMError::RuntimeError(format!(
                "SetLocalIndex slot {} out of bounds (stack len {})",
                local_idx,
                self.stack.len()
            )));
        }
        let (arr_bits, arr_kind) = self.stack_read_kinded_raw(slot);
        if arr_kind != NativeKind::Ptr(HeapKind::TypedArray) {
            drop_with_kind(val_bits, val_kind);
            return Err(VMError::TypeError {
                expected: "array (TypedArray) local for index assignment",
                got: "non-array local",
            });
        }
        let view = match as_v2_typed_array(arr_bits, arr_kind) {
            Some(v) => v,
            None => {
                drop_with_kind(val_bits, val_kind);
                return Err(VMError::NotImplemented(
                    "SetLocalIndex: TypedArray local did not resolve to a v2 \
                     typed-array pointer (HEAP_KIND_V2_TYPED_ARRAY header \
                     missing). ADR-006 §2.7.6 / §2.7.7."
                        .to_string(),
                ));
            }
        };
        if index >= view.len {
            drop_with_kind(val_bits, val_kind);
            return Err(VMError::IndexOutOfBounds {
                index: index as i32,
                length: view.len as usize,
            });
        }
        crate::memory::record_heap_write();
        match write_element(&view, index, val_bits, val_kind) {
            Ok(()) => Ok(()),
            Err(msg) => {
                drop_with_kind(val_bits, val_kind);
                Err(VMError::TypeError {
                    expected: "v2 typed-array element",
                    got: msg,
                })
            }
        }
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
                // r5c-2-β-CKPT-C: the v2-raw `*mut TypedArray<T>` carrier —
                // length is the `len` field at a fixed offset (T-independent),
                // read via `as_v2_typed_array`. `Ptr(HeapKind::TypedArray)`
                // is the single canonical carrier kind. A genuine scalar
                // `u64` (`NativeKind::UInt64`) is NOT an array — it falls
                // through to the inline-scalar `_` arm as a TypeError, never
                // dereferenced (the pre-fix `UInt64` arm dereferenced an
                // arbitrary scalar value as a header pointer → SIGSEGV).
                if bits == 0 {
                    Err(VMError::RuntimeError(
                        "length() on null TypedArray".to_string(),
                    ))
                } else {
                    use crate::executor::v2_handlers::v2_array_detect::as_v2_typed_array;
                    match as_v2_typed_array(bits, kind) {
                        Some(view) => self.push_kinded(view.len as u64, NativeKind::Int64),
                        None => Err(VMError::TypeError {
                            expected: "array, object, or string",
                            got: "scalar",
                        }),
                    }
                }
            }
            NativeKind::String | NativeKind::Ptr(HeapKind::String) => {
                if bits == 0 {
                    Err(VMError::RuntimeError("length() on null string".to_string()))
                } else {
                    // SAFETY: kind is `String` / `Ptr(HeapKind::String)`;
                    // bits are `Arc::into_raw::<String>`. Transient borrow.
                    let s: Arc<String> = unsafe { Arc::from_raw(bits as *const String) };
                    let len = s.chars().count() as i64;
                    let _ = Arc::into_raw(s);
                    self.push_kinded(len as u64, NativeKind::Int64)
                }
            }
            NativeKind::StringV2 => {
                // C3: `Array<string>` elements read back with the v2-raw
                // `*const StringObj` carrier (`NativeKind::StringV2`), not the
                // Arc<String> carrier. `.length` on such an element reached the
                // scalar `_` arm and raised a spurious TypeError. Borrow the
                // StringObj's UTF-8 bytes and count chars to match the
                // `String`/`Ptr(String)` arm's semantics exactly. The popped
                // share is retired by `drop_with_kind` below.
                if bits == 0 {
                    Err(VMError::RuntimeError("length() on null string".to_string()))
                } else {
                    use shape_value::v2::string_obj::StringObj;
                    // SAFETY: kind == StringV2 means bits = `*const StringObj`
                    // (the v2-raw carrier the element-read producer stamped).
                    let s = unsafe { StringObj::as_str(bits as *const StringObj) };
                    let len = s.chars().count() as i64;
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
                    let map: Arc<HashMapKindedRef> =
                        unsafe { Arc::from_raw(bits as *const HashMapKindedRef) };
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
/// Phase 4b Round 4 W15 LANG-9-spin-3-first: now reachable via the v2
/// typed-array `UInt64` arm of `dispatch_get_prop` for `arr[i]` reads
/// flowing through stdlib `Vec.first()` body's `self[0]` shape.
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

/// Sign-preserving `(bits, kind) → i64` projection for the `s[i]` string
/// index path. Unlike `numeric_index_from_kinded` (which rejects negatives
/// with `IndexOutOfBounds` for the array-receiver model), the string
/// char-model treats a negative or out-of-range index as a *miss* that
/// yields the empty string (charAt parity, STAGE-S4). So the caller needs
/// the signed value back, not an early error — only a genuinely non-numeric
/// key kind is a `TypeError`.
#[inline]
fn string_index_from_kinded(bits: u64, kind: NativeKind) -> Result<i64, VMError> {
    match kind {
        NativeKind::Int8 => Ok((bits as i8) as i64),
        NativeKind::Int16 => Ok((bits as i16) as i64),
        NativeKind::Int32 => Ok((bits as i32) as i64),
        NativeKind::Int64 | NativeKind::IntSize => Ok(bits as i64),
        NativeKind::UInt8 => Ok((bits as u8) as i64),
        NativeKind::UInt16 => Ok((bits as u16) as i64),
        NativeKind::UInt32 => Ok((bits as u32) as i64),
        NativeKind::UInt64 | NativeKind::UIntSize => Ok(bits as i64),
        _ => Err(VMError::TypeError {
            expected: "int",
            got: "non-int string index",
        }),
    }
}

/// Borrow a string operand from any of the three string carriers
/// (`NativeKind::String` / `Ptr(HeapKind::String)` Arc carriers + the
/// v2-raw `StringV2` `*const StringObj`) and copy its bytes into an owned
/// `String`. Does NOT consume the strong-count share the popped slot owns —
/// the caller releases it via `drop_with_kind`. Returns `None` for a null
/// pointer (treated as the empty string by the index path). Mirrors
/// `typed_access::read_string_operand`; kept local to avoid a cross-module
/// pub of that file-private helper.
///
/// SAFETY: when `kind` is a string carrier, `bits` is the raw pointer the
/// matching producer stamped (`Arc::into_raw::<String>` for the Arc
/// carriers, `*const StringObj` for `StringV2`).
#[inline]
fn borrow_string_for_index(bits: u64, kind: NativeKind) -> Option<String> {
    if bits == 0 {
        return None;
    }
    match kind {
        NativeKind::String | NativeKind::Ptr(HeapKind::String) => {
            let s = unsafe { &*(bits as *const String) };
            Some(s.clone())
        }
        NativeKind::StringV2 => {
            let s = unsafe {
                shape_value::v2::string_obj::StringObj::as_str(
                    bits as *const shape_value::v2::string_obj::StringObj,
                )
            };
            Some(s.to_string())
        }
        _ => None,
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
        let schema = TypeSchema::new("Probe".to_string(), vec![("x".to_string(), FieldType::I64)]);
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

    /// Helper: drive `op_get_prop` for `string[index]` and return the
    /// resulting 1-char `String` (asserting the result kind is
    /// `NativeKind::String`, retiring its share). The receiver is pushed
    /// first, the index second (the stack order `op_get_prop` pops).
    fn string_index_via_get_prop(
        vm: &mut VirtualMachine,
        s: &str,
        index_bits: u64,
        index_kind: NativeKind,
    ) -> String {
        let recv: Arc<String> = Arc::new(s.to_string());
        let recv_bits = Arc::into_raw(recv) as u64;
        vm.push_kinded(recv_bits, NativeKind::String).unwrap();
        vm.push_kinded(index_bits, index_kind).unwrap();
        vm.op_get_prop(None).unwrap();
        let (out_bits, out_kind) = vm.pop_kinded().unwrap();
        assert_eq!(
            out_kind,
            NativeKind::String,
            "s[i] must yield a 1-char NativeKind::String (STAGE-S4 char model), got {:?}",
            out_kind
        );
        // SAFETY: kind == String means out_bits is Arc::into_raw::<String>
        // with one share owned by the popped slot.
        let arc: Arc<String> = unsafe { Arc::from_raw(out_bits as *const String) };
        let result = (*arc).clone();
        drop(arc);
        result
    }

    /// `s[i]` (string GetProp index) returns the i-th character as a real
    /// 1-char `string` — exact parity with `s.charAt(i)` (STAGE-S4 char
    /// model: Shape has no first-class `char` type; book
    /// `fundamentals/strings.mdx` llm_summary "Index chars via `s[i]`" +
    /// operators.mdx §Indexing). Covers in-range ASCII, multi-byte Unicode
    /// (codepoint indexing, NOT byte indexing), and the out-of-range +
    /// negative neutral (empty string, NOT an `IndexOutOfBounds` error).
    #[test]
    fn get_prop_string_index_returns_one_char_string() {
        let mut vm = VirtualMachine::new(VMConfig::default());

        // In-range ASCII: "hello"[1] == "e", "hello"[0] == "h".
        assert_eq!(
            string_index_via_get_prop(&mut vm, "hello", 1, NativeKind::Int64),
            "e"
        );
        assert_eq!(
            string_index_via_get_prop(&mut vm, "hello", 0, NativeKind::Int64),
            "h"
        );

        // Multi-byte Unicode is indexed by codepoint: "世界"[0] == "世",
        // "世界"[1] == "界" (each is a 3-byte UTF-8 sequence).
        assert_eq!(
            string_index_via_get_prop(&mut vm, "世界", 0, NativeKind::Int64),
            "世"
        );
        assert_eq!(
            string_index_via_get_prop(&mut vm, "世界", 1, NativeKind::Int64),
            "界"
        );

        // Past-the-end → empty string (string-model neutral, charAt parity).
        assert_eq!(
            string_index_via_get_prop(&mut vm, "hi", 5, NativeKind::Int64),
            ""
        );

        // Negative index (sign-preserved as i64) → empty string, NOT an
        // array-style IndexOutOfBounds. -1 in two's complement is a huge
        // u64; string_index_from_kinded recovers the signed -1 and the
        // index path maps idx < 0 to the empty neutral.
        assert_eq!(
            string_index_via_get_prop(&mut vm, "hi", (-1i64) as u64, NativeKind::Int64),
            ""
        );
    }
}
