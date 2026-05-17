//! Typed-return ABI for native stdlib modules (Phase 4b).
//!
//! Companion to [`crate::module_exports`]. The legacy `ModuleExports` ABI
//! exposes every native function as
//! `fn(&[ValueWord], &ModuleContext) -> Result<ValueWord, String>` — i.e.,
//! the function body is responsible for hand-marshalling its result into a
//! `ValueWord` via `ValueWord::from_string` / `from_bool` / `from_array` /
//! etc. This forces every static-typed export to carry the runtime tag-bit
//! representation as part of its source surface, even though the return type
//! is fully determined at registration time.
//!
//! `TypedModuleExports` is the parallel typed-return ABI. Each function
//! declares its return type via [`TypedReturn`] (a sum over the primitive
//! native types) and the marshalling to `ValueWord` happens at the registry
//! boundary — not inside the function body. The function body returns
//! e.g. `TypedReturn::String(s)` directly.
//!
//! ## Single registry (Phase 4c.4)
//!
//! Phase 4c.4 deleted the legacy `ModuleExports::exports` /
//! `async_exports` parallel registry and the `add_function*` surface.
//! All native module function bodies live in `TypedModuleExports`,
//! dispatched through `ModuleFnEntry::Typed` / `ModuleFnEntry::TypedAsync`.
//! Test fixtures that don't care about typed dispatch use the
//! `register_test_function*` helpers, which wrap a legacy-style
//! `Fn(...) -> Result<ValueWord, String>` body into a
//! `TypedReturn::ValueWord` passthrough.

use crate::module_exports::ModuleContext;
use shape_value::datatable::DataTable;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// Strictly-typed leaf value returned by a native function body.
///
/// Two-tier split (`docs/defections.md` 2026-05-06): the leaf variants
/// live here, and the wrapper variants (`Ok`/`Err`/`Some`, `ObjectPairs`,
/// etc.) on [`TypedReturn`] take `ConcreteReturn` rather than a recursive
/// `Box<TypedReturn>`. The Rust type system enforces that no
/// `TypedReturn::Ok(TypedReturn::Ok(...))` and (post-Phase-2a) no
/// `TypedReturn::Ok(TypedReturn::ValueWord(...))`-shaped patterns are
/// constructible — the forbidden state is unrepresentable, not just
/// unreachable. Mirrors the `ProofGap` discipline.
///
// ADR-005: do not add per-HeapKind variants here. HeapValue is the canonical
// discriminator; the existing heap-arm variants (`ArrayHeapValue`,
// `HashMapStringHeapValue`, `JsonValue`, `OpaqueTypedObject`) are scheduled
// for cluster #7 cleanup — fold into a single `Heap(Arc<HeapValue>)` arm.
// See docs/adr/005-typed-slot-construction.md before extending this enum.
#[derive(Debug, Clone)]
pub enum ConcreteReturn {
    /// 64-bit signed integer.
    I64(i64),
    /// 64-bit floating-point number (the `number` type in Shape).
    F64(f64),
    /// Boolean.
    Bool(bool),
    /// Unit / void / `()`.
    Unit,
    /// Owned UTF-8 string.
    String(String),
    /// Monotonic instant (the `Instant` type).
    Instant(std::time::Instant),
    /// Array of i64s (compiles to `Array<int>`).
    ArrayI64(Vec<i64>),
    /// Array of f64s (compiles to `Array<number>`).
    ArrayF64(Vec<f64>),
    /// Array of strings (compiles to `Array<string>`).
    ArrayString(Vec<String>),
    /// Array whose elements are heap-allocated typed values (Phase 2d
    /// Array cluster, 2026-05-07). Each element is an opaque
    /// `Arc<HeapValue>`; the body is responsible for ensuring all
    /// elements share the same statically-declared element type.
    /// Used for `Array<DataTable>`, `Array<Array<string>>`,
    /// `Array<TypedObject>`, etc. — anywhere the element shape is
    /// itself a heap-resident typed value.
    ///
    /// Discriminator-level homogeneity: there is **one**
    /// `ConcreteReturn::ArrayHeapValue` (matching one
    /// `the-deleted-heterogeneous-element-carrier` storage variant). Per-element-kind
    /// variants (`ArrayDataTable` / `ArrayIoHandle` / etc.) are
    /// rejected on the same grounds as the parametric-NativeKind
    /// pattern — see `docs/defections.md` Phase 2d Array cluster
    /// entry.
    ArrayHeapValue(Vec<Arc<shape_value::heap_value::HeapValue>>),
    /// Byte array, surfaced as `Array<int>` to user code (each byte
    /// widened to i64 in 0..=255).
    Bytes(Vec<u8>),
    /// HashMap with string keys and string values.
    HashMapStringString(Vec<(String, String)>),
    /// HashMap with string keys and heap-allocated values (Stage C
    /// HashMap-marshal P1(b), 2026-05-07). Polymorphic-value form
    /// covering 8 of 9 stdlib body consumers (json/yaml/toml/msgpack/
    /// xml-attributes/http-headers/http-options + xml-node-attributes).
    /// Each element is an opaque `Arc<HeapValue>`; the body is
    /// responsible for ensuring all values share the same statically-
    /// declared element type.
    ///
    /// Discriminator-level homogeneity per option ε pattern: there is
    /// **one** `ConcreteReturn::HashMapStringHeapValue` matching the
    /// `HeapValue::HashMap(HashMapData)` storage variant.
    /// Per-element-kind variants (`HashMapStringDataTable` etc.) are
    /// rejected on the same grounds as the parametric-NativeKind
    /// pattern — see `docs/defections.md` HashMap-marshal entry's
    /// audit-grounded correction subsection.
    HashMapStringHeapValue(Vec<(String, Arc<shape_value::heap_value::HeapValue>)>),
    /// Strict-typed parsed-data tree (Stage D N6 sub-shape (b),
    /// 2026-05-07). Replaces the deleted
    /// `TypedReturn::Ok(Box::new(TypedReturn::ValueWord(arc)))` body
    /// pattern that pre-bulldozer JSON parsing used. The payload is the
    /// recursive [`crate::json_value::JsonValue`] sum
    /// (`Null/Bool/Int/Number/String/Bytes/Array/Object`); recursion is
    /// at the `JsonValue` payload layer, NOT at `ConcreteReturn`.
    /// `ConcreteReturn::JsonValue` itself is a leaf variant — its
    /// payload is independently recursive in the same way that
    /// `ConcreteReturn::HashMapStringHeapValue`'s payload is recursive
    /// at the `HeapValue` layer. The leaf-only invariant of
    /// `ConcreteReturn` is preserved.
    ///
    /// Used by `json.parse(text) -> Result<Json>` and
    /// `json.__parse_typed(text, schema) -> Result<any>` body
    /// projection. Cross-cluster precedent at the user-facing Shape
    /// API: `crates/shape-runtime/stdlib-src/core/json_value.shape`
    /// already declares the `Json` enum with matching variants.
    /// See `docs/defections.md` HashMap-marshal cluster N6
    /// consumer-expansion subsection (2026-05-07) for the
    /// sub-shape (b) categorization.
    JsonValue(crate::json_value::JsonValue),
    /// Opaque `Arc<HeapValue::TypedObject>` handle for dynamic-schema
    /// returns (Stage B+D close-out, N8 sign-off, 2026-05-07).
    ///
    /// Used by stdlib bodies that take a runtime `schema_id` parameter
    /// and produce a `HeapValue::TypedObject{schema_id, slots,
    /// heap_mask}` conforming to that schema. The dispatcher projects
    /// the `Arc<HeapValue>` directly into a slot via
    /// `NativeKind::Ptr(HeapKind::TypedObject)` — `TypedObject` is a
    /// **specific** `HeapKind`, NOT wildcard. Schema is data carried
    /// by the heap value's existing `schema_id` field; NOT
    /// architectural metadata at the dispatcher layer.
    ///
    /// Leaf in `ConcreteReturn`-recursion sense (`Arc<HeapValue>` is
    /// the leaf payload); recursive at `HeapValue` layer (consistent
    /// with `ConcreteReturn::HashMapStringHeapValue` precedent).
    /// Naming: "Opaque" reflects supervisor-side does not decompose
    /// the TypedObject's slots; "TypedObject" reflects the known
    /// `HeapKind` discriminant.
    ///
    /// Distinct from `TypedReturn::TypedObject(Vec<(String,
    /// ConcreteReturn)>)` which is schemaful at REGISTRATION time and
    /// requires per-field decomposition. `OpaqueTypedObject` is for
    /// bodies that have already produced a fully-built
    /// `Arc<HeapValue::TypedObject>` keyed by a runtime `schema_id`.
    ///
    /// Confirmed consumer (current): `json.__parse_typed(text,
    /// schema_id) -> Result<any>` at
    /// `crates/shape-runtime/src/stdlib/json.rs`.
    ///
    /// See `docs/defections.md` Stage B+D close-out batch dispositions
    /// (2026-05-07) for the N8 sign-off framing + refused alternatives
    /// (`TypedObjectHandle`, `TypedObjectByRef`, `OpaqueAnyHeapValue`).
    OpaqueTypedObject(Arc<shape_value::heap_value::HeapValue>),
    /// `DataTable` heap handle — opaque columnar table from
    /// `arrow_module` / wire conversion. Surfaces to Shape as the
    /// built-in `DataTable` type.
    DataTable(Arc<DataTable>),
    /// `IoHandle` heap handle — opaque OS resource (file, socket,
    /// process) from `stdlib_io`. Surfaces to Shape as the
    /// built-in `IoHandle` type. Cluster #2 option γ per
    /// `docs/defections.md` 2026-05-06.
    IoHandle(Arc<shape_value::heap_value::IoHandleData>),
}

/// Typed return value from a native module function.
///
/// Two-tier with [`ConcreteReturn`]: every wrapper variant
/// (`Ok`/`Err`/`Some`/`ObjectPairs`/etc.) takes a `ConcreteReturn` payload.
/// Nesting `TypedReturn` inside `TypedReturn` is unrepresentable, which
/// also makes the long-deleted `TypedReturn::ValueWord` escape hatch
/// unreachable from any container variant.
///
/// The Phase 2b kind-threaded marshal layer projects each variant
/// directly into a typed VM slot using the function's registered
/// [`ConcreteType`] return descriptor.
#[derive(Debug, Clone)]
pub enum TypedReturn {
    /// Direct leaf-typed return.
    Concrete(ConcreteReturn),
    /// Object built from string→leaf pairs, materialized as a
    /// `HashMap`-shaped TypedObject. Insertion order preserved.
    ObjectPairs(Vec<(String, ConcreteReturn)>),
    /// Anonymous typed object — looked up via
    /// [`crate::type_schema::typed_object_from_pairs`] using the field
    /// names as the schema discriminator. Panics at marshal time if no
    /// matching predeclared schema is registered. Used by
    /// `time.benchmark` whose return shape is
    /// `{ elapsed_ms, iterations, avg_ms }`.
    TypedObject(Vec<(String, ConcreteReturn)>),
    /// `Ok(payload)` — `Result<T,E>` success constructor.
    Ok(ConcreteReturn),
    /// `Err(payload)` — `Result<T,E>` error constructor.
    Err(ConcreteReturn),
    /// `Some(payload)` — `Option<T>` present constructor.
    Some(ConcreteReturn),
    /// `None` — `Option<T>` absent constructor.
    None,
    /// Array of typed-object rows. Used by xml/regex/csv where the
    /// function returns `Array<{...}>`.
    ArrayObjectPairs(Vec<Vec<(String, ConcreteReturn)>>),
    /// `Some(typed_object)` — `Option<{...}>` present constructor whose
    /// payload is a typed-object pair-list. Phase 2d Cluster #4 (option β,
    /// 2026-05-07): flat per-wrapper variant rather than recursive
    /// `ConcreteReturn::TypedObject` (option α), preserving the leaf-only
    /// invariant of `ConcreteReturn` as unrepresentably-violated by
    /// Rust's type system. Mirrors the existing `ObjectPairs` /
    /// `ArrayObjectPairs` variant shape — pattern continuation, not
    /// pattern invention. Used by `regex.match` / `regex.find`.
    SomeObjectPairs(Vec<(String, ConcreteReturn)>),
    /// `Ok(typed_object)` — `Result<{...}, E>` success constructor whose
    /// payload is a typed-object pair-list. Same Cluster #4 option β
    /// shape as `SomeObjectPairs`. Used by future stdlib returns whose
    /// success case is a typed object (e.g. `arrow.metadata` after a
    /// HashMap-marshal landing rewrites it as a typed object).
    OkObjectPairs(Vec<(String, ConcreteReturn)>),
    /// `Err(typed_object)` — `Result<T, {...}>` error constructor whose
    /// payload is a typed-object pair-list. Same Cluster #4 option β
    /// shape as `SomeObjectPairs`. Used by future stdlib returns whose
    /// error case is a structured error object.
    ErrObjectPairs(Vec<(String, ConcreteReturn)>),
}

impl From<ConcreteReturn> for TypedReturn {
    fn from(c: ConcreteReturn) -> Self {
        TypedReturn::Concrete(c)
    }
}

// `TypedReturn::into_value_word()` is removed. The strict-typed marshal
// boundary projects each variant directly into a typed slot via the
// per-function `NativeKind` declared at registration. That boundary
// landing is Phase 2b — see `docs/defections.md`.

/// Concrete return-type discriminant for a typed module function.
///
/// Used at registration time to record what shape the function returns.
/// The LSP and content-addressed schema can read this without invoking
/// the function. Mirrors the variant shape of [`TypedReturn`] but carries
/// no payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConcreteType {
    Int,
    Number,
    Bool,
    Unit,
    String,
    Instant,
    ArrayInt,
    ArrayNumber,
    ArrayString,
    /// Array of heap-allocated typed values. The displayed type-name is
    /// caller-provided to keep the LSP surface readable
    /// (`Array<DataTable>`, `Array<Array<string>>`, etc.); element-kind
    /// homogeneity is a body-side type contract per the Phase 2d Array
    /// cluster decision.
    ArrayHeapValue(String),
    /// `Array<int>` semantically (each element a u8 widened to i64).
    Bytes,
    HashMapStringString,
    /// `HashMap<string, *>` with heap-allocated values (Stage C
    /// HashMap-marshal P1(b), 2026-05-07). The displayed type-name is
    /// caller-provided to keep the LSP surface readable
    /// (`HashMap<string, Json>`, `HashMap<string, any>`, etc.);
    /// element-kind homogeneity is a body-side type contract per the
    /// option ε pattern.
    HashMapStringHeapValue(String),
    /// Strict-typed parsed-data tree (Stage D N6 sub-shape (b),
    /// 2026-05-07). Mirror for `ConcreteReturn::JsonValue`. The displayed
    /// type-name is caller-provided so the LSP can surface either
    /// `Json` (for `json.parse`'s typed return) or `any` (for
    /// `json.__parse_typed`'s polymorphic return). The actual payload
    /// shape is identical (recursive `JsonValue` sum); the visible
    /// type-name carries the user-API distinction.
    JsonValue(String),
    /// Opaque `Arc<HeapValue::TypedObject>` for dynamic-schema returns
    /// (Stage B+D close-out, N8 sign-off, 2026-05-07). Mirror for
    /// `ConcreteReturn::OpaqueTypedObject`. The displayed type-name is
    /// caller-provided to keep the LSP surface readable
    /// (`any` for `json.__parse_typed`'s polymorphic return; in future
    /// could be a specific Shape type-name when the schema's name is
    /// statically known).
    OpaqueTypedObject(String),
    /// Heterogeneous object built from string→typed pairs (materialized
    /// as a `HashMap`).
    Object,
    /// Anonymous TypedObject (looked up via predeclared schema).
    TypedObject,
    /// Array whose element shape is per-export-defined (e.g.,
    /// `Array<{name: string, data: string}>`). Carries the user-visible
    /// type-name string for documentation/LSP.
    ArrayObject(String),
    /// Untyped HashMap (e.g. set_module returns where elements are
    /// user-provided). Surfaces as `HashMap` in the LSP.
    HashMap,
    /// Untyped Array (escape hatch for borderline cases).
    Array,
    /// `Result<T>` (single-arg form). Common across stdlib (file/csv/yaml).
    Result(Box<ConcreteType>),
    /// `Result<T, E>` (two-arg form). Used by arrow/wire returns whose
    /// LSP surface is `Result<DataTable, string>` etc.
    Result2(Box<ConcreteType>, Box<ConcreteType>),
    /// `Option<T>`. Used by regex/csv returns whose LSP surface is
    /// `Option<{...}>`.
    Option(Box<ConcreteType>),
    /// `DataTable` opaque heap handle (arrow / wire).
    DataTable,
    /// `IoHandle` opaque OS-resource heap handle (file/socket/process)
    /// from `stdlib_io`. Cluster #2 option γ.
    IoHandle,
    /// `HashMap<string, string>` — alias for `HashMapStringString`. New
    /// callers should prefer this name; kept distinct for clarity.
    /// Free-form generic type name. Escape hatch for `Result<any>`,
    /// `Result<DataTable, string>` (already supported via Result2 plus
    /// payload), and ad-hoc shapes that don't decompose. Use sparingly.
    Named(String),
    /// `any` — polymorphic return. Used by msgpack.decode whose payload
    /// is `serde_json::Value`-derived.
    Any,
}

impl ConcreteType {
    /// Return the user-facing type name, matching the strings already used
    /// in `ModuleFunction::return_type`. This keeps the LSP surface stable
    /// across the migration.
    pub fn shape_type_name(&self) -> String {
        match self {
            ConcreteType::Int => "int".to_string(),
            ConcreteType::Number => "number".to_string(),
            ConcreteType::Bool => "bool".to_string(),
            ConcreteType::Unit => "unit".to_string(),
            ConcreteType::String => "string".to_string(),
            ConcreteType::Instant => "Instant".to_string(),
            ConcreteType::ArrayInt => "Array<int>".to_string(),
            ConcreteType::ArrayNumber => "Array<number>".to_string(),
            ConcreteType::ArrayString => "Array<string>".to_string(),
            ConcreteType::ArrayHeapValue(s) => s.clone(),
            ConcreteType::Bytes => "Array<int>".to_string(),
            ConcreteType::HashMapStringString => "HashMap<string, string>".to_string(),
            ConcreteType::HashMapStringHeapValue(s) => s.clone(),
            ConcreteType::JsonValue(s) => s.clone(),
            ConcreteType::OpaqueTypedObject(s) => s.clone(),
            ConcreteType::Object => "object".to_string(),
            ConcreteType::TypedObject => "object".to_string(),
            ConcreteType::ArrayObject(s) => s.clone(),
            ConcreteType::HashMap => "HashMap".to_string(),
            ConcreteType::Array => "Array".to_string(),
            ConcreteType::Result(inner) => format!("Result<{}>", inner.shape_type_name()),
            ConcreteType::Result2(t, e) => {
                format!("Result<{}, {}>", t.shape_type_name(), e.shape_type_name())
            }
            ConcreteType::Option(inner) => format!("Option<{}>", inner.shape_type_name()),
            ConcreteType::DataTable => "DataTable".to_string(),
            ConcreteType::IoHandle => "IoHandle".to_string(),
            ConcreteType::Named(s) => s.clone(),
            ConcreteType::Any => "any".to_string(),
        }
    }
}

/// One typed-return native module function entry.
///
/// Stores the typed body alongside the declared return type, parameter
/// types, and the per-arg [`shape_value::NativeKind`] table that the
/// dispatcher uses to read each slot's bits. Built only by the typed
/// `register_typed_fn_N` helpers in [`crate::marshal`] — those helpers
/// derive `arg_kinds` from each parameter's `FromSlot::NATIVE_KIND`
/// associated constant, so the kinds cannot drift from the body's
/// actual Rust signature.
#[derive(Clone)]
pub struct TypedModuleFunction {
    /// The typed function body. Receives a raw `&[u64]` slot-bits slice
    /// (the dispatcher has guaranteed each slot's kind matches
    /// [`Self::arg_kinds`]) plus the `ModuleContext`; returns a
    /// `TypedReturn`.
    pub invoke: Arc<
        dyn for<'ctx> Fn(&[u64], &ModuleContext<'ctx>) -> Result<TypedReturn, String>
            + Send
            + Sync,
    >,
    /// Declared return type (used for LSP and consistency checks).
    pub return_type: ConcreteType,
    /// Parameter type names (mirrors `ModuleParam::type_name` for LSP
    /// hover/completions).
    pub arg_types: Vec<String>,
    /// Per-arg `NativeKind` derived from `FromSlot::NATIVE_KIND` at
    /// registration. The dispatcher uses this to decode each slot's
    /// bits with the correct kind. Length matches `arg_types`.
    pub arg_kinds: Vec<shape_value::NativeKind>,
}

/// One typed-return native module *async* function entry.
///
/// Mirrors [`TypedModuleFunction`] but the body returns a future that
/// resolves to `Result<TypedReturn, String>`. Async exports do not get a
/// `ModuleContext` (the context borrows from the VM and cannot cross
/// await points); permission gating must happen synchronously around the
/// dispatch site or up-front in the body before the await.
#[derive(Clone)]
pub struct TypedModuleAsyncFunction {
    /// The typed async function body. Owns its arg vec to satisfy
    /// `'static` future bounds. Receives raw `Vec<u64>` slot bits
    /// whose kinds match `Self::arg_kinds`.
    pub invoke: Arc<
        dyn Fn(
                Vec<u64>,
            ) -> Pin<Box<dyn Future<Output = Result<TypedReturn, String>> + Send>>
            + Send
            + Sync,
    >,
    /// Declared return type (used for LSP and consistency checks).
    pub return_type: ConcreteType,
    /// Parameter type names (mirrors `ModuleParam::type_name`).
    pub arg_types: Vec<String>,
    /// Per-arg `NativeKind`. See [`TypedModuleFunction::arg_kinds`].
    pub arg_kinds: Vec<shape_value::NativeKind>,
}

/// Per-module registry of typed exports.
///
/// Lives alongside the legacy [`ModuleExports::exports`] map. The
/// boundary marshalling (`TypedReturn` → `ValueWord`) happens in the
/// auto-installed wrapper that `register_typed_function` adds to the
/// legacy `ModuleExports::exports` table — so the VM invoke path remains
/// unchanged. The typed entry is preserved separately for
/// introspection.
#[derive(Default, Clone)]
pub struct TypedModuleExports {
    /// `name → TypedModuleFunction`. Insertion mirrors
    /// `ModuleExports::exports` so every typed export also has a legacy
    /// entry.
    pub functions: HashMap<String, TypedModuleFunction>,
    /// `name → TypedModuleAsyncFunction`. Sibling map for typed async
    /// exports. Mirrors `ModuleExports::async_exports`.
    pub async_functions: HashMap<String, TypedModuleAsyncFunction>,
}

impl TypedModuleExports {
    pub fn new() -> Self {
        Self {
            functions: HashMap::new(),
            async_functions: HashMap::new(),
        }
    }

    pub fn get(&self, name: &str) -> Option<&TypedModuleFunction> {
        self.functions.get(name)
    }

    pub fn get_async(&self, name: &str) -> Option<&TypedModuleAsyncFunction> {
        self.async_functions.get(name)
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.functions
            .keys()
            .chain(self.async_functions.keys())
            .map(|s| s.as_str())
    }

    pub fn is_empty(&self) -> bool {
        self.functions.is_empty() && self.async_functions.is_empty()
    }
}

// `register_typed_function` and `register_typed_async_function` are
// re-introduced at the [`shape_value::KindedSlot`] shape per ADR-006
// §2.7.4 ruling. Per-arity helpers (`register_typed_fn_N`) remain the
// preferred path when the function arity is fixed; the variadic
// helpers below cover the genuine §2.7.1.4 dispatch-slice case
// (functions with optional arguments — json/msgpack/toml/yaml/
// stdlib_time bodies that take `pretty?: bool` / `iterations?: int` /
// etc.). Both registration paths are valid; pick by arity contract.
pub use crate::marshal::{register_typed_async_function, register_typed_function};

// `register_test_function` / `_with_schema` / `register_test_async_function`
// were thin wrappers that fed a `Fn(&[ValueWord]) -> Result<ValueWord, ..>`
// body into the typed registry as a `TypedReturn::ValueWord` passthrough.
// Deleted alongside the three explicit ValueWord variants — the marshal
// boundary they fed is being rebuilt in Phase 2b.

// Marshal-layer round-trip tests removed alongside `into_value_word()`.
// The Phase 2b marshal layer rebuilds them as kind-threaded slot-write
// tests on top of the new (NativeKind, u64) projection.
