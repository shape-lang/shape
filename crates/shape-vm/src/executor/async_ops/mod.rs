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
    KindedSlot, NativeKind, VMError,
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
        // R5b-2-bool-null-sentinel-cluster (ADR-006 §2.7 + §2.7.7/Q9,
        // 2026-05-19): no event available — push `NativeKind::Null` per
        // post-disposition; pre-disposition `(0u64, NativeKind::Bool)`
        // collided with legitimate `false` bool slots.
        self.push_kinded(0u64, NativeKind::Null)?;
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
                // WF-2D real concurrency: if this future names an in-flight
                // async module task (spawned onto the shared runtime by
                // `spawn_async_module_future`), block on its completion
                // channel. Because the task started when the async call was
                // evaluated — not here — a sibling launched earlier has been
                // running concurrently, so awaiting an already-elapsed sibling
                // returns promptly. Otherwise fall back to inline synchronous
                // resolution of a registered closure / cached value.
                let result = if self.task_scheduler.has_pending_async(task_id) {
                    self.resolve_pending_async_task(task_id)?
                } else {
                    self.resolve_spawned_task(task_id)?
                };

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
    /// Spawn a task from a callable or pre-resolved value on the stack.
    ///
    /// Pops a top-of-stack kinded slot and creates a new async task identified
    /// by a fresh future id. Pushes a `Ptr(HeapKind::Future)` onto the stack
    /// representing the spawned task.
    ///
    /// Two callable-classification paths per the §2.7.11/Q12 value-call dispatch:
    ///
    /// - **Callable kinds** (`NativeKind::Ptr(HeapKind::Closure)` /
    ///   `NativeKind::UInt64`): registered with the scheduler via `register`
    ///   for later inline execution at `resolve_spawned_task` time. Same shape
    ///   as the W7-cv-static / W7-cv-async dispatch classification at
    ///   `call_convention.rs::resolve_spawned_task` (line 438).
    ///
    /// - **Non-callable kinds** (every other `NativeKind`, including scalars
    ///   `Int64`/`Float64`/`Bool`/`String` and heap-bearing
    ///   `Ptr(HeapKind::*)` for non-callable heap types): treated as already-
    ///   resolved values per the §2.7.4 sync-shortcut semantic. The compiler
    ///   surface emits `compile_expr + SpawnTask` for `async let x = 42`,
    ///   `join all { 1+2, 3+4 }`, and other RHS / branch expressions that
    ///   evaluate to plain values (not closure literals or function
    ///   references). Without this path, `resolve_spawned_task` would
    ///   surface `callable must be NativeKind::Ptr(HeapKind::Closure) or
    ///   NativeKind::UInt64` for every non-callable RHS.
    ///
    ///   The non-callable path uses the scheduler's existing `complete(id,
    ///   bits, kind)` API (same shape as the external-completion path used
    ///   by remote calls); the share transfers from the stack slot into the
    ///   scheduler's cached-result entry. `resolve_spawned_task` then hits
    ///   its `TaskStatus::Completed` cached fast-path at line 407 and
    ///   returns the value cleanly via `clone_with_kind`.
    ///
    /// If inside an async scope, the spawned future id is tracked for
    /// cancellation. Cancellation of a non-callable / pre-completed task is
    /// a no-op (the result is already stored — `cancel` only releases the
    /// `callables` entry, which is empty for non-callable kinds).
    fn op_spawn_task(&mut self) -> Result<AsyncExecutionResult, VMError> {
        let sp_before = self.sp;
        // Pop the top-of-stack kinded slot. The share transfers to the
        // task_scheduler via either `register` (callable kinds) or
        // `complete` (non-callable kinds) — same retain-on-store contract
        // as the §2.7.7 stack and §2.7.8 cell-storage tracks (one
        // strong-count share owned by the storage; released by
        // `take_callable` / cached-result drop / `Drop`). No
        // `drop_with_kind` here: the share is moved, not released.
        let (slot_bits, slot_kind) = self.pop_kinded()?;

        // Allocate a fresh future id. `next_future_id` is monotonic and
        // single-threaded (the VM is `!Sync` per the module docstring's
        // concurrency model section).
        let task_id = self.next_future_id();

        // Classify the popped slot per the §2.7.11/Q12 callable-vs-value
        // dispatch shape (mirrors `call_convention.rs::resolve_spawned_task`
        // line 438 + `call_value_immediate_nb` line 854). Callables route
        // through `register` for later inline execution; non-callable
        // values route through `complete` as pre-resolved results.
        match slot_kind {
            NativeKind::Ptr(HeapKind::Future) => {
                // WF-2D real concurrency: the RHS was an async module call
                // (e.g. `async let a = time::sleep(1000)`). The call already
                // spawned a real background task onto the shared runtime and
                // produced this `Future(id)` handle — it is running NOW. Do
                // NOT allocate a new task id or re-register: pass the existing
                // future id straight through so `async let a = time::sleep(..)`
                // and `async let b = time::sleep(..)` overlap. `bits` IS the
                // in-flight task id. Discard the freshly-minted `task_id`; the
                // scope tracks the real one below.
                //
                // Track the real future id in the active async scope so scope
                // exit / cancel can abort it (structured concurrency).
                if let Some(scope) = self.async_scope_stack.last_mut() {
                    scope.push(slot_bits);
                }
                // Future is an inline-scalar payload (bits IS the id, no Arc
                // share); re-push unchanged.
                self.push_kinded(slot_bits, NativeKind::Ptr(HeapKind::Future))?;
                debug_assert_eq!(
                    self.sp, sp_before,
                    "op_spawn_task (Future passthrough): stack depth changed \
                     (before={}, after={})",
                    sp_before, self.sp
                );
                return Ok(AsyncExecutionResult::Continue);
            }
            NativeKind::Ptr(HeapKind::Closure) | NativeKind::UInt64 => {
                // Callable — register for later execution.
                self.task_scheduler.register(task_id, slot_bits, slot_kind);
            }
            _ => {
                // Non-callable pre-resolved value. Register with Pending
                // status so the scope tracking + `is_resolved` checks
                // behave uniformly with the callable path, then `complete`
                // immediately so `resolve_spawned_task` hits the
                // cached-result fast-path. The scheduler's `register`
                // requires a `(bits, kind)` pair to slot into the
                // `callables` map for refcount discipline; for the
                // pre-resolved path we transfer the share directly to
                // the `results` map via `complete` and ensure the
                // `callables` entry is empty so the take-callable arm
                // never fires.
                //
                // Refcount discipline: `complete(task_id, bits, kind)`
                // takes one strong-count share into the cached-result
                // entry; the share transferred to us from `pop_kinded`
                // moves into the scheduler. `resolve_spawned_task`'s
                // cached fast-path at `call_convention.rs:407` clones a
                // fresh share via `clone_with_kind` for the returned
                // `KindedSlot`, leaving the cached entry's share intact.
                self.task_scheduler.complete(task_id, slot_bits, slot_kind);
            }
        }

        // Track the spawned future id in the active async scope (if any)
        // so `op_async_scope_exit` can cancel still-pending tasks in
        // LIFO order (structured concurrency contract — see module
        // docstring's "Structured Concurrency" section). Pre-completed
        // tasks ignore `cancel` per `TaskScheduler::cancel` line 141
        // ("Only cancel if still pending").
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
                    // All: block on every task and drop each per-task share.
                    // WF-2D: async module tasks in the group are already
                    // running concurrently on the shared runtime (each was
                    // spawned when its branch expression was evaluated), so
                    // blocking on them in sequence costs ~max(branch), not
                    // sum(branch). The aggregate carrier is a `TaskGroupData`
                    // of ids only. Push a fresh `Ptr(HeapKind::TaskGroup)`.
                    0 => {
                        for &id in &task_ids {
                            let result = self.resolve_join_task_blocking(id)?;
                            drop_with_kind(result.raw(), result.kind());
                            std::mem::forget(result);
                        }
                        let aggregate: Arc<TaskGroupData> = Arc::new(TaskGroupData {
                            kind: 0,
                            task_ids: task_ids.clone(),
                        });
                        let result_bits = Arc::into_raw(aggregate) as u64;
                        self.push_kinded(result_bits, NativeKind::Ptr(HeapKind::TaskGroup))?;
                    }
                    // Race: return the FIRST branch to settle (success or
                    // error); genuinely cancel the losers (WF-2D). Empty
                    // list → RuntimeError.
                    1 => {
                        if task_ids.is_empty() {
                            return Err(VMError::RuntimeError(
                                "Race join with empty task list".to_string(),
                            ));
                        }
                        let (winner_id, result) = self.join_race_first_settled(&task_ids)?;
                        // Cancel/abort every non-winner: in-flight async tasks
                        // are aborted on the shared runtime; registered
                        // callables release their share.
                        for &other in &task_ids {
                            if other != winner_id {
                                self.task_scheduler.cancel(other);
                            }
                        }
                        self.push_kinded(result.raw(), result.kind())?;
                        std::mem::forget(result);
                    }
                    // Any: return the first branch to SUCCEED, skipping
                    // failures; cancel the losers once a winner is found.
                    // All failed → surface the last error (WF-2D).
                    2 => {
                        if task_ids.is_empty() {
                            return Err(VMError::RuntimeError(
                                "Any join with empty task list".to_string(),
                            ));
                        }
                        let (winner_id, result) = self.join_any_first_success(&task_ids)?;
                        for &other in &task_ids {
                            if other != winner_id {
                                self.task_scheduler.cancel(other);
                            }
                        }
                        self.push_kinded(result.raw(), result.kind())?;
                        std::mem::forget(result);
                    }
                    // AllSettled: block on every task; successes drop their
                    // share, errors are preserved in the scheduler result
                    // map (caller can inspect via `get_result`). Aggregate
                    // carrier kind=3. The {status, value/error} array view
                    // depends on a kinded VMArray helper that's Phase-2c per
                    // ADR-006 §2.7.4 — the TaskGroup carrier is the minimum
                    // shape the await-time decoder can re-walk.
                    3 => {
                        for &id in &task_ids {
                            if let Ok(result) = self.resolve_join_task_blocking(id) {
                                drop_with_kind(result.raw(), result.kind());
                                std::mem::forget(result);
                            }
                            // Errors per-task are preserved in the scheduler's
                            // result map.
                        }
                        let aggregate: Arc<TaskGroupData> = Arc::new(TaskGroupData {
                            kind: 3,
                            task_ids: task_ids.clone(),
                        });
                        let result_bits = Arc::into_raw(aggregate) as u64;
                        self.push_kinded(result_bits, NativeKind::Ptr(HeapKind::TaskGroup))?;
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

    /// Resolve one join-group member to completion, blocking (WF-2D).
    ///
    /// Routes to the in-flight async resolver when the id names a real
    /// background task (`time::sleep`-style), otherwise to the synchronous
    /// closure / cached-value resolver. The returned `KindedSlot` owns one
    /// share (the caller drops or transfers it).
    fn resolve_join_task_blocking(&mut self, id: u64) -> Result<KindedSlot, VMError> {
        if self.task_scheduler.has_pending_async(id) {
            self.resolve_pending_async_task(id)
        } else {
            self.resolve_spawned_task(id)
        }
    }

    /// `join race`: return the id + result of the first branch to SETTLE
    /// (success or error) (WF-2D real race).
    ///
    /// A non-async branch (immediate value / cached / synchronous closure)
    /// settles the instant it is resolved, so the first such in source order
    /// wins. If every branch is an in-flight async task, poll their
    /// completion channels until one settles — because they are all running
    /// concurrently on the shared runtime, this returns at roughly the
    /// shortest branch's duration, not their sum.
    fn join_race_first_settled(
        &mut self,
        task_ids: &[u64],
    ) -> Result<(u64, KindedSlot), VMError> {
        for &id in task_ids {
            if !self.task_scheduler.has_pending_async(id) {
                let result = self.resolve_spawned_task(id)?;
                return Ok((id, result));
            }
        }
        // All in-flight async — poll for the first to settle. First error
        // also counts as "settled" for race and is surfaced.
        loop {
            for &id in task_ids {
                if let Some(result) = self.try_resolve_pending_async_task(id)? {
                    return Ok((id, result));
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    /// `join any`: return the id + result of the first branch to SUCCEED,
    /// skipping failures; if every branch fails, surface the last error
    /// (WF-2D real any).
    fn join_any_first_success(
        &mut self,
        task_ids: &[u64],
    ) -> Result<(u64, KindedSlot), VMError> {
        let mut last_err: Option<VMError> = None;
        // Synchronous branches settle immediately — try them in order first.
        for &id in task_ids {
            if !self.task_scheduler.has_pending_async(id) {
                match self.resolve_spawned_task(id) {
                    Ok(result) => return Ok((id, result)),
                    Err(e) => last_err = Some(e),
                }
            }
        }
        // Poll the in-flight async branches; a branch that errors drops out
        // of the running set, a branch that succeeds wins.
        let mut remaining: Vec<u64> = task_ids
            .iter()
            .copied()
            .filter(|&id| self.task_scheduler.has_pending_async(id))
            .collect();
        while !remaining.is_empty() {
            let mut still_pending: Vec<u64> = Vec::with_capacity(remaining.len());
            for &id in &remaining {
                match self.try_resolve_pending_async_task(id) {
                    Ok(Some(result)) => return Ok((id, result)),
                    Ok(None) => still_pending.push(id),
                    Err(e) => last_err = Some(e),
                }
            }
            remaining = still_pending;
            if !remaining.is_empty() {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
        Err(last_err
            .unwrap_or_else(|| VMError::RuntimeError("Any join with empty task list".to_string())))
    }

    /// Cancel a task by its future ID
    ///
    /// Pops a Future(task_id) from the stack and signals cancellation.
    /// `task_scheduler.cancel` aborts an in-flight async task on the shared
    /// runtime (WF-2D real cancellation) or releases a registered callable.
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
