//! v2 typed array opcode emission helpers.
//!
//! This module is the *gating layer* between the bytecode compiler and the
//! v2 typed array opcode set. Given a `ConcreteType` for an array's element
//! type, [`should_use_typed_array`] reports whether the compiler can safely
//! emit one of the typed `NewTypedArray*`/`TypedArrayPush*`/`TypedArrayGet*`
//! opcodes (Phase 3.1).
//!
//! Element types this layer recognises:
//!
//! - `f64` (number)
//! - `i64` (int)
//! - `i32`
//! - `bool`
//!
//! Anything else returns `None`. Under the strict-typing policy, callers must
//! either route to a statically specified carrier or surface a compile-time
//! diagnostic; `None` is not permission to resurrect a dynamic `NewArray`
//! fallback.
//!
//! As more typed array opcodes land (`u8`, `i16`, etc.) this helper will grow
//! more `Some(...)` arms; callers don't need to change.

use shape_value::v2::ConcreteType;

use crate::bytecode::OpCode;

/// The kind of typed array the compiler should emit for a known element type.
///
/// Each variant corresponds to a `TypedArray<T>` instantiation that has a
/// matching set of `NewTypedArray*`/`TypedArrayGet*`/`TypedArrayPush*`/
/// `TypedArraySet*` opcodes (Phase 3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypedArrayKind {
    /// `TypedArray<f64>` — backing for `Array<number>`.
    F64,
    /// `TypedArray<i64>` — backing for `Array<int>`.
    I64,
    /// `TypedArray<i32>` — backing for `Array<i32>`.
    I32,
    /// `TypedArray<bool>` — backing for `Array<bool>`.
    Bool,
    /// `TypedArray<i8>` — backing for `Array<i8>` (W12 S1, 2026-05-13).
    I8,
    /// `TypedArray<u8>` — backing for `Array<u8>` (W12 S1, 2026-05-13).
    /// Distinct from `Bool` at the runtime element-type-tag layer
    /// (`ELEM_TYPE_U8` vs `ELEM_TYPE_BOOL`) — same byte storage, different
    /// user-facing semantics.
    U8,
    /// `TypedArray<i16>` — backing for `Array<i16>` (W12 S1, 2026-05-13).
    I16,
    /// `TypedArray<u16>` — backing for `Array<u16>` (W12 S1, 2026-05-13).
    U16,
    /// `TypedArray<u32>` — backing for `Array<u32>` (W12 S1, 2026-05-13).
    U32,
    // U64 deliberately omitted. Per S1 reopen (2026-05-13), `Array<u64>`
    // migration is deferred to S1.5: `NativeKind::UInt64` at HEAD is
    // ambiguous between scalar u64 and v2-typed-array-pointer carrier,
    // and the §2.7.7 / Q9 parallel-kind track has no discriminating
    // variant. The defensive low-address-pointer guard the pre-reopen
    // commit `4bcae991` added at `as_v2_typed_array` was an `is_heap()`
    // probe in different framing — refused on sight per CLAUDE.md
    // §"Parallel-implementation across producer/consumer carrier-shape
    // boundaries".
    /// `TypedArray<f32>` — backing for `Array<f32>` (Wave 2 A1, 2026-05-14).
    F32,
    /// `TypedArray<char>` — backing for `Array<char>` (Wave 2 A1, 2026-05-14).
    /// Per R19 S1.5 amendment to ADR-006 §2.7.5, `Char` is a scalar bucket
    /// carrier (4-byte `Copy`, no heap indirection) — same recipe as F32.
    Char,
    /// `TypedArray<*const StringObj>` — backing for `Array<string>` (Wave 2
    /// A2, 2026-05-14). Per ADR-006 §2.7.24 Q25.A SUPERSEDED + audit §3.2
    /// sub-cluster S2-prime, the v2-raw String element carrier. Element-read
    /// pushes `NativeKind::StringV2` (Agent B Round 1 carrier-shape variant);
    /// distinct from the legacy `NativeKind::String` (Phase-2c `Arc<String>`
    /// carrier; ADR-005 §2 String exception). Per-element retain via
    /// `v2_retain(&(*elem_ptr).header)` at element-read time before pushing
    /// the slot per audit §4.1.B.4 migration recipe.
    String,
    /// `TypedArray<*const DecimalObj>` — backing for `Array<decimal>` (Wave 2
    /// A2, 2026-05-14). Per ADR-006 §2.7.24 Q25.A SUPERSEDED + audit §3.2
    /// sub-cluster S2-prime, the v2-raw Decimal element carrier. Element-read
    /// pushes `NativeKind::DecimalV2`; per-element retain at element-read time.
    Decimal,
    /// `TypedArray<*const TypedObjectStorage>` — backing for
    /// `Array<UserStruct>` (Phase 4b Round 4 W16.2-A
    /// op_new_array-typed-object-element, 2026-05-18). Per ADR-006 §2.7.5
    /// stamp-at-compile-time + §2.7.24 Q25.A SUPERSEDED + audit
    /// `v0.3-w16-v3s5-ckpt56-strict-close-audit.md` §2.1 + §3.A row 1, the
    /// v2-raw TypedObject element carrier. Element-read pushes
    /// `NativeKind::Ptr(HeapKind::TypedObject)` (the existing
    /// kind-discriminator for `*const TypedObjectStorage` slot bits per
    /// `vm_impl/stack.rs:115` clone_with_kind + drop_with_kind dispatch);
    /// per-element retain via `v2_retain(&(*elem_ptr).header)` at element-read
    /// time. The `HeapElement` impl + `_new`/`_drop` raw-pointer allocators
    /// are RESOLVED at HEAD per W12 audit §2.2 Obstacle O-3 (verified at
    /// `crates/shape-value/src/heap_value.rs:3584` `_new` returning
    /// `*mut Self` with `HeapHeader` + `:4058` `unsafe impl HeapElement for
    /// TypedObjectStorage`).
    TypedObject,
    /// `TypedArray<*const TraitObjectStorage>` — backing for `Array<dyn Trait>`
    /// (Phase 4b W16.2-B op_new_array-trait-object-element, 2026-06-05). Per
    /// ADR-006 §2.7.5 stamp-at-compile-time + §2.7.24 Q25.C (TraitObject
    /// re-introduction, all-traits-dyn-able), the v2-raw TraitObject element
    /// carrier. Element-read pushes `NativeKind::Ptr(HeapKind::TraitObject)`
    /// (the kind `DynMethodCall` dispatches on for vtable method calls).
    /// Distinct from `TypedObject`: each element literal is BOXED via
    /// `OpCode::BoxTraitObject` (with the trait-name operand) at the producer
    /// site before push, converting the concrete `Ptr(HeapKind::TypedObject)`
    /// struct value to a fat-pointer `Ptr(HeapKind::TraitObject)`. The
    /// `HeapElement` impl + `_new`/`_drop` allocators are RESOLVED at HEAD
    /// (heap_value.rs:2948 `_new` / :3092 `impl HeapElement`).
    TraitObject,
    /// `TypedArray<*const TypedArrayElem>` — backing for a NESTED array
    /// (`[[1,2],[3,4]]`, Construction strict-typing close, USER RULING
    /// 2026-06-05). Each element is itself a v2-raw `*mut TypedArray<U>`
    /// viewed through its HeapHeader. Element carrier kind is
    /// `NativeKind::Ptr(HeapKind::TypedArray)`; per-element release dispatches
    /// through the kind-erased `release_v2_typed_array`. Producer-side proof:
    /// the element is an `Expr::Array` literal (structurally an inner typed
    /// array) per ADR-006 §2.7.5.
    TypedArray,
}

impl TypedArrayKind {
    /// The `NewTypedArray*` opcode that allocates this kind of array.
    #[inline]
    pub fn new_opcode(self) -> OpCode {
        match self {
            TypedArrayKind::F64 => OpCode::NewTypedArrayF64,
            TypedArrayKind::I64 => OpCode::NewTypedArrayI64,
            TypedArrayKind::I32 => OpCode::NewTypedArrayI32,
            TypedArrayKind::Bool => OpCode::NewTypedArrayBool,
            TypedArrayKind::I8 => OpCode::NewTypedArrayI8,
            TypedArrayKind::U8 => OpCode::NewTypedArrayU8,
            TypedArrayKind::I16 => OpCode::NewTypedArrayI16,
            TypedArrayKind::U16 => OpCode::NewTypedArrayU16,
            TypedArrayKind::U32 => OpCode::NewTypedArrayU32,
            TypedArrayKind::F32 => OpCode::NewTypedArrayF32,
            TypedArrayKind::Char => OpCode::NewTypedArrayChar,
            // Wave 2 Agent A2 (2026-05-14) — String + Decimal heap-element monomorphizations.
            TypedArrayKind::String => OpCode::NewTypedArrayString,
            TypedArrayKind::Decimal => OpCode::NewTypedArrayDecimal,
            // Phase 4b Round 4 W16.2-A op_new_array-typed-object-element (2026-05-18).
            TypedArrayKind::TypedObject => OpCode::NewTypedArrayTypedObject,
            // Phase 4b W16.2-B op_new_array-trait-object-element (2026-06-05).
            TypedArrayKind::TraitObject => OpCode::NewTypedArrayTraitObject,
            // Construction strict-typing close (2026-06-05) — nested array.
            TypedArrayKind::TypedArray => OpCode::NewTypedArrayNested,
        }
    }

    /// The `TypedArrayGet*` opcode for this kind.
    #[inline]
    pub fn get_opcode(self) -> OpCode {
        match self {
            TypedArrayKind::F64 => OpCode::TypedArrayGetF64,
            TypedArrayKind::I64 => OpCode::TypedArrayGetI64,
            TypedArrayKind::I32 => OpCode::TypedArrayGetI32,
            TypedArrayKind::Bool => OpCode::TypedArrayGetBool,
            TypedArrayKind::I8 => OpCode::TypedArrayGetI8,
            TypedArrayKind::U8 => OpCode::TypedArrayGetU8,
            TypedArrayKind::I16 => OpCode::TypedArrayGetI16,
            TypedArrayKind::U16 => OpCode::TypedArrayGetU16,
            TypedArrayKind::U32 => OpCode::TypedArrayGetU32,
            TypedArrayKind::F32 => OpCode::TypedArrayGetF32,
            TypedArrayKind::Char => OpCode::TypedArrayGetChar,
            // Wave 2 Agent A2 (2026-05-14) — String + Decimal heap-element monomorphizations.
            TypedArrayKind::String => OpCode::TypedArrayGetString,
            TypedArrayKind::Decimal => OpCode::TypedArrayGetDecimal,
            // Phase 4b Round 4 W16.2-A op_new_array-typed-object-element (2026-05-18).
            TypedArrayKind::TypedObject => OpCode::TypedArrayGetTypedObject,
            // Phase 4b W16.2-B op_new_array-trait-object-element (2026-06-05).
            TypedArrayKind::TraitObject => OpCode::TypedArrayGetTraitObject,
            // Construction strict-typing close (2026-06-05) — nested array.
            TypedArrayKind::TypedArray => OpCode::TypedArrayGetNested,
        }
    }

    /// The `TypedArrayPush*` opcode for this kind.
    #[inline]
    pub fn push_opcode(self) -> OpCode {
        match self {
            TypedArrayKind::F64 => OpCode::TypedArrayPushF64,
            TypedArrayKind::I64 => OpCode::TypedArrayPushI64,
            TypedArrayKind::I32 => OpCode::TypedArrayPushI32,
            TypedArrayKind::Bool => OpCode::TypedArrayPushBool,
            TypedArrayKind::I8 => OpCode::TypedArrayPushI8,
            TypedArrayKind::U8 => OpCode::TypedArrayPushU8,
            TypedArrayKind::I16 => OpCode::TypedArrayPushI16,
            TypedArrayKind::U16 => OpCode::TypedArrayPushU16,
            TypedArrayKind::U32 => OpCode::TypedArrayPushU32,
            TypedArrayKind::F32 => OpCode::TypedArrayPushF32,
            TypedArrayKind::Char => OpCode::TypedArrayPushChar,
            // Wave 2 Agent A2 (2026-05-14) — String + Decimal heap-element monomorphizations.
            TypedArrayKind::String => OpCode::TypedArrayPushString,
            TypedArrayKind::Decimal => OpCode::TypedArrayPushDecimal,
            // Phase 4b Round 4 W16.2-A op_new_array-typed-object-element (2026-05-18).
            TypedArrayKind::TypedObject => OpCode::TypedArrayPushTypedObject,
            // Phase 4b W16.2-B op_new_array-trait-object-element (2026-06-05).
            TypedArrayKind::TraitObject => OpCode::TypedArrayPushTraitObject,
            // Construction strict-typing close (2026-06-05) — nested array.
            TypedArrayKind::TypedArray => OpCode::TypedArrayPushNested,
        }
    }

    /// The `TypedArraySet*` opcode for this kind.
    #[inline]
    pub fn set_opcode(self) -> OpCode {
        match self {
            TypedArrayKind::F64 => OpCode::TypedArraySetF64,
            TypedArrayKind::I64 => OpCode::TypedArraySetI64,
            TypedArrayKind::I32 => OpCode::TypedArraySetI32,
            TypedArrayKind::Bool => OpCode::TypedArraySetBool,
            TypedArrayKind::I8 => OpCode::TypedArraySetI8,
            TypedArrayKind::U8 => OpCode::TypedArraySetU8,
            TypedArrayKind::I16 => OpCode::TypedArraySetI16,
            TypedArrayKind::U16 => OpCode::TypedArraySetU16,
            TypedArrayKind::U32 => OpCode::TypedArraySetU32,
            TypedArrayKind::F32 => OpCode::TypedArraySetF32,
            TypedArrayKind::Char => OpCode::TypedArraySetChar,
            // Wave 2 Agent A2 (2026-05-14) — String + Decimal heap-element monomorphizations.
            TypedArrayKind::String => OpCode::TypedArraySetString,
            TypedArrayKind::Decimal => OpCode::TypedArraySetDecimal,
            // Phase 4b Round 4 W16.2-A op_new_array-typed-object-element (2026-05-18).
            TypedArrayKind::TypedObject => OpCode::TypedArraySetTypedObject,
            // Phase 4b W16.2-B op_new_array-trait-object-element (2026-06-05).
            TypedArrayKind::TraitObject => OpCode::TypedArraySetTraitObject,
            // Construction strict-typing close (2026-06-05) — nested array.
            TypedArrayKind::TypedArray => OpCode::TypedArraySetNested,
        }
    }
}

/// Map a `ConcreteType` element type to a `TypedArrayKind`, if a typed-array
/// fast path exists for that element type.
///
/// Returns `None` for element types that have no direct scalar/heap typed-array
/// opcode in this mapper. Strict callers must either prove a separate
/// structural carrier (for example the nested-array literal path) or reject at
/// compile time; this is not a runtime fallback signal.
///
/// **Important**: this function is the *single source of truth* for the
/// "do we have a typed-array opcode for this element type?"
#[inline]
pub fn should_use_typed_array(elem_type: &ConcreteType) -> Option<TypedArrayKind> {
    match elem_type {
        ConcreteType::F64 => Some(TypedArrayKind::F64),
        ConcreteType::I64 => Some(TypedArrayKind::I64),
        ConcreteType::I32 => Some(TypedArrayKind::I32),
        ConcreteType::Bool => Some(TypedArrayKind::Bool),
        // W12 S1 (2026-05-13) — sized integer monomorphizations.
        ConcreteType::I8 => Some(TypedArrayKind::I8),
        ConcreteType::U8 => Some(TypedArrayKind::U8),
        ConcreteType::I16 => Some(TypedArrayKind::I16),
        ConcreteType::U16 => Some(TypedArrayKind::U16),
        ConcreteType::U32 => Some(TypedArrayKind::U32),
        // ConcreteType::U64 intentionally falls through to the legacy
        // NaN-boxed path. Per S1 reopen (2026-05-13), `TypedArray<u64>`
        // migration is deferred to S1.5 — the §2.7.7/Q9 parallel-kind-
        // track invariant requires a discriminator between scalar u64
        // and v2-typed-array-pointer before the U64 carrier can dispatch
        // without runtime `is_heap()` probes.
        // Wave 2 Agent A1 (2026-05-14) — F32 + Char scalar monomorphizations
        // per R19 S1.5 amendment to ADR-006 §2.7.5.
        ConcreteType::F32 => Some(TypedArrayKind::F32),
        ConcreteType::Char => Some(TypedArrayKind::Char),
        // Wave 2 Round 3a' A2-followup-gate-flip (2026-05-14) — String +
        // Decimal heap-element gate FLIPPED to v2-raw `TypedArray<*const
        // StringObj>` / `TypedArray<*const DecimalObj>` shape, in lockstep
        // with the Round 3a' α/β/γ/δ/ε/ζ/η v2-raw consumer arms landed in
        // the array_{transform,aggregation,query,sets,sort,basic,joins}.rs
        // executor handlers. Per ADR-006 §2.7.24 Q25.A SUPERSEDED + audit
        // §3.2 sub-cluster S2-prime + §4.1.B per-element retain/release ABI:
        // `Array<string>` / `Array<decimal>` literals now route through the
        // NewTypedArrayString / NewTypedArrayDecimal opcodes; element-read
        // pushes `NativeKind::StringV2` / `NativeKind::DecimalV2`; per-element
        // retain via `v2_retain(&(*elem_ptr).header)` at element-read time.
        ConcreteType::String => Some(TypedArrayKind::String),
        ConcreteType::Decimal => Some(TypedArrayKind::Decimal),
        // Phase 4b Round 4 W16.2-A op_new_array-typed-object-element (2026-05-18)
        // per ADR-006 §2.7.5 stamp-at-compile-time + §2.7.24 Q25.A SUPERSEDED +
        // audit `v0.3-w16-v3s5-ckpt56-strict-close-audit.md` §2.1 + §3.A row 1.
        // `Array<UserStruct>` routes to v2-raw `TypedArray<*const
        // TypedObjectStorage>` carrier. The producer-site proof is the bytecode
        // compiler's `ConcreteType::Struct(StructLayoutId)` for the element
        // type — recorded on every literal at compile time. NO ConcreteType
        // round-trip needed: the slot-bits at runtime carry
        // `NativeKind::Ptr(HeapKind::TypedObject)`, the same kind label the
        // existing single-TypedObject path already uses, so the 4-table
        // HeapKind dispatch (clone_with_kind / drop_with_kind / ...) handles
        // the carrier uniformly without per-instantiation discriminator.
        ConcreteType::Struct(_) => Some(TypedArrayKind::TypedObject),
        // STAGE C2 (2026-06-17): enum-element arrays reuse the W16.2-A
        // TypedObject carrier. Enum values are TypedObjects at runtime
        // (`compile_expr_enum_constructor` emits `NewTypedObject` carrying
        // `NativeKind::Ptr(HeapKind::TypedObject)` for unit / tuple-payload /
        // struct-payload variants alike), so an enum element's slot-bits kind
        // is identical to a struct's. The 4-table HeapKind dispatch handles the
        // carrier uniformly; no new HeapKind / ELEM_TYPE discriminant. This arm
        // is what lets the `Vec.filter` body's `result.push(item)` accumulator
        // resolve its element kind when `item: SomeEnum` (the R3-subcase shape
        // for enum-element arrays). Per ADR-006 §2.7.5 the producer-side proof
        // is `ConcreteType::Enum`; the enum-vs-struct distinction is irrelevant
        // to the runtime carrier.
        ConcreteType::Enum(_) => Some(TypedArrayKind::TypedObject),
        _ => None,
    }
}

/// Reverse mapping: derive the `ConcreteType` element type a
/// [`TypedArrayKind`] was minted for.
///
/// Mirror of [`should_use_typed_array`]: every variant produced by that
/// function round-trips back to its source `ConcreteType` here.
///
/// LANG-9 (Phase 4b round 2, 2026-05-18): inline array literals
/// (`[1,2,3].map(...)`) monomorphize the method call via
/// `concrete_type_for_expr(Expr::Array)`. U4-6a (2026-06-24) deleted the
/// per-span `array_element_types` cache this helper used to feed; the element
/// `ConcreteType` is now derived STRUCTURALLY by `concrete_type_for_expr`'s
/// element recursion. Per ADR-006 §2.7.5 stamp-at-compile-time, the literal's
/// chosen `TypedArrayKind` IS the proof of element-type at construction time —
/// this helper round-trips that proof for the scalar/primitive kinds.
#[inline]
pub fn concrete_type_for_typed_array_kind(kind: TypedArrayKind) -> ConcreteType {
    match kind {
        TypedArrayKind::F64 => ConcreteType::F64,
        TypedArrayKind::I64 => ConcreteType::I64,
        TypedArrayKind::I32 => ConcreteType::I32,
        TypedArrayKind::Bool => ConcreteType::Bool,
        TypedArrayKind::I8 => ConcreteType::I8,
        TypedArrayKind::U8 => ConcreteType::U8,
        TypedArrayKind::I16 => ConcreteType::I16,
        TypedArrayKind::U16 => ConcreteType::U16,
        TypedArrayKind::U32 => ConcreteType::U32,
        TypedArrayKind::F32 => ConcreteType::F32,
        TypedArrayKind::Char => ConcreteType::Char,
        TypedArrayKind::String => ConcreteType::String,
        TypedArrayKind::Decimal => ConcreteType::Decimal,
        // Phase 4b Round 4 W16.2-A op_new_array-typed-object-element (2026-05-18).
        // Returns `ConcreteType::placeholder_struct(StructLayoutId(0))` as a placeholder —
        // every typed-object struct schema collapses to the same TypedArrayKind
        // (the slot-bits kind is uniformly `Ptr(HeapKind::TypedObject)`), so
        // the kind→ConcreteType round-trip cannot recover the specific
        // StructLayoutId without an additional structural lookup. This mirrors
        // the `helpers.rs:719` shape `ConcreteType::placeholder_struct(StructLayoutId(0))`
        // used by `StatementKind::ObjectStore` slot-stamping. Downstream
        // consumers that need the precise schema recover it STRUCTURALLY via
        // `concrete_type_for_expr` over the literal's named-struct elements
        // (U4-6a: the former `array_element_types[span]` cache is deleted).
        TypedArrayKind::TypedObject => {
            ConcreteType::placeholder_struct(shape_value::v2::concrete_type::StructLayoutId(0))
        }
        // Phase 4b W16.2-B op_new_array-trait-object-element (2026-06-05).
        // ConcreteType has no `dyn Trait` variant; the kind→ConcreteType
        // round-trip cannot recover the trait identity (every `Array<dyn T>`
        // collapses to the same TraitObject carrier, slot-bits kind uniformly
        // `Ptr(HeapKind::TraitObject)`). Return a placeholder_struct mirroring
        // the TypedObject arm; the precise trait identity is recovered
        // structurally by `concrete_type_for_expr` over the literal's elements
        // (U4-6a: the former `array_element_types` cache is deleted).
        TypedArrayKind::TraitObject => {
            ConcreteType::placeholder_struct(shape_value::v2::concrete_type::StructLayoutId(0))
        }
        // Construction strict-typing close (2026-06-05) — nested array. The
        // kind→ConcreteType round-trip cannot recover the inner element type
        // (every nested-array monomorphization collapses to this one kind, the
        // carrier kind being uniformly `Ptr(HeapKind::TypedArray)`). Return a
        // `Array<Array<?>>` placeholder mirroring the TypedObject placeholder
        // shape; downstream consumers that need the precise inner element type
        // recover it structurally via `concrete_type_for_expr` over the
        // literal's inner-array elements (U4-6a: the former
        // `array_element_types[span]` cache is deleted).
        TypedArrayKind::TypedArray => ConcreteType::Array(Box::new(ConcreteType::Array(Box::new(
            ConcreteType::placeholder_struct(shape_value::v2::concrete_type::StructLayoutId(0)),
        )))),
    }
}

/// `NativeKind` analogue.
///
/// Provided as a bridge for compiler call sites that haven't yet been
/// converted to use `ConcreteType`. The current Phase 1.2 element-type
/// inference (`v2_array_emission::infer_array_element_type`) returns
/// `NativeKind`, so `compile_expr_array` calls this variant to look up a
/// typed array kind directly.
///
/// The mapping mirrors [`should_use_typed_array`]. A `None` result means this
/// slot kind has no typed-array carrier in this mapper; callers must use an
/// explicit static policy, not a dynamic array fallback.
#[inline]
pub fn should_use_typed_array_from_slot_kind(
    slot: crate::type_tracking::NativeKind,
) -> Option<TypedArrayKind> {
    use crate::type_tracking::NativeKind;
    match slot {
        NativeKind::Float64 => Some(TypedArrayKind::F64),
        NativeKind::Int64 => Some(TypedArrayKind::I64),
        NativeKind::Int32 => Some(TypedArrayKind::I32),
        NativeKind::Bool => Some(TypedArrayKind::Bool),
        // W12 S1 (2026-05-13) — sized integer monomorphizations.
        NativeKind::Int8 => Some(TypedArrayKind::I8),
        NativeKind::UInt8 => Some(TypedArrayKind::U8),
        NativeKind::Int16 => Some(TypedArrayKind::I16),
        NativeKind::UInt16 => Some(TypedArrayKind::U16),
        NativeKind::UInt32 => Some(TypedArrayKind::U32),
        // NativeKind::UInt64 deliberately falls through. The slot kind
        // is shared with v2-typed-array pointers (every `*mut TypedArray<T>`
        // flows through `NativeKind::UInt64`), so a `Some(TypedArrayKind::U64)`
        // dispatch here would route producer-emission and consumer-classification
        // through the same overloaded discriminator — the §2.7.7/Q9 parallel-
        // kind-track invariant the S1.5 sub-cluster must resolve before this
        // arm can light up.
        // Wave 2 Agent A1 (2026-05-14) — F32 + Char scalar monomorphizations.
        NativeKind::Float32 => Some(TypedArrayKind::F32),
        NativeKind::Char => Some(TypedArrayKind::Char),
        // Wave 2 Round 3a' A2-followup-gate-flip (2026-05-14) — String +
        // Decimal heap-element slot-kind mirror flipped in lockstep with
        // the `should_use_typed_array(ConcreteType)` gate above. Two routing
        // shapes light up:
        //
        //   - `NativeKind::String` (legacy Phase-2c `Arc<String>` carrier
        //     label) routes here from `typed_array_from_annotation("string")`
        //     when an `Array<string>` annotation drives binding initialization
        //     (`statements.rs:681` flow). Post-flip, the literal-elements'
        //     legacy `Arc<String>` bits + `NativeKind::String` source label
        //     no longer match the `TypedArrayPushString` consumer invariant
        //     (which strictly requires `NativeKind::StringV2`). The literal-
        //     upgrade transition (string literal `LoadConst` → `NewStringV2`)
        //     is the downstream A2-followup-producer-cascade territory; until
        //     that lands, `let xs: Array<string> = [...]` literals will
        //     surface a structured RuntimeError at push time, NOT a SIGSEGV.
        //   - `NativeKind::StringV2` / `NativeKind::DecimalV2` are the v2-raw
        //     carrier labels that match the typed opcode runtime invariants;
        //     these flow from `TypedArrayGetString` / `TypedArrayGetDecimal`
        //     element reads (per `v2_handlers/array.rs:682`).
        //
        // Per ADR-006 §2.7.24 Q25.A SUPERSEDED + audit §3.2 sub-cluster
        // S2-prime + §4.1.B per-element retain/release ABI. Aligned with the
        // Round 3a' α/β/γ/δ/ε/ζ/η consumer arms that landed as UNREACHABLE
        // code with the gate closed; this flip makes them reachable atomically.
        NativeKind::String => Some(TypedArrayKind::String),
        NativeKind::StringV2 => Some(TypedArrayKind::String),
        NativeKind::DecimalV2 => Some(TypedArrayKind::Decimal),
        // Phase 4b Round 4 W16.2-A op_new_array-typed-object-element (2026-05-18).
        // `NativeKind::Ptr(HeapKind::TypedObject)` is the existing slot-bits
        // kind label for `*const TypedObjectStorage` carriers. When an array
        // literal's elements are typed-object scalars (`B { v: 1 }` shape),
        // the type-tracker hint reports `Ptr(TypedObject)` per existing
        // single-TypedObject carrier flow; this arm routes the literal to the
        // typed-array fast path (`NewTypedArrayTypedObject` + per-element
        // `TypedArrayPushTypedObject`). Per ADR-006 §2.7.5 the kind is
        // statically proven at the producer site, never decoded from runtime
        // bits.
        NativeKind::Ptr(shape_value::HeapKind::TypedObject) => Some(TypedArrayKind::TypedObject),
        _ => None,
    }
}

/// Phase 4b Round 6 WS-1 W16.2-C (2026-05-21) — the `Vec<…>` type-tracker
/// name for a [`TypedArrayKind`].
///
/// `compile_list_comprehension` / `compile_array_with_spread` stamp this as
/// `last_expr_type_info` once the accumulator kind is resolved, so the
/// enclosing `let` binding's `propagate_initializer_type_to_slot` records the
/// destination slot as an array (not as the bare element scalar). Mirrors the
/// `Vec<int>` / `Vec<number>` / `Vec<bool>` names `compile_expr_array` stamps.
#[inline]
pub fn vec_type_name_for_typed_array_kind(kind: TypedArrayKind) -> &'static str {
    match kind {
        TypedArrayKind::F64 => "Vec<number>",
        TypedArrayKind::I64 => "Vec<int>",
        TypedArrayKind::I32 => "Vec<i32>",
        TypedArrayKind::Bool => "Vec<bool>",
        TypedArrayKind::I8 => "Vec<i8>",
        TypedArrayKind::U8 => "Vec<u8>",
        TypedArrayKind::I16 => "Vec<i16>",
        TypedArrayKind::U16 => "Vec<u16>",
        TypedArrayKind::U32 => "Vec<u32>",
        TypedArrayKind::F32 => "Vec<f32>",
        TypedArrayKind::Char => "Vec<char>",
        TypedArrayKind::String => "Vec<string>",
        TypedArrayKind::Decimal => "Vec<decimal>",
        TypedArrayKind::TypedObject => "Vec<object>",
        // Phase 4b W16.2-B op_new_array-trait-object-element (2026-06-05).
        TypedArrayKind::TraitObject => "Vec<dyn>",
        TypedArrayKind::TypedArray => "Vec<array>",
    }
}

/// Phase 4b Round 6 WS-1b W16.2-C residual (2026-05-21) — the user-facing
/// ELEMENT type name for a [`TypedArrayKind`] (`I64` → `"int"`), used in
/// heterogeneous-push diagnostics. Distinct from
/// [`vec_type_name_for_typed_array_kind`], which renders the array type
/// (`"Vec<int>"`).
pub fn vec_element_type_name_for_typed_array_kind(kind: TypedArrayKind) -> &'static str {
    match kind {
        TypedArrayKind::F64 => "number",
        TypedArrayKind::I64 => "int",
        TypedArrayKind::I32 => "i32",
        TypedArrayKind::Bool => "bool",
        TypedArrayKind::I8 => "i8",
        TypedArrayKind::U8 => "u8",
        TypedArrayKind::I16 => "i16",
        TypedArrayKind::U16 => "u16",
        TypedArrayKind::U32 => "u32",
        TypedArrayKind::F32 => "f32",
        TypedArrayKind::Char => "char",
        TypedArrayKind::String => "string",
        TypedArrayKind::Decimal => "decimal",
        TypedArrayKind::TypedObject => "object",
        // Phase 4b W16.2-B op_new_array-trait-object-element (2026-06-05).
        TypedArrayKind::TraitObject => "dyn",
        TypedArrayKind::TypedArray => "array",
    }
}

/// Phase 4b Round 6 WS-1 W16.2-C (2026-05-21) — map a proven [`NumericType`]
/// to its [`TypedArrayKind`].
///
/// Used by the spread / list-comprehension accumulator rebuild: once the
/// element expression has been compiled and the bytecode compiler's
/// `last_expr_numeric_type` reports a proven scalar numeric type, this maps
/// it to the typed-array carrier kind so the accumulator can be allocated
/// with the matching `NewTypedArray*` opcode. Per ADR-006 §2.7.5 the kind is
/// proven at the producer site (the compiled element expression) — never
/// decoded from runtime bits, never Bool-defaulted.
#[inline]
pub fn typed_array_kind_from_numeric_type(nt: crate::type_tracking::NumericType) -> TypedArrayKind {
    use crate::type_tracking::NumericType;
    use shape_ast::IntWidth;
    match nt {
        NumericType::Int => TypedArrayKind::I64,
        NumericType::Number => TypedArrayKind::F64,
        NumericType::Decimal => TypedArrayKind::Decimal,
        NumericType::IntWidth(w) => match w {
            IntWidth::I8 => TypedArrayKind::I8,
            IntWidth::U8 => TypedArrayKind::U8,
            IntWidth::I16 => TypedArrayKind::I16,
            IntWidth::U16 => TypedArrayKind::U16,
            IntWidth::I32 => TypedArrayKind::I32,
            IntWidth::U32 => TypedArrayKind::U32,
            // `u64` has no typed-array carrier yet (S1.5 territory — the
            // §2.7.7/Q9 parallel-kind track has no discriminator between
            // scalar u64 and a v2-typed-array pointer). Route `u64`
            // elements to the i64 carrier's storage shape: `IntWidth::U64`
            // values still fit an 8-byte slot and round-trip through the
            // i64 push/get opcodes bit-identically. (A genuine `Array<u64>`
            // monomorphization is deferred; this is the spread/comprehension
            // accumulator, not an annotated `Array<u64>` binding.)
            IntWidth::U64 => TypedArrayKind::I64,
        },
    }
}

impl super::BytecodeCompiler {
    /// Phase 4b Round 4 W16.2-A op_new_array-typed-object-element (2026-05-18).
    ///
    /// Compiler-aware homogeneous-element-kind inference over an array literal's
    /// elements. Wraps the AST-only `v2_array_emission::infer_array_element_type`
    /// (which handles bare literals + `Expr::StructLiteral`/`Expr::Object`
    /// per the W16.2-A new arms) with compiler-state-aware function-call
    /// return-type lookup: when every element is a `FunctionCall { name, .. }`
    /// whose return type the type tracker reports as a registered struct,
    /// the array kind is TypedObject.
    ///
    /// This is the producer-side proof for `let boxes = [aabb(...),
    /// aabb(...), ...]` per ADR-006 §2.7.5: `aabb` is statically known to
    /// return a struct (`type AABB { ... }` + `fn aabb(...) -> AABB`), so
    /// the literal's element kind is proven at compile time without runtime
    /// inference. NO fabrication, NO Bool-default.
    /// U4-5b: the `ConcreteType` a free-function call returns, resolved
    /// STRUCTURALLY. Prefers the declared `-> T` annotation resolved
    /// SCHEMA-AWARELY (`declared_annotation_concrete_type` — struct/enum names
    /// resolve to `ConcreteType::Struct`/`Enum` because the struct registry is
    /// fully populated by bytecode-emission time, even though the function was
    /// registered before its struct schema). Falls back to the inferred return
    /// `ConcreteType` recorded for unannotated functions
    /// (`function_return_concrete_types`). Replaces the deleted return-NAME
    /// string lookup + `struct_types.contains_key` round-trip.
    fn function_call_return_concrete_type(
        &self,
        name: &str,
    ) -> Option<shape_value::v2::ConcreteType> {
        if let Some(def) = self.function_defs.get(name) {
            if let Some(ann) = def.return_type.as_ref() {
                if let Some(ct) =
                    crate::compiler::monomorphization::type_resolution::declared_annotation_concrete_type(
                        self, ann,
                    )
                {
                    return Some(ct);
                }
            }
        }
        self.type_tracker
            .get_function_return_concrete_type(name)
            .cloned()
    }

    pub(crate) fn array_elements_all_typed_object(
        &mut self,
        elements: &[shape_ast::ast::Expr],
    ) -> bool {
        use shape_ast::ast::Expr;
        if elements.is_empty() {
            return false;
        }
        for elem in elements {
            // U4-5b: the element is a named-struct array slot iff the called
            // function's return type is a `Struct`. Resolved STRUCTURALLY — no
            // return-NAME string round-trip through `struct_types`/`type_aliases`.
            let returned_ct: Option<shape_value::v2::ConcreteType> = match elem {
                Expr::FunctionCall { name, .. } => {
                    // Construction strict-typing close (2026-06-05): a
                    // function with an INFERRED anonymous-object return
                    // (`fn aabb(lo, hi) { {min: lo, max: hi} }`) has no named
                    // return type but DOES have an inferred structural object
                    // return. That return is a TypedObject, so the literal is
                    // `Array<TypedObject>` and routes to the same v2-raw
                    // `TypedArray<*const TypedObjectStorage>` carrier.
                    if self.inferred_return_object_schema_id(name).is_some() {
                        continue;
                    }
                    self.function_call_return_concrete_type(name)
                }
                Expr::QualifiedFunctionCall {
                    namespace,
                    function,
                    ..
                } => {
                    let qualified = format!("{}::{}", namespace, function);
                    if self.inferred_return_object_schema_id(&qualified).is_some() {
                        continue;
                    }
                    self.function_call_return_concrete_type(&qualified)
                }
                _ => return false,
            };
            if !matches!(returned_ct, Some(shape_value::v2::ConcreteType::Struct(_))) {
                return false;
            }
        }
        true
    }

    /// Construction strict-typing close (USER RULING 2026-06-05): infer a
    /// homogeneous element [`TypedArrayKind`] for an array literal whose
    /// elements are NON-literal expressions (function calls, identifiers,
    /// loop counters, …), via `concrete_type_for_expr`.
    ///
    /// Returns `Some(kind)` only when EVERY element resolves through the type
    /// tracker to the SAME scalar `ConcreteType` that has a typed-array
    /// carrier (`should_use_typed_array`). Any element that does not resolve,
    /// or that resolves to a different kind, yields `None` — the caller then
    /// surface-and-stops with a clean "cannot infer array element type"
    /// compile error (no untyped runtime array carrier exists).
    ///
    /// Nested-array elements (`Expr::Array`) are intentionally NOT handled
    /// here — they are resolved by the dedicated `all_nested_array_elem`
    /// branch upstream. Per ADR-006 §2.7.5 the element kind is the type
    /// tracker's producer-side proof, never a runtime decode.
    pub(crate) fn infer_array_element_kind_from_concrete_types(
        &self,
        elements: &[shape_ast::ast::Expr],
    ) -> Option<TypedArrayKind> {
        use shape_ast::ast::Expr;
        if elements.is_empty() {
            return None;
        }
        // Nested-array elements are handled by the upstream nested branch.
        if elements.iter().any(|e| matches!(e, Expr::Array(..))) {
            return None;
        }
        let mut acc: Option<TypedArrayKind> = None;
        for elem in elements {
            let ct = super::monomorphization::type_resolution::concrete_type_for_expr(self, elem)?;
            // An array-typed element (`[source]` where `source: Array<int>`)
            // makes the outer a nested array — the inner array carrier is a
            // v2-raw `*mut TypedArray<U>` regardless of `U`, so it maps to the
            // single `TypedArrayKind::TypedArray` nested carrier (same as the
            // literal `[[..],[..]]` shape, just reached via an identifier /
            // expression element instead of an inline `Expr::Array`).
            let kind = match ct {
                shape_value::v2::ConcreteType::Array(_) => TypedArrayKind::TypedArray,
                other => should_use_typed_array(&other)?,
            };
            match acc {
                Some(prev) if prev != kind => return None,
                _ => acc = Some(kind),
            }
        }
        acc
    }

    /// Compiler-aware resolution of a `let arr: Array<T> = [...]` binding's
    /// element annotation to a [`TypedArrayKind`]. Wraps the
    /// `v2_array_emission::typed_array_from_annotation` →
    /// `should_use_typed_array_from_slot_kind` chain but ALSO recognizes the
    /// `Array<UserStruct>` shape: when the inner annotation is
    /// `TypeAnnotation::Basic(name)` and `name` is a registered struct type
    /// (or resolves through `type_aliases` to one), maps to
    /// [`TypedArrayKind::TypedObject`].
    ///
    /// Per ADR-006 §2.7.5 stamp-at-compile-time + audit
    /// `v0.3-w16-v3s5-ckpt56-strict-close-audit.md` §2.1: the producer-site
    /// proof is the explicit annotation; no runtime inference. The bytecode
    /// compiler's `struct_types` map is the authoritative source of "is this
    /// a registered struct?", populated by the type-definition pass.
    pub(crate) fn resolve_typed_array_kind_from_annotation(
        &self,
        annotation: &shape_ast::ast::TypeAnnotation,
    ) -> Option<TypedArrayKind> {
        // Existing scalar / decimal / string mapping.
        if let Some(scalar_kind) =
            crate::compiler::v2_array_emission::typed_array_from_annotation(annotation)
        {
            if let Some(kind) = should_use_typed_array_from_slot_kind(scalar_kind) {
                return Some(kind);
            }
        }
        // Phase 4b W16.2-B op_new_array-trait-object-element (2026-06-05) —
        // `Array<dyn Trait>` annotation. The inner type is `TypeAnnotation::Dyn`,
        // NOT a struct, so this MUST be checked before the struct branch below
        // (the struct branch's `inner_name?` would early-return None on a Dyn
        // inner). Per ADR-006 §2.7.5 + §2.7.24 Q25.C (all-traits-dyn-able): the
        // producer-side proof is the explicit `dyn Trait` annotation. Maps to
        // `TypedArrayKind::TraitObject`; each element literal is boxed via
        // `BoxTraitObject` at the emission site (the trait name is recovered
        // there from the same annotation).
        use shape_ast::ast::TypeAnnotation;
        let dyn_inner: Option<&TypeAnnotation> = match annotation {
            TypeAnnotation::Generic { name, args }
                if name.as_str() == "Array" && args.len() == 1 =>
            {
                Some(&args[0])
            }
            TypeAnnotation::Array(inner) => Some(inner.as_ref()),
            _ => None,
        };
        if let Some(inner) = dyn_inner {
            if crate::compiler::trait_object_emission::trait_name_from_annotation(inner).is_some() {
                return Some(TypedArrayKind::TraitObject);
            }
        }
        // User-struct annotation: `Array<B>` / `B[]` where B is a registered
        // struct type. Map to TypedArrayKind::TypedObject per §2.1 + §3.A row 1.
        let inner_name = match annotation {
            TypeAnnotation::Generic { name, args }
                if name.as_str() == "Array" && args.len() == 1 =>
            {
                match &args[0] {
                    TypeAnnotation::Basic(n) => Some(n.as_str()),
                    _ => None,
                }
            }
            TypeAnnotation::Array(inner) => match inner.as_ref() {
                TypeAnnotation::Basic(n) => Some(n.as_str()),
                _ => None,
            },
            _ => None,
        }?;
        // Resolve through type aliases (`type P = Point` → check Point).
        let resolved = self
            .type_aliases
            .get(inner_name)
            .map(|s| s.as_str())
            .unwrap_or(inner_name);
        if self.struct_types.contains_key(resolved) || self.struct_types.contains_key(inner_name) {
            return Some(TypedArrayKind::TypedObject);
        }
        // STAGE C2 (2026-06-17): `Array<Color>` where `Color` is a registered
        // enum. Enum values are TypedObjects at runtime — `compile_expr_enum_constructor`
        // emits `NewTypedObject` (collections.rs:1569) carrying
        // `NativeKind::Ptr(HeapKind::TypedObject)` for unit / tuple-payload /
        // struct-payload variants alike. So an enum-element array reuses the
        // W16.2-A `TypedArray<*const TypedObjectStorage>` carrier — no new
        // HeapKind, no new ELEM_TYPE discriminant. The producer-side proof is
        // the explicit annotation + the registered enum schema (ADR-006
        // §2.7.5 stamp-at-compile-time); the enum-vs-struct distinction is
        // irrelevant to the runtime carrier (both are TypedObject pointers).
        let is_enum = self
            .type_tracker
            .schema_registry()
            .get(resolved)
            .or_else(|| self.type_tracker.schema_registry().get(inner_name))
            .and_then(|s| s.get_enum_info())
            .is_some();
        if is_enum {
            Some(TypedArrayKind::TypedObject)
        } else {
            None
        }
    }

    /// Phase 4b W16.2-B op_new_array-trait-object-element (2026-06-05).
    ///
    /// Wrapper over [`resolve_typed_array_kind_from_annotation`] that ALSO, as
    /// a side effect, stashes the trait name into
    /// `self.pending_trait_object_array_trait` when the annotation resolves to
    /// [`TypedArrayKind::TraitObject`] (`Array<dyn Trait>`). The element-loop in
    /// `compile_expr_array` reads that field to emit the per-element
    /// `BoxTraitObject`. For all non-dyn kinds the trait field is cleared.
    /// Per ADR-006 §2.7.5 the trait name is the producer-side proof (explicit
    /// annotation), never runtime-derived.
    pub(crate) fn resolve_typed_array_kind_and_record_trait(
        &mut self,
        annotation: &shape_ast::ast::TypeAnnotation,
    ) -> Option<TypedArrayKind> {
        let kind = self.resolve_typed_array_kind_from_annotation(annotation);
        if kind == Some(TypedArrayKind::TraitObject) {
            // Recover the trait name from `Array<dyn Trait>` / `(dyn Trait)[]`.
            use shape_ast::ast::TypeAnnotation;
            let inner: Option<&TypeAnnotation> = match annotation {
                TypeAnnotation::Generic { name, args }
                    if name.as_str() == "Array" && args.len() == 1 =>
                {
                    Some(&args[0])
                }
                TypeAnnotation::Array(inner) => Some(inner.as_ref()),
                _ => None,
            };
            self.pending_trait_object_array_trait = inner
                .and_then(crate::compiler::trait_object_emission::trait_name_from_annotation)
                .map(|s| s.to_string());
        } else {
            self.pending_trait_object_array_trait = None;
        }
        kind
    }

    /// Kind-changing-map carrier reconciliation (2026-06-15).
    ///
    /// When a `let r = <init>` binding's initializer is a value whose PROVEN
    /// element type (via the inference engine) is an `Array<C>`/`Vec<C>`, the
    /// binding's typed-array carrier stamp MUST match C — NOT a
    /// `pending_variable_typed_array_kind` value that leaked from compiling a
    /// SUB-expression of the initializer.
    ///
    /// The canonical defect this closes: `let r = [1,2,3].map(|x| x as number)`.
    /// Compiling the receiver array literal `[1,2,3]` sets
    /// `pending_variable_typed_array_kind = Some(I64)` (the INPUT element kind),
    /// which then leaks onto the outer binding `r`. The map RESULT carrier is
    /// `TypedArray<f64>` (the closure-return kind, correctly built by
    /// `run_select_builder`), so a `TypedArrayGetI64` index read on `r`
    /// reinterprets the f64 bits as i64 (a forbidden bit-reinterpret — `int`
    /// and `number` do not unify, CLAUDE.md §Type-System-Rules).
    ///
    /// Returns a reconciliation DIRECTIVE keyed off the binding's proven type:
    /// - `Some(Some(kind))` — the binding IS an array whose element C maps to a
    ///   scalar typed-array carrier `kind`; stamp `kind` (authoritative).
    /// - `Some(None)` — the binding IS an array but C has NO scalar typed-array
    ///   carrier (heap element, or a kind the carrier set doesn't cover). The
    ///   stale scalar stamp MUST be suppressed so index access falls to the
    ///   carrier-reading `GetProp` (`read_element`) path. SOUNDNESS FLOOR — never
    ///   keep a mismatched scalar stamp that would bit-reinterpret.
    /// - `None` — the binding is not a provable array; leave the existing
    ///   capture untouched (no array-index opcode keys off it).
    ///
    /// Per ADR-006 §2.7.5 the proof is the inference engine's element type — no
    /// runtime bit inspection, no fabricated default.
    pub(crate) fn reconcile_binding_typed_array_kind(
        &mut self,
        init_expr: &shape_ast::ast::Expr,
    ) -> Option<Option<TypedArrayKind>> {
        use shape_ast::ast::TypeAnnotation;
        use shape_runtime::type_system::Type;

        let inferred = self.infer_expr_type(init_expr).ok()?;
        // Extract the element annotation when the inferred type is a homogeneous
        // array carrier (`Array<C>` / `Vec<C>` / `C[]`). Anything else is not a
        // typed-array binding and yields `None` (leave capture untouched).
        //
        // The inference engine carries an instantiated array either as a
        // `Type::Concrete(TypeAnnotation::Array/Generic)` OR as a
        // `Type::Generic { base: Vec/Array, args: [elem] }` (the shape the
        // method-call return inference produces, e.g. `<arr>.map(...)`).
        let elem_ann: TypeAnnotation = match &inferred {
            Type::Concrete(TypeAnnotation::Array(inner)) => (**inner).clone(),
            Type::Concrete(TypeAnnotation::Generic { name, args })
                if (name.as_str() == "Array" || name.as_str() == "Vec") && args.len() == 1 =>
            {
                args[0].clone()
            }
            Type::Generic { base, args } if args.len() == 1 => {
                let base_name: Option<&str> = match base.as_ref() {
                    Type::Concrete(TypeAnnotation::Basic(n)) => Some(n.as_str()),
                    Type::Concrete(TypeAnnotation::Reference(p)) => Some(p.as_str()),
                    _ => None,
                };
                let base_is_array = matches!(base_name, Some(n) if n == "Array" || n == "Vec");
                if !base_is_array {
                    return None;
                }
                Self::inferred_type_to_annotation(&args[0])?
            }
            _ => return None,
        };
        // The binding IS an array. Resolve C's carrier kind through the same
        // annotation→kind path the annotated `let r: Array<C> = ...` binding
        // uses. `Some(kind)` => authoritative scalar/heap carrier; `None` =>
        // no carrier monomorphization (suppress stale stamp, fall to GetProp).
        let array_ann = TypeAnnotation::Generic {
            name: shape_ast::ast::TypePath::simple("Array"),
            args: vec![elem_ann],
        };
        Some(self.resolve_typed_array_kind_from_annotation(&array_ann))
    }

    /// Project an inferred element [`Type`] down to a [`TypeAnnotation`] for the
    /// array-carrier-kind resolution in [`Self::reconcile_binding_typed_array_kind`].
    /// Only the shapes that have (or could have) a typed-array carrier are
    /// projected; an unresolved type variable / function / nested generic yields
    /// `None` (the caller then leaves the binding's capture untouched — no
    /// fabricated annotation, no Bool-default).
    fn inferred_type_to_annotation(
        elem: &shape_runtime::type_system::Type,
    ) -> Option<shape_ast::ast::TypeAnnotation> {
        use shape_runtime::type_system::Type;
        match elem {
            Type::Concrete(ann) => Some(ann.clone()),
            _ => None,
        }
    }

    /// Resolve an array receiver expression (`Identifier(name)`) to a
    /// [`TypedArrayKind`], if the receiver is a tracked array whose element
    /// type has a typed-array fast path.
    ///
    /// Walks the receiver name through:
    ///   1. Local slot type-tracker entry (`Vec<int>` etc).
    ///   2. Module binding type-tracker entry.
    ///
    /// Returns `None` for non-identifier receivers, for unresolved names, for
    /// receivers tracked as something other than a homogeneous typed array,
    /// and for element types that have no typed opcode kind. Callers must not
    /// infer a carrier from runtime bits in those cases.
    ///
    /// Phase 3.1 Agent 3 entry point — used by `compile_expr_method_call`,
    /// `compile_expr_index_access`, and `compile_expr_assign` to gate typed
    /// array opcode emission for `arr.push(x)`, `arr.pop()`, `arr.length`,
    /// `arr[i]`, and `arr[i] = x`.
    pub(crate) fn resolve_receiver_typed_array_kind(
        &self,
        receiver: &shape_ast::ast::Expr,
    ) -> Option<TypedArrayKind> {
        let name = match receiver {
            shape_ast::ast::Expr::Identifier(name, _) => name,
            _ => return None,
        };

        // Local slot first — ONLY if the slot was actually allocated as a
        // v2 typed array via `compile_expr_array`'s typed path. We CANNOT
        // simply trust the type-tracker name here because legacy untyped
        // literals (`let mut a = [1, 2, 3]`) get a `Vec<int>` type-tracker
        // entry too, but the runtime value is a NaN-boxed VMArray, not a
        // `*const TypedArray<i64>`. Emitting a typed get/set for that
        // would corrupt memory.
        if let Some(local_idx) = self.resolve_local(name) {
            if let Some(&kind) = self.v2_typed_array_locals.get(&local_idx) {
                return Some(kind);
            }
            return None;
        }

        // Module binding fallback (same restriction).
        let scoped_name = self
            .resolve_scoped_module_binding_name(name)
            .unwrap_or_else(|| name.to_string());
        if let Some(&binding_idx) = self.module_bindings.get(&scoped_name) {
            if let Some(&kind) = self.v2_typed_array_module_bindings.get(&binding_idx) {
                return Some(kind);
            }
        }

        None
    }

    /// Phase 4b Round 6 WS-1b W16.2-C residual (2026-05-21): resolve the
    /// `EmptyArrayAccumulatorKey` for a receiver name, if it is a registered
    /// bare empty-array accumulator awaiting an element kind.
    ///
    /// Mirrors the local-then-module-binding lookup order of
    /// [`resolve_receiver_typed_array_kind`].
    fn empty_array_accumulator_key(
        &self,
        recv_name: &str,
    ) -> Option<super::EmptyArrayAccumulatorKey> {
        if let Some(local_idx) = self.resolve_local(recv_name) {
            let key = super::EmptyArrayAccumulatorKey::Local(local_idx);
            return self
                .empty_array_accumulators
                .contains_key(&key)
                .then_some(key);
        }
        let scoped_name = self
            .resolve_scoped_module_binding_name(recv_name)
            .unwrap_or_else(|| recv_name.to_string());
        if let Some(&binding_idx) = self.module_bindings.get(&scoped_name) {
            let key = super::EmptyArrayAccumulatorKey::ModuleBinding(binding_idx);
            return self
                .empty_array_accumulators
                .contains_key(&key)
                .then_some(key);
        }
        None
    }

    /// Resolve the `TypedArrayKind` an `arr.push(arg)` argument contributes
    /// WITHOUT compiling `arg` — from a structural producer-side type proof
    /// (ADR-006 §2.7.5; never from runtime bits, never Bool-defaulted).
    ///
    /// A literal argument is its own proof (`1` → `I64`, `"x"` → `String`,
    /// …); a non-literal resolves through `concrete_type_for_expr` (the
    /// type tracker — a range-loop counter is typed `int`, an
    /// iterator-bound loop variable carries its element type, …). Returns
    /// `None` when no kind is structurally provable — the caller then
    /// compiles `arg` and reads the post-compile numeric / storage-hint
    /// proof via [`push_element_kind_from_compiled_arg`].
    fn structural_push_argument_typed_array_kind(
        &self,
        arg: &shape_ast::ast::Expr,
    ) -> Option<TypedArrayKind> {
        use shape_ast::ast::{Expr, Literal};
        if let Expr::Literal(lit, _) = arg {
            match lit {
                Literal::Int(_) => return Some(TypedArrayKind::I64),
                Literal::Number(_) => return Some(TypedArrayKind::F64),
                Literal::Decimal(_) => return Some(TypedArrayKind::Decimal),
                Literal::Bool(_) => return Some(TypedArrayKind::Bool),
                Literal::String(_) => return Some(TypedArrayKind::String),
                _ => {}
            }
        }
        super::monomorphization::type_resolution::concrete_type_for_expr(self, arg)
            .and_then(|ct| should_use_typed_array(&ct))
    }

    /// Resolve the `TypedArrayKind` for a just-compiled push argument `arg`.
    ///
    /// U4-4: the numeric kind is derived from the one resolved Type
    /// (`numeric_type_of(arg)` → `infer_expr_type`), not the deleted
    /// `last_expr_numeric_type` register — covering numeric literals /
    /// operations (`j * 2`, `x + 1`). `last_expr_type_info`'s
    /// `storage_hint == Bool` still covers comparison results (a separate
    /// non-numeric carrier). Returns `None` when no scalar kind is proven.
    fn push_element_kind_from_compiled_arg(
        &mut self,
        arg: &shape_ast::ast::Expr,
    ) -> Option<TypedArrayKind> {
        if let Some(nt) = self.numeric_type_of(arg) {
            return Some(typed_array_kind_from_numeric_type(nt));
        }
        if let Some(info) = &self.last_expr_type_info {
            if info.storage_hint == Some(crate::type_tracking::NativeKind::Bool) {
                return Some(TypedArrayKind::Bool);
            }
        }
        None
    }

    /// Patch a pending empty-array accumulator's placeholder allocator to the
    /// typed `NewTypedArray*` opcode for `kind`, and promote the binding into
    /// `v2_typed_array_locals` / `v2_typed_array_module_bindings`.
    ///
    /// The runtime element kind is STAMPED into the bytecode at compile time
    /// (ADR-006 §2.7.5) — the placeholder `NewArray(0)` becomes
    /// `kind.new_opcode()` with `Count(0)` capacity; the typed array grows
    /// via `TypedArrayPush*`. After this call the accumulator is no longer
    /// pending and `resolve_receiver_typed_array_kind` reports `kind`.
    fn finalize_empty_array_accumulator_kind(
        &mut self,
        key: super::EmptyArrayAccumulatorKey,
        kind: TypedArrayKind,
    ) {
        let acc = self
            .empty_array_accumulators
            .remove(&key)
            .expect("caller verified key presence");
        self.program.instructions[acc.alloc_instr_idx] = crate::bytecode::Instruction::new(
            kind.new_opcode(),
            Some(crate::bytecode::Operand::Count(0)),
        );
        // The resolved element type and the `Array<elem>` carrier type — the
        // same stamps the annotated `let mut xs: Array<T> = []` path records,
        // so a downstream `xs[i]` index access / `.method()` dispatch on the
        // promoted accumulator resolves through the type tracker exactly as
        // it would for an annotated binding (ADR-006 §2.7.5).
        let elem_ct = concrete_type_for_typed_array_kind(kind);
        let array_ct = shape_value::v2::ConcreteType::Array(Box::new(elem_ct.clone()));
        let array_type_name = vec_type_name_for_typed_array_kind(kind);
        match key {
            super::EmptyArrayAccumulatorKey::Local(local_idx) => {
                self.v2_typed_array_locals.insert(local_idx, kind);
                self.set_local_type_info(local_idx, array_type_name);
                crate::compiler::monomorphization::type_resolution::record_binding_concrete_fact(
                    self,
                    crate::compiler::monomorphization::type_resolution::BindingInitializerTarget::Local(local_idx),
                    array_ct,
                    crate::compiler::BindingConcreteFactSource::EmptyArrayAccumulator,
                );
            }
            super::EmptyArrayAccumulatorKey::ModuleBinding(binding_idx) => {
                self.v2_typed_array_module_bindings
                    .insert(binding_idx, kind);
                self.set_module_binding_type_info(binding_idx, array_type_name);
                crate::compiler::monomorphization::type_resolution::record_binding_concrete_fact(
                    self,
                    crate::compiler::monomorphization::type_resolution::BindingInitializerTarget::ModuleBinding(binding_idx),
                    array_ct,
                    crate::compiler::BindingConcreteFactSource::EmptyArrayAccumulator,
                );
            }
        }
    }

    /// Phase 4b Round 6 WS-1b W16.2-C residual (2026-05-21): compile the
    /// FIRST `arr.push(arg)` on a bare empty-array accumulator.
    ///
    /// When `recv_name` is a registered empty-array accumulator
    /// (`let mut out = []`, no annotation), this resolves the element
    /// `TypedArrayKind` from `arg`'s producer-side type proof, patches the
    /// placeholder `NewArray(0)` allocator to the typed `NewTypedArray*`
    /// opcode, promotes the binding, and emits the typed push — leaving the
    /// pushed-into array on the stack as the expression result.
    ///
    /// Returns `Ok(true)` when `recv_name` was a pending accumulator and the
    /// push was fully emitted; `Ok(false)` when it was not (the caller falls
    /// through to its normal push path). A genuinely un-resolvable element
    /// type is a clean structured compile error.
    ///
    /// Element-kind resolution is two-tier: a structural proof
    /// ([`structural_push_argument_typed_array_kind`]) when `arg` is a
    /// literal or a type-tracked identifier — which also lets string /
    /// decimal literals route through `compile_typed_array_element_value`'s
    /// `NewStringV2` / `NewDecimalV2` carrier; otherwise `arg` is compiled
    /// and the kind read from the post-compile numeric / storage-hint proof
    /// (covers `j * 2`, `x + 1`, comparison results — all scalar). The
    /// compiled value is then ordered under the array via `Swap`.
    pub(crate) fn compile_first_push_to_empty_accumulator(
        &mut self,
        recv_name: &str,
        arg: &shape_ast::ast::Expr,
        receiver_loc: Option<shape_ast::error::SourceLocation>,
    ) -> shape_ast::error::Result<bool> {
        let Some(key) = self.empty_array_accumulator_key(recv_name) else {
            return Ok(false);
        };
        // Immutability check — `let out = []` (no `mut`) cannot be pushed
        // into. Runs before any emission so the diagnostic is clean. On
        // failure, drop the accumulator entry so the end-of-compilation
        // finalizer does not ALSO emit a redundant "never pushed" error —
        // the immutability error is the single accurate diagnostic.
        if let Err(e) = self.check_named_binding_write_allowed(recv_name, receiver_loc) {
            self.empty_array_accumulators.remove(&key);
            return Err(e);
        }

        // Tier 1: structural resolution (literal / type-tracked identifier).
        if let Some(kind) = self.structural_push_argument_typed_array_kind(arg) {
            self.finalize_empty_array_accumulator_kind(key, kind);
            self.record_pushed_element_concrete_type(recv_name, arg);
            self.emit_load_accumulator_binding(key);
            self.compile_typed_array_element_value(kind, arg)?;
            self.emit(crate::bytecode::Instruction::simple(kind.push_opcode()));
            self.emit_load_accumulator_binding(key);
            return Ok(true);
        }

        // Tier 2: compile `arg`, then read the post-compile numeric /
        // storage-hint proof. Non-structural push arguments are scalar
        // numeric / bool expressions (`j * 2`, `x + 1`, `a < b`) — never a
        // string / decimal that would need the `NewStringV2` carrier.
        self.compile_expr(arg)?;
        let Some(kind) = self.push_element_kind_from_compiled_arg(arg) else {
            let acc = &self.empty_array_accumulators[&key];
            return Err(shape_ast::error::ShapeError::SemanticError {
                message: format!(
                    "cannot determine the element type of empty array \
                     `{}`. The array is created empty with no `Array<T>` \
                     annotation, so its element type must come from the \
                     first `.push(...)` — but the type of the value pushed \
                     here is not statically known. Strict typing requires a \
                     proven concrete element type: annotate the binding \
                     (`let mut {}: Array<T> = []`) or push a value whose \
                     type the compiler can resolve.",
                    acc.var_name, acc.var_name,
                ),
                location: acc.literal_loc.clone(),
            });
        };
        self.finalize_empty_array_accumulator_kind(key, kind);
        self.record_pushed_element_concrete_type(recv_name, arg);
        // Stack: [value]. The typed push needs [arr, value] — load the
        // array and swap it under the already-compiled value.
        self.emit_load_accumulator_binding(key);
        self.emit(crate::bytecode::Instruction::simple(
            crate::bytecode::OpCode::Swap,
        ));
        self.emit(crate::bytecode::Instruction::simple(kind.push_opcode()));
        self.emit_load_accumulator_binding(key);
        Ok(true)
    }

    /// Emit a `LoadLocal` / `LoadModuleBinding` for an empty-array
    /// accumulator's binding slot.
    fn emit_load_accumulator_binding(&mut self, key: super::EmptyArrayAccumulatorKey) {
        match key {
            super::EmptyArrayAccumulatorKey::Local(local_idx) => {
                self.emit(crate::bytecode::Instruction::new(
                    crate::bytecode::OpCode::LoadLocal,
                    Some(crate::bytecode::Operand::Local(local_idx)),
                ));
            }
            super::EmptyArrayAccumulatorKey::ModuleBinding(binding_idx) => {
                self.emit(crate::bytecode::Instruction::new(
                    crate::bytecode::OpCode::LoadModuleBinding,
                    Some(crate::bytecode::Operand::ModuleBinding(binding_idx)),
                ));
            }
        }
    }

    /// Phase 4b Round 6 WS-1b W16.2-C residual (2026-05-21): surface-and-stop
    /// for bare empty-array accumulators whose element type was never
    /// resolved.
    ///
    /// A `let mut out = []` (no annotation) that is never pushed to has a
    /// genuinely un-resolvable element type — there is no runtime untyped
    /// array carrier, so the placeholder `NewArray(0)` would SURFACE. Rather
    /// than ship that internal-jargon dump, emit a clean structured compile
    /// error. Called once per compilation unit after all code is compiled.
    pub(crate) fn finalize_unresolved_empty_array_accumulators(
        &mut self,
    ) -> shape_ast::error::Result<()> {
        if let Some((_, acc)) = self.empty_array_accumulators.iter().next() {
            let err = shape_ast::error::ShapeError::SemanticError {
                message: format!(
                    "empty array `{}` has an un-resolvable element type. \
                     It is created empty (`[]`) with no `Array<T>` \
                     annotation and is never pushed to, so the compiler \
                     cannot prove what element type it holds. Strict typing \
                     requires a known concrete element type: add an \
                     annotation (`let {}: Array<T> = []`) or remove the \
                     unused binding.",
                    acc.var_name, acc.var_name,
                ),
                location: acc.literal_loc.clone(),
            };
            self.empty_array_accumulators.clear();
            return Err(err);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_value::v2::concrete_type::{EnumLayoutId, StructLayoutId};

    #[test]
    fn test_f64_maps_to_typed_array_f64() {
        assert_eq!(
            should_use_typed_array(&ConcreteType::F64),
            Some(TypedArrayKind::F64)
        );
    }

    #[test]
    fn test_i64_maps_to_typed_array_i64() {
        assert_eq!(
            should_use_typed_array(&ConcreteType::I64),
            Some(TypedArrayKind::I64)
        );
    }

    #[test]
    fn test_i32_maps_to_typed_array_i32() {
        assert_eq!(
            should_use_typed_array(&ConcreteType::I32),
            Some(TypedArrayKind::I32)
        );
    }

    #[test]
    fn test_bool_maps_to_typed_array_bool() {
        assert_eq!(
            should_use_typed_array(&ConcreteType::Bool),
            Some(TypedArrayKind::Bool)
        );
    }

    #[test]
    fn test_string_maps_to_typed_array_string() {
        // Wave 2 Round 3a' A2-followup-gate-flip (2026-05-14) — gate flipped
        // in lockstep with the Round 3a' v2-raw consumer arms.
        assert_eq!(
            should_use_typed_array(&ConcreteType::String),
            Some(TypedArrayKind::String)
        );
    }

    #[test]
    fn test_decimal_maps_to_typed_array_decimal() {
        // Wave 2 Round 3a' A2-followup-gate-flip (2026-05-14).
        assert_eq!(
            should_use_typed_array(&ConcreteType::Decimal),
            Some(TypedArrayKind::Decimal)
        );
    }

    #[test]
    fn test_struct_maps_to_typed_array_typed_object() {
        // Phase 4b Round 4 W16.2-A op_new_array-typed-object-element (2026-05-18).
        // ConcreteType::Struct(_) now routes to the v2-raw TypedArray<*const
        // TypedObjectStorage> fast path per ADR-006 §2.7.5 + audit §2.1.
        assert_eq!(
            should_use_typed_array(&ConcreteType::placeholder_struct(StructLayoutId(0))),
            Some(TypedArrayKind::TypedObject)
        );
        assert_eq!(
            should_use_typed_array(&ConcreteType::placeholder_struct(StructLayoutId(42))),
            Some(TypedArrayKind::TypedObject)
        );
    }

    #[test]
    fn test_enum_maps_to_typed_object() {
        // STAGE C2 (2026-06-17): enum elements reuse the W16.2-A TypedObject
        // carrier — enum values are TypedObjects at runtime (the enum
        // constructor emits `NewTypedObject` carrying
        // `NativeKind::Ptr(HeapKind::TypedObject)`). Pre-C2 this returned
        // `None`, which surfaced "cannot infer the element type of this array
        // literal" for `let xs: Array<Color> = [...]`.
        assert_eq!(
            should_use_typed_array(&ConcreteType::placeholder_enum(EnumLayoutId(0))),
            Some(TypedArrayKind::TypedObject)
        );
        assert_eq!(
            should_use_typed_array(&ConcreteType::placeholder_enum(EnumLayoutId(7))),
            Some(TypedArrayKind::TypedObject)
        );
    }

    #[test]
    fn test_nested_array_is_not_direct_scalar_mapper_kind() {
        // `Array<Array<int>>` is handled by the array-literal structural branch
        // as `TypedArrayKind::TypedArray`, not by the scalar ConcreteType mapper.
        let nested = ConcreteType::Array(Box::new(ConcreteType::I64));
        assert_eq!(should_use_typed_array(&nested), None);
    }

    #[test]
    fn test_u8_maps_to_typed_array_u8() {
        // W12 S1 (2026-05-13) — U8 now has a typed array opcode kind.
        assert_eq!(
            should_use_typed_array(&ConcreteType::U8),
            Some(TypedArrayKind::U8)
        );
    }

    #[test]
    fn test_i8_maps_to_typed_array_i8() {
        assert_eq!(
            should_use_typed_array(&ConcreteType::I8),
            Some(TypedArrayKind::I8)
        );
    }

    #[test]
    fn test_i16_maps_to_typed_array_i16() {
        assert_eq!(
            should_use_typed_array(&ConcreteType::I16),
            Some(TypedArrayKind::I16)
        );
    }

    #[test]
    fn test_u16_maps_to_typed_array_u16() {
        assert_eq!(
            should_use_typed_array(&ConcreteType::U16),
            Some(TypedArrayKind::U16)
        );
    }

    #[test]
    fn test_u32_maps_to_typed_array_u32() {
        assert_eq!(
            should_use_typed_array(&ConcreteType::U32),
            Some(TypedArrayKind::U32)
        );
    }

    #[test]
    fn test_f32_maps_to_typed_array_f32() {
        // Wave 2 Agent A1 (2026-05-14) — F32 scalar monomorphization.
        assert_eq!(
            should_use_typed_array(&ConcreteType::F32),
            Some(TypedArrayKind::F32)
        );
    }

    #[test]
    fn test_char_maps_to_typed_array_char() {
        // Wave 2 Agent A1 (2026-05-14) — Char scalar monomorphization.
        assert_eq!(
            should_use_typed_array(&ConcreteType::Char),
            Some(TypedArrayKind::Char)
        );
    }

    #[test]
    fn test_slot_kind_float32_maps_to_f32() {
        use crate::type_tracking::NativeKind;
        assert_eq!(
            should_use_typed_array_from_slot_kind(NativeKind::Float32),
            Some(TypedArrayKind::F32)
        );
    }

    #[test]
    fn test_slot_kind_char_maps_to_char() {
        use crate::type_tracking::NativeKind;
        assert_eq!(
            should_use_typed_array_from_slot_kind(NativeKind::Char),
            Some(TypedArrayKind::Char)
        );
    }

    #[test]
    fn test_u64_falls_back_to_legacy() {
        // Per S1 reopen (2026-05-13), `Array<u64>` deliberately falls
        // back to the legacy NaN-boxed `NewArray` path: `NativeKind::UInt64`
        // is overloaded between scalar u64 and v2-typed-array-pointer
        // carrier at HEAD, so a typed-array fast path would route both
        // through the same overloaded discriminator. Deferred to S1.5
        // pending §2.7.7/Q9 parallel-kind-track extension.
        assert_eq!(should_use_typed_array(&ConcreteType::U64), None);
    }

    #[test]
    fn test_option_falls_back_to_legacy() {
        let opt = ConcreteType::Option(Box::new(ConcreteType::I64));
        assert_eq!(should_use_typed_array(&opt), None);
    }

    #[test]
    fn test_opcode_lookup_round_trip() {
        // Sanity check that all kinds expose all four opcodes.
        // U64 deliberately omitted — deferred to S1.5 per S1 reopen.
        // Wave 2 Agent A1 (2026-05-14) — F32 + Char added.
        // Wave 2 Agent A2 (2026-05-14) — String + Decimal added.
        // Phase 4b Round 4 W16.2-A (2026-05-18) — TypedObject added.
        for kind in [
            TypedArrayKind::F64,
            TypedArrayKind::I64,
            TypedArrayKind::I32,
            TypedArrayKind::Bool,
            TypedArrayKind::I8,
            TypedArrayKind::U8,
            TypedArrayKind::I16,
            TypedArrayKind::U16,
            TypedArrayKind::U32,
            TypedArrayKind::F32,
            TypedArrayKind::Char,
            TypedArrayKind::String,
            TypedArrayKind::Decimal,
            TypedArrayKind::TypedObject,
        ] {
            let _ = kind.new_opcode();
            let _ = kind.get_opcode();
            let _ = kind.push_opcode();
            let _ = kind.set_opcode();
        }
    }

    // ---- NativeKind variant ----

    #[test]
    fn test_slot_kind_float64_maps_to_f64() {
        use crate::type_tracking::NativeKind;
        assert_eq!(
            should_use_typed_array_from_slot_kind(NativeKind::Float64),
            Some(TypedArrayKind::F64)
        );
    }

    #[test]
    fn test_slot_kind_int64_maps_to_i64() {
        use crate::type_tracking::NativeKind;
        assert_eq!(
            should_use_typed_array_from_slot_kind(NativeKind::Int64),
            Some(TypedArrayKind::I64)
        );
    }

    #[test]
    fn test_slot_kind_int32_maps_to_i32() {
        use crate::type_tracking::NativeKind;
        assert_eq!(
            should_use_typed_array_from_slot_kind(NativeKind::Int32),
            Some(TypedArrayKind::I32)
        );
    }

    #[test]
    fn test_slot_kind_bool_maps_to_bool() {
        use crate::type_tracking::NativeKind;
        assert_eq!(
            should_use_typed_array_from_slot_kind(NativeKind::Bool),
            Some(TypedArrayKind::Bool)
        );
    }

    #[test]
    fn test_slot_kind_string_maps_to_string() {
        // Wave 2 Round 3a' A2-followup-gate-flip (2026-05-14) — legacy
        // `NativeKind::String` (Arc<String> carrier label) routes here from
        // `typed_array_from_annotation("string")`; literal-upgrade transition
        // to `StringV2` is downstream A2-followup-producer-cascade territory.
        use crate::type_tracking::NativeKind;
        assert_eq!(
            should_use_typed_array_from_slot_kind(NativeKind::String),
            Some(TypedArrayKind::String)
        );
    }

    #[test]
    fn test_slot_kind_stringv2_maps_to_string() {
        // Wave 2 Round 3a' A2-followup-gate-flip — v2-raw `StringV2` label
        // (matches `TypedArrayPushString` consumer kind invariant).
        use crate::type_tracking::NativeKind;
        assert_eq!(
            should_use_typed_array_from_slot_kind(NativeKind::StringV2),
            Some(TypedArrayKind::String)
        );
    }

    #[test]
    fn test_slot_kind_decimalv2_maps_to_decimal() {
        // Wave 2 Round 3a' A2-followup-gate-flip — v2-raw `DecimalV2` label
        // (matches `TypedArrayPushDecimal` consumer kind invariant).
        use crate::type_tracking::NativeKind;
        assert_eq!(
            should_use_typed_array_from_slot_kind(NativeKind::DecimalV2),
            Some(TypedArrayKind::Decimal)
        );
    }

    // `NativeKind::Unknown` and `NativeKind::Dynamic` were deleted per
    // ADR-006 §2.7.5.1 — every NativeKind in compiled bytecode must be
    // proven. The "falls back to None" tests for those variants were
    // removed (the variants no longer exist; nothing to fall back from).

    #[test]
    fn test_slot_kind_int8_maps_to_i8() {
        // W12 S1 (2026-05-13) — sized integer kinds now have typed opcode
        // monomorphizations.
        use crate::type_tracking::NativeKind;
        assert_eq!(
            should_use_typed_array_from_slot_kind(NativeKind::Int8),
            Some(TypedArrayKind::I8)
        );
    }

    #[test]
    fn test_slot_kind_uint8_maps_to_u8() {
        use crate::type_tracking::NativeKind;
        assert_eq!(
            should_use_typed_array_from_slot_kind(NativeKind::UInt8),
            Some(TypedArrayKind::U8)
        );
    }

    #[test]
    fn test_slot_kind_int16_maps_to_i16() {
        use crate::type_tracking::NativeKind;
        assert_eq!(
            should_use_typed_array_from_slot_kind(NativeKind::Int16),
            Some(TypedArrayKind::I16)
        );
    }

    #[test]
    fn test_slot_kind_uint16_maps_to_u16() {
        use crate::type_tracking::NativeKind;
        assert_eq!(
            should_use_typed_array_from_slot_kind(NativeKind::UInt16),
            Some(TypedArrayKind::U16)
        );
    }

    #[test]
    fn test_slot_kind_uint32_maps_to_u32() {
        use crate::type_tracking::NativeKind;
        assert_eq!(
            should_use_typed_array_from_slot_kind(NativeKind::UInt32),
            Some(TypedArrayKind::U32)
        );
    }

    #[test]
    fn test_slot_kind_uint64_falls_back_to_legacy() {
        // Per S1 reopen (2026-05-13): `NativeKind::UInt64` is overloaded
        // between scalar u64 and v2-typed-array-pointer carrier at HEAD
        // (every `*mut TypedArray<T>` flows through `UInt64`). Routing
        // through a typed-array fast path here would conflate the two
        // shapes. Deferred to S1.5 pending §2.7.7/Q9 parallel-kind-track
        // extension that adds a discriminating variant.
        use crate::type_tracking::NativeKind;
        assert_eq!(
            should_use_typed_array_from_slot_kind(NativeKind::UInt64),
            None
        );
    }
}

// ──────────────────────────────────────────────────────────────────────
// Compile integration tests — verify `compile_expr_array` emits the
// correct opcode (`NewTypedArray*` vs legacy `NewArray`/`NewTypedArray`)
// for the array literal shapes called out in the Phase 3.1 deliverables.
// ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod compile_integration_tests {
    use crate::bytecode::{BytecodeProgram, OpCode};
    use crate::compiler::BytecodeCompiler;

    fn compile(src: &str) -> BytecodeProgram {
        let program = shape_ast::parser::parse_program(src).expect("parse should succeed");
        BytecodeCompiler::new()
            .compile_with_source(&program, src)
            .expect("compile should succeed")
    }

    fn compile_error(src: &str) -> String {
        let program = shape_ast::parser::parse_program(src).expect("parse should succeed");
        let err = BytecodeCompiler::new()
            .compile_with_source(&program, src)
            .expect_err("compile should fail");
        format!("{err:?}")
    }

    fn has_opcode(prog: &BytecodeProgram, op: OpCode) -> bool {
        prog.instructions.iter().any(|i| i.opcode == op)
    }

    // ──────────────────────────────────────────────────────────────────
    // W16.2-C (Round 6 WS-1) — spread / list-comprehension typed-array
    // accumulator construction.
    // ──────────────────────────────────────────────────────────────────

    #[test]
    fn ws1_int_comprehension_emits_typed_array() {
        let prog = compile("let c=[x for x in 0..5]\nc\n");
        assert!(
            has_opcode(&prog, OpCode::NewTypedArrayI64),
            "int comprehension accumulator must allocate via NewTypedArrayI64"
        );
        assert!(
            has_opcode(&prog, OpCode::TypedArrayPushI64),
            "int comprehension element push must use TypedArrayPushI64"
        );
        assert!(
            !has_opcode(&prog, OpCode::NewArray),
            "int comprehension must NOT emit the generic NewArray opcode"
        );
    }

    #[test]
    fn ws1_int_spread_emits_typed_array() {
        let prog = compile("let a=[1,2,3]\nlet b=[...a,4,5]\nb\n");
        assert!(
            has_opcode(&prog, OpCode::NewTypedArrayI64),
            "int spread accumulator must allocate via NewTypedArrayI64"
        );
        assert!(
            !has_opcode(&prog, OpCode::NewArray),
            "int spread must NOT emit the generic NewArray opcode"
        );
    }

    #[test]
    fn ws1_number_comprehension_emits_typed_array() {
        let prog = compile("let c=[x * 1.5 for x in 0..4]\nc\n");
        assert!(
            has_opcode(&prog, OpCode::NewTypedArrayF64),
            "number comprehension accumulator must allocate via NewTypedArrayF64"
        );
        assert!(
            has_opcode(&prog, OpCode::TypedArrayPushF64),
            "number comprehension element push must use TypedArrayPushF64"
        );
    }

    #[test]
    fn ws1_heterogeneous_spread_is_clean_compile_error() {
        // `[...intArr, "str"]` mixes an int-array spread with a string
        // tail — no homogeneous scalar element kind. Must surface a clean
        // SemanticError, NOT a runtime jargon dump.
        let src = "let a=[1,2,3]\nlet b=[...a,\"str\"]\nb\n";
        let program = shape_ast::parser::parse_program(src).expect("parse should succeed");
        let result = BytecodeCompiler::new().compile_with_source(&program, src);
        assert!(
            result.is_err(),
            "heterogeneous spread must be a compile error"
        );
        let msg = format!("{:?}", result.unwrap_err());
        assert!(
            msg.contains("spread element types could not be reconciled")
                || (msg.contains("int") && msg.contains("string")),
            "heterogeneous spread error must be the clean structured message, got: {msg}"
        );
    }

    // ──────────────────────────────────────────────────────────────────
    // W16.2-C residual (Round 6 WS-1b) — bare empty-array accumulator
    // (`let mut out = []`) whose element kind comes from downstream
    // `.push()`. The placeholder `NewArray(0)` must be patched to the
    // typed `NewTypedArray*` allocator with the kind proven at the first
    // push, and every push must be a typed `TypedArrayPush*`.
    // ──────────────────────────────────────────────────────────────────

    #[test]
    fn ws1b_bare_int_accumulator_emits_typed_array() {
        // `let mut out = []` then int pushes — the placeholder NewArray(0)
        // must be patched to NewTypedArrayI64, pushes to TypedArrayPushI64.
        let prog = compile("let mut out = []\nfor i in 0..3 { out.push(i) }\nout.len()\n");
        assert!(
            has_opcode(&prog, OpCode::NewTypedArrayI64),
            "bare int accumulator must allocate via NewTypedArrayI64"
        );
        assert!(
            has_opcode(&prog, OpCode::TypedArrayPushI64),
            "bare int accumulator push must use TypedArrayPushI64"
        );
        assert!(
            !has_opcode(&prog, OpCode::NewArray),
            "bare int accumulator must NOT leave a kind-erased NewArray"
        );
    }

    #[test]
    fn ws1b_bare_number_accumulator_emits_typed_array() {
        let prog = compile("let mut out = []\nout.push(1.5)\nout.push(2.5)\nout.len()\n");
        assert!(
            has_opcode(&prog, OpCode::NewTypedArrayF64),
            "bare number accumulator must allocate via NewTypedArrayF64"
        );
        assert!(
            has_opcode(&prog, OpCode::TypedArrayPushF64),
            "bare number accumulator push must use TypedArrayPushF64"
        );
        assert!(!has_opcode(&prog, OpCode::NewArray));
    }

    #[test]
    fn ws1b_bare_string_accumulator_emits_typed_array() {
        let prog = compile("let mut out = []\nout.push(\"a\")\nout[0]\n");
        assert!(
            has_opcode(&prog, OpCode::NewTypedArrayString),
            "bare string accumulator must allocate via NewTypedArrayString"
        );
        assert!(
            has_opcode(&prog, OpCode::TypedArrayPushString),
            "bare string accumulator push must use TypedArrayPushString"
        );
        // A string literal element must be produced via NewStringV2 so the
        // TypedArrayPushString strict-kind check accepts the StringV2 carrier.
        assert!(
            has_opcode(&prog, OpCode::NewStringV2),
            "bare string accumulator literal element must use NewStringV2"
        );
        assert!(!has_opcode(&prog, OpCode::NewArray));
    }

    #[test]
    fn ws1b_bare_accumulator_in_function_body_emits_typed_array() {
        // Function-local `let mut acc = []` — exercises the local-slot
        // accumulator path (distinct from the top-level module-binding path).
        let prog = compile(
            "fn build() -> int {\n  let mut acc = []\n  acc.push(10)\n  acc.push(20)\n  acc[0] + acc[1]\n}\nbuild()\n",
        );
        assert!(
            has_opcode(&prog, OpCode::NewTypedArrayI64),
            "function-local int accumulator must allocate via NewTypedArrayI64"
        );
        assert!(!has_opcode(&prog, OpCode::NewArray));
    }

    #[test]
    fn empty_array_reassign_to_typed_local_emits_typed_array() {
        // STAGE T4 (V3-S5 empty-array reassign, 2026-06-20): reassigning a
        // bare empty literal (`a = []`) to a binding whose proven element type
        // is `Array<int>` must recover that element type from the type tracker
        // and lower the empty literal to the typed `NewTypedArrayI64`
        // allocator (count 0) — NOT the generic `NewArray(0)` that SURFACEd
        // `op_new_array(0)` at runtime mid-program. Mirrors the var-decl
        // annotation hand-off (statements.rs:967).
        let prog = compile("let mut a: Array<int> = [1,2,3]\na = []\na.len()\n");
        assert!(
            has_opcode(&prog, OpCode::NewTypedArrayI64),
            "empty-array reassign to Array<int> must allocate via NewTypedArrayI64"
        );
        assert!(
            !has_opcode(&prog, OpCode::NewArray),
            "empty-array reassign must NOT emit the generic NewArray op_new_array(0) placeholder"
        );
    }

    #[test]
    fn empty_array_reassign_to_module_string_binding_emits_typed_array() {
        // Module-binding string variant (the resource-mgmt gate's
        // `let mut LOG: Array<string> = []` cleared via `LOG = []`).
        let prog = compile(
            "let mut LOG: Array<string> = []\nfn clear() { LOG = [] }\nLOG.push(\"x\")\nclear()\nLOG.len()\n",
        );
        assert!(
            has_opcode(&prog, OpCode::NewTypedArrayString),
            "empty-array reassign to module Array<string> must allocate via NewTypedArrayString"
        );
        assert!(
            !has_opcode(&prog, OpCode::NewArray),
            "module-binding empty-array reassign must NOT emit the generic NewArray placeholder"
        );
    }

    #[test]
    fn ws1b_bare_accumulator_complex_push_arg_emits_typed_array() {
        // `out.push(i * i)` — a non-literal scalar push argument. The first
        // push resolves the kind via the post-compile numeric proof (Tier 2).
        let prog = compile("let mut out = []\nfor i in 0..5 { out.push(i * i) }\nout.len()\n");
        assert!(
            has_opcode(&prog, OpCode::NewTypedArrayI64),
            "accumulator with a complex int push arg must allocate via NewTypedArrayI64"
        );
        assert!(!has_opcode(&prog, OpCode::NewArray));
    }

    #[test]
    fn ws1b_never_pushed_empty_array_is_clean_compile_error() {
        // A bare empty array that is never pushed to and never annotated has
        // a genuinely un-resolvable element type — a clean structured
        // compile error, NOT a runtime jargon dump.
        let src = "let mut never = []\nnever\n";
        let program = shape_ast::parser::parse_program(src).expect("parse should succeed");
        let result = BytecodeCompiler::new().compile_with_source(&program, src);
        assert!(
            result.is_err(),
            "never-pushed bare empty array must be a compile error"
        );
        let msg = format!("{:?}", result.unwrap_err());
        assert!(
            msg.contains("un-resolvable element type"),
            "never-pushed empty array error must be the clean structured message, got: {msg}"
        );
    }

    #[test]
    fn ws1b_heterogeneous_accumulator_push_is_clean_compile_error() {
        // `out.push(1)` then `out.push("x")` — the second push's element
        // type disagrees with the accumulator's resolved `int` kind. Must
        // surface a clean SemanticError, not a silent wrong result.
        let src = "let mut out = []\nout.push(1)\nout.push(\"x\")\nout.len()\n";
        let program = shape_ast::parser::parse_program(src).expect("parse should succeed");
        let result = BytecodeCompiler::new().compile_with_source(&program, src);
        assert!(
            result.is_err(),
            "heterogeneous accumulator push must be a compile error"
        );
        let msg = format!("{:?}", result.unwrap_err());
        assert!(
            (msg.contains("type mismatch") && msg.contains("element type"))
                || msg.contains(
                    "cannot push a `string` value into an array whose element type is `int`"
                ),
            "heterogeneous accumulator error must be the clean structured message, got: {msg}"
        );
    }

    #[test]
    fn test_number_literal_emits_new_typed_array_f64() {
        // Annotated: `let arr: Array<number> = [1.0, 2.0, 3.0]` -> NewTypedArrayF64
        let prog = compile(
            r#"
            let arr: Array<number> = [1.0, 2.0, 3.0]
            arr
            "#,
        );
        assert!(
            has_opcode(&prog, OpCode::NewTypedArrayF64),
            "expected NewTypedArrayF64 in instruction stream"
        );
        assert!(
            has_opcode(&prog, OpCode::TypedArrayPushF64),
            "expected TypedArrayPushF64 in instruction stream"
        );
    }

    #[test]
    fn test_int_literal_emits_new_typed_array_i64() {
        // Annotated: `let arr: Array<int> = [1, 2, 3]` -> NewTypedArrayI64
        // Bare literals (`[1, 2, 3]` with no annotation) deliberately
        // stay on the legacy `NewTypedArray` path because runtime tests
        // depend on the v1 NaN-boxed shape.
        let prog = compile(
            r#"
            let arr: Array<int> = [1, 2, 3]
            arr
            "#,
        );
        assert!(
            has_opcode(&prog, OpCode::NewTypedArrayI64),
            "expected NewTypedArrayI64 in instruction stream"
        );
        assert!(
            has_opcode(&prog, OpCode::TypedArrayPushI64),
            "expected TypedArrayPushI64 in instruction stream"
        );
    }

    #[test]
    fn test_bool_literal_emits_new_typed_array_bool() {
        // Annotated: `let arr: Array<bool> = [true, false]` -> NewTypedArrayBool
        let prog = compile(
            r#"
            let arr: Array<bool> = [true, false]
            arr
            "#,
        );
        assert!(
            has_opcode(&prog, OpCode::NewTypedArrayBool),
            "expected NewTypedArrayBool in instruction stream"
        );
        assert!(
            has_opcode(&prog, OpCode::TypedArrayPushBool),
            "expected TypedArrayPushBool in instruction stream"
        );
    }

    #[test]
    fn test_typed_int_literal_emits_new_typed_array_i32() {
        // Annotated: `let arr: Array<i32> = [1, 2, 3]` -> NewTypedArrayI32
        let prog = compile(
            r#"
            let arr: Array<i32> = [1, 2, 3]
            arr
            "#,
        );
        assert!(
            has_opcode(&prog, OpCode::NewTypedArrayI32),
            "expected NewTypedArrayI32 in instruction stream"
        );
        assert!(
            has_opcode(&prog, OpCode::TypedArrayPushI32),
            "expected TypedArrayPushI32 in instruction stream"
        );
    }

    #[test]
    fn test_heterogeneous_literal_is_clean_compile_error() {
        // `[1, "x", true]` is heterogeneous and has no union/tuple/object
        // carrier. Strict typing rejects it at compile time instead of
        // falling back to a dynamic legacy array.
        let msg = compile_error("[1, \"x\", true]");
        assert!(
            msg.contains("int") && (msg.contains("string") || msg.contains("bool")),
            "heterogeneous literal must be rejected with a static type diagnostic, got: {msg}"
        );
    }

    #[test]
    fn test_struct_array_emits_typed_object_array() {
        // Phase 4b Round 4 W16.2-A op_new_array-typed-object-element (2026-05-18).
        // Pre-W16.2-A: `let arr = [Point{...}, Point{...}]` fell back to
        // legacy `NewArray` (no typed opcode for struct elements).
        // Post-W16.2-A: routes through the v2-raw `TypedArray<*const
        // TypedObjectStorage>` carrier via the new
        // `TypedArrayKind::TypedObject` arm + new opcodes.
        let prog = compile(
            r#"
            type Point { x: int, y: int }
            let arr = [Point { x: 1, y: 2 }, Point { x: 3, y: 4 }]
            arr
            "#,
        );
        assert!(
            has_opcode(&prog, OpCode::NewTypedArrayTypedObject),
            "struct-element array must emit NewTypedArrayTypedObject per W16.2-A"
        );
        assert!(
            has_opcode(&prog, OpCode::TypedArrayPushTypedObject),
            "struct-element array must emit per-element TypedArrayPushTypedObject"
        );
        assert!(
            !has_opcode(&prog, OpCode::NewArray),
            "struct-element array must NOT fall back to legacy NewArray"
        );
        assert!(
            !has_opcode(&prog, OpCode::NewTypedArrayF64),
            "struct-element array must not emit NewTypedArrayF64"
        );
        assert!(
            !has_opcode(&prog, OpCode::NewTypedArrayI64),
            "struct-element array must not emit NewTypedArrayI64"
        );
    }

    #[test]
    fn test_empty_literal_falls_back_to_legacy_new_array() {
        // Empty literal has no element type the compiler can prove —
        // fall back to legacy `NewArray`.
        let prog = compile("[]");
        assert!(
            has_opcode(&prog, OpCode::NewArray),
            "empty array must emit legacy NewArray (no element type)"
        );
        assert!(
            !has_opcode(&prog, OpCode::NewTypedArrayF64),
            "empty array must not emit a typed-array opcode"
        );
    }

    // ──────────────────────────────────────────────────────────────────
    // Nested-array literal policy.
    //
    // Homogeneous nested arrays use the dedicated typed nested-array carrier:
    // the outer array stores inner typed-array pointers as
    // `TypedArray<*const TypedArrayElem>`, while each row keeps its own scalar
    // typed carrier. A scalar annotation such as `Array<number>` is
    // structurally wrong for `[[...]]` and must reject cleanly.
    // ──────────────────────────────────────────────────────────────────

    #[test]
    fn test_nested_array_literal_mat_number_uses_nested_typed_carrier() {
        // `Mat<number>` is structurally a numeric row matrix: the outer array
        // uses the nested typed-array carrier, and inner rows use F64.
        let prog = compile(
            r#"
            let m: Mat<number> = [[1.0, 2.0], [3.0, 4.0]]
            m
            "#,
        );
        assert!(
            has_opcode(&prog, OpCode::NewTypedArrayNested),
            "nested `Mat<number>` literal must emit NewTypedArrayNested, got opcodes: {:?}",
            prog.instructions
                .iter()
                .map(|i| i.opcode)
                .collect::<Vec<_>>()
        );
        assert!(
            has_opcode(&prog, OpCode::NewTypedArrayF64),
            "nested `Mat<number>` rows must emit NewTypedArrayF64, got opcodes: {:?}",
            prog.instructions
                .iter()
                .map(|i| i.opcode)
                .collect::<Vec<_>>()
        );
        assert!(
            !has_opcode(&prog, OpCode::NewArray),
            "nested `Mat<number>` literal must not emit legacy NewArray, got opcodes: {:?}",
            prog.instructions
                .iter()
                .map(|i| i.opcode)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_nested_array_literal_array_number_annotation_rejects() {
        // `Array<number>` promises scalar number elements; `[[...]]`
        // contributes array elements. There is no dynamic fallback that can
        // make that structurally correct.
        let msg = compile_error(
            r#"
            let m: Array<number> = [[1.0, 2.0], [3.0, 4.0]]
            m
            "#,
        );
        assert!(
            msg.contains("type mismatch")
                && msg.contains("Array<f64>")
                && msg.contains("Array<array_f64>")
                && msg.contains("nested arrays require"),
            "nested `Array<number>` annotation must reject structurally, got: {msg}"
        );
    }

    #[test]
    fn test_non_nested_vec_number_still_emits_new_typed_array_f64() {
        // Regression safety: the R5.4B nested-array guard must NOT
        // disturb the typed fast path for a plain
        // `let v: Vec<number> = [1.0, 2.0, 3.0]`.
        let prog = compile(
            r#"
            let v: Vec<number> = [1.0, 2.0, 3.0]
            v
            "#,
        );
        assert!(
            has_opcode(&prog, OpCode::NewTypedArrayF64),
            "non-nested `Vec<number>` must still emit NewTypedArrayF64, got opcodes: {:?}",
            prog.instructions
                .iter()
                .map(|i| i.opcode)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_non_nested_vec_int_still_emits_new_typed_array_i64() {
        // Same regression safety check for `Vec<int>`.
        let prog = compile(
            r#"
            let v: Vec<int> = [1, 2, 3]
            v
            "#,
        );
        assert!(
            has_opcode(&prog, OpCode::NewTypedArrayI64),
            "non-nested `Vec<int>` must still emit NewTypedArrayI64, got opcodes: {:?}",
            prog.instructions
                .iter()
                .map(|i| i.opcode)
                .collect::<Vec<_>>()
        );
    }

    // ──────────────────────────────────────────────────────────────────
    // v2 Phase 3.1 (Agent 3): method dispatch / index access / index
    // assignment / property access fast paths.
    //
    // These tests verify that `arr.push(x)`, `arr[i]`, `arr[i] = x`,
    // and `arr.length` lower to typed array opcodes when the receiver
    // is a tracked array with a homogeneous, typed-opcode-backed element
    // type. They also verify the fail-soft fallback to generic
    // `GetProp`/`SetProp`/`Length`/`ArrayPushLocal` when the element
    // type is unknown.
    // ──────────────────────────────────────────────────────────────────

    #[test]
    fn test_index_access_typed_int_array_emits_typed_get_i64() {
        // `let arr: Array<int> = [1, 2, 3]; arr[0]` -> TypedArrayGetI64
        let prog = compile(
            r#"
            let arr: Array<int> = [1, 2, 3]
            arr[0]
            "#,
        );
        assert!(
            has_opcode(&prog, OpCode::TypedArrayGetI64),
            "expected TypedArrayGetI64 in instruction stream, got: {:?}",
            prog.instructions
                .iter()
                .map(|i| i.opcode)
                .collect::<Vec<_>>()
        );
        // Generic GetProp must NOT be present for the index access.
        // (Legacy NewTypedArrayI64 from the literal IS expected.)
    }

    #[test]
    fn test_index_access_typed_number_array_emits_typed_get_f64() {
        let prog = compile(
            r#"
            let arr: Array<number> = [1.0, 2.0, 3.0]
            arr[0]
            "#,
        );
        assert!(
            has_opcode(&prog, OpCode::TypedArrayGetF64),
            "expected TypedArrayGetF64 in instruction stream"
        );
    }

    #[test]
    fn test_index_access_typed_bool_array_emits_typed_get_bool() {
        let prog = compile(
            r#"
            let arr: Array<bool> = [true, false]
            arr[0]
            "#,
        );
        assert!(
            has_opcode(&prog, OpCode::TypedArrayGetBool),
            "expected TypedArrayGetBool in instruction stream"
        );
    }

    #[test]
    fn test_index_access_heterogeneous_array_is_compile_error() {
        // Heterogeneous arrays have no typed carrier and no runtime fallback.
        let msg = compile_error(
            r#"
            let arr = [1, "x", true]
            arr[0]
            "#,
        );
        assert!(
            msg.contains("int") && (msg.contains("string") || msg.contains("bool")),
            "heterogeneous array index source must reject before GetProp fallback, got: {msg}"
        );
    }

    #[test]
    fn test_push_typed_number_array_emits_typed_push_f64() {
        // `let arr: Array<number> = [1.0]; arr.push(2.0)` -> TypedArrayPushF64
        // Note: the literal `[1.0]` already emits TypedArrayPushF64 for the
        // initial elements, so we additionally check that there's a
        // TypedArrayPushF64 after a LoadLocal — i.e. the explicit push call.
        let prog = compile(
            r#"
            let mut arr: Array<number> = [1.0]
            arr.push(2.0)
            "#,
        );
        assert!(
            has_opcode(&prog, OpCode::TypedArrayPushF64),
            "expected TypedArrayPushF64 for arr.push(2.0)"
        );
        assert!(
            !has_opcode(&prog, OpCode::ArrayPushLocal),
            "should not emit legacy ArrayPushLocal when typed-array path applies"
        );
    }

    #[test]
    fn test_push_typed_int_array_emits_typed_push_i64() {
        let prog = compile(
            r#"
            let mut arr: Array<int> = [1]
            arr.push(2)
            "#,
        );
        assert!(
            has_opcode(&prog, OpCode::TypedArrayPushI64),
            "expected TypedArrayPushI64 for arr.push(2)"
        );
        assert!(
            !has_opcode(&prog, OpCode::ArrayPushLocal),
            "should not emit legacy ArrayPushLocal when typed-array path applies"
        );
    }

    #[test]
    fn test_index_assign_typed_int_array_emits_typed_set_i64() {
        // `let mut arr: Array<int> = [1]; arr[0] = 99` -> TypedArraySetI64
        let prog = compile(
            r#"
            let mut arr: Array<int> = [1]
            arr[0] = 99
            "#,
        );
        assert!(
            has_opcode(&prog, OpCode::TypedArraySetI64),
            "expected TypedArraySetI64 in instruction stream"
        );
        assert!(
            !has_opcode(&prog, OpCode::SetLocalIndex),
            "should not emit legacy SetLocalIndex for typed array"
        );
    }

    #[test]
    fn test_index_assign_typed_number_array_emits_typed_set_f64() {
        let prog = compile(
            r#"
            let mut arr: Array<number> = [1.0]
            arr[0] = 99.0
            "#,
        );
        assert!(
            has_opcode(&prog, OpCode::TypedArraySetF64),
            "expected TypedArraySetF64 in instruction stream"
        );
    }

    #[test]
    fn test_length_typed_bool_array_emits_typed_array_len() {
        // `let arr: Array<bool> = [true]; arr.length` -> TypedArrayLen
        let prog = compile(
            r#"
            let arr: Array<bool> = [true]
            arr.length
            "#,
        );
        assert!(
            has_opcode(&prog, OpCode::TypedArrayLen),
            "expected TypedArrayLen for arr.length"
        );
        // Generic Length opcode should NOT be present.
        assert!(
            !has_opcode(&prog, OpCode::Length),
            "should not emit legacy Length for typed array"
        );
    }

    #[test]
    fn test_length_typed_int_array_emits_typed_array_len() {
        let prog = compile(
            r#"
            let arr: Array<int> = [1, 2, 3]
            arr.length
            "#,
        );
        assert!(
            has_opcode(&prog, OpCode::TypedArrayLen),
            "expected TypedArrayLen for arr.length"
        );
    }

    #[test]
    fn test_length_heterogeneous_array_is_compile_error() {
        let msg = compile_error(
            r#"
            let arr = [1, "x", true]
            arr.length
            "#,
        );
        assert!(
            msg.contains("int") && (msg.contains("string") || msg.contains("bool")),
            "heterogeneous array length source must reject before Length fallback, got: {msg}"
        );
    }

    // ──────────────────────────────────────────────────────────────────
    // Wave 3 Stabilize Round 1 V3-A2-followup-producer-cascade (2026-05-15)
    //
    // Per ADR-006 §2.7.5 stamp-at-compile-time + §2.7.24 Q25.A SUPERSEDED:
    // `Array<string>` / `Array<decimal>` literals must emit
    // `NewStringV2` / `NewDecimalV2` for the per-element literal-upgrade
    // path (the Round 3a' gate-flip's downstream pre-req). The element
    // value rides through the kinded stack with `NativeKind::StringV2` /
    // `NativeKind::DecimalV2`, satisfying the strict-kind check at
    // `v2_handlers/array.rs:687/703` (`TypedArrayPushString` /
    // `TypedArrayPushDecimal`).
    // ──────────────────────────────────────────────────────────────────

    #[test]
    fn test_string_literal_array_emits_new_string_v2_per_element() {
        // `let arr: Array<string> = ["a", "b"]` →
        //   NewTypedArrayString, Dup, NewStringV2("a"), TypedArrayPushString,
        //                        Dup, NewStringV2("b"), TypedArrayPushString.
        let prog = compile(
            r#"
            let arr: Array<string> = ["a", "b"]
            arr
            "#,
        );
        assert!(
            has_opcode(&prog, OpCode::NewTypedArrayString),
            "expected NewTypedArrayString for Array<string> literal"
        );
        assert!(
            has_opcode(&prog, OpCode::TypedArrayPushString),
            "expected TypedArrayPushString for per-element push"
        );
        assert!(
            has_opcode(&prog, OpCode::NewStringV2),
            "expected NewStringV2 for per-element v2-raw String literal \
             upgrade (Wave 3 producer-cascade)"
        );
        // The legacy LoadConst(String) path MUST NOT carry the element
        // for the typed-array push: kind would be NativeKind::String
        // (Arc<String>), which the strict-kind check at
        // `v2_handlers/array.rs:687` rejects.
        let new_string_v2_count = prog
            .instructions
            .iter()
            .filter(|i| i.opcode == OpCode::NewStringV2)
            .count();
        assert_eq!(
            new_string_v2_count, 2,
            "expected one NewStringV2 per string literal element (got {})",
            new_string_v2_count
        );
    }

    #[test]
    fn test_decimal_literal_array_emits_new_decimal_v2_per_element() {
        // `let arr: Array<decimal> = [1.5D, 2.5D]` →
        //   NewTypedArrayDecimal, Dup, NewDecimalV2(1.5), TypedArrayPushDecimal,
        //                         Dup, NewDecimalV2(2.5), TypedArrayPushDecimal.
        let prog = compile(
            r#"
            let arr: Array<decimal> = [1.5D, 2.5D]
            arr
            "#,
        );
        assert!(
            has_opcode(&prog, OpCode::NewTypedArrayDecimal),
            "expected NewTypedArrayDecimal for Array<decimal> literal"
        );
        assert!(
            has_opcode(&prog, OpCode::TypedArrayPushDecimal),
            "expected TypedArrayPushDecimal for per-element push"
        );
        assert!(
            has_opcode(&prog, OpCode::NewDecimalV2),
            "expected NewDecimalV2 for per-element v2-raw Decimal \
             literal upgrade (Wave 3 producer-cascade)"
        );
        let new_decimal_v2_count = prog
            .instructions
            .iter()
            .filter(|i| i.opcode == OpCode::NewDecimalV2)
            .count();
        assert_eq!(
            new_decimal_v2_count, 2,
            "expected one NewDecimalV2 per decimal literal element (got {})",
            new_decimal_v2_count
        );
    }

    #[test]
    fn test_single_element_string_literal_array_emits_new_string_v2() {
        // Minimal Array<string> literal — one element. Verifies the per-
        // element opcode wiring for a single-element capacity.
        let prog = compile(
            r#"
            let arr: Array<string> = ["only"]
            arr
            "#,
        );
        assert!(has_opcode(&prog, OpCode::NewTypedArrayString));
        assert!(has_opcode(&prog, OpCode::NewStringV2));
        assert!(has_opcode(&prog, OpCode::TypedArrayPushString));
    }

    #[test]
    fn test_single_element_decimal_literal_array_emits_new_decimal_v2() {
        let prog = compile(
            r#"
            let arr: Array<decimal> = [3.14D]
            arr
            "#,
        );
        assert!(has_opcode(&prog, OpCode::NewTypedArrayDecimal));
        assert!(has_opcode(&prog, OpCode::NewDecimalV2));
        assert!(has_opcode(&prog, OpCode::TypedArrayPushDecimal));
    }

    #[test]
    #[ignore] // diagnostic-only — enable to trace opcode emission for decimal
    fn debug_decimal_opcodes() {
        let prog = compile(
            r#"
            let arr: Array<decimal> = [1.5D, 2.5D]
            arr
            "#,
        );
        for (i, instr) in prog.instructions.iter().enumerate() {
            eprintln!("[{i}] {:?} {:?}", instr.opcode, instr.operand);
        }
        eprintln!("--- constants ---");
        for (i, c) in prog.constants.iter().enumerate() {
            eprintln!("[{i}] {:?}", c);
        }
        eprintln!("--- strings ---");
        for (i, s) in prog.strings.iter().enumerate() {
            eprintln!("[{i}] {:?}", s);
        }
        panic!("DEBUG");
    }
}
