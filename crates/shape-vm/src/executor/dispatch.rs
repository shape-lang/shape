//! Main execution loop and opcode dispatch.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use crate::bytecode::{Instruction, OpCode};
use shape_value::{VMError, ValueWord, ValueWordExt};

use super::debugger_integration::DebuggerIntegration;
use super::{DebugVMState, ExecutionResult, VirtualMachine, async_ops};

impl VirtualMachine {
    /// Execute the loaded program
    ///
    /// # Arguments
    /// * `ctx` - Optional ExecutionContext for trading operations (rows, indicators, etc.)
    pub fn execute(
        &mut self,
        ctx: Option<&mut shape_runtime::context::ExecutionContext>,
    ) -> Result<ValueWord, VMError> {
        match self.execute_with_suspend(ctx)? {
            ExecutionResult::Completed(value) => Ok(value),
            ExecutionResult::Suspended { future_id, .. } => Err(VMError::Suspended {
                future_id,
                resume_ip: 0,
            }),
        }
    }

    /// Execute the loaded program, returning either a completed value or suspension info.
    ///
    /// Unlike `execute()`, this method distinguishes between completion and suspension,
    /// allowing the host to resume execution after resolving a future.
    pub fn execute_with_suspend(
        &mut self,
        mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
    ) -> Result<ExecutionResult, VMError> {
        self.clear_last_uncaught_exception();

        // Fast path: when no debugger is attached and tracing is off, use the
        // streamlined loop that skips per-instruction debug/trace checks.
        if self.debugger.is_none() && !self.config.trace_execution {
            return self.execute_fast_with_exceptions(ctx);
        }

        // Start debugger if enabled
        if let Some(ref mut debugger) = self.debugger {
            debugger.start();
        }

        while self.ip < self.program.instructions.len() {
            // Check for debug break
            let should_break = if let Some(ref mut debugger) = self.debugger {
                debugger.should_break(
                    &DebugVMState {
                        ip: self.ip,
                        call_stack_depth: self.call_stack.len(),
                    },
                    self.ip,
                )
            } else {
                false
            };

            if should_break {
                if let Some(ref mut debugger) = self.debugger {
                    debugger.debug_break(
                        &DebugVMState {
                            ip: self.ip,
                            call_stack_depth: self.call_stack.len(),
                        },
                        &self.program,
                    );
                }
            }

            let instruction = self.program.instructions[self.ip];

            // Record instruction in metrics (opt-in, near-zero cost when None)
            if let Some(ref mut metrics) = self.metrics {
                metrics.record_instruction();
            }

            // Trace instruction if enabled
            if self.config.trace_execution {
                if let Some(ref debugger) = self.debugger {
                    debugger.trace_instruction(
                        &DebugVMState {
                            ip: self.ip,
                            call_stack_depth: self.call_stack.len(),
                        },
                        &self.program,
                        &instruction,
                    );
                } else {
                    self.trace_state();
                }
            }

            self.ip += 1;
            self.instruction_count += 1;

            // Check for Ctrl+C interrupt every 1024 instructions
            if self.instruction_count & 0x3FF == 0 && self.interrupt.load(Ordering::Relaxed) > 0 {
                return Err(VMError::Interrupted);
            }

            // Resource limit check (sandboxed execution)
            if let Some(ref mut usage) = self.resource_usage {
                usage
                    .tick_instruction()
                    .map_err(|e| VMError::RuntimeError(e.to_string()))?;
            }

            // Poll for completed tier promotions every 1024 instructions.
            if self.instruction_count & 0x3FF == 0 {
                self.poll_tier_completions();
            }

            // GC safepoint poll (gc feature only)
            #[cfg(feature = "gc")]
            if self.instruction_count & 0x3FF == 0 {
                self.gc_safepoint_poll();

                // Incremental marking: make bounded progress on the gray worklist
                // when a marking cycle is active, without stopping the world.
                if self.gc_heap.as_ref().map_or(false, |h| h.is_marking()) {
                    self.gc_incremental_mark_step();
                }
            }

            // Time-travel capture check (debug path).
            if let Some(ref mut tt) = self.time_travel {
                let current_ip = self.ip.saturating_sub(1);
                let is_call_or_return = matches!(
                    instruction.opcode,
                    OpCode::Call | OpCode::CallValue | OpCode::Return | OpCode::ReturnValue
                );
                if tt.should_capture(current_ip, self.instruction_count as u64, is_call_or_return) {
                    if let Ok(store) = tt.snapshot_store() {
                        let store_ptr = store as *const shape_runtime::snapshot::SnapshotStore;
                        if let Ok(snap) = self.snapshot(unsafe { &*store_ptr }) {
                            let call_depth = self.call_stack.len();
                            self.time_travel.as_mut().unwrap().record(
                                snap,
                                current_ip,
                                self.instruction_count as u64,
                                call_depth,
                            );
                        }
                    }
                }
            }

            // Track instruction index before execution for error reporting
            let error_ip = self.ip.saturating_sub(1);

            if let Err(err) = self.execute_instruction(&instruction, ctx.as_deref_mut()) {
                // Check for suspension (not a real error)
                if let VMError::Suspended {
                    future_id,
                    resume_ip,
                } = err
                {
                    return Ok(ExecutionResult::Suspended {
                        future_id,
                        resume_ip,
                    });
                }

                // Check for state.resume() request
                if matches!(err, VMError::ResumeRequested) {
                    self.apply_pending_resume()?;
                    continue;
                }

                if !self.exception_handlers.is_empty() {
                    let error_nb = ValueWord::from_string(Arc::new(err.to_string()));
                    self.handle_exception_nb(error_nb)?;
                } else {
                    // Enrich error with source location before returning
                    return Err(self.enrich_error_with_location(err, error_ip));
                }
            }

            // Check for pending frame resume (from state.resume_frame)
            if self.pending_frame_resume.is_some() {
                self.apply_pending_frame_resume()?;
            }

            // Check for halt
            if matches!(instruction.opcode, OpCode::Halt) {
                break;
            }
        }

        // Return top of stack or none (only if sp is above top-level locals region)
        let tl = self.program.top_level_locals_count as usize;
        Ok(ExecutionResult::Completed(if self.sp > tl {
            self.sp -= 1;
            self.stack_take_raw(self.sp)
        } else {
            ValueWord::none()
        }))
    }

    /// Fast execution loop: no debugger/trace checks, but full exception handling
    /// and halt/suspension support. This is the default hot path for production code.
    fn execute_fast_with_exceptions(
        &mut self,
        mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
    ) -> Result<ExecutionResult, VMError> {
        while self.ip < self.program.instructions.len() {
            let ip = self.ip;
            self.ip += 1;
            self.instruction_count += 1;

            // Check for Ctrl+C interrupt every 1024 instructions
            if self.instruction_count & 0x3FF == 0 && self.interrupt.load(Ordering::Relaxed) > 0 {
                return Err(VMError::Interrupted);
            }

            // Poll for completed tier promotions every 1024 instructions.
            if self.instruction_count & 0x3FF == 0 {
                self.poll_tier_completions();
            }

            // GC safepoint poll (gc feature only)
            #[cfg(feature = "gc")]
            if self.instruction_count & 0x3FF == 0 {
                self.gc_safepoint_poll();

                // Incremental marking: make bounded progress on the gray worklist
                // when a marking cycle is active, without stopping the world.
                if self.gc_heap.as_ref().map_or(false, |h| h.is_marking()) {
                    self.gc_incremental_mark_step();
                }
            }

            let instruction = self.program.instructions[ip];

            // Record instruction in metrics (opt-in, near-zero cost when None)
            if let Some(ref mut metrics) = self.metrics {
                metrics.record_instruction();
            }

            // Time-travel capture check (cheap: just a mode check + counter).
            if let Some(ref mut tt) = self.time_travel {
                let is_call_or_return = matches!(
                    instruction.opcode,
                    OpCode::Call | OpCode::CallValue | OpCode::Return | OpCode::ReturnValue
                );
                if tt.should_capture(ip, self.instruction_count as u64, is_call_or_return) {
                    if let Ok(store) = tt.snapshot_store() {
                        let store_ptr = store as *const shape_runtime::snapshot::SnapshotStore;
                        if let Ok(snap) = self.snapshot(unsafe { &*store_ptr }) {
                            let call_depth = self.call_stack.len();
                            self.time_travel.as_mut().unwrap().record(
                                snap,
                                ip,
                                self.instruction_count as u64,
                                call_depth,
                            );
                        }
                    }
                }
            }

            if let Err(err) = self.execute_instruction(&instruction, ctx.as_deref_mut()) {
                // Check for suspension (not a real error)
                if let VMError::Suspended {
                    future_id,
                    resume_ip,
                } = err
                {
                    return Ok(ExecutionResult::Suspended {
                        future_id,
                        resume_ip,
                    });
                }

                // Check for state.resume() request
                if matches!(err, VMError::ResumeRequested) {
                    self.apply_pending_resume()?;
                    continue;
                }

                if !self.exception_handlers.is_empty() {
                    let error_nb = ValueWord::from_string(Arc::new(err.to_string()));
                    self.handle_exception_nb(error_nb)?;
                } else {
                    return Err(self.enrich_error_with_location(err, ip));
                }
            }

            // Check for pending frame resume (from state.resume_frame)
            if self.pending_frame_resume.is_some() {
                self.apply_pending_frame_resume()?;
            }

            if matches!(instruction.opcode, OpCode::Halt) {
                break;
            }
        }

        let tl = self.program.top_level_locals_count as usize;
        Ok(ExecutionResult::Completed(if self.sp > tl {
            self.sp -= 1;
            self.stack_take_raw(self.sp)
        } else {
            ValueWord::none()
        }))
    }

    /// Fast execution loop without debugging overhead or exception handling.
    /// Used for hot inner loops (e.g., function calls) where we need maximum performance
    /// and exceptions propagate via `?`.
    #[inline]
    pub(crate) fn execute_fast(
        &mut self,
        mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
    ) -> Result<ValueWord, VMError> {
        while self.ip < self.program.instructions.len() {
            // Get index first, then increment
            let ip = self.ip;
            self.ip += 1;
            self.instruction_count += 1;

            // Check for Ctrl+C interrupt every 1024 instructions
            if self.instruction_count & 0x3FF == 0 && self.interrupt.load(Ordering::Relaxed) > 0 {
                return Err(VMError::Interrupted);
            }

            // GC safepoint poll (gc feature only)
            #[cfg(feature = "gc")]
            if self.instruction_count & 0x3FF == 0 {
                self.gc_safepoint_poll();

                // Incremental marking: make bounded progress on the gray worklist
                // when a marking cycle is active, without stopping the world.
                if self.gc_heap.as_ref().map_or(false, |h| h.is_marking()) {
                    self.gc_incremental_mark_step();
                }
            }

            let instruction = self.program.instructions[ip];

            // Record instruction in metrics (opt-in, near-zero cost when None)
            if let Some(ref mut metrics) = self.metrics {
                metrics.record_instruction();
            }

            self.execute_instruction(&instruction, ctx.as_deref_mut())?;

            if matches!(instruction.opcode, OpCode::Halt) {
                break;
            }
        }

        let tl = self.program.top_level_locals_count as usize;
        Ok(if self.sp > tl {
            self.sp -= 1;
            self.stack_take_raw(self.sp)
        } else {
            ValueWord::none()
        })
    }

    pub(crate) fn execute_until_call_depth(
        &mut self,
        target_depth: usize,
        mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
    ) -> Result<(), VMError> {
        loop {
            if self.ip >= self.program.instructions.len() {
                break;
            }

            let instruction = self.program.instructions[self.ip];
            self.ip += 1;
            self.instruction_count += 1;

            match self.execute_instruction(&instruction, ctx.as_deref_mut()) {
                Ok(()) => {}
                Err(VMError::ResumeRequested) => {
                    self.apply_pending_resume()?;
                    continue;
                }
                Err(err) => return Err(err),
            }

            if self.pending_frame_resume.is_some() {
                self.apply_pending_frame_resume()?;
            }

            if matches!(instruction.opcode, OpCode::Halt) || self.call_stack.len() == target_depth {
                break;
            }
        }
        Ok(())
    }

    /// Execute a single instruction
    pub(crate) fn execute_instruction(
        &mut self,
        instruction: &Instruction,
        ctx: Option<&mut shape_runtime::context::ExecutionContext>,
    ) -> Result<(), VMError> {
        use OpCode::*;

        match instruction.opcode {
            // Stack operations
            PushConst | PushNull | Pop | Dup | Swap | PromoteToOwned => {
                return self.exec_stack_ops(instruction);
            }

            // Arithmetic (dynamic runtime dispatch -- types unresolvable at compile time)
            AddDynamic | SubDynamic | MulDynamic | DivDynamic | ModDynamic | PowDynamic
            | BitAnd | BitOr | BitXor | BitShl | BitShr | BitNot => {
                return self.exec_arithmetic(instruction);
            }

            // Typed arithmetic (compiler-guaranteed types, zero dispatch)
            AddInt | AddNumber | AddDecimal | SubInt | SubNumber | SubDecimal | MulInt
            | MulNumber | MulDecimal | DivInt | DivNumber | DivDecimal | ModInt | ModNumber
            | ModDecimal | PowInt | PowNumber | PowDecimal | IntToNumber | NumberToInt
            | NegInt | NegNumber | NegDecimal => {
                return self.exec_typed_arithmetic(instruction);
            }

            // NOTE: Trusted arithmetic/comparison opcodes removed — the typed
            // variants (AddInt, GtInt, etc.) already provide zero-dispatch execution.

            // Compact typed arithmetic (width-parameterised, ABI-stable)
            AddTyped | SubTyped | MulTyped | DivTyped | ModTyped | CmpTyped => {
                return self.exec_compact_typed_arithmetic(instruction);
            }

            // CastWidth: integer width casting (bit truncation)
            CastWidth => {
                return self.op_cast_width(instruction);
            }

            // Comparison (dynamic runtime dispatch -- types unresolvable at compile time)
            GtDynamic | LtDynamic | GteDynamic | LteDynamic | EqDynamic | NeqDynamic => {
                return self.exec_comparison(instruction);
            }

            // Typed comparison (compiler-guaranteed types, zero dispatch)
            GtInt | GtNumber | GtDecimal | LtInt | LtNumber | LtDecimal | GteInt | GteNumber
            | GteDecimal | LteInt | LteNumber | LteDecimal | EqInt | EqNumber | NeqInt
            | NeqNumber | EqString | EqDecimal | IsNull | GtString | LtString | GteString
            | LteString => {
                return self.exec_typed_comparison(instruction);
            }

            // Logical
            And | Or | Not => {
                return self.exec_logical(instruction);
            }

            // Control flow
            Jump | JumpIfFalse | JumpIfTrue | JumpIfFalseTrusted | Call | CallValue
            | CallForeign | Return | ReturnValue => {
                return self.exec_control_flow(instruction);
            }

            // Variables (including reference operations)
            LoadLocal
            | LoadLocalTrusted
            | LoadLocalMove
            | LoadLocalClone
            | StoreLocal
            | StoreLocalTyped
            | StoreLocalDrop
            | LoadModuleBinding
            | StoreModuleBinding
            | StoreModuleBindingTyped
            | LoadClosure
            | StoreClosure
            | CloseUpvalue
            | MakeRef
            | MakeFieldRef
            | MakeIndexRef
            | DerefLoad
            | DerefStore
            | SetIndexRef
            | BoxLocal
            | BoxModuleBinding => {
                return self.exec_variables(instruction);
            }

            // Objects/Arrays
            NewArray
            | NewMatrix
            | NewObject
            | GetProp
            | SetProp
            | SetLocalIndex
            | SetModuleBindingIndex
            | Length
            | ArrayPush
            | ArrayPushLocal
            | ArrayPop
            | MakeClosure
            | MergeObject
            | NewTypedObject
            | NewTypedArray
            | TypedMergeObject
            | WrapTypeAnnotation => {
                return self.exec_objects(instruction, ctx);
            }

            // Built-in functions
            BuiltinCall | TypeCheck | Convert => {
                return self.exec_builtins(instruction, ctx);
            }

            // Typed conversion opcodes (zero-dispatch, no operand)
            ConvertToInt => return self.op_convert_to_int(),
            ConvertToNumber => return self.op_convert_to_number(),
            ConvertToString => return self.op_convert_to_string(),
            ConvertToBool => return self.op_convert_to_bool(),
            ConvertToDecimal => return self.op_convert_to_decimal(),
            ConvertToChar => return self.op_convert_to_char(),
            TryConvertToInt => return self.op_try_convert_to_int(),
            TryConvertToNumber => return self.op_try_convert_to_number(),
            TryConvertToString => return self.op_try_convert_to_string(),
            TryConvertToBool => return self.op_try_convert_to_bool(),
            TryConvertToDecimal => return self.op_try_convert_to_decimal(),
            TryConvertToChar => return self.op_try_convert_to_char(),

            // Exception handling
            SetupTry | PopHandler | Throw | TryUnwrap | UnwrapOption | ErrorContext | IsOk
            | IsErr | UnwrapOk | UnwrapErr => {
                return self.exec_exceptions(instruction);
            }

            // Additional operations
            SliceAccess | NullCoalesce | MakeRange => {
                return self.exec_additional(instruction);
            }

            // Loop control
            LoopStart | LoopEnd | Break | Continue | IterNext | IterDone => {
                return self.exec_loops(instruction);
            }

            // Method calls on values
            CallMethod => {
                return self.op_call_method(instruction, ctx);
            }

            // Dedicated concatenation opcodes (Phase 2.3 / 2.4): replace the
            // generic Add overload for built-in heap types whose operand types
            // the compiler can prove statically.
            StringConcat => {
                return self.op_string_concat();
            }
            ArrayConcat => {
                return self.op_array_concat();
            }

            PushTimeframe => {
                return Err(VMError::NotImplemented(
                    "Opcode 'PushTimeframe' is reserved but not yet implemented".into(),
                ));
            }
            PopTimeframe => {
                return Err(VMError::NotImplemented(
                    "Opcode 'PopTimeframe' is reserved but not yet implemented".into(),
                ));
            }

            // Typed column access on RowView values
            LoadColF64 | LoadColI64 | LoadColBool | LoadColStr => {
                return self.exec_load_col(instruction);
            }

            // Bind DataTable to TypeSchema (runtime safety net)
            BindSchema => {
                return self.exec_bind_schema(instruction);
            }

            // Type-specialized operations (JIT optimization)
            GetFieldTyped | SetFieldTyped => {
                return self.exec_jit_ops(instruction);
            }

            // v2 typed struct field operations
            FieldLoadF64 | FieldLoadI64 | FieldLoadI32 | FieldLoadBool | FieldLoadPtr
            | FieldStoreF64 | FieldStoreI64 | FieldStoreI32 | NewTypedStruct => {
                return self.exec_v2_typed_field(instruction);
            }

            // v2 sized integer (i32) arithmetic and comparison
            AddI32 | SubI32 | MulI32 | DivI32 | ModI32 | EqI32 | NeqI32 | LtI32 | GtI32
            | LteI32 | GteI32 => {
                return self.exec_v2_sized_int(instruction);
            }

            // Async operations
            Yield | Suspend | Resume | Poll | AwaitBar | AwaitTick | EmitAlert | EmitEvent
            | Await | SpawnTask | JoinInit | JoinAwait | CancelTask | AsyncScopeEnter
            | AsyncScopeExit => {
                match self.exec_async_op(instruction) {
                    Ok(async_ops::AsyncExecutionResult::Continue) => return Ok(()),
                    Ok(async_ops::AsyncExecutionResult::Yielded) => {
                        return Ok(());
                    }
                    Ok(async_ops::AsyncExecutionResult::Suspended(info)) => {
                        // Propagate suspension as VMError::Suspended so execute() can catch it
                        match info.wait_type {
                            async_ops::WaitType::Future { id } => {
                                return Err(VMError::Suspended {
                                    future_id: id,
                                    resume_ip: info.resume_ip,
                                });
                            }
                            async_ops::WaitType::TaskGroup { kind, task_ids } => {
                                // TaskGroup suspension: propagate with first task_id as marker
                                // The host resolves the group based on kind + task_ids
                                let marker_id = task_ids.first().copied().unwrap_or(0);
                                let _ = (kind, task_ids); // Host retrieves from SuspensionInfo
                                return Err(VMError::Suspended {
                                    future_id: marker_id,
                                    resume_ip: info.resume_ip,
                                });
                            }
                            _ => {
                                // Non-future suspensions (NextBar, Timer, AnyEvent) cannot be
                                // resumed by the host via future_id. Drain any open async scopes
                                // to prevent leaked task tracking, then continue execution.
                                while let Some(mut scope_tasks) = self.async_scope_stack.pop() {
                                    scope_tasks.reverse();
                                    for task_id in scope_tasks {
                                        self.task_scheduler.cancel(task_id);
                                    }
                                }
                                return Ok(());
                            }
                        }
                    }
                    Err(e) => return Err(e),
                }
            }

            // Trait object operations
            BoxTraitObject | DynMethodCall | DropCall | DropCallAsync => {
                return self.exec_trait_object_ops(instruction, ctx);
            }

            // v2 typed array operations
            NewTypedArrayF64
            | NewTypedArrayI64
            | NewTypedArrayI32
            | NewTypedArrayBool
            | TypedArrayGetF64
            | TypedArrayGetI64
            | TypedArrayGetI32
            | TypedArrayGetBool
            | TypedArraySetF64
            | TypedArraySetI64
            | TypedArraySetI32
            | TypedArraySetBool
            | TypedArrayPushF64
            | TypedArrayPushI64
            | TypedArrayPushI32
            | TypedArrayPushBool
            | TypedArrayLen => {
                return self.exec_v2_typed_array(instruction);
            }

            // v2 typed map operations
            NewTypedMapStringF64
            | NewTypedMapStringI64
            | NewTypedMapStringPtr
            | NewTypedMapI64F64
            | NewTypedMapI64I64
            | NewTypedMapI64Ptr
            | TypedMapStringF64Get
            | TypedMapStringI64Get
            | TypedMapStringPtrGet
            | TypedMapI64F64Get
            | TypedMapI64I64Get
            | TypedMapI64PtrGet
            | TypedMapStringF64Set
            | TypedMapStringI64Set
            | TypedMapStringPtrSet
            | TypedMapI64F64Set
            | TypedMapI64I64Set
            | TypedMapI64PtrSet
            | TypedMapStringF64Has
            | TypedMapStringI64Has
            | TypedMapStringPtrHas
            | TypedMapI64F64Has
            | TypedMapI64I64Has
            | TypedMapI64PtrHas
            | TypedMapStringF64Delete
            | TypedMapStringI64Delete
            | TypedMapStringPtrDelete
            | TypedMapI64F64Delete
            | TypedMapI64I64Delete
            | TypedMapI64PtrDelete => {
                return self.exec_v2_typed_map(instruction);
            }

            // Special
            Nop => {}
            Halt => {}
            // Stage 2.6.5.0: Debug opcode removed; slot 0xF2 reused for IsNull.
            // No compiler ever emitted Debug — it was a stale runtime hook.

            _ => return Err(VMError::InvalidOperand),
        }

        Ok(())
    }

    /// Enrich an error with source location context
    ///
    /// Uses debug_info from the program to add line numbers and source context
    /// to the error message for better debugging.
    pub(crate) fn enrich_error_with_location(&mut self, error: VMError, ip: usize) -> VMError {
        let debug_info = &self.program.debug_info;

        // Try to get line number and file for this instruction
        let location = debug_info.get_location_for_instruction(ip);

        if let Some((file_id, line_num)) = location {
            // Store the line number and file for LSP integration
            self.last_error_line = Some(line_num);
            self.last_error_file = debug_info
                .source_map
                .get_file(file_id)
                .map(|s| s.to_string());

            // Try to get the source line from the correct file
            let source_context = debug_info
                .get_source_line_from_file(file_id, line_num as usize)
                .map(|s| s.trim())
                .filter(|s| !s.is_empty());

            let base_msg = match &error {
                VMError::RuntimeError(msg) => msg.clone(),
                VMError::TypeError { expected, got } => {
                    format!("TypeError: expected {}, got {}", expected, got)
                }
                VMError::StackUnderflow => "Stack underflow".to_string(),
                VMError::StackOverflow => "Stack overflow".to_string(),
                VMError::DivisionByZero => "Division by zero".to_string(),
                VMError::UndefinedVariable(name) => format!("Undefined variable: {}", name),
                VMError::UndefinedProperty(name) => format!("Undefined property: {}", name),
                VMError::InvalidCall => "Invalid function call".to_string(),
                VMError::IndexOutOfBounds { index, length } => {
                    format!("Index {} out of bounds (length {})", index, length)
                }
                VMError::InvalidOperand => "Invalid operand".to_string(),
                VMError::ArityMismatch {
                    function,
                    expected,
                    got,
                } => {
                    format!(
                        "{}() expects {} argument(s), got {}",
                        function, expected, got
                    )
                }
                VMError::InvalidArgument { function, message } => {
                    format!("{}(): {}", function, message)
                }
                VMError::NotImplemented(feature) => format!("Not implemented: {}", feature),
                VMError::Suspended { .. } | VMError::Interrupted | VMError::ResumeRequested => {
                    return error;
                } // Don't enrich suspension/interrupt/resume signals
            };

            // Build enhanced error message with source context
            let enhanced = if let Some(source) = source_context {
                format!(
                    "{}\n  --> line {}\n   |\n{:>3} | {}\n   |",
                    base_msg, line_num, line_num, source
                )
            } else {
                format!("{} (line {})", base_msg, line_num)
            };

            VMError::RuntimeError(enhanced)
        } else {
            self.last_error_line = None;
            self.last_error_file = None;
            error
        }
    }

    // apply_pending_resume() and apply_pending_frame_resume() moved to resume.rs
}
