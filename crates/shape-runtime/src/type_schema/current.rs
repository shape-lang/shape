//! Task-local "current" `TypeSchemaRegistry` handle.
//!
//! This module exposes an ambient registry that is pushed by `Runtime`'s
//! execution entry points and consumed by free functions that previously
//! reached for the process-global `STDLIB_SCHEMA_REGISTRY` / `NEXT_SCHEMA_ID`
//! statics. The scoping is three-layered to cover both async and synchronous
//! entry points, with a process-wide default as the final fallback:
//!
//! 1. **Task-local** (`CURRENT_SCHEMA_REGISTRY`) — survives task migration
//!    across tokio worker threads, so any descendant `.await` in a Shape
//!    execution future inherits the registry automatically.
//! 2. **Thread-local** (`SYNC_CURRENT_SCHEMA_REGISTRY`) — fallback for
//!    synchronous entry points (CLI, tests, REPL one-shots) that are not
//!    running under a tokio task. A [`SyncRegistryScope`] RAII guard
//!    pushes/pops the value so nested Runtimes on one thread compose
//!    correctly.
//! 3. **Process-wide default** (`DEFAULT_SCHEMA_REGISTRY`) — a single
//!    stdlib-populated registry shared by scopeless callers. Preserves the
//!    pre-B1.7 semantic that every caller sees *some* registry: ad-hoc
//!    tooling, static initialisers, and unit tests that don't install a
//!    scope all share this handle instead of being forced to panic or
//!    observe `None`.
//!
//! Mirrors the B5 `shape_value::shape_graph_current::DEFAULT_SHAPE_TABLE`
//! pattern that retired the legacy `GLOBAL_SHAPE_TABLE` static.
//!
//! Lookup order: task-local → thread-local → process default. Both
//! [`current_registry`] and [`try_current_registry`] always return a
//! usable handle; callers never need to branch on `None` or on a panic
//! any more. Scoped callers (installed by `Runtime`) still get per-VM
//! isolation, which was the original B1 goal.

use super::TypeSchemaRegistry;
use std::cell::RefCell;
use std::future::Future;
use std::sync::{Arc, LazyLock};

tokio::task_local! {
    /// Task-local current registry. Set by the `with_async_scope` helper
    /// around any async execution entry. Inherited by all descendant
    /// `.await`s of that future.
    static CURRENT_SCHEMA_REGISTRY: Arc<TypeSchemaRegistry>;
}

thread_local! {
    /// Synchronous fallback. Managed exclusively by [`SyncRegistryScope`]
    /// for push/pop semantics — callers should not touch this directly.
    static SYNC_CURRENT_SCHEMA_REGISTRY: RefCell<Option<Arc<TypeSchemaRegistry>>> =
        const { RefCell::new(None) };
}

/// Process-wide default registry used when neither a task-local nor a
/// thread-local scope is active.
///
/// Callers that poke `lookup_schema_for_fields` /
/// `register_predeclared_any_schema` directly from a stdlib helper or a
/// unit test — without a `Runtime::enter_schema_scope` — historically
/// relied on the pre-B1.7 `STDLIB_SCHEMA_REGISTRY` /
/// `FALLBACK_PREDECLARED_REGISTRY` statics always being available. This
/// fallback preserves that semantic: scopeless callers share one
/// isolated-per-process registry instead of panicking or getting `None`.
/// Scoped callers (Runtime-installed) still get per-VM isolation.
///
/// The registry is seeded with the canonical stdlib types
/// (Row / Option / Result / builtin fixed-layout) via
/// [`TypeSchemaRegistry::new_with_stdlib`] so predeclared-schema
/// resolution can match against the same stdlib surface that scoped
/// registries expose.
static DEFAULT_SCHEMA_REGISTRY: LazyLock<Arc<TypeSchemaRegistry>> =
    LazyLock::new(|| Arc::new(TypeSchemaRegistry::new_with_stdlib()));

/// Return the process-wide default schema-registry handle.
///
/// Exposed so downstream crates and tests that want to explicitly mirror
/// a schema into the default registry (for example, snapshot-decoding
/// tooling that runs outside any Runtime scope) can do so without
/// constructing a fresh registry and losing shared predeclared caches.
pub fn default_registry() -> Arc<TypeSchemaRegistry> {
    DEFAULT_SCHEMA_REGISTRY.clone()
}

/// RAII guard that installs a registry on the thread-local slot for the
/// lifetime of the guard.
///
/// The previous value (if any) is captured on construction and restored on
/// drop, so nested scopes compose correctly. Used by synchronous Runtime
/// entry points (CLI, unit tests, REPL one-shot) that are not running under
/// a tokio task.
///
/// ```ignore
/// let _scope = SyncRegistryScope::enter(runtime.schema_registry_arc());
/// // ... invoke any code that calls current_registry() ...
/// // scope restores previous value on drop
/// ```
#[must_use = "the scope only lives as long as the guard is held"]
pub struct SyncRegistryScope {
    prev: Option<Arc<TypeSchemaRegistry>>,
}

impl SyncRegistryScope {
    /// Install `registry` as the current thread-local registry, saving the
    /// previous value for restoration on drop.
    pub fn enter(registry: Arc<TypeSchemaRegistry>) -> Self {
        let prev = SYNC_CURRENT_SCHEMA_REGISTRY
            .with(|cell| cell.borrow_mut().replace(registry));
        Self { prev }
    }
}

impl Drop for SyncRegistryScope {
    fn drop(&mut self) {
        SYNC_CURRENT_SCHEMA_REGISTRY.with(|cell| {
            *cell.borrow_mut() = self.prev.take();
        });
    }
}

/// Return a handle to the current ambient `TypeSchemaRegistry`.
///
/// Lookup order: task-local → thread-local → process-wide default. This
/// function never panics and always returns a usable registry. Scoped
/// callers get their own per-Runtime registry; scopeless callers share
/// the process-wide default seeded with canonical stdlib types.
pub fn current_registry() -> Arc<TypeSchemaRegistry> {
    if let Ok(r) = CURRENT_SCHEMA_REGISTRY.try_with(|r| r.clone()) {
        return r;
    }
    if let Some(r) = SYNC_CURRENT_SCHEMA_REGISTRY.with(|cell| cell.borrow().clone()) {
        return r;
    }
    DEFAULT_SCHEMA_REGISTRY.clone()
}

/// Alias for [`current_registry`] returning `Option` for historical API
/// compatibility.
///
/// Returns `Some(registry)` unconditionally — the process-wide default is
/// always available. Retained so pre-B1.7 call sites that matched on
/// `Option` compile without churn; prefer [`current_registry`] in new code.
pub fn try_current_registry() -> Option<Arc<TypeSchemaRegistry>> {
    Some(current_registry())
}

/// Run `fut` with `registry` installed as the task-local current registry.
///
/// The installed registry is inherited by all descendant `.await` points of
/// `fut` and survives tokio task migration across worker threads.
pub async fn with_async_scope<R>(
    registry: Arc<TypeSchemaRegistry>,
    fut: impl Future<Output = R>,
) -> R {
    CURRENT_SCHEMA_REGISTRY.scope(registry, fut).await
}

/// Test-only helper: construct a default scope with a fresh
/// stdlib-populated registry. Held by the returned guard; drop it to
/// restore the previous value.
///
/// Prefer this in unit tests that indirectly touch `current_registry()` so
/// they don't all need to thread a registry manually.
#[cfg(test)]
pub(crate) fn test_runtime_scope() -> SyncRegistryScope {
    let registry = Arc::new(TypeSchemaRegistry::new_with_stdlib());
    SyncRegistryScope::enter(registry)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::type_schema::FieldType;

    #[test]
    fn sync_scope_push_pop_restores_previous() {
        let r1 = Arc::new(TypeSchemaRegistry::new_with_stdlib());
        let r2 = Arc::new(TypeSchemaRegistry::new_with_stdlib());

        // Baseline is the process-wide default handle (B1.7: no more None).
        let baseline = current_registry();
        assert!(Arc::ptr_eq(&baseline, &DEFAULT_SCHEMA_REGISTRY));

        let outer = SyncRegistryScope::enter(r1.clone());
        assert!(Arc::ptr_eq(&current_registry(), &r1));

        {
            let inner = SyncRegistryScope::enter(r2.clone());
            assert!(Arc::ptr_eq(&current_registry(), &r2));
            drop(inner);
        }

        // r1 restored after inner drop
        assert!(Arc::ptr_eq(&current_registry(), &r1));
        drop(outer);

        // Default fallback visible again after outer drop.
        assert!(Arc::ptr_eq(&current_registry(), &baseline));
    }

    #[test]
    fn current_registry_falls_back_to_process_default_without_scope() {
        // On a fresh thread with no installed scope, the process-wide
        // default handle is returned so scopeless stdlib / unit-test
        // callers retain pre-B1.7 predeclared-schema semantics.
        let first = current_registry();
        let second = current_registry();
        assert!(Arc::ptr_eq(&first, &second));
        // The default is a populated stdlib registry.
        assert!(first.has_type("Row"));
        assert!(first.has_type("Option"));
        assert!(first.has_type("Result"));
    }

    #[test]
    fn test_runtime_scope_installs_stdlib_registry() {
        let _guard = test_runtime_scope();
        let reg = current_registry();
        assert!(reg.has_type("Row"));
        assert!(reg.has_type("Option"));
        assert!(reg.has_type("Result"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn async_scope_survives_task_migration() {
        let registry = Arc::new(TypeSchemaRegistry::new_with_stdlib());
        let expected_id = registry.clone();

        with_async_scope(registry, async move {
            // Force the task to yield so the scheduler may migrate us.
            tokio::task::yield_now().await;

            let observed = current_registry();
            assert!(Arc::ptr_eq(&observed, &expected_id));

            // And nested scopes compose normally.
            let mut inner = TypeSchemaRegistry::new();
            inner.register_type("Inner", vec![("n".to_string(), FieldType::F64)]);
            let inner = Arc::new(inner);

            with_async_scope(inner.clone(), async {
                tokio::task::yield_now().await;
                assert!(Arc::ptr_eq(&current_registry(), &inner));
            })
            .await;

            // Outer scope restored.
            assert!(Arc::ptr_eq(&current_registry(), &expected_id));
        })
        .await;
    }

    #[test]
    fn task_local_takes_precedence_over_thread_local() {
        // Build a single-thread runtime so we stay on this test's thread
        // and can observe the thread-local being visible from inside.
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("current-thread runtime");

        let sync_reg = Arc::new(TypeSchemaRegistry::new_with_stdlib());
        let async_reg = Arc::new(TypeSchemaRegistry::new_with_stdlib());

        let _guard = SyncRegistryScope::enter(sync_reg.clone());
        // Without an async scope, the sync one wins.
        assert!(Arc::ptr_eq(&current_registry(), &sync_reg));

        rt.block_on(async {
            // Task-local overrides thread-local while it's active.
            with_async_scope(async_reg.clone(), async {
                assert!(Arc::ptr_eq(&current_registry(), &async_reg));
            })
            .await;
        });

        // After the async scope ends, the thread-local is visible again.
        assert!(Arc::ptr_eq(&current_registry(), &sync_reg));
    }
}
