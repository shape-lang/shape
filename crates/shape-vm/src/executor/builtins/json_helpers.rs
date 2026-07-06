//! WF-2E (2026-07-05): JSON navigation helper bodies.
//!
//! Implements the five `Json*` `BuiltinFunction` bodies backing the
//! `std::core::json_value` navigation methods (`get`, `at`, `keys`, `len`).
//! Each reads a `Json` enum payload carrier via its STAMPED `NativeKind`
//! (ADR-006 §2.7.7 parallel-kind track — the single discriminator):
//!
//! - a `Json::Object` payload is a `Ptr(HeapKind::HashMap)` whose values are
//!   `Json` enum `TypedObject` nodes (`HashMapKindedRef::TypedObject`);
//! - a `Json::Array` payload is a `Ptr(HeapKind::TypedArray)` of
//!   `*const TypedObjectStorage` `Json` nodes (element-type stamp
//!   `ELEM_TYPE_TYPED_OBJECT`).
//!
//! Both carriers are produced by the parse-side projector
//! (`vm_impl/modules.rs::project_json_value_to_slot`). No blind
//! `as_heap_value()` reinterpretation: each carrier is read through the
//! concrete pointer shape its stamped kind names, exactly as the marshal
//! reader (`shape_runtime::json_value::slot_to_json_value`) does.
//!
//! Ownership: the `args` slice is BORROWED only — the caller's
//! `pop_builtin_args()` `Vec<KindedSlot>` retires each arg's share on drop.
//! Every returned `KindedSlot` carries a fresh, independently-owned share
//! (bumped via `get_share` / `v2_retain`, or a freshly-allocated node).
//!
//! Pre-WF-2E these five arms surfaced `VMError::NotImplemented`
//! ("phase-1b-vm-wave-5e-json-nav …"), which is why every navigation call
//! on a parsed `Json` value failed.

use super::super::VirtualMachine;
use shape_value::heap_value::{HashMapKindedRef, HeapKind, TypedObjectStorage};
use shape_value::v2::string_obj::StringObj;
use shape_value::v2::typed_array::{
    ELEM_TYPE_STRING, ELEM_TYPE_TYPED_OBJECT, TypedArray, read_elem_type, stamp_elem_type,
};
use shape_value::{KindedSlot, NativeKind, VMError, ValueSlot};
use std::sync::Arc;

// Json enum variant IDs (must match `stdlib-src/core/json_value.shape` +
// `vm_impl/modules.rs`). Only `Null` is needed here (the miss / non-object
// / non-array fallback result).
const JSON_VARIANT_NULL: i64 = 0;

impl VirtualMachine {
    /// Build a fresh `Json::Null` enum value as an owned
    /// `Ptr(HeapKind::TypedObject)` `KindedSlot`.
    ///
    /// Requires the registered `Json` enum schema (loaded via
    /// `std::core::json_value`); navigation only reaches this code on a
    /// value that is already a `Json`, so the schema is always present. The
    /// two slots (`__variant = Int64`, `__payload_0 = Null`) carry no heap
    /// share, so `heap_mask = 0` and the `field_kinds` track is
    /// `[Int64, Null]` — matching the parse-side `build_json_enum_slot`.
    fn build_json_null_slot(&self) -> Result<KindedSlot, VMError> {
        let json_schema_id = self
            .lookup_schema_by_name("Json")
            .map(|s| s.id as u64)
            .ok_or_else(|| {
                VMError::RuntimeError(
                    "json navigation: `Json` enum schema not registered \
                     (load std::core::json_value)"
                        .to_string(),
                )
            })?;
        let slots =
            vec![ValueSlot::from_int(JSON_VARIANT_NULL), ValueSlot::none()].into_boxed_slice();
        let field_kinds: Arc<[NativeKind]> =
            Arc::from(vec![NativeKind::Int64, NativeKind::Null].into_boxed_slice());
        let ptr = TypedObjectStorage::_new(json_schema_id, slots, 0, field_kinds);
        Ok(KindedSlot::from_typed_object_raw(ptr))
    }

    /// `__json_object_get(obj, key) -> Json`
    ///
    /// `obj` is the `Json::Object` payload (`Ptr(HeapKind::HashMap)`),
    /// `key` a `string`. Returns the stored `Json` node for `key`, or
    /// `Json::Null` for a missing key / non-object receiver.
    pub(in crate::executor) fn builtin_json_object_get(
        &self,
        args: &[KindedSlot],
    ) -> Result<KindedSlot, VMError> {
        let obj = args.first().ok_or_else(|| {
            VMError::RuntimeError("__json_object_get: missing object argument".to_string())
        })?;
        let key = args.get(1).and_then(|s| s.as_str()).ok_or_else(|| {
            VMError::RuntimeError("__json_object_get: key argument is not a string".to_string())
        })?;

        let bits = obj.raw();
        if obj.kind() != NativeKind::Ptr(HeapKind::HashMap) || bits == 0 {
            return self.build_json_null_slot();
        }
        // SAFETY: the slot's stamped kind is `Ptr(HeapKind::HashMap)`, so
        // `bits` is `Arc::into_raw(Arc<HashMapKindedRef>)`. Borrow only —
        // the arg's share is retired by the caller's args-Vec drop.
        let kref: &HashMapKindedRef = unsafe { &*(bits as *const HashMapKindedRef) };
        match kref {
            HashMapKindedRef::TypedObject(arc) => {
                // `get_share` bumps the element's refcount (v2_retain via
                // `TypedObjectPtr::share_clone`) and returns an owned
                // `TypedObjectPtr`; `into_raw` transfers that share onto the
                // returned slot.
                match arc.get_share(key) {
                    Some(ptr) => Ok(KindedSlot::from_typed_object_raw(ptr.into_raw())),
                    None => self.build_json_null_slot(),
                }
            }
            // A `Json::Object` whose value carrier is not a `Json` node is
            // only reachable via a hand-built (non-parse) HashMap; navigation
            // returns `Json::Null` rather than misinterpret a non-Json value.
            _ => self.build_json_null_slot(),
        }
    }

    /// `__json_array_at(arr, index) -> Json`
    ///
    /// `arr` is the `Json::Array` payload (`Ptr(HeapKind::TypedArray)` of
    /// `Json` nodes), `index` a `number`. Returns the element `Json` node,
    /// or `Json::Null` for a non-array receiver / out-of-range index.
    pub(in crate::executor) fn builtin_json_array_at(
        &self,
        args: &[KindedSlot],
    ) -> Result<KindedSlot, VMError> {
        let arr = args.first().ok_or_else(|| {
            VMError::RuntimeError("__json_array_at: missing array argument".to_string())
        })?;
        let index_slot = args.get(1).ok_or_else(|| {
            VMError::RuntimeError("__json_array_at: missing index argument".to_string())
        })?;

        // The declared `at(index: number)` param arrives Float64; accept the
        // integer family too. A negative index is out of range → Json::Null.
        let idx_i64: i64 = match index_slot.kind() {
            NativeKind::Float64 => f64::from_bits(index_slot.raw()) as i64,
            k if k.is_integer_family() => index_slot.raw() as i64,
            _ => return self.build_json_null_slot(),
        };
        if idx_i64 < 0 {
            return self.build_json_null_slot();
        }
        let idx = idx_i64 as u32;

        let bits = arr.raw();
        if arr.kind() != NativeKind::Ptr(HeapKind::TypedArray) || bits == 0 {
            return self.build_json_null_slot();
        }
        let base = bits as *const u8;
        // SAFETY: stamped kind `Ptr(HeapKind::TypedArray)` ⇒ `base` is a live
        // `*mut TypedArray<T>`; the element-type stamp selects `T`. Json
        // arrays are always TypedObject-elemented (parse-side stamp).
        let elem_type = unsafe { read_elem_type(base) };
        if elem_type != ELEM_TYPE_TYPED_OBJECT {
            return self.build_json_null_slot();
        }
        let ta = base as *const TypedArray<*const TypedObjectStorage>;
        let len = unsafe { TypedArray::<*const TypedObjectStorage>::len(ta) };
        if idx >= len {
            return self.build_json_null_slot();
        }
        // SAFETY: `idx < len`. The element is a borrowed
        // `*const TypedObjectStorage`; bump its refcount so the returned slot
        // owns an independent share (mirrors `TypedObjectPtr::clone`).
        let elem = unsafe { TypedArray::<*const TypedObjectStorage>::get_unchecked(ta, idx) };
        if elem.is_null() {
            return self.build_json_null_slot();
        }
        unsafe { shape_value::v2::refcount::v2_retain(&(*elem).header) };
        Ok(KindedSlot::from_typed_object_raw(elem))
    }

    /// `__json_object_keys(obj) -> Array<string>`
    ///
    /// Returns the object's keys as a fresh `TypedArray<*const StringObj>`
    /// (`Ptr(HeapKind::TypedArray)`, stamp `ELEM_TYPE_STRING`). Each key is a
    /// freshly-allocated `StringObj` owned by the result array. A non-object
    /// receiver yields an empty array.
    pub(in crate::executor) fn builtin_json_object_keys(
        &self,
        args: &[KindedSlot],
    ) -> Result<KindedSlot, VMError> {
        let obj = args.first().ok_or_else(|| {
            VMError::RuntimeError("__json_object_keys: missing object argument".to_string())
        })?;
        let bits = obj.raw();

        let out = TypedArray::<*const StringObj>::with_capacity(0);
        // SAFETY: `out` is a freshly-allocated `TypedArray` with a live
        // HeapHeader; stamp the element-type discriminant so the reader /
        // drop path pick the `*const StringObj` monomorphization.
        unsafe { stamp_elem_type(out as *mut u8, ELEM_TYPE_STRING) };

        if obj.kind() == NativeKind::Ptr(HeapKind::HashMap) && bits != 0 {
            // SAFETY: stamped kind `Ptr(HeapKind::HashMap)` ⇒ `bits` is
            // `Arc::into_raw(Arc<HashMapKindedRef>)`. Borrow only.
            let kref: &HashMapKindedRef = unsafe { &*(bits as *const HashMapKindedRef) };
            if let HashMapKindedRef::TypedObject(arc) = kref {
                let keys_ptr = arc.keys;
                let n = unsafe { TypedArray::<*const StringObj>::len(keys_ptr) };
                for i in 0..n {
                    // SAFETY: `i < n`; `keys` buffer holds live StringObjs.
                    let kp =
                        unsafe { TypedArray::<*const StringObj>::get_unchecked(keys_ptr, i) };
                    let s = unsafe { StringObj::as_str(kp) };
                    // Fresh StringObj (refcount = 1) owned by the result array.
                    let fresh = StringObj::new(s) as *const StringObj;
                    // SAFETY: `out` is a live `TypedArray<*const StringObj>`;
                    // `fresh` is a valid owned element (one share transferred).
                    unsafe { TypedArray::<*const StringObj>::push(out, fresh) };
                }
            }
        }
        Ok(KindedSlot::new(
            ValueSlot::from_raw(out as u64),
            NativeKind::Ptr(HeapKind::TypedArray),
        ))
    }

    /// `__json_array_len(arr) -> number`
    ///
    /// Length of the `Json::Array` payload, or `0` for a non-array receiver.
    pub(in crate::executor) fn builtin_json_array_len(
        &self,
        args: &[KindedSlot],
    ) -> Result<KindedSlot, VMError> {
        let arr = args.first().ok_or_else(|| {
            VMError::RuntimeError("__json_array_len: missing array argument".to_string())
        })?;
        let bits = arr.raw();
        let len = if arr.kind() == NativeKind::Ptr(HeapKind::TypedArray) && bits != 0 {
            // `TypedArray::len` reads the `len` field at a T-independent
            // offset (16); any element-type parameterization is sound here.
            unsafe {
                TypedArray::<*const TypedObjectStorage>::len(
                    bits as *const TypedArray<*const TypedObjectStorage>,
                ) as usize
            }
        } else {
            0
        };
        Ok(KindedSlot::from_number(len as f64))
    }

    /// `__json_object_len(obj) -> number`
    ///
    /// Entry count of the `Json::Object` payload, or `0` for a non-object
    /// receiver.
    pub(in crate::executor) fn builtin_json_object_len(
        &self,
        args: &[KindedSlot],
    ) -> Result<KindedSlot, VMError> {
        let obj = args.first().ok_or_else(|| {
            VMError::RuntimeError("__json_object_len: missing object argument".to_string())
        })?;
        let bits = obj.raw();
        let len = if obj.kind() == NativeKind::Ptr(HeapKind::HashMap) && bits != 0 {
            // SAFETY: stamped kind `Ptr(HeapKind::HashMap)` ⇒ `bits` is
            // `Arc::into_raw(Arc<HashMapKindedRef>)`. Borrow only. `len()`
            // is V-agnostic (keys-buffer length).
            let kref: &HashMapKindedRef = unsafe { &*(bits as *const HashMapKindedRef) };
            kref.len()
        } else {
            0
        };
        Ok(KindedSlot::from_number(len as f64))
    }
}
