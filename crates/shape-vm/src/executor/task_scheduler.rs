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
//! in the scheduler owns one strong-count for heap-bearing kinds.
//! `take_callable`, `take_external_receiver`, `try_resolve_external`, and
//! `Drop` transfer that share to the caller (or release it). `register`,
//! `complete`, and the cached-result paths in `resolve_task` /
//! `resolve_task_group` use `clone_with_kind` when handing the share to
//! a second consumer.
//!
//! ## One map, one state (#237 / ADR-020 grill R-G5)
//!
//! Tasks live in a single `task_id -> `[`TaskState`] map whose entry carries
//! the driver — callable, external receiver, or in-flight tokio task —
//! *inside* the variant. Status is read off the entry. The four parallel maps
//! this replaced (callables / results / external_receivers / pending_async)
//! made a task's apparent status depend on which map was consulted first, and
//! could not tell "pending, driver checked out" from "no such task": both
//! rendered as "unknown scheduler entry" in the user-facing snapshot barrier.
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
    /// cannot interrupt a closure that has already started: the foreign body
    /// runs to completion and its result is discarded. (If it had not yet begun,
    /// the abort prevents it starting, and no foreign code runs at all.) Either
    /// way it is a discard, never a confirmation that the foreign runtime
    /// stopped (ADR-019 §5 / #202).
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

/// The complete state of one scheduled task — the scheduler's single
/// discriminator for a `task_id`.
///
/// One entry per task, and the entry OWNS whatever drives that task forward:
/// the callable to run, the channel a background thread will answer on, or
/// the in-flight tokio task itself. A task's status is therefore *read* off
/// its entry. It is never reconstructed by consulting several parallel maps
/// in priority order — the shape this type replaced, where a task's apparent
/// status depended on which of four maps happened to answer first, and where
/// "pending, driver checked out" was indistinguishable from "no such task"
/// (#237 / ADR-020 grill R-G5; the #233 "no runtime inference" ruling applies
/// at this site).
///
/// # Driver ownership
///
/// Every pending variant except [`TaskState::PendingUndriven`] holds the
/// task's driver. Handing that driver to a consumer
/// (`take_callable` / `take_external_receiver` / `take_pending_async`) leaves
/// `PendingUndriven` behind: the entry still exists and the task is still
/// pending, but the scheduler no longer holds anything that can advance it.
/// That covers both a task executing right now and a task orphaned by a
/// driver that failed without recording an outcome — from the scheduler's
/// side those are the same fact, and neither is "not started".
///
/// # Share accounting (§2.7.7 retain-on-store)
///
/// [`TaskState::PendingCallable`] and [`TaskState::Completed`] each own
/// exactly one strong-count share of their kinded pair. Replacing an entry
/// releases the share the displaced entry owned; [`TaskScheduler::drop`]
/// releases every share the map still holds.
///
/// # Extending this enum (W17 resume)
///
/// The W17 whole-VM restore path will be the first code to really branch on
/// these states, and the axis it needs is the axis that already separates the
/// variants: whether the state's payload is a plain value a snapshot can
/// persist (`Completed`'s kinded pair, `Cancelled`'s absence of one) or a live
/// host resource that cannot cross a checkpoint (`PendingExternal`'s oneshot
/// receiver, `PendingAsync`'s tokio task, and `PendingUndriven`'s driver,
/// which is off-map entirely and so cannot even be inspected). A new variant
/// lands on one side or the other and inherits that side's contract — a
/// resume-side "completed, value restored from the snapshot" arm sits next to
/// `Completed` and reuses its share-owning rule verbatim. Adding one needs no
/// change to this type's shape: the exhaustive matches in this file, plus the
/// capture barrier in `executor/snapshot.rs`, are the complete list of sites
/// the compiler will make you account for.
pub enum TaskState {
    /// Pending; the scheduler holds the callable that will produce the result.
    /// Owns one share of the pair.
    PendingCallable(Kinded),
    /// Pending; the scheduler holds the receiver a background task will
    /// deliver the result on (remote calls and other externally-completed
    /// futures). The in-transit share lives on the channel, not here.
    PendingExternal(tokio::sync::oneshot::Receiver<Result<Kinded, String>>),
    /// Pending; a real tokio task is in flight on the shared runtime and the
    /// scheduler holds its completion channel and abort handle.
    PendingAsync {
        task: PendingAsyncTask,
        /// Best-effort notification to an external owner that this task was
        /// cancelled (`remote::call_async` sends a receiver-side cancellation
        /// keyed by the internal wire call id). Consumed at most once, by
        /// explicit cancellation or by VM teardown while still pending;
        /// ordinary await/join completion drops it unrun.
        cancel_hook: Option<Box<dyn FnOnce() + Send + 'static>>,
    },
    /// Pending, with the driver checked out of the scheduler: a consumer took
    /// it and is executing it right now, or a consumer took it and failed
    /// without recording an outcome (`resolve_task`'s `executor_fn` returned
    /// `Err`; `try_resolve_external` saw the delivery channel error or close).
    /// Owns nothing.
    PendingUndriven,
    /// Finished with a result. Owns one share of the pair.
    Completed(Kinded),
    /// Cancelled before completion. Owns nothing.
    Cancelled,
}

impl std::fmt::Debug for TaskState {
    /// Variant name only — the payloads are a callable's raw bits, a channel,
    /// and a tokio handle, none of which are meaningful in a diagnostic. Used
    /// by the internal (non-user-facing) half of the capture-barrier message
    /// in `executor/snapshot.rs`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::PendingCallable(_) => "PendingCallable",
            Self::PendingExternal(_) => "PendingExternal",
            Self::PendingAsync { .. } => "PendingAsync",
            Self::PendingUndriven => "PendingUndriven",
            Self::Completed(_) => "Completed",
            Self::Cancelled => "Cancelled",
        })
    }
}

/// Release the share a displaced entry owned (§2.7.7 retain-on-store).
///
/// Replacement only — a displaced `PendingAsync` is dropped exactly as the
/// four-map scheduler dropped it: its cancel hook is discarded unrun and its
/// tokio task is NOT aborted (dropping the receiver closes the completion
/// channel, so the task's send fails silently). Cancellation and teardown are
/// the two paths that abort, and they say so explicitly in `cancel` and
/// `Drop`.
fn release_displaced(state: TaskState) {
    match state {
        TaskState::PendingCallable((bits, kind)) | TaskState::Completed((bits, kind)) => {
            drop_with_kind(bits, kind);
        }
        TaskState::PendingExternal(_)
        | TaskState::PendingAsync { .. }
        | TaskState::PendingUndriven
        | TaskState::Cancelled => {}
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
    /// Every scheduled task, by id. One entry per task, carrying both its
    /// status and (for the pending states) the driver that will advance it —
    /// see [`TaskState`].
    tasks: HashMap<u64, TaskState>,
}

impl TaskScheduler {
    /// Create a new, empty scheduler.
    pub fn new() -> Self {
        Self {
            tasks: HashMap::new(),
        }
    }

    /// Install `state` as the entry for `task_id`, releasing whatever share
    /// the displaced entry owned (§2.7.7 retain-on-store).
    fn set_state(&mut self, task_id: u64, state: TaskState) {
        if let Some(displaced) = self.tasks.insert(task_id, state) {
            release_displaced(displaced);
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
        self.set_state(
            task_id,
            TaskState::PendingAsync {
                task,
                cancel_hook: None,
            },
        );
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
        if let Some(TaskState::PendingAsync { cancel_hook, .. }) = self.tasks.get_mut(&task_id) {
            *cancel_hook = Some(hook);
        }
    }

    /// Whether `task_id` names an in-flight async module-function task.
    pub fn has_pending_async(&self, task_id: u64) -> bool {
        matches!(
            self.tasks.get(&task_id),
            Some(TaskState::PendingAsync { .. })
        )
    }

    /// The scheduler entry for `task_id`, or `None` when no task by that id
    /// has ever been registered.
    ///
    /// The one read accessor: callers match on [`TaskState`] rather than
    /// asking a question per map. `None` is a real answer here — "there is no
    /// such task" is distinct from every pending state, including
    /// [`TaskState::PendingUndriven`].
    pub fn task_state(&self, task_id: u64) -> Option<&TaskState> {
        self.tasks.get(&task_id)
    }

    /// Take (remove) the in-flight async task so the caller can drive its
    /// completion channel. Ownership of the receiver + abort handle transfers
    /// out, leaving the entry [`TaskState::PendingUndriven`].
    ///
    /// The cancel hook is dropped unrun: collecting a task's result is not
    /// cancelling it.
    pub fn take_pending_async(&mut self, task_id: u64) -> Option<PendingAsyncTask> {
        let entry = self.tasks.get_mut(&task_id)?;
        match std::mem::replace(entry, TaskState::PendingUndriven) {
            TaskState::PendingAsync { task, .. } => Some(task),
            // Not an in-flight async task — put the entry back untouched.
            other => {
                *entry = other;
                None
            }
        }
    }

    /// Non-blocking poll of an in-flight async task's completion channel
    /// WITHOUT removing the entry (used by the `race`/`any` first-completion
    /// spin loop). Returns `None` if no such pending-async task exists.
    #[allow(clippy::type_complexity)]
    pub fn peek_pending_async_try_recv(
        &mut self,
        task_id: u64,
    ) -> Option<Result<AsyncTaskOutcome, std::sync::mpsc::TryRecvError>> {
        match self.tasks.get_mut(&task_id) {
            Some(TaskState::PendingAsync { task, .. }) => Some(task.completion.try_recv()),
            _ => None,
        }
    }

    /// Register a callable for a given task_id.
    ///
    /// Called by `op_spawn_task` when a new task is spawned. The caller
    /// transfers one strong-count share for the kinded pair into the
    /// scheduler; on `take_callable` (or `Drop`) the share transfers back
    /// out (or is released). Any prior entry for this id is displaced and its
    /// share released.
    pub fn register(&mut self, task_id: u64, callable_bits: u64, callable_kind: NativeKind) {
        self.set_state(
            task_id,
            TaskState::PendingCallable((callable_bits, callable_kind)),
        );
    }

    /// Take (remove) the callable for `task_id` so it can be executed,
    /// leaving the entry [`TaskState::PendingUndriven`].
    ///
    /// Returns `None` if the task was already consumed, never registered, or
    /// is not driven by a callable. Ownership of the kinded pair transfers to
    /// the caller.
    pub fn take_callable(&mut self, task_id: u64) -> Option<Kinded> {
        let entry = self.tasks.get_mut(&task_id)?;
        match std::mem::replace(entry, TaskState::PendingUndriven) {
            TaskState::PendingCallable(callable) => Some(callable),
            // Not callable-driven — put the entry back untouched.
            other => {
                *entry = other;
                None
            }
        }
    }

    /// Record a completed result for a task.
    ///
    /// The caller transfers one strong-count share into the scheduler. Any
    /// prior entry is displaced and its share released — including the
    /// defensive double-completion case.
    pub fn complete(&mut self, task_id: u64, value_bits: u64, value_kind: NativeKind) {
        self.set_state(task_id, TaskState::Completed((value_bits, value_kind)));
    }

    /// Mark a task as cancelled, releasing or aborting whatever was driving it.
    ///
    /// A task that already has an outcome (completed or cancelled) keeps it.
    pub fn cancel(&mut self, task_id: u64) {
        let Some(entry) = self.tasks.get_mut(&task_id) else {
            return;
        };
        match std::mem::replace(entry, TaskState::Cancelled) {
            // The callable will never run; release its share.
            TaskState::PendingCallable((bits, kind)) => drop_with_kind(bits, kind),
            // Dropping the receiver closes the channel, so the background
            // task's delivery fails instead of resurrecting a cancelled task.
            TaskState::PendingExternal(_) => {}
            // Abort the underlying tokio task so a cancelled `time::sleep`
            // really stops rather than run to completion unobserved (WF-2D
            // real cancellation), and notify any external owner.
            TaskState::PendingAsync { task, cancel_hook } => {
                if let Some(hook) = cancel_hook {
                    hook();
                }
                if let Some(abort) = task.abort {
                    abort.abort();
                }
            }
            // Whoever holds the driver will find the entry cancelled.
            TaskState::PendingUndriven => {}
            // Already resolved — restore the outcome we just displaced.
            resolved @ (TaskState::Completed(_) | TaskState::Cancelled) => *entry = resolved,
        }
    }

    /// Check whether a task has an outcome (completed or cancelled).
    pub fn is_resolved(&self, task_id: u64) -> bool {
        matches!(
            self.tasks.get(&task_id),
            Some(TaskState::Completed(_) | TaskState::Cancelled)
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
        self.set_state(task_id, TaskState::PendingExternal(rx));
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
        if let Some(TaskState::Completed((bits, kind))) = self.tasks.get(&task_id) {
            // Hand out a fresh share — the cached entry retains its own.
            let (bits, kind) = (*bits, *kind);
            clone_with_kind(bits, kind);
            return Some(Ok((bits, kind)));
        }
        let delivery = match self.tasks.get_mut(&task_id) {
            Some(TaskState::PendingExternal(rx)) => rx.try_recv(),
            _ => return None,
        };
        match delivery {
            Ok(Ok((bits, kind))) => {
                // The result share transferred from the background task.
                // Cache one share (clone) and hand out the original.
                clone_with_kind(bits, kind);
                self.set_state(task_id, TaskState::Completed((bits, kind)));
                Some(Ok((bits, kind)))
            }
            // The delivery failed, so the task has no outcome and nothing
            // left to drive it — exactly `PendingUndriven`.
            Ok(Err(e)) => {
                self.set_state(task_id, TaskState::PendingUndriven);
                Some(Err(VMError::RuntimeError(e)))
            }
            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => None,
            Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                self.set_state(task_id, TaskState::PendingUndriven);
                Some(Err(VMError::RuntimeError(
                    "Remote task cancelled".to_string(),
                )))
            }
        }
    }

    /// Check whether a task has an external receiver (is externally-completed).
    pub fn has_external(&self, task_id: u64) -> bool {
        matches!(
            self.tasks.get(&task_id),
            Some(TaskState::PendingExternal(_))
        )
    }

    /// Take the external receiver for async awaiting, leaving the entry
    /// [`TaskState::PendingUndriven`].
    ///
    /// Used by `execute_with_async` when it needs to truly `.await` an external
    /// task's completion.
    pub fn take_external_receiver(
        &mut self,
        task_id: u64,
    ) -> Option<tokio::sync::oneshot::Receiver<Result<Kinded, String>>> {
        let entry = self.tasks.get_mut(&task_id)?;
        match std::mem::replace(entry, TaskState::PendingUndriven) {
            TaskState::PendingExternal(rx) => Some(rx),
            // Not externally driven — put the entry back untouched.
            other => {
                *entry = other;
                None
            }
        }
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
        match self.tasks.get(&task_id) {
            Some(TaskState::Completed((bits, kind))) => {
                let (bits, kind) = (*bits, *kind);
                clone_with_kind(bits, kind);
                return Ok((bits, kind));
            }
            Some(TaskState::Cancelled) => {
                return Err(VMError::RuntimeError(format!(
                    "Task {} was cancelled",
                    task_id
                )));
            }
            _ => {}
        }

        // Take the callable (consume it — share transfers to executor_fn).
        let callable = self.take_callable(task_id).ok_or_else(|| {
            VMError::RuntimeError(format!("No callable registered for task {}", task_id))
        })?;

        // Execute synchronously — share transfers in, share transfers out.
        let (bits, kind) = executor_fn(callable)?;

        // Cache a clone of the result; hand the original share back. (On an
        // `executor_fn` error the `?` above leaves the entry `PendingUndriven`,
        // where `take_callable` put it: pending, orphaned by a failed driver.)
        clone_with_kind(bits, kind);
        self.set_state(task_id, TaskState::Completed((bits, kind)));
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
        for (_, state) in self.tasks.drain() {
            match state {
                TaskState::PendingCallable((bits, kind)) | TaskState::Completed((bits, kind)) => {
                    drop_with_kind(bits, kind);
                }
                // Abort any still-in-flight async task so its background tokio
                // task stops when the VM tears down (WF-2D), and notify any
                // external owner. Dropping the receiver also closes the
                // completion channel; the aborted task's send (if any) fails
                // silently.
                TaskState::PendingAsync { task, cancel_hook } => {
                    if let Some(hook) = cancel_hook {
                        hook();
                    }
                    if let Some(abort) = task.abort {
                        abort.abort();
                    }
                }
                // An external receiver owns no scheduler-side share: the share
                // is in transit on the channel, and the dropping receiver
                // releases it on the sender side. `PendingUndriven` handed its
                // share to whoever took the driver; `Cancelled` owns nothing.
                TaskState::PendingExternal(_)
                | TaskState::PendingUndriven
                | TaskState::Cancelled => {}
            }
        }
    }
}

impl std::fmt::Debug for TaskScheduler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut counts: [usize; 6] = [0; 6];
        for state in self.tasks.values() {
            counts[match state {
                TaskState::PendingCallable(_) => 0,
                TaskState::PendingExternal(_) => 1,
                TaskState::PendingAsync { .. } => 2,
                TaskState::PendingUndriven => 3,
                TaskState::Completed(_) => 4,
                TaskState::Cancelled => 5,
            }] += 1;
        }
        f.debug_struct("TaskScheduler")
            .field("tasks", &self.tasks.len())
            .field("pending_callable", &counts[0])
            .field("pending_external", &counts[1])
            .field("pending_async", &counts[2])
            .field("pending_undriven", &counts[3])
            .field("completed", &counts[4])
            .field("cancelled", &counts[5])
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

    /// Helper: a refcounted heap value whose strong count is directly
    /// observable through its `HeapHeader` (same fixture shape as
    /// `vm_impl/stack.rs`'s GC barrier tests).
    ///
    /// The `function_callable` fixture above is deliberately share-free
    /// (`HeapKind::Future` is an inline scalar — `clone_with_kind` /
    /// `drop_with_kind` are no-ops for it), so it cannot witness a
    /// retain/release imbalance. Every test that asserts share accounting
    /// uses this probe instead. Returned with strong count 1, owned by the
    /// caller.
    fn heap_probe() -> Kinded {
        let field_kinds: Arc<[NativeKind]> = Arc::from(vec![NativeKind::Int64]);
        let obj = shape_value::heap_value::TypedObjectStorage::_new(
            7,
            vec![shape_value::slot::ValueSlot::from_int(0)].into_boxed_slice(),
            0,
            field_kinds,
        );
        (obj as u64, NativeKind::Ptr(HeapKind::TypedObject))
    }

    /// Read a [`heap_probe`]'s strong count. Caller must hold at least one
    /// live share.
    fn probe_rc(bits: u64) -> u32 {
        // SAFETY: `bits` came from `heap_probe` and the caller still owns a
        // share, so the allocation is live.
        unsafe {
            (*(bits as *const shape_value::heap_value::TypedObjectStorage))
                .header
                .refcount
                .load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    // ---------------------------------------------------------------
    // Share-accounting pins (#237).
    //
    // These fix the scheduler's §2.7.7 retain-on-store balance BEFORE the
    // four-map → one-`TaskState`-map restructure, so the restructure has to
    // reproduce it exactly rather than argue that it did. Each test hands
    // the scheduler exactly one share of a live probe, keeps one for itself,
    // and asserts the count the scheduler is responsible for.
    // ---------------------------------------------------------------

    #[test]
    fn drop_releases_a_registered_callable_share() {
        let (bits, kind) = heap_probe(); // rc 1: ours
        clone_with_kind(bits, kind); // rc 2: the scheduler's
        let mut sched = TaskScheduler::new();
        sched.register(1, bits, kind);
        assert_eq!(probe_rc(bits), 2);

        drop(sched);
        assert_eq!(
            probe_rc(bits),
            1,
            "scheduler Drop must release the registered callable's share"
        );
        drop_with_kind(bits, kind);
    }

    #[test]
    fn drop_releases_a_completed_result_share() {
        let (bits, kind) = heap_probe();
        clone_with_kind(bits, kind);
        let mut sched = TaskScheduler::new();
        sched.complete(1, bits, kind);
        assert_eq!(probe_rc(bits), 2);

        drop(sched);
        assert_eq!(
            probe_rc(bits),
            1,
            "scheduler Drop must release the cached result's share"
        );
        drop_with_kind(bits, kind);
    }

    #[test]
    fn take_callable_transfers_the_share_out_and_drop_does_not_double_release() {
        let (bits, kind) = heap_probe();
        clone_with_kind(bits, kind);
        let mut sched = TaskScheduler::new();
        sched.register(1, bits, kind);

        let taken = sched.take_callable(1).expect("callable registered");
        assert_eq!(taken, (bits, kind));
        assert_eq!(
            probe_rc(bits),
            2,
            "take_callable transfers, it does not clone"
        );

        drop(sched);
        assert_eq!(
            probe_rc(bits),
            2,
            "the taken share belongs to the caller; Drop must not release it again"
        );
        drop_with_kind(bits, kind); // the caller retires the taken share
        assert_eq!(probe_rc(bits), 1);
        drop_with_kind(bits, kind);
    }

    #[test]
    fn cancel_releases_the_pending_callable_share() {
        let (bits, kind) = heap_probe();
        clone_with_kind(bits, kind);
        let mut sched = TaskScheduler::new();
        sched.register(1, bits, kind);

        sched.cancel(1);
        assert_eq!(
            probe_rc(bits),
            1,
            "cancel must release the callable it will never run"
        );
        drop(sched);
        assert_eq!(probe_rc(bits), 1, "and Drop must not release it twice");
        drop_with_kind(bits, kind);
    }

    #[test]
    fn re_registering_a_callable_releases_the_displaced_share() {
        let (first, first_kind) = heap_probe();
        clone_with_kind(first, first_kind);
        let (second, second_kind) = heap_probe();
        clone_with_kind(second, second_kind);

        let mut sched = TaskScheduler::new();
        sched.register(1, first, first_kind);
        sched.register(1, second, second_kind);
        assert_eq!(
            probe_rc(first),
            1,
            "the displaced callable's share must be released, not leaked"
        );
        assert_eq!(probe_rc(second), 2);

        drop(sched);
        assert_eq!(probe_rc(second), 1);
        drop_with_kind(first, first_kind);
        drop_with_kind(second, second_kind);
    }

    #[test]
    fn completing_twice_releases_the_displaced_result_share() {
        let (first, first_kind) = heap_probe();
        clone_with_kind(first, first_kind);
        let (second, second_kind) = heap_probe();
        clone_with_kind(second, second_kind);

        let mut sched = TaskScheduler::new();
        sched.complete(1, first, first_kind);
        sched.complete(1, second, second_kind);
        assert_eq!(
            probe_rc(first),
            1,
            "the displaced result's share must be released, not leaked"
        );
        assert_eq!(probe_rc(second), 2);

        drop(sched);
        assert_eq!(probe_rc(second), 1);
        drop_with_kind(first, first_kind);
        drop_with_kind(second, second_kind);
    }

    #[test]
    fn resolve_task_caches_one_share_and_hands_one_out() {
        let (callable, callable_kind) = heap_probe(); // rc 1: ours
        clone_with_kind(callable, callable_kind); // rc 2: the scheduler's
        let (result, result_kind) = heap_probe(); // rc 1: ours
        clone_with_kind(result, result_kind); // rc 2: the one the executor yields

        let mut sched = TaskScheduler::new();
        sched.register(1, callable, callable_kind);

        // The executor consumes the callable share it is handed (as a real
        // callee frame does at `op_return`) and yields the result share.
        let produced = sched
            .resolve_task(1, |(bits, kind)| {
                drop_with_kind(bits, kind);
                Ok((result, result_kind))
            })
            .expect("resolve");
        assert_eq!(produced, (result, result_kind));
        assert_eq!(
            probe_rc(callable),
            1,
            "the callable share transferred to the executor and was retired there"
        );
        assert_eq!(
            probe_rc(result),
            3,
            "the executor's share is cached and a fresh one is returned"
        );

        drop_with_kind(result, result_kind); // caller retires the returned share
        drop(sched);
        assert_eq!(
            probe_rc(result),
            1,
            "scheduler Drop retires the cached result share"
        );
        drop_with_kind(result, result_kind);
        drop_with_kind(callable, callable_kind);
    }

    #[test]
    fn a_cached_resolve_hands_out_a_fresh_share_each_time() {
        let (result, result_kind) = heap_probe();
        clone_with_kind(result, result_kind);
        let mut sched = TaskScheduler::new();
        sched.complete(1, result, result_kind); // rc 2: ours + cached

        let a = sched.resolve_task(1, |_| panic!("cached")).expect("cached");
        assert_eq!(probe_rc(result), 3);
        let b = sched.resolve_task(1, |_| panic!("cached")).expect("cached");
        assert_eq!(probe_rc(result), 4);
        assert_eq!(a, b);

        drop_with_kind(result, result_kind);
        drop_with_kind(result, result_kind);
        drop(sched);
        assert_eq!(probe_rc(result), 1, "only the test's own share survives");
        drop_with_kind(result, result_kind);
    }

    #[test]
    fn try_resolve_external_balances_the_delivered_share() {
        let (result, result_kind) = heap_probe();
        clone_with_kind(result, result_kind); // rc 2: ours + the one we send

        let mut sched = TaskScheduler::new();
        let tx = sched.register_external(1);
        tx.send(Ok((result, result_kind)))
            .expect("receiver is registered");

        let delivered = sched
            .try_resolve_external(1)
            .expect("ready")
            .expect("delivered ok");
        assert_eq!(delivered, (result, result_kind));
        assert_eq!(
            probe_rc(result),
            3,
            "the delivered share is handed out and a second is cached"
        );

        drop_with_kind(result, result_kind); // caller retires the delivered share
        drop(sched);
        assert_eq!(probe_rc(result), 1, "only the test's own share survives");
        drop_with_kind(result, result_kind);
    }

    #[test]
    fn test_register_and_take_callable() {
        let mut sched = TaskScheduler::new();
        let (bits, kind) = function_callable(42);
        sched.register(1, bits, kind);
        assert!(matches!(
            sched.task_state(1),
            Some(TaskState::PendingCallable(_))
        ));

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
        assert!(matches!(
            sched.task_state(100),
            Some(TaskState::PendingExternal(_))
        ));

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

    /// Helper: an in-flight-async entry that never delivers. Nothing in these
    /// tests polls the channel — they exercise classification and ownership,
    /// not collection.
    fn stalled_pending_async() -> PendingAsyncTask {
        let (_tx, rx) = std::sync::mpsc::channel();
        PendingAsyncTask {
            completion: AsyncCompletion::Typed(rx),
            abort: None,
        }
    }

    #[test]
    fn task_state_classifies_every_scheduler_entry() {
        let mut sched = TaskScheduler::new();

        // No entry is its own answer, distinct from every state below.
        assert!(sched.task_state(900).is_none());

        sched.register(10, 0, NativeKind::Int64);
        assert!(matches!(
            sched.task_state(10),
            Some(TaskState::PendingCallable(_))
        ));

        let _external_tx = sched.register_external(11);
        assert!(matches!(
            sched.task_state(11),
            Some(TaskState::PendingExternal(_))
        ));

        sched.store_pending_async(12, stalled_pending_async());
        assert!(matches!(
            sched.task_state(12),
            Some(TaskState::PendingAsync { .. })
        ));

        sched.complete(13, 99, NativeKind::Int64);
        assert!(matches!(
            sched.task_state(13),
            Some(TaskState::Completed(_))
        ));

        sched.register(14, 0, NativeKind::Int64);
        sched.cancel(14);
        assert!(matches!(sched.task_state(14), Some(TaskState::Cancelled)));
    }

    /// The state #237 was really about. The four-map scheduler reported this
    /// as `FutureSnapshotStatus::Unknown` — "no scheduler entry exists" — for
    /// a task that exists, is pending, and is executing right now. Both the
    /// issue body's "NotStarted" and the downgrade comment's "Registered"
    /// named it wrong: the driver is gone precisely because it is *running*.
    #[test]
    fn taking_a_driver_leaves_the_entry_pending_and_undriven() {
        let mut sched = TaskScheduler::new();

        sched.register(1, 0, NativeKind::Int64);
        let _callable = sched.take_callable(1).expect("registered");
        assert!(matches!(
            sched.task_state(1),
            Some(TaskState::PendingUndriven)
        ));
        assert!(!sched.is_resolved(1), "taking the driver resolves nothing");

        let _tx = sched.register_external(2);
        let _rx = sched.take_external_receiver(2).expect("registered");
        assert!(matches!(
            sched.task_state(2),
            Some(TaskState::PendingUndriven)
        ));

        sched.store_pending_async(3, stalled_pending_async());
        let _task = sched.take_pending_async(3).expect("registered");
        assert!(matches!(
            sched.task_state(3),
            Some(TaskState::PendingUndriven)
        ));

        // And none of them collapsed into "no such task".
        assert!(sched.task_state(1).is_some());
        assert!(sched.task_state(2).is_some());
        assert!(sched.task_state(3).is_some());
        assert!(sched.task_state(900).is_none());
    }

    /// The other half of the same state: a driver that was taken and then
    /// failed without recording an outcome orphans the entry. Same fact from
    /// the scheduler's side — pending, nothing here can advance it.
    #[test]
    fn a_failed_driver_orphans_the_entry_as_undriven() {
        let mut sched = TaskScheduler::new();
        sched.register(1, 0, NativeKind::Int64);

        let err = sched.resolve_task(1, |_| {
            Err(VMError::RuntimeError("body blew up".to_string()))
        });
        assert!(err.is_err());
        assert!(matches!(
            sched.task_state(1),
            Some(TaskState::PendingUndriven)
        ));
        assert!(!sched.is_resolved(1));

        // An external delivery that errors lands in the same state.
        let tx = sched.register_external(2);
        tx.send(Err("connection refused".to_string())).unwrap();
        assert!(sched.try_resolve_external(2).expect("ready").is_err());
        assert!(matches!(
            sched.task_state(2),
            Some(TaskState::PendingUndriven)
        ));
    }

    /// Cancelling an external task takes its receiver out of the scheduler, so
    /// a late delivery cannot resurrect it. The four-map scheduler left the
    /// receiver in `external_receivers` and only flipped `results` to
    /// `Cancelled`, so `future_snapshot_status` still answered
    /// `PendingExternal` (the external map was consulted first) and a later
    /// `try_resolve_external` overwrote `Cancelled` with `Completed`.
    #[test]
    fn cancelling_an_external_task_drops_its_receiver() {
        let mut sched = TaskScheduler::new();
        let tx = sched.register_external(1);

        sched.cancel(1);
        assert!(matches!(sched.task_state(1), Some(TaskState::Cancelled)));
        assert!(!sched.has_external(1));

        // The background task's delivery now fails instead of landing.
        assert!(tx.send(Ok(float_result(1.0))).is_err());
        assert!(sched.try_resolve_external(1).is_none());
        assert!(matches!(sched.task_state(1), Some(TaskState::Cancelled)));
    }

    // The two share-accounting cases the four parallel maps could not cover:
    // a displaced entry of a DIFFERENT kind than the one being installed. The
    // old `register` dropped a displaced callable but discarded the return of
    // `results.insert(Pending)`, so a displaced `Completed` share leaked; the
    // old `complete` was the mirror image. One map, one release path, no gap.

    #[test]
    fn registering_over_a_completed_task_releases_the_result_share() {
        let (result, result_kind) = heap_probe();
        clone_with_kind(result, result_kind);
        let mut sched = TaskScheduler::new();
        sched.complete(1, result, result_kind);
        assert_eq!(probe_rc(result), 2);

        sched.register(1, 0, NativeKind::Int64);
        assert_eq!(
            probe_rc(result),
            1,
            "the displaced result's share must be released, not leaked"
        );
        drop(sched);
        drop_with_kind(result, result_kind);
    }

    #[test]
    fn completing_a_task_that_still_holds_a_callable_releases_it() {
        let (callable, callable_kind) = heap_probe();
        clone_with_kind(callable, callable_kind);
        let mut sched = TaskScheduler::new();
        sched.register(1, callable, callable_kind);
        assert_eq!(probe_rc(callable), 2);

        sched.complete(1, 99, NativeKind::Int64);
        assert_eq!(
            probe_rc(callable),
            1,
            "the displaced callable's share must be released, not leaked"
        );
        drop(sched);
        drop_with_kind(callable, callable_kind);
    }
}
