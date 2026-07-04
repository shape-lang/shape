//! v2 typed FFI functions for JIT-compiled code.
//!
//! These functions use native types (f64, i64, i32, raw pointers) instead of
//! NaN-boxed u64 values. They are called from JIT-compiled v2 code via direct
//! extern "C" calls.

#![allow(clippy::approx_constant)] // arbitrary test floats; not math constants
pub mod collection_arc;
pub mod typed_map;

// jit_v2_map_* re-exports removed — see typed_map.rs SURFACE comment.
// The kind-blind ValueWord-shape map FFI is gone; the strict-typing
// rebuild routes through `Arc<HashMapData>` + `KindedSlot` per ADR-006
// §2.7.5 / §2.7.6 / Q8.
//
// W12-jit-collection-arc-ffi-ctors-and-refcount (Phase 3 cluster-0
// Round 9 / 8B.1, 2026-05-13): 8 typed-Arc collection ctors + 16
// per-HeapKind kinded retain/release entries live in `collection_arc`.
// Carrier shape is `Arc::into_raw(Arc<XData>) as u64` per audit §5,
// distinct from the W11 `UnifiedValue<T>` HeapHeader-style allocations
// in the rest of this module. Round 10 (8B.2) wires the MIR EnumStore
// consumer + `jit_call_method` shell to dispatch through these
// entry-points; until then they are inert at the program surface.

use shape_value::heap_value::{TraitObjectStorage, TypedObjectStorage};
use shape_value::v2::decimal_obj::DecimalObj;
use shape_value::v2::heap_header::HeapHeader;
use shape_value::v2::string_obj::StringObj;
use shape_value::v2::typed_array::TypedArray;

// W11-fup-C (Phase 3d, 2026-05-18): stamp the v2-raw element-type byte
// at `HeapHeader._pad` (offset 7) on every JIT-side TypedArray allocator
// — mirror of the VM-side `op_new_typed_array_*` handlers at
// `crates/shape-vm/src/executor/v2_handlers/array.rs:40-81` which call
// `stamp_elem_type(ptr, ELEM_TYPE_<KIND>)` immediately after `with_capacity`.
// Without the stamp, the canonical `as_v2_typed_array` detection at
// `crates/shape-vm/src/executor/v2_handlers/v2_array_detect.rs:181-216`
// returns `None` (reads `_pad = 0 = ELEM_TYPE_UNKNOWN`) — the JIT-allocated
// array's print path, method-dispatch path, and any other consumer of the
// canonical v2-detect interface would all silently fall back to non-typed
// rendering (the empirical pre-fix `print(arr)` output was the raw pointer
// `100218864737360` printed as a u64 scalar).
//
// Per ADR-006 §2.7.5 stamp-at-compile-time: the element kind is statically
// proven at the JIT codegen site (`mir_compiler/v2_array.rs:166-176`
// dispatch table picks the matching `jit_v2_array_new_<kind>` FuncRef per
// the producing slot's `NativeKind`), so the stamp byte authoritatively
// records compile-time-proven element kind into the v2-raw allocation's
// heap-header at construction time — the same compile-time-proof shape
// the VM-side `op_new_typed_array_*` handlers use.
use shape_vm::executor::v2_handlers::v2_array_detect::{
    ELEM_TYPE_BOOL, ELEM_TYPE_CHAR, ELEM_TYPE_DECIMAL, ELEM_TYPE_F32, ELEM_TYPE_F64, ELEM_TYPE_I8,
    ELEM_TYPE_I16, ELEM_TYPE_I32, ELEM_TYPE_I64, ELEM_TYPE_STRING, ELEM_TYPE_TRAIT_OBJECT,
    ELEM_TYPE_TYPED_OBJECT, ELEM_TYPE_U8, ELEM_TYPE_U16, ELEM_TYPE_U32, stamp_elem_type,
};

// ============================================================================
// W16.2-J.3 (2026-05-22): macro-generated per-kind TypedArray FFI symbols
// ============================================================================
//
// Mirror of the VM-side W16.2-J.2 opcode macro-gen for the per-kind JIT FFI
// surface. Generates `jit_v2_array_new_<suffix>` / `jit_v2_array_get_<suffix>`
// / `jit_v2_array_set_<suffix>` per `(suffix, native_ty, ELEM_TYPE_<KIND>)`
// triple. 14 `TypedArrayKind` variants covered: F64, I64, I32, Bool, I8, U8,
// I16, U16, U32, F32, Char, String, Decimal, TypedObject.
//
// Each allocator stamps the element-kind byte at `HeapHeader._pad` (offset 7)
// per ADR-006 §2.7.5 producer-side stamp + W11-fup-C 2026-05-18 stamp
// discipline (`crates/shape-vm/src/executor/v2_handlers/array.rs:40-81`
// matching VM-side `op_new_typed_array_*` shape). The element kind is
// statically proven at the JIT codegen site that picks the FuncRef, so the
// stamp byte authoritatively records the compile-time-proven kind into the
// v2-raw allocation at construction time — zero-tag runtime preserved.
//
// `Bool` uses `u8` storage to match the VM-side `TypedArray<u8>` carrier
// (`crates/shape-vm/src/executor/v2_handlers/v2_array_detect.rs:1476-1490`).
// `Char` uses `char` storage (4-byte `Copy`) matching the VM; the get/set
// FFI signatures use `u32` for C-ABI-safety with `char::from_u32_unchecked`
// at the boundary (the kind is statically proven so the bits are always a
// valid Unicode scalar). `String` / `Decimal` / `TypedObject` use the
// `*const T` heap-element carrier per ADR-006 §2.7.24 Q25.A SUPERSEDED +
// audit `v0.3-w16-2-j-audit.md` §1.D; the FFI signatures use `*const u8` at
// the boundary to match the existing `jit_v2_field_load_ptr` carrier
// convention. Per-element refcount discipline is the caller's responsibility
// per the VM-side `NewStringV2` / `TypedArrayPushString` / `_Decimal` /
// `_TypedObject` transfer convention.
//
// The 3 heap-element kinds retain the legacy `jit_new_typed_array_<kind>`
// allocator names (consumer-wired via `compiler/ffi_builder.rs:232-235`);
// the macro additionally generates `jit_v2_array_new_<kind>` aliases that
// share the same body, keeping the symbol surface uniform for the macro-
// generated `get`/`set` entries.

/// Generate `new`, `get`, `set` FFI symbols for one `TypedArrayKind` variant
/// whose elements are POD scalar (`f64`, `i64`, `i32`, etc.).
///
/// The macro takes the fully-spelled symbol names so the expansion matches
/// the existing `jit_v2_array_<op>_<suffix>` convention referenced from
/// `crates/shape-jit/src/compiler/ffi_builder.rs` and
/// `crates/shape-jit/src/ffi_symbols/v2_symbols.rs`.
macro_rules! v2_typed_array_scalar {
    (
        new = $new_fn:ident,
        get = $get_fn:ident,
        set = $set_fn:ident,
        ty = $native_ty:ty,
        elem_byte = $elem_byte:expr,
        kind_label = $kind_label:literal $(,)?
    ) => {
        #[doc = concat!(
            "Allocate an empty `TypedArray<", stringify!($native_ty), ">` of given capacity. ",
            "Stamps `", $kind_label, "` at `HeapHeader._pad` offset 7 per ADR-006 §2.7.5."
        )]
        #[unsafe(no_mangle)]
        pub extern "C" fn $new_fn(capacity: u32) -> *mut TypedArray<$native_ty> {
            let ptr = TypedArray::<$native_ty>::with_capacity(capacity);
            // SAFETY: `with_capacity` returned a freshly-allocated TypedArray
            // with a valid HeapHeader at offset 0; stamping the `_pad` byte
            // at offset 7 is the documented W11-fup-C producer-side stamp.
            unsafe { stamp_elem_type(ptr as *mut u8, $elem_byte) };
            ptr
        }

        #[doc = concat!(
            "Read the element at `index` from a `TypedArray<", stringify!($native_ty), ">`. ",
            "Panics on out-of-bounds."
        )]
        #[unsafe(no_mangle)]
        pub extern "C" fn $get_fn(
            arr: *const TypedArray<$native_ty>,
            index: i64,
        ) -> $native_ty {
            unsafe {
                if index < 0 || index as u32 >= (*arr).len {
                    panic!(
                        concat!(
                            "v2 array ", $kind_label, " index {} out of bounds (len {})"
                        ),
                        index,
                        (*arr).len
                    );
                }
                TypedArray::get_unchecked(arr, index as u32)
            }
        }

        #[doc = concat!(
            "Write `val` to the element at `index` in a `TypedArray<",
            stringify!($native_ty), ">`."
        )]
        #[unsafe(no_mangle)]
        pub extern "C" fn $set_fn(
            arr: *mut TypedArray<$native_ty>,
            index: i64,
            val: $native_ty,
        ) {
            unsafe {
                TypedArray::set(arr, index as u32, val);
            }
        }
    };
}

// ── 11 POD-element kinds (f64 / i64 / i32 / bool / i8 / u8 / i16 / u16 /
//    u32 / f32 / char) ──────────────────────────────────────────────────────

v2_typed_array_scalar! {
    new = jit_v2_array_new_f64,
    get = jit_v2_array_get_f64,
    set = jit_v2_array_set_f64,
    ty = f64,
    elem_byte = ELEM_TYPE_F64,
    kind_label = "f64",
}

v2_typed_array_scalar! {
    new = jit_v2_array_new_i64,
    get = jit_v2_array_get_i64,
    set = jit_v2_array_set_i64,
    ty = i64,
    elem_byte = ELEM_TYPE_I64,
    kind_label = "i64",
}

v2_typed_array_scalar! {
    new = jit_v2_array_new_i32,
    get = jit_v2_array_get_i32,
    set = jit_v2_array_set_i32,
    ty = i32,
    elem_byte = ELEM_TYPE_I32,
    kind_label = "i32",
}

// `Bool` uses `u8` storage per the VM-side `TypedArray<u8>` carrier
// (1 byte / element, 0 = false, 1 = true). Same shape as the dedicated
// `U8` kind below; distinguished at the runtime element-type-tag layer
// (`ELEM_TYPE_BOOL` vs `ELEM_TYPE_U8`).
v2_typed_array_scalar! {
    new = jit_v2_array_new_bool,
    get = jit_v2_array_get_bool,
    set = jit_v2_array_set_bool,
    ty = u8,
    elem_byte = ELEM_TYPE_BOOL,
    kind_label = "bool",
}

// W12 S1 (2026-05-13) — sized-integer monomorphizations for `TypedArray<i8>` /
// `<u8>` / `<i16>` / `<u16>` / `<u32>`. `U64` deliberately omitted per the
// `v2_array_detect.rs::V2ElemType` doc (deferred to S1.5 per the
// `NativeKind::UInt64` carrier-disambiguation ruling).
v2_typed_array_scalar! {
    new = jit_v2_array_new_i8,
    get = jit_v2_array_get_i8,
    set = jit_v2_array_set_i8,
    ty = i8,
    elem_byte = ELEM_TYPE_I8,
    kind_label = "i8",
}

v2_typed_array_scalar! {
    new = jit_v2_array_new_u8,
    get = jit_v2_array_get_u8,
    set = jit_v2_array_set_u8,
    ty = u8,
    elem_byte = ELEM_TYPE_U8,
    kind_label = "u8",
}

v2_typed_array_scalar! {
    new = jit_v2_array_new_i16,
    get = jit_v2_array_get_i16,
    set = jit_v2_array_set_i16,
    ty = i16,
    elem_byte = ELEM_TYPE_I16,
    kind_label = "i16",
}

v2_typed_array_scalar! {
    new = jit_v2_array_new_u16,
    get = jit_v2_array_get_u16,
    set = jit_v2_array_set_u16,
    ty = u16,
    elem_byte = ELEM_TYPE_U16,
    kind_label = "u16",
}

v2_typed_array_scalar! {
    new = jit_v2_array_new_u32,
    get = jit_v2_array_get_u32,
    set = jit_v2_array_set_u32,
    ty = u32,
    elem_byte = ELEM_TYPE_U32,
    kind_label = "u32",
}

// Wave 2 Agent A1 (2026-05-14) — `F32` scalar bucket.
v2_typed_array_scalar! {
    new = jit_v2_array_new_f32,
    get = jit_v2_array_get_f32,
    set = jit_v2_array_set_f32,
    ty = f32,
    elem_byte = ELEM_TYPE_F32,
    kind_label = "f32",
}

// `Char` storage is `TypedArray<char>` (4-byte `Copy`) matching the VM-side
// carrier at `crates/shape-vm/src/executor/v2_handlers/v2_array_detect.rs:1587`.
// `char` is not C-ABI-safe as a scalar parameter/return per the
// `improper_ctypes_definitions` lint, so the FFI signatures use `u32`
// codepoint at the boundary with `char::from_u32_unchecked` /
// `<char as Into<u32>>` conversion. The kind is statically proven at the JIT
// codegen site, so the bits are always a valid Unicode scalar value (per
// the matching VM-side compile-time-proof at the producing
// `NewTypedArrayChar` opcode).

#[doc = "Allocate an empty `TypedArray<char>` of given capacity. \
Stamps `ELEM_TYPE_CHAR` at `HeapHeader._pad` offset 7 per ADR-006 §2.7.5."]
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_new_char(capacity: u32) -> *mut TypedArray<char> {
    let ptr = TypedArray::<char>::with_capacity(capacity);
    unsafe { stamp_elem_type(ptr as *mut u8, ELEM_TYPE_CHAR) };
    ptr
}

#[doc = "Read the codepoint at `index` from a `TypedArray<char>`. Panics on \
out-of-bounds."]
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_get_char(arr: *const TypedArray<char>, index: i64) -> u32 {
    unsafe {
        if index < 0 || index as u32 >= (*arr).len {
            panic!(
                "v2 array char index {} out of bounds (len {})",
                index,
                (*arr).len
            );
        }
        TypedArray::<char>::get_unchecked(arr, index as u32) as u32
    }
}

#[doc = "Write a Unicode codepoint to the element at `index` in a \
`TypedArray<char>`. The caller must ensure `val` is a valid Unicode scalar \
value (the JIT codegen site proves this at compile time)."]
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_set_char(arr: *mut TypedArray<char>, index: i64, val: u32) {
    // SAFETY: the JIT codegen site that emits this call has compile-time-proven
    // the producing slot's NativeKind::Char, so `val` is always a valid
    // Unicode scalar value (matching the VM-side `NewTypedArrayChar` +
    // `TypedArraySetChar` producer compile-time proof).
    let c = unsafe { char::from_u32_unchecked(val) };
    unsafe {
        TypedArray::set(arr, index as u32, c);
    }
}

// ── 3 heap-element kinds (`*const StringObj` / `*const DecimalObj` /
//    `*const TypedObjectStorage`) ───────────────────────────────────────────
//
// Element type is `*const T` where `T: HeapElement`, not a flat scalar. The
// allocators mirror the scalar shape (`TypedArray::with_capacity` returning a
// `*mut TypedArray<*const T>` carrier); per-element refcount discipline is
// the caller's responsibility (the bytecode emits `TypedArrayPushString` /
// `TypedArrayPushDecimal` / `TypedArrayPushTypedObject` consuming the
// per-element share allocated by the corresponding `NewStringV2` /
// `NewDecimalV2` / `op_new_typed_object` site — matches the VM-side handler
// at `crates/shape-vm/src/executor/v2_handlers/array.rs:803-858`).
//
// Two allocator names per heap kind: the legacy `jit_new_typed_array_<kind>`
// (consumer-wired via `crates/shape-jit/src/compiler/ffi_builder.rs:232-235`)
// AND the macro-uniform `jit_v2_array_new_<kind>` (forwarder for code paths
// that walk the per-kind symbol matrix). Bodies are identical.
//
// `get` / `set` FFI signatures use `*const u8` for the pointer payload to
// match the existing `jit_v2_field_load_ptr` / `_store_ptr` carrier
// convention. Callers transfer per-element refcount shares per the VM-side
// transfer rules.

/// Generate `get` / `set` FFI symbols for one heap-element `TypedArrayKind`
/// variant. The `new` allocator is defined separately to keep the legacy
/// `jit_new_typed_array_<kind>` name + add the macro-uniform alias.
macro_rules! v2_typed_array_heap_get_set {
    (
        get = $get_fn:ident,
        set = $set_fn:ident,
        elem_obj_ty = $elem_obj_ty:ty,
        kind_label = $kind_label:literal $(,)?
    ) => {
        #[doc = concat!(
                            "Read the per-element heap pointer at `index` from a ",
                            "`TypedArray<*const ", stringify!($elem_obj_ty), ">`. ",
                            "Panics on out-of-bounds. Caller is responsible for per-element ",
                            "retain discipline before transferring the share."
                        )]
        #[unsafe(no_mangle)]
        pub extern "C" fn $get_fn(
            arr: *const TypedArray<*const $elem_obj_ty>,
            index: i64,
        ) -> *const u8 {
            unsafe {
                if index < 0 || index as u32 >= (*arr).len {
                    panic!(
                        concat!("v2 array ", $kind_label, " index {} out of bounds (len {})"),
                        index,
                        (*arr).len
                    );
                }
                TypedArray::<*const $elem_obj_ty>::get_unchecked(arr, index as u32) as *const u8
            }
        }

        #[doc = concat!(
                            "Write a per-element heap pointer at `index` in a ",
                            "`TypedArray<*const ", stringify!($elem_obj_ty), ">`. The caller ",
                            "must have an owning refcount share for `val` and is responsible ",
                            "for releasing any previous element at `index` before this call."
                        )]
        #[unsafe(no_mangle)]
        pub extern "C" fn $set_fn(
            arr: *mut TypedArray<*const $elem_obj_ty>,
            index: i64,
            val: *const u8,
        ) {
            unsafe {
                TypedArray::set(arr, index as u32, val as *const $elem_obj_ty);
            }
        }
    };
}

#[doc = "Allocate an empty `TypedArray<*const StringObj>` of given capacity. \
Stamps `ELEM_TYPE_STRING` at `HeapHeader._pad` offset 7 per ADR-006 §2.7.5. \
ckpt-6-prime Group X JIT FFI String/Decimal BUILD (2026-05-15)."]
#[unsafe(no_mangle)]
pub extern "C" fn jit_new_typed_array_string(capacity: u32) -> *mut TypedArray<*const StringObj> {
    let ptr = TypedArray::<*const StringObj>::with_capacity(capacity);
    unsafe { stamp_elem_type(ptr as *mut u8, ELEM_TYPE_STRING) };
    ptr
}

/// Macro-uniform alias for `jit_new_typed_array_string`. Same body; exists so
/// the per-kind symbol matrix (`jit_v2_array_new_<suffix>`) is complete for
/// all 14 `TypedArrayKind` variants. Consumer-wired alias to the legacy name.
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_new_string(capacity: u32) -> *mut TypedArray<*const StringObj> {
    jit_new_typed_array_string(capacity)
}

v2_typed_array_heap_get_set! {
    get = jit_v2_array_get_string,
    set = jit_v2_array_set_string,
    elem_obj_ty = StringObj,
    kind_label = "string",
}

#[doc = "Allocate an empty `TypedArray<*const DecimalObj>` of given capacity. \
Stamps `ELEM_TYPE_DECIMAL` at `HeapHeader._pad` offset 7 per ADR-006 §2.7.5. \
ckpt-6-prime Group X JIT FFI String/Decimal BUILD (2026-05-15)."]
#[unsafe(no_mangle)]
pub extern "C" fn jit_new_typed_array_decimal(capacity: u32) -> *mut TypedArray<*const DecimalObj> {
    let ptr = TypedArray::<*const DecimalObj>::with_capacity(capacity);
    unsafe { stamp_elem_type(ptr as *mut u8, ELEM_TYPE_DECIMAL) };
    ptr
}

/// Macro-uniform alias for `jit_new_typed_array_decimal`. See
/// `jit_v2_array_new_string` for rationale.
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_new_decimal(capacity: u32) -> *mut TypedArray<*const DecimalObj> {
    jit_new_typed_array_decimal(capacity)
}

v2_typed_array_heap_get_set! {
    get = jit_v2_array_get_decimal,
    set = jit_v2_array_set_decimal,
    elem_obj_ty = DecimalObj,
    kind_label = "decimal",
}

/// Phase 4b Round 4 W16.2-A op_new_array-typed-object-element (2026-05-18).
///
/// JIT-side allocator for `TypedArray<*const TypedObjectStorage>` carriers.
/// Mirror of `jit_new_typed_array_string` / `_decimal` — allocates an empty
/// typed-array with given capacity, stamps the v2-raw element-type byte at
/// `HeapHeader._pad` (offset 7) per the W11-fup-C 2026-05-18 stamp-discipline
/// rule, returns the raw `*mut TypedArray<*const TypedObjectStorage>` carrier.
///
/// Per-element refcount discipline (caller's responsibility): the JIT-emitted
/// code allocates fresh `TypedObjectStorage` instances via
/// `TypedObjectStorage::_new` (refcount = 1) and transfers per-element share
/// to the array via the matching `TypedArrayPushTypedObject` opcode (or its
/// inline JIT analogue if/when wired). Mirror of the VM-side handler at
/// `crates/shape-vm/src/executor/v2_handlers/array.rs::NewTypedArrayTypedObject`.
#[unsafe(no_mangle)]
pub extern "C" fn jit_new_typed_array_typed_object(
    capacity: u32,
) -> *mut TypedArray<*const TypedObjectStorage> {
    let ptr = TypedArray::<*const TypedObjectStorage>::with_capacity(capacity);
    unsafe { stamp_elem_type(ptr as *mut u8, ELEM_TYPE_TYPED_OBJECT) };
    ptr
}

/// Macro-uniform alias for `jit_new_typed_array_typed_object`. See
/// `jit_v2_array_new_string` for rationale.
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_new_typed_object(
    capacity: u32,
) -> *mut TypedArray<*const TypedObjectStorage> {
    jit_new_typed_array_typed_object(capacity)
}

v2_typed_array_heap_get_set! {
    get = jit_v2_array_get_typed_object,
    set = jit_v2_array_set_typed_object,
    elem_obj_ty = TypedObjectStorage,
    kind_label = "typed_object",
}

/// Phase 4b W16.2-B op_new_array-trait-object-element (2026-06-05).
///
/// JIT-side allocator for `TypedArray<*const TraitObjectStorage>` carriers —
/// the `Array<dyn Trait>` backing per ADR-006 §2.7.5 + §2.7.24 Q25.C. Mirror
/// of `jit_new_typed_array_typed_object`, swapping `TypedObjectStorage` →
/// `TraitObjectStorage` and `ELEM_TYPE_TYPED_OBJECT` → `ELEM_TYPE_TRAIT_OBJECT`.
/// Element values are boxed at the producer site via `BoxTraitObject` (which
/// `_new`-allocates the `TraitObjectStorage`), then transferred per-element via
/// the matching `TypedArrayPushTraitObject` opcode.
#[unsafe(no_mangle)]
pub extern "C" fn jit_new_typed_array_trait_object(
    capacity: u32,
) -> *mut TypedArray<*const TraitObjectStorage> {
    let ptr = TypedArray::<*const TraitObjectStorage>::with_capacity(capacity);
    unsafe { stamp_elem_type(ptr as *mut u8, ELEM_TYPE_TRAIT_OBJECT) };
    ptr
}

/// Macro-uniform alias for `jit_new_typed_array_trait_object`. See
/// `jit_v2_array_new_string` for rationale.
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_new_trait_object(
    capacity: u32,
) -> *mut TypedArray<*const TraitObjectStorage> {
    jit_new_typed_array_trait_object(capacity)
}

v2_typed_array_heap_get_set! {
    get = jit_v2_array_get_trait_object,
    set = jit_v2_array_set_trait_object,
    elem_obj_ty = TraitObjectStorage,
    kind_label = "trait_object",
}

/// JIT-compile-time per-element materializer for `*const StringObj` constants
/// inside Array<string> literal aggregates. Allocates a fresh `StringObj`
/// (refcount = 1) and boosts the refcount by one for the constant's permanent
/// share — the JIT-emitted code pushes the returned pointer onto a freshly
/// allocated `TypedArray<*const StringObj>` and transfers the share to the
/// array's per-element slot (matches the VM-side `NewStringV2` +
/// `TypedArrayPushString` transfer convention at
/// `crates/shape-vm/src/executor/v2_handlers/array.rs:803-828`).
///
/// Mirrors `crate::ffi::string::arc_string_constant`'s permanent-share
/// discipline for the §2.7.5 `Arc<String>` carrier, but uses the v2-raw
/// `StringObj` carrier per ADR-006 §2.7.5 + §2.7.24 Q25.A SUPERSEDED + audit
/// deliverable (b) §4.1.B.
///
/// ckpt-6-prime Group X JIT FFI String/Decimal BUILD (2026-05-15) — per-
/// element NewStringV2 equivalent at the JIT mir_compiler dispatch site.
pub fn string_obj_constant(s: &str) -> *const StringObj {
    let ptr = StringObj::new(s);
    // SAFETY: ptr was just produced by StringObj::new with refcount=1.
    // Bump the refcount to 2 to retain the constant's permanent share.
    // The constant's "active share" is the one the JIT-emitted code
    // pushes onto the freshly allocated TypedArray; the permanent share
    // is the one that survives Drop chains across multiple invocations
    // of the JIT-compiled function. Same lifecycle as
    // `crate::ffi::string::arc_string_constant`.
    unsafe {
        (*ptr).header.retain();
    }
    ptr as *const StringObj
}

/// JIT-compile-time per-element materializer for `*const DecimalObj`
/// constants inside Array<decimal> literal aggregates. Same permanent-share
/// discipline as `string_obj_constant`. The `bits` argument is the
/// `Decimal::serialize()` byte-array packed into a `[u8; 16]` carrier —
/// `Decimal::deserialize(bits)` reconstructs the `Decimal` value at JIT
/// compile time, then `DecimalObj::new(value)` allocates a refcounted
/// `DecimalObj` heap object.
///
/// ckpt-6-prime Group X JIT FFI String/Decimal BUILD (2026-05-15) — per-
/// element NewDecimalV2 equivalent at the JIT mir_compiler dispatch site.
pub fn decimal_obj_constant(value: rust_decimal::Decimal) -> *const DecimalObj {
    let ptr = DecimalObj::new(value);
    // SAFETY: ptr was just produced by DecimalObj::new with refcount=1.
    // See `string_obj_constant` for the permanent-share rationale.
    unsafe {
        (*ptr).header.retain();
    }
    ptr as *const DecimalObj
}

/// Generic typed-array push dispatcher (R7.2).
///
/// Accepts an erased typed-array pointer, the element's bit pattern zero/sign-
/// extended to 64 bits, and the element's byte size. Dispatches to the
/// matching `TypedArray<T>::push` instantiation by `elem_size`.
///
/// `TypedArray<T>` has identical struct layout regardless of T (the `T` only
/// appears behind a `*mut T`), and `Layout::array::<T>` is identical for types
/// of the same size & alignment, so routing 8-byte pushes through
/// `TypedArray<i64>` is byte-equivalent to going through `TypedArray<f64>`.
///
/// # Safety
/// `arr` must point to a live `TypedArray<T>` whose element type has size
/// `elem_size` (1, 4, or 8). `bits` must contain the value to store in the
/// low `elem_size` bytes.
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_push(arr: *mut HeapHeader, bits: u64, elem_size: u8) {
    unsafe {
        match elem_size {
            1 => TypedArray::<u8>::push(arr as *mut TypedArray<u8>, bits as u8),
            4 => TypedArray::<i32>::push(arr as *mut TypedArray<i32>, bits as i32),
            8 => TypedArray::<i64>::push(arr as *mut TypedArray<i64>, bits as i64),
            _ => unreachable!("jit_v2_array_push: invalid elem_size {}", elem_size),
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_len_f64(arr: *const TypedArray<f64>) -> u32 {
    unsafe { TypedArray::len(arr) }
}

/// SIMD-accelerated sum over a `TypedArray<f64>` (Phase C.3).
///
/// Uses `wide::f64x4` for 4-lane parallel addition when `len >= 16`. Below
/// that threshold, the vector load/splat overhead exceeds the savings so we
/// fall back to scalar accumulation. Returns `0.0` for null or empty arrays.
///
/// # Safety
/// `arr` must be a valid `TypedArray<f64>*` (or null).
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_sum_f64(arr: *const TypedArray<f64>) -> f64 {
    if arr.is_null() {
        return 0.0;
    }
    let (data, len) = unsafe { ((*arr).data as *const f64, (*arr).len as usize) };
    if len == 0 || data.is_null() {
        return 0.0;
    }
    unsafe { simd_sum_f64_inner(data, len) }
}

/// SIMD-accelerated sum over a `TypedArray<i64>` (Phase C.3). Uses wrapping
/// arithmetic (matches Shape's v2 int-sum semantics — no overflow panic).
///
/// # Safety
/// `arr` must be a valid `TypedArray<i64>*` (or null).
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_sum_i64(arr: *const TypedArray<i64>) -> i64 {
    if arr.is_null() {
        return 0;
    }
    let (data, len) = unsafe { ((*arr).data as *const i64, (*arr).len as usize) };
    if len == 0 || data.is_null() {
        return 0;
    }
    unsafe { simd_sum_i64_inner(data, len) }
}

/// SIMD reduction threshold — below this, setup cost dominates.
const SIMD_SUM_THRESHOLD: usize = 16;

// ── Min / Max / Mean / Sum-of-squares over Array<number> ─────────────────

/// SIMD-accelerated minimum over a `TypedArray<f64>`. Returns `NaN` for null
/// or empty arrays (matches `Vec<number>.min()` semantics on empty input).
/// NaN propagates naturally — `fast_min(NaN, v)` yields `NaN` on all
/// compliant backends.
///
/// # Safety
/// `arr` must be a valid `TypedArray<f64>*` (or null).
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_min_f64(arr: *const TypedArray<f64>) -> f64 {
    if arr.is_null() {
        return f64::NAN;
    }
    let (data, len) = unsafe { ((*arr).data as *const f64, (*arr).len as usize) };
    if len == 0 || data.is_null() {
        return f64::NAN;
    }
    unsafe { simd_min_f64_inner(data, len) }
}

/// SIMD-accelerated maximum over a `TypedArray<f64>`. Returns `NaN` for null
/// or empty arrays. NaN propagates.
///
/// # Safety
/// `arr` must be a valid `TypedArray<f64>*` (or null).
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_max_f64(arr: *const TypedArray<f64>) -> f64 {
    if arr.is_null() {
        return f64::NAN;
    }
    let (data, len) = unsafe { ((*arr).data as *const f64, (*arr).len as usize) };
    if len == 0 || data.is_null() {
        return f64::NAN;
    }
    unsafe { simd_max_f64_inner(data, len) }
}

/// SIMD-accelerated mean (arithmetic average) over a `TypedArray<f64>`.
/// Returns `NaN` for null or empty arrays.
///
/// # Safety
/// `arr` must be a valid `TypedArray<f64>*` (or null).
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_mean_f64(arr: *const TypedArray<f64>) -> f64 {
    if arr.is_null() {
        return f64::NAN;
    }
    let (data, len) = unsafe { ((*arr).data as *const f64, (*arr).len as usize) };
    if len == 0 || data.is_null() {
        return f64::NAN;
    }
    let sum = unsafe { simd_sum_f64_inner(data, len) };
    sum / (len as f64)
}

/// SIMD-accelerated sum-of-squares over a `TypedArray<f64>` — `Σ x²`.
/// Single-pass: load, multiply, accumulate. Useful as a building block for
/// variance/std and for `arr.map(|x| x*x).sum()` patterns. Returns `0.0`
/// for null or empty arrays.
///
/// # Safety
/// `arr` must be a valid `TypedArray<f64>*` (or null).
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_sum_squares_f64(arr: *const TypedArray<f64>) -> f64 {
    if arr.is_null() {
        return 0.0;
    }
    let (data, len) = unsafe { ((*arr).data as *const f64, (*arr).len as usize) };
    if len == 0 || data.is_null() {
        return 0.0;
    }
    unsafe { simd_sum_squares_f64_inner(data, len) }
}

// ── Element-wise scalar ops returning a new Array<number> ────────────────

/// Allocate a new `TypedArray<f64>` with length equal to `arr.len`, populated
/// by multiplying each element by `factor`. Returns a null pointer when the
/// receiver is null.
///
/// # Safety
/// `arr` must be a valid `TypedArray<f64>*` (or null).
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_scale_f64(
    arr: *const TypedArray<f64>,
    factor: f64,
) -> *mut TypedArray<f64> {
    if arr.is_null() {
        return std::ptr::null_mut();
    }
    let (data, len) = unsafe { ((*arr).data as *const f64, (*arr).len as usize) };
    let out = TypedArray::<f64>::with_capacity(len as u32);
    if len == 0 || data.is_null() {
        return out;
    }
    unsafe {
        let out_data = (*out).data as *mut f64;
        simd_scale_f64_inner(data, out_data, len, factor);
        (*out).len = len as u32;
    }
    out
}

/// Allocate a new `TypedArray<f64>` with length equal to `arr.len`, populated
/// by adding `offset` to each element. Returns a null pointer when the
/// receiver is null.
///
/// # Safety
/// `arr` must be a valid `TypedArray<f64>*` (or null).
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_add_scalar_f64(
    arr: *const TypedArray<f64>,
    offset: f64,
) -> *mut TypedArray<f64> {
    if arr.is_null() {
        return std::ptr::null_mut();
    }
    let (data, len) = unsafe { ((*arr).data as *const f64, (*arr).len as usize) };
    let out = TypedArray::<f64>::with_capacity(len as u32);
    if len == 0 || data.is_null() {
        return out;
    }
    unsafe {
        let out_data = (*out).data as *mut f64;
        simd_add_scalar_f64_inner(data, out_data, len, offset);
        (*out).len = len as u32;
    }
    out
}

// ── Element-wise binary ops (two arrays) ─────────────────────────────────

/// Allocate a new `TypedArray<f64>` holding the element-wise sum of `a` and
/// `b`. Requires matching lengths — panics on mismatch to mirror Shape's
/// `dot()`/runtime length-mismatch semantics. Returns null for null inputs.
///
/// # Safety
/// `a` and `b` must be valid `TypedArray<f64>*` (or null).
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_add_f64(
    a: *const TypedArray<f64>,
    b: *const TypedArray<f64>,
) -> *mut TypedArray<f64> {
    if a.is_null() || b.is_null() {
        return std::ptr::null_mut();
    }
    let (a_data, a_len) = unsafe { ((*a).data as *const f64, (*a).len as usize) };
    let (b_data, b_len) = unsafe { ((*b).data as *const f64, (*b).len as usize) };
    if a_len != b_len {
        panic!("v2 array_add_f64: length mismatch ({} vs {})", a_len, b_len);
    }
    let out = TypedArray::<f64>::with_capacity(a_len as u32);
    if a_len == 0 {
        return out;
    }
    unsafe {
        let out_data = (*out).data as *mut f64;
        simd_binary_add_f64_inner(a_data, b_data, out_data, a_len);
        (*out).len = a_len as u32;
    }
    out
}

/// Allocate a new `TypedArray<f64>` holding the element-wise product of `a`
/// and `b`. Requires matching lengths.
///
/// # Safety
/// `a` and `b` must be valid `TypedArray<f64>*` (or null).
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_mul_f64(
    a: *const TypedArray<f64>,
    b: *const TypedArray<f64>,
) -> *mut TypedArray<f64> {
    if a.is_null() || b.is_null() {
        return std::ptr::null_mut();
    }
    let (a_data, a_len) = unsafe { ((*a).data as *const f64, (*a).len as usize) };
    let (b_data, b_len) = unsafe { ((*b).data as *const f64, (*b).len as usize) };
    if a_len != b_len {
        panic!("v2 array_mul_f64: length mismatch ({} vs {})", a_len, b_len);
    }
    let out = TypedArray::<f64>::with_capacity(a_len as u32);
    if a_len == 0 {
        return out;
    }
    unsafe {
        let out_data = (*out).data as *mut f64;
        simd_binary_mul_f64_inner(a_data, b_data, out_data, a_len);
        (*out).len = a_len as u32;
    }
    out
}

#[inline]
unsafe fn simd_sum_f64_inner(data: *const f64, len: usize) -> f64 {
    use wide::f64x4;
    if len < SIMD_SUM_THRESHOLD {
        let mut s = 0.0_f64;
        for i in 0..len {
            s += unsafe { *data.add(i) };
        }
        return s;
    }
    let chunks = len / 4;
    let mut acc = f64x4::splat(0.0);
    for i in 0..chunks {
        let b = i * 4;
        let v = unsafe {
            f64x4::from([
                *data.add(b),
                *data.add(b + 1),
                *data.add(b + 2),
                *data.add(b + 3),
            ])
        };
        acc += v;
    }
    let parts = acc.to_array();
    let mut s = parts[0] + parts[1] + parts[2] + parts[3];
    for i in (chunks * 4)..len {
        s += unsafe { *data.add(i) };
    }
    s
}

#[inline]
unsafe fn simd_sum_i64_inner(data: *const i64, len: usize) -> i64 {
    use wide::i64x4;
    if len < SIMD_SUM_THRESHOLD {
        let mut s: i64 = 0;
        for i in 0..len {
            s = s.wrapping_add(unsafe { *data.add(i) });
        }
        return s;
    }
    let chunks = len / 4;
    let mut acc = i64x4::splat(0);
    for i in 0..chunks {
        let b = i * 4;
        let v = unsafe {
            i64x4::from([
                *data.add(b),
                *data.add(b + 1),
                *data.add(b + 2),
                *data.add(b + 3),
            ])
        };
        // wide::i64x4 lacks AddAssign; rebind the accumulator.
        acc = acc + v;
    }
    let parts = acc.to_array();
    let mut s = parts[0]
        .wrapping_add(parts[1])
        .wrapping_add(parts[2])
        .wrapping_add(parts[3]);
    for i in (chunks * 4)..len {
        s = s.wrapping_add(unsafe { *data.add(i) });
    }
    s
}

/// Load four f64 values starting at `data[base]` into an `f64x4` lane.
///
/// # Safety
/// `data` must point to at least `base + 4` valid `f64` values.
#[inline]
unsafe fn load_f64x4(data: *const f64, base: usize) -> wide::f64x4 {
    unsafe {
        wide::f64x4::from([
            *data.add(base),
            *data.add(base + 1),
            *data.add(base + 2),
            *data.add(base + 3),
        ])
    }
}

/// Detect a NaN anywhere in a f64 buffer. Uses SIMD lanes via a
/// bitwise-equal comparison-against-self (NaN is the only value that is
/// not equal to itself under IEEE 754).
///
/// # Safety
/// `data` must point to at least `len` valid `f64` values.
#[inline]
unsafe fn contains_nan_f64(data: *const f64, len: usize) -> bool {
    if len < SIMD_SUM_THRESHOLD {
        for i in 0..len {
            if unsafe { *data.add(i) }.is_nan() {
                return true;
            }
        }
        return false;
    }
    let chunks = len / 4;
    for i in 0..chunks {
        let v = unsafe { load_f64x4(data, i * 4) };
        // NaN != NaN — a SIMD self-compare leaves NaN lanes as 0x0, other
        // lanes as all-ones. `wide` doesn't expose a portable movemask, so
        // we materialize the 4 lanes and scalar-check.
        let arr = v.to_array();
        if arr[0].is_nan() || arr[1].is_nan() || arr[2].is_nan() || arr[3].is_nan() {
            return true;
        }
    }
    for i in (chunks * 4)..len {
        if unsafe { *data.add(i) }.is_nan() {
            return true;
        }
    }
    false
}

#[inline]
unsafe fn simd_min_f64_inner(data: *const f64, len: usize) -> f64 {
    // Hardware `min_pd` does NOT reliably propagate NaN (it returns the
    // non-NaN operand in whichever slot based on the comparison order).
    // Do a cheap SIMD NaN scan first — if present, short-circuit to NaN
    // to match scalar `f64::min` semantics that our consumers expect.
    if unsafe { contains_nan_f64(data, len) } {
        return f64::NAN;
    }
    if len < SIMD_SUM_THRESHOLD {
        let mut m = unsafe { *data };
        for i in 1..len {
            let v = unsafe { *data.add(i) };
            if v < m {
                m = v;
            }
        }
        return m;
    }
    let chunks = len / 4;
    let mut acc = unsafe { load_f64x4(data, 0) };
    for i in 1..chunks {
        let v = unsafe { load_f64x4(data, i * 4) };
        acc = acc.fast_min(v);
    }
    let parts = acc.to_array();
    let mut m = parts[0];
    for &p in &parts[1..] {
        if p < m {
            m = p;
        }
    }
    for i in (chunks * 4)..len {
        let v = unsafe { *data.add(i) };
        if v < m {
            m = v;
        }
    }
    m
}

#[inline]
unsafe fn simd_max_f64_inner(data: *const f64, len: usize) -> f64 {
    if unsafe { contains_nan_f64(data, len) } {
        return f64::NAN;
    }
    if len < SIMD_SUM_THRESHOLD {
        let mut m = unsafe { *data };
        for i in 1..len {
            let v = unsafe { *data.add(i) };
            if v > m {
                m = v;
            }
        }
        return m;
    }
    let chunks = len / 4;
    let mut acc = unsafe { load_f64x4(data, 0) };
    for i in 1..chunks {
        let v = unsafe { load_f64x4(data, i * 4) };
        acc = acc.fast_max(v);
    }
    let parts = acc.to_array();
    let mut m = parts[0];
    for &p in &parts[1..] {
        if p > m {
            m = p;
        }
    }
    for i in (chunks * 4)..len {
        let v = unsafe { *data.add(i) };
        if v > m {
            m = v;
        }
    }
    m
}

#[inline]
unsafe fn simd_sum_squares_f64_inner(data: *const f64, len: usize) -> f64 {
    use wide::f64x4;
    if len < SIMD_SUM_THRESHOLD {
        let mut s = 0.0_f64;
        for i in 0..len {
            let v = unsafe { *data.add(i) };
            s += v * v;
        }
        return s;
    }
    let chunks = len / 4;
    let mut acc = f64x4::splat(0.0);
    for i in 0..chunks {
        let v = unsafe { load_f64x4(data, i * 4) };
        acc += v * v;
    }
    let parts = acc.to_array();
    let mut s = parts[0] + parts[1] + parts[2] + parts[3];
    for i in (chunks * 4)..len {
        let v = unsafe { *data.add(i) };
        s += v * v;
    }
    s
}

#[inline]
unsafe fn simd_scale_f64_inner(src: *const f64, dst: *mut f64, len: usize, factor: f64) {
    use wide::f64x4;
    if len < SIMD_SUM_THRESHOLD {
        for i in 0..len {
            unsafe { *dst.add(i) = *src.add(i) * factor };
        }
        return;
    }
    let chunks = len / 4;
    let splat = f64x4::splat(factor);
    for i in 0..chunks {
        let base = i * 4;
        let v = unsafe { load_f64x4(src, base) };
        let r = (v * splat).to_array();
        unsafe {
            *dst.add(base) = r[0];
            *dst.add(base + 1) = r[1];
            *dst.add(base + 2) = r[2];
            *dst.add(base + 3) = r[3];
        }
    }
    for i in (chunks * 4)..len {
        unsafe { *dst.add(i) = *src.add(i) * factor };
    }
}

#[inline]
unsafe fn simd_add_scalar_f64_inner(src: *const f64, dst: *mut f64, len: usize, offset: f64) {
    use wide::f64x4;
    if len < SIMD_SUM_THRESHOLD {
        for i in 0..len {
            unsafe { *dst.add(i) = *src.add(i) + offset };
        }
        return;
    }
    let chunks = len / 4;
    let splat = f64x4::splat(offset);
    for i in 0..chunks {
        let base = i * 4;
        let v = unsafe { load_f64x4(src, base) };
        let r = (v + splat).to_array();
        unsafe {
            *dst.add(base) = r[0];
            *dst.add(base + 1) = r[1];
            *dst.add(base + 2) = r[2];
            *dst.add(base + 3) = r[3];
        }
    }
    for i in (chunks * 4)..len {
        unsafe { *dst.add(i) = *src.add(i) + offset };
    }
}

#[inline]
unsafe fn simd_binary_add_f64_inner(a: *const f64, b: *const f64, dst: *mut f64, len: usize) {
    if len < SIMD_SUM_THRESHOLD {
        for i in 0..len {
            unsafe { *dst.add(i) = *a.add(i) + *b.add(i) };
        }
        return;
    }
    let chunks = len / 4;
    for i in 0..chunks {
        let base = i * 4;
        let va = unsafe { load_f64x4(a, base) };
        let vb = unsafe { load_f64x4(b, base) };
        let r = (va + vb).to_array();
        unsafe {
            *dst.add(base) = r[0];
            *dst.add(base + 1) = r[1];
            *dst.add(base + 2) = r[2];
            *dst.add(base + 3) = r[3];
        }
    }
    for i in (chunks * 4)..len {
        unsafe { *dst.add(i) = *a.add(i) + *b.add(i) };
    }
}

#[inline]
unsafe fn simd_binary_mul_f64_inner(a: *const f64, b: *const f64, dst: *mut f64, len: usize) {
    if len < SIMD_SUM_THRESHOLD {
        for i in 0..len {
            unsafe { *dst.add(i) = *a.add(i) * *b.add(i) };
        }
        return;
    }
    let chunks = len / 4;
    for i in 0..chunks {
        let base = i * 4;
        let va = unsafe { load_f64x4(a, base) };
        let vb = unsafe { load_f64x4(b, base) };
        let r = (va * vb).to_array();
        unsafe {
            *dst.add(base) = r[0];
            *dst.add(base + 1) = r[1];
            *dst.add(base + 2) = r[2];
            *dst.add(base + 3) = r[3];
        }
    }
    for i in (chunks * 4)..len {
        unsafe { *dst.add(i) = *a.add(i) * *b.add(i) };
    }
}

// ============================================================================
// Per-kind `len` accessors for the 3 non-f64 base kinds (i64 / i32 / bool).
// The `len_f64` companion is defined above (next to the f64 SIMD ops). `len`
// is not in the audit §1.D macro scope (new/get/set only) so these stay as
// hand-written entries until a follow-up extends the macro.
// ============================================================================

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_len_i64(arr: *const TypedArray<i64>) -> u32 {
    unsafe { TypedArray::len(arr) }
}

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_len_i32(arr: *const TypedArray<i32>) -> u32 {
    unsafe { TypedArray::len(arr) }
}

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_len_bool(arr: *const TypedArray<u8>) -> u32 {
    unsafe { TypedArray::len(arr) }
}

// ============================================================================
// Struct field access FFI
// ============================================================================

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_field_load_f64(ptr: *const u8, offset: u32) -> f64 {
    unsafe { (ptr.add(offset as usize) as *const f64).read_unaligned() }
}

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_field_load_i64(ptr: *const u8, offset: u32) -> i64 {
    unsafe { (ptr.add(offset as usize) as *const i64).read_unaligned() }
}

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_field_load_i32(ptr: *const u8, offset: u32) -> i32 {
    unsafe { (ptr.add(offset as usize) as *const i32).read_unaligned() }
}

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_field_load_ptr(ptr: *const u8, offset: u32) -> *const u8 {
    unsafe { (ptr.add(offset as usize) as *const *const u8).read_unaligned() }
}

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_field_store_f64(ptr: *mut u8, offset: u32, val: f64) {
    unsafe {
        (ptr.add(offset as usize) as *mut f64).write_unaligned(val);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_field_store_i64(ptr: *mut u8, offset: u32, val: i64) {
    unsafe {
        (ptr.add(offset as usize) as *mut i64).write_unaligned(val);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_field_store_i32(ptr: *mut u8, offset: u32, val: i32) {
    unsafe {
        (ptr.add(offset as usize) as *mut i32).write_unaligned(val);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_field_store_ptr(ptr: *mut u8, offset: u32, val: *const u8) {
    unsafe {
        (ptr.add(offset as usize) as *mut *const u8).write_unaligned(val);
    }
}

// ============================================================================
// Refcount FFI
// ============================================================================

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_retain(ptr: *const u8) {
    unsafe {
        let header = ptr as *const HeapHeader;
        (*header).retain();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_release(ptr: *const u8) {
    unsafe {
        let header = ptr as *const HeapHeader;
        if (*header).release() {
            // Refcount reached zero — deallocate.
            // For now, we only deallocate the struct itself.
            // Future: dispatch on kind for proper cleanup of nested resources.
            let kind = (*header).kind();
            let _ = kind; // TODO: dispatch cleanup based on kind
            std::alloc::dealloc(
                ptr as *mut u8,
                std::alloc::Layout::from_size_align(8, 8).unwrap(), // minimum — real size TBD
            );
        }
    }
}

// ── r5c-2-β-δ-(α): v2-raw `TypedArray<T>` retain / release ──────────────────
//
// JIT-side retain/release for a `NativeKind::Ptr(HeapKind::TypedArray)` slot.
// The carrier is the v2-raw `*mut TypedArray<T>` flat struct (24-byte
// `repr(C)`, HeapHeader at offset 0, separate element buffer). The generic
// `arc_retain` / `arc_release` are WRONG for this carrier — they assume a
// `UnifiedValue<T>` HeapHeader at offset +4 and a single-allocation layout,
// so `arc_release` deallocs the wrong size and leaks the element buffer
// (heap corruption). These two route through the same kind-blind helpers the
// VM uses (`retain_v2_typed_array` / `release_v2_typed_array` in
// `shape_value::v2::typed_array`), keeping VM and JIT on one carrier.

/// Retain (bump refcount of) a v2-raw `*mut TypedArray<T>` carrier.
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_typed_array_retain(ptr: *const u8) {
    if ptr.is_null() {
        return;
    }
    unsafe { shape_value::v2::typed_array::retain_v2_typed_array(ptr as *mut u8) };
}

/// Release one refcount share of a v2-raw `*mut TypedArray<T>` carrier;
/// on the last share, free via the stamped-element-type `drop_array` /
/// `drop_array_heap`.
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_typed_array_release(ptr: *const u8) {
    if ptr.is_null() {
        return;
    }
    unsafe { shape_value::v2::typed_array::release_v2_typed_array(ptr as *mut u8) };
}

// ============================================================================
// Struct allocation FFI
// ============================================================================

/// Allocate a v2 struct of the given total size (including header).
/// Initializes the HeapHeader with refcount=1 and the given kind.
/// Returns a pointer to the start of the struct (i.e., to the HeapHeader).
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_alloc_struct(size: u32, kind: u16) -> *mut u8 {
    let align = 8; // all v2 structs are 8-byte aligned
    let layout = std::alloc::Layout::from_size_align(size as usize, align).unwrap();
    let ptr = unsafe { std::alloc::alloc_zeroed(layout) };
    // Initialize the header
    unsafe {
        let header = ptr as *mut HeapHeader;
        std::ptr::write(header, HeapHeader::new(kind));
    }
    ptr
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_value::v2::heap_header::HEAP_KIND_V2_STRUCT;

    // ── Test-only shims for the pre-R7.2 typed push helpers ──────────────
    //
    // R7.2 consolidated the four `jit_v2_array_push_{f64,i64,i32,bool}`
    // entry points into a single `jit_v2_array_push(ptr, bits, elem_size)`
    // dispatcher. These shims keep the tests below readable while exercising
    // the same underlying push paths.

    fn jit_v2_array_push_f64(arr: *mut TypedArray<f64>, val: f64) {
        jit_v2_array_push(arr as *mut HeapHeader, val.to_bits(), 8);
    }

    fn jit_v2_array_push_i64(arr: *mut TypedArray<i64>, val: i64) {
        jit_v2_array_push(arr as *mut HeapHeader, val as u64, 8);
    }

    fn jit_v2_array_push_i32(arr: *mut TypedArray<i32>, val: i32) {
        jit_v2_array_push(arr as *mut HeapHeader, (val as u32) as u64, 4);
    }

    fn jit_v2_array_push_bool(arr: *mut TypedArray<u8>, val: u8) {
        jit_v2_array_push(arr as *mut HeapHeader, val as u64, 1);
    }

    // ── Phase C.3 SIMD sum tests ─────────────────────────────────────────

    #[test]
    fn test_simd_sum_f64_small_scalar_path() {
        // Below SIMD_SUM_THRESHOLD — exercises scalar accumulation.
        let arr = jit_v2_array_new_f64(8);
        for i in 0..8 {
            jit_v2_array_push_f64(arr, (i + 1) as f64); // 1..=8
        }
        let sum = jit_v2_array_sum_f64(arr);
        assert!((sum - 36.0).abs() < 1e-12);
        unsafe { TypedArray::drop_array(arr) };
    }

    #[test]
    fn test_simd_sum_f64_large_vector_path() {
        // Above SIMD_SUM_THRESHOLD with non-multiple-of-4 length (exercises
        // both the f64x4 loop and the scalar remainder).
        let arr = jit_v2_array_new_f64(128);
        let mut expected = 0.0_f64;
        for i in 0..101 {
            let v = i as f64 * 0.5;
            jit_v2_array_push_f64(arr, v);
            expected += v;
        }
        let sum = jit_v2_array_sum_f64(arr);
        assert!(
            (sum - expected).abs() < 1e-9,
            "sum={} expected={}",
            sum,
            expected
        );
        unsafe { TypedArray::drop_array(arr) };
    }

    #[test]
    fn test_simd_sum_f64_empty() {
        let arr = jit_v2_array_new_f64(0);
        let sum = jit_v2_array_sum_f64(arr);
        assert_eq!(sum, 0.0);
        unsafe { TypedArray::drop_array(arr) };
    }

    #[test]
    fn test_simd_sum_f64_null_safe() {
        assert_eq!(jit_v2_array_sum_f64(std::ptr::null()), 0.0);
    }

    #[test]
    fn test_simd_sum_i64_small_scalar_path() {
        let arr = jit_v2_array_new_i64(16);
        for i in 0..10 {
            jit_v2_array_push_i64(arr, (i + 1) as i64);
        }
        let sum = jit_v2_array_sum_i64(arr);
        assert_eq!(sum, 55);
        unsafe { TypedArray::drop_array(arr) };
    }

    #[test]
    fn test_simd_sum_i64_large_vector_path() {
        let arr = jit_v2_array_new_i64(128);
        let mut expected: i64 = 0;
        for i in 0..103 {
            let v = i as i64;
            jit_v2_array_push_i64(arr, v);
            expected = expected.wrapping_add(v);
        }
        let sum = jit_v2_array_sum_i64(arr);
        assert_eq!(sum, expected);
        unsafe { TypedArray::drop_array(arr) };
    }

    #[test]
    fn test_simd_sum_i64_wrapping_overflow() {
        // Two i64::MAX values should wrap without panicking. Padded to 16
        // elements so we go down the SIMD path that also uses wrapping adds.
        let arr = jit_v2_array_new_i64(16);
        jit_v2_array_push_i64(arr, i64::MAX);
        jit_v2_array_push_i64(arr, 1);
        for _ in 2..16 {
            jit_v2_array_push_i64(arr, 0);
        }
        let sum = jit_v2_array_sum_i64(arr);
        assert_eq!(sum, i64::MAX.wrapping_add(1));
        unsafe { TypedArray::drop_array(arr) };
    }

    #[test]
    fn test_array_f64_roundtrip() {
        let arr = jit_v2_array_new_f64(4);
        jit_v2_array_push_f64(arr, 1.0);
        jit_v2_array_push_f64(arr, 2.5);
        jit_v2_array_push_f64(arr, 3.14);
        assert_eq!(jit_v2_array_len_f64(arr), 3);
        assert!((jit_v2_array_get_f64(arr, 0) - 1.0).abs() < f64::EPSILON);
        assert!((jit_v2_array_get_f64(arr, 1) - 2.5).abs() < f64::EPSILON);
        assert!((jit_v2_array_get_f64(arr, 2) - 3.14).abs() < f64::EPSILON);
        unsafe { TypedArray::drop_array(arr) };
    }

    #[test]
    fn test_array_i64_roundtrip() {
        let arr = jit_v2_array_new_i64(4);
        jit_v2_array_push_i64(arr, 42);
        jit_v2_array_push_i64(arr, -100);
        assert_eq!(jit_v2_array_len_i64(arr), 2);
        assert_eq!(jit_v2_array_get_i64(arr, 0), 42);
        assert_eq!(jit_v2_array_get_i64(arr, 1), -100);
        unsafe { TypedArray::drop_array(arr) };
    }

    #[test]
    fn test_array_i32_roundtrip() {
        let arr = jit_v2_array_new_i32(4);
        jit_v2_array_push_i32(arr, 7);
        jit_v2_array_push_i32(arr, -3);
        assert_eq!(jit_v2_array_len_i32(arr), 2);
        assert_eq!(jit_v2_array_get_i32(arr, 0), 7);
        assert_eq!(jit_v2_array_get_i32(arr, 1), -3);
        unsafe { TypedArray::drop_array(arr) };
    }

    #[test]
    fn test_array_bool_roundtrip() {
        // Bool elements are stored as u8 internally (0 = false, 1 = true).
        let arr = jit_v2_array_new_bool(4);
        jit_v2_array_push_bool(arr, 1);
        jit_v2_array_push_bool(arr, 0);
        jit_v2_array_push_bool(arr, 1);
        assert_eq!(jit_v2_array_len_bool(arr), 3);
        assert_eq!(jit_v2_array_get_bool(arr, 0), 1);
        assert_eq!(jit_v2_array_get_bool(arr, 1), 0);
        assert_eq!(jit_v2_array_get_bool(arr, 2), 1);
        unsafe { TypedArray::drop_array(arr) };
    }

    #[test]
    fn test_array_set_bool() {
        let arr = jit_v2_array_new_bool(4);
        jit_v2_array_push_bool(arr, 0);
        jit_v2_array_push_bool(arr, 0);
        jit_v2_array_set_bool(arr, 0, 1);
        assert_eq!(jit_v2_array_get_bool(arr, 0), 1);
        assert_eq!(jit_v2_array_get_bool(arr, 1), 0);
        unsafe { TypedArray::drop_array(arr) };
    }

    #[test]
    fn test_array_set_f64() {
        let arr = jit_v2_array_new_f64(4);
        jit_v2_array_push_f64(arr, 1.0);
        jit_v2_array_push_f64(arr, 2.0);
        jit_v2_array_set_f64(arr, 0, 99.0);
        assert!((jit_v2_array_get_f64(arr, 0) - 99.0).abs() < f64::EPSILON);
        assert!((jit_v2_array_get_f64(arr, 1) - 2.0).abs() < f64::EPSILON);
        unsafe { TypedArray::drop_array(arr) };
    }

    #[test]
    fn test_array_get_oob_returns_none_via_typed_array() {
        // Can't use #[should_panic] on extern "C" functions (UB).
        // Instead, test bounds via the underlying TypedArray::get which returns None.
        let arr = jit_v2_array_new_f64(4);
        jit_v2_array_push_f64(arr, 1.0);
        unsafe {
            assert_eq!(TypedArray::get(arr, 5), None);
            TypedArray::drop_array(arr);
        }
    }

    #[test]
    fn test_field_load_store_f64() {
        let ptr = jit_v2_alloc_struct(24, HEAP_KIND_V2_STRUCT);
        jit_v2_field_store_f64(ptr, 8, 3.14);
        let val = jit_v2_field_load_f64(ptr, 8);
        assert!((val - 3.14).abs() < f64::EPSILON);
        unsafe { std::alloc::dealloc(ptr, std::alloc::Layout::from_size_align(24, 8).unwrap()) };
    }

    #[test]
    fn test_field_load_store_i64() {
        let ptr = jit_v2_alloc_struct(24, HEAP_KIND_V2_STRUCT);
        jit_v2_field_store_i64(ptr, 8, -42);
        assert_eq!(jit_v2_field_load_i64(ptr, 8), -42);
        unsafe { std::alloc::dealloc(ptr, std::alloc::Layout::from_size_align(24, 8).unwrap()) };
    }

    #[test]
    fn test_field_load_store_i32() {
        let ptr = jit_v2_alloc_struct(16, HEAP_KIND_V2_STRUCT);
        jit_v2_field_store_i32(ptr, 8, 999);
        assert_eq!(jit_v2_field_load_i32(ptr, 8), 999);
        unsafe { std::alloc::dealloc(ptr, std::alloc::Layout::from_size_align(16, 8).unwrap()) };
    }

    #[test]
    fn test_alloc_struct_initializes_header() {
        let ptr = jit_v2_alloc_struct(24, HEAP_KIND_V2_STRUCT);
        unsafe {
            let header = &*(ptr as *const HeapHeader);
            assert_eq!(header.kind(), HEAP_KIND_V2_STRUCT);
            assert_eq!(header.get_refcount(), 1);
            std::alloc::dealloc(ptr, std::alloc::Layout::from_size_align(24, 8).unwrap());
        }
    }

    // ── SIMD min/max/mean/sum-of-squares tests ────────────────────────────

    fn fill_f64(arr: *mut TypedArray<f64>, vals: &[f64]) {
        for &v in vals {
            jit_v2_array_push_f64(arr, v);
        }
    }

    #[test]
    fn test_simd_min_f64_small_scalar_path() {
        let arr = jit_v2_array_new_f64(8);
        fill_f64(arr, &[3.0, 1.5, 4.0, -2.0, 0.5, 7.0, -9.0, 2.0]);
        let m = jit_v2_array_min_f64(arr);
        assert_eq!(m, -9.0);
        unsafe { TypedArray::drop_array(arr) };
    }

    #[test]
    fn test_simd_min_f64_large_vector_path() {
        let arr = jit_v2_array_new_f64(64);
        let mut expected = f64::INFINITY;
        for i in 0..33 {
            // non-multiple of 4 for remainder coverage
            let v = (17 - i) as f64 * 0.25;
            jit_v2_array_push_f64(arr, v);
            if v < expected {
                expected = v;
            }
        }
        let m = jit_v2_array_min_f64(arr);
        assert!((m - expected).abs() < 1e-12);
        unsafe { TypedArray::drop_array(arr) };
    }

    #[test]
    fn test_simd_min_f64_empty_and_null() {
        let arr = jit_v2_array_new_f64(0);
        assert!(jit_v2_array_min_f64(arr).is_nan());
        unsafe { TypedArray::drop_array(arr) };
        assert!(jit_v2_array_min_f64(std::ptr::null()).is_nan());
    }

    #[test]
    fn test_simd_min_f64_single_element() {
        let arr = jit_v2_array_new_f64(1);
        jit_v2_array_push_f64(arr, 42.5);
        assert_eq!(jit_v2_array_min_f64(arr), 42.5);
        unsafe { TypedArray::drop_array(arr) };
    }

    #[test]
    fn test_simd_min_f64_nan_propagates() {
        // Scalar path
        let arr = jit_v2_array_new_f64(4);
        fill_f64(arr, &[1.0, 2.0, f64::NAN, 4.0]);
        assert!(jit_v2_array_min_f64(arr).is_nan());
        unsafe { TypedArray::drop_array(arr) };
        // SIMD path (>= 16 elements)
        let arr = jit_v2_array_new_f64(20);
        let mut v = vec![1.0_f64; 20];
        v[7] = f64::NAN;
        fill_f64(arr, &v);
        assert!(jit_v2_array_min_f64(arr).is_nan());
        unsafe { TypedArray::drop_array(arr) };
    }

    #[test]
    fn test_simd_max_f64_small_scalar_path() {
        let arr = jit_v2_array_new_f64(8);
        fill_f64(arr, &[3.0, 1.5, 4.0, -2.0, 0.5, 7.0, -9.0, 2.0]);
        let m = jit_v2_array_max_f64(arr);
        assert_eq!(m, 7.0);
        unsafe { TypedArray::drop_array(arr) };
    }

    #[test]
    fn test_simd_max_f64_large_vector_path() {
        let arr = jit_v2_array_new_f64(64);
        let mut expected = f64::NEG_INFINITY;
        for i in 0..37 {
            let v = (i as f64 * 1.3) - 5.0;
            jit_v2_array_push_f64(arr, v);
            if v > expected {
                expected = v;
            }
        }
        let m = jit_v2_array_max_f64(arr);
        assert!((m - expected).abs() < 1e-12);
        unsafe { TypedArray::drop_array(arr) };
    }

    #[test]
    fn test_simd_max_f64_nan_propagates() {
        let arr = jit_v2_array_new_f64(20);
        let mut v = vec![1.0_f64; 20];
        v[3] = f64::NAN;
        fill_f64(arr, &v);
        assert!(jit_v2_array_max_f64(arr).is_nan());
        unsafe { TypedArray::drop_array(arr) };
    }

    #[test]
    fn test_simd_mean_f64_small() {
        let arr = jit_v2_array_new_f64(4);
        fill_f64(arr, &[1.0, 2.0, 3.0, 4.0]);
        let m = jit_v2_array_mean_f64(arr);
        assert!((m - 2.5).abs() < 1e-12);
        unsafe { TypedArray::drop_array(arr) };
    }

    #[test]
    fn test_simd_mean_f64_large() {
        let arr = jit_v2_array_new_f64(64);
        let mut total = 0.0_f64;
        for i in 0..50 {
            let v = i as f64 + 0.5;
            jit_v2_array_push_f64(arr, v);
            total += v;
        }
        let expected = total / 50.0;
        let m = jit_v2_array_mean_f64(arr);
        assert!(
            (m - expected).abs() < 1e-9,
            "mean={} expected={}",
            m,
            expected
        );
        unsafe { TypedArray::drop_array(arr) };
    }

    #[test]
    fn test_simd_mean_f64_empty_is_nan() {
        let arr = jit_v2_array_new_f64(0);
        assert!(jit_v2_array_mean_f64(arr).is_nan());
        unsafe { TypedArray::drop_array(arr) };
        assert!(jit_v2_array_mean_f64(std::ptr::null()).is_nan());
    }

    #[test]
    fn test_simd_sum_squares_f64_small() {
        let arr = jit_v2_array_new_f64(4);
        fill_f64(arr, &[1.0, 2.0, 3.0, 4.0]);
        // 1 + 4 + 9 + 16 = 30
        let s = jit_v2_array_sum_squares_f64(arr);
        assert!((s - 30.0).abs() < 1e-12);
        unsafe { TypedArray::drop_array(arr) };
    }

    #[test]
    fn test_simd_sum_squares_f64_large() {
        let arr = jit_v2_array_new_f64(64);
        let mut expected = 0.0_f64;
        for i in 0..50 {
            let v = (i as f64 - 25.0) * 0.5;
            jit_v2_array_push_f64(arr, v);
            expected += v * v;
        }
        let s = jit_v2_array_sum_squares_f64(arr);
        assert!((s - expected).abs() < 1e-8);
        unsafe { TypedArray::drop_array(arr) };
    }

    #[test]
    fn test_simd_sum_squares_f64_empty() {
        let arr = jit_v2_array_new_f64(0);
        assert_eq!(jit_v2_array_sum_squares_f64(arr), 0.0);
        unsafe { TypedArray::drop_array(arr) };
        assert_eq!(jit_v2_array_sum_squares_f64(std::ptr::null()), 0.0);
    }

    // ── SIMD allocating transforms ────────────────────────────────────────

    fn collect_f64(arr: *const TypedArray<f64>) -> Vec<f64> {
        let len = unsafe { (*arr).len } as usize;
        let mut out = Vec::with_capacity(len);
        for i in 0..len {
            out.push(unsafe { TypedArray::<f64>::get_unchecked(arr, i as u32) });
        }
        out
    }

    #[test]
    fn test_simd_scale_f64_small() {
        let a = jit_v2_array_new_f64(4);
        fill_f64(a, &[1.0, 2.0, 3.0, 4.0]);
        let out = jit_v2_array_scale_f64(a, 2.5);
        assert_eq!(collect_f64(out), vec![2.5, 5.0, 7.5, 10.0]);
        unsafe {
            TypedArray::drop_array(a);
            TypedArray::drop_array(out);
        }
    }

    #[test]
    fn test_simd_scale_f64_large() {
        let a = jit_v2_array_new_f64(32);
        for i in 0..20 {
            jit_v2_array_push_f64(a, i as f64);
        }
        let out = jit_v2_array_scale_f64(a, -0.5);
        let got = collect_f64(out);
        for i in 0..20 {
            assert!((got[i] - (i as f64 * -0.5)).abs() < 1e-12);
        }
        unsafe {
            TypedArray::drop_array(a);
            TypedArray::drop_array(out);
        }
    }

    #[test]
    fn test_simd_scale_f64_empty() {
        let a = jit_v2_array_new_f64(0);
        let out = jit_v2_array_scale_f64(a, 3.0);
        assert_eq!(unsafe { (*out).len }, 0);
        unsafe {
            TypedArray::drop_array(a);
            TypedArray::drop_array(out);
        }
    }

    #[test]
    fn test_simd_add_scalar_f64_small() {
        let a = jit_v2_array_new_f64(3);
        fill_f64(a, &[1.0, 2.0, 3.0]);
        let out = jit_v2_array_add_scalar_f64(a, 10.0);
        assert_eq!(collect_f64(out), vec![11.0, 12.0, 13.0]);
        unsafe {
            TypedArray::drop_array(a);
            TypedArray::drop_array(out);
        }
    }

    #[test]
    fn test_simd_add_scalar_f64_large() {
        let a = jit_v2_array_new_f64(32);
        for i in 0..25 {
            jit_v2_array_push_f64(a, i as f64);
        }
        let out = jit_v2_array_add_scalar_f64(a, 100.0);
        let got = collect_f64(out);
        for i in 0..25 {
            assert!((got[i] - (i as f64 + 100.0)).abs() < 1e-12);
        }
        unsafe {
            TypedArray::drop_array(a);
            TypedArray::drop_array(out);
        }
    }

    #[test]
    fn test_simd_add_f64_small() {
        let a = jit_v2_array_new_f64(4);
        let b = jit_v2_array_new_f64(4);
        fill_f64(a, &[1.0, 2.0, 3.0, 4.0]);
        fill_f64(b, &[10.0, 20.0, 30.0, 40.0]);
        let out = jit_v2_array_add_f64(a, b);
        assert_eq!(collect_f64(out), vec![11.0, 22.0, 33.0, 44.0]);
        unsafe {
            TypedArray::drop_array(a);
            TypedArray::drop_array(b);
            TypedArray::drop_array(out);
        }
    }

    #[test]
    fn test_simd_add_f64_large() {
        let a = jit_v2_array_new_f64(32);
        let b = jit_v2_array_new_f64(32);
        for i in 0..23 {
            jit_v2_array_push_f64(a, i as f64);
            jit_v2_array_push_f64(b, (i * 2) as f64);
        }
        let out = jit_v2_array_add_f64(a, b);
        let got = collect_f64(out);
        for i in 0..23 {
            assert!((got[i] - (i as f64 + (i * 2) as f64)).abs() < 1e-12);
        }
        unsafe {
            TypedArray::drop_array(a);
            TypedArray::drop_array(b);
            TypedArray::drop_array(out);
        }
    }

    #[test]
    fn test_simd_mul_f64_small() {
        let a = jit_v2_array_new_f64(4);
        let b = jit_v2_array_new_f64(4);
        fill_f64(a, &[1.0, 2.0, 3.0, 4.0]);
        fill_f64(b, &[10.0, 20.0, 30.0, 40.0]);
        let out = jit_v2_array_mul_f64(a, b);
        assert_eq!(collect_f64(out), vec![10.0, 40.0, 90.0, 160.0]);
        unsafe {
            TypedArray::drop_array(a);
            TypedArray::drop_array(b);
            TypedArray::drop_array(out);
        }
    }

    #[test]
    fn test_simd_mul_f64_large() {
        let a = jit_v2_array_new_f64(32);
        let b = jit_v2_array_new_f64(32);
        for i in 0..20 {
            jit_v2_array_push_f64(a, (i as f64 + 1.0) * 0.5);
            jit_v2_array_push_f64(b, (i as f64 + 1.0) * 2.0);
        }
        let out = jit_v2_array_mul_f64(a, b);
        let got = collect_f64(out);
        for i in 0..20 {
            let expected = ((i as f64 + 1.0) * 0.5) * ((i as f64 + 1.0) * 2.0);
            assert!((got[i] - expected).abs() < 1e-12);
        }
        unsafe {
            TypedArray::drop_array(a);
            TypedArray::drop_array(b);
            TypedArray::drop_array(out);
        }
    }

    #[test]
    fn test_simd_add_f64_empty() {
        let a = jit_v2_array_new_f64(0);
        let b = jit_v2_array_new_f64(0);
        let out = jit_v2_array_add_f64(a, b);
        assert_eq!(unsafe { (*out).len }, 0);
        unsafe {
            TypedArray::drop_array(a);
            TypedArray::drop_array(b);
            TypedArray::drop_array(out);
        }
    }

    #[test]
    fn test_simd_add_f64_null_inputs() {
        assert!(jit_v2_array_add_f64(std::ptr::null(), std::ptr::null()).is_null());
    }

    #[test]
    fn test_retain_increments_refcount() {
        let ptr = jit_v2_alloc_struct(24, HEAP_KIND_V2_STRUCT);
        unsafe {
            let header = &*(ptr as *const HeapHeader);
            assert_eq!(header.get_refcount(), 1);
            jit_v2_retain(ptr);
            assert_eq!(header.get_refcount(), 2);
            jit_v2_retain(ptr);
            assert_eq!(header.get_refcount(), 3);
            // Clean up manually (don't use jit_v2_release which would dealloc wrong size)
            std::alloc::dealloc(ptr, std::alloc::Layout::from_size_align(24, 8).unwrap());
        }
    }
}
