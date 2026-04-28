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
//! ## Coexistence with the legacy registry
//!
//! Each [`TypedModuleFunction`] auto-registers a wrapping
//! `ModuleFn` on the same [`crate::module_exports::ModuleExports`] under
//! the same export name. From the VM's invoke path's point of view, the
//! function looks identical to a hand-rolled `ModuleFn`. The difference is
//! purely on the registration side: typed exports declare their return
//! type concretely via `TypedReturn`, eliminating the ad-hoc
//! `ValueWord::from_*` round-trip in the function body.
//!
//! Phase 4c will migrate the remaining ~65 sum-typed and polymorphic
//! exports (parallel, regex, file, csv, http, yaml, toml, xml, arrow,
//! msgpack) and then delete the legacy `ModuleExports::add_function*`
//! surface. Until then the two registries coexist.

use crate::module_exports::{
    ModuleContext, ModuleExports, ModuleFunction, ModuleParam,
};
use shape_value::datatable::DataTable;
use shape_value::{ArgVec, ValueWord, ValueWordExt};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// Typed return value from a native module function.
///
/// Each variant maps deterministically to a `ValueWord` representation via
/// [`TypedReturn::into_value_word`]. The function body produces a
/// `TypedReturn` directly; marshalling happens at the registry boundary.
///
/// Phase 4b covers static-typed return shapes only. Phase 4c grows the
/// enum to cover sum-typed shapes (`Result<T,E>`, `Option<T>`) and the
/// `Opaque` heap-handle variant used by arrow/wire (DataTable) — see
/// [`TypedReturn::Ok`], [`TypedReturn::Err`], [`TypedReturn::Some`],
/// [`TypedReturn::None`], [`TypedReturn::DataTable`],
/// [`TypedReturn::ArrayObjectPairs`].
#[derive(Debug, Clone)]
pub enum TypedReturn {
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
    /// Object built from string→TypedReturn pairs, materialized as a
    /// `HashMap` ValueWord (same shape as `from_hashmap_pairs`). Used by
    /// e.g. `crypto.ed25519_generate_keypair` and the legacy `archive`
    /// entry shape. Insertion order is preserved.
    ObjectPairs(Vec<(String, TypedReturn)>),
    /// Anonymous typed object — looked up via
    /// [`crate::type_schema::typed_object_from_pairs`] using the field
    /// names as the schema discriminator. Panics at marshal time if no
    /// matching predeclared schema is registered (matches the existing
    /// helper's contract). Used by `time.benchmark` whose return shape is
    /// `{ elapsed_ms, iterations, avg_ms }`.
    TypedObject(Vec<(String, TypedReturn)>),
    /// Generic ValueWord-typed array (used for archive entry arrays where
    /// each element is itself a heap object). The function builds the
    /// elements as ValueWords directly. Phase 4b uses this for the
    /// `archive.zip_extract` / `tar_extract` returns whose element shape
    /// is `{name: string, data: string}` — once Phase 4c adds nested
    /// `TypedReturn::ObjectPairs` array support this can shrink.
    ArrayValueWord(Vec<ValueWord>),
    /// Generic HashMap with ValueWord keys and values. Used by `set_module`
    /// where set elements pass through as-is (they may be any user type).
    /// The typed return shape is `HashMap` but the elements aren't
    /// necessarily strings.
    HashMapValueWord {
        keys: Vec<ValueWord>,
        values: Vec<ValueWord>,
    },
    /// `Ok(payload)` — `Result<T,E>` success constructor. The payload type
    /// follows the function's declared `Result<T,…>` shape; mismatched
    /// payload variants are a registration-time bug.
    Ok(Box<TypedReturn>),
    /// `Err(payload)` — `Result<T,E>` error constructor. For functions
    /// declared `Result<T, string>`, `payload` is `TypedReturn::String(…)`.
    Err(Box<TypedReturn>),
    /// `Some(payload)` — `Option<T>` present constructor.
    Some(Box<TypedReturn>),
    /// `None` — `Option<T>` absent constructor.
    None,
    /// `DataTable` heap handle — opaque columnar table from
    /// `arrow_module` / wire conversion. Surfaces to Shape as the
    /// built-in `DataTable` type.
    DataTable(Arc<DataTable>),
    /// Array of typed object pairs. Each element is a
    /// `Vec<(String, TypedReturn)>` (same shape as
    /// `TypedReturn::ObjectPairs`), materialized as a `HashMap`
    /// ValueWord. Used by xml/regex/csv where the function returns
    /// `Array<{...}>`.
    ArrayObjectPairs(Vec<Vec<(String, TypedReturn)>>),
    /// Pass-through: hand-rolled ValueWord. Escape hatch for borderline
    /// cases where the function body still needs the legacy
    /// `ValueWord::from_*` API (e.g., set operations that construct from
    /// other set inputs, msgpack's `Result<any>` where the inner shape is
    /// `serde_json::Value`-derived). Narrowed module-by-module across
    /// the migration.
    ValueWord(ValueWord),
}

impl TypedReturn {
    /// Marshal this typed return into the runtime `ValueWord` representation.
    pub fn into_value_word(self) -> ValueWord {
        match self {
            TypedReturn::I64(i) => ValueWord::from_i64(i),
            TypedReturn::F64(f) => ValueWord::from_f64(f),
            TypedReturn::Bool(b) => ValueWord::from_bool(b),
            TypedReturn::Unit => ValueWord::unit(),
            TypedReturn::String(s) => ValueWord::from_string(Arc::new(s)),
            TypedReturn::Instant(t) => ValueWord::from_instant(t),
            TypedReturn::ArrayI64(items) => {
                let vs: ArgVec = ArgVec::from_vec(
                    items.into_iter().map(ValueWord::from_i64).collect(),
                );
                ValueWord::from_array(shape_value::vmarray_from_vec(vs.into_inner()))
            }
            TypedReturn::ArrayF64(items) => {
                let vs: ArgVec = ArgVec::from_vec(
                    items.into_iter().map(ValueWord::from_f64).collect(),
                );
                ValueWord::from_array(shape_value::vmarray_from_vec(vs.into_inner()))
            }
            TypedReturn::ArrayString(items) => {
                let vs: ArgVec = ArgVec::from_vec(
                    items
                        .into_iter()
                        .map(|s| ValueWord::from_string(Arc::new(s)))
                        .collect(),
                );
                ValueWord::from_array(shape_value::vmarray_from_vec(vs.into_inner()))
            }
            TypedReturn::Bytes(bytes) => {
                let vs: ArgVec = ArgVec::from_vec(
                    bytes
                        .into_iter()
                        .map(|b| ValueWord::from_i64(b as i64))
                        .collect(),
                );
                ValueWord::from_array(shape_value::vmarray_from_vec(vs.into_inner()))
            }
            TypedReturn::HashMapStringString(pairs) => {
                let mut keys = Vec::with_capacity(pairs.len());
                let mut values = Vec::with_capacity(pairs.len());
                for (k, v) in pairs {
                    keys.push(ValueWord::from_string(Arc::new(k)));
                    values.push(ValueWord::from_string(Arc::new(v)));
                }
                ValueWord::from_hashmap_pairs(keys, values)
            }
            TypedReturn::ObjectPairs(pairs) => {
                let mut keys = Vec::with_capacity(pairs.len());
                let mut values = Vec::with_capacity(pairs.len());
                for (k, v) in pairs {
                    keys.push(ValueWord::from_string(Arc::new(k)));
                    values.push(v.into_value_word());
                }
                ValueWord::from_hashmap_pairs(keys, values)
            }
            TypedReturn::TypedObject(pairs) => {
                let owned: Vec<(String, ValueWord)> = pairs
                    .into_iter()
                    .map(|(k, v)| (k, v.into_value_word()))
                    .collect();
                let view: Vec<(&str, ValueWord)> = owned
                    .iter()
                    .map(|(k, v)| (k.as_str(), v.clone()))
                    .collect();
                crate::type_schema::typed_object_from_pairs(&view)
            }
            TypedReturn::ArrayValueWord(items) => {
                ValueWord::from_array(shape_value::vmarray_from_vec(items))
            }
            TypedReturn::HashMapValueWord { keys, values } => {
                ValueWord::from_hashmap_pairs(keys, values)
            }
            TypedReturn::Ok(inner) => ValueWord::from_ok(inner.into_value_word()),
            TypedReturn::Err(inner) => ValueWord::from_err(inner.into_value_word()),
            TypedReturn::Some(inner) => ValueWord::from_some(inner.into_value_word()),
            TypedReturn::None => ValueWord::none(),
            TypedReturn::DataTable(dt) => ValueWord::from_datatable(dt),
            TypedReturn::ArrayObjectPairs(rows) => {
                let elements: Vec<ValueWord> = rows
                    .into_iter()
                    .map(|pairs| TypedReturn::ObjectPairs(pairs).into_value_word())
                    .collect();
                ValueWord::from_array(shape_value::vmarray_from_vec(elements))
            }
            TypedReturn::ValueWord(v) => v,
        }
    }
}

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
            ConcreteType::Named(s) => s.clone(),
            ConcreteType::Any => "any".to_string(),
        }
    }
}

/// One typed-return native module function entry.
///
/// Stores the typed body alongside the declared return type and parameter
/// types. The legacy `ModuleFn` wrapper is built at registration time from
/// these — see [`register_typed_function`].
#[derive(Clone)]
pub struct TypedModuleFunction {
    /// The typed function body. Receives the raw `&[ValueWord]` arg slice
    /// and the existing `ModuleContext`; returns a `TypedReturn`.
    pub invoke: Arc<
        dyn for<'ctx> Fn(&[ValueWord], &ModuleContext<'ctx>) -> Result<TypedReturn, String>
            + Send
            + Sync,
    >,
    /// Declared return type (used for LSP and consistency checks).
    pub return_type: ConcreteType,
    /// Parameter type names (mirrors `ModuleParam::type_name` for LSP
    /// hover/completions). Phase 4c will tighten these to a typed enum.
    pub arg_types: Vec<String>,
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
    /// `'static` future bounds.
    pub invoke: Arc<
        dyn Fn(
                Vec<ValueWord>,
            ) -> Pin<Box<dyn Future<Output = Result<TypedReturn, String>> + Send>>
            + Send
            + Sync,
    >,
    /// Declared return type (used for LSP and consistency checks).
    pub return_type: ConcreteType,
    /// Parameter type names (mirrors `ModuleParam::type_name`).
    pub arg_types: Vec<String>,
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

/// Register a typed-return function on a `ModuleExports`.
///
/// Adds:
/// 1. A `TypedModuleFunction` entry to the typed registry on the module
///    (created lazily via `ModuleExports::typed_exports_mut`).
/// 2. An auto-wrapping `ModuleFn` to the legacy
///    `ModuleExports::exports` table that runs `body` and marshals the
///    `TypedReturn` to a `ValueWord`. This keeps the existing VM invoke
///    path unchanged.
/// 3. A `ModuleFunction` schema (description + params + return type
///    string from `ConcreteType::shape_type_name`) on
///    `ModuleExports::schemas`.
pub fn register_typed_function<F>(
    module: &mut ModuleExports,
    name: impl Into<String>,
    description: impl Into<String>,
    params: Vec<ModuleParam>,
    return_type: ConcreteType,
    body: F,
) where
    F: for<'ctx> Fn(&[ValueWord], &ModuleContext<'ctx>) -> Result<TypedReturn, String>
        + Send
        + Sync
        + 'static,
{
    let name = name.into();
    let body_arc: Arc<
        dyn for<'ctx> Fn(&[ValueWord], &ModuleContext<'ctx>) -> Result<TypedReturn, String>
            + Send
            + Sync,
    > = Arc::new(body);

    let arg_types = params.iter().map(|p| p.type_name.clone()).collect();
    let return_type_str = return_type.shape_type_name();

    // 2. Auto-wrapping legacy ModuleFn (preserves the current invoke path).
    let body_for_wrapper = body_arc.clone();
    module.add_function_with_schema(
        name.clone(),
        move |args: &[ValueWord], ctx: &ModuleContext| {
            let typed = body_for_wrapper(args, ctx)?;
            Ok(typed.into_value_word())
        },
        ModuleFunction {
            description: description.into(),
            params,
            return_type: Some(return_type_str),
        },
    );

    // 1. Typed-registry entry, alongside the legacy entry.
    module
        .typed_exports_mut()
        .functions
        .insert(
            name,
            TypedModuleFunction {
                invoke: body_arc,
                return_type,
                arg_types,
            },
        );
}

/// Register a typed-return *async* function on a `ModuleExports`.
///
/// Mirrors [`register_typed_function`] but installs an
/// `add_async_function_with_schema` wrapper. The async body returns
/// `Result<TypedReturn, String>` and the boundary marshalling
/// (`TypedReturn` → `ValueWord`) happens after the future resolves.
///
/// Note: async functions don't get a `ModuleContext` (the context borrows
/// from the VM and can't cross await points). Permission checks must be
/// performed before the await — typically by inspecting the args and
/// short-circuiting in the body. The HTTP module relies on a
/// host-supplied `NetConnect` permission gate around the dispatch site.
pub fn register_typed_async_function<F, Fut>(
    module: &mut ModuleExports,
    name: impl Into<String>,
    description: impl Into<String>,
    params: Vec<ModuleParam>,
    return_type: ConcreteType,
    body: F,
) where
    F: Fn(Vec<ValueWord>) -> Fut + Send + Sync + Clone + 'static,
    Fut: std::future::Future<Output = Result<TypedReturn, String>> + Send + 'static,
{
    let name = name.into();
    let arg_types: Vec<String> = params.iter().map(|p| p.type_name.clone()).collect();
    let return_type_str = return_type.shape_type_name();

    // Auto-wrapping async ModuleFn — runs the typed body, then marshals
    // its TypedReturn into a ValueWord at the await boundary.
    let body_for_async = body.clone();
    module.add_async_function_with_schema(
        name.clone(),
        move |args: Vec<ValueWord>| {
            let body = body_for_async.clone();
            async move {
                let typed = body(args).await?;
                Ok(typed.into_value_word())
            }
        },
        ModuleFunction {
            description: description.into(),
            params,
            return_type: Some(return_type_str),
        },
    );

    // Typed-registry entry — sibling to the sync `functions` map.
    // The typed body is wrapped to box+pin its future so all async
    // exports share a uniform `Pin<Box<dyn Future<...>>>` invocation
    // shape regardless of the concrete `Fut` type.
    let body_for_typed = body;
    let typed_invoke: Arc<
        dyn Fn(
                Vec<ValueWord>,
            ) -> Pin<Box<dyn Future<Output = Result<TypedReturn, String>> + Send>>
            + Send
            + Sync,
    > = Arc::new(move |args: Vec<ValueWord>| {
        let fut = body_for_typed(args);
        Box::pin(fut)
            as Pin<Box<dyn Future<Output = Result<TypedReturn, String>> + Send>>
    });

    module
        .typed_exports_mut()
        .async_functions
        .insert(
            name,
            TypedModuleAsyncFunction {
                invoke: typed_invoke,
                return_type,
                arg_types,
            },
        );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_ctx() -> ModuleContext<'static> {
        let registry = Box::leak(Box::new(crate::type_schema::TypeSchemaRegistry::new()));
        ModuleContext {
            schemas: registry,
            invoke_callable: None,
            raw_invoker: None,
            function_hashes: None,
            vm_state: None,
            granted_permissions: None,
            scope_constraints: None,
            set_pending_resume: None,
            set_pending_frame_resume: None,
        }
    }

    #[test]
    fn typed_return_string_marshal() {
        let v = TypedReturn::String("hello".to_string()).into_value_word();
        assert_eq!(v.as_str(), Some("hello"));
    }

    #[test]
    fn typed_return_bool_marshal() {
        let v = TypedReturn::Bool(true).into_value_word();
        assert_eq!(v.as_bool(), Some(true));
    }

    #[test]
    fn typed_return_unit_marshal() {
        let v = TypedReturn::Unit.into_value_word();
        assert!(v.is_unit());
    }

    #[test]
    fn typed_return_array_string_marshal() {
        let v = TypedReturn::ArrayString(vec!["a".to_string(), "b".to_string()])
            .into_value_word();
        let arr = v.as_any_array().unwrap().to_generic();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0].as_str(), Some("a"));
    }

    #[test]
    fn typed_return_bytes_marshal() {
        let v = TypedReturn::Bytes(vec![1, 2, 255]).into_value_word();
        let arr = v.as_any_array().unwrap().to_generic();
        assert_eq!(arr.len(), 3);
        assert_eq!(arr[2].as_i64(), Some(255));
    }

    #[test]
    fn typed_return_hashmap_marshal() {
        let v = TypedReturn::HashMapStringString(vec![("k".to_string(), "v".to_string())])
            .into_value_word();
        let (keys, values, _) = v.as_hashmap().unwrap();
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].as_str(), Some("k"));
        assert_eq!(values[0].as_str(), Some("v"));
    }

    #[test]
    fn typed_return_ok_marshal() {
        let v = TypedReturn::Ok(Box::new(TypedReturn::String("yay".to_string())))
            .into_value_word();
        let inner = v.as_ok_inner().expect("Ok wrapped");
        assert_eq!(inner.as_str(), Some("yay"));
    }

    #[test]
    fn typed_return_err_marshal() {
        let v = TypedReturn::Err(Box::new(TypedReturn::String("oops".to_string())))
            .into_value_word();
        let inner = v.as_err_inner().expect("Err wrapped");
        assert_eq!(inner.as_str(), Some("oops"));
    }

    #[test]
    fn typed_return_some_none_marshal() {
        let some = TypedReturn::Some(Box::new(TypedReturn::I64(42))).into_value_word();
        assert_eq!(some.as_some_inner().and_then(|v| v.as_i64()), Some(42));

        let none = TypedReturn::None.into_value_word();
        assert!(none.is_none());
    }

    #[test]
    fn typed_return_array_object_pairs_marshal() {
        let v = TypedReturn::ArrayObjectPairs(vec![
            vec![("k".to_string(), TypedReturn::I64(1))],
            vec![("k".to_string(), TypedReturn::I64(2))],
        ])
        .into_value_word();
        let arr = v.as_any_array().unwrap().to_generic();
        assert_eq!(arr.len(), 2);
        let (_, values, _) = arr[1].as_hashmap().unwrap();
        assert_eq!(values[0].as_i64(), Some(2));
    }

    #[test]
    fn concrete_type_result_option_names() {
        assert_eq!(
            ConcreteType::Result(Box::new(ConcreteType::String)).shape_type_name(),
            "Result<string>"
        );
        assert_eq!(
            ConcreteType::Result2(
                Box::new(ConcreteType::DataTable),
                Box::new(ConcreteType::String),
            )
            .shape_type_name(),
            "Result<DataTable, string>"
        );
        assert_eq!(
            ConcreteType::Option(Box::new(ConcreteType::Int)).shape_type_name(),
            "Option<int>"
        );
    }

    #[test]
    fn typed_return_object_pairs_marshal() {
        let v = TypedReturn::ObjectPairs(vec![
            ("count".to_string(), TypedReturn::I64(42)),
            ("ok".to_string(), TypedReturn::Bool(true)),
        ])
        .into_value_word();
        let (keys, values, _) = v.as_hashmap().unwrap();
        assert_eq!(keys.len(), 2);
        assert_eq!(values[0].as_i64(), Some(42));
        assert_eq!(values[1].as_bool(), Some(true));
    }

    #[test]
    fn register_typed_async_function_populates_typed_registry() {
        let mut module = ModuleExports::new("std::core::test_typed_async");
        register_typed_async_function(
            &mut module,
            "get_n",
            "Return a constant int via async path",
            vec![],
            ConcreteType::Int,
            |_args: Vec<ValueWord>| async move { Ok(TypedReturn::I64(7)) },
        );

        // Legacy async surface still works (auto-wrapping).
        assert!(module.is_async("get_n"));

        // Typed async registry has the entry with the declared return type.
        let typed_entry = module
            .typed_exports()
            .get_async("get_n")
            .expect("typed async registry should hold the entry");
        assert_eq!(typed_entry.return_type, ConcreteType::Int);
    }

    #[test]
    fn register_typed_function_round_trip() {
        let mut module = ModuleExports::new("std::core::test_typed");
        register_typed_function(
            &mut module,
            "echo",
            "Echo a string",
            vec![ModuleParam {
                name: "s".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "input".to_string(),
                ..Default::default()
            }],
            ConcreteType::String,
            |args, _ctx| {
                let s = args
                    .first()
                    .and_then(|a| a.as_str())
                    .ok_or_else(|| "echo() requires a string".to_string())?
                    .to_string();
                Ok(TypedReturn::String(s))
            },
        );

        // Legacy invoke surface still works.
        let f = module.get_export("echo").unwrap();
        let arg = ValueWord::from_string(Arc::new("hi".to_string()));
        let ctx = empty_ctx();
        let result = f(&[arg], &ctx).unwrap();
        assert_eq!(result.as_str(), Some("hi"));

        // Schema is populated.
        let schema = module.get_schema("echo").unwrap();
        assert_eq!(schema.return_type.as_deref(), Some("string"));
        assert_eq!(schema.params.len(), 1);

        // Typed registry has the entry with the declared return type.
        let typed_entry = module.typed_exports().get("echo").unwrap();
        assert_eq!(typed_entry.return_type, ConcreteType::String);
    }
}
