//! Task scheduler for the async host runtime.
//!
//! Manages spawned async tasks: stores their callables, tracks completion,
//! and executes them (synchronously for now) when the VM suspends on an await.
//!
//! The initial design runs tasks inline (synchronous execution at await-time).
//! True concurrent execution via Tokio can be layered on later by changing
//! `resolve_task` to spawn on the Tokio runtime.
//!
//! ## Wave 6.5 R-async-time / E-async surface follow-up
//!
//! The pre-bulldozer scheduler stored callables and results as `ValueWord`
//! and exposed an `executor_fn: FnOnce(ValueWord) -> Result<ValueWord, _>`
//! callback for inline execution. `ValueWord` is deleted per ADR-006 §2.7
//! / CLAUDE.md "Forbidden Patterns"; the post-§2.7.7 carrier shape is the
//! `(bits: u64, kind: NativeKind)` pair (the same shape `pop_kinded()` /
//! `push_kinded(...)` thread through the typed VM stack — see playbook
//! §3 canonical pattern). This file's API now takes and returns kinded
//! pairs end-to-end:
//!
//! - `register(task_id, callable_bits, callable_kind)`
//! - `take_callable(task_id) -> Option<(u64, NativeKind)>`
//! - `complete(task_id, result_bits, result_kind)`
//! - `register_external(task_id) -> oneshot::Sender<Result<(u64, NativeKind), String>>`
//! - `resolve_task<F>(task_id, executor_fn)` where
//!   `F: FnOnce((u64, NativeKind)) -> Result<(u64, NativeKind), VMError>`
//!
//! Refcount discipline (playbook §3 drop discipline): every share stored
//! in the scheduler's maps owns one strong-count for heap-bearing kinds.
//! `take_callable`, `take_external_receiver`, `try_resolve_external`, and
//! `Drop` transfer that share to the caller (or release it). `register`,
//! `complete`, and the cached-result paths in `resolve_task` /
//! `resolve_task_group` use `clone_with_kind` when handing the share to
//! a second consumer.
//!
//! Out-of-territory callers (`call_convention.rs::resolve_spawned_task`,
//! `async_ops/mod.rs::op_await` / `op_spawn_task` / `op_join_await`,
//! `gc_integration.rs::scan_roots`) still reference deleted ValueWord-shape
//! APIs; their migration is owned by separate sub-clusters and is out of
//! R-async-time scope per playbook §10 dispatch protocol.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::Receiver;

use shape_runtime::typed_module_exports::TypedReturn;
use shape_value::heap_value::{HeapKind, TaskGroupData};
use shape_value::{NativeKind, VMError};
use tokio::task::AbortHandle;

use crate::executor::vm_impl::stack::{clone_with_kind, drop_with_kind};

/// A kinded value held by the scheduler (post-§2.7.7 carrier shape).
///
/// `bits` is the raw 8-byte slot payload; `kind` is the parallel-track
/// `NativeKind` interpretation. The pair owns one strong-count share for
/// heap-bearing kinds; producing a copy bumps it via `clone_with_kind`,
/// dropping it releases via `drop_with_kind`.
type Kinded = (u64, NativeKind);

/// An async module-function task that is genuinely in flight on the shared
/// background runtime (WF-2D real concurrency).
///
/// The `time::sleep`-style future was `spawn`ed onto
/// [`crate::executor::async_runtime::shared_runtime`] the instant the async
/// module call was evaluated; it is making progress on a worker thread right
/// now. `completion` delivers the body's raw `Result<TypedReturn, String>`
/// back to the interpreter thread (blocking `recv` for `await`, non-blocking
/// `try_recv` for the `race`/`any` first-completion poll). `abort` lets
/// `race`/`any` losers and `async scope` exit genuinely cancel the underlying
/// tokio task rather than let it run to completion unobserved.
pub struct PendingAsyncTask {
    /// Delivers the async body result from the worker thread. Projection into a
    /// `KindedSlot` happens on the interpreter thread (it needs the VM's schema
    /// registry), so the channel carries the body's own payload rather than a
    /// projected kinded pair.
    pub completion: AsyncCompletion,
    /// Cancels an abortable in-flight tokio task. Detached remote socket
    /// workers use `None`; their receiver-side cancellation is driven by the
    /// companion hook.
    ///
    /// For an offloaded foreign invoke this aborts the blocking-pool task, which
    /// does NOT interrupt a closure that has already started: the foreign body
    /// runs to completion and its result is discarded. It is a discard, never a
    /// confirmation that the foreign runtime stopped (ADR-019 §5 / #202).
    pub abort: Option<AbortHandle>,
}

/// The completion channel of an in-flight async task, by what crosses back.
///
/// Two payloads share one pending-task table so `await`, `join`, `race` and
/// `any` need no knowledge of which kind of body they are collecting.
pub enum AsyncCompletion {
    /// A module or user async body: an owned, `Send` `TypedReturn`.
    Typed(Receiver<Result<TypedReturn, String>>),
    /// An `async fn <language>` foreign invoke (ADR-019 §5 / #202): the
    /// extension's raw msgpack return payload. Unmarshalling is deferred to the
    /// interpreter thread, which owns the schema registry and the declared
    /// return type that `foreign_idx` names.
    Foreign {
        foreign_idx: usize,
        bytes: Receiver<Result<Vec<u8>, String>>,
    },
}

/// A completed async body, still in its own payload form.
pub enum AsyncTaskOutcome {
    Typed(Result<TypedReturn, String>),
    Foreign {
        foreign_idx: usize,
        result: Result<Vec<u8>, String>,
    },
}

impl AsyncCompletion {
    /// Block until the body completes.
    pub fn recv(&self) -> Result<AsyncTaskOutcome, std::sync::mpsc::RecvError> {
        match self {
            Self::Typed(rx) => rx.recv().map(AsyncTaskOutcome::Typed),
            Self::Foreign { foreign_idx, bytes } => {
                bytes.recv().map(|result| AsyncTaskOutcome::Foreign {
                    foreign_idx: *foreign_idx,
                    result,
                })
            }
        }
    }

    /// Poll without blocking (the `race` / `any` first-completion spin).
    pub fn try_recv(&self) -> Result<AsyncTaskOutcome, std::sync::mpsc::TryRecvError> {
        match self {
            Self::Typed(rx) => rx.try_recv().map(AsyncTaskOutcome::Typed),
            Self::Foreign { foreign_idx, bytes } => {
                bytes.try_recv().map(|result| AsyncTaskOutcome::Foreign {
                    foreign_idx: *foreign_idx,
                    result,
                })
            }
        }
    }
}

/// Completion status of a spawned task.
#[derive(Debug, Clone)]
pub enum TaskStatus {
    /// Task has been spawned but not yet executed.
    Pending,
    /// Task finished successfully with a result value (kinded pair).
    Completed(Kinded),
    /// Task was cancelled before completion.
    Cancelled,
}

/// Snapshot-facing status of a `Future(id)` handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FutureSnapshotStatus {
    /// No scheduler entry exists for this id.
    Unknown,
    /// A callable is registered and has not yet run.
    PendingCallable,
    /// An external completion receiver is still waiting.
    PendingExternal,
    /// A module/user/remote async body is in flight on the shared runtime.
    PendingAsync,
    /// The future's result has been cached, but the handle itself still needs
    /// scheduler state to be meaningful after restore.
    Completed,
    /// The task was cancelled.
    Cancelled,
}

impl std::fmt::Display for FutureSnapshotStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown => write!(f, "unknown scheduler entry"),
            Self::PendingCallable => write!(f, "pending callable"),
            Self::PendingExternal => write!(f, "pending external completion"),
            Self::PendingAsync => write!(f, "pending async task"),
            Self::Completed => write!(f, "completed result handle"),
            Self::Cancelled => write!(f, "cancelled task"),
        }
    }
}

/// Scheduler that tracks spawned async tasks by their future ID.
///
/// The VM's `SpawnTask` opcode registers a callable here. When the VM later
/// suspends on `WaitType::Future { id }`, the host looks up the callable,
/// executes it, and stores the result so the VM can resume.
///
/// Supports both inline tasks (callable executed synchronously at await-time)
/// and external tasks (completed by background Tokio tasks via oneshot channels).
pub struct TaskScheduler {
    /// Map from task_id to the callable kinded pair (Closure or Function bits
    /// plus its `NativeKind`) that was passed to `spawn`. Consumed on first
    /// execution.
    callables: HashMap<u64, Kinded>,

    /// Map from task_id to its completion status.
    results: HashMap<u64, TaskStatus>,

    /// External completion channels — Tokio background tasks send results here.
    /// Used for remote calls and other externally-completed futures.
    /// Result is a kinded pair on success.
    external_receivers: HashMap<u64, tokio::sync::oneshot::Receiver<Result<Kinded, String>>>,

    /// In-flight async module-function tasks (WF-2D real concurrency). Each
    /// entry is a `time::sleep`-style future already running on the shared
    /// background runtime; the interpreter thread collects the result via the
    /// task's `completion` channel at `await`/`join` time.
    pending_async: HashMap<u64, PendingAsyncTask>,

    /// Optional side effects to notify external owners that a pending async
    /// task was cancelled. Used by `remote::call_async` to send a best-effort
    /// receiver cancellation keyed by the internal wire call id.
    pending_async_cancel_hooks: HashMap<u64, Box<dyn FnOnce() + Send + 'static>>,
}

impl TaskScheduler {
    /// Create a new, empty scheduler.
    pub fn new() -> Self {
        Self {
            callables: HashMap::new(),
            results: HashMap::new(),
            external_receivers: HashMap::new(),
            pending_async: HashMap::new(),
            pending_async_cancel_hooks: HashMap::new(),
        }
    }

    /// Record an in-flight async module-function task (WF-2D real concurrency).
    ///
    /// Called by `spawn_async_module_future` the instant an async module call
    /// is evaluated: the future is already `spawn`ed onto the shared
    /// background runtime and running. The scheduler marks the task `Pending`
    /// so scope-tracking / `is_resolved` behave uniformly with the other task
    /// kinds, and stores the completion channel + abort handle.
    pub fn store_pending_async(&mut self, task_id: u64, task: PendingAsyncTask) {
        self.results.insert(task_id, TaskStatus::Pending);
        self.pending_async_cancel_hooks.remove(&task_id);
        self.pending_async.insert(task_id, task);
    }

    /// Attach a cancellation hook to an already-registered pending async task.
    ///
    /// The hook is consumed at most once: on explicit cancellation or VM
    /// teardown while the task is still pending. Normal await/join completion
    /// drops the hook without running it.
    pub fn set_pending_async_cancellation_hook(
        &mut self,
        task_id: u64,
        hook: Box<dyn FnOnce() + Send + 'static>,
    ) {
        if self.pending_async.contains_key(&task_id) {
            self.pending_async_cancel_hooks.insert(task_id, hook);
        }
    }

    /// Whether `task_id` names an in-flight async module-function task.
    pub fn has_pending_async(&self, task_id: u64) -> bool {
        self.pending_async.contains_key(&task_id)
    }

    /// Snapshot-facing status for a `Future(id)` handle.
    ///
    /// VM snapshots do not persist scheduler state yet, so any live future
    /// handle is non-restorable. This introspection keeps the capture-side
    /// diagnostic precise without exposing scheduler internals to the
    /// serializer.
    pub fn future_snapshot_status(&self, task_id: u64) -> FutureSnapshotStatus {
        if self.pending_async.contains_key(&task_id) {
            return FutureSnapshotStatus::PendingAsync;
        }
        if self.external_receivers.contains_key(&task_id) {
            return FutureSnapshotStatus::PendingExternal;
        }
        match self.results.get(&task_id) {
            Some(TaskStatus::Pending) if self.callables.contains_key(&task_id) => {
                FutureSnapshotStatus::PendingCallable
            }
            Some(TaskStatus::Pending) => FutureSnapshotStatus::Unknown,
            Some(TaskStatus::Completed(_)) => FutureSnapshotStatus::Completed,
            Some(TaskStatus::Cancelled) => FutureSnapshotStatus::Cancelled,
            None => FutureSnapshotStatus::Unknown,
        }
    }

    /// Take (remove) the in-flight async task so the caller can drive its
    /// completion channel. Ownership of the receiver + abort handle transfers
    /// out.
    pub fn take_pending_async(&mut self, task_id: u64) -> Option<PendingAsyncTask> {
        self.pending_async_cancel_hooks.remove(&task_id);
        self.pending_async.remove(&task_id)
    }

    /// Non-blocking poll of an in-flight async task's completion channel
    /// WITHOUT removing the entry (used by the `race`/`any` first-completion
    /// spin loop). Returns `None` if no such pending-async task exists.
    #[allow(clippy::type_complexity)]
    pub fn peek_pending_async_try_recv(
        &mut self,
        task_id: u64,
    ) -> Option<Result<AsyncTaskOutcome, std::sync::mpsc::TryRecvError>> {
        self.pending_async
            .get_mut(&task_id)
            .map(|task| task.completion.try_recv())
    }

    /// Register a callable for a given task_id.
    ///
    /// Called by `op_spawn_task` when a new task is spawned. The caller
    /// transfers one strong-count share for the kinded pair into the
    /// scheduler; on `take_callable` (or `Drop`) the share transfers back
    /// out (or is released).
    pub fn register(&mut self, task_id: u64, callable_bits: u64, callable_kind: NativeKind) {
        // Replace any prior callable to preserve refcount discipline.
        if let Some((old_bits, old_kind)) = self.callables.remove(&task_id) {
            drop_with_kind(old_bits, old_kind);
        }
        self.callables
            .insert(task_id, (callable_bits, callable_kind));
        self.results.insert(task_id, TaskStatus::Pending);
    }

    /// Take (remove) the callable for `task_id` so it can be executed.
    ///
    /// Returns `None` if the task was already consumed or never registered.
    /// Ownership of the kinded pair transfers to the caller.
    pub fn take_callable(&mut self, task_id: u64) -> Option<Kinded> {
        self.callables.remove(&task_id)
    }

    /// Record a completed result for a task.
    ///
    /// The caller transfers one strong-count share into the scheduler. If
    /// a completion was already recorded, the prior share is released.
    pub fn complete(&mut self, task_id: u64, value_bits: u64, value_kind: NativeKind) {
        // Releasing a prior result preserves refcount discipline if a task
        // is somehow completed twice (defensive — should not normally happen).
        if let Some(TaskStatus::Completed((old_bits, old_kind))) = self
            .results
            .insert(task_id, TaskStatus::Completed((value_bits, value_kind)))
        {
            drop_with_kind(old_bits, old_kind);
        }
    }

    /// Mark a task as cancelled.
    pub fn cancel(&mut self, task_id: u64) {
        // In-flight async module task: abort the underlying tokio task so a
        // cancelled `time::sleep` really stops rather than run to completion
        // unobserved (WF-2D real cancellation).
        if let Some(hook) = self.pending_async_cancel_hooks.remove(&task_id) {
            hook();
        }
        if let Some(task) = self.pending_async.remove(&task_id)
            && let Some(abort) = task.abort
        {
            abort.abort();
        }
        // Only cancel if still pending
        if let Some(TaskStatus::Pending) = self.results.get(&task_id) {
            self.results.insert(task_id, TaskStatus::Cancelled);
            // Release the callable's share if still present.
            if let Some((bits, kind)) = self.callables.remove(&task_id) {
                drop_with_kind(bits, kind);
            }
        }
    }

    /// Get the result for a task, if it has completed.
    pub fn get_result(&self, task_id: u64) -> Option<&TaskStatus> {
        self.results.get(&task_id)
    }

    /// Check whether a task has a stored result (completed or cancelled).
    pub fn is_resolved(&self, task_id: u64) -> bool {
        matches!(
            self.results.get(&task_id),
            Some(TaskStatus::Completed(_)) | Some(TaskStatus::Cancelled)
        )
    }

    /// Register an externally-completed task (e.g., remote call).
    ///
    /// Returns a `oneshot::Sender` that the background task uses to deliver the
    /// result (kinded pair). The scheduler marks the task as Pending and
    /// stores the receiver.
    pub fn register_external(
        &mut self,
        task_id: u64,
    ) -> tokio::sync::oneshot::Sender<Result<Kinded, String>> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.results.insert(task_id, TaskStatus::Pending);
        self.external_receivers.insert(task_id, rx);
        tx
    }

    /// Try to resolve an external task (non-blocking check).
    ///
    /// Returns `Some(Ok((bits, kind)))` if the external task completed
    /// successfully, `Some(Err(..))` on error/cancellation, or `None` if
    /// still pending.
    ///
    /// On the cached-completion fast path, the cached share is cloned
    /// (`clone_with_kind`) so both the scheduler entry and the returned
    /// pair own independent shares — caller drops/uses freely.
    pub fn try_resolve_external(&mut self, task_id: u64) -> Option<Result<Kinded, VMError>> {
        if let Some(TaskStatus::Completed((bits, kind))) = self.results.get(&task_id).cloned() {
            // Hand out a fresh share — the cached entry retains its own.
            clone_with_kind(bits, kind);
            return Some(Ok((bits, kind)));
        }
        if let Some(rx) = self.external_receivers.get_mut(&task_id) {
            match rx.try_recv() {
                Ok(Ok((bits, kind))) => {
                    // The result share transferred from the background task.
                    // Cache one share (clone) and hand out the original.
                    clone_with_kind(bits, kind);
                    self.results
                        .insert(task_id, TaskStatus::Completed((bits, kind)));
                    self.external_receivers.remove(&task_id);
                    Some(Ok((bits, kind)))
                }
                Ok(Err(e)) => {
                    self.external_receivers.remove(&task_id);
                    Some(Err(VMError::RuntimeError(e)))
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => None,
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                    self.external_receivers.remove(&task_id);
                    Some(Err(VMError::RuntimeError(
                        "Remote task cancelled".to_string(),
                    )))
                }
            }
        } else {
            None
        }
    }

    /// Check whether a task has an external receiver (is externally-completed).
    pub fn has_external(&self, task_id: u64) -> bool {
        self.external_receivers.contains_key(&task_id)
    }

    /// Take the external receiver for async awaiting.
    ///
    /// Used by `execute_with_async` when it needs to truly `.await` an external
    /// task's completion.
    pub fn take_external_receiver(
        &mut self,
        task_id: u64,
    ) -> Option<tokio::sync::oneshot::Receiver<Result<Kinded, String>>> {
        self.external_receivers.remove(&task_id)
    }

    /// Resolve a single task by executing its callable on a fresh VM executor.
    ///
    /// This is the synchronous (inline) strategy: the callable is executed
    /// immediately when awaited. Returns the result kinded pair, or an error.
    ///
    /// The `executor_fn` callback receives the callable kinded pair and must
    /// execute it, returning the result kinded pair. Ownership of the pair
    /// transfers into the callback; the callback's returned pair owns one
    /// share which is then cached and a clone returned to the caller.
    pub fn resolve_task<F>(&mut self, task_id: u64, executor_fn: F) -> Result<Kinded, VMError>
    where
        F: FnOnce(Kinded) -> Result<Kinded, VMError>,
    {
        // If already resolved, hand out a clone of the cached share.
        if let Some(TaskStatus::Completed((bits, kind))) = self.results.get(&task_id).cloned() {
            clone_with_kind(bits, kind);
            return Ok((bits, kind));
        }
        if let Some(TaskStatus::Cancelled) = self.results.get(&task_id) {
            return Err(VMError::RuntimeError(format!(
                "Task {} was cancelled",
                task_id
            )));
        }

        // Take the callable (consume it — share transfers to executor_fn).
        let callable = self.take_callable(task_id).ok_or_else(|| {
            VMError::RuntimeError(format!("No callable registered for task {}", task_id))
        })?;

        // Execute synchronously — share transfers in, share transfers out.
        let (bits, kind) = executor_fn(callable)?;

        // Cache a clone of the result; hand the original share back.
        clone_with_kind(bits, kind);
        self.results
            .insert(task_id, TaskStatus::Completed((bits, kind)));
        Ok((bits, kind))
    }

    /// Resolve a task group according to the join strategy.
    ///
    /// Join kinds (encoded in the high 2 bits of JoinInit's packed operand):
    ///   0 = All  — wait for all tasks, return array of results
    ///   1 = Race — return first completed result
    ///   2 = Any  — return first successful result (skip errors)
    ///   3 = AllSettled — return array of {status, value/error} for every task
    ///
    /// Since we execute synchronously, "race" and "any" still run all tasks
    /// sequentially but return early on the first applicable result.
    ///
    /// Returned aggregate is a `TaskGroup`-shaped heap value (Arc<TaskGroupData>)
    /// holding the constituent task ids; the caller pushes the kinded pair
    /// onto the stack with `NativeKind::Ptr(HeapKind::TaskGroup)`. (The
    /// pre-bulldozer code returned a heap array of kinded results; without
    /// a kinded VMArray helper post-§2.7.4, the TaskGroup carrier is the
    /// minimum shape the await-time decoder can re-walk.)
    pub fn resolve_task_group<F>(
        &mut self,
        kind: u8,
        task_ids: &[u64],
        mut executor_fn: F,
    ) -> Result<Kinded, VMError>
    where
        F: FnMut(Kinded) -> Result<Kinded, VMError>,
    {
        match kind {
            // All: collect all results — drop each child share since the
            // aggregate carrier (TaskGroup) holds only ids, not values.
            0 => {
                for &id in task_ids {
                    let (bits, k) = self.resolve_task(id, &mut executor_fn)?;
                    drop_with_kind(bits, k);
                }
                let bits = Arc::into_raw(Arc::new(TaskGroupData {
                    kind: 0,
                    task_ids: task_ids.to_vec(),
                })) as u64;
                Ok((bits, NativeKind::Ptr(HeapKind::TaskGroup)))
            }
            // Race: return first result (all run, but we return first).
            // The loop intentionally returns on the first id (or errors if the
            // task list is empty); the single-iteration shape is deliberate.
            #[allow(clippy::never_loop)]
            1 => {
                for &id in task_ids {
                    let res = self.resolve_task(id, &mut executor_fn)?;
                    return Ok(res);
                }
                Err(VMError::RuntimeError(
                    "Race join with empty task list".to_string(),
                ))
            }
            // Any: return first success, skip errors.
            2 => {
                let mut last_err = None;
                for &id in task_ids {
                    match self.resolve_task(id, &mut executor_fn) {
                        Ok(res) => return Ok(res),
                        Err(e) => last_err = Some(e),
                    }
                }
                Err(last_err.unwrap_or_else(|| {
                    VMError::RuntimeError("Any join with empty task list".to_string())
                }))
            }
            // AllSettled: drive every task, drop each result share, return
            // a TaskGroup with kind=3 so the await-time decoder can rebuild
            // the {status, value/error} array view (Phase-2c work — see
            // ADR-006 §2.7.4).
            3 => {
                for &id in task_ids {
                    if let Ok((bits, k)) = self.resolve_task(id, &mut executor_fn) {
                        drop_with_kind(bits, k);
                    }
                    // Errors per-task are preserved in the scheduler's
                    // result map; the caller can inspect via `get_result`.
                }
                let bits = Arc::into_raw(Arc::new(TaskGroupData {
                    kind: 3,
                    task_ids: task_ids.to_vec(),
                })) as u64;
                Ok((bits, NativeKind::Ptr(HeapKind::TaskGroup)))
            }
            _ => Err(VMError::RuntimeError(format!(
                "Unknown join kind: {}",
                kind
            ))),
        }
    }
}

impl Drop for TaskScheduler {
    /// Release every heap-bearing share the scheduler still owns.
    ///
    /// Required to honor the §2.7.7 retain-on-store contract: every value
    /// inserted via `register` / `complete` carries a strong-count share;
    /// if the scheduler is dropped before consumers retire those shares,
    /// `drop_with_kind` releases them here.
    fn drop(&mut self) {
        for (_, (bits, kind)) in self.callables.drain() {
            drop_with_kind(bits, kind);
        }
        for (_, status) in self.results.drain() {
            if let TaskStatus::Completed((bits, kind)) = status {
                drop_with_kind(bits, kind);
            }
        }
        // pending_async: abort any still-in-flight async module tasks so their
        // background tokio tasks stop when the VM tears down (WF-2D). Dropping
        // the receiver also closes the completion channel; the aborted task's
        // send (if any) fails silently.
        for (task_id, task) in self.pending_async.drain() {
            if let Some(hook) = self.pending_async_cancel_hooks.remove(&task_id) {
                hook();
            }
            if let Some(abort) = task.abort {
                abort.abort();
            }
        }
        self.pending_async_cancel_hooks.clear();
        // external_receivers: Receivers do not own scheduler-side shares;
        // the share is in transit on the channel and the dropping receiver
        // releases it on the sender side.
    }
}

impl std::fmt::Debug for TaskScheduler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaskScheduler")
            .field("callables", &format!("[{} pending]", self.callables.len()))
            .field("results", &format!("[{} entries]", self.results.len()))
            .field(
                "external_receivers",
                &format!("[{} pending]", self.external_receivers.len()),
            )
            .finish()
    }
}

impl Default for TaskScheduler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: a function-id "callable" — inline scalar payload. Post-W11
    /// the dedicated `HeapKind::Function` variant was never added; the
    /// `Future` variant has the same drop-shape (inline scalar, no
    /// Arc-backed retain/release per `kinded_slot.rs:394`) and is the
    /// stand-in test fixture for this scheduler-only register/take/resolve
    /// cycle.
    fn function_callable(func_id: u64) -> Kinded {
        (func_id, NativeKind::Ptr(HeapKind::Future))
    }

    /// Helper: a float result.
    fn float_result(v: f64) -> Kinded {
        (v.to_bits(), NativeKind::Float64)
    }

    #[test]
    fn test_register_and_take_callable() {
        let mut sched = TaskScheduler::new();
        let (bits, kind) = function_callable(42);
        sched.register(1, bits, kind);
        assert!(matches!(sched.get_result(1), Some(TaskStatus::Pending)));

        let callable = sched.take_callable(1);
        assert!(callable.is_some());

        // Second take returns None (consumed)
        assert!(sched.take_callable(1).is_none());
    }

    #[test]
    fn test_resolve_task_synchronous() {
        let mut sched = TaskScheduler::new();
        let (b, k) = function_callable(0);
        sched.register(1, b, k);

        let result = sched.resolve_task(1, |_callable| Ok(float_result(99.0)));
        assert!(result.is_ok());
        let (bits, kind) = result.unwrap();
        assert_eq!(kind, NativeKind::Float64);
        assert!((f64::from_bits(bits) - 99.0).abs() < f64::EPSILON);

        // Second resolve returns cached result (clone of the cached share).
        let cached = sched.resolve_task(1, |_| panic!("should not be called"));
        assert!(cached.is_ok());
    }

    #[test]
    fn test_cancel_task() {
        let mut sched = TaskScheduler::new();
        let (b, k) = function_callable(0);
        sched.register(1, b, k);

        sched.cancel(1);
        assert!(sched.is_resolved(1));

        let result = sched.resolve_task(1, |_| Ok(float_result(0.0)));
        assert!(result.is_err());
    }

    #[test]
    fn test_resolve_all_group() {
        let mut sched = TaskScheduler::new();
        let (b1, k1) = function_callable(0);
        let (b2, k2) = function_callable(1);
        sched.register(1, b1, k1);
        sched.register(2, b2, k2);

        let mut call_count = 0u32;
        let result = sched.resolve_task_group(0, &[1, 2], |_callable| {
            call_count += 1;
            Ok(float_result(call_count as f64))
        });
        assert!(result.is_ok());
        let (_bits, kind) = result.unwrap();
        // All-mode aggregate is a TaskGroup carrier (kinded TaskGroup ptr).
        assert_eq!(kind, NativeKind::Ptr(HeapKind::TaskGroup));
        assert_eq!(call_count, 2);
    }

    #[test]
    fn test_resolve_race_group() {
        let mut sched = TaskScheduler::new();
        let (b1, k1) = function_callable(0);
        let (b2, k2) = function_callable(1);
        sched.register(10, b1, k1);
        sched.register(20, b2, k2);

        let result = sched.resolve_task_group(1, &[10, 20], |_| Ok(float_result(7.0)));
        assert!(result.is_ok());
        let (bits, kind) = result.unwrap();
        assert_eq!(kind, NativeKind::Float64);
        assert!((f64::from_bits(bits) - 7.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_register_external_and_resolve() {
        let mut sched = TaskScheduler::new();
        let tx = sched.register_external(100);
        assert!(sched.has_external(100));
        assert!(matches!(sched.get_result(100), Some(TaskStatus::Pending)));

        // Not yet resolved
        assert!(sched.try_resolve_external(100).is_none());

        // Send result from "background task"
        tx.send(Ok(float_result(42.0))).unwrap();

        // Now resolves
        let result = sched.try_resolve_external(100);
        assert!(result.is_some());
        let (bits, kind) = result.unwrap().unwrap();
        assert_eq!(kind, NativeKind::Float64);
        assert!((f64::from_bits(bits) - 42.0).abs() < f64::EPSILON);

        // Receiver removed after resolution
        assert!(!sched.has_external(100));
    }

    #[test]
    fn test_external_task_error() {
        let mut sched = TaskScheduler::new();
        let tx = sched.register_external(200);

        tx.send(Err("connection refused".to_string())).unwrap();

        let result = sched.try_resolve_external(200);
        assert!(result.is_some());
        assert!(result.unwrap().is_err());
    }

    #[test]
    fn test_external_task_cancelled() {
        let mut sched = TaskScheduler::new();
        let tx = sched.register_external(300);

        // Drop sender to simulate cancellation
        drop(tx);

        let result = sched.try_resolve_external(300);
        assert!(result.is_some());
        assert!(result.unwrap().is_err());
    }

    #[test]
    fn test_take_external_receiver() {
        let mut sched = TaskScheduler::new();
        let _tx = sched.register_external(400);

        assert!(sched.has_external(400));
        let rx = sched.take_external_receiver(400);
        assert!(rx.is_some());
        assert!(!sched.has_external(400));
    }

    #[test]
    fn test_future_snapshot_status_classifies_scheduler_entries() {
        let mut sched = TaskScheduler::new();

        assert_eq!(
            sched.future_snapshot_status(900),
            FutureSnapshotStatus::Unknown
        );

        sched.register(10, 0, NativeKind::Int64);
        assert_eq!(
            sched.future_snapshot_status(10),
            FutureSnapshotStatus::PendingCallable
        );

        let _external_tx = sched.register_external(11);
        assert_eq!(
            sched.future_snapshot_status(11),
            FutureSnapshotStatus::PendingExternal
        );

        let (_tx, rx) = std::sync::mpsc::channel();
        sched.store_pending_async(
            12,
            PendingAsyncTask {
                completion: crate::executor::task_scheduler::AsyncCompletion::Typed(rx),
                abort: None,
            },
        );
        assert_eq!(
            sched.future_snapshot_status(12),
            FutureSnapshotStatus::PendingAsync
        );

        sched.complete(13, 99, NativeKind::Int64);
        assert_eq!(
            sched.future_snapshot_status(13),
            FutureSnapshotStatus::Completed
        );

        sched.register(14, 0, NativeKind::Int64);
        sched.cancel(14);
        assert_eq!(
            sched.future_snapshot_status(14),
            FutureSnapshotStatus::Cancelled
        );
    }
}
