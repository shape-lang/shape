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

use crate::module_exports::{
    ModuleContext, ModuleExports, ModuleFunction, ModuleParam,
};
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
    /// Byte array, surfaced as `Array<int>` to user code (each byte
    /// widened to i64 in 0..=255).
    Bytes(Vec<u8>),
    /// HashMap with string keys and string values.
    HashMapStringString(Vec<(String, String)>),
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
    /// `Array<int>` semantically (each element a u8 widened to i64).
    Bytes,
    HashMapStringString,
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
            ConcreteType::Bytes => "Array<int>".to_string(),
            ConcreteType::HashMapStringString => "HashMap<string, string>".to_string(),
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

// `register_typed_function` and `register_typed_async_function` (the
// polymorphic-arg legacy registration helpers that took
// `Fn(&[ValueWord], ..)` bodies) are deleted. Use the per-arity helpers
// in [`crate::marshal`] instead — they enforce typed args via
// `FromSlot` at the type-system level. Stdlib modules will be migrated
// module-by-module in Phase 2c.

// `register_test_function` / `_with_schema` / `register_test_async_function`
// were thin wrappers that fed a `Fn(&[ValueWord]) -> Result<ValueWord, ..>`
// body into the typed registry as a `TypedReturn::ValueWord` passthrough.
// Deleted alongside the three explicit ValueWord variants — the marshal
// boundary they fed is being rebuilt in Phase 2b.

// Marshal-layer round-trip tests removed alongside `into_value_word()`.
// The Phase 2b marshal layer rebuilds them as kind-threaded slot-write
// tests on top of the new (NativeKind, u64) projection.
