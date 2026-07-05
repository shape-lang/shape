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
use shape_value::{KindedSlot, NativeKind};
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

    /// Read from a caller-side [`KindedSlot`], checking the slot's
    /// **stamped** kind (from the VM's §2.7.7 parallel kind track — never
    /// fabricated from bits) is in this parameter's kind class before
    /// reading the carrier. This is the class-aware fixed-arity entry
    /// point (ADR-006 §2.7.5 amendment §4.2.2a/2b): the scrutinee is the
    /// stamped kind, the read is per-carrier-native, and no type
    /// information is reconstructed from untyped bits.
    ///
    /// Default impl: scalars match exactly, heap `Ptr(_)` params accept
    /// any `Ptr(_)` (the concrete `HeapValue` arm is the body's
    /// discriminator per ADR-005 §1), on match the carrier is read via
    /// [`Self::from_slot`]. `Arc<String>` (String/StringV2 carrier split)
    /// and `Option<f64>` (nullable) override this to read each carrier
    /// natively.
    fn from_kinded(slot: &KindedSlot) -> Result<Self, MarshalError> {
        let declared = Self::NATIVE_KIND;
        let actual = slot.kind();
        // Class-membership PREDICATE (not a per-HeapKind dispatch): a heap
        // (`Ptr`) parameter accepts any heap carrier — the dispatch shell
        // does NOT re-derive HeapKind granularity here (that would
        // duplicate the type system at a runtime boundary); the concrete
        // HeapKind discriminator is `slot.as_heap_value()` + `HeapValue`
        // match inside the body's `from_slot` (ADR-005 §1). Scalars are
        // exact. Spelled as a `matches!` predicate — the same non-dispatch
        // Ptr-membership form as `NativeKind::is_refcounted`, NOT a
        // `Ptr(_) =>` dispatch arm (check-heapkind-wildcards.sh CHECK 14:
        // new HeapKind variants are caught at the `as_heap_value()` /
        // `HeapValue` seam, not absorbed by a wildcard dispatch arm here).
        let in_class = if matches!(declared, NativeKind::Ptr(_)) {
            matches!(actual, NativeKind::Ptr(_))
        } else {
            actual == declared
        };
        if in_class {
            Ok(Self::from_slot(slot.raw()))
        } else {
            Err(MarshalError::KindMismatch {
                expected: declared,
                got: actual,
            })
        }
    }
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
    /// A caller-side slot's stamped `NativeKind` was not in the declared
    /// parameter's kind class (e.g. an `int` argument reaching a `string`
    /// parameter). Compared between two independently-stamped sources
    /// (caller kind track vs registration schema) — not a bit decode.
    KindMismatch {
        expected: NativeKind,
        got: NativeKind,
    },
    /// The body returned an `Err(String)` — surfaced verbatim.
    Body(String),
}

impl std::fmt::Display for MarshalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MarshalError::ArgCount { expected, got } => {
                write!(f, "expected {} arg(s), got {}", expected, got)
            }
            MarshalError::KindMismatch { expected, got } => {
                write!(
                    f,
                    "argument kind mismatch: expected {:?}, got {:?}",
                    expected, got
                )
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
        if v.is_nan() { None } else { Some(v) }
    }

    /// Nullable `number?` class (ADR-006 §4.2.2a): a stamped
    /// `NativeKind::Null` is the absence signal (kind IS the
    /// discriminator, bits ignored — R5b-2 disposition) → `None`; the
    /// `NullableFloat64` NaN-sentinel carrier reads via `from_slot`; a
    /// plain `Float64` reads as `Some`.
    #[inline]
    fn from_kinded(slot: &KindedSlot) -> Result<Self, MarshalError> {
        match slot.kind() {
            NativeKind::Null => Ok(None),
            NativeKind::NullableFloat64 => Ok(Self::from_slot(slot.raw())),
            NativeKind::Float64 => Ok(Some(f64::from_bits(slot.raw()))),
            got => Err(MarshalError::KindMismatch {
                expected: NativeKind::NullableFloat64,
                got,
            }),
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

    /// `string` carrier split (ADR-006 §4.2.2b): a `string` parameter can
    /// receive EITHER the `Arc<String>` carrier (`NativeKind::String`) or
    /// the v2-raw `*const StringObj` carrier (`NativeKind::StringV2` —
    /// e.g. an `Array<string>` element read). Each carrier is read
    /// natively — the `String` arm shares the `Arc<String>`, the
    /// `StringV2` arm reads the `StringObj` UTF-8 content (via the
    /// kind-directed `KindedSlot::as_str`) into a fresh `Arc<String>`.
    /// No structural bridging between the two carriers (the per-carrier
    /// discriminator decision is respected, not papered over).
    #[inline]
    fn from_kinded(slot: &KindedSlot) -> Result<Self, MarshalError> {
        match slot.kind() {
            NativeKind::String => Ok(Self::from_slot(slot.raw())),
            NativeKind::StringV2 => Ok(Arc::new(slot.as_str().unwrap_or("").to_string())),
            got => Err(MarshalError::KindMismatch {
                expected: NativeKind::String,
                got,
            }),
        }
    }
}

/// Read a `TypedObjectPtr` from a `Ptr(HeapKind::TypedObject)` slot.
///
/// TypedObject slots are v2-raw carriers: bits are the raw
/// `*const TypedObjectStorage`, and lifetime is governed by the embedded
/// `HeapHeader`, not by `Arc<TypedObjectStorage>`. The caller-owned slot
/// keeps its share; the body receives an independent retained wrapper.
impl FromSlot for shape_value::heap_value::TypedObjectPtr {
    const NATIVE_KIND: NativeKind = NativeKind::Ptr(shape_value::HeapKind::TypedObject);
    #[inline]
    fn from_slot(bits: u64) -> Self {
        let ptr = bits as *const shape_value::heap_value::TypedObjectStorage;
        if !ptr.is_null() {
            // SAFETY: the registered native kind pins this slot to a live
            // v2-raw TypedObjectStorage carrier with a HeapHeader at offset 0.
            unsafe { shape_value::v2::refcount::v2_retain(&(*ptr).header) };
        }
        shape_value::heap_value::TypedObjectPtr::new(ptr)
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
    const NATIVE_KIND: NativeKind = NativeKind::Ptr(shape_value::HeapKind::DataTable);
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
    const NATIVE_KIND: NativeKind = NativeKind::Ptr(shape_value::HeapKind::DataTable);
    #[inline]
    fn to_slot(self) -> u64 {
        let hv = Arc::new(shape_value::HeapValue::DataTable(self));
        Arc::into_raw(hv) as u64
    }
}

// ────────────────────── IoHandle FromSlot/ToSlot (option γ) ───────────────
//
// Cluster #2 (docs/defections.md 2026-05-06): IoHandle marshal extension
// via Arc<IoHandleData>.
//
// Strict IoHandle slots use the direct `Arc::into_raw(Arc<IoHandleData>)`
// carrier that `KindedSlot::from_io_handle` and the HeapKind::IoHandle
// clone/drop arms retain and release. Bodies declare
// `handle: Arc<IoHandleData>` and call methods on it via Arc::deref.

/// Read the direct `Arc<IoHandleData>` from a `NativeKind::Ptr(HeapKind::IoHandle)` slot.
impl FromSlot for Arc<shape_value::heap_value::IoHandleData>
where
    Self: Sized,
{
    const NATIVE_KIND: NativeKind = NativeKind::Ptr(shape_value::HeapKind::IoHandle);
    #[inline]
    fn from_slot(bits: u64) -> Self {
        let ptr = bits as *const shape_value::heap_value::IoHandleData;
        // SAFETY: KindedSlot::from_io_handle and the HeapKind::IoHandle
        // clone/drop tables store a direct Arc<IoHandleData> carrier.
        // Increment then rebuild an Arc from that retained share, leaving
        // the caller-owned slot share untouched.
        unsafe {
            Arc::increment_strong_count(ptr);
            Arc::from_raw(ptr)
        }
    }
}

/// Write an `Arc<IoHandleData>` using the strict direct-Arc IoHandle carrier.
impl ToSlot for Arc<shape_value::heap_value::IoHandleData> {
    const NATIVE_KIND: NativeKind = NativeKind::Ptr(shape_value::HeapKind::IoHandle);
    #[inline]
    fn to_slot(self) -> u64 {
        Arc::into_raw(self) as u64
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
    const NATIVE_KIND: NativeKind = NativeKind::Ptr(shape_value::HeapKind::TypedArray);
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
        unsafe { shape_value::v2::typed_array::TypedArray::<u8>::as_slice(arr).to_vec() }
    }
}

/// Read a `Vec<i64>` from a `NativeKind::Ptr(HeapKind::TypedArray)` slot
/// whose payload is `*mut TypedArray<i64>`. V3-S5 ckpt-5-prime²c
/// (2026-05-15): rewritten for the v2-raw flat-struct carrier. Owns-clone
/// semantics.
impl FromSlot for Vec<i64> {
    const NATIVE_KIND: NativeKind = NativeKind::Ptr(shape_value::HeapKind::TypedArray);
    #[inline]
    fn from_slot(bits: u64) -> Self {
        // SAFETY: see Vec<u8>::from_slot above.
        let arr = bits as usize as *const shape_value::v2::typed_array::TypedArray<i64>;
        if arr.is_null() {
            return Vec::new();
        }
        unsafe { shape_value::v2::typed_array::TypedArray::<i64>::as_slice(arr).to_vec() }
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
    const NATIVE_KIND: NativeKind = NativeKind::Ptr(shape_value::HeapKind::TypedArray);
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
                .map(|&p| Arc::new(shape_value::v2::string_obj::StringObj::as_str(p).to_owned()))
                .collect()
        }
    }
}

/// Read a `Vec<Arc<HeapValue>>` from a `NativeKind::Ptr(HeapKind::TypedArray)`
/// slot.
///
/// STAGE K2 (2026-06-02) — per-element-T marshal dispatcher over the v2-raw
/// `*mut TypedArray<T>` flat-struct carrier. The element type `T` was
/// monomorphized at compile time by the `NewTypedArray*` allocation opcodes
/// and is recorded at runtime by `stamp_elem_type` in the `_pad` byte
/// (offset 7) of the array's `HeapHeader` (`crates/shape-vm/src/executor/
/// v2_handlers/v2_array_detect.rs::stamp_elem_type`, canonical discriminants
/// at `crates/shape-value/src/v2/typed_array.rs`). This reader threads that
/// existing discriminator to the marshal boundary via the public
/// `TypedArray::read_elem_type` accessor — no new side-channel, no kind
/// fabrication, no `is_heap()` probe.
///
/// Each element projects into the canonical `Arc<HeapValue>` arm (ADR-005
/// §1 single-discriminator):
///
///   `ELEM_TYPE_CHAR`         → `HeapValue::Char(c)`
///   `ELEM_TYPE_STRING`       → `HeapValue::String(Arc::new(s.to_owned()))`
///   `ELEM_TYPE_DECIMAL`      → `HeapValue::Decimal(Arc::new(d))`
///   `ELEM_TYPE_TYPED_OBJECT` → `HeapValue::TypedObject(TypedObjectPtr::new(p))`
///                              (per-element `v2_retain` — owns-clone share)
///
/// Owns-clone semantics: the returned `Vec<Arc<HeapValue>>` is independent of
/// the source array. String/Decimal elements are deep-copied into fresh
/// `Arc<String>` / `Arc<Decimal>` payloads; each TypedObject element bumps the
/// pointed-to v2-raw HeapHeader refcount so the wrapper owns a real share.
///
/// Scalar element kinds (`F64`/`I64`/`I32`/`Bool`/sized-ints/`F32`) have **no
/// canonical `Arc<HeapValue>` arm** — `number`/`int`/`bool`/etc. are
/// inline-scalar NativeKinds, not heap values (same contract as the
/// `Vec<(Arc<String>, Arc<HeapValue>)>` HashMap reader above). Those arrays
/// marshal via the dedicated `Arc<Vec<T>>` impls; reaching this reader with a
/// scalar-stamped array is a marshal kind-contract violation by the caller and
/// SURFACEs (panic with a precise message — the established surface mechanism
/// for a `from_slot` that cannot return `Result`). Nested-array elements
/// (`Array<Array<...>>`) are slot-level carriers (`ELEM_TYPE_TYPED_ARRAY`),
/// not `HeapValue` arms; exact nested consumers use a concrete body type such
/// as `Vec<Vec<Arc<String>>>`.
impl FromSlot for Vec<Arc<shape_value::heap_value::HeapValue>> {
    const NATIVE_KIND: NativeKind = NativeKind::Ptr(shape_value::HeapKind::TypedArray);
    #[inline]
    fn from_slot(bits: u64) -> Self {
        use shape_value::heap_value::{HeapValue, TypedObjectPtr, TypedObjectStorage};
        use shape_value::v2::decimal_obj::DecimalObj;
        use shape_value::v2::refcount::v2_retain;
        use shape_value::v2::string_obj::StringObj;
        use shape_value::v2::typed_array::{
            ELEM_TYPE_CHAR, ELEM_TYPE_DECIMAL, ELEM_TYPE_STRING, ELEM_TYPE_TYPED_ARRAY,
            ELEM_TYPE_TYPED_OBJECT, TypedArray, read_elem_type,
        };

        let base = bits as usize as *const u8;
        if base.is_null() {
            return Vec::new();
        }
        // SAFETY: NATIVE_KIND::Ptr(HeapKind::TypedArray) pins the slot bits to
        // a live `*mut TypedArray<T>` (HeapHeader at offset 0, element-type
        // byte stamped at offset 7 by the producer).
        let elem_type = unsafe { read_elem_type(base) };
        unsafe {
            match elem_type {
                ELEM_TYPE_CHAR => {
                    let arr = base as *const TypedArray<char>;
                    let slice = TypedArray::<char>::as_slice(arr);
                    slice
                        .iter()
                        .map(|&c| Arc::new(HeapValue::Char(c)))
                        .collect()
                }
                ELEM_TYPE_STRING => {
                    let arr = base as *const TypedArray<*const StringObj>;
                    let slice = TypedArray::<*const StringObj>::as_slice(arr);
                    slice
                        .iter()
                        .map(|&p| {
                            Arc::new(HeapValue::String(Arc::new(StringObj::as_str(p).to_owned())))
                        })
                        .collect()
                }
                ELEM_TYPE_DECIMAL => {
                    let arr = base as *const TypedArray<*const DecimalObj>;
                    let slice = TypedArray::<*const DecimalObj>::as_slice(arr);
                    slice
                        .iter()
                        .map(|&p| Arc::new(HeapValue::Decimal(Arc::new(DecimalObj::value(p)))))
                        .collect()
                }
                ELEM_TYPE_TYPED_OBJECT => {
                    let arr = base as *const TypedArray<*const TypedObjectStorage>;
                    let slice = TypedArray::<*const TypedObjectStorage>::as_slice(arr);
                    slice
                        .iter()
                        .map(|&p| {
                            // The array owns one share per stored pointer; the
                            // returned wrapper gets a fresh share retired by
                            // TypedObjectPtr::drop.
                            v2_retain(&(*p).header);
                            Arc::new(HeapValue::TypedObject(TypedObjectPtr::new(p)))
                        })
                        .collect()
                }
                ELEM_TYPE_TYPED_ARRAY => panic!(
                    "FromSlot<Vec<Arc<HeapValue>>>: TypedArray element-type \
                     ELEM_TYPE_TYPED_ARRAY is a slot-level nested-array \
                     carrier, not a canonical HeapValue arm. Use an exact \
                     body type such as Vec<Vec<Arc<String>>> for \
                     Array<Array<string>> so the inner element contract \
                     stays producer-stamped and monomorphic (STAGE K2)."
                ),
                other => panic!(
                    "FromSlot<Vec<Arc<HeapValue>>>: TypedArray element-type \
                     discriminant {} has no canonical Arc<HeapValue> arm. \
                     Scalar element kinds (number/int/bool/sized-ints/f32) are \
                     inline-scalar NativeKinds with no HeapValue projection and \
                     marshal via Arc<Vec<T>>; an unstamped (0) array means the \
                     producer-side stamp_elem_type contract was violated. \
                     Marshal kind contract violated by caller (STAGE K2).",
                    other
                ),
            }
        }
    }
}

/// Read an `Array<Array<string>>` from a `NativeKind::Ptr(HeapKind::TypedArray)`
/// slot.
///
/// The outer slot must be a v2-raw
/// `TypedArray<*const TypedArrayElem>` stamped `ELEM_TYPE_TYPED_ARRAY`.
/// Every inner row must be a v2-raw `TypedArray<*const StringObj>` stamped
/// `ELEM_TYPE_STRING`. The stamps are producer-side element contracts; this
/// reader does not infer element kinds from payload bits.
impl FromSlot for Vec<Vec<Arc<String>>> {
    const NATIVE_KIND: NativeKind = NativeKind::Ptr(shape_value::HeapKind::TypedArray);

    #[inline]
    fn from_slot(bits: u64) -> Self {
        use shape_value::v2::string_obj::StringObj;
        use shape_value::v2::typed_array::{
            ELEM_TYPE_STRING, ELEM_TYPE_TYPED_ARRAY, TypedArray, TypedArrayElem, read_elem_type,
        };

        let base = bits as usize as *const u8;
        if base.is_null() {
            return Vec::new();
        }

        unsafe {
            let outer_elem_type = read_elem_type(base);
            if outer_elem_type != ELEM_TYPE_TYPED_ARRAY {
                panic!(
                    "FromSlot<Vec<Vec<Arc<String>>>>: expected outer \
                     Array<Array<string>> carrier stamped \
                     ELEM_TYPE_TYPED_ARRAY ({}), got element-type \
                     discriminant {}.",
                    ELEM_TYPE_TYPED_ARRAY, outer_elem_type
                );
            }

            let outer = base as *const TypedArray<*const TypedArrayElem>;
            let outer_slice = TypedArray::<*const TypedArrayElem>::as_slice(outer);
            let mut rows = Vec::with_capacity(outer_slice.len());

            for (row_index, &row_ptr) in outer_slice.iter().enumerate() {
                if row_ptr.is_null() {
                    panic!(
                        "FromSlot<Vec<Vec<Arc<String>>>>: row {} is a null \
                         inner typed-array pointer; Array<Array<string>> rows \
                         must be stamped Array<string> carriers.",
                        row_index
                    );
                }

                let row_base = row_ptr as *const u8;
                let inner_elem_type = read_elem_type(row_base);
                if inner_elem_type != ELEM_TYPE_STRING {
                    panic!(
                        "FromSlot<Vec<Vec<Arc<String>>>>: row {} expected \
                         inner Array<string> carrier stamped \
                         ELEM_TYPE_STRING ({}), got element-type \
                         discriminant {}.",
                        row_index, ELEM_TYPE_STRING, inner_elem_type
                    );
                }

                let row_arr = row_ptr as *const TypedArray<*const StringObj>;
                let row_slice = TypedArray::<*const StringObj>::as_slice(row_arr);
                let row = row_slice
                    .iter()
                    .map(|&p| Arc::new(StringObj::as_str(p).to_owned()))
                    .collect();
                rows.push(row);
            }

            rows
        }
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
    const NATIVE_KIND: NativeKind = NativeKind::Ptr(shape_value::HeapKind::TypedArray);
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
        unsafe {
            shape_value::v2::typed_array::stamp_elem_type(
                arr as *mut u8,
                shape_value::v2::typed_array::ELEM_TYPE_STRING,
            );
        }
        arr as usize as u64
    }
}

/// Project an `Array<Array<string>>` into the v2-raw nested typed-array
/// carrier.
///
/// The outer allocation is `TypedArray<*const TypedArrayElem>` stamped
/// `ELEM_TYPE_TYPED_ARRAY`. Each row is a freshly allocated
/// `TypedArray<*const StringObj>` stamped `ELEM_TYPE_STRING`; the row's
/// initial refcount share transfers into the outer array, whose drop path
/// releases inner rows via `TypedArrayElem::release_elem`.
impl ToSlot for Vec<Vec<Arc<String>>> {
    const NATIVE_KIND: NativeKind = NativeKind::Ptr(shape_value::HeapKind::TypedArray);

    #[inline]
    fn to_slot(self) -> u64 {
        use shape_value::v2::string_obj::StringObj;
        use shape_value::v2::typed_array::{
            ELEM_TYPE_STRING, ELEM_TYPE_TYPED_ARRAY, TypedArray, TypedArrayElem, stamp_elem_type,
        };

        let outer = TypedArray::<*const TypedArrayElem>::with_capacity(self.len() as u32);
        unsafe {
            stamp_elem_type(outer as *mut u8, ELEM_TYPE_TYPED_ARRAY);

            for row in self {
                let row_arr = TypedArray::<*const StringObj>::with_capacity(row.len() as u32);
                stamp_elem_type(row_arr as *mut u8, ELEM_TYPE_STRING);

                for cell in row {
                    let p = StringObj::new(cell.as_str()) as *const StringObj;
                    TypedArray::<*const StringObj>::push(row_arr, p);
                }

                TypedArray::<*const TypedArrayElem>::push(outer, row_arr as *const TypedArrayElem);
            }
        }

        outer as usize as u64
    }
}

/// Project a `Vec<Arc<HeapValue>>` into a `NativeKind::Ptr(HeapKind::TypedArray)`
/// slot.
///
/// STAGE K2 (2026-06-02) — mirror of the `FromSlot<Vec<Arc<HeapValue>>>`
/// reader above. The element kind is chosen by inspecting the first element's
/// `HeapValue` variant (ADR-005 §1: `HeapValue` IS the discriminator); the
/// matching `TypedArray<T>` monomorphization is allocated, stamped with the
/// element discriminator via `stamp_elem_type`, and each element is pushed
/// after building its v2-raw carrier:
///
///   `HeapValue::Char(c)`      → `TypedArray<char>`               (`ELEM_TYPE_CHAR`)
///   `HeapValue::String(s)`    → `TypedArray<*const StringObj>`   (`ELEM_TYPE_STRING`)
///   `HeapValue::Decimal(d)`   → `TypedArray<*const DecimalObj>`  (`ELEM_TYPE_DECIMAL`)
///   `HeapValue::TypedObject`  → `TypedArray<*const TypedObjectStorage>`
///                               (`ELEM_TYPE_TYPED_OBJECT`)
///
/// Owns-clone / refcount discipline: String and Decimal elements allocate
/// fresh `StringObj` / `DecimalObj` (refcount = 1) — the new array owns those
/// allocations outright. TypedObject elements `v2_retain` the pointed-to
/// HeapHeader before pushing the raw pointer, so the array's one share per
/// element is independent of the input `Vec`'s share (which Drop retires when
/// the `Vec<Arc<HeapValue>>` is dropped by the caller).
///
/// The array must be homogeneous (a single `HeapValue` variant) — a mixed
/// `Vec` cannot map to one monomorphized `TypedArray<T>` and SURFACEs.
/// Variants with no v2-raw element carrier (scalar `HeapValue`s, `HashMap`,
/// nested arrays, etc.) SURFACE rather than shim. An empty `Vec` returns a
/// null pointer (bits = 0); the reader maps null → `Vec::new()`, preserving
/// round-trip identity for the empty case.
impl ToSlot for Vec<Arc<shape_value::heap_value::HeapValue>> {
    const NATIVE_KIND: NativeKind = NativeKind::Ptr(shape_value::HeapKind::TypedArray);
    #[inline]
    fn to_slot(self) -> u64 {
        use shape_value::heap_value::{HeapValue, TypedObjectStorage};
        use shape_value::v2::decimal_obj::DecimalObj;
        use shape_value::v2::refcount::v2_retain;
        use shape_value::v2::string_obj::StringObj;
        use shape_value::v2::typed_array::{
            ELEM_TYPE_CHAR, ELEM_TYPE_DECIMAL, ELEM_TYPE_STRING, ELEM_TYPE_TYPED_OBJECT,
            TypedArray, stamp_elem_type,
        };

        let first = match self.first() {
            // Empty array: null carrier; the reader maps null → Vec::new().
            None => return 0,
            Some(arc) => arc,
        };

        // Per-variant homogeneity guard: every element must share the first
        // element's variant. A heterogeneous Vec cannot project to a single
        // monomorphized TypedArray<T>.
        macro_rules! assert_homogeneous {
            ($variant:literal, $pat:pat) => {
                for (i, arc) in self.iter().enumerate() {
                    if !matches!(&**arc, $pat) {
                        panic!(
                            "ToSlot<Vec<Arc<HeapValue>>>: heterogeneous array — \
                             element 0 is {} but element {} is HeapValue::{:?}. \
                             A v2-raw TypedArray<T> is monomorphic; mixed-variant \
                             arrays have no single-T carrier (STAGE K2).",
                            $variant,
                            i,
                            arc.kind()
                        );
                    }
                }
            };
        }

        match &**first {
            HeapValue::Char(_) => {
                assert_homogeneous!("Char", HeapValue::Char(_));
                let out = TypedArray::<char>::with_capacity(self.len() as u32);
                unsafe {
                    stamp_elem_type(out as *mut u8, ELEM_TYPE_CHAR);
                    for arc in &self {
                        if let HeapValue::Char(c) = &**arc {
                            TypedArray::<char>::push(out, *c);
                        }
                    }
                }
                out as usize as u64
            }
            HeapValue::String(_) => {
                assert_homogeneous!("String", HeapValue::String(_));
                let out = TypedArray::<*const StringObj>::with_capacity(self.len() as u32);
                unsafe {
                    stamp_elem_type(out as *mut u8, ELEM_TYPE_STRING);
                    for arc in &self {
                        if let HeapValue::String(s) = &**arc {
                            // Fresh StringObj (refcount = 1); the array owns it.
                            let p = StringObj::new(s.as_str()) as *const StringObj;
                            TypedArray::<*const StringObj>::push(out, p);
                        }
                    }
                }
                out as usize as u64
            }
            HeapValue::Decimal(_) => {
                assert_homogeneous!("Decimal", HeapValue::Decimal(_));
                let out = TypedArray::<*const DecimalObj>::with_capacity(self.len() as u32);
                unsafe {
                    stamp_elem_type(out as *mut u8, ELEM_TYPE_DECIMAL);
                    for arc in &self {
                        if let HeapValue::Decimal(d) = &**arc {
                            // Fresh DecimalObj (refcount = 1); the array owns it.
                            let p = DecimalObj::new(**d) as *const DecimalObj;
                            TypedArray::<*const DecimalObj>::push(out, p);
                        }
                    }
                }
                out as usize as u64
            }
            HeapValue::TypedObject(_) => {
                assert_homogeneous!("TypedObject", HeapValue::TypedObject(_));
                let out = TypedArray::<*const TypedObjectStorage>::with_capacity(self.len() as u32);
                unsafe {
                    stamp_elem_type(out as *mut u8, ELEM_TYPE_TYPED_OBJECT);
                    for arc in &self {
                        if let HeapValue::TypedObject(tp) = &**arc {
                            let p = tp.as_ptr();
                            // Array takes its own share; input Vec keeps its
                            // share (released on the caller's Vec Drop).
                            v2_retain(&(*p).header);
                            TypedArray::<*const TypedObjectStorage>::push(out, p);
                        }
                    }
                }
                out as usize as u64
            }
            other => panic!(
                "ToSlot<Vec<Arc<HeapValue>>>: HeapValue::{:?} has no v2-raw \
                 TypedArray<T> element carrier. Only Char / String / Decimal / \
                 TypedObject elements have a stamped ELEM_TYPE_* monomorphization; \
                 scalar / HashMap / nested-array variants SURFACE rather than \
                 shim (STAGE K2).",
                other.kind()
            ),
        }
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
    const NATIVE_KIND: NativeKind = NativeKind::Ptr(shape_value::HeapKind::HashMap);
    #[inline]
    fn from_slot(bits: u64) -> Self {
        use shape_value::heap_value::HashMapKindedRef;

        // SAFETY: `ValueSlot::from_hashmap` / `KindedSlot::from_hashmap`
        // store `Arc::into_raw(Arc<HashMapKindedRef>)` directly and stamp
        // `Ptr(HeapKind::HashMap)`. The marshal wrapper proved the slot kind;
        // borrow the caller-owned Arc allocation without changing ownership.
        let kref: &HashMapKindedRef = unsafe { &*(bits as *const HashMapKindedRef) };
        unsafe {
            // Wave 2 Round 3b C2-joint ckpt-4 (2026-05-14): per-V
            // walk for HashMap<string, string> (V=String). Other V
            // variants panic — the marshal contract says caller
            // declared a string-valued map; non-string V is a
            // construction-side type error.
            match kref {
                HashMapKindedRef::String(arc) => {
                    let n = arc.len();
                    let mut out: Vec<(Arc<String>, Arc<String>)> = Vec::with_capacity(n);
                    for i in 0..n {
                        let key = {
                            let ptr = shape_value::v2::typed_array::TypedArray::get_unchecked(
                                arc.keys, i as u32,
                            );
                            Arc::new(shape_value::v2::string_obj::StringObj::as_str(ptr).to_owned())
                        };
                        let val = {
                            let v_ptr: *const shape_value::v2::string_obj::StringObj =
                                *(*arc.values).data.add(i);
                            Arc::new(
                                shape_value::v2::string_obj::StringObj::as_str(v_ptr).to_owned(),
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
    const NATIVE_KIND: NativeKind = NativeKind::Ptr(shape_value::HeapKind::HashMap);
    #[inline]
    fn from_slot(bits: u64) -> Self {
        use shape_value::heap_value::{HashMapKindedRef, HeapValue};

        // SAFETY: `ValueSlot::from_hashmap` / `KindedSlot::from_hashmap`
        // store `Arc::into_raw(Arc<HashMapKindedRef>)` directly and stamp
        // `Ptr(HeapKind::HashMap)`. The marshal wrapper proved the slot kind;
        // borrow the caller-owned Arc allocation without changing ownership.
        let kref: &HashMapKindedRef = unsafe { &*(bits as *const HashMapKindedRef) };
        unsafe {
            // Wave 2 Round 3b C2-joint ckpt-4 (2026-05-14): per-V
            // walk → `Vec<(Arc<String>, Arc<HeapValue>)>` for the
            // polymorphic-valued marshal path. Each per-V slot
            // projects into the canonical `Arc<HeapValue>` arm.
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
                HashMapKindedRef::Callable(arc) => arc.keys,
                HashMapKindedRef::HashMap(arc) => arc.keys,
            };
            for i in 0..n {
                let key = {
                    let ptr =
                        shape_value::v2::typed_array::TypedArray::get_unchecked(keys_ptr, i as u32);
                    Arc::new(shape_value::v2::string_obj::StringObj::as_str(ptr).to_owned())
                };
                let value: Arc<HeapValue> = match kref {
                    HashMapKindedRef::I64(arc) => {
                        let v: i64 = *(*arc.values).data.add(i);
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
                        let v: char = *(*arc.values).data.add(i);
                        Arc::new(HeapValue::Char(v))
                    }
                    HashMapKindedRef::String(arc) => {
                        let ptr: *const shape_value::v2::string_obj::StringObj =
                            *(*arc.values).data.add(i);
                        let s = shape_value::v2::string_obj::StringObj::as_str(ptr).to_owned();
                        Arc::new(HeapValue::String(Arc::new(s)))
                    }
                    HashMapKindedRef::Decimal(arc) => {
                        let ptr: *const shape_value::v2::decimal_obj::DecimalObj =
                            *(*arc.values).data.add(i);
                        let d = (*ptr).value;
                        Arc::new(HeapValue::Decimal(Arc::new(d)))
                    }
                    HashMapKindedRef::TypedObject(arc) => {
                        let elem: &shape_value::heap_value::TypedObjectPtr =
                            &*(*arc.values).data.add(i);
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
                    HashMapKindedRef::Callable(arc) => {
                        let elem: &shape_value::heap_value::CallablePtr =
                            &*(*arc.values).data.add(i);
                        Arc::increment_strong_count(elem.as_ptr());
                        Arc::from_raw(elem.as_ptr())
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
                        let inner_ref: &HashMapKindedRef = &*(*arc.values).data.add(i);
                        Arc::new(HeapValue::HashMap(inner_ref.clone()))
                    }
                };
                out.push((key, value));
            }
            out
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
    const NATIVE_KIND: NativeKind = NativeKind::Ptr(shape_value::HeapKind::TypedArray);
    #[inline]
    fn from_slot(bits: u64) -> Self {
        // SAFETY: NATIVE_KIND::Ptr(HeapKind::TypedArray) + body-declared
        // element type Arc<Vec<f64>> pins the slot bits to a live
        // *mut TypedArray<f64>.
        let arr = bits as usize as *const shape_value::v2::typed_array::TypedArray<f64>;
        if arr.is_null() {
            return Arc::new(Vec::new());
        }
        unsafe { Arc::new(shape_value::v2::typed_array::TypedArray::<f64>::as_slice(arr).to_vec()) }
    }
}

/// Project an `Arc<Vec<f64>>` into a `NativeKind::Ptr(HeapKind::TypedArray)`
/// slot whose payload is `*mut TypedArray<f64>`. V3-S5 ckpt-5-prime²c
/// Migration shape (a) — replaces the pre-migration
/// `ToSlot for Arc<AlignedTypedBuffer>` entry.
impl ToSlot for Arc<Vec<f64>> {
    const NATIVE_KIND: NativeKind = NativeKind::Ptr(shape_value::HeapKind::TypedArray);
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
    const NATIVE_KIND: NativeKind = NativeKind::Ptr(shape_value::HeapKind::TypedArray);
    #[inline]
    fn from_slot(bits: u64) -> Self {
        // SAFETY: see Arc<Vec<f64>>::from_slot above.
        let arr = bits as usize as *const shape_value::v2::typed_array::TypedArray<i64>;
        if arr.is_null() {
            return Arc::new(Vec::new());
        }
        unsafe { Arc::new(shape_value::v2::typed_array::TypedArray::<i64>::as_slice(arr).to_vec()) }
    }
}

/// Project an `Arc<Vec<i64>>` into a `NativeKind::Ptr(HeapKind::TypedArray)`
/// slot whose payload is `*mut TypedArray<i64>`. V3-S5 ckpt-5-prime²c
/// Migration shape (a).
impl ToSlot for Arc<Vec<i64>> {
    const NATIVE_KIND: NativeKind = NativeKind::Ptr(shape_value::HeapKind::TypedArray);
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
    const NATIVE_KIND: NativeKind = NativeKind::Ptr(shape_value::HeapKind::TypedArray);
    #[inline]
    fn from_slot(bits: u64) -> Self {
        // SAFETY: see Arc<Vec<f64>>::from_slot above.
        let arr = bits as usize as *const shape_value::v2::typed_array::TypedArray<u8>;
        if arr.is_null() {
            return Arc::new(Vec::new());
        }
        unsafe { Arc::new(shape_value::v2::typed_array::TypedArray::<u8>::as_slice(arr).to_vec()) }
    }
}

/// Project an `Arc<Vec<u8>>` into a `NativeKind::Ptr(HeapKind::TypedArray)`
/// slot whose payload is `*mut TypedArray<u8>`. V3-S5 ckpt-5-prime²c
/// Migration shape (a).
impl ToSlot for Arc<Vec<u8>> {
    const NATIVE_KIND: NativeKind = NativeKind::Ptr(shape_value::HeapKind::TypedArray);
    #[inline]
    fn to_slot(self) -> u64 {
        let arr = shape_value::v2::typed_array::TypedArray::<u8>::from_slice(self.as_slice());
        arr as usize as u64
    }
}

// ─────────────────────── per-arity register helpers ───────────────────────

/// Body type stored in the typed registry: takes per-position typed
/// [`KindedSlot`] carriers (kinds stamped by the VM's §2.7.7 parallel
/// kind track) and returns a [`TypedReturn`]. Internal Rust trait object
/// → carries `KindedSlot`, not raw `&[u64]` (ADR-006 §2.7.5). Constructed
/// only by the typed `register_typed_fn_N` helpers, which type-check the
/// body's actual Rust signature against `FromSlot` for each arg.
type TypedInvoke = Arc<
    dyn for<'ctx> Fn(&[KindedSlot], &ModuleContext<'ctx>) -> Result<TypedReturn, String>
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
    F: for<'ctx> Fn(&ModuleContext<'ctx>) -> Result<TypedReturn, String> + Send + Sync + 'static,
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
    install(
        module,
        name,
        description,
        vec![],
        return_type,
        vec![],
        invoke,
    );
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
        let p0 = P0::from_kinded(&slots[0])?;
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
        let p0 = P0::from_kinded(&slots[0])?;
        let p1 = P1::from_kinded(&slots[1])?;
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
        let p0 = P0::from_kinded(&slots[0])?;
        let p1 = P1::from_kinded(&slots[1])?;
        let p2 = P2::from_kinded(&slots[2])?;
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

/// Register a **per-arity** (fixed 3-arg) native whose body receives the raw
/// `&[KindedSlot]` carriers plus the [`ModuleContext`], rather than
/// `FromSlot`-marshaled Rust params. Used where the argument shapes are
/// polymorphic across call sites but the arity is fixed and every slot's kind
/// is real (from the VM §2.7.7 stack kind track) — e.g. `remote::call`'s
/// `(addr, fn_ref, arg-pack)`, whose `fn_ref` (named-fn id / closure) and
/// arg-pack (per-callee TypedObject / Array) inhabit no single `FromSlot`
/// carrier (distributed §4.1.1).
///
/// This is the **per-arity** typed path (ADR-006 §2.7.4 "per-arity is preferred
/// when the function arity is fixed"): `arg_kinds` are the declared, non-`Bool`
/// kinds and the body dispatches on each slot's stamped `NativeKind` /
/// `as_heap_value()` (ADR-005 §1). It is NOT the variadic `register_typed_function`
/// Bool-default shape (CLAUDE.md §Forbidden Patterns) — no `KindedSlot::new(_,
/// NativeKind::Bool)` fabrication, no kind-from-bits.
pub fn register_typed_fn_3_raw<F>(
    module: &mut crate::module_exports::ModuleExports,
    name: impl Into<String>,
    description: impl Into<String>,
    param_names: [(&str, &str); 3],
    arg_kinds: [NativeKind; 3],
    return_type: crate::typed_module_exports::ConcreteType,
    body: F,
) where
    F: for<'ctx> Fn(&[KindedSlot], &ModuleContext<'ctx>) -> Result<TypedReturn, String>
        + Send
        + Sync
        + 'static,
{
    let invoke: TypedInvoke = Arc::new(move |slots, ctx| {
        if slots.len() != 3 {
            return Err(MarshalError::ArgCount {
                expected: 3,
                got: slots.len(),
            }
            .into());
        }
        body(slots, ctx)
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
        arg_kinds.to_vec(),
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
        let p0 = P0::from_kinded(&slots[0])?;
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
        let p0 = P0::from_kinded(&slots[0])?;
        let p1 = P1::from_kinded(&slots[1])?;
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
        let p0 = P0::from_kinded(&slots[0])?;
        let p1 = P1::from_kinded(&slots[1])?;
        let p2 = P2::from_kinded(&slots[2])?;
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
        let p0 = P0::from_kinded(&slots[0])?;
        let p1 = P1::from_kinded(&slots[1])?;
        let p2 = P2::from_kinded(&slots[2])?;
        let p3 = P3::from_kinded(&slots[3])?;
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
        let p0 = P0::from_kinded(&slots[0])?;
        let p1 = P1::from_kinded(&slots[1])?;
        let p2 = P2::from_kinded(&slots[2])?;
        let p3 = P3::from_kinded(&slots[3])?;
        let p4 = P4::from_kinded(&slots[4])?;
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
        let p0 = P0::from_kinded(&slots[0])?;
        let p1 = P1::from_kinded(&slots[1])?;
        let p2 = P2::from_kinded(&slots[2])?;
        let p3 = P3::from_kinded(&slots[3])?;
        let p4 = P4::from_kinded(&slots[4])?;
        let p5 = P5::from_kinded(&slots[5])?;
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
        let p0 = P0::from_kinded(&slots[0])?;
        let p1 = P1::from_kinded(&slots[1])?;
        let p2 = P2::from_kinded(&slots[2])?;
        let p3 = P3::from_kinded(&slots[3])?;
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
        let p0 = P0::from_kinded(&slots[0])?;
        let p1 = P1::from_kinded(&slots[1])?;
        let p2 = P2::from_kinded(&slots[2])?;
        let p3 = P3::from_kinded(&slots[3])?;
        let p4 = P4::from_kinded(&slots[4])?;
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
        let p0 = P0::from_kinded(&slots[0])?;
        let p1 = P1::from_kinded(&slots[1])?;
        let p2 = P2::from_kinded(&slots[2])?;
        let p3 = P3::from_kinded(&slots[3])?;
        let p4 = P4::from_kinded(&slots[4])?;
        let p5 = P5::from_kinded(&slots[5])?;
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
            Vec<KindedSlot>,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<TypedReturn, String>> + Send>,
        > + Send
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
    let invoke: TypedAsyncInvoke = Arc::new(move |slots: Vec<KindedSlot>| {
        let body = body.clone();
        Box::pin(async move {
            if slots.len() != 1 {
                return Err(MarshalError::ArgCount {
                    expected: 1,
                    got: slots.len(),
                }
                .into());
            }
            let p0 = P0::from_kinded(&slots[0])?;
            body(p0).await
        })
    });
    let params = vec![crate::module_exports::ModuleParam {
        name: param_name.into(),
        type_name: param_type_name.into(),
        required: true,
        ..Default::default()
    }];
    install_async(
        module,
        name,
        description,
        params,
        return_type,
        arg_kinds,
        invoke,
    );
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
    let invoke: TypedAsyncInvoke = Arc::new(move |slots: Vec<KindedSlot>| {
        let body = body.clone();
        Box::pin(async move {
            if slots.len() != 2 {
                return Err(MarshalError::ArgCount {
                    expected: 2,
                    got: slots.len(),
                }
                .into());
            }
            let p0 = P0::from_kinded(&slots[0])?;
            let p1 = P1::from_kinded(&slots[1])?;
            body(p0, p1).await
        })
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
    install_async(
        module,
        name,
        description,
        params,
        return_type,
        arg_kinds,
        invoke,
    );
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
    let invoke: TypedAsyncInvoke = Arc::new(move |slots: Vec<KindedSlot>| {
        let body = body.clone();
        Box::pin(async move {
            if slots.len() != 3 {
                return Err(MarshalError::ArgCount {
                    expected: 3,
                    got: slots.len(),
                }
                .into());
            }
            let p0 = P0::from_kinded(&slots[0])?;
            let p1 = P1::from_kinded(&slots[1])?;
            let p2 = P2::from_kinded(&slots[2])?;
            body(p0, p1, p2).await
        })
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
    install_async(
        module,
        name,
        description,
        params,
        return_type,
        arg_kinds,
        invoke,
    );
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
    let invoke: TypedAsyncInvoke = Arc::new(move |slots: Vec<KindedSlot>| {
        let body = body.clone();
        Box::pin(async move {
            if slots.len() != 1 {
                return Err(MarshalError::ArgCount {
                    expected: 1,
                    got: slots.len(),
                }
                .into());
            }
            let p0 = P0::from_kinded(&slots[0])?;
            body(p0).await
        })
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
    let invoke: TypedAsyncInvoke = Arc::new(move |slots: Vec<KindedSlot>| {
        let body = body.clone();
        Box::pin(async move {
            if slots.len() != 2 {
                return Err(MarshalError::ArgCount {
                    expected: 2,
                    got: slots.len(),
                }
                .into());
            }
            let p0 = P0::from_kinded(&slots[0])?;
            let p1 = P1::from_kinded(&slots[1])?;
            body(p0, p1).await
        })
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
    let invoke: TypedAsyncInvoke = Arc::new(move |slots: Vec<KindedSlot>| {
        let body = body.clone();
        Box::pin(async move {
            if slots.len() != 3 {
                return Err(MarshalError::ArgCount {
                    expected: 3,
                    got: slots.len(),
                }
                .into());
            }
            let p0 = P0::from_kinded(&slots[0])?;
            let p1 = P1::from_kinded(&slots[1])?;
            let p2 = P2::from_kinded(&slots[2])?;
            body(p0, p1, p2).await
        })
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
// matching the §2.7.1.4 dispatch-slice contract. The per-position kinds
// arrive already stamped on the caller's `KindedSlot` carriers (sourced
// from the VM's §2.7.7 parallel kind track at the dispatch site) and are
// passed through UNCHANGED — the body sees the true kinds. There is no
// registration-time placeholder: `arg_kinds` is left empty for variadic
// registrations (per-call kinds come solely from the caller's track,
// never a fabricated `NativeKind::Bool` default — ADR-006 §2.7.8).

use crate::typed_module_exports::TypedModuleFunction;

/// Body signature for a [`register_typed_function`] caller.
///
/// Variadic — the body inspects the slot slice itself rather than
/// declaring a per-arg type at registration. Used by stdlib functions
/// with optional / overload-shaped arguments (json.stringify's optional
/// `pretty`, time.benchmark's optional `iterations`, etc.). For
/// fixed-arity functions, prefer [`register_typed_fn_N`].
pub type VariadicTypedBody = dyn for<'ctx> Fn(&[KindedSlot], &ModuleContext<'ctx>) -> Result<TypedReturn, String>
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
    // Variadic registration carries NO per-position kind schema: the
    // caller's `KindedSlot` carriers already hold the true, compile-time-
    // stamped kinds (VM §2.7.7 parallel kind track). `arg_kinds` stays
    // empty — no `NativeKind::Bool` placeholder is fabricated (ADR-006
    // §2.7.8: never a Bool-default).
    let arg_kinds: Vec<NativeKind> = Vec::new();
    let return_type_str = return_type.shape_type_name();

    // The variadic body already takes `&[KindedSlot]`; the dispatcher
    // hands it the caller's carriers straight through — true kinds flow
    // end-to-end, no re-wrap, no placeholder.
    let invoke: TypedInvoke = Arc::new(body);

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
pub type VariadicTypedAsyncBody<Fut> = dyn Fn(Vec<KindedSlot>) -> Fut + Send + Sync;

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
    // No placeholder kinds: the owned `Vec<KindedSlot>` handed to the body
    // carries the caller's true, compile-time-stamped kinds (§2.7.7).
    let arg_kinds: Vec<NativeKind> = Vec::new();
    let return_type_str = return_type.shape_type_name();

    let invoke: TypedAsyncInvoke = Arc::new(move |slots: Vec<KindedSlot>| {
        let body = body.clone();
        Box::pin(async move { body(slots).await })
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

// ───────── STAGE K2 — Vec<Arc<HeapValue>> round-trip identity tests ─────────
#[cfg(test)]
mod heap_value_vec_marshal_tests {
    use super::{FromSlot, ToSlot};
    use shape_value::heap_value::{HeapValue, TypedObjectStorage};
    use shape_value::v2::typed_array::release_v2_typed_array;
    use std::sync::Arc;

    type HeapVec = Vec<Arc<HeapValue>>;

    /// Build a heap-kinded-field-free `HeapValue::TypedObject` for the given
    /// schema id (no slots → `heap_mask = 0`, so Drop touches no element
    /// shares). Identity for round-trip is the `schema_id`.
    fn make_typed_object(schema_id: u64) -> Arc<HeapValue> {
        let ptr = TypedObjectStorage::_new(
            schema_id,
            Box::new([]),
            0,
            Arc::from(Vec::<shape_value::NativeKind>::new()),
        );
        Arc::new(HeapValue::TypedObject(
            shape_value::heap_value::TypedObjectPtr::new(ptr),
        ))
    }

    #[test]
    fn array_string_round_trips_fromslot_toslot_identity() {
        let original: HeapVec = vec![
            Arc::new(HeapValue::String(Arc::new("alpha".to_string()))),
            Arc::new(HeapValue::String(Arc::new(String::new()))),
            Arc::new(HeapValue::String(Arc::new("βγδ unicode".to_string()))),
        ];

        // ToSlot: project into the v2-raw *mut TypedArray<*const StringObj>.
        let bits = original.clone().to_slot();
        assert_ne!(bits, 0, "non-empty Array<string> must carry a real pointer");

        // FromSlot: read back into a fresh Vec<Arc<HeapValue>>.
        let round: HeapVec = <HeapVec as FromSlot>::from_slot(bits);

        assert_eq!(round.len(), original.len());
        for (a, b) in original.iter().zip(round.iter()) {
            match (&**a, &**b) {
                (HeapValue::String(sa), HeapValue::String(sb)) => {
                    assert_eq!(sa.as_str(), sb.as_str(), "string element identity");
                }
                (x, _) => panic!("expected String/String, got {:?}", x.kind()),
            }
        }

        // Release the carrier (drops the per-element StringObj allocations).
        unsafe { release_v2_typed_array(bits as usize as *mut u8) };
    }

    #[test]
    fn array_typed_object_round_trips_fromslot_toslot_identity() {
        let original: HeapVec = vec![
            make_typed_object(7),
            make_typed_object(42),
            make_typed_object(7),
        ];

        let bits = original.clone().to_slot();
        assert_ne!(bits, 0, "non-empty Array<TypedObject> must carry a pointer");

        let round: HeapVec = <HeapVec as FromSlot>::from_slot(bits);

        assert_eq!(round.len(), original.len());
        for (a, b) in original.iter().zip(round.iter()) {
            match (&**a, &**b) {
                (HeapValue::TypedObject(ta), HeapValue::TypedObject(tb)) => {
                    assert_eq!(
                        ta.schema_id, tb.schema_id,
                        "typed-object schema-id identity"
                    );
                }
                (x, _) => panic!("expected TypedObject/TypedObject, got {:?}", x.kind()),
            }
        }

        // Release the read-back wrappers (each owns one v2_retain'd share),
        // then the carrier array (owns one share per stored pointer), then the
        // original Vec (Drop of each Arc<HeapValue::TypedObject> releases its
        // own share). The underlying TypedObjectStorage allocations are freed
        // when the last of those three shares is retired.
        drop(round);
        unsafe { release_v2_typed_array(bits as usize as *mut u8) };
        drop(original);
    }

    #[test]
    fn empty_array_round_trips_via_null_carrier() {
        let original: HeapVec = Vec::new();
        let bits = original.to_slot();
        assert_eq!(bits, 0, "empty array projects to the null carrier");
        let round: HeapVec = <HeapVec as FromSlot>::from_slot(bits);
        assert!(round.is_empty(), "null carrier reads back as empty");
    }

    #[test]
    fn array_char_round_trips_fromslot_toslot_identity() {
        let original: HeapVec = vec![
            Arc::new(HeapValue::Char('a')),
            Arc::new(HeapValue::Char('Z')),
            Arc::new(HeapValue::Char('λ')),
        ];
        let bits = original.clone().to_slot();
        let round: HeapVec = <HeapVec as FromSlot>::from_slot(bits);
        assert_eq!(round.len(), original.len());
        for (a, b) in original.iter().zip(round.iter()) {
            match (&**a, &**b) {
                (HeapValue::Char(ca), HeapValue::Char(cb)) => assert_eq!(ca, cb),
                (x, _) => panic!("expected Char/Char, got {:?}", x.kind()),
            }
        }
        unsafe { release_v2_typed_array(bits as usize as *mut u8) };
    }

    #[test]
    fn nested_array_string_round_trips_fromslot_toslot_identity() {
        type NestedRows = Vec<Vec<Arc<String>>>;

        let original: NestedRows = vec![
            vec![Arc::new("x".to_string()), Arc::new("y".to_string())],
            vec![Arc::new("1".to_string()), Arc::new("2".to_string())],
            vec![Arc::new(String::new())],
        ];

        let bits = original.clone().to_slot();
        assert_ne!(
            bits, 0,
            "Array<Array<string>> must carry a stamped outer typed array"
        );

        let round: NestedRows = <NestedRows as FromSlot>::from_slot(bits);

        assert_eq!(round.len(), original.len());
        for (row_a, row_b) in original.iter().zip(round.iter()) {
            assert_eq!(row_a.len(), row_b.len());
            for (a, b) in row_a.iter().zip(row_b.iter()) {
                assert_eq!(a.as_str(), b.as_str());
            }
        }

        unsafe { release_v2_typed_array(bits as usize as *mut u8) };
    }

    #[test]
    #[should_panic(expected = "heterogeneous array")]
    fn heterogeneous_array_surfaces_on_toslot() {
        let mixed: HeapVec = vec![
            Arc::new(HeapValue::String(Arc::new("x".to_string()))),
            Arc::new(HeapValue::Char('y')),
        ];
        let _ = mixed.to_slot();
    }
}

// ───────── HashMap direct-carrier marshal tests ─────────
#[cfg(test)]
mod hashmap_marshal_tests {
    use super::FromSlot;
    use shape_value::KindedSlot;
    use shape_value::heap_value::{HashMapData, HashMapKindedRef, HeapValue};
    use shape_value::v2::string_obj::StringObj;
    use std::sync::Arc;

    fn string_hashmap_ref(pairs: &[(&str, &str)]) -> Arc<HashMapKindedRef> {
        let mut data = HashMapData::<*const StringObj>::new();
        for (key, value) in pairs {
            let value_obj = StringObj::new(value);
            unsafe {
                data.insert(key, value_obj as *const StringObj);
            }
        }
        Arc::new(HashMapKindedRef::String(Arc::new(data)))
    }

    fn i64_hashmap_ref(pairs: &[(&str, i64)]) -> Arc<HashMapKindedRef> {
        let mut data = HashMapData::<i64>::new();
        for (key, value) in pairs {
            unsafe {
                data.insert(key, *value);
            }
        }
        Arc::new(HashMapKindedRef::I64(Arc::new(data)))
    }

    #[test]
    fn hashmap_string_string_fromslot_reads_direct_kinded_ref_carrier() {
        let carrier = string_hashmap_ref(&[("accept", "application/json"), ("trace", "on")]);
        let slot = KindedSlot::from_hashmap(Arc::clone(&carrier));
        let bits = slot.slot.raw();

        assert_eq!(Arc::strong_count(&carrier), 2);

        let out: Vec<(Arc<String>, Arc<String>)> =
            <Vec<(Arc<String>, Arc<String>)> as FromSlot>::from_slot(bits);

        assert_eq!(Arc::strong_count(&carrier), 2);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].0.as_str(), "accept");
        assert_eq!(out[0].1.as_str(), "application/json");
        assert_eq!(out[1].0.as_str(), "trace");
        assert_eq!(out[1].1.as_str(), "on");

        drop(slot);
        assert_eq!(Arc::strong_count(&carrier), 1);
    }

    #[test]
    fn hashmap_heapvalue_fromslot_reads_direct_kinded_ref_carrier() {
        let carrier = string_hashmap_ref(&[("method", "GET"), ("content-type", "text/plain")]);
        let slot = KindedSlot::from_hashmap(Arc::clone(&carrier));
        let bits = slot.slot.raw();

        let out: Vec<(Arc<String>, Arc<HeapValue>)> =
            <Vec<(Arc<String>, Arc<HeapValue>)> as FromSlot>::from_slot(bits);

        assert_eq!(out.len(), 2);
        assert_eq!(out[0].0.as_str(), "method");
        match &*out[0].1 {
            HeapValue::String(value) => assert_eq!(value.as_str(), "GET"),
            other => panic!("expected string HeapValue, got {:?}", other.kind()),
        }
        assert_eq!(out[1].0.as_str(), "content-type");
        match &*out[1].1 {
            HeapValue::String(value) => assert_eq!(value.as_str(), "text/plain"),
            other => panic!("expected string HeapValue, got {:?}", other.kind()),
        }

        drop(slot);
        assert_eq!(Arc::strong_count(&carrier), 1);
    }

    #[test]
    fn hashmap_heapvalue_fromslot_projects_i64_values_deterministically() {
        let carrier = i64_hashmap_ref(&[("limit", 10), ("offset", -2)]);
        let slot = KindedSlot::from_hashmap(Arc::clone(&carrier));
        let bits = slot.slot.raw();

        let out: Vec<(Arc<String>, Arc<HeapValue>)> =
            <Vec<(Arc<String>, Arc<HeapValue>)> as FromSlot>::from_slot(bits);

        assert_eq!(out.len(), 2);
        assert_eq!(out[0].0.as_str(), "limit");
        match &*out[0].1 {
            HeapValue::BigInt(value) => assert_eq!(**value, 10),
            other => panic!("expected BigInt HeapValue, got {:?}", other.kind()),
        }
        assert_eq!(out[1].0.as_str(), "offset");
        match &*out[1].1 {
            HeapValue::BigInt(value) => assert_eq!(**value, -2),
            other => panic!("expected BigInt HeapValue, got {:?}", other.kind()),
        }

        drop(slot);
        assert_eq!(Arc::strong_count(&carrier), 1);
    }
}
