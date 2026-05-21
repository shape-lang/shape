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
//! Anything else (heap types like `string`, structs, arrays of arrays, sized
//! ints we don't yet have opcodes for, etc.) returns `None`. The compiler is
//! expected to fail soft and emit the legacy NaN-boxed `NewArray` opcode for
//! those cases — Phase 3.1 is intentionally narrow on the typed-fast-path.
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
        }
    }
}

/// Map a `ConcreteType` element type to a `TypedArrayKind`, if a typed-array
/// fast path exists for that element type.
///
/// Returns `None` for element types that have no typed array opcode yet
/// (heap types like `string`/`struct`/nested arrays, sized integer widths
/// like `i8`/`u16`/etc.). Callers must fall back to the legacy NaN-boxed
/// `NewArray` opcode in that case.
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
        _ => None,
    }
}

/// Reverse mapping: derive the `ConcreteType` element type a
/// [`TypedArrayKind`] was minted for.
///
/// Mirror of [`should_use_typed_array`]: every variant produced by that
/// function round-trips back to its source `ConcreteType` here.
///
/// LANG-9 fix (Phase 4b round 2, 2026-05-18): inline array literals
/// (`[1,2,3].map(...)`) failed to monomorphize the method call because
/// `concrete_type_for_expr(Expr::Array)` reads
/// `compiler.array_element_types[span]`, which `compile_expr_array` did
/// not populate at typed-literal lowering time. Per ADR-006 §2.7.5
/// stamp-at-compile-time, the literal's chosen `TypedArrayKind` IS the
/// proof of element-type at construction time; this helper lets the
/// producer record that proof in the side-table so subsequent
/// `Expr::MethodCall` monomorphization on the inline receiver succeeds
/// — same code path the bound form (`let xs = [1,2,3]; xs.map(...)`)
/// reaches via the `identifier_concrete_type` side-table arm.
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
        // StructLayoutId without an additional side-table lookup. This mirrors
        // the `helpers.rs:719` shape `ConcreteType::placeholder_struct(StructLayoutId(0))`
        // used by `StatementKind::ObjectStore` slot-stamping. Downstream
        // consumers that need the precise schema must read from the bytecode
        // compiler's `array_element_types[span]` side-table populated at the
        // literal site (which records the resolved struct schema, NOT this
        // round-trip placeholder).
        TypedArrayKind::TypedObject => ConcreteType::placeholder_struct(
            shape_value::v2::concrete_type::StructLayoutId(0),
        ),
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
/// The mapping mirrors [`should_use_typed_array`]: only the four element
/// types backed by typed array opcodes today (`Float64`/`Int64`/`Int32`/
/// `Bool`) return `Some`. Anything else (`String`, sized ints other than
/// i32/i64, nullable variants, `Dynamic`/`Unknown`) falls back to the
/// legacy NaN-boxed `NewArray` path.
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

/// Map a tracked type name like `"Vec<int>"` / `"Array<number>"` to a [`TypedArrayKind`].
#[inline]
#[allow(dead_code)]
pub fn typed_array_kind_from_type_name(type_name: &str) -> Option<TypedArrayKind> {
    let trimmed = type_name.trim();
    let inner = trimmed
        .strip_prefix("Vec<")
        .or_else(|| trimmed.strip_prefix("Array<"))?
        .strip_suffix('>')?;
    match inner.trim() {
        "number" | "f64" => Some(TypedArrayKind::F64),
        "int" | "i64" => Some(TypedArrayKind::I64),
        "i32" => Some(TypedArrayKind::I32),
        "bool" => Some(TypedArrayKind::Bool),
        // W12 S1 (2026-05-13) — sized integer monomorphizations.
        "i8" => Some(TypedArrayKind::I8),
        "u8" => Some(TypedArrayKind::U8),
        "i16" => Some(TypedArrayKind::I16),
        "u16" => Some(TypedArrayKind::U16),
        "u32" => Some(TypedArrayKind::U32),
        // "u64" intentionally falls through — `Array<u64>` migration
        // deferred to S1.5 per the supervisor's S1 reopen.
        // Wave 2 Agent A1 (2026-05-14) — F32 + Char.
        "f32" => Some(TypedArrayKind::F32),
        "char" => Some(TypedArrayKind::Char),
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
pub fn typed_array_kind_from_numeric_type(
    nt: crate::type_tracking::NumericType,
) -> TypedArrayKind {
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
    pub(crate) fn array_elements_all_typed_object(
        &self,
        elements: &[shape_ast::ast::Expr],
    ) -> bool {
        use shape_ast::ast::Expr;
        if elements.is_empty() {
            return false;
        }
        for elem in elements {
            let returned_type_name: Option<String> = match elem {
                Expr::FunctionCall { name, .. } => {
                    self.type_tracker.get_function_return_type(name).cloned()
                }
                Expr::QualifiedFunctionCall {
                    namespace, function, ..
                } => {
                    let qualified = format!("{}::{}", namespace, function);
                    self.type_tracker
                        .get_function_return_type(&qualified)
                        .cloned()
                }
                _ => return false,
            };
            let Some(name) = returned_type_name else {
                return false;
            };
            let resolved = self
                .type_aliases
                .get(&name)
                .map(|s| s.as_str())
                .unwrap_or(name.as_str());
            if !self.struct_types.contains_key(resolved)
                && !self.struct_types.contains_key(&name)
            {
                return false;
            }
        }
        true
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
        // User-struct annotation: `Array<B>` / `B[]` where B is a registered
        // struct type. Map to TypedArrayKind::TypedObject per §2.1 + §3.A row 1.
        use shape_ast::ast::TypeAnnotation;
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
        if self.struct_types.contains_key(resolved)
            || self.struct_types.contains_key(inner_name)
        {
            Some(TypedArrayKind::TypedObject)
        } else {
            None
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
    /// and for element types that have no typed opcode kind today (`string`,
    /// sized ints other than `i32`/`i64`, etc). The caller is expected to
    /// fall back to the legacy NaN-boxed path in those cases.
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
    fn test_enum_falls_back_to_legacy() {
        assert_eq!(
            should_use_typed_array(&ConcreteType::placeholder_enum(EnumLayoutId(0))),
            None
        );
    }

    #[test]
    fn test_nested_array_falls_back_to_legacy() {
        // Array<Array<int>> — element type is Array<int>, not yet handled
        // by typed opcodes (would need TypedArray<*const TypedArray<i64>>).
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
    fn test_type_name_vec_f32_maps_to_f32() {
        assert_eq!(
            typed_array_kind_from_type_name("Vec<f32>"),
            Some(TypedArrayKind::F32)
        );
        assert_eq!(
            typed_array_kind_from_type_name("Array<f32>"),
            Some(TypedArrayKind::F32)
        );
    }

    #[test]
    fn test_type_name_vec_char_maps_to_char() {
        assert_eq!(
            typed_array_kind_from_type_name("Vec<char>"),
            Some(TypedArrayKind::Char)
        );
        assert_eq!(
            typed_array_kind_from_type_name("Array<char>"),
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

    // ---- typed_array_kind_from_type_name ----

    #[test]
    fn test_type_name_vec_int_maps_to_i64() {
        assert_eq!(
            typed_array_kind_from_type_name("Vec<int>"),
            Some(TypedArrayKind::I64)
        );
    }

    #[test]
    fn test_type_name_vec_number_maps_to_f64() {
        assert_eq!(
            typed_array_kind_from_type_name("Vec<number>"),
            Some(TypedArrayKind::F64)
        );
    }

    #[test]
    fn test_type_name_vec_bool_maps_to_bool() {
        assert_eq!(
            typed_array_kind_from_type_name("Vec<bool>"),
            Some(TypedArrayKind::Bool)
        );
    }

    #[test]
    fn test_type_name_vec_i32_maps_to_i32() {
        assert_eq!(
            typed_array_kind_from_type_name("Vec<i32>"),
            Some(TypedArrayKind::I32)
        );
    }

    #[test]
    fn test_type_name_array_int_maps_to_i64() {
        assert_eq!(
            typed_array_kind_from_type_name("Array<int>"),
            Some(TypedArrayKind::I64)
        );
    }

    #[test]
    fn test_type_name_vec_string_falls_back() {
        assert_eq!(typed_array_kind_from_type_name("Vec<string>"), None);
    }

    #[test]
    fn test_type_name_non_array_falls_back() {
        assert_eq!(typed_array_kind_from_type_name("HashMap<int, int>"), None);
        assert_eq!(typed_array_kind_from_type_name("int"), None);
    }
}

// ──────────────────────────────────────────────────────────────────────
// Compile integration tests — verify `compile_expr_array` emits the
// correct opcode (`NewTypedArray*` vs legacy `NewArray`/`NewTypedArray`)
// for the array literal shapes called out in the Phase 3.1 deliverables.
// ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod compile_integration_tests {
    use super::*;
    use crate::bytecode::{BytecodeProgram, OpCode};
    use crate::compiler::BytecodeCompiler;

    fn compile(src: &str) -> BytecodeProgram {
        let program = shape_ast::parser::parse_program(src).expect("parse should succeed");
        BytecodeCompiler::new()
            .compile_with_source(&program, src)
            .expect("compile should succeed")
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
            msg.contains("spread element types could not be reconciled"),
            "heterogeneous spread error must be the clean structured message, got: {msg}"
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
    fn test_heterogeneous_literal_falls_back_to_legacy_new_array() {
        // `[1, "x", true]` is heterogeneous → no typed array fast path,
        // falls back to legacy `NewArray` (NaN-boxed Vec<ValueWord>).
        let prog = compile("[1, \"x\", true]");
        assert!(
            has_opcode(&prog, OpCode::NewArray),
            "heterogeneous literal must emit legacy NewArray"
        );
        assert!(
            !has_opcode(&prog, OpCode::NewTypedArrayI64),
            "heterogeneous literal must not emit NewTypedArrayI64"
        );
        assert!(
            !has_opcode(&prog, OpCode::NewTypedArrayF64),
            "heterogeneous literal must not emit NewTypedArrayF64"
        );
        assert!(
            !has_opcode(&prog, OpCode::NewTypedArrayBool),
            "heterogeneous literal must not emit NewTypedArrayBool"
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
    // R5.4B: nested-array-literal guard.
    //
    // The typed `NewTypedArrayF64/I64/I32/Bool` opcodes store scalars
    // (raw f64/i64/i32/bool bits). An outer array whose rows are
    // themselves arrays cannot use the typed fast path — it would store
    // inner typed-array pointers as scalar bits, which downstream
    // consumers (`intrinsic_matmul_mat`, `as_any_array()`) can't decode.
    //
    // The regression guard refuses typed emission at both:
    //   - inference: `infer_array_element_type` returns None when any
    //     element is `Expr::Array`.
    //   - annotation override: `compile_expr_array` refuses the typed
    //     path when any element is `Expr::Array`, regardless of
    //     `pending_variable_typed_array_kind`.
    //   - recursion: the inner rows compile with
    //     `nested_array_literal_depth > 0`, which forces them back to
    //     the legacy `NewArray` path so they round-trip as heap-ref
    //     ValueWords, not as `NativeScalar::Ptr` words.
    // ──────────────────────────────────────────────────────────────────

    #[test]
    fn test_nested_array_literal_mat_number_emits_new_array_for_outer_and_inner() {
        // `let m: Mat<number> = [[1.0, 2.0], [3.0, 4.0]]` — outer AND
        // inner must fall back to the generic `NewArray` path (not the
        // typed `NewTypedArrayF64` path).
        let prog = compile(
            r#"
            let m: Mat<number> = [[1.0, 2.0], [3.0, 4.0]]
            m
            "#,
        );
        assert!(
            has_opcode(&prog, OpCode::NewArray),
            "nested `Mat<number>` literal must emit legacy NewArray, got opcodes: {:?}",
            prog.instructions
                .iter()
                .map(|i| i.opcode)
                .collect::<Vec<_>>()
        );
        assert!(
            !has_opcode(&prog, OpCode::NewTypedArrayF64),
            "nested `Mat<number>` literal must NOT emit NewTypedArrayF64 \
             (would splice inner typed-array pointers into f64 slots); \
             got opcodes: {:?}",
            prog.instructions
                .iter()
                .map(|i| i.opcode)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_nested_array_literal_array_number_annotation_refuses_typed() {
        // Same nested shape but with an `Array<number>` annotation —
        // without the R5.4B guard this path took the annotation-driven
        // typed branch (`pending_variable_typed_array_kind = Some(F64)`)
        // and emitted `NewTypedArrayF64` for BOTH outer and inner,
        // splicing inner pointers into f64 slots of the outer.
        let prog = compile(
            r#"
            let m: Array<number> = [[1.0, 2.0], [3.0, 4.0]]
            m
            "#,
        );
        assert!(
            has_opcode(&prog, OpCode::NewArray),
            "nested `Array<number>` literal must emit legacy NewArray, got opcodes: {:?}",
            prog.instructions
                .iter()
                .map(|i| i.opcode)
                .collect::<Vec<_>>()
        );
        assert!(
            !has_opcode(&prog, OpCode::NewTypedArrayF64),
            "nested `Array<number>` annotation with nested rows must NOT \
             emit NewTypedArrayF64; got opcodes: {:?}",
            prog.instructions
                .iter()
                .map(|i| i.opcode)
                .collect::<Vec<_>>()
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
    fn test_index_access_untyped_falls_back_to_get_prop() {
        // Empty / heterogeneous array → no typed kind → falls back.
        let prog = compile(
            r#"
            let arr = [1, "x", true]
            arr[0]
            "#,
        );
        assert!(
            has_opcode(&prog, OpCode::GetProp),
            "expected legacy GetProp for untyped array index access"
        );
        assert!(
            !has_opcode(&prog, OpCode::TypedArrayGetI64),
            "untyped array must not emit TypedArrayGetI64"
        );
        assert!(
            !has_opcode(&prog, OpCode::TypedArrayGetF64),
            "untyped array must not emit TypedArrayGetF64"
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
    fn test_length_untyped_array_falls_back_to_legacy_length() {
        let prog = compile(
            r#"
            let arr = [1, "x", true]
            arr.length
            "#,
        );
        assert!(
            has_opcode(&prog, OpCode::Length),
            "expected legacy Length for untyped array"
        );
        assert!(
            !has_opcode(&prog, OpCode::TypedArrayLen),
            "untyped array must not emit TypedArrayLen"
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
    #[ignore]  // diagnostic-only — enable to trace opcode emission for decimal
    fn debug_decimal_opcodes() {
        let prog = compile(r#"
            let arr: Array<decimal> = [1.5D, 2.5D]
            arr
            "#);
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
