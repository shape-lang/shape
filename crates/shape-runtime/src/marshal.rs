//! Strict-typed marshal layer for native module function dispatch.
//!
//! Replaces the deleted `Fn(&[ValueWord], &ModuleContext) -> Result<ValueWord>`
//! body shape (the dynamic-FFI escape hatch). Native function bodies now
//! take **typed Rust arguments** that implement [`FromSlot`]; the function's
//! Rust signature *is* the typed signature, and the marshal layer cannot be
//! registered against mismatching kinds because the Rust trait system rejects
//! the [`register_typed_fn_N`] generic constraints.
//!
//! Mirrors the structural-enforcement track from Phase 2a: forbidden
//! mismatches are unrepresentable, not just unreachable. See
//! `docs/defections.md` 2026-05-06 (Phase 2b unified marshal + wire/snapshot).
//!
//! ## What's here
//!
//! - [`FromSlot`] / [`ToSlot`]: read/write a typed value from/to an 8-byte
//!   `u64` slot. Each impl pins a single [`NativeKind`] via the associated
//!   constant.
//! - [`MarshalError`]: typed error returned by the marshal boundary.
//! - [`register_typed_fn_0`] … [`register_typed_fn_3`]: per-arity
//!   registration helpers. Each wraps a body whose Rust parameter types
//!   carry the typed argument contract (each `Pi: FromSlot`).
//!
//! ## What's not here yet
//!
//! - Higher-arity helpers (4+) — added on demand when stdlib migrations need them.
//! - `ToSlot` for container `TypedReturn` variants (`Ok`/`Err`/`Some`/
//!   `ObjectPairs`/etc.) — these need monomorphized heap representations
//!   and land alongside the per-stdlib-module migrations in Phase 2c.

use crate::module_exports::ModuleContext;
use crate::typed_module_exports::TypedReturn;
use shape_value::NativeKind;
use std::sync::Arc;

/// Read a typed value from an 8-byte raw-bits slot.
///
/// The associated constant [`Self::NATIVE_KIND`] declares which kind
/// the slot must have. The marshal-layer dispatcher guarantees the
/// contract by reading `arg_kinds()` at registration and only invoking
/// the body with matching slot bits — `from_slot` impls therefore do
/// not invoke the deleted `tag_bits` dispatch.
pub trait FromSlot: Sized {
    const NATIVE_KIND: NativeKind;
    /// SAFETY contract (enforced by the marshal-layer wrapper, not by
    /// this trait method): `bits` must have been produced by a slot
    /// that was statically proven to have kind `NATIVE_KIND`.
    fn from_slot(bits: u64) -> Self;
}

/// Write a typed value into an 8-byte raw-bits slot.
///
/// Symmetric to [`FromSlot`]. Used by per-arity registration helpers
/// when the body returns a primitive-typed value directly. Container
/// `TypedReturn` variants (`Ok`/`Err`/`Some`/`ObjectPairs`/etc.)
/// don't impl `ToSlot` — they're projected by the dispatcher's
/// `TypedReturn → slot push` step (Phase 2c per-module migrations).
pub trait ToSlot {
    const NATIVE_KIND: NativeKind;
    fn to_slot(self) -> u64;
}

/// Typed error returned at the marshal boundary.
///
/// Replaces panics from the deleted `into_value_word()` boundary. The
/// dispatcher converts `MarshalError` into a `Result<TypedReturn, String>`
/// at the registry edge so legacy `String`-error paths keep working
/// during the migration.
#[derive(Debug, Clone, PartialEq)]
pub enum MarshalError {
    /// Arg count mismatch between the function's registered arity and
    /// the slot slice handed in by the dispatcher.
    ArgCount { expected: usize, got: usize },
    /// The body returned an `Err(String)` — surfaced verbatim.
    Body(String),
}

impl std::fmt::Display for MarshalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MarshalError::ArgCount { expected, got } => {
                write!(f, "expected {} arg(s), got {}", expected, got)
            }
            MarshalError::Body(msg) => f.write_str(msg),
        }
    }
}

impl std::error::Error for MarshalError {}

impl From<MarshalError> for String {
    fn from(e: MarshalError) -> Self {
        e.to_string()
    }
}

// ───────────────────────────── FromSlot impls ─────────────────────────────

impl FromSlot for i64 {
    const NATIVE_KIND: NativeKind = NativeKind::Int64;
    #[inline]
    fn from_slot(bits: u64) -> Self {
        bits as i64
    }
}

impl FromSlot for f64 {
    const NATIVE_KIND: NativeKind = NativeKind::Float64;
    #[inline]
    fn from_slot(bits: u64) -> Self {
        f64::from_bits(bits)
    }
}

// NaN-sentinel discrimination matches NullableFloat64's documented contract
// (native_kind.rs:36). Reusing an already-declared sentinel kind is consumer-side
// adoption, not a new sentinel introduction.
impl FromSlot for Option<f64> {
    const NATIVE_KIND: NativeKind = NativeKind::NullableFloat64;
    #[inline]
    fn from_slot(bits: u64) -> Self {
        let v = f64::from_bits(bits);
        if v.is_nan() {
            None
        } else {
            Some(v)
        }
    }
}

impl FromSlot for bool {
    const NATIVE_KIND: NativeKind = NativeKind::Bool;
    #[inline]
    fn from_slot(bits: u64) -> Self {
        bits != 0
    }
}

/// Read an `Arc<String>` from a heap-pointer slot.
///
/// The slot owns one strong reference; cloning it for the body's use
/// requires incrementing the refcount. The marshal wrapper does not
/// take ownership of the slot — it stays valid for the duration of
/// the call. The body receives an independent strong reference.
impl FromSlot for Arc<String> {
    const NATIVE_KIND: NativeKind = NativeKind::String;
    #[inline]
    fn from_slot(bits: u64) -> Self {
        let ptr = bits as *const String;
        // SAFETY: NATIVE_KIND::String pins this slot to an Arc<String>
        // raw pointer produced by `Arc::into_raw` at write time. The
        // dispatcher guarantees kind match via the Phase 2b registration
        // contract.
        unsafe {
            Arc::increment_strong_count(ptr);
            Arc::from_raw(ptr)
        }
    }
}

// ───────────────────────────── ToSlot impls ─────────────────────────────

impl ToSlot for i64 {
    const NATIVE_KIND: NativeKind = NativeKind::Int64;
    #[inline]
    fn to_slot(self) -> u64 {
        self as u64
    }
}

impl ToSlot for f64 {
    const NATIVE_KIND: NativeKind = NativeKind::Float64;
    #[inline]
    fn to_slot(self) -> u64 {
        self.to_bits()
    }
}

impl ToSlot for bool {
    const NATIVE_KIND: NativeKind = NativeKind::Bool;
    #[inline]
    fn to_slot(self) -> u64 {
        self as u64
    }
}

impl ToSlot for Arc<String> {
    const NATIVE_KIND: NativeKind = NativeKind::String;
    #[inline]
    fn to_slot(self) -> u64 {
        Arc::into_raw(self) as u64
    }
}

// ──────────────────── heap-pointer FromSlot/ToSlot ────────────────────
//
// Heap-allocated stdlib returns and slot reads project through
// `Arc<HeapValue>`. The slot bits are an `Arc<HeapValue>` raw pointer;
// the kind (`NativeKind::Ptr(HeapKind::*)`) tells the dispatcher which
// `HeapValue` arm decodes the bits without probing the object's
// self-reported discriminant.
//
// Body-side helpers below construct typed return values from the inner
// Rust types (`Arc<DataTable>`, `Arc<Instant>`, etc.) by wrapping in
// `HeapValue::*` then `Arc::new`. Reading goes the other way: cast bits
// to `*const HeapValue`, pattern-match the expected arm.

/// Read the inner `Arc<DataTable>` from a `NativeKind::Ptr(HeapKind::DataTable)` slot.
impl FromSlot for Arc<shape_value::DataTable>
where
    Self: Sized,
{
    const NATIVE_KIND: NativeKind =
        NativeKind::Ptr(shape_value::HeapKind::DataTable);
    #[inline]
    fn from_slot(bits: u64) -> Self {
        let ptr = bits as *const shape_value::HeapValue;
        // SAFETY: NATIVE_KIND::Ptr(HeapKind::DataTable) pins the bits to
        // an Arc<HeapValue> with the DataTable variant. We clone the
        // inner Arc<DataTable> without consuming the slot's strong ref.
        unsafe {
            Arc::increment_strong_count(ptr);
            let arc_hv = Arc::from_raw(ptr);
            match &*arc_hv {
                shape_value::HeapValue::DataTable(arc_dt) => Arc::clone(arc_dt),
                other => panic!(
                    "FromSlot<Arc<DataTable>>: slot bits decoded to HeapValue::{:?}, \
                     not DataTable. Marshal kind contract violated by caller.",
                    other.kind()
                ),
            }
        }
    }
}

/// Write an `Arc<DataTable>` into a heap slot by wrapping in
/// `HeapValue::DataTable` and producing the raw `Arc<HeapValue>` pointer.
impl ToSlot for Arc<shape_value::DataTable> {
    const NATIVE_KIND: NativeKind =
        NativeKind::Ptr(shape_value::HeapKind::DataTable);
    #[inline]
    fn to_slot(self) -> u64 {
        let hv = Arc::new(shape_value::HeapValue::DataTable(self));
        Arc::into_raw(hv) as u64
    }
}

// ────────────────────── IoHandle FromSlot/ToSlot (option γ) ───────────────
//
// Cluster #2 (docs/defections.md 2026-05-06): IoHandle marshal extension
// via Arc<IoHandleData>. Mirrors the Arc<DataTable> shape exactly.
//
// `HeapValue::IoHandle` payload was changed from Box<IoHandleData> to
// Arc<IoHandleData> in the prior commit specifically so the FromSlot
// projection here is one atomic op (Arc::clone of the inner Arc) rather
// than a Box clone (alloc + memcpy). Bodies declare
// `handle: Arc<IoHandleData>` and call methods on it via Arc::deref —
// `handle.is_open()`, `handle.close()`, `handle.resource.lock()`.
//
// Same consistency-check residual as Arc<DataTable> at marshal.rs:193 —
// the body's `Arc<IoHandleData>` parameter type pins the expected
// `HeapValue::IoHandle` variant; the panic-on-mismatch arm is
// unreachable in a well-typed system per
// `docs/runtime-v2-spec.md` ("consistency check, not probe").

/// Read the inner `Arc<IoHandleData>` from a `NativeKind::Ptr(HeapKind::IoHandle)` slot.
impl FromSlot for Arc<shape_value::heap_value::IoHandleData>
where
    Self: Sized,
{
    const NATIVE_KIND: NativeKind =
        NativeKind::Ptr(shape_value::HeapKind::IoHandle);
    #[inline]
    fn from_slot(bits: u64) -> Self {
        let ptr = bits as *const shape_value::HeapValue;
        // SAFETY: NATIVE_KIND::Ptr(HeapKind::IoHandle) pins the bits to
        // an Arc<HeapValue> with the IoHandle variant. We clone the
        // inner Arc<IoHandleData> without consuming the slot's strong ref.
        unsafe {
            Arc::increment_strong_count(ptr);
            let arc_hv = Arc::from_raw(ptr);
            match &*arc_hv {
                shape_value::HeapValue::IoHandle(arc_io) => Arc::clone(arc_io),
                other => panic!(
                    "FromSlot<Arc<IoHandleData>>: slot bits decoded to HeapValue::{:?}, \
                     not IoHandle. Marshal kind contract violated by caller.",
                    other.kind()
                ),
            }
        }
    }
}

/// Write an `Arc<IoHandleData>` into a heap slot by wrapping in
/// `HeapValue::IoHandle` and producing the raw `Arc<HeapValue>` pointer.
impl ToSlot for Arc<shape_value::heap_value::IoHandleData> {
    const NATIVE_KIND: NativeKind =
        NativeKind::Ptr(shape_value::HeapKind::IoHandle);
    #[inline]
    fn to_slot(self) -> u64 {
        let hv = Arc::new(shape_value::HeapValue::IoHandle(self));
        Arc::into_raw(hv) as u64
    }
}

// ──────────────────── typed-array FromSlot/ToSlot (option β) ─────────────────
//
// V3-S5 ckpt-5-prime²c (2026-05-15): the `HeapValue::TypedArray` outer arm +
// `TypedArrayData` enum + `TypedBuffer<T>` / `AlignedTypedBuffer` wrapper
// layer were retired wholesale at V3-S5 ckpt-1..ckpt-5 per
// `docs/cluster-audits/w12-typed-array-data-deletion-audit.md` §3.5/§3.6 +
// audit §B + ADR-006 §2.7.24 Q25.A SUPERSEDED. The strict-typed array
// carrier is now the monomorphic flat-struct `*mut TypedArray<T>` shape per
// `docs/runtime-v2-spec.md`; slot bits for `NativeKind::Ptr(HeapKind::
// TypedArray)` are a raw pointer to a `crate::v2::typed_array::TypedArray<T>`
// for the element-width `T` that the body declares.
//
// Element-width discrimination is via the body's declared parameter type
// (`Vec<u8>` vs `Vec<i64>` vs `Vec<f64>` vs `Vec<Arc<String>>`, or their
// `Arc<Vec<T>>` wrappers), not via NativeKind: `NATIVE_KIND` stays
// `Ptr(HeapKind::TypedArray)` for all element widths. Element-width threading
// is enforced by the Rust type system at the impl level, with an unsafe
// raw-pointer read of the matching `TypedArray<T>::as_slice` that copies
// elements into a fresh `Vec<T>` (owns-clone semantics) or wraps in
// `Arc<Vec<T>>` (zero-copy of the inner `Vec`, one `Arc::new` for the outer).
//
// Per `docs/runtime-v2-spec.md`: "the kind tells you the arm; the body's
// declared parameter type tells you the element width; no runtime
// element-width probe." The dispatcher's registration-time arg-kind contract
// already verified the slot bits decode to a `HeapKind::TypedArray` raw
// pointer; the per-`T` impl picks the element width via the Rust type
// system. If a slot's actual element-width disagrees with the impl's
// declared `T`, the result is UB by design (compiler/dispatcher contract
// violation), not a panic — same as the post-strict-typing dispatch
// contract for typed slots in general.
//
// V3-S5 ckpt-5-prime²c migration shape (a) RATIFIED:
//   `Arc<AlignedTypedBuffer>` → `Arc<Vec<f64>>`  (intrinsics body-type)
//   `Arc<TypedBuffer<i64>>`   → `Arc<Vec<i64>>`  (intrinsics body-type)
//   `Arc<TypedBuffer<u8>>`    → `Arc<Vec<u8>>`   (intrinsics body-type)
//   `Arc<TypedBuffer<Arc<String>>>` → `Arc<Vec<Arc<String>>>`  (not yet
//     reached by intrinsics; kept aligned to the same shape for the future
//     string-cluster migration when a stdlib body surfaces).
//
// The `Vec<Arc<HeapValue>>` polymorphic-element marshal path is
// surface-and-stop in this checkpoint: the `materialize_heap_arcs` helper
// that re-wrapped each strict-typed element into a `HeapValue::*` Arc
// referenced the deleted `TypedArrayData` enum directly. Stdlib bodies
// declaring `Vec<Arc<HeapValue>>` parameters cannot decode the new
// `*mut TypedArray<T>` slot bits without a per-`T` dispatcher (Round 2
// follow-up — pairs with the `from_typed_array_<T>` constructor wave at
// `crates/shape-value/src/slot.rs:142`). Active impl panics with a
// structured error pointing at the follow-up.

/// Read a `Vec<u8>` from a `NativeKind::Ptr(HeapKind::TypedArray)` slot
/// whose payload is `*mut TypedArray<u8>`.
///
/// V3-S5 ckpt-5-prime²c (2026-05-15): rewritten for the v2-raw flat-struct
/// carrier. Slot bits are a raw `*mut TypedArray<u8>` pointer; element-data
/// is copied into a fresh `Vec<u8>` (owns-clone semantics — body receives
/// an owned vector independent of the slot's refcount share).
impl FromSlot for Vec<u8> {
    const NATIVE_KIND: NativeKind =
        NativeKind::Ptr(shape_value::HeapKind::TypedArray);
    #[inline]
    fn from_slot(bits: u64) -> Self {
        // SAFETY: NATIVE_KIND::Ptr(HeapKind::TypedArray) + body-declared
        // element type Vec<u8> pins the slot bits to a live
        // *mut TypedArray<u8>. The marshal kind contract guarantees both;
        // dispatcher-side stamp_elem_type at array.rs:78 carries the
        // element discriminant for completeness but the body type is the
        // primary discriminator per the post-strict-typing contract.
        let arr = bits as usize as *const shape_value::v2::typed_array::TypedArray<u8>;
        if arr.is_null() {
            return Vec::new();
        }
        unsafe {
            shape_value::v2::typed_array::TypedArray::<u8>::as_slice(arr).to_vec()
        }
    }
}

/// Read a `Vec<i64>` from a `NativeKind::Ptr(HeapKind::TypedArray)` slot
/// whose payload is `*mut TypedArray<i64>`. V3-S5 ckpt-5-prime²c
/// (2026-05-15): rewritten for the v2-raw flat-struct carrier. Owns-clone
/// semantics.
impl FromSlot for Vec<i64> {
    const NATIVE_KIND: NativeKind =
        NativeKind::Ptr(shape_value::HeapKind::TypedArray);
    #[inline]
    fn from_slot(bits: u64) -> Self {
        // SAFETY: see Vec<u8>::from_slot above.
        let arr = bits as usize as *const shape_value::v2::typed_array::TypedArray<i64>;
        if arr.is_null() {
            return Vec::new();
        }
        unsafe {
            shape_value::v2::typed_array::TypedArray::<i64>::as_slice(arr).to_vec()
        }
    }
}

/// Read a `Vec<Arc<String>>` from a `NativeKind::Ptr(HeapKind::TypedArray)`
/// slot whose payload is `*mut TypedArray<*const StringObj>`. V3-S5
/// ckpt-5-prime²c (2026-05-15): rewritten for the v2-raw flat-struct
/// carrier (each element is a raw `*const StringObj` — the per-element
/// allocator-managed v2 string carrier per `crates/shape-value/src/v2/
/// string_obj.rs`). Each element string is copied into a fresh
/// `Arc<String>` (owns-clone semantics — body receives an owned vector
/// of independent Arcs).
impl FromSlot for Vec<Arc<String>> {
    const NATIVE_KIND: NativeKind =
        NativeKind::Ptr(shape_value::HeapKind::TypedArray);
    #[inline]
    fn from_slot(bits: u64) -> Self {
        // SAFETY: see Vec<u8>::from_slot above. The body-declared element
        // type pins the slot to a *mut TypedArray<*const StringObj>.
        let arr = bits as usize
            as *const shape_value::v2::typed_array::TypedArray<
                *const shape_value::v2::string_obj::StringObj,
            >;
        if arr.is_null() {
            return Vec::new();
        }
        unsafe {
            let slice = shape_value::v2::typed_array::TypedArray::<
                *const shape_value::v2::string_obj::StringObj,
            >::as_slice(arr);
            slice
                .iter()
                .map(|&p| {
                    Arc::new(
                        shape_value::v2::string_obj::StringObj::as_str(p).to_owned(),
                    )
                })
                .collect()
        }
    }
}

/// Read a `Vec<Arc<HeapValue>>` from a `NativeKind::Ptr(HeapKind::TypedArray)`
/// slot.
///
/// V3-S5 ckpt-5-prime²c (2026-05-15) SURFACE-AND-STOP: the
/// `materialize_heap_arcs` helper (deleted alongside this comment block)
/// re-wrapped each strict-typed element into a `HeapValue::*` Arc by
/// pattern-matching the deleted `TypedArrayData` enum. The new
/// `*mut TypedArray<T>` flat-struct carrier needs a per-`T` dispatcher
/// (one impl per element width: `*const StringObj`, `*const DecimalObj`,
/// `TypedObjectPtr`, char, etc.) plus a parallel element-kind discriminator
/// in the marshal layer to know which `T` the slot was constructed for.
/// The element discriminator already exists at the VM level
/// (`stamp_elem_type` at `crates/shape-vm/src/executor/v2_handlers/array.rs`)
/// but isn't yet exposed at the marshal-`FromSlot` boundary.
///
/// Adding this dispatcher is the Round 2 `Vec<Arc<HeapValue>>` rewire
/// follow-up; pairs with the `from_typed_array_<T>` constructor wave at
/// `crates/shape-value/src/slot.rs:142` and the matching ckpt-6 JIT FFI
/// String/Decimal build work. Until that lands, the marshal-FromSlot panics
/// with a structured error pointing at the follow-up — any stdlib body
/// declaring `Vec<Arc<HeapValue>>` and reaching this from_slot is currently
/// dead at the marshal boundary (consistent with V3-S5 ckpt-5's wholesale
/// `HeapValue::TypedArray` outer-arm deletion).
impl FromSlot for Vec<Arc<shape_value::heap_value::HeapValue>> {
    const NATIVE_KIND: NativeKind =
        NativeKind::Ptr(shape_value::HeapKind::TypedArray);
    #[inline]
    fn from_slot(_bits: u64) -> Self {
        panic!(
            "FromSlot<Vec<Arc<HeapValue>>>: V3-S5 ckpt-5-prime²c SURFACE — \
             the polymorphic Vec<Arc<HeapValue>> marshal path needs a \
             per-element-T dispatcher over the v2-raw *mut TypedArray<T> \
             carrier. Round 2 `Vec<Arc<HeapValue>>` rewire follow-up \
             (pairs with from_typed_array_<T> constructor wave). \
             ADR-006 §2.7.24 Q25.A SUPERSEDED."
        )
    }
}

/// Project a `Vec<Arc<String>>` into a `NativeKind::Ptr(HeapKind::TypedArray)`
/// slot whose payload is `*mut TypedArray<*const StringObj>`. V3-S5
/// ckpt-5-prime²c (2026-05-15): rewritten for the v2-raw flat-struct
/// carrier — each input `Arc<String>` is allocated as a fresh `StringObj`
/// with refcount=1, and the per-element pointers are packed into a new
/// `TypedArray<*const StringObj>`. The slot takes ownership of the
/// resulting raw pointer (refcount discipline goes through `v2_retain` /
/// `v2_release` per `HeapHeader`).
impl ToSlot for Vec<Arc<String>> {
    const NATIVE_KIND: NativeKind =
        NativeKind::Ptr(shape_value::HeapKind::TypedArray);
    #[inline]
    fn to_slot(self) -> u64 {
        let elems: Vec<*const shape_value::v2::string_obj::StringObj> = self
            .into_iter()
            .map(|s| {
                shape_value::v2::string_obj::StringObj::new(s.as_str())
                    as *const shape_value::v2::string_obj::StringObj
            })
            .collect();
        let arr = shape_value::v2::typed_array::TypedArray::<
            *const shape_value::v2::string_obj::StringObj,
        >::from_slice(&elems);
        arr as usize as u64
    }
}

/// Project a `Vec<Arc<HeapValue>>` into a `NativeKind::Ptr(HeapKind::TypedArray)`
/// slot.
///
/// V3-S5 ckpt-5-prime²c (2026-05-15) SURFACE-AND-STOP: same per-element-T
/// dispatcher gap as the `FromSlot<Vec<Arc<HeapValue>>>` reader above.
/// The pre-deletion `build_specialized_from_heap_arcs` helper dispatched
/// each `HeapValue` arm into the matching `TypedArrayData::*` variant; the
/// new flat-struct carrier needs to pick the matching
/// `TypedArray<T>::from_slice` instantiation per element kind and
/// stamp the element discriminator before push. Pairs with the
/// Round 2 follow-up cited in the FromSlot impl.
impl ToSlot for Vec<Arc<shape_value::heap_value::HeapValue>> {
    const NATIVE_KIND: NativeKind =
        NativeKind::Ptr(shape_value::HeapKind::TypedArray);
    #[inline]
    fn to_slot(self) -> u64 {
        panic!(
            "ToSlot<Vec<Arc<HeapValue>>>: V3-S5 ckpt-5-prime²c SURFACE — \
             polymorphic Vec<Arc<HeapValue>> projection needs a per-element-T \
             dispatcher over the v2-raw *mut TypedArray<T> carrier (mirror \
             of the deleted build_specialized_from_heap_arcs). Round 2 \
             `Vec<Arc<HeapValue>>` rewire follow-up. ADR-006 §2.7.24 Q25.A \
             SUPERSEDED."
        )
    }
}

// ────── HashMap FromSlot impls (Stage C P1(b), 2026-05-07) ──────
//
// Stage C HashMap-marshal P1(b) per supervisor sign-off
// (`docs/defections.md` 2026-05-07 HashMap-marshal entry +
// audit-grounded correction subsection).
//
// Two `FromSlot` impls cover the dynamic-keys consumer surface (8 of 9
// stdlib body cases per Audit-1):
//
//   `Vec<(Arc<String>, Arc<String>)>`     — string-string maps (csv.parse_records,
//                                            csv.stringify_records, http inner
//                                            headers, xml attributes)
//   `Vec<(Arc<String>, Arc<HeapValue>)>`  — polymorphic-value maps (json
//                                            Json::Object, yaml, toml, msgpack,
//                                            xml node, http options arg)
//
// `NATIVE_KIND` stays `Ptr(HeapKind::HashMap)` for both — the value-element
// width discriminator lives in the body-side Rust type (option ε pattern),
// not in slot bits or `NativeKind`. Same consistency-check residual as
// Phase 2d Array's `Vec<Arc<String>>`/`Vec<Arc<HeapValue>>` impls: the
// in-body pattern match panics on a wrong inner-element shape (currently
// any HashMap stores `Arc<HeapValue>` values; the string-string variant
// pattern-matches each value as `HeapValue::String(_)` and unwraps).
//
// **No `ToSlot` impls in this commit** per supervisor instruction. Body
// returns of `ConcreteReturn::HashMapStringString` /
// `ConcreteReturn::HashMapStringHeapValue` are projected via the
// shape-vm dispatcher's `ConcreteReturn → slot push` path (shape-vm
// cleanup workstream territory, not Stage C scope). Adding `ToSlot`
// impls now would create dead-at-marshal-layer trait surface (per X4
// finding) AND specifically refused for `HashMapData` per supervisor
// sign-off ("no direct ToSlot for HashMapData; route through
// ConcreteReturn::HashMapStringHeapValue dispatch").

/// Read a `Vec<(Arc<String>, Arc<String>)>` from a
/// `NativeKind::Ptr(HeapKind::HashMap)` slot.
///
/// Body-type pattern: bodies declaring `args: Vec<(Arc<String>, Arc<String>)>`
/// receive an owned pair-list with insertion order preserved. Each value is
/// expected to be `HeapValue::String(_)`; mismatch panics as the
/// spec-permitted consistency check (`docs/runtime-v2-spec.md`).
impl FromSlot for Vec<(Arc<String>, Arc<String>)> {
    const NATIVE_KIND: NativeKind =
        NativeKind::Ptr(shape_value::HeapKind::HashMap);
    #[inline]
    fn from_slot(bits: u64) -> Self {
        let ptr = bits as *const shape_value::HeapValue;
        // SAFETY: see Vec<u8>::from_slot above — slot bits were proven by
        // the dispatcher to point to a valid `Arc<HeapValue>`.
        unsafe {
            Arc::increment_strong_count(ptr);
            let arc_hv = Arc::from_raw(ptr);
            match &*arc_hv {
                shape_value::HeapValue::HashMap(kref) => {
                    // Wave 2 Round 3b C2-joint ckpt-4 (2026-05-14): per-V
                    // walk for HashMap<string, string> (V=String). Other V
                    // variants panic — the marshal contract says caller
                    // declared a string-valued map; non-string V is a
                    // construction-side type error.
                    use shape_value::heap_value::HashMapKindedRef;
                    match kref {
                        HashMapKindedRef::String(arc) => {
                            let n = arc.len();
                            let mut out: Vec<(Arc<String>, Arc<String>)> =
                                Vec::with_capacity(n);
                            for i in 0..n {
                                let key = unsafe {
                                    let ptr = shape_value::v2::typed_array::TypedArray::get_unchecked(
                                        arc.keys, i as u32,
                                    );
                                    Arc::new(
                                        shape_value::v2::string_obj::StringObj::as_str(ptr)
                                            .to_owned(),
                                    )
                                };
                                let val = unsafe {
                                    let v_ptr: *const shape_value::v2::string_obj::StringObj =
                                        *(*arc.values).data.add(i);
                                    Arc::new(
                                        shape_value::v2::string_obj::StringObj::as_str(v_ptr)
                                            .to_owned(),
                                    )
                                };
                                out.push((key, val));
                            }
                            out
                        }
                        other => panic!(
                            "FromSlot<Vec<(Arc<String>, Arc<String>)>>: HashMap V \
                             variant {:?} not supported — marshal contract requires \
                             V=String. ADR-006 §2.7.24 Q25.B SUPERSEDED.",
                            other.values_kind()
                        ),
                    }
                }
                other => panic!(
                    "FromSlot<Vec<(Arc<String>, Arc<String>)>>: slot bits decoded to \
                     HeapValue::{:?}, not HashMap. Marshal kind contract violated by caller.",
                    other.kind()
                ),
            }
        }
    }
}

/// Read a `Vec<(Arc<String>, Arc<HeapValue>)>` from a
/// `NativeKind::Ptr(HeapKind::HashMap)` slot.
///
/// Body-type pattern: bodies declaring
/// `args: Vec<(Arc<String>, Arc<HeapValue>)>` receive an owned pair-list
/// with insertion order preserved and polymorphic-typed values. Each
/// element is an opaque `Arc<HeapValue>`; the body is responsible for
/// pattern-matching the inner kind per the option ε contract. No
/// element-kind constraint at the marshal boundary.
impl FromSlot for Vec<(Arc<String>, Arc<shape_value::heap_value::HeapValue>)> {
    const NATIVE_KIND: NativeKind =
        NativeKind::Ptr(shape_value::HeapKind::HashMap);
    #[inline]
    fn from_slot(bits: u64) -> Self {
        let ptr = bits as *const shape_value::HeapValue;
        // SAFETY: see Vec<u8>::from_slot above.
        unsafe {
            Arc::increment_strong_count(ptr);
            let arc_hv = Arc::from_raw(ptr);
            match &*arc_hv {
                shape_value::HeapValue::HashMap(kref) => {
                    // Wave 2 Round 3b C2-joint ckpt-4 (2026-05-14): per-V
                    // walk → `Vec<(Arc<String>, Arc<HeapValue>)>` for the
                    // polymorphic-valued marshal path. Each per-V slot
                    // projects into the canonical `Arc<HeapValue>` arm.
                    use shape_value::heap_value::{HashMapKindedRef, HeapValue};
                    let n = kref.len();
                    let mut out: Vec<(Arc<String>, Arc<HeapValue>)> = Vec::with_capacity(n);
                    let keys_ptr = match kref {
                        HashMapKindedRef::I64(arc) => arc.keys,
                        HashMapKindedRef::F64(arc) => arc.keys,
                        HashMapKindedRef::Bool(arc) => arc.keys,
                        HashMapKindedRef::Char(arc) => arc.keys,
                        HashMapKindedRef::String(arc) => arc.keys,
                        HashMapKindedRef::Decimal(arc) => arc.keys,
                        HashMapKindedRef::TypedObject(arc) => arc.keys,
                        HashMapKindedRef::TraitObject(arc) => arc.keys,
                        HashMapKindedRef::HashMap(arc) => arc.keys,
                    };
                    for i in 0..n {
                        let key = unsafe {
                            let ptr = shape_value::v2::typed_array::TypedArray::get_unchecked(
                                keys_ptr, i as u32,
                            );
                            Arc::new(
                                shape_value::v2::string_obj::StringObj::as_str(ptr)
                                    .to_owned(),
                            )
                        };
                        let value: Arc<HeapValue> = match kref {
                            HashMapKindedRef::I64(arc) => {
                                let v: i64 = unsafe { *(*arc.values).data.add(i) };
                                Arc::new(HeapValue::BigInt(Arc::new(v)))
                            }
                            HashMapKindedRef::F64(_) => {
                                panic!(
                                    "FromSlot<Vec<(Arc<String>, Arc<HeapValue>)>>: \
                                     HashMap<string, number> has no canonical \
                                     HeapValue arm (number is inline-scalar). \
                                     Marshal contract violation."
                                );
                            }
                            HashMapKindedRef::Bool(_) => {
                                panic!(
                                    "FromSlot<Vec<(Arc<String>, Arc<HeapValue>)>>: \
                                     HashMap<string, bool> has no canonical \
                                     HeapValue arm (bool is inline-scalar). \
                                     Marshal contract violation."
                                );
                            }
                            HashMapKindedRef::Char(arc) => {
                                let v: char = unsafe { *(*arc.values).data.add(i) };
                                Arc::new(HeapValue::Char(v))
                            }
                            HashMapKindedRef::String(arc) => {
                                let ptr: *const shape_value::v2::string_obj::StringObj =
                                    unsafe { *(*arc.values).data.add(i) };
                                let s = unsafe {
                                    shape_value::v2::string_obj::StringObj::as_str(ptr)
                                        .to_owned()
                                };
                                Arc::new(HeapValue::String(Arc::new(s)))
                            }
                            HashMapKindedRef::Decimal(arc) => {
                                let ptr: *const shape_value::v2::decimal_obj::DecimalObj =
                                    unsafe { *(*arc.values).data.add(i) };
                                let d = unsafe { (*ptr).value };
                                Arc::new(HeapValue::Decimal(Arc::new(d)))
                            }
                            HashMapKindedRef::TypedObject(arc) => {
                                let elem: &shape_value::heap_value::TypedObjectPtr =
                                    unsafe { &*(*arc.values).data.add(i) };
                                Arc::new(HeapValue::TypedObject(elem.clone()))
                            }
                            HashMapKindedRef::TraitObject(_) => {
                                panic!(
                                    "FromSlot<Vec<(Arc<String>, Arc<HeapValue>)>>: \
                                     HashMap<string, TraitObject> marshal not yet \
                                     wired (HeapValue::TraitObject arm exists but \
                                     payload kind dispatch is its own cluster)."
                                );
                            }
                            HashMapKindedRef::HashMap(arc) => {
                                // Recursive carrier (Wave N hashmap-value-v-arm
                                // follow-up, cluster-2 closure-wave-C,
                                // 2026-05-16). Each inner element is itself a
                                // HashMapKindedRef; wrap as a fresh
                                // HeapValue::HashMap. The inner Arc is
                                // share-cloned (Arc::clone on
                                // HashMapKindedRef::clone — single refcount
                                // bump on the inner Arc<HashMapData<V_inner>>).
                                // Per outer `unsafe` block at line 655; no
                                // inner unsafe wrapper needed.
                                let inner_ref: &HashMapKindedRef =
                                    &*(*arc.values).data.add(i);
                                Arc::new(HeapValue::HashMap(inner_ref.clone()))
                            }
                        };
                        out.push((key, value));
                    }
                    out
                }
                other => panic!(
                    "FromSlot<Vec<(Arc<String>, Arc<HeapValue>)>>: slot bits decoded to \
                     HeapValue::{:?}, not HashMap. Marshal kind contract violated by caller.",
                    other.kind()
                ),
            }
        }
    }
}

// ────── typed-array Arc<Vec<T>> FromSlot/ToSlot (Migration shape (a)) ──────
//
// V3-S5 ckpt-5-prime²c (2026-05-15) — supervisor 2026-05-15 Migration shape
// (a) RATIFIED. The prior `Arc<AlignedTypedBuffer>` / `Arc<TypedBuffer<i64>>`
// / `Arc<TypedBuffer<u8>>` zero-copy section is rewritten on the new
// `*mut TypedArray<T>` flat-struct carrier shape. The pre-migration
// per-storage-variant body-type map:
//
//   TypedArrayData::F64 ↔ Arc<AlignedTypedBuffer>   → Arc<Vec<f64>>
//   TypedArrayData::I64 ↔ Arc<TypedBuffer<i64>>     → Arc<Vec<i64>>
//   TypedArrayData::U8  ↔ Arc<TypedBuffer<u8>>      → Arc<Vec<u8>>
//
// `NATIVE_KIND` stays `Ptr(HeapKind::TypedArray)` for all three — the
// element-width discriminator lives in the body-side Rust type (option ε
// pattern), not in slot bits or `NativeKind`. Each `from_slot` impl reads
// the slot's raw `*mut TypedArray<T>`, materializes a `Vec<T>` by copying
// from `TypedArray::as_slice`, and wraps in `Arc::new`. The body accesses
// `&[T]` via `Arc::deref` → `Vec<T>`'s `Deref<Target=[T]>` impl — same
// API surface as the prior `Arc<AlignedTypedBuffer>` / `Arc<TypedBuffer<T>>`
// for the 39 migrated intrinsics in ckpt-5-prime²b (zero body adaptation).
//
// Owns-clone semantics (full element copy at the marshal boundary): a
// later wave can revisit zero-copy by switching the body parameter type
// to `*const TypedArray<T>` and exposing `TypedArray::<T>::as_slice` to
// stdlib bodies; deferred per `docs/defections.md` zero-copy follow-on
// subsection (now-superseded — the v2-raw flat-struct carrier means
// AlignedVec SIMD-alignment is at the v2/typed_array level itself, not
// at a wrapper).

/// Read an `Arc<Vec<f64>>` from a `NativeKind::Ptr(HeapKind::TypedArray)`
/// slot whose payload is `*mut TypedArray<f64>`. V3-S5 ckpt-5-prime²c
/// Migration shape (a) — replaces the pre-migration
/// `FromSlot for Arc<AlignedTypedBuffer>` entry.
impl FromSlot for Arc<Vec<f64>> {
    const NATIVE_KIND: NativeKind =
        NativeKind::Ptr(shape_value::HeapKind::TypedArray);
    #[inline]
    fn from_slot(bits: u64) -> Self {
        // SAFETY: NATIVE_KIND::Ptr(HeapKind::TypedArray) + body-declared
        // element type Arc<Vec<f64>> pins the slot bits to a live
        // *mut TypedArray<f64>.
        let arr = bits as usize as *const shape_value::v2::typed_array::TypedArray<f64>;
        if arr.is_null() {
            return Arc::new(Vec::new());
        }
        unsafe {
            Arc::new(
                shape_value::v2::typed_array::TypedArray::<f64>::as_slice(arr).to_vec(),
            )
        }
    }
}

/// Project an `Arc<Vec<f64>>` into a `NativeKind::Ptr(HeapKind::TypedArray)`
/// slot whose payload is `*mut TypedArray<f64>`. V3-S5 ckpt-5-prime²c
/// Migration shape (a) — replaces the pre-migration
/// `ToSlot for Arc<AlignedTypedBuffer>` entry.
impl ToSlot for Arc<Vec<f64>> {
    const NATIVE_KIND: NativeKind =
        NativeKind::Ptr(shape_value::HeapKind::TypedArray);
    #[inline]
    fn to_slot(self) -> u64 {
        let arr = shape_value::v2::typed_array::TypedArray::<f64>::from_slice(self.as_slice());
        arr as usize as u64
    }
}

/// Read an `Arc<Vec<i64>>` from a `NativeKind::Ptr(HeapKind::TypedArray)`
/// slot whose payload is `*mut TypedArray<i64>`. V3-S5 ckpt-5-prime²c
/// Migration shape (a) — replaces the pre-migration
/// `FromSlot for Arc<TypedBuffer<i64>>` entry.
impl FromSlot for Arc<Vec<i64>> {
    const NATIVE_KIND: NativeKind =
        NativeKind::Ptr(shape_value::HeapKind::TypedArray);
    #[inline]
    fn from_slot(bits: u64) -> Self {
        // SAFETY: see Arc<Vec<f64>>::from_slot above.
        let arr = bits as usize as *const shape_value::v2::typed_array::TypedArray<i64>;
        if arr.is_null() {
            return Arc::new(Vec::new());
        }
        unsafe {
            Arc::new(
                shape_value::v2::typed_array::TypedArray::<i64>::as_slice(arr).to_vec(),
            )
        }
    }
}

/// Project an `Arc<Vec<i64>>` into a `NativeKind::Ptr(HeapKind::TypedArray)`
/// slot whose payload is `*mut TypedArray<i64>`. V3-S5 ckpt-5-prime²c
/// Migration shape (a).
impl ToSlot for Arc<Vec<i64>> {
    const NATIVE_KIND: NativeKind =
        NativeKind::Ptr(shape_value::HeapKind::TypedArray);
    #[inline]
    fn to_slot(self) -> u64 {
        let arr = shape_value::v2::typed_array::TypedArray::<i64>::from_slice(self.as_slice());
        arr as usize as u64
    }
}

/// Read an `Arc<Vec<u8>>` from a `NativeKind::Ptr(HeapKind::TypedArray)`
/// slot whose payload is `*mut TypedArray<u8>`. V3-S5 ckpt-5-prime²c
/// Migration shape (a) — replaces the pre-migration
/// `FromSlot for Arc<TypedBuffer<u8>>` entry.
///
/// Note: the pre-migration Bool-vs-U8 Rust-type-collision residual carries
/// forward — `Array<bool>` lowers to `*mut TypedArray<u8>` per the v2
/// carrier shape (bool is stored as u8 with the dispatch-level Bool stamp
/// at `stamp_elem_type`). A body declaring `Arc<Vec<u8>>` and being handed
/// an `Array<bool>` slot will read raw bytes correctly but cannot
/// distinguish "Array<u8>" from "Array<bool>" at this boundary. Resolution
/// when a Bool consumer surfaces.
impl FromSlot for Arc<Vec<u8>> {
    const NATIVE_KIND: NativeKind =
        NativeKind::Ptr(shape_value::HeapKind::TypedArray);
    #[inline]
    fn from_slot(bits: u64) -> Self {
        // SAFETY: see Arc<Vec<f64>>::from_slot above.
        let arr = bits as usize as *const shape_value::v2::typed_array::TypedArray<u8>;
        if arr.is_null() {
            return Arc::new(Vec::new());
        }
        unsafe {
            Arc::new(
                shape_value::v2::typed_array::TypedArray::<u8>::as_slice(arr).to_vec(),
            )
        }
    }
}

/// Project an `Arc<Vec<u8>>` into a `NativeKind::Ptr(HeapKind::TypedArray)`
/// slot whose payload is `*mut TypedArray<u8>`. V3-S5 ckpt-5-prime²c
/// Migration shape (a).
impl ToSlot for Arc<Vec<u8>> {
    const NATIVE_KIND: NativeKind =
        NativeKind::Ptr(shape_value::HeapKind::TypedArray);
    #[inline]
    fn to_slot(self) -> u64 {
        let arr = shape_value::v2::typed_array::TypedArray::<u8>::from_slice(self.as_slice());
        arr as usize as u64
    }
}

// ─────────────────────── per-arity register helpers ───────────────────────

/// Body type stored in the typed registry: takes raw `&[u64]` slots and
/// returns a [`TypedReturn`]. Constructed only by the typed
/// `register_typed_fn_N` helpers, which type-check the body's actual
/// Rust signature against `FromSlot` for each arg.
type TypedInvoke = Arc<
    dyn for<'ctx> Fn(&[u64], &ModuleContext<'ctx>) -> Result<TypedReturn, String>
        + Send
        + Sync,
>;

/// Register a 0-arg native function whose body takes only the
/// `ModuleContext` and returns a [`TypedReturn`].
pub fn register_typed_fn_0<F>(
    module: &mut crate::module_exports::ModuleExports,
    name: impl Into<String>,
    description: impl Into<String>,
    return_type: crate::typed_module_exports::ConcreteType,
    body: F,
) where
    F: for<'ctx> Fn(&ModuleContext<'ctx>) -> Result<TypedReturn, String>
        + Send
        + Sync
        + 'static,
{
    let invoke: TypedInvoke = Arc::new(move |slots, ctx| {
        if !slots.is_empty() {
            return Err(MarshalError::ArgCount {
                expected: 0,
                got: slots.len(),
            }
            .into());
        }
        body(ctx)
    });
    install(module, name, description, vec![], return_type, vec![], invoke);
}

/// Register a 1-arg native function. The body's `P0` parameter type
/// declares the typed contract via [`FromSlot::NATIVE_KIND`] — there is
/// no separate kind annotation to keep in sync.
pub fn register_typed_fn_1<F, P0>(
    module: &mut crate::module_exports::ModuleExports,
    name: impl Into<String>,
    description: impl Into<String>,
    param_name: impl Into<String>,
    param_type_name: impl Into<String>,
    return_type: crate::typed_module_exports::ConcreteType,
    body: F,
) where
    F: for<'ctx> Fn(P0, &ModuleContext<'ctx>) -> Result<TypedReturn, String>
        + Send
        + Sync
        + 'static,
    P0: FromSlot + Send + Sync + 'static,
{
    let arg_kinds = vec![P0::NATIVE_KIND];
    let invoke: TypedInvoke = Arc::new(move |slots, ctx| {
        if slots.len() != 1 {
            return Err(MarshalError::ArgCount {
                expected: 1,
                got: slots.len(),
            }
            .into());
        }
        let p0 = P0::from_slot(slots[0]);
        body(p0, ctx)
    });
    let params = vec![crate::module_exports::ModuleParam {
        name: param_name.into(),
        type_name: param_type_name.into(),
        required: true,
        ..Default::default()
    }];
    install(
        module,
        name,
        description,
        params,
        return_type,
        arg_kinds,
        invoke,
    );
}

/// Register a 2-arg native function.
pub fn register_typed_fn_2<F, P0, P1>(
    module: &mut crate::module_exports::ModuleExports,
    name: impl Into<String>,
    description: impl Into<String>,
    param_names: [(&str, &str); 2],
    return_type: crate::typed_module_exports::ConcreteType,
    body: F,
) where
    F: for<'ctx> Fn(P0, P1, &ModuleContext<'ctx>) -> Result<TypedReturn, String>
        + Send
        + Sync
        + 'static,
    P0: FromSlot + Send + Sync + 'static,
    P1: FromSlot + Send + Sync + 'static,
{
    let arg_kinds = vec![P0::NATIVE_KIND, P1::NATIVE_KIND];
    let invoke: TypedInvoke = Arc::new(move |slots, ctx| {
        if slots.len() != 2 {
            return Err(MarshalError::ArgCount {
                expected: 2,
                got: slots.len(),
            }
            .into());
        }
        let p0 = P0::from_slot(slots[0]);
        let p1 = P1::from_slot(slots[1]);
        body(p0, p1, ctx)
    });
    let params = param_names
        .iter()
        .map(|(name, ty)| crate::module_exports::ModuleParam {
            name: (*name).to_string(),
            type_name: (*ty).to_string(),
            required: true,
            ..Default::default()
        })
        .collect();
    install(
        module,
        name,
        description,
        params,
        return_type,
        arg_kinds,
        invoke,
    );
}

/// Register a 3-arg native function.
pub fn register_typed_fn_3<F, P0, P1, P2>(
    module: &mut crate::module_exports::ModuleExports,
    name: impl Into<String>,
    description: impl Into<String>,
    param_names: [(&str, &str); 3],
    return_type: crate::typed_module_exports::ConcreteType,
    body: F,
) where
    F: for<'ctx> Fn(P0, P1, P2, &ModuleContext<'ctx>) -> Result<TypedReturn, String>
        + Send
        + Sync
        + 'static,
    P0: FromSlot + Send + Sync + 'static,
    P1: FromSlot + Send + Sync + 'static,
    P2: FromSlot + Send + Sync + 'static,
{
    let arg_kinds = vec![P0::NATIVE_KIND, P1::NATIVE_KIND, P2::NATIVE_KIND];
    let invoke: TypedInvoke = Arc::new(move |slots, ctx| {
        if slots.len() != 3 {
            return Err(MarshalError::ArgCount {
                expected: 3,
                got: slots.len(),
            }
            .into());
        }
        let p0 = P0::from_slot(slots[0]);
        let p1 = P1::from_slot(slots[1]);
        let p2 = P2::from_slot(slots[2]);
        body(p0, p1, p2, ctx)
    });
    let params = param_names
        .iter()
        .map(|(name, ty)| crate::module_exports::ModuleParam {
            name: (*name).to_string(),
            type_name: (*ty).to_string(),
            required: true,
            ..Default::default()
        })
        .collect();
    install(
        module,
        name,
        description,
        params,
        return_type,
        arg_kinds,
        invoke,
    );
}

// ─────────────── per-arity `_full` register helpers (optional-arg) ───────────
//
// Mirror `register_typed_fn_N` but take `[ModuleParam; N]` directly instead
// of `[(&str, &str); N]`. This lets per-param `required: bool` and
// `default_snippet: Option<String>` flow through to the schema-introspection
// layer and the compiler-side default-arg insertion path
// (`crates/shape-vm/src/compiler/functions_foreign.rs:433`,
// `statements.rs:540`). Bodies stay typed — the dispatcher always sees N
// typed args because the compiler synthesizes any missing trailing optional
// before emitting the call.
//
// On-record marshal-API extension per `docs/defections.md` 2026-05-06
// `marshal-optional-args`. Considered + rejected: option 2 (sentinel values
// inline — W-series shape at marshal-API level) and option 3 (defer with
// user-facing Shape signature regression on canonical I/O).

/// Register a 1-arg native function with full param spec (per-arg
/// `required` + `default_snippet`).
pub fn register_typed_fn_1_full<F, P0>(
    module: &mut crate::module_exports::ModuleExports,
    name: impl Into<String>,
    description: impl Into<String>,
    params: [crate::module_exports::ModuleParam; 1],
    return_type: crate::typed_module_exports::ConcreteType,
    body: F,
) where
    F: for<'ctx> Fn(P0, &ModuleContext<'ctx>) -> Result<TypedReturn, String>
        + Send
        + Sync
        + 'static,
    P0: FromSlot + Send + Sync + 'static,
{
    let arg_kinds = vec![P0::NATIVE_KIND];
    let invoke: TypedInvoke = Arc::new(move |slots, ctx| {
        if slots.len() != 1 {
            return Err(MarshalError::ArgCount {
                expected: 1,
                got: slots.len(),
            }
            .into());
        }
        let p0 = P0::from_slot(slots[0]);
        body(p0, ctx)
    });
    install(
        module,
        name,
        description,
        params.into_iter().collect(),
        return_type,
        arg_kinds,
        invoke,
    );
}

/// Register a 2-arg native function with full param spec.
pub fn register_typed_fn_2_full<F, P0, P1>(
    module: &mut crate::module_exports::ModuleExports,
    name: impl Into<String>,
    description: impl Into<String>,
    params: [crate::module_exports::ModuleParam; 2],
    return_type: crate::typed_module_exports::ConcreteType,
    body: F,
) where
    F: for<'ctx> Fn(P0, P1, &ModuleContext<'ctx>) -> Result<TypedReturn, String>
        + Send
        + Sync
        + 'static,
    P0: FromSlot + Send + Sync + 'static,
    P1: FromSlot + Send + Sync + 'static,
{
    let arg_kinds = vec![P0::NATIVE_KIND, P1::NATIVE_KIND];
    let invoke: TypedInvoke = Arc::new(move |slots, ctx| {
        if slots.len() != 2 {
            return Err(MarshalError::ArgCount {
                expected: 2,
                got: slots.len(),
            }
            .into());
        }
        let p0 = P0::from_slot(slots[0]);
        let p1 = P1::from_slot(slots[1]);
        body(p0, p1, ctx)
    });
    install(
        module,
        name,
        description,
        params.into_iter().collect(),
        return_type,
        arg_kinds,
        invoke,
    );
}

/// Register a 3-arg native function with full param spec.
pub fn register_typed_fn_3_full<F, P0, P1, P2>(
    module: &mut crate::module_exports::ModuleExports,
    name: impl Into<String>,
    description: impl Into<String>,
    params: [crate::module_exports::ModuleParam; 3],
    return_type: crate::typed_module_exports::ConcreteType,
    body: F,
) where
    F: for<'ctx> Fn(P0, P1, P2, &ModuleContext<'ctx>) -> Result<TypedReturn, String>
        + Send
        + Sync
        + 'static,
    P0: FromSlot + Send + Sync + 'static,
    P1: FromSlot + Send + Sync + 'static,
    P2: FromSlot + Send + Sync + 'static,
{
    let arg_kinds = vec![P0::NATIVE_KIND, P1::NATIVE_KIND, P2::NATIVE_KIND];
    let invoke: TypedInvoke = Arc::new(move |slots, ctx| {
        if slots.len() != 3 {
            return Err(MarshalError::ArgCount {
                expected: 3,
                got: slots.len(),
            }
            .into());
        }
        let p0 = P0::from_slot(slots[0]);
        let p1 = P1::from_slot(slots[1]);
        let p2 = P2::from_slot(slots[2]);
        body(p0, p1, p2, ctx)
    });
    install(
        module,
        name,
        description,
        params.into_iter().collect(),
        return_type,
        arg_kinds,
        invoke,
    );
}

// ─────────────── per-arity register helpers — arities 4/5/6 (N2 extension) ──
//
// Per-arity parallel-impl extension to support intrinsics with > 3 typed args.
// Mechanical mirror of arities 0..3 above; no new architectural surface — no
// dyn, no parametric NativeKind, no rename-to-less-suspicious-name. Same
// per-arity pattern as `marshal-optional-args`'s `_full` extension precedent.
//
// On-record marshal-API extension per `docs/defections.md` 2026-05-07
// intrinsics-typed-CC entry's N2 sub-decision queue subsection (queue
// item #6, supervisor sign-off relayed via team-lead). Sync-only at first
// landing per consumer pattern (stochastic gbm/ou_process synchronous);
// async _N variants deferred until consumer-driven need.

/// Register a 4-arg native function with positional `(name, type)` param spec.
pub fn register_typed_fn_4<F, P0, P1, P2, P3>(
    module: &mut crate::module_exports::ModuleExports,
    name: impl Into<String>,
    description: impl Into<String>,
    param_names: [(&str, &str); 4],
    return_type: crate::typed_module_exports::ConcreteType,
    body: F,
) where
    F: for<'ctx> Fn(P0, P1, P2, P3, &ModuleContext<'ctx>) -> Result<TypedReturn, String>
        + Send
        + Sync
        + 'static,
    P0: FromSlot + Send + Sync + 'static,
    P1: FromSlot + Send + Sync + 'static,
    P2: FromSlot + Send + Sync + 'static,
    P3: FromSlot + Send + Sync + 'static,
{
    let arg_kinds = vec![
        P0::NATIVE_KIND,
        P1::NATIVE_KIND,
        P2::NATIVE_KIND,
        P3::NATIVE_KIND,
    ];
    let invoke: TypedInvoke = Arc::new(move |slots, ctx| {
        if slots.len() != 4 {
            return Err(MarshalError::ArgCount {
                expected: 4,
                got: slots.len(),
            }
            .into());
        }
        let p0 = P0::from_slot(slots[0]);
        let p1 = P1::from_slot(slots[1]);
        let p2 = P2::from_slot(slots[2]);
        let p3 = P3::from_slot(slots[3]);
        body(p0, p1, p2, p3, ctx)
    });
    let params = param_names
        .iter()
        .map(|(name, ty)| crate::module_exports::ModuleParam {
            name: (*name).to_string(),
            type_name: (*ty).to_string(),
            required: true,
            ..Default::default()
        })
        .collect();
    install(
        module,
        name,
        description,
        params,
        return_type,
        arg_kinds,
        invoke,
    );
}

/// Register a 5-arg native function with positional `(name, type)` param spec.
pub fn register_typed_fn_5<F, P0, P1, P2, P3, P4>(
    module: &mut crate::module_exports::ModuleExports,
    name: impl Into<String>,
    description: impl Into<String>,
    param_names: [(&str, &str); 5],
    return_type: crate::typed_module_exports::ConcreteType,
    body: F,
) where
    F: for<'ctx> Fn(P0, P1, P2, P3, P4, &ModuleContext<'ctx>) -> Result<TypedReturn, String>
        + Send
        + Sync
        + 'static,
    P0: FromSlot + Send + Sync + 'static,
    P1: FromSlot + Send + Sync + 'static,
    P2: FromSlot + Send + Sync + 'static,
    P3: FromSlot + Send + Sync + 'static,
    P4: FromSlot + Send + Sync + 'static,
{
    let arg_kinds = vec![
        P0::NATIVE_KIND,
        P1::NATIVE_KIND,
        P2::NATIVE_KIND,
        P3::NATIVE_KIND,
        P4::NATIVE_KIND,
    ];
    let invoke: TypedInvoke = Arc::new(move |slots, ctx| {
        if slots.len() != 5 {
            return Err(MarshalError::ArgCount {
                expected: 5,
                got: slots.len(),
            }
            .into());
        }
        let p0 = P0::from_slot(slots[0]);
        let p1 = P1::from_slot(slots[1]);
        let p2 = P2::from_slot(slots[2]);
        let p3 = P3::from_slot(slots[3]);
        let p4 = P4::from_slot(slots[4]);
        body(p0, p1, p2, p3, p4, ctx)
    });
    let params = param_names
        .iter()
        .map(|(name, ty)| crate::module_exports::ModuleParam {
            name: (*name).to_string(),
            type_name: (*ty).to_string(),
            required: true,
            ..Default::default()
        })
        .collect();
    install(
        module,
        name,
        description,
        params,
        return_type,
        arg_kinds,
        invoke,
    );
}

/// Register a 6-arg native function with positional `(name, type)` param spec.
pub fn register_typed_fn_6<F, P0, P1, P2, P3, P4, P5>(
    module: &mut crate::module_exports::ModuleExports,
    name: impl Into<String>,
    description: impl Into<String>,
    param_names: [(&str, &str); 6],
    return_type: crate::typed_module_exports::ConcreteType,
    body: F,
) where
    F: for<'ctx> Fn(P0, P1, P2, P3, P4, P5, &ModuleContext<'ctx>) -> Result<TypedReturn, String>
        + Send
        + Sync
        + 'static,
    P0: FromSlot + Send + Sync + 'static,
    P1: FromSlot + Send + Sync + 'static,
    P2: FromSlot + Send + Sync + 'static,
    P3: FromSlot + Send + Sync + 'static,
    P4: FromSlot + Send + Sync + 'static,
    P5: FromSlot + Send + Sync + 'static,
{
    let arg_kinds = vec![
        P0::NATIVE_KIND,
        P1::NATIVE_KIND,
        P2::NATIVE_KIND,
        P3::NATIVE_KIND,
        P4::NATIVE_KIND,
        P5::NATIVE_KIND,
    ];
    let invoke: TypedInvoke = Arc::new(move |slots, ctx| {
        if slots.len() != 6 {
            return Err(MarshalError::ArgCount {
                expected: 6,
                got: slots.len(),
            }
            .into());
        }
        let p0 = P0::from_slot(slots[0]);
        let p1 = P1::from_slot(slots[1]);
        let p2 = P2::from_slot(slots[2]);
        let p3 = P3::from_slot(slots[3]);
        let p4 = P4::from_slot(slots[4]);
        let p5 = P5::from_slot(slots[5]);
        body(p0, p1, p2, p3, p4, p5, ctx)
    });
    let params = param_names
        .iter()
        .map(|(name, ty)| crate::module_exports::ModuleParam {
            name: (*name).to_string(),
            type_name: (*ty).to_string(),
            required: true,
            ..Default::default()
        })
        .collect();
    install(
        module,
        name,
        description,
        params,
        return_type,
        arg_kinds,
        invoke,
    );
}

/// Register a 4-arg native function with full param spec.
pub fn register_typed_fn_4_full<F, P0, P1, P2, P3>(
    module: &mut crate::module_exports::ModuleExports,
    name: impl Into<String>,
    description: impl Into<String>,
    params: [crate::module_exports::ModuleParam; 4],
    return_type: crate::typed_module_exports::ConcreteType,
    body: F,
) where
    F: for<'ctx> Fn(P0, P1, P2, P3, &ModuleContext<'ctx>) -> Result<TypedReturn, String>
        + Send
        + Sync
        + 'static,
    P0: FromSlot + Send + Sync + 'static,
    P1: FromSlot + Send + Sync + 'static,
    P2: FromSlot + Send + Sync + 'static,
    P3: FromSlot + Send + Sync + 'static,
{
    let arg_kinds = vec![
        P0::NATIVE_KIND,
        P1::NATIVE_KIND,
        P2::NATIVE_KIND,
        P3::NATIVE_KIND,
    ];
    let invoke: TypedInvoke = Arc::new(move |slots, ctx| {
        if slots.len() != 4 {
            return Err(MarshalError::ArgCount {
                expected: 4,
                got: slots.len(),
            }
            .into());
        }
        let p0 = P0::from_slot(slots[0]);
        let p1 = P1::from_slot(slots[1]);
        let p2 = P2::from_slot(slots[2]);
        let p3 = P3::from_slot(slots[3]);
        body(p0, p1, p2, p3, ctx)
    });
    install(
        module,
        name,
        description,
        params.into_iter().collect(),
        return_type,
        arg_kinds,
        invoke,
    );
}

/// Register a 5-arg native function with full param spec.
pub fn register_typed_fn_5_full<F, P0, P1, P2, P3, P4>(
    module: &mut crate::module_exports::ModuleExports,
    name: impl Into<String>,
    description: impl Into<String>,
    params: [crate::module_exports::ModuleParam; 5],
    return_type: crate::typed_module_exports::ConcreteType,
    body: F,
) where
    F: for<'ctx> Fn(P0, P1, P2, P3, P4, &ModuleContext<'ctx>) -> Result<TypedReturn, String>
        + Send
        + Sync
        + 'static,
    P0: FromSlot + Send + Sync + 'static,
    P1: FromSlot + Send + Sync + 'static,
    P2: FromSlot + Send + Sync + 'static,
    P3: FromSlot + Send + Sync + 'static,
    P4: FromSlot + Send + Sync + 'static,
{
    let arg_kinds = vec![
        P0::NATIVE_KIND,
        P1::NATIVE_KIND,
        P2::NATIVE_KIND,
        P3::NATIVE_KIND,
        P4::NATIVE_KIND,
    ];
    let invoke: TypedInvoke = Arc::new(move |slots, ctx| {
        if slots.len() != 5 {
            return Err(MarshalError::ArgCount {
                expected: 5,
                got: slots.len(),
            }
            .into());
        }
        let p0 = P0::from_slot(slots[0]);
        let p1 = P1::from_slot(slots[1]);
        let p2 = P2::from_slot(slots[2]);
        let p3 = P3::from_slot(slots[3]);
        let p4 = P4::from_slot(slots[4]);
        body(p0, p1, p2, p3, p4, ctx)
    });
    install(
        module,
        name,
        description,
        params.into_iter().collect(),
        return_type,
        arg_kinds,
        invoke,
    );
}

/// Register a 6-arg native function with full param spec.
pub fn register_typed_fn_6_full<F, P0, P1, P2, P3, P4, P5>(
    module: &mut crate::module_exports::ModuleExports,
    name: impl Into<String>,
    description: impl Into<String>,
    params: [crate::module_exports::ModuleParam; 6],
    return_type: crate::typed_module_exports::ConcreteType,
    body: F,
) where
    F: for<'ctx> Fn(P0, P1, P2, P3, P4, P5, &ModuleContext<'ctx>) -> Result<TypedReturn, String>
        + Send
        + Sync
        + 'static,
    P0: FromSlot + Send + Sync + 'static,
    P1: FromSlot + Send + Sync + 'static,
    P2: FromSlot + Send + Sync + 'static,
    P3: FromSlot + Send + Sync + 'static,
    P4: FromSlot + Send + Sync + 'static,
    P5: FromSlot + Send + Sync + 'static,
{
    let arg_kinds = vec![
        P0::NATIVE_KIND,
        P1::NATIVE_KIND,
        P2::NATIVE_KIND,
        P3::NATIVE_KIND,
        P4::NATIVE_KIND,
        P5::NATIVE_KIND,
    ];
    let invoke: TypedInvoke = Arc::new(move |slots, ctx| {
        if slots.len() != 6 {
            return Err(MarshalError::ArgCount {
                expected: 6,
                got: slots.len(),
            }
            .into());
        }
        let p0 = P0::from_slot(slots[0]);
        let p1 = P1::from_slot(slots[1]);
        let p2 = P2::from_slot(slots[2]);
        let p3 = P3::from_slot(slots[3]);
        let p4 = P4::from_slot(slots[4]);
        let p5 = P5::from_slot(slots[5]);
        body(p0, p1, p2, p3, p4, p5, ctx)
    });
    install(
        module,
        name,
        description,
        params.into_iter().collect(),
        return_type,
        arg_kinds,
        invoke,
    );
}

/// Internal helper: install a fully-prepared typed function entry into a
/// module's typed registry plus its schema-only entry.
fn install(
    module: &mut crate::module_exports::ModuleExports,
    name: impl Into<String>,
    description: impl Into<String>,
    params: Vec<crate::module_exports::ModuleParam>,
    return_type: crate::typed_module_exports::ConcreteType,
    arg_kinds: Vec<NativeKind>,
    invoke: TypedInvoke,
) {
    use crate::module_exports::ModuleFunction;
    use crate::typed_module_exports::TypedModuleFunction;

    let name = name.into();
    let arg_types: Vec<String> = params.iter().map(|p| p.type_name.clone()).collect();
    let return_type_str = return_type.shape_type_name();
    module.add_schema_only(
        name.clone(),
        ModuleFunction {
            description: description.into(),
            params,
            return_type: Some(return_type_str),
        },
    );
    module.typed_exports_mut().functions.insert(
        name,
        TypedModuleFunction {
            invoke,
            return_type,
            arg_types,
            arg_kinds,
        },
    );
}

// ─────────────────────── async per-arity register helpers ───────────────────────
//
// Async typed registration mirrors the sync `register_typed_fn_N` family
// with two structural differences enforced by the existing
// `TypedModuleAsyncFunction` shape (see `typed_module_exports.rs`):
//
// 1. **No `&ModuleContext`.** `ModuleContext` borrows from the VM and
//    cannot cross await points. Permission gating must happen
//    synchronously upstream of the dispatch site, not inside the async
//    body. (This matches the pre-bulldozer convention used by
//    `stdlib_io::async_file_ops` and `stdlib::http`.)
// 2. **Body returns `Future + Send + 'static`.** The wrapper boxes and
//    pins the future so the synchronous dispatch path can block on it.
//
// No new architectural decisions — the `TypedModuleAsyncFunction`
// struct is the contract; these helpers are the per-arity adapters.

type TypedAsyncInvoke = Arc<
    dyn Fn(
            Vec<u64>,
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<TypedReturn, String>> + Send>>
        + Send
        + Sync,
>;

/// Register a 1-arg async native function. Body returns a `Future`; the
/// dispatcher blocks on it at the call boundary.
pub fn register_typed_async_fn_1<F, Fut, P0>(
    module: &mut crate::module_exports::ModuleExports,
    name: impl Into<String>,
    description: impl Into<String>,
    param_name: impl Into<String>,
    param_type_name: impl Into<String>,
    return_type: crate::typed_module_exports::ConcreteType,
    body: F,
) where
    F: Fn(P0) -> Fut + Send + Sync + Clone + 'static,
    Fut: std::future::Future<Output = Result<TypedReturn, String>> + Send + 'static,
    P0: FromSlot + Send + Sync + 'static,
{
    let arg_kinds = vec![P0::NATIVE_KIND];
    let invoke: TypedAsyncInvoke = Arc::new(move |slots: Vec<u64>| {
        if slots.len() != 1 {
            let err = MarshalError::ArgCount {
                expected: 1,
                got: slots.len(),
            };
            return Box::pin(async move { Err(err.into()) });
        }
        let p0 = P0::from_slot(slots[0]);
        let body = body.clone();
        Box::pin(async move { body(p0).await })
    });
    let params = vec![crate::module_exports::ModuleParam {
        name: param_name.into(),
        type_name: param_type_name.into(),
        required: true,
        ..Default::default()
    }];
    install_async(module, name, description, params, return_type, arg_kinds, invoke);
}

/// Register a 2-arg async native function.
pub fn register_typed_async_fn_2<F, Fut, P0, P1>(
    module: &mut crate::module_exports::ModuleExports,
    name: impl Into<String>,
    description: impl Into<String>,
    param_names: [(&str, &str); 2],
    return_type: crate::typed_module_exports::ConcreteType,
    body: F,
) where
    F: Fn(P0, P1) -> Fut + Send + Sync + Clone + 'static,
    Fut: std::future::Future<Output = Result<TypedReturn, String>> + Send + 'static,
    P0: FromSlot + Send + Sync + 'static,
    P1: FromSlot + Send + Sync + 'static,
{
    let arg_kinds = vec![P0::NATIVE_KIND, P1::NATIVE_KIND];
    let invoke: TypedAsyncInvoke = Arc::new(move |slots: Vec<u64>| {
        if slots.len() != 2 {
            let err = MarshalError::ArgCount {
                expected: 2,
                got: slots.len(),
            };
            return Box::pin(async move { Err(err.into()) });
        }
        let p0 = P0::from_slot(slots[0]);
        let p1 = P1::from_slot(slots[1]);
        let body = body.clone();
        Box::pin(async move { body(p0, p1).await })
    });
    let params = param_names
        .iter()
        .map(|(name, ty)| crate::module_exports::ModuleParam {
            name: (*name).to_string(),
            type_name: (*ty).to_string(),
            required: true,
            ..Default::default()
        })
        .collect();
    install_async(module, name, description, params, return_type, arg_kinds, invoke);
}

/// Register a 3-arg async native function.
pub fn register_typed_async_fn_3<F, Fut, P0, P1, P2>(
    module: &mut crate::module_exports::ModuleExports,
    name: impl Into<String>,
    description: impl Into<String>,
    param_names: [(&str, &str); 3],
    return_type: crate::typed_module_exports::ConcreteType,
    body: F,
) where
    F: Fn(P0, P1, P2) -> Fut + Send + Sync + Clone + 'static,
    Fut: std::future::Future<Output = Result<TypedReturn, String>> + Send + 'static,
    P0: FromSlot + Send + Sync + 'static,
    P1: FromSlot + Send + Sync + 'static,
    P2: FromSlot + Send + Sync + 'static,
{
    let arg_kinds = vec![P0::NATIVE_KIND, P1::NATIVE_KIND, P2::NATIVE_KIND];
    let invoke: TypedAsyncInvoke = Arc::new(move |slots: Vec<u64>| {
        if slots.len() != 3 {
            let err = MarshalError::ArgCount {
                expected: 3,
                got: slots.len(),
            };
            return Box::pin(async move { Err(err.into()) });
        }
        let p0 = P0::from_slot(slots[0]);
        let p1 = P1::from_slot(slots[1]);
        let p2 = P2::from_slot(slots[2]);
        let body = body.clone();
        Box::pin(async move { body(p0, p1, p2).await })
    });
    let params = param_names
        .iter()
        .map(|(name, ty)| crate::module_exports::ModuleParam {
            name: (*name).to_string(),
            type_name: (*ty).to_string(),
            required: true,
            ..Default::default()
        })
        .collect();
    install_async(module, name, description, params, return_type, arg_kinds, invoke);
}

// ──────────── async per-arity `_full` register helpers (optional-arg) ────────
//
// Mirror the sync `_full` family for async. See the sync block above for
// rationale (`docs/defections.md` 2026-05-06 `marshal-optional-args`).

/// Register a 1-arg async native function with full param spec.
pub fn register_typed_async_fn_1_full<F, Fut, P0>(
    module: &mut crate::module_exports::ModuleExports,
    name: impl Into<String>,
    description: impl Into<String>,
    params: [crate::module_exports::ModuleParam; 1],
    return_type: crate::typed_module_exports::ConcreteType,
    body: F,
) where
    F: Fn(P0) -> Fut + Send + Sync + Clone + 'static,
    Fut: std::future::Future<Output = Result<TypedReturn, String>> + Send + 'static,
    P0: FromSlot + Send + Sync + 'static,
{
    let arg_kinds = vec![P0::NATIVE_KIND];
    let invoke: TypedAsyncInvoke = Arc::new(move |slots: Vec<u64>| {
        if slots.len() != 1 {
            let err = MarshalError::ArgCount {
                expected: 1,
                got: slots.len(),
            };
            return Box::pin(async move { Err(err.into()) });
        }
        let p0 = P0::from_slot(slots[0]);
        let body = body.clone();
        Box::pin(async move { body(p0).await })
    });
    install_async(
        module,
        name,
        description,
        params.into_iter().collect(),
        return_type,
        arg_kinds,
        invoke,
    );
}

/// Register a 2-arg async native function with full param spec.
pub fn register_typed_async_fn_2_full<F, Fut, P0, P1>(
    module: &mut crate::module_exports::ModuleExports,
    name: impl Into<String>,
    description: impl Into<String>,
    params: [crate::module_exports::ModuleParam; 2],
    return_type: crate::typed_module_exports::ConcreteType,
    body: F,
) where
    F: Fn(P0, P1) -> Fut + Send + Sync + Clone + 'static,
    Fut: std::future::Future<Output = Result<TypedReturn, String>> + Send + 'static,
    P0: FromSlot + Send + Sync + 'static,
    P1: FromSlot + Send + Sync + 'static,
{
    let arg_kinds = vec![P0::NATIVE_KIND, P1::NATIVE_KIND];
    let invoke: TypedAsyncInvoke = Arc::new(move |slots: Vec<u64>| {
        if slots.len() != 2 {
            let err = MarshalError::ArgCount {
                expected: 2,
                got: slots.len(),
            };
            return Box::pin(async move { Err(err.into()) });
        }
        let p0 = P0::from_slot(slots[0]);
        let p1 = P1::from_slot(slots[1]);
        let body = body.clone();
        Box::pin(async move { body(p0, p1).await })
    });
    install_async(
        module,
        name,
        description,
        params.into_iter().collect(),
        return_type,
        arg_kinds,
        invoke,
    );
}

/// Register a 3-arg async native function with full param spec.
pub fn register_typed_async_fn_3_full<F, Fut, P0, P1, P2>(
    module: &mut crate::module_exports::ModuleExports,
    name: impl Into<String>,
    description: impl Into<String>,
    params: [crate::module_exports::ModuleParam; 3],
    return_type: crate::typed_module_exports::ConcreteType,
    body: F,
) where
    F: Fn(P0, P1, P2) -> Fut + Send + Sync + Clone + 'static,
    Fut: std::future::Future<Output = Result<TypedReturn, String>> + Send + 'static,
    P0: FromSlot + Send + Sync + 'static,
    P1: FromSlot + Send + Sync + 'static,
    P2: FromSlot + Send + Sync + 'static,
{
    let arg_kinds = vec![P0::NATIVE_KIND, P1::NATIVE_KIND, P2::NATIVE_KIND];
    let invoke: TypedAsyncInvoke = Arc::new(move |slots: Vec<u64>| {
        if slots.len() != 3 {
            let err = MarshalError::ArgCount {
                expected: 3,
                got: slots.len(),
            };
            return Box::pin(async move { Err(err.into()) });
        }
        let p0 = P0::from_slot(slots[0]);
        let p1 = P1::from_slot(slots[1]);
        let p2 = P2::from_slot(slots[2]);
        let body = body.clone();
        Box::pin(async move { body(p0, p1, p2).await })
    });
    install_async(
        module,
        name,
        description,
        params.into_iter().collect(),
        return_type,
        arg_kinds,
        invoke,
    );
}

fn install_async(
    module: &mut crate::module_exports::ModuleExports,
    name: impl Into<String>,
    description: impl Into<String>,
    params: Vec<crate::module_exports::ModuleParam>,
    return_type: crate::typed_module_exports::ConcreteType,
    arg_kinds: Vec<NativeKind>,
    invoke: TypedAsyncInvoke,
) {
    use crate::module_exports::ModuleFunction;
    use crate::typed_module_exports::TypedModuleAsyncFunction;

    let name = name.into();
    let arg_types: Vec<String> = params.iter().map(|p| p.type_name.clone()).collect();
    let return_type_str = return_type.shape_type_name();
    module.add_schema_only(
        name.clone(),
        ModuleFunction {
            description: description.into(),
            params,
            return_type: Some(return_type_str),
        },
    );
    module.typed_exports_mut().async_functions.insert(
        name,
        TypedModuleAsyncFunction {
            invoke,
            return_type,
            arg_types,
            arg_kinds,
        },
    );
}

// ─────────────────── variadic register helpers (ADR-006 §2.7.4) ───────────────────
//
// Per ADR-006 §2.7.4 (stdlib registration ruling), the variadic
// `register_typed_function` / `register_typed_async_function` helpers
// are re-introduced at the [`KindedSlot`] shape. Per-arity helpers
// remain the preferred path when the function arity is fixed; the
// variadic helpers exist for the genuine §2.7.1.4 dispatch-slice case
// (functions with optional / variadic arguments — json/msgpack/toml/
// yaml/stdlib_time bodies that take optional `pretty?: bool`,
// `iterations?: int`, etc.).
//
// The variadic body signature is
// `Fn(&[KindedSlot], &ModuleContext) -> Result<TypedReturn, String>`,
// matching the §2.7.1.4 dispatch-slice contract. The `arg_kinds` field
// of [`TypedModuleFunction`] is left as a per-param-position table
// derived from the registered `ModuleParam` slice (each slot is
// declared `NativeKind::Bool` placeholder for the variadic case;
// dispatch reads bits and bundles them as `KindedSlot` carriers
// regardless of the placeholder kind, since the body interprets the
// slots itself per its variadic contract).

use crate::typed_module_exports::TypedModuleFunction;
use shape_value::KindedSlot;

/// Body signature for a [`register_typed_function`] caller.
///
/// Variadic — the body inspects the slot slice itself rather than
/// declaring a per-arg type at registration. Used by stdlib functions
/// with optional / overload-shaped arguments (json.stringify's optional
/// `pretty`, time.benchmark's optional `iterations`, etc.). For
/// fixed-arity functions, prefer [`register_typed_fn_N`].
pub type VariadicTypedBody = dyn for<'ctx> Fn(
        &[KindedSlot],
        &ModuleContext<'ctx>,
    ) -> Result<TypedReturn, String>
    + Send
    + Sync;

/// Register a native function whose body inspects a variadic
/// [`KindedSlot`] slice.
///
/// Per ADR-006 §2.7.4 ruling, the variadic helper is the §2.7.1.4
/// dispatch-slice case — `KindedSlot` is the right carrier because the
/// kind-per-position is determined by the registered `ModuleParam`
/// schema, not by `FromSlot` constraints on the body's Rust signature.
/// Conversion from raw `&[u64]` to `&[KindedSlot]` happens inside the
/// runtime-side wrapper installed below; the body sees the typed
/// carrier directly.
pub fn register_typed_function<F>(
    module: &mut crate::module_exports::ModuleExports,
    name: impl Into<String>,
    description: impl Into<String>,
    params: Vec<crate::module_exports::ModuleParam>,
    return_type: crate::typed_module_exports::ConcreteType,
    body: F,
) where
    F: for<'ctx> Fn(&[KindedSlot], &ModuleContext<'ctx>) -> Result<TypedReturn, String>
        + Send
        + Sync
        + 'static,
{
    use crate::module_exports::ModuleFunction;

    let name = name.into();
    let arg_types: Vec<String> = params.iter().map(|p| p.type_name.clone()).collect();
    // Variadic registration: `arg_kinds` is a placeholder schema. The
    // dispatcher constructs `KindedSlot`s by pairing each slot with the
    // declared `NativeKind` from the typed registry — for variadic
    // bodies the kind-per-position is the body's contract, not the
    // dispatcher's. Phase 2c wires per-position `NativeKind` derivation
    // from the schema annotations.
    let arg_kinds: Vec<NativeKind> = params.iter().map(|_| NativeKind::Bool).collect();
    let return_type_str = return_type.shape_type_name();

    let body = Arc::new(body);
    let invoke: TypedInvoke = Arc::new(move |slots, ctx| {
        // Phase 1.B variadic shim: read each raw u64 slot as a
        // placeholder `KindedSlot::Bool`. The body is responsible for
        // interpreting the slot bits per its own contract (which is the
        // pre-bulldozer behaviour — variadic bodies always inspected
        // their args). Phase 2c lands proper per-position kind
        // threading from the registered schema.
        let kinded: Vec<KindedSlot> = slots
            .iter()
            .map(|&bits| {
                KindedSlot::new(
                    shape_value::ValueSlot::from_raw(bits),
                    NativeKind::Bool,
                )
            })
            .collect();
        body(&kinded, ctx)
    });

    module.add_schema_only(
        name.clone(),
        ModuleFunction {
            description: description.into(),
            params,
            return_type: Some(return_type_str),
        },
    );
    module.typed_exports_mut().functions.insert(
        name,
        TypedModuleFunction {
            invoke,
            return_type,
            arg_types,
            arg_kinds,
        },
    );
}

/// Body signature for a [`register_typed_async_function`] caller.
///
/// Variadic — same shape as [`VariadicTypedBody`] but returning a
/// `Future`. No `&ModuleContext` (the borrow cannot cross await
/// points); permission gating must happen synchronously upstream.
pub type VariadicTypedAsyncBody<Fut> =
    dyn Fn(Vec<KindedSlot>) -> Fut + Send + Sync;

/// Register an async native function whose body inspects a variadic
/// [`KindedSlot`] vector.
pub fn register_typed_async_function<F, Fut>(
    module: &mut crate::module_exports::ModuleExports,
    name: impl Into<String>,
    description: impl Into<String>,
    params: Vec<crate::module_exports::ModuleParam>,
    return_type: crate::typed_module_exports::ConcreteType,
    body: F,
) where
    F: Fn(Vec<KindedSlot>) -> Fut + Send + Sync + Clone + 'static,
    Fut: std::future::Future<Output = Result<TypedReturn, String>> + Send + 'static,
{
    use crate::module_exports::ModuleFunction;
    use crate::typed_module_exports::TypedModuleAsyncFunction;

    let name = name.into();
    let arg_types: Vec<String> = params.iter().map(|p| p.type_name.clone()).collect();
    let arg_kinds: Vec<NativeKind> = params.iter().map(|_| NativeKind::Bool).collect();
    let return_type_str = return_type.shape_type_name();

    let invoke: TypedAsyncInvoke = Arc::new(move |slots: Vec<u64>| {
        let kinded: Vec<KindedSlot> = slots
            .into_iter()
            .map(|bits| {
                KindedSlot::new(
                    shape_value::ValueSlot::from_raw(bits),
                    NativeKind::Bool,
                )
            })
            .collect();
        let body = body.clone();
        Box::pin(async move { body(kinded).await })
    });

    module.add_schema_only(
        name.clone(),
        ModuleFunction {
            description: description.into(),
            params,
            return_type: Some(return_type_str),
        },
    );
    module.typed_exports_mut().async_functions.insert(
        name,
        TypedModuleAsyncFunction {
            invoke,
            return_type,
            arg_types,
            arg_kinds,
        },
    );
}
