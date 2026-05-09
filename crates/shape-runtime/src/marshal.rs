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
// `Array<int>` / `Array<u8>` slots come in as `Arc<HeapValue>` pointing at
// `HeapValue::TypedArray(TypedArrayData::*)`. The element-width discrimination
// is via the body's declared parameter type (`Vec<u8>` vs `Vec<i64>` vs …),
// not via NativeKind: `NATIVE_KIND` stays `Ptr(HeapKind::TypedArray)` for all
// element widths. Element-width threading is enforced by the Rust type
// system at the impl level, with an in-body pattern match that panics on
// mismatch.
//
// Per `docs/runtime-v2-spec.md`: "the kind tells you the arm; HeapValue
// dispatch is a consistency check, not a probe." The `_ => panic!()` arm
// in each `from_slot` is `debug_assert!`-equivalent — unreachable in a
// well-typed system. The dispatcher's registration-time arg-kind contract
// already verified the slot bits decode to a `HeapKind::TypedArray` heap
// pointer; the variant pattern match verifies the body's declared element
// type matches the actual `TypedArrayData::*` arm. If this panics in
// production, a compiler/dispatcher contract has been violated, not a
// user-facing condition.
//
// Option β chosen over option α (`Arc<TypedBuffer<T>>` zero-copy), option ε
// (FromSlot variants per element type with consistency checks), option γ
// (`&[T]` borrowed), and path 2 (per-element `HeapKind` split). See
// `docs/defections.md` 2026-05-06 cluster #3 entry for the trade-off
// discussion. Adjacent deferred follow-up workstreams: `maximalist-v2-redesign`
// (dissolve `HeapValue` sum type at discriminator level) and
// `move-semantics-marshal` (leverage the existing `LoadLocalMove`/
// `LoadLocalClone` bytecode opcodes at the FFI boundary instead of always-
// clone). Both are on-record DEFERRED in the same defections.md page.

/// Read a `Vec<u8>` from a `NativeKind::Ptr(HeapKind::TypedArray)` slot
/// whose payload is `TypedArrayData::U8`.
///
/// Owns-clone semantics: the body receives an owned `Vec<u8>` independent
/// of the slot's strong reference. The slot's lifetime is unaffected.
///
/// Panics on `TypedArrayData::*` mismatch. The dispatcher's registration
/// contract guarantees a `HeapKind::TypedArray` slot; the body's declared
/// `Vec<u8>` parameter type pins the expected `TypedArrayData::U8` arm.
/// Per spec, this is a consistency check, not a runtime probe.
impl FromSlot for Vec<u8> {
    const NATIVE_KIND: NativeKind =
        NativeKind::Ptr(shape_value::HeapKind::TypedArray);
    #[inline]
    fn from_slot(bits: u64) -> Self {
        let ptr = bits as *const shape_value::HeapValue;
        // SAFETY: NATIVE_KIND::Ptr(HeapKind::TypedArray) pins the bits to
        // an Arc<HeapValue> with the TypedArray variant. We clone the
        // inner buffer's data into an owned Vec without consuming the
        // slot's strong ref.
        unsafe {
            Arc::increment_strong_count(ptr);
            let arc_hv = Arc::from_raw(ptr);
            match &*arc_hv {
                shape_value::HeapValue::TypedArray(arc) => match &**arc {
                    shape_value::TypedArrayData::U8(buf) => buf.data.clone(),
                    other => panic!(
                        "FromSlot<Vec<u8>>: slot bits decoded to HeapValue::TypedArray::{}, \
                         not U8. Body's parameter type Vec<u8> requires the U8 element-width \
                         variant. Marshal kind contract violated by caller (compiler/dispatcher \
                         bug, not a user-facing condition).",
                        other.type_name()
                    ),
                },
                other => panic!(
                    "FromSlot<Vec<u8>>: slot bits decoded to HeapValue::{:?}, \
                     not TypedArray. Marshal kind contract violated by caller.",
                    other.kind()
                ),
            }
        }
    }
}

/// Read a `Vec<i64>` from a `NativeKind::Ptr(HeapKind::TypedArray)` slot
/// whose payload is `TypedArrayData::I64`.
///
/// Owns-clone semantics. Panics on `TypedArrayData::*` mismatch — same
/// consistency-check rationale as `Vec<u8>` above.
impl FromSlot for Vec<i64> {
    const NATIVE_KIND: NativeKind =
        NativeKind::Ptr(shape_value::HeapKind::TypedArray);
    #[inline]
    fn from_slot(bits: u64) -> Self {
        let ptr = bits as *const shape_value::HeapValue;
        // SAFETY: see Vec<u8>::from_slot above.
        unsafe {
            Arc::increment_strong_count(ptr);
            let arc_hv = Arc::from_raw(ptr);
            match &*arc_hv {
                shape_value::HeapValue::TypedArray(arc) => match &**arc {
                    shape_value::TypedArrayData::I64(buf) => buf.data.clone(),
                    other => panic!(
                        "FromSlot<Vec<i64>>: slot bits decoded to HeapValue::TypedArray::{}, \
                         not I64. Body's parameter type Vec<i64> requires the I64 element-width \
                         variant. Marshal kind contract violated by caller.",
                        other.type_name()
                    ),
                },
                other => panic!(
                    "FromSlot<Vec<i64>>: slot bits decoded to HeapValue::{:?}, \
                     not TypedArray. Marshal kind contract violated by caller.",
                    other.kind()
                ),
            }
        }
    }
}

/// Read a `Vec<Arc<String>>` from a `NativeKind::Ptr(HeapKind::TypedArray)` slot
/// whose payload is `TypedArrayData::String`.
///
/// Phase 2d Array cluster (2026-05-07). Owns-clone semantics: the body
/// receives an owned `Vec<Arc<String>>` whose Arcs are individually
/// retained from the inner buffer. Panics on `TypedArrayData::*`
/// mismatch — same consistency-check rationale as `Vec<u8>` above.
impl FromSlot for Vec<Arc<String>> {
    const NATIVE_KIND: NativeKind =
        NativeKind::Ptr(shape_value::HeapKind::TypedArray);
    #[inline]
    fn from_slot(bits: u64) -> Self {
        let ptr = bits as *const shape_value::HeapValue;
        // SAFETY: see Vec<u8>::from_slot above.
        unsafe {
            Arc::increment_strong_count(ptr);
            let arc_hv = Arc::from_raw(ptr);
            match &*arc_hv {
                shape_value::HeapValue::TypedArray(arc) => match &**arc {
                    shape_value::TypedArrayData::String(buf) => buf.data.clone(),
                    other => panic!(
                        "FromSlot<Vec<Arc<String>>>: slot bits decoded to HeapValue::TypedArray::{}, \
                         not String. Body's parameter type Vec<Arc<String>> requires the String \
                         element-width variant. Marshal kind contract violated by caller.",
                        other.type_name()
                    ),
                },
                other => panic!(
                    "FromSlot<Vec<Arc<String>>>: slot bits decoded to HeapValue::{:?}, \
                     not TypedArray. Marshal kind contract violated by caller.",
                    other.kind()
                ),
            }
        }
    }
}

/// Read a `Vec<Arc<HeapValue>>` from a `NativeKind::Ptr(HeapKind::TypedArray)`
/// slot whose payload is `TypedArrayData::HeapValue`.
///
/// Phase 2d Array cluster (2026-05-07). Each element is an opaque
/// `Arc<HeapValue>` whose inner kind is a body-side type contract.
/// E.g. an `Array<DataTable>` body uses this `FromSlot` and then
/// pattern-matches each element's `HeapValue::DataTable` arm —
/// homogeneity is enforced by the body, not by the marshal layer.
///
/// Panics on `TypedArrayData::*` mismatch — same consistency-check
/// rationale as `Vec<u8>` above.
impl FromSlot for Vec<Arc<shape_value::heap_value::HeapValue>> {
    const NATIVE_KIND: NativeKind =
        NativeKind::Ptr(shape_value::HeapKind::TypedArray);
    #[inline]
    fn from_slot(bits: u64) -> Self {
        let ptr = bits as *const shape_value::HeapValue;
        // SAFETY: see Vec<u8>::from_slot above.
        unsafe {
            Arc::increment_strong_count(ptr);
            let arc_hv = Arc::from_raw(ptr);
            match &*arc_hv {
                shape_value::HeapValue::TypedArray(arc) => match &**arc {
                    shape_value::TypedArrayData::HeapValue(buf) => buf.data.clone(),
                    other => panic!(
                        "FromSlot<Vec<Arc<HeapValue>>>: slot bits decoded to HeapValue::TypedArray::{}, \
                         not HeapValue. Body's parameter type Vec<Arc<HeapValue>> requires the \
                         HeapValue element-width variant. Marshal kind contract violated by caller.",
                        other.type_name()
                    ),
                },
                other => panic!(
                    "FromSlot<Vec<Arc<HeapValue>>>: slot bits decoded to HeapValue::{:?}, \
                     not TypedArray. Marshal kind contract violated by caller.",
                    other.kind()
                ),
            }
        }
    }
}

/// Project a `Vec<Arc<String>>` into a `NativeKind::Ptr(HeapKind::TypedArray)`
/// slot whose payload is `TypedArrayData::String`.
///
/// Phase 2d Array cluster (2026-05-07). Used by the dispatcher's
/// `ConcreteReturn::ArrayString → slot push` step (the body returns
/// `Vec<String>`; the dispatcher wraps each into `Arc<String>` then
/// hands the resulting `Vec` to this impl). Yields raw bits suitable
/// for placement into a typed slot — the slot takes ownership of the
/// resulting `Arc<HeapValue>` strong reference.
impl ToSlot for Vec<Arc<String>> {
    const NATIVE_KIND: NativeKind =
        NativeKind::Ptr(shape_value::HeapKind::TypedArray);
    #[inline]
    fn to_slot(self) -> u64 {
        let buf = shape_value::TypedBuffer::from_vec(self);
        let data = Arc::new(shape_value::TypedArrayData::String(Arc::new(buf)));
        let hv = shape_value::HeapValue::TypedArray(data);
        Arc::into_raw(Arc::new(hv)) as u64
    }
}

/// Project a `Vec<Arc<HeapValue>>` into a `NativeKind::Ptr(HeapKind::TypedArray)`
/// slot whose payload is `TypedArrayData::HeapValue`.
///
/// Phase 2d Array cluster (2026-05-07). Used by the dispatcher's
/// `ConcreteReturn::ArrayHeapValue → slot push` step. Element-kind
/// homogeneity is the body's responsibility.
impl ToSlot for Vec<Arc<shape_value::heap_value::HeapValue>> {
    const NATIVE_KIND: NativeKind =
        NativeKind::Ptr(shape_value::HeapKind::TypedArray);
    #[inline]
    fn to_slot(self) -> u64 {
        let buf = shape_value::TypedBuffer::from_vec(self);
        let data = Arc::new(shape_value::TypedArrayData::HeapValue(Arc::new(buf)));
        let hv = shape_value::HeapValue::TypedArray(data);
        Arc::into_raw(Arc::new(hv)) as u64
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
                shape_value::HeapValue::HashMap(d) => {
                    d.keys
                        .data
                        .iter()
                        .zip(d.values.data.iter())
                        .map(|(k, v)| match &**v {
                            shape_value::HeapValue::String(s) => (Arc::clone(k), Arc::clone(s)),
                            other => panic!(
                                "FromSlot<Vec<(Arc<String>, Arc<String>)>>: HashMap value at \
                                 key '{}' is HeapValue::{:?}, not String. Body's parameter \
                                 type Vec<(Arc<String>, Arc<String>)> requires string-typed \
                                 values. Marshal kind contract violated by caller.",
                                k,
                                other.kind()
                            ),
                        })
                        .collect()
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
                shape_value::HeapValue::HashMap(d) => d
                    .keys
                    .data
                    .iter()
                    .zip(d.values.data.iter())
                    .map(|(k, v)| (Arc::clone(k), Arc::clone(v)))
                    .collect(),
                other => panic!(
                    "FromSlot<Vec<(Arc<String>, Arc<HeapValue>)>>: slot bits decoded to \
                     HeapValue::{:?}, not HashMap. Marshal kind contract violated by caller.",
                    other.kind()
                ),
            }
        }
    }
}

// ────── typed-array Arc<TypedBuffer<T>> zero-copy FromSlot/ToSlot (option α + ε) ──────
//
// Stage B (`docs/defections.md` 2026-05-07 — Arc<TypedBuffer<T>> zero-copy
// named cluster, lines 226-371; refined by the dated correction
// subsection — per-storage-variant body-type map). The α + ε zero-copy
// pattern lands alongside β: each impl pins a single `TypedArrayData::*`
// storage variant via the body's declared parameter type, and the
// `from_slot` path performs **one `Arc::clone` (atomic refcount op) and
// zero data copy** rather than β's owned-clone (full element-by-element
// copy of the inner buffer). β stays in production parallel for existing
// consumers (compress / archive / byte_utils / csv / etc.) that don't
// benefit from zero-copy.
//
// Per-storage-variant body-type map (per the 2026-05-07 dated correction
// subsection):
//
//   TypedArrayData::F64 ↔ Arc<AlignedTypedBuffer>      (this section)
//   TypedArrayData::I64 ↔ Arc<TypedBuffer<i64>>        (this section)
//   TypedArrayData::U8  ↔ Arc<TypedBuffer<u8>>         (this section)
//   TypedArrayData::Bool deferred — Rust-type-collision with U8;
//                                   resolution comes when a Bool consumer
//                                   surfaces, likely via a newtype wrapper.
//   I32 / Matrix / others — deferred per consumer-driven scope.
//
// `NATIVE_KIND` stays `Ptr(HeapKind::TypedArray)` for all three — the
// element-width discriminator lives in the body-side Rust type (ε
// pattern), not in slot bits or `NativeKind`. Same consistency-check
// residual as β: the in-body pattern match panics on the wrong
// `TypedArrayData::*` arm. Per `docs/runtime-v2-spec.md`, this is a
// `debug_assert!`-equivalent — unreachable in a well-typed system.
//
// f64 uses `Arc<AlignedTypedBuffer>` rather than `Arc<TypedBuffer<f64>>`
// because `TypedArrayData::F64` stores the SIMD-aligned `AlignedVec<f64>`
// shape (`heap_value.rs:482`, `typed_buffer.rs:230`). This asymmetric
// body-type map is the established codebase pattern (mirrors shape-vm's
// `extract_float_array` / `extract_int_array` helpers in
// `crates/shape-vm/src/executor/objects/typed_array_methods.rs:19,26`).
// See the 2026-05-07 dated correction subsection of the zero-copy entry
// for A1/A2/A3 surfacing, A4-A7 supervisor-checked alternatives, and
// the structural rationale.

/// Read an `Arc<AlignedTypedBuffer>` from a `NativeKind::Ptr(HeapKind::TypedArray)`
/// slot whose payload is `TypedArrayData::F64`.
///
/// Zero-copy semantics: the body receives a strong reference cloned from
/// the inner `Arc<AlignedTypedBuffer>` of the heap value's TypedArray
/// payload — one `Arc::clone` (single atomic refcount op) and **zero
/// data copy**. The body accesses the f64 buffer via
/// `Arc::deref` → `&[f64]` (`AlignedTypedBuffer`'s `Deref<Target=[f64]>` impl).
///
/// Panics on `TypedArrayData::*` mismatch — same consistency-check
/// rationale as the β `Vec<u8>` impl above.
impl FromSlot for Arc<shape_value::AlignedTypedBuffer> {
    const NATIVE_KIND: NativeKind =
        NativeKind::Ptr(shape_value::HeapKind::TypedArray);
    #[inline]
    fn from_slot(bits: u64) -> Self {
        let ptr = bits as *const shape_value::HeapValue;
        // SAFETY: see Vec<u8>::from_slot above. Marshal kind contract
        // pins this slot to an Arc<HeapValue> with the TypedArray
        // variant; this impl pins the F64 storage arm via the body's
        // declared Arc<AlignedTypedBuffer> parameter type.
        unsafe {
            Arc::increment_strong_count(ptr);
            let arc_hv = Arc::from_raw(ptr);
            match &*arc_hv {
                shape_value::HeapValue::TypedArray(arc) => match &**arc {
                    shape_value::TypedArrayData::F64(buf) => Arc::clone(buf),
                    other => panic!(
                        "FromSlot<Arc<AlignedTypedBuffer>>: slot bits decoded to \
                         HeapValue::TypedArray::{}, not F64. Body's parameter type \
                         Arc<AlignedTypedBuffer> requires the F64 storage variant. \
                         Marshal kind contract violated by caller (compiler/dispatcher \
                         bug, not a user-facing condition).",
                        other.type_name()
                    ),
                },
                other => panic!(
                    "FromSlot<Arc<AlignedTypedBuffer>>: slot bits decoded to \
                     HeapValue::{:?}, not TypedArray. Marshal kind contract \
                     violated by caller.",
                    other.kind()
                ),
            }
        }
    }
}

/// Project an `Arc<AlignedTypedBuffer>` into a `NativeKind::Ptr(HeapKind::TypedArray)`
/// slot whose payload is `TypedArrayData::F64`.
///
/// The body's `Arc<AlignedTypedBuffer>` is already wrapped — the only
/// allocations are constructing `HeapValue::TypedArray(TypedArrayData::F64(self))`
/// and `Arc::new(hv)` for the slot's outer `Arc<HeapValue>`. **No
/// element copy.**
impl ToSlot for Arc<shape_value::AlignedTypedBuffer> {
    const NATIVE_KIND: NativeKind =
        NativeKind::Ptr(shape_value::HeapKind::TypedArray);
    #[inline]
    fn to_slot(self) -> u64 {
        let data = Arc::new(shape_value::TypedArrayData::F64(self));
        let hv = shape_value::HeapValue::TypedArray(data);
        Arc::into_raw(Arc::new(hv)) as u64
    }
}

/// Read an `Arc<TypedBuffer<i64>>` from a `NativeKind::Ptr(HeapKind::TypedArray)`
/// slot whose payload is `TypedArrayData::I64`.
///
/// Zero-copy semantics: see `Arc<AlignedTypedBuffer>::from_slot` above.
/// Body accesses `&[i64]` via `Arc::deref` → `TypedBuffer<i64>`'s
/// `Deref<Target=[i64]>` impl.
impl FromSlot for Arc<shape_value::TypedBuffer<i64>> {
    const NATIVE_KIND: NativeKind =
        NativeKind::Ptr(shape_value::HeapKind::TypedArray);
    #[inline]
    fn from_slot(bits: u64) -> Self {
        let ptr = bits as *const shape_value::HeapValue;
        // SAFETY: see Vec<u8>::from_slot above.
        unsafe {
            Arc::increment_strong_count(ptr);
            let arc_hv = Arc::from_raw(ptr);
            match &*arc_hv {
                shape_value::HeapValue::TypedArray(arc) => match &**arc {
                    shape_value::TypedArrayData::I64(buf) => Arc::clone(buf),
                    other => panic!(
                        "FromSlot<Arc<TypedBuffer<i64>>>: slot bits decoded to \
                         HeapValue::TypedArray::{}, not I64. Body's parameter type \
                         Arc<TypedBuffer<i64>> requires the I64 storage variant. \
                         Marshal kind contract violated by caller.",
                        other.type_name()
                    ),
                },
                other => panic!(
                    "FromSlot<Arc<TypedBuffer<i64>>>: slot bits decoded to \
                     HeapValue::{:?}, not TypedArray. Marshal kind contract \
                     violated by caller.",
                    other.kind()
                ),
            }
        }
    }
}

/// Project an `Arc<TypedBuffer<i64>>` into a `NativeKind::Ptr(HeapKind::TypedArray)`
/// slot whose payload is `TypedArrayData::I64`. **No element copy.**
impl ToSlot for Arc<shape_value::TypedBuffer<i64>> {
    const NATIVE_KIND: NativeKind =
        NativeKind::Ptr(shape_value::HeapKind::TypedArray);
    #[inline]
    fn to_slot(self) -> u64 {
        let data = Arc::new(shape_value::TypedArrayData::I64(self));
        let hv = shape_value::HeapValue::TypedArray(data);
        Arc::into_raw(Arc::new(hv)) as u64
    }
}

/// Read an `Arc<TypedBuffer<u8>>` from a `NativeKind::Ptr(HeapKind::TypedArray)`
/// slot whose payload is `TypedArrayData::U8`.
///
/// Zero-copy semantics: see `Arc<AlignedTypedBuffer>::from_slot` above.
/// Body accesses `&[u8]` via `Arc::deref`.
///
/// Note: `TypedArrayData::Bool` also stores `Arc<TypedBuffer<u8>>`; this
/// impl matches `TypedArrayData::U8` only. Bool consumer projection is
/// deferred per the 2026-05-07 dated correction subsection (Rust-type-
/// collision with U8 — body type alone cannot disambiguate; resolution
/// comes when a Bool consumer surfaces, likely via a newtype wrapper).
/// A Bool slot reaching this impl is a marshal-kind-contract violation.
impl FromSlot for Arc<shape_value::TypedBuffer<u8>> {
    const NATIVE_KIND: NativeKind =
        NativeKind::Ptr(shape_value::HeapKind::TypedArray);
    #[inline]
    fn from_slot(bits: u64) -> Self {
        let ptr = bits as *const shape_value::HeapValue;
        // SAFETY: see Vec<u8>::from_slot above.
        unsafe {
            Arc::increment_strong_count(ptr);
            let arc_hv = Arc::from_raw(ptr);
            match &*arc_hv {
                shape_value::HeapValue::TypedArray(arc) => match &**arc {
                    shape_value::TypedArrayData::U8(buf) => Arc::clone(buf),
                    other => panic!(
                        "FromSlot<Arc<TypedBuffer<u8>>>: slot bits decoded to \
                         HeapValue::TypedArray::{}, not U8. Body's parameter type \
                         Arc<TypedBuffer<u8>> requires the U8 storage variant \
                         (Bool deferred — see zero-copy entry's 2026-05-07 dated \
                         correction subsection). Marshal kind contract violated \
                         by caller.",
                        other.type_name()
                    ),
                },
                other => panic!(
                    "FromSlot<Arc<TypedBuffer<u8>>>: slot bits decoded to \
                     HeapValue::{:?}, not TypedArray. Marshal kind contract \
                     violated by caller.",
                    other.kind()
                ),
            }
        }
    }
}

/// Project an `Arc<TypedBuffer<u8>>` into a `NativeKind::Ptr(HeapKind::TypedArray)`
/// slot whose payload is `TypedArrayData::U8`. **No element copy.**
///
/// As with `FromSlot` above, this projects to the U8 storage variant
/// only; Bool projection is deferred.
impl ToSlot for Arc<shape_value::TypedBuffer<u8>> {
    const NATIVE_KIND: NativeKind =
        NativeKind::Ptr(shape_value::HeapKind::TypedArray);
    #[inline]
    fn to_slot(self) -> u64 {
        let data = Arc::new(shape_value::TypedArrayData::U8(self));
        let hv = shape_value::HeapValue::TypedArray(data);
        Arc::into_raw(Arc::new(hv)) as u64
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
