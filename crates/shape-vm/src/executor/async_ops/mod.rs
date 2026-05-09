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
//! The legacy `task_scheduler::TaskScheduler` API takes `ValueWord` (deleted
//! in shape-value), so any handler that needs to thread a callable into the
//! scheduler — `op_spawn_task`, `op_await`, `op_join_await` — surfaces the
//! out-of-territory dependency via `todo!("phase-2c — ADR-006 §2.7.4: \
//! task_scheduler ValueWord API not migrated to kinded; out of E-async \
//! territory")`. The §10 dispatch protocol forbids editing files outside the
//! sub-cluster's listed territory; `task_scheduler.rs` is unowned and must be
//! migrated in a follow-up cluster before the spawn/await fast-paths can
//! re-light. Stack-side migration here is complete; suspended call sites are
//! tracked, not silently papered over.

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
                // Future(id) is an inline scalar — `bits` IS the future ID.
                // No Arc share to drop (HeapKind::Future is a no-op in
                // drop_with_kind). task_scheduler API still takes `ValueWord`
                // (out-of-territory for E-async; see ADR-006 §2.7.4 / playbook
                // §10 E-async row). Surface the suspended call rather than
                // fabricate a forbidden ValueWord shim.
                let _id = bits;
                let _ = sp_before;
                todo!(
                    "phase-2c — ADR-006 §2.7.4: task_scheduler::resolve_task \
                     takes ValueWord; migration belongs to a separate \
                     task_scheduler cluster (out of E-async territory)"
                );
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
        let _sp_before = self.sp;
        // Pop the callable's kinded slot. The share would transfer to the
        // task_scheduler — but the scheduler API takes `ValueWord` (deleted),
        // so this surfaces as out-of-territory work.
        let (callable_bits, callable_kind) = self.pop_kinded()?;
        // Drop the popped share to keep refcount discipline correct under the
        // todo!() surface — without this, the share leaks until phase-2c
        // unblocks the scheduler migration.
        drop_with_kind(callable_bits, callable_kind);
        todo!(
            "phase-2c — ADR-006 §2.7.4: task_scheduler::register takes \
             ValueWord; spawn-task callable threading belongs to a separate \
             task_scheduler cluster (out of E-async territory). Future ID \
             allocation + async-scope tracking + Future-kinded push will \
             re-light once the scheduler API is migrated to (bits, kind)."
        );
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
                // Reclaim the Arc<TaskGroupData> share that pop_kinded just
                // transferred to us. We extract `kind` + `task_ids.clone()`
                // for the suspension fall-back, then drop the Arc.
                //
                // SAFETY: the construction-side contract for
                // `push_kinded(bits, Ptr(HeapKind::TaskGroup))` (see
                // `op_join_init` above + ADR-006 §2.3) guarantees `bits` is
                // the result of `Arc::into_raw::<TaskGroupData>` and we own
                // exactly one strong-count share.
                let arc: Arc<TaskGroupData> =
                    unsafe { Arc::from_raw(bits as *const TaskGroupData) };
                let _kind = arc.kind;
                let _task_ids = arc.task_ids.clone();
                drop(arc);
                let _ = sp_before;
                // task_scheduler::resolve_task_group still takes `ValueWord`
                // (out-of-territory). Surface and stop per playbook §10 row.
                todo!(
                    "phase-2c — ADR-006 §2.7.4: task_scheduler::resolve_task_group \
                     takes ValueWord; join-await result threading belongs to a \
                     separate task_scheduler cluster (out of E-async territory)."
                );
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
