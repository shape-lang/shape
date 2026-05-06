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
/// the body with matching slot bits — readers of `from_slot` therefore
/// do not perform any tag-decode dispatch themselves.
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
