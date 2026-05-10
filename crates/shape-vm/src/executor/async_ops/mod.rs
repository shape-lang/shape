//! Async operations for the VM executor.
//!
//! # Concurrency Model
//!
//! The Shape VM uses **cooperative, single-threaded concurrency**. All async
//! operations execute on the thread that owns the `VirtualMachine` instance --
//! there is no work-stealing or multi-threaded task execution within the VM
//! itself. The VM is `!Sync` by design.
//!
//! ## Task Lifecycle
//!
//! 1. **Spawn** (`SpawnTask`): Pops a callable from the stack, assigns a
//!    monotonic future ID, registers it with the `TaskScheduler`, and pushes
//!    a `Future(id)` value onto the stack.
//! 2. **Await** (`Await`): Pops a `Future(id)`, attempts synchronous inline
//!    resolution via the `TaskScheduler`. If the task cannot be resolved
//!    (e.g., it depends on an external I/O operation), execution suspends
//!    with `VMError::Suspended` so the host runtime can schedule it.
//! 3. **Join** (`JoinInit` + `JoinAwait`): Collects multiple futures into a
//!    `TaskGroup` value, then resolves them according to a join strategy
//!    (all, race, any, all-settled).
//! 4. **Cancel** (`CancelTask`): Marks a task as cancelled in the scheduler.
//!
//! ## Structured Concurrency
//!
//! `AsyncScopeEnter` / `AsyncScopeExit` bracket a structured concurrency
//! region. All tasks spawned within a scope are tracked; on scope exit, any
//! still-pending tasks are cancelled in LIFO order. This guarantees that no
//! task outlives its enclosing scope.
//!
//! ## Suspension Protocol
//!
//! When an operation cannot complete synchronously, it returns
//! `AsyncExecutionResult::Suspended(SuspensionInfo)`. The dispatch layer in
//! `dispatch.rs` converts this into `VMError::Suspended { future_id, resume_ip }`
//! which propagates up to the host runtime. The host resolves the future and
//! calls back into the VM to resume execution at `resume_ip`.
//!
//! ## Opcodes Handled
//!
//! `Yield`, `Suspend`, `Resume`, `Poll`, `AwaitBar`, `AwaitTick`,
//! `EmitAlert`, `EmitEvent`, `Await`, `SpawnTask`, `JoinInit`, `JoinAwait`,
//! `CancelTask`, `AsyncScopeEnter`, `AsyncScopeExit`.
//!
//! ## Wave 6.5 / E-async migration (ADR-006 §2.7.7 / Q9, §10 E-async row)
//!
//! Every push/pop in this file threads the kinded API
//! (`push_kinded(bits, kind)` / `pop_kinded()`) per the playbook §2 / §3
//! kind-sourcing rules. Future and TaskGroup payload kinds:
//!
//! - `Future(id)` ⇒ `NativeKind::Ptr(HeapKind::Future)` — inline scalar
//!   payload (the future ID is stored directly in `bits`; no `Arc<T>`).
//! - `TaskGroup(Arc<TaskGroupData>)` ⇒ `NativeKind::Ptr(HeapKind::TaskGroup)`
//!   — `Arc<TaskGroupData>` payload per ADR-006 §2.3.
//!
//! ## Wave 8 W8-AS migration (ADR-006 §2.7.11/Q12, §2.7.4 Phase-2c boundary)
//!
//! The `task_scheduler::TaskScheduler` API was migrated to the kinded
//! `(bits, kind)` carrier shape during Wave 6.5 R-async-time / E-async
//! close, and `call_convention.rs::resolve_spawned_task` was filled by
//! W7-cv-async (close `f3502b0`) per §2.7.11/Q12 — sync resolution of
//! spawned closures + function-id callables routes through
//! `call_closure_with_nb_args_keepalive` / `call_function_with_nb_args`.
//! W8-AS lights up the per-await-site integration:
//!
//! - `op_await` resolves a `Future(id)`-kinded slot synchronously via
//!   `vm.resolve_spawned_task(task_id)` and pushes the kinded result.
//! - `op_spawn_task` allocates a fresh `future_id`, transfers the popped
//!   callable share into `task_scheduler.register(id, bits, kind)`,
//!   tracks the future id in the active async-scope stack, and pushes
//!   the future id as `Ptr(HeapKind::Future)`.
//! - `op_join_await` walks the carried `task_ids` and dispatches per-id
//!   to `resolve_spawned_task` per the join strategy (All / Race / Any
//!   / AllSettled), aggregating into an `Arc<TaskGroupData>` `Ptr(HeapKind::TaskGroup)`
//!   carrier (Race/Any return the per-task result directly).
//!
//! ### §2.7.4 Phase-2c boundary
//!
//! Suspension state crossing a `resolve_spawned_task` frame boundary —
//! a `VMError::Suspended` raised inside a spawned closure body — stays
//! out of scope. The current sync-resolution path propagates the error
//! upward; the task's cached entry remains `Pending` until a future
//! Phase-2c rebuild lands the snapshot-tier resumption.

use crate::{
    bytecode::{Instruction, OpCode, Operand},
    executor::VirtualMachine,
    executor::vm_impl::stack::drop_with_kind,
};
use shape_value::{
    NativeKind, VMError,
    heap_value::{HeapKind, TaskGroupData},
};
use std::sync::Arc;

/// Result of executing an async operation
#[derive(Debug, Clone)]
pub enum AsyncExecutionResult {
    /// Continue normal execution
    Continue,
    /// Yield to event loop (cooperative scheduling)
    Yielded,
    /// Suspended waiting for external event
    Suspended(SuspensionInfo),
}

/// Information about why execution was suspended
#[derive(Debug, Clone)]
pub struct SuspensionInfo {
    /// What we're waiting for
    pub wait_type: WaitType,
    /// Instruction pointer to resume at
    pub resume_ip: usize,
}

/// Type of wait condition
#[derive(Debug, Clone)]
pub enum WaitType {
    /// Waiting for next data bar from source
    NextBar { source: String },
    /// Waiting for timer
    Timer { id: u64 },
    /// Waiting for any event
    AnyEvent,
    /// Waiting for a future to resolve (general-purpose await)
    Future { id: u64 },
    /// Waiting for a task group to resolve (join await)
    TaskGroup { kind: u8, task_ids: Vec<u64> },
}

impl VirtualMachine {
    /// Execute an async opcode
    ///
    /// Returns `AsyncExecutionResult` to indicate whether execution should
    /// continue, yield, or suspend.
    #[inline(always)]
    pub(in crate::executor) fn exec_async_op(
        &mut self,
        instruction: &Instruction,
    ) -> Result<AsyncExecutionResult, VMError> {
        use OpCode::*;
        match instruction.opcode {
            Yield => self.op_yield(),
            Suspend => self.op_suspend(instruction),
            Resume => self.op_resume(instruction),
            Poll => self.op_poll(),
            AwaitBar => self.op_await_bar(instruction),
            AwaitTick => self.op_await_tick(instruction),
            EmitAlert => self.op_emit_alert(),
            EmitEvent => self.op_emit_event(),
            Await => self.op_await(),
            SpawnTask => self.op_spawn_task(),
            JoinInit => self.op_join_init(instruction),
            JoinAwait => self.op_join_await(),
            CancelTask => self.op_cancel_task(),
            AsyncScopeEnter => self.op_async_scope_enter(),
            AsyncScopeExit => self.op_async_scope_exit(),
            _ => unreachable!(
                "exec_async_op called with non-async opcode: {:?}",
                instruction.opcode
            ),
        }
    }

    /// Yield to the event loop for cooperative scheduling
    ///
    /// This allows other tasks to run and prevents long-running
    /// computations from blocking the event loop.
    fn op_yield(&mut self) -> Result<AsyncExecutionResult, VMError> {
        // Save current state - the IP is already pointing to next instruction
        Ok(AsyncExecutionResult::Yielded)
    }

    /// Suspend execution until a condition is met
    ///
    /// The operand specifies the wait condition type.
    fn op_suspend(&mut self, instruction: &Instruction) -> Result<AsyncExecutionResult, VMError> {
        let wait_type = match &instruction.operand {
            Some(Operand::Const(idx)) => {
                // Get wait type from constant pool
                // For now, default to waiting for any event
                let _ = idx;
                WaitType::AnyEvent
            }
            _ => WaitType::AnyEvent,
        };

        Ok(AsyncExecutionResult::Suspended(SuspensionInfo {
            wait_type,
            resume_ip: self.ip,
        }))
    }

    /// Resume from suspension
    ///
    /// Called by the runtime when resuming suspended execution.
    /// The resume value (if any) should be on the stack.
    fn op_resume(&mut self, _instruction: &Instruction) -> Result<AsyncExecutionResult, VMError> {
        // Resume is handled by the outer execution loop
        // This opcode is a marker for where to resume
        Ok(AsyncExecutionResult::Continue)
    }

    /// Poll the event queue
    ///
    /// Pushes the next event from the queue onto the stack,
    /// or null if the queue is empty.
    fn op_poll(&mut self) -> Result<AsyncExecutionResult, VMError> {
        // In the VM, we don't have direct access to the event queue
        // This is handled via the VMContext passed from the runtime.
        // No event available — push the §2.7 null sentinel (zero bits, Bool kind).
        self.push_kinded(0u64, NativeKind::Bool)?;
        Ok(AsyncExecutionResult::Continue)
    }

    /// Await next data bar from a source
    ///
    /// Suspends execution until the next data point arrives
    /// from the specified source.
    fn op_await_bar(&mut self, instruction: &Instruction) -> Result<AsyncExecutionResult, VMError> {
        let source = match &instruction.operand {
            Some(Operand::Const(idx)) => {
                // Get source name from constant pool
                match self.program.constants.get(*idx as usize) {
                    Some(crate::bytecode::Constant::String(s)) => s.clone(),
                    _ => "default".to_string(),
                }
            }
            _ => "default".to_string(),
        };

        Ok(AsyncExecutionResult::Suspended(SuspensionInfo {
            wait_type: WaitType::NextBar { source },
            resume_ip: self.ip,
        }))
    }

    /// Await next timer tick
    ///
    /// Suspends execution until the specified timer fires.
    fn op_await_tick(
        &mut self,
        instruction: &Instruction,
    ) -> Result<AsyncExecutionResult, VMError> {
        let timer_id = match &instruction.operand {
            Some(Operand::Const(idx)) => {
                // Get timer ID from constant pool
                match self.program.constants.get(*idx as usize) {
                    Some(crate::bytecode::Constant::Number(n)) => *n as u64,
                    _ => 0,
                }
            }
            _ => 0,
        };

        Ok(AsyncExecutionResult::Suspended(SuspensionInfo {
            wait_type: WaitType::Timer { id: timer_id },
            resume_ip: self.ip,
        }))
    }

    /// Emit an alert to the alert pipeline
    ///
    /// Pops an alert object from the stack and sends it to
    /// the alert router for processing.
    fn op_emit_alert(&mut self) -> Result<AsyncExecutionResult, VMError> {
        // Pop the alert payload and release its share — alert pipeline
        // integration is deferred. Drop discipline (playbook §3): every
        // `pop_kinded` either re-pushes or `drop_with_kind`s.
        let (bits, kind) = self.pop_kinded()?;
        drop_with_kind(bits, kind);
        Ok(AsyncExecutionResult::Continue)
    }

    /// General-purpose await
    ///
    /// Pops a value from the stack. If it's a Future(id), attempts to resolve
    /// the task inline from the task scheduler. If the task's callable is a
    /// plain value (not a closure/function), it is used directly as the result.
    /// Otherwise, suspends execution so the host runtime can schedule the task.
    /// If the value is not a Future, pushes it back (sync shortcut).
    fn op_await(&mut self) -> Result<AsyncExecutionResult, VMError> {
        let sp_before = self.sp;
        let (bits, kind) = self.pop_kinded()?;
        match kind {
            NativeKind::Ptr(HeapKind::Future) => {
                // Future(id) is an inline scalar — `bits` IS the future ID
                // (see TaskScheduler docstring + §2.7.11/Q12 Future row).
                // No Arc share to drop on the popped slot; HeapKind::Future
                // is a no-op in drop_with_kind / clone_with_kind.
                //
                // Sync-resolution path (§2.7.11/Q12 dispatch precedent,
                // closed by W7-cv-async at `f3502b0`): hand the future id
                // to `resolve_spawned_task`, which routes through the
                // kinded `call_*_with_nb_args` family for closure /
                // function-id callables and returns the result `KindedSlot`.
                //
                // Phase-2c boundary (ADR-006 §2.7.4): if the spawned
                // body suspends mid-execution, `resolve_spawned_task`
                // propagates `VMError::Suspended` up. The task's cached
                // entry remains `Pending` until a future snapshot-tier
                // rebuild lands. This is explicitly out-of-scope here.
                let task_id = bits;
                let result = self.resolve_spawned_task(task_id)?;

                // Transfer the result share onto the stack via the
                // canonical `push_kinded(raw, kind)` + `mem::forget`
                // pattern (per §2.7.10/§2.7.11 dispatch-shell shape —
                // `control_flow/mod.rs::dispatch_call_value_immediate`
                // is the precedent). The carrier's Drop must not fire
                // after the share moves to the stack slot.
                self.push_kinded(result.raw(), result.kind())?;
                std::mem::forget(result);
                debug_assert_eq!(
                    self.sp, sp_before,
                    "op_await (Future): stack depth changed (before={}, after={})",
                    sp_before, self.sp
                );
                Ok(AsyncExecutionResult::Continue)
            }
            _ => {
                // Sync shortcut: value is already resolved, push it back.
                // The popped share transfers directly back onto the stack —
                // no `clone_with_kind` / `drop_with_kind` needed.
                self.push_kinded(bits, kind)?;
                debug_assert_eq!(
                    self.sp, sp_before,
                    "op_await (sync shortcut): stack depth changed (before={}, after={})",
                    sp_before, self.sp
                );
                Ok(AsyncExecutionResult::Continue)
            }
        }
    }

    /// Await with a timeout.
    ///
    /// Spawn a task from a closure/function on the stack
    ///
    /// Pops a closure or function reference from the stack and creates a new async task.
    /// Pushes a Future(task_id) onto the stack representing the spawned task.
    /// The host runtime is responsible for actually scheduling the task.
    ///
    /// If inside an async scope, the spawned future ID is tracked for cancellation.
    fn op_spawn_task(&mut self) -> Result<AsyncExecutionResult, VMError> {
        let sp_before = self.sp;
        // Pop the callable's kinded slot. The share transfers to the
        // task_scheduler via `register(id, bits, kind)` — same retain-on-store
        // contract as the §2.7.7 stack and §2.7.8 cell-storage tracks (one
        // strong-count share owned by the storage; released by `take_callable`
        // / `cancel` / `Drop`). No `drop_with_kind` here: the share is moved,
        // not released.
        let (callable_bits, callable_kind) = self.pop_kinded()?;

        // Allocate a fresh future id. `next_future_id` is monotonic and
        // single-threaded (the VM is `!Sync` per the module docstring's
        // concurrency model section).
        let task_id = self.next_future_id();

        // Transfer the share into the scheduler. `register` honours the
        // Wave-6.5 R-async-time kinded API (bits + NativeKind pair).
        self.task_scheduler
            .register(task_id, callable_bits, callable_kind);

        // Track the spawned future id in the active async scope (if any)
        // so `op_async_scope_exit` can cancel still-pending tasks in
        // LIFO order (structured concurrency contract — see module
        // docstring's "Structured Concurrency" section).
        if let Some(scope) = self.async_scope_stack.last_mut() {
            scope.push(task_id);
        }

        // Push the future id as `Ptr(HeapKind::Future)`. The Future kind
        // is an inline-scalar payload — `bits` IS the future id, no Arc
        // backing. Drop is a no-op in `drop_with_kind`.
        self.push_kinded(task_id, NativeKind::Ptr(HeapKind::Future))?;
        debug_assert_eq!(
            self.sp, sp_before,
            "op_spawn_task: stack depth changed (before={}, after={})",
            sp_before, self.sp
        );
        Ok(AsyncExecutionResult::Continue)
    }

    /// Initialize a join group from futures on the stack
    ///
    /// Operand: Count(packed_u16) where high 2 bits = join kind, low 14 bits = arity.
    /// Pops `arity` Future values from the stack (in reverse order).
    /// Pushes a `Ptr(HeapKind::TaskGroup)`-kinded `Arc<TaskGroupData>` payload.
    fn op_join_init(&mut self, instruction: &Instruction) -> Result<AsyncExecutionResult, VMError> {
        let packed = match &instruction.operand {
            Some(Operand::Count(n)) => *n,
            _ => {
                return Err(VMError::RuntimeError(
                    "JoinInit requires Count operand".to_string(),
                ));
            }
        };

        let kind = ((packed >> 14) & 0x03) as u8;
        let arity = (packed & 0x3FFF) as usize;

        if self.sp < arity {
            return Err(VMError::StackUnderflow);
        }

        let mut task_ids: Vec<u64> = Vec::with_capacity(arity);
        for _ in 0..arity {
            let (bits, slot_kind) = self.pop_kinded()?;
            match slot_kind {
                NativeKind::Ptr(HeapKind::Future) => {
                    // Future is an inline scalar — bits IS the id. No share
                    // to drop (HeapKind::Future is a no-op in drop_with_kind).
                    task_ids.push(bits);
                }
                _ => {
                    // Type mismatch — drop the popped share before surfacing
                    // the error so refcount discipline holds (playbook §3).
                    drop_with_kind(bits, slot_kind);
                    return Err(VMError::RuntimeError(format!(
                        "JoinInit expected Future, got {:?}",
                        slot_kind
                    )));
                }
            }
        }
        // Reverse so task_ids[0] corresponds to first branch
        task_ids.reverse();

        // Construct an Arc<TaskGroupData> and push as Ptr(HeapKind::TaskGroup).
        // ADR-006 §2.3 / playbook §3 per-HeapKind push pattern: heap-bearing
        // kinds push the `Arc::into_raw` pointer with the matching kind.
        let arc: Arc<TaskGroupData> = Arc::new(TaskGroupData { kind, task_ids });
        let bits = Arc::into_raw(arc) as u64;
        self.push_kinded(bits, NativeKind::Ptr(HeapKind::TaskGroup))?;
        Ok(AsyncExecutionResult::Continue)
    }

    /// Await a task group, resolving tasks inline
    ///
    /// Pops a `Ptr(HeapKind::TaskGroup)`-kinded slot from the stack.
    /// Resolves all tasks inline using the task scheduler's `resolve_task_group`,
    /// which executes each task's callable synchronously (same strategy as `op_await`).
    /// Pushes the result value onto the stack according to the join strategy.
    fn op_join_await(&mut self) -> Result<AsyncExecutionResult, VMError> {
        let sp_before = self.sp;
        let (bits, slot_kind) = self.pop_kinded()?;
        match slot_kind {
            NativeKind::Ptr(HeapKind::TaskGroup) => {
                // Reclaim the `Arc<TaskGroupData>` share that `pop_kinded`
                // transferred to us. Extract `kind` + `task_ids` for the
                // join walk, then drop the Arc.
                //
                // SAFETY: the construction-side contract for
                // `push_kinded(bits, Ptr(HeapKind::TaskGroup))` (see
                // `op_join_init` above + ADR-006 §2.3) guarantees `bits`
                // is the result of `Arc::into_raw::<TaskGroupData>` and
                // we own exactly one strong-count share.
                let arc: Arc<TaskGroupData> =
                    unsafe { Arc::from_raw(bits as *const TaskGroupData) };
                let join_kind = arc.kind;
                let task_ids = arc.task_ids.clone();
                drop(arc);

                // Per-id sync resolution via the §2.7.11/Q12 dispatch
                // entry-point. `resolve_spawned_task` consults the
                // scheduler's cached-result fast-path first, then takes
                // the callable share and routes through
                // `call_*_with_nb_args` family. The borrow shape here
                // mirrors `resolve_spawned_task`'s own `take_callable` /
                // `complete` cycle — no per-call closure capture of
                // `&mut self.task_scheduler` is needed because the
                // scheduler is consulted/mutated point-wise inside
                // `resolve_spawned_task` itself.
                //
                // Phase-2c boundary (ADR-006 §2.7.4): if a constituent
                // task's body suspends (`VMError::Suspended`), the join
                // surfaces the error upward. Snapshot-tier resumption
                // of in-flight join groups stays out of scope per
                // §2.7.11 out-of-scope clause.
                match join_kind {
                    // All: resolve every task, drop each per-task share
                    // (the aggregate carrier is a `TaskGroupData` of
                    // ids only, mirroring `TaskScheduler::resolve_task_group`'s
                    // All-mode shape). Push a fresh `Arc<TaskGroupData>`
                    // result carrier kinded `Ptr(HeapKind::TaskGroup)`.
                    0 => {
                        for &id in &task_ids {
                            let result = self.resolve_spawned_task(id)?;
                            drop_with_kind(result.raw(), result.kind());
                            std::mem::forget(result);
                        }
                        let aggregate: Arc<TaskGroupData> = Arc::new(TaskGroupData {
                            kind: 0,
                            task_ids: task_ids.clone(),
                        });
                        let result_bits = Arc::into_raw(aggregate) as u64;
                        self.push_kinded(
                            result_bits,
                            NativeKind::Ptr(HeapKind::TaskGroup),
                        )?;
                    }
                    // Race: resolve all tasks; return the first result.
                    // Matches `TaskScheduler::resolve_task_group`'s
                    // race-mode semantics. Empty list → RuntimeError.
                    1 => {
                        let mut pushed = false;
                        for (idx, &id) in task_ids.iter().enumerate() {
                            let result = self.resolve_spawned_task(id)?;
                            if idx == 0 {
                                self.push_kinded(result.raw(), result.kind())?;
                                std::mem::forget(result);
                                pushed = true;
                            } else {
                                // Subsequent results: their shares aren't
                                // returned to the user; release each.
                                drop_with_kind(result.raw(), result.kind());
                                std::mem::forget(result);
                            }
                        }
                        if !pushed {
                            return Err(VMError::RuntimeError(
                                "Race join with empty task list".to_string(),
                            ));
                        }
                    }
                    // Any: return first success; on errors, keep the
                    // last for the empty-success fallback. Matches
                    // `TaskScheduler::resolve_task_group`'s any-mode.
                    2 => {
                        let mut last_err: Option<VMError> = None;
                        let mut pushed = false;
                        for &id in &task_ids {
                            match self.resolve_spawned_task(id) {
                                Ok(result) => {
                                    self.push_kinded(result.raw(), result.kind())?;
                                    std::mem::forget(result);
                                    pushed = true;
                                    break;
                                }
                                Err(e) => last_err = Some(e),
                            }
                        }
                        if !pushed {
                            return Err(last_err.unwrap_or_else(|| {
                                VMError::RuntimeError(
                                    "Any join with empty task list".to_string(),
                                )
                            }));
                        }
                    }
                    // AllSettled: drive every task; per-task errors are
                    // preserved in the scheduler's result map (caller
                    // can inspect via `get_result`). Aggregate carrier
                    // kind=3 mirrors `TaskScheduler::resolve_task_group`.
                    // The {status, value/error} array view depends on
                    // a kinded VMArray helper that's Phase-2c per
                    // ADR-006 §2.7.4 — the TaskGroup carrier is the
                    // minimum shape the await-time decoder can re-walk.
                    3 => {
                        for &id in &task_ids {
                            if let Ok(result) = self.resolve_spawned_task(id) {
                                drop_with_kind(result.raw(), result.kind());
                                std::mem::forget(result);
                            }
                            // Errors per-task are preserved in the
                            // scheduler's result map.
                        }
                        let aggregate: Arc<TaskGroupData> = Arc::new(TaskGroupData {
                            kind: 3,
                            task_ids: task_ids.clone(),
                        });
                        let result_bits = Arc::into_raw(aggregate) as u64;
                        self.push_kinded(
                            result_bits,
                            NativeKind::Ptr(HeapKind::TaskGroup),
                        )?;
                    }
                    other => {
                        return Err(VMError::RuntimeError(format!(
                            "Unknown join kind: {}",
                            other
                        )));
                    }
                }

                debug_assert_eq!(
                    self.sp, sp_before,
                    "op_join_await: stack depth changed (before={}, after={})",
                    sp_before, self.sp
                );
                Ok(AsyncExecutionResult::Continue)
            }
            _ => {
                drop_with_kind(bits, slot_kind);
                Err(VMError::RuntimeError(format!(
                    "JoinAwait expected TaskGroup, got {:?}",
                    slot_kind
                )))
            }
        }
    }

    /// Cancel a task by its future ID
    ///
    /// Pops a Future(task_id) from the stack and signals cancellation.
    /// The host runtime is responsible for actually cancelling the task.
    fn op_cancel_task(&mut self) -> Result<AsyncExecutionResult, VMError> {
        let (bits, slot_kind) = self.pop_kinded()?;
        match slot_kind {
            NativeKind::Ptr(HeapKind::Future) => {
                // Future is an inline scalar — bits IS the id. No Arc share
                // to drop (Future is a no-op in drop_with_kind).
                let id = bits;
                self.task_scheduler.cancel(id);
                Ok(AsyncExecutionResult::Continue)
            }
            _ => {
                drop_with_kind(bits, slot_kind);
                Err(VMError::RuntimeError(format!(
                    "CancelTask expected Future, got {:?}",
                    slot_kind
                )))
            }
        }
    }

    /// Enter a structured concurrency scope
    ///
    /// Pushes a new empty Vec onto the async_scope_stack.
    /// All tasks spawned while this scope is active are tracked in that Vec.
    fn op_async_scope_enter(&mut self) -> Result<AsyncExecutionResult, VMError> {
        let depth_before = self.async_scope_stack.len();
        self.async_scope_stack.push(Vec::new());
        debug_assert_eq!(
            self.async_scope_stack.len(),
            depth_before + 1,
            "op_async_scope_enter: scope stack depth not incremented"
        );
        Ok(AsyncExecutionResult::Continue)
    }

    /// Exit a structured concurrency scope
    ///
    /// Pops the current scope from the async_scope_stack and cancels
    /// all tasks spawned within it that are still pending, in LIFO order.
    /// The body's result value remains on top of the stack.
    fn op_async_scope_exit(&mut self) -> Result<AsyncExecutionResult, VMError> {
        debug_assert!(
            !self.async_scope_stack.is_empty(),
            "op_async_scope_exit: scope stack is empty (mismatched Enter/Exit)"
        );
        if let Some(mut scope_tasks) = self.async_scope_stack.pop() {
            // Cancel in LIFO order (last spawned first)
            scope_tasks.reverse();
            for task_id in scope_tasks {
                self.task_scheduler.cancel(task_id);
            }
        }
        // Result value from the body is already on top of the stack
        Ok(AsyncExecutionResult::Continue)
    }

    /// Emit a generic event to the event queue
    ///
    /// Pops an event object from the stack and pushes it to
    /// the event queue for external consumers.
    fn op_emit_event(&mut self) -> Result<AsyncExecutionResult, VMError> {
        // Pop the event payload and release its share — event queue
        // integration is deferred. Drop discipline (playbook §3): every
        // `pop_kinded` either re-pushes or `drop_with_kind`s.
        let (bits, kind) = self.pop_kinded()?;
        drop_with_kind(bits, kind);
        Ok(AsyncExecutionResult::Continue)
    }
}

/// Check if an opcode is an async operation
#[cfg(test)]
pub fn is_async_opcode(opcode: OpCode) -> bool {
    matches!(
        opcode,
        OpCode::Yield
            | OpCode::Suspend
            | OpCode::Resume
            | OpCode::Poll
            | OpCode::AwaitBar
            | OpCode::AwaitTick
            | OpCode::EmitAlert
            | OpCode::EmitEvent
            | OpCode::Await
            | OpCode::SpawnTask
            | OpCode::JoinInit
            | OpCode::JoinAwait
            | OpCode::CancelTask
            | OpCode::AsyncScopeEnter
            | OpCode::AsyncScopeExit
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_async_opcode() {
        assert!(is_async_opcode(OpCode::Yield));
        assert!(is_async_opcode(OpCode::Suspend));
        assert!(is_async_opcode(OpCode::EmitAlert));
        assert!(is_async_opcode(OpCode::AsyncScopeEnter));
        assert!(is_async_opcode(OpCode::AsyncScopeExit));
        assert!(!is_async_opcode(OpCode::AddInt));
        assert!(!is_async_opcode(OpCode::Jump));
    }

    #[test]
    fn test_is_async_opcode_all_variants() {
        // Test all async opcodes
        assert!(is_async_opcode(OpCode::Yield));
        assert!(is_async_opcode(OpCode::Suspend));
        assert!(is_async_opcode(OpCode::Resume));
        assert!(is_async_opcode(OpCode::Poll));
        assert!(is_async_opcode(OpCode::AwaitBar));
        assert!(is_async_opcode(OpCode::AwaitTick));
        assert!(is_async_opcode(OpCode::EmitAlert));
        assert!(is_async_opcode(OpCode::EmitEvent));

        // Test non-async opcodes
        assert!(!is_async_opcode(OpCode::PushConst));
        assert!(!is_async_opcode(OpCode::Return));
        assert!(!is_async_opcode(OpCode::Call));
        assert!(!is_async_opcode(OpCode::Nop));
    }

    #[test]
    fn test_async_execution_result_variants() {
        // Test Continue
        let continue_result = AsyncExecutionResult::Continue;
        assert!(matches!(continue_result, AsyncExecutionResult::Continue));

        // Test Yielded
        let yielded_result = AsyncExecutionResult::Yielded;
        assert!(matches!(yielded_result, AsyncExecutionResult::Yielded));

        // Test Suspended
        let suspended_result = AsyncExecutionResult::Suspended(SuspensionInfo {
            wait_type: WaitType::AnyEvent,
            resume_ip: 42,
        });
        match suspended_result {
            AsyncExecutionResult::Suspended(info) => {
                assert_eq!(info.resume_ip, 42);
                assert!(matches!(info.wait_type, WaitType::AnyEvent));
            }
            _ => panic!("Expected Suspended"),
        }
    }

    #[test]
    fn test_wait_type_variants() {
        // NextBar
        let next_bar = WaitType::NextBar {
            source: "market_data".to_string(),
        };
        match next_bar {
            WaitType::NextBar { source } => assert_eq!(source, "market_data"),
            _ => panic!("Expected NextBar"),
        }

        // Timer
        let timer = WaitType::Timer { id: 123 };
        match timer {
            WaitType::Timer { id } => assert_eq!(id, 123),
            _ => panic!("Expected Timer"),
        }

        // AnyEvent
        let any = WaitType::AnyEvent;
        assert!(matches!(any, WaitType::AnyEvent));
    }

    #[test]
    fn test_suspension_info_creation() {
        let info = SuspensionInfo {
            wait_type: WaitType::Timer { id: 999 },
            resume_ip: 100,
        };

        assert_eq!(info.resume_ip, 100);
        assert!(matches!(info.wait_type, WaitType::Timer { id: 999 }));
    }

    #[test]
    fn test_is_async_opcode_await() {
        assert!(is_async_opcode(OpCode::Await));
    }

    #[test]
    fn test_wait_type_future() {
        let future = WaitType::Future { id: 42 };
        match future {
            WaitType::Future { id } => assert_eq!(id, 42),
            _ => panic!("Expected Future"),
        }
    }

    #[test]
    fn test_is_async_opcode_join_opcodes() {
        assert!(is_async_opcode(OpCode::SpawnTask));
        assert!(is_async_opcode(OpCode::JoinInit));
        assert!(is_async_opcode(OpCode::JoinAwait));
        assert!(is_async_opcode(OpCode::CancelTask));
    }

    #[test]
    fn test_wait_type_task_group() {
        let tg = WaitType::TaskGroup {
            kind: 0,
            task_ids: vec![1, 2, 3],
        };
        match tg {
            WaitType::TaskGroup { kind, task_ids } => {
                assert_eq!(kind, 0); // All
                assert_eq!(task_ids.len(), 3);
                assert_eq!(task_ids, vec![1, 2, 3]);
            }
            _ => panic!("Expected TaskGroup"),
        }
    }

    #[test]
    fn test_wait_type_task_group_race() {
        let tg = WaitType::TaskGroup {
            kind: 1,
            task_ids: vec![10, 20],
        };
        match tg {
            WaitType::TaskGroup { kind, task_ids } => {
                assert_eq!(kind, 1); // Race
                assert_eq!(task_ids, vec![10, 20]);
            }
            _ => panic!("Expected TaskGroup"),
        }
    }
}
