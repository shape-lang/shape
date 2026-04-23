//! Task-local "current" `TypeSchemaRegistry` handle.
//!
//! This module exposes an ambient registry that is pushed by `Runtime`'s
//! execution entry points and consumed by free functions that previously
//! reached for the process-global `STDLIB_SCHEMA_REGISTRY` / `NEXT_SCHEMA_ID`
//! statics. The scoping is two-layered to cover both async and synchronous
//! entry points:
//!
//! 1. **Task-local** (`CURRENT_SCHEMA_REGISTRY`) — survives task migration
//!    across tokio worker threads, so any descendant `.await` in a Shape
//!    execution future inherits the registry automatically.
//! 2. **Thread-local** (`SYNC_CURRENT_SCHEMA_REGISTRY`) — fallback for
//!    synchronous entry points (CLI, tests, REPL one-shots) that are not
//!    running under a tokio task. A [`SyncRegistryScope`] RAII guard
//!    pushes/pops the value so nested Runtimes on one thread compose
//!    correctly.
//!
//! Lookup order: task-local → thread-local. If neither is set,
//! [`current_registry`] panics with a message pointing callers at the two
//! entry helpers. This is deliberate: it fails fast during development when
//! a new code path forgets to establish a scope.
//!
//! # Migration status (Track B1)
//!
//! Introduced in B1.3 as the plumbing layer. B1.4 migrates free-function
//! call sites (`next_schema_id()`, `STDLIB_SCHEMA_REGISTRY.get_*()`) to
//! consult [`current_registry`]. B1.6 moves `PREDECLARED_SCHEMA_*` statics
//! onto `TypeSchemaRegistry`. B1.7 removes the legacy globals entirely.

use super::TypeSchemaRegistry;
use std::cell::RefCell;
use std::future::Future;
use std::sync::Arc;

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
/// Checks the task-local slot first, then falls back to the thread-local
/// slot. Panics if neither is set — callers should either wrap async
/// execution in [`with_async_scope`] or hold a [`SyncRegistryScope`] for the
/// duration of the synchronous call.
///
/// # Panics
///
/// Panics if no scope is active. This is intentional: it surfaces missing
/// plumbing during development rather than silently reading from a stale
/// global.
pub fn current_registry() -> Arc<TypeSchemaRegistry> {
    if let Ok(r) = CURRENT_SCHEMA_REGISTRY.try_with(|r| r.clone()) {
        return r;
    }
    SYNC_CURRENT_SCHEMA_REGISTRY
        .with(|cell| cell.borrow().clone())
        .expect(
            "no current TypeSchemaRegistry is active; wrap execution in \
             type_schema::current::with_async_scope or hold a SyncRegistryScope",
        )
}

/// Non-panicking variant of [`current_registry`]. Returns `None` if no
/// scope is active. Useful for callers that have a legitimate fallback
/// path (e.g. pre-runtime bootstrap) during the B1 migration window.
pub fn try_current_registry() -> Option<Arc<TypeSchemaRegistry>> {
    if let Ok(r) = CURRENT_SCHEMA_REGISTRY.try_with(|r| r.clone()) {
        return Some(r);
    }
    SYNC_CURRENT_SCHEMA_REGISTRY.with(|cell| cell.borrow().clone())
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

        assert!(try_current_registry().is_none());

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

        // nothing after outer drop
        assert!(try_current_registry().is_none());
    }

    #[test]
    #[should_panic(expected = "no current TypeSchemaRegistry")]
    fn current_registry_panics_without_scope() {
        // Make sure no ambient scope from a parallel test leaks in; the
        // thread-local is owned per-thread and the panic message is the
        // contract we advertise.
        assert!(try_current_registry().is_none());
        let _ = current_registry();
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
