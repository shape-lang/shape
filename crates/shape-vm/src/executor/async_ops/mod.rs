//! Async operations for the VM executor
//!
//! Handles: Yield, Suspend, Resume, Poll, AwaitBar, AwaitTick, EmitAlert, EmitEvent
//!
//! These opcodes enable cooperative multitasking and event-driven execution
//! in a platform-agnostic way (works on Tokio and bare metal).

use crate::{
    bytecode::{Instruction, OpCode, Operand},
    executor::VirtualMachine,
};
use shape_value::heap_value::HeapValue;
use shape_value::{VMError, ValueWord};

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
        // This is handled via the VMContext passed from the runtime
        // For now, push None to indicate no event
        self.push_vw(ValueWord::none()).map_err(|e| e)?;
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
        let _alert_nb = self.pop_vw()?;
        // Alert pipeline integration pending — consume and continue
        Ok(AsyncExecutionResult::Continue)
    }

    /// General-purpose await
    ///
    /// Pops a value from the stack. If it's a Future(id), suspends execution.
    /// If it's any other value, pushes it back (sync shortcut — the value is already resolved).
    fn op_await(&mut self) -> Result<AsyncExecutionResult, VMError> {
        let nb = self.pop_vw()?;
        match nb.as_heap_ref() {
            Some(HeapValue::Future(id)) => {
                let id = *id;
                Ok(AsyncExecutionResult::Suspended(SuspensionInfo {
                    wait_type: WaitType::Future { id },
                    resume_ip: self.ip,
                }))
            }
            _ => {
                // Sync shortcut: value is already resolved, push it back
                self.push_vw(nb)?;
                Ok(AsyncExecutionResult::Continue)
            }
        }
    }

    /// Spawn a task from a closure/function on the stack
    ///
    /// Pops a closure or function reference from the stack and creates a new async task.
    /// Pushes a Future(task_id) onto the stack representing the spawned task.
    /// The host runtime is responsible for actually scheduling the task.
    ///
    /// If inside an async scope, the spawned future ID is tracked for cancellation.
    fn op_spawn_task(&mut self) -> Result<AsyncExecutionResult, VMError> {
        let callable_nb = self.pop_vw()?;

        let task_id = self.next_future_id();
        self.task_scheduler.register(task_id, callable_nb);

        if let Some(scope) = self.async_scope_stack.last_mut() {
            scope.push(task_id);
        }

        self.push_vw(ValueWord::from_future(task_id))?;
        Ok(AsyncExecutionResult::Continue)
    }

    /// Initialize a join group from futures on the stack
    ///
    /// Operand: Count(packed_u16) where high 2 bits = join kind, low 14 bits = arity.
    /// Pops `arity` Future values from the stack (in reverse order).
    /// Pushes a ValueWord::TaskGroup with the collected future IDs.
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

        let mut task_ids = Vec::with_capacity(arity);
        for _ in 0..arity {
            let nb = self.pop_vw()?;
            match nb.as_heap_ref() {
                Some(HeapValue::Future(id)) => task_ids.push(*id),
                _ => {
                    return Err(VMError::RuntimeError(format!(
                        "JoinInit expected Future, got {}",
                        nb.type_name()
                    )));
                }
            }
        }
        // Reverse so task_ids[0] corresponds to first branch
        task_ids.reverse();

        self.push_vw(ValueWord::from_heap_value(
            shape_value::heap_value::HeapValue::TaskGroup { kind, task_ids },
        ))?;
        Ok(AsyncExecutionResult::Continue)
    }

    /// Await a task group, suspending until the join condition is met
    ///
    /// Pops a ValueWord::TaskGroup from the stack.
    /// Suspends execution with WaitType::TaskGroup so the host can resolve it
    /// according to the join strategy (all/race/any/settle).
    /// On resume, the host pushes the result value onto the stack.
    fn op_join_await(&mut self) -> Result<AsyncExecutionResult, VMError> {
        let nb = self.pop_vw()?;
        match nb.as_heap_ref() {
            Some(HeapValue::TaskGroup { kind, task_ids }) => {
                Ok(AsyncExecutionResult::Suspended(SuspensionInfo {
                    wait_type: WaitType::TaskGroup {
                        kind: *kind,
                        task_ids: task_ids.clone(),
                    },
                    resume_ip: self.ip,
                }))
            }
            _ => Err(VMError::RuntimeError(format!(
                "JoinAwait expected TaskGroup, got {}",
                nb.type_name()
            ))),
        }
    }

    /// Cancel a task by its future ID
    ///
    /// Pops a Future(task_id) from the stack and signals cancellation.
    /// The host runtime is responsible for actually cancelling the task.
    fn op_cancel_task(&mut self) -> Result<AsyncExecutionResult, VMError> {
        let nb = self.pop_vw()?;
        match nb.as_heap_ref() {
            Some(HeapValue::Future(id)) => {
                self.task_scheduler.cancel(*id);
                Ok(AsyncExecutionResult::Continue)
            }
            _ => Err(VMError::RuntimeError(format!(
                "CancelTask expected Future, got {}",
                nb.type_name()
            ))),
        }
    }

    /// Enter a structured concurrency scope
    ///
    /// Pushes a new empty Vec onto the async_scope_stack.
    /// All tasks spawned while this scope is active are tracked in that Vec.
    fn op_async_scope_enter(&mut self) -> Result<AsyncExecutionResult, VMError> {
        self.async_scope_stack.push(Vec::new());
        Ok(AsyncExecutionResult::Continue)
    }

    /// Exit a structured concurrency scope
    ///
    /// Pops the current scope from the async_scope_stack and cancels
    /// all tasks spawned within it that are still pending, in LIFO order.
    /// The body's result value remains on top of the stack.
    fn op_async_scope_exit(&mut self) -> Result<AsyncExecutionResult, VMError> {
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
        let _event_nb = self.pop_vw()?;
        // Event queue integration pending — consume and continue
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
        assert!(!is_async_opcode(OpCode::Add));
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
