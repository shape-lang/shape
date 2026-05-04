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
    ///
    /// Returns a `ValueWord` representing the program's final value. Wave E+4
    /// flips top-level emission to typed opcodes that push raw native bits
    /// (no NaN-box tag) onto the stack. To keep the host boundary stable,
    /// `execute()` synthesises a tagged `ValueWord` from those raw bits per
    /// the program's declared top-level return kind (read from
    /// `BytecodeProgram::top_level_frame.return_kind` when present and not
    /// `Unknown`). When the kind is unknown — the legacy / pre-E+4
    /// situation — the raw bits are interpreted directly as a tagged
    /// `ValueWord` (passthrough), preserving the historical behaviour.
    ///
    /// Hosts that want raw bits should call [`Self::execute_raw`] instead
    /// and synthesize a `ValueWord` themselves (see
    /// `crate::test_utils::eval_with_kind` for an example).
    pub fn execute(
        &mut self,
        ctx: Option<&mut shape_runtime::context::ExecutionContext>,
    ) -> Result<ValueWord, VMError> {
        let bits = self.execute_raw(ctx)?;
        // Read the return kind AFTER execution so the runtime-observed
        // `last_program_return_kind` (set by typed `op_return_value_<kind>`
        // handlers landing at the top-level frame) is available.
        let return_kind = self.program_top_level_return_kind();
        Ok(synthesize_value_word_from_raw(bits, return_kind))
    }

    /// Execute the loaded program and return the raw u64 bits at the top of
    /// stack. After Wave E+4 flips top-level emission, those bits may be
    /// raw native values (e.g. `i64`, `f64::to_bits()`, `0u64`/`1u64` for
    /// bool, raw heap pointer for ptr) rather than tagged `ValueWord` bits.
    /// Use this when the host wants to control kind-hint synthesis itself,
    /// or when interpreting the bits as a non-default kind.
    pub fn execute_raw(
        &mut self,
        ctx: Option<&mut shape_runtime::context::ExecutionContext>,
    ) -> Result<u64, VMError> {
        match self.execute_with_suspend(ctx)? {
            ExecutionResult::Completed(value) => Ok(value.into_raw_bits()),
            ExecutionResult::Suspended { future_id, .. } => Err(VMError::Suspended {
                future_id,
                resume_ip: 0,
            }),
        }
    }

    /// Read the program's declared top-level return kind, if present.
    ///
    /// Returns `Some(kind)` when `top_level_frame.return_kind` is set to a
    /// concrete `SlotKind` (i.e. the compiler proved a return type for the
    /// top-level program). Returns `None` for the legacy / unproven case,
    /// signalling that raw bits should be passed through as a
    /// `ValueWord` directly.
    #[inline]
    fn program_top_level_return_kind(&self) -> Option<crate::type_tracking::SlotKind> {
        // Prefer the runtime-observed kind from the most-recent typed
        // `op_return_value_<kind>` that landed at the top-level boundary
        // (see `last_program_return_kind` doc on `VirtualMachine`). This
        // covers the polymorphic `let g = make(); g(arg)` case where the
        // compiler can't statically prove the kind but the closure body's
        // typed `ReturnValueI64`/`F64`/`Bool` did push native bits.
        if let Some(kind) = self.last_program_return_kind {
            return Some(kind);
        }
        let kind = self.program.top_level_frame.as_ref()?.return_kind;
        match kind {
            crate::type_tracking::SlotKind::Unknown => None,
            _ => Some(kind),
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
        // Install this VM's ShapeTableHandle as the ambient current
        // shape table for the duration of this execution. Mirrors B1's
        // pattern for TypeSchemaRegistry. The guard restores any outer
        // scope (e.g. a host-installed async scope) on drop, so nested
        // or re-entrant VM execution composes correctly.
        let _shape_scope =
            shape_value::SyncShapeTableScope::enter(self.shape_table.clone());

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
                    OpCode::Call
                        | OpCode::CallValue
                        | OpCode::CallClosure
                        | OpCode::CallFunctionIndirect
                        | OpCode::Return
                        | OpCode::ReturnValue
                        | OpCode::ReturnValueI64
                        | OpCode::ReturnValueU64
                        | OpCode::ReturnValueF64
                        | OpCode::ReturnValueI32
                        | OpCode::ReturnValueU32
                        | OpCode::ReturnValueI16
                        | OpCode::ReturnValueU16
                        | OpCode::ReturnValueI8
                        | OpCode::ReturnValueU8
                        | OpCode::ReturnValueBool
                        | OpCode::ReturnValuePtr
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
                    OpCode::Call
                        | OpCode::CallValue
                        | OpCode::CallClosure
                        | OpCode::CallFunctionIndirect
                        | OpCode::Return
                        | OpCode::ReturnValue
                        | OpCode::ReturnValueI64
                        | OpCode::ReturnValueU64
                        | OpCode::ReturnValueF64
                        | OpCode::ReturnValueI32
                        | OpCode::ReturnValueU32
                        | OpCode::ReturnValueI16
                        | OpCode::ReturnValueU16
                        | OpCode::ReturnValueI8
                        | OpCode::ReturnValueU8
                        | OpCode::ReturnValueBool
                        | OpCode::ReturnValuePtr
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
            PushConst | PushNull | Pop | Dup | Swap | PromoteToOwned | ReturnOwned
            | PromoteToShared => {
                return self.exec_stack_ops(instruction);
            }

            // Bitwise dynamic ops (int-typed bitwise still routes here when operand
            // types aren't proven at compile time). The strict-typing sweep
            // (Phase 1+2) deleted the `*Dynamic` arithmetic/comparison opcodes;
            // the bitwise variants remain because typed `BitAndInt`/etc. only
            // fire when both operands are proven `int`.
            BitAnd | BitOr | BitXor | BitShl | BitShr | BitNot => {
                return self.exec_dyn_bit_dispatch(instruction);
            }

            // Typed arithmetic (compiler-guaranteed types, zero dispatch).
            //
            // R5.1B adds the six int-typed bitwise opcodes to this arm.
            // They are structurally identical to the other typed int ops
            // (raw i48-tagged operand slots, zero dispatch) and therefore
            // share the same exec_typed_arithmetic handler.
            AddInt | AddNumber | AddDecimal | SubInt | SubNumber | SubDecimal | MulInt
            | MulNumber | MulDecimal | DivInt | DivNumber | DivDecimal | ModInt | ModNumber
            | ModDecimal | PowInt | PowNumber | PowDecimal | IntToNumber | NumberToInt
            | NegInt | NegNumber | NegDecimal | BitAndInt | BitOrInt | BitXorInt
            | BitShlInt | BitShrInt | BitNotInt => {
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
            | CallClosure | CallFunctionIndirect | CallForeign | Return | ReturnValue
            | ReturnValueI64 | ReturnValueU64 | ReturnValueF64 | ReturnValueI32
            | ReturnValueU32 | ReturnValueI16 | ReturnValueU16 | ReturnValueI8
            | ReturnValueU8 | ReturnValueBool | ReturnValuePtr => {
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
            | LoadLocalI64
            | LoadLocalU64
            | LoadLocalF64
            | LoadLocalI32
            | LoadLocalU32
            | LoadLocalI16
            | LoadLocalU16
            | LoadLocalI8
            | LoadLocalU8
            | LoadLocalBool
            | LoadLocalPtr
            | StoreLocalI64
            | StoreLocalU64
            | StoreLocalF64
            | StoreLocalI32
            | StoreLocalU32
            | StoreLocalI16
            | StoreLocalU16
            | StoreLocalI8
            | StoreLocalU8
            | StoreLocalBool
            | StoreLocalPtr
            | LoadModuleBinding
            | StoreModuleBinding
            | StoreModuleBindingTyped
            | LoadModuleBindingI64
            | LoadModuleBindingU64
            | LoadModuleBindingF64
            | LoadModuleBindingI32
            | LoadModuleBindingU32
            | LoadModuleBindingI16
            | LoadModuleBindingU16
            | LoadModuleBindingI8
            | LoadModuleBindingU8
            | LoadModuleBindingBool
            | LoadModuleBindingPtr
            | StoreModuleBindingI64
            | StoreModuleBindingU64
            | StoreModuleBindingF64
            | StoreModuleBindingI32
            | StoreModuleBindingU32
            | StoreModuleBindingI16
            | StoreModuleBindingU16
            | StoreModuleBindingI8
            | StoreModuleBindingU8
            | StoreModuleBindingBool
            | StoreModuleBindingPtr
            | LoadClosure
            | StoreClosure
            | CloseUpvalue
            | MakeRef
            | MakeFieldRef
            | MakeIndexRef
            | DerefLoad
            | DerefStore
            | SetIndexRef
            | LoadOwnedMutableCapture
            | StoreOwnedMutableCapture
            | LoadOwnedMutableCaptureI64
            | LoadOwnedMutableCaptureU64
            | LoadOwnedMutableCaptureF64
            | LoadOwnedMutableCaptureI32
            | LoadOwnedMutableCaptureU32
            | LoadOwnedMutableCaptureI16
            | LoadOwnedMutableCaptureU16
            | LoadOwnedMutableCaptureI8
            | LoadOwnedMutableCaptureU8
            | LoadOwnedMutableCaptureBool
            | LoadOwnedMutableCapturePtr
            | StoreOwnedMutableCaptureI64
            | StoreOwnedMutableCaptureU64
            | StoreOwnedMutableCaptureF64
            | StoreOwnedMutableCaptureI32
            | StoreOwnedMutableCaptureU32
            | StoreOwnedMutableCaptureI16
            | StoreOwnedMutableCaptureU16
            | StoreOwnedMutableCaptureI8
            | StoreOwnedMutableCaptureU8
            | StoreOwnedMutableCaptureBool
            | StoreOwnedMutableCapturePtr
            | LoadSharedCapture
            | StoreSharedCapture
            | LoadSharedCaptureI64
            | LoadSharedCaptureU64
            | LoadSharedCaptureF64
            | LoadSharedCaptureI32
            | LoadSharedCaptureU32
            | LoadSharedCaptureI16
            | LoadSharedCaptureU16
            | LoadSharedCaptureI8
            | LoadSharedCaptureU8
            | LoadSharedCaptureBool
            | LoadSharedCapturePtr
            | StoreSharedCaptureI64
            | StoreSharedCaptureU64
            | StoreSharedCaptureF64
            | StoreSharedCaptureI32
            | StoreSharedCaptureU32
            | StoreSharedCaptureI16
            | StoreSharedCaptureU16
            | StoreSharedCaptureI8
            | StoreSharedCaptureU8
            | StoreSharedCaptureBool
            | StoreSharedCapturePtr
            | AllocSharedLocal
            | LoadSharedLocal
            | StoreSharedLocal
            | DropSharedLocal
            | AllocSharedModuleBinding
            | LoadSharedModuleBinding
            | StoreSharedModuleBinding => {
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

            // Typed array element access (local-slot based, skip HeapValue dispatch)
            GetElemI64 | GetElemF64 | SetElemI64 | SetElemF64
            | ArrayPushI64 | ArrayPushF64 | ArrayLenTyped => {
                return self.exec_typed_array_elem_ops(instruction);
            }

            // Typed HashMap access (local-slot based, skip HeapValue dispatch)
            MapGetStrI64 | MapGetStrF64 | MapSetStrI64 | MapHasStr | MapLenTyped => {
                return self.exec_typed_map_access(instruction);
            }

            // Typed String access (local-slot based or stack-based).
            //
            // R5.5 adds the three typed string+scalar concat opcodes to this
            // arm; they share the `exec_typed_string_access` handler since
            // they are structurally identical to `StringConcatTyped` (stack-
            // based, single heap allocation, no local-slot operand).
            StringLenTyped | StringCharAt | StringConcatTyped
            | StringConcatInt | StringConcatNumber | StringConcatBool => {
                return self.exec_typed_string_access(instruction);
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

            // V1.1B: ownership-aware local opcodes.
            //
            // Phase 1 of `docs/ownership-aware-runtime-v2.md`: these
            // handlers read the local slot bits directly and delegate
            // refcount adjustment to `raw_helpers::{clone,drop}_raw_bits`.
            // The compiler does not yet emit them — V1.1C adds emission
            // behind the `SHAPE_V2_OWNERSHIP_MOVES` flag — so at this
            // stage only hand-crafted bytecode exercises these arms.
            MoveLocal => self.op_move_local(instruction)?,
            CloneLocal => self.op_clone_local(instruction)?,
            DropLocal => self.op_drop_local(instruction)?,

            // V1.2B: `PromoteToShared` is dispatched above through the
            // Stack-category arm alongside `PromoteToOwned`; no separate
            // arm is needed here. V1.2C will add compiler emission.

            // R5.1B: typed bitwise opcodes (BitAndInt/BitOrInt/BitXorInt/
            // BitShlInt/BitShrInt/BitNotInt) are dispatched above via the
            // typed-arithmetic arm alongside the other typed int ops; no
            // separate arm is needed here. R5.1C will add compiler
            // emission behind `SHAPE_V2_TYPED_BITWISE=1`.

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

/// Synthesize a tagged `ValueWord` from raw native bits per the
/// supplied `SlotKind`. Used by [`VirtualMachine::execute`] to bridge the
/// host boundary after Wave E+4 flips top-level emission to typed opcodes
/// that leave raw bits on the stack.
///
/// When `kind` is `None`, the bits are returned unchanged — i.e. they are
/// already a tagged `ValueWord`. This is the pre-E+4 / unproven-type path,
/// matching the historical behaviour of `execute()`.
///
/// The encoding mirrors `unmarshal_jit_result` in `jit_abi.rs` (which
/// targets the JIT call boundary). Both kinds of boundary need the same
/// bits → `ValueWord` synthesis; we do not share the function only because
/// `unmarshal_jit_result` is gated behind the `jit` feature today and
/// shape-vm's host-side `execute()` must work without it.
#[inline]
pub(crate) fn synthesize_value_word_from_raw(
    bits: u64,
    kind: Option<crate::type_tracking::SlotKind>,
) -> ValueWord {
    use crate::type_tracking::SlotKind;
    let Some(kind) = kind else {
        return ValueWord::from_raw_bits(bits);
    };
    match kind {
        // Signed / sub-i64 ints all flow through the i48 inline path.
        SlotKind::Int8
        | SlotKind::NullableInt8
        | SlotKind::Int16
        | SlotKind::NullableInt16
        | SlotKind::Int32
        | SlotKind::NullableInt32
        | SlotKind::Int64
        | SlotKind::NullableInt64
        | SlotKind::IntSize
        | SlotKind::NullableIntSize => ValueWord::from_i64(bits as i64),

        // Unsigned sub-i64 ints fit in i64.
        SlotKind::UInt8
        | SlotKind::NullableUInt8
        | SlotKind::UInt16
        | SlotKind::NullableUInt16
        | SlotKind::UInt32
        | SlotKind::NullableUInt32
        | SlotKind::UIntSize
        | SlotKind::NullableUIntSize => ValueWord::from_i64(bits as i64),

        // U64 may exceed i64::MAX — promote to native u64 heap encoding.
        SlotKind::UInt64 | SlotKind::NullableUInt64 => {
            if bits <= i64::MAX as u64 {
                ValueWord::from_i64(bits as i64)
            } else {
                ValueWord::from_native_u64(bits)
            }
        }

        // Float64: NaN-boxed encoding (re-tag via from_f64 to ensure the
        // result respects the canonical NaN-box representation; the bits
        // arriving from a typed `ReturnValueF64` are an `f64::to_bits()`
        // payload, which is exactly what `from_f64` expects after a round
        // trip through `f64::from_bits`).
        SlotKind::Float64 | SlotKind::NullableFloat64 => {
            ValueWord::from_f64(f64::from_bits(bits))
        }

        // Bool: 0 → false, anything else → true.
        SlotKind::Bool => ValueWord::from_bool(bits != 0),

        // String / Dynamic / Unknown: passthrough — bits already encode
        // a tagged ValueWord (heap pointer, NaN-box, etc.).
        SlotKind::String | SlotKind::Dynamic | SlotKind::Unknown => {
            ValueWord::from_raw_bits(bits)
        }
    }
}

#[cfg(test)]
mod execute_raw_tests {
    use super::*;
    use crate::compiler::BytecodeCompiler;
    use crate::executor::VMConfig;
    use crate::type_tracking::SlotKind;

    /// Today, no top-level program declares a typed return; verify that
    /// `execute()` and `execute_raw()` agree on the result bits and that
    /// the synthesizer falls through to passthrough.
    #[test]
    fn execute_raw_matches_execute_for_legacy_program() {
        let program = shape_ast::parser::parse_program("42").expect("parse");
        let compiler = BytecodeCompiler::new();
        let bytecode = compiler.compile(&program).expect("compile");

        // First run: tagged ValueWord via execute()
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(bytecode.clone());
        let tagged = vm.execute(None).expect("execute");

        // Second run: raw bits via execute_raw()
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(bytecode);
        let raw = vm.execute_raw(None).expect("execute_raw");

        assert_eq!(
            tagged.into_raw_bits(),
            raw,
            "execute and execute_raw disagree on bits for legacy program"
        );
    }

    #[test]
    fn synthesize_int64_from_raw_bits() {
        let raw = 12345i64 as u64;
        let vw = synthesize_value_word_from_raw(raw, Some(SlotKind::Int64));
        assert_eq!(vw.as_i64(), Some(12345));
    }

    #[test]
    fn synthesize_int64_negative() {
        let raw = (-7i64) as u64;
        let vw = synthesize_value_word_from_raw(raw, Some(SlotKind::Int64));
        assert_eq!(vw.as_i64(), Some(-7));
    }

    #[test]
    fn synthesize_float64_from_raw_bits() {
        let raw = 3.14159f64.to_bits();
        let vw = synthesize_value_word_from_raw(raw, Some(SlotKind::Float64));
        assert_eq!(vw.as_f64(), Some(3.14159));
    }

    #[test]
    fn synthesize_bool_true() {
        let vw = synthesize_value_word_from_raw(1, Some(SlotKind::Bool));
        assert_eq!(vw.as_bool(), Some(true));
    }

    #[test]
    fn synthesize_bool_false() {
        let vw = synthesize_value_word_from_raw(0, Some(SlotKind::Bool));
        assert_eq!(vw.as_bool(), Some(false));
    }

    #[test]
    fn synthesize_passthrough_when_kind_is_none() {
        let original = ValueWord::from_f64(2.5);
        let vw = synthesize_value_word_from_raw(original.into_raw_bits(), None);
        assert_eq!(vw.as_f64(), Some(2.5));
    }

    #[test]
    fn synthesize_passthrough_when_kind_is_unknown_via_dynamic() {
        let original = ValueWord::from_bool(true);
        let vw = synthesize_value_word_from_raw(original.into_raw_bits(), Some(SlotKind::Dynamic));
        assert_eq!(vw.as_bool(), Some(true));
    }
}
