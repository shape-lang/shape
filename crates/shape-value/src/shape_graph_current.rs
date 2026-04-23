//! Task-local "current" `ShapeTransitionTable` handle.
//!
//! Mirrors the pattern established in
//! `shape-runtime::type_schema::current` for `TypeSchemaRegistry` (B1.3),
//! but lives in `shape-value` because `ShapeTransitionTable` is a
//! shape-value type and pulling it up into shape-runtime would invert
//! the crate dependency.
//!
//! Two layers, consulted in order:
//!
//! 1. **Task-local** (`CURRENT_SHAPE_TABLE`) — survives task migration
//!    across tokio worker threads so any descendant `.await` in a Shape
//!    execution future inherits the handle automatically.
//! 2. **Thread-local** (`SYNC_CURRENT_SHAPE_TABLE`) — fallback for
//!    synchronous entry points (CLI, tests, REPL one-shots) that are not
//!    running under a tokio task. A [`SyncShapeTableScope`] RAII guard
//!    pushes/pops the value so nested scopes compose correctly.
//!
//! Unlike the type-schema current module, this one exposes a
//! [`try_current_shape_table`] accessor that is the primary lookup used
//! by `shape_graph`'s free functions (`shape_transition`,
//! `shape_for_hashmap_keys`, `shape_property_index`,
//! `drain_shape_transitions`). When no scope is active those free
//! functions degrade to `None` / empty-drain — this preserves the
//! existing "fall back to hash lookup, no shape tracking" semantic that
//! was already returned for lock-poisoned or overflow cases by the
//! previous global-backed implementation, which keeps unit tests that
//! poke `HashMapData::compute_shape` without a VM alive.

use crate::shape_graph::{ShapeId, ShapeTransitionTable};
use std::cell::RefCell;
use std::future::Future;
use std::sync::{Arc, Mutex};

/// Shareable handle to a shape transition table and its transition log.
///
/// The table is the same object that the pre-B5 `GLOBAL_SHAPE_TABLE`
/// exposed — a `Mutex`-guarded transition graph. The log records
/// `(parent, child)` pairs for JIT shape-guard invalidation and is
/// drained by `TierManager::check_shape_invalidations`.
///
/// The interior `Mutex`s are deliberately simple: table writes are
/// expected to be rare (only when a HashMap gains a new key) and the
/// lock is held briefly.
pub struct ShapeTableHandle {
    table: Mutex<ShapeTransitionTable>,
    transition_log: Mutex<Vec<(ShapeId, ShapeId)>>,
}

impl ShapeTableHandle {
    /// Build a fresh handle over an empty transition table.
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            table: Mutex::new(ShapeTransitionTable::new()),
            transition_log: Mutex::new(Vec::new()),
        })
    }

    /// Access the inner transition table mutex.
    #[inline]
    pub fn table(&self) -> &Mutex<ShapeTransitionTable> {
        &self.table
    }

    /// Access the inner transition-log mutex.
    #[inline]
    pub fn transition_log(&self) -> &Mutex<Vec<(ShapeId, ShapeId)>> {
        &self.transition_log
    }
}

impl Default for ShapeTableHandle {
    fn default() -> Self {
        Self {
            table: Mutex::new(ShapeTransitionTable::new()),
            transition_log: Mutex::new(Vec::new()),
        }
    }
}

tokio::task_local! {
    /// Task-local current handle. Set by [`with_async_shape_table_scope`]
    /// around any async execution entry. Inherited by all descendant
    /// `.await`s of that future.
    static CURRENT_SHAPE_TABLE: Arc<ShapeTableHandle>;
}

thread_local! {
    /// Synchronous fallback. Managed exclusively by
    /// [`SyncShapeTableScope`] for push/pop semantics.
    static SYNC_CURRENT_SHAPE_TABLE: RefCell<Option<Arc<ShapeTableHandle>>> =
        const { RefCell::new(None) };
}

/// RAII guard that installs a shape-table handle on the thread-local
/// slot for the lifetime of the guard.
///
/// The previous value (if any) is captured on construction and restored
/// on drop, so nested scopes compose correctly. Used by synchronous VM
/// execution entry points (CLI, unit tests, REPL one-shot) that are not
/// running under a tokio task.
#[must_use = "the scope only lives as long as the guard is held"]
pub struct SyncShapeTableScope {
    prev: Option<Arc<ShapeTableHandle>>,
}

impl SyncShapeTableScope {
    /// Install `handle` as the current thread-local shape-table handle,
    /// saving the previous value for restoration on drop.
    pub fn enter(handle: Arc<ShapeTableHandle>) -> Self {
        let prev = SYNC_CURRENT_SHAPE_TABLE
            .with(|cell| cell.borrow_mut().replace(handle));
        Self { prev }
    }
}

impl Drop for SyncShapeTableScope {
    fn drop(&mut self) {
        SYNC_CURRENT_SHAPE_TABLE.with(|cell| {
            *cell.borrow_mut() = self.prev.take();
        });
    }
}

/// Return the current ambient shape-table handle, or `None` if no scope
/// is active.
///
/// Checks the task-local slot first, then falls back to the
/// thread-local slot. Returning `None` rather than panicking is
/// deliberate: the shape-graph free functions that consult this handle
/// (`shape_transition`, `shape_for_hashmap_keys`,
/// `shape_property_index`, `drain_shape_transitions`) already return
/// `Option`/`Vec` and already degrade gracefully when no shape table is
/// accessible.
pub fn try_current_shape_table() -> Option<Arc<ShapeTableHandle>> {
    if let Ok(h) = CURRENT_SHAPE_TABLE.try_with(|h| h.clone()) {
        return Some(h);
    }
    SYNC_CURRENT_SHAPE_TABLE.with(|cell| cell.borrow().clone())
}

/// Panicking variant of [`try_current_shape_table`]. Callers that are
/// guaranteed to execute within a VM scope may prefer this for fast-
/// fail diagnostics.
///
/// # Panics
///
/// Panics if no scope is active.
pub fn current_shape_table() -> Arc<ShapeTableHandle> {
    try_current_shape_table().expect(
        "no current ShapeTransitionTable is active; wrap execution in \
         shape_graph_current::with_async_shape_table_scope or hold a \
         SyncShapeTableScope",
    )
}

/// Run `fut` with `handle` installed as the task-local current shape
/// table. Inherited by all descendant `.await` points and survives
/// tokio task migration across worker threads.
pub async fn with_async_shape_table_scope<R>(
    handle: Arc<ShapeTableHandle>,
    fut: impl Future<Output = R>,
) -> R {
    CURRENT_SHAPE_TABLE.scope(handle, fut).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_scope_push_pop_restores_previous() {
        let h1 = ShapeTableHandle::new();
        let h2 = ShapeTableHandle::new();

        assert!(try_current_shape_table().is_none());

        let outer = SyncShapeTableScope::enter(h1.clone());
        assert!(Arc::ptr_eq(&current_shape_table(), &h1));

        {
            let inner = SyncShapeTableScope::enter(h2.clone());
            assert!(Arc::ptr_eq(&current_shape_table(), &h2));
            drop(inner);
        }

        // Outer restored after inner drop.
        assert!(Arc::ptr_eq(&current_shape_table(), &h1));
        drop(outer);

        // Nothing after outer drop.
        assert!(try_current_shape_table().is_none());
    }

    #[test]
    fn try_current_returns_none_without_scope() {
        // On a fresh thread (cargo test runs each test on its own
        // thread-local storage), no scope is installed by default.
        assert!(try_current_shape_table().is_none());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn async_scope_survives_task_migration() {
        let handle = ShapeTableHandle::new();
        let expected = handle.clone();

        with_async_shape_table_scope(handle, async move {
            tokio::task::yield_now().await;
            let observed = current_shape_table();
            assert!(Arc::ptr_eq(&observed, &expected));

            // Nested scopes compose normally.
            let inner = ShapeTableHandle::new();
            with_async_shape_table_scope(inner.clone(), async {
                tokio::task::yield_now().await;
                assert!(Arc::ptr_eq(&current_shape_table(), &inner));
            })
            .await;

            assert!(Arc::ptr_eq(&current_shape_table(), &expected));
        })
        .await;
    }

    #[test]
    fn task_local_takes_precedence_over_thread_local() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("current-thread runtime");

        let sync_handle = ShapeTableHandle::new();
        let async_handle = ShapeTableHandle::new();

        let _guard = SyncShapeTableScope::enter(sync_handle.clone());
        assert!(Arc::ptr_eq(&current_shape_table(), &sync_handle));

        rt.block_on(async {
            with_async_shape_table_scope(async_handle.clone(), async {
                assert!(Arc::ptr_eq(&current_shape_table(), &async_handle));
            })
            .await;
        });

        // After the async scope ends, the thread-local is visible again.
        assert!(Arc::ptr_eq(&current_shape_table(), &sync_handle));
    }
}
