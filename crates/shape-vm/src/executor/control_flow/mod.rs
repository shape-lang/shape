//! Control flow operations for the VM executor
//!
//! Handles: Jump, JumpIfFalse, JumpIfTrue, Call, CallValue, CallForeign, Return, ReturnValue

pub mod foreign_marshal;
pub mod jit_abi;
pub mod native_abi;

use crate::executor::objects::raw_helpers;
use crate::{
    bytecode::{Instruction, OpCode, Operand},
    executor::{ForeignFunctionHandle, VirtualMachine},
};
use shape_value::tag_bits::{TAG_FUNCTION, TAG_HEAP, TAG_MODULE_FN};
use shape_value::value_word_drop::vw_drop;
use shape_value::{VMError, ValueWord, ValueWordExt};

impl VirtualMachine {
    #[inline(always)]
    pub(in crate::executor) fn exec_control_flow(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use OpCode::*;
        match instruction.opcode {
            Jump => self.op_jump(instruction)?,
            JumpIfFalse => self.op_jump_if_false(instruction)?,
            JumpIfFalseTrusted => self.op_jump_if_false_trusted(instruction)?,
            JumpIfTrue => self.op_jump_if_true(instruction)?,
            Call => self.op_call(instruction)?,
            CallValue => self.op_call_value()?,
            CallClosure => self.op_call_closure(instruction)?,
            CallFunctionIndirect => self.op_call_function_indirect(instruction)?,
            CallForeign => self.op_call_foreign(instruction)?,
            Return => self.op_return()?,
            ReturnValue => self.op_return_value()?,
            ReturnValueI64 => self.op_return_value_i64()?,
            ReturnValueU64 => self.op_return_value_u64()?,
            ReturnValueF64 => self.op_return_value_f64()?,
            ReturnValueI32 => self.op_return_value_i32()?,
            ReturnValueU32 => self.op_return_value_u32()?,
            ReturnValueI16 => self.op_return_value_i16()?,
            ReturnValueU16 => self.op_return_value_u16()?,
            ReturnValueI8 => self.op_return_value_i8()?,
            ReturnValueU8 => self.op_return_value_u8()?,
            ReturnValueBool => self.op_return_value_bool()?,
            ReturnValuePtr => self.op_return_value_ptr()?,
            _ => unreachable!(
                "exec_control_flow called with non-control-flow opcode: {:?}",
                instruction.opcode
            ),
        }
        Ok(())
    }

    // Jump operations

    pub(in crate::executor) fn op_jump(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        if let Some(Operand::Offset(offset)) = instruction.operand {
            // OSR: backward jumps (negative offset) are loop back-edges.
            // Record the iteration and attempt OSR entry if JIT code is ready.
            #[cfg(feature = "jit")]
            if offset < 0 {
                let target_ip = (self.ip as i32 + offset) as usize;
                if let Some(func_id) = self.current_function_id() {
                    self.check_osr_back_edge(func_id, target_ip);
                    // Note: we do NOT attempt try_osr_entry here because the
                    // canonical OSR entry point is at LoopStart, not at an
                    // arbitrary backward jump. The back-edge counter is
                    // incremented here to catch loops that use Jump-backward
                    // without a LoopStart instruction.
                }
            }
            self.ip = (self.ip as i32 + offset) as usize;
        } else {
            return Err(VMError::InvalidOperand);
        }
        Ok(())
    }

    pub(in crate::executor) fn op_jump_if_false(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        if let Some(Operand::Offset(offset)) = instruction.operand {
            let condition = self.pop_raw_u64()?.is_truthy();
            if !condition {
                self.ip = (self.ip as i32 + offset) as usize;
            }
        } else {
            return Err(VMError::InvalidOperand);
        }
        Ok(())
    }

    /// JumpIfFalse — trusted variant.
    ///
    /// The compiler has proved the condition is a boolean value.
    /// E+5.4: producers (typed comparison, `Not`) now push raw native bool
    /// bits (0u64 / 1u64), not NaN-tagged ValueWord — uses `pop_native_bool`
    /// to read those bits directly with no decode overhead. The legacy
    /// `op_jump_if_false` (above) handles polymorphic ValueWord input via
    /// `is_truthy()` and is unaffected.
    #[inline(always)]
    pub(in crate::executor) fn op_jump_if_false_trusted(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        if let Some(Operand::Offset(offset)) = instruction.operand {
            let cond = self.pop_native_bool()?;
            if !cond {
                self.ip = (self.ip as i32 + offset) as usize;
            }
        } else {
            return Err(VMError::InvalidOperand);
        }
        Ok(())
    }

    pub(in crate::executor) fn op_jump_if_true(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        if let Some(Operand::Offset(offset)) = instruction.operand {
            let condition = self.pop_raw_u64()?.is_truthy();
            if condition {
                self.ip = (self.ip as i32 + offset) as usize;
            }
        } else {
            return Err(VMError::InvalidOperand);
        }
        Ok(())
    }

    // Call operations

    pub(in crate::executor) fn op_call(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let arg_count = self
            .pop_raw_u64()?
            .as_number_coerce()
            .ok_or_else(|| VMError::RuntimeError("Expected number for arg count".to_string()))?
            as usize;

        if let Some(Operand::Function(func_id)) = instruction.operand {
            // ---- JIT fast path ----
            // Check the JIT dispatch table for a natively compiled version.
            // Also checks TierManager's native_code_table for tier-promoted
            // functions that were compiled in the background.
            #[cfg(feature = "jit")]
            {
                let jit_fn_opt = self
                    .jit_dispatch_table
                    .get(&func_id.0)
                    .copied()
                    .or_else(|| {
                        self.tier_manager
                            .as_ref()
                            .and_then(|mgr| mgr.get_native_code(func_id.0))
                            .map(|ptr| unsafe {
                                std::mem::transmute::<*const u8, super::JitFnPtr>(ptr)
                            })
                    });

                if let Some(jit_fn) = jit_fn_opt {
                    // Extract all needed data from the function struct up front
                    // to avoid holding an immutable borrow across mutable calls.
                    let func = self
                        .program
                        .functions
                        .get(func_id.0 as usize)
                        .ok_or(VMError::InvalidCall)?;
                    let arity = func.arity as usize;
                    let copy_count = arg_count.min(arity).min(64);

                    // Snapshot per-parameter SlotKinds and the return kind from
                    // the callee's FrameDescriptor. These small copies release
                    // the borrow on `self.program` before we touch the stack.
                    let mut param_kinds = [crate::type_tracking::SlotKind::Unknown; 256];
                    let return_kind;
                    if let Some(fd) = &func.frame_descriptor {
                        for i in 0..copy_count {
                            param_kinds[i] = fd.slot(i);
                        }
                        return_kind = fd.return_kind;
                    } else {
                        return_kind = crate::type_tracking::SlotKind::Unknown;
                    }
                    // --- borrow on `func` / `self.program.functions` ends here ---

                    // Marshal args from VM stack into a JIT context buffer.
                    //
                    // When the callee has a FrameDescriptor with known SlotKinds
                    // for its parameter slots, we use typed marshaling via
                    // `marshal_arg_to_jit` instead of raw NaN-boxed passthrough.
                    // This gives the JIT side accurate type information.
                    //
                    // Fallback is ALWAYS NaN-boxed passthrough (never None/null).
                    let mut jit_locals = [0u64; 256];
                    let args_base = self.sp.saturating_sub(arg_count);
                    for i in 0..copy_count {
                        let vw_slice = self.stack_slice_raw((args_base + i)..(args_base + i + 1));
                        jit_locals[i] = jit_abi::marshal_arg_to_jit(&vw_slice[0], param_kinds[i]);
                    }

                    // Build a JIT context on the stack matching the JITContext
                    // layout (from shape-jit/src/context.rs):
                    //   locals:    byte 64   (u64 index 8),  256 slots
                    //   stack:     byte 2112  (u64 index 264), 512 slots
                    //   stack_ptr: byte 6208  (u64 index 776)
                    //   gc_safepoint_flag_ptr: byte 6328
                    //   foreign_bridge_ptr:    byte 6344
                    //   total: ~6352 bytes = 794 u64s, rounded to 800
                    const CTX_U64_SIZE: usize = 800;
                    const LOCALS_U64_OFFSET: usize = 8; // byte 64 / 8

                    // Compile-time assertion: CTX_U64_SIZE covers through stack_ptr + remaining fields.
                    const _: () = {
                        assert!(
                            CTX_U64_SIZE * 8 >= 6216 + 136,
                            "CTX_U64_SIZE too small for JITContext layout"
                        );
                    };
                    let mut ctx_buf = [0u64; CTX_U64_SIZE];
                    for i in 0..copy_count {
                        ctx_buf[LOCALS_U64_OFFSET + i] = jit_locals[i];
                    }

                    // Invoke JIT-compiled function.
                    let result_bits =
                        unsafe { jit_fn(ctx_buf.as_mut_ptr() as *mut u8, std::ptr::null()) };

                    // Deopt sentinel: u64::MAX means "fall back to interpreter".
                    if result_bits == u64::MAX {
                        let func_id_u16 = func_id.0;

                        // Always invalidate to prevent repeated guard failures.
                        if let Some(ref mut mgr) = self.tier_manager {
                            mgr.invalidate_function(func_id_u16);
                        }

                        // Try precise mid-function deopt recovery.
                        // Read deopt_id from ctx_buf[0] (stored by JIT deopt block).
                        let deopt_id = ctx_buf[0] as usize;
                        let deopt_info = if deopt_id != u32::MAX as usize {
                            self.tier_manager
                                .as_ref()
                                .and_then(|mgr| mgr.get_deopt_info(func_id_u16, deopt_id))
                                .cloned()
                        } else {
                            None
                        };

                        if let Some(ref info) = deopt_info {
                            if !info.local_mapping.is_empty() {
                                let bp = self.sp.saturating_sub(arg_count);

                                if !info.inline_frames.is_empty() {
                                    // Multi-frame deopt: reconstruct full call stack.
                                    // First push the outermost physical function's frame,
                                    // then intermediate frames, then innermost callee.
                                    self.deopt_with_inline_frames(info, &ctx_buf, func_id_u16, bp)?;
                                    // Interpreter now has full call stack reconstructed.
                                    return Ok(());
                                }

                                // Single-frame precise deopt: push synthetic callee frame, restore state.
                                let func = &self.program.functions[func_id_u16 as usize];
                                let locals_count = func.locals_count as usize;
                                let needed = bp + locals_count + info.stack_depth as usize;
                                if needed > self.stack.len() {
                                    self.stack.resize_with(needed * 2 + 1, || Self::NONE_BITS);
                                }
                                // Zero-init local slots (deopt_with_info overwrites the live ones)
                                for i in 0..locals_count {
                                    self.stack_write_raw(bp + i, ValueWord::none());
                                }

                                let blob_hash = self.blob_hash_for_function(func_id_u16);
                                self.call_stack.push(super::super::CallFrame {
                                    return_ip: self.ip,
                                    base_pointer: bp,
                                    locals_count,
                                    function_id: Some(func_id_u16),
                                    upvalues: None,
                                    blob_hash,
                                    closure_heap_bits: None,
                                });

                                self.deopt_with_info(info, &ctx_buf)?;
                                // Interpreter now has ip=resume_ip, sp set correctly.
                                // Execution continues from the guard point.
                                return Ok(());
                            }
                        }

                        // Emergency fallback: no precise deopt metadata found.
                        // This should not happen in production — all speculative
                        // guards emit per-guard spill blocks with precise metadata.
                        // If reached, it indicates a compiler bug (missing deopt point).
                        debug_assert!(
                            false,
                            "re-exec-from-entry fallback reached for func {}: \
                             deopt_id={}, info={:?}",
                            func_id_u16, deopt_id, deopt_info
                        );
                        if let Some(ref mut metrics) = self.metrics {
                            metrics.record_deopt_fallback();
                        }
                        self.call_function_from_stack(func_id_u16, arg_count)?;
                        return Ok(());
                    }

                    // Record successful JIT dispatch in metrics.
                    if let Some(ref mut metrics) = self.metrics {
                        metrics.record_jit_dispatch();
                    }

                    // Success: unmarshal the JIT return value using the callee's
                    // return_kind from its FrameDescriptor. For Unknown return
                    // kinds this is a zero-cost NaN-boxed passthrough (transmute).
                    let result_vw = jit_abi::unmarshal_jit_result(result_bits, return_kind);

                    // Drop call arguments from the VM stack now that native call succeeded.
                    for i in args_base..self.sp {
                        // FR.2: real release (was no-op drop of Copy u64).
                        vw_drop(self.stack[i]);
                        self.stack[i] = Self::NONE_BITS;
                    }
                    self.sp = args_base;

                    self.push_raw_u64(result_vw)?;
                    return Ok(());
                }
            }

            // ---- Tier promotion ----
            // Record the call and check if promotion threshold is crossed.
            // This is a no-op when tier_manager is None.
            if let Some(ref mut tier_mgr) = self.tier_manager {
                let fv = self
                    .feedback_vectors
                    .get(func_id.0 as usize)
                    .and_then(|o| o.as_ref());
                let _ = tier_mgr.record_call(func_id.0, fv);
            }

            // Record interpreter fallback in metrics.
            if let Some(ref mut metrics) = self.metrics {
                metrics.record_interpreter_call();
            }

            // Record call target for IC profiling.
            {
                let ip = self.ip;
                if let Some(fv) = self.current_feedback_vector() {
                    fv.record_call(ip, func_id.0);
                }
            }

            // Args are already on the stack in left-to-right order.
            // Read them directly into locals — no Vec allocation needed.
            self.call_function_from_stack(func_id.0, arg_count)?;
        } else {
            return Err(VMError::InvalidOperand);
        }
        Ok(())
    }

    /// Closure spec Phase F — direct dispatch on a statically-typed closure.
    ///
    /// `CallClosure(Count(arity))` is emitted at call sites where the
    /// compiler has proven the callee's `ClosureTypeId` (typically because a
    /// closure literal was bound to a `let` and then called through that
    /// binding, or after Phase C-style specialization narrowed the closure
    /// type). The VM behaviourally equals `CallValue` with `arg_count` read
    /// from the operand instead of popped from the stack. The JIT uses the
    /// statically-known type id to emit a direct `call` with typed capture
    /// loads — see `docs/v2-closure-specialization.md` §1.3.
    ///
    /// Stack layout (both before and after mirrors `CallValue`):
    /// - Before: `[..., callee, arg0, arg1, ..., arg_{N-1}]`
    /// - After:  `[..., result]`
    pub(in crate::executor) fn op_call_closure(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let arity = match instruction.operand {
            Some(Operand::Count(n)) => n as usize,
            _ => return Err(VMError::InvalidOperand),
        };
        self.dispatch_call_closure_like(arity)
    }

    /// Closure spec Phase F — polymorphic dispatch through `Function<A, R>`.
    ///
    /// `CallFunctionIndirect(Count(arity))` is emitted at call sites where
    /// the callee's concrete `ClosureTypeId` is not known but the signature
    /// is (i.e. the callee is typed as `Function<A, R>`). The JIT lowers
    /// this to a `call_indirect` with the `FunctionTypeId`'s Cranelift
    /// signature; the VM dispatches through the same runtime path as
    /// `CallValue`. The opcode distinction exists so the JIT can avoid the
    /// full tag-dispatch cost when it knows the callee is a callable value
    /// (not an arbitrary ValueWord).
    pub(in crate::executor) fn op_call_function_indirect(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let arity = match instruction.operand {
            Some(Operand::Count(n)) => n as usize,
            _ => return Err(VMError::InvalidOperand),
        };
        self.dispatch_call_closure_like(arity)
    }

    /// Shared VM dispatch helper for `CallClosure` / `CallFunctionIndirect`.
    ///
    /// The arity comes from the opcode operand rather than the stack, so
    /// this helper does not pop a Count sentinel before peeking the
    /// callee. Otherwise the dispatch tree mirrors `op_call_value`.
    ///
    /// Closure spec Phase G §5.4: records the resolved target `function_id`
    /// into the current function's feedback vector so the JIT Tier 2 can
    /// emit speculative direct-call guards when the site has gone
    /// monomorphic. The feedback recording happens on the indirect path
    /// (closure / function-ref callees). Host closures and module
    /// functions are not recorded (no stable `function_id` / different
    /// call ABI).
    fn dispatch_call_closure_like(&mut self, arg_count: usize) -> Result<(), VMError> {
        // Stack layout: [callee, arg0, arg1, ..., argN-1]
        let callee_idx = self
            .sp
            .checked_sub(arg_count + 1)
            .ok_or(VMError::StackUnderflow)?;
        let callee_nb = self.stack_read_raw(callee_idx);

        let _bits = callee_nb.raw_bits();
        if !shape_value::tag_bits::is_tagged(_bits) {
            return Err(VMError::RuntimeError(
                "Cannot call non-function value".to_string(),
            ));
        }
        // `self.ip` points one past the call opcode by this time (the
        // dispatch loop bumps `ip` before handing off to the handler);
        // the call opcode's IP is therefore `ip - 1`. Feedback is keyed
        // on the call opcode's IP to match what the JIT observes in
        // its MIR view of the bytecode.
        let call_ip = self.ip.saturating_sub(1);
        match shape_value::tag_bits::get_tag(_bits) {
            TAG_FUNCTION => {
                let func_id = callee_nb.as_function_id().ok_or(VMError::InvalidCall)?;
                let _ = callee_nb;
                // WB2.3: function-id callees are inline (no heap share), so
                // `vw_drop` is a no-op here — kept for contract uniformity.
                vw_drop(self.stack[callee_idx]);
                for i in callee_idx..callee_idx + arg_count {
                    self.stack[i] = self.stack[i + 1];
                    self.stack[i + 1] = Self::NONE_BITS;
                }
                self.sp -= 1;
                // Closure spec Phase G §5.4: record monomorphic target.
                if let Some(fv) = self.current_feedback_vector() {
                    fv.record_call(call_ip, func_id);
                }
                self.call_function_from_stack(func_id, arg_count)
            }
            TAG_MODULE_FN => {
                let func_id = callee_nb.as_module_function().ok_or(VMError::InvalidCall)?;
                let _ = callee_nb;
                let args_base = callee_idx + 1;
                let mut args_nb: Vec<ValueWord> = Vec::with_capacity(arg_count);
                for i in 0..arg_count {
                    args_nb.push(self.stack_take_raw(args_base + i));
                }
                // WB2.3: module-fn callees are inline, `vw_drop` is a no-op
                // for the tag but the clear preserves NONE_BITS invariant.
                vw_drop(self.stack[callee_idx]);
                self.stack[callee_idx] = Self::NONE_BITS;
                self.sp = callee_idx;
                let result_nb = self.invoke_module_fn_id(func_id, &args_nb)?;
                self.push_raw_u64(result_nb)?;
                Ok(())
            }
            TAG_HEAP => {
                if let Some((function_id, upvalues_slice)) =
                    raw_helpers::extract_closure_info(callee_nb.raw_bits())
                {
                    let function_id = function_id;
                    let upvalues = upvalues_slice.to_vec();
                    let _ = callee_nb;
                    let args_base = callee_idx + 1;
                    let mut args_nb = Vec::with_capacity(arg_count);
                    for i in 0..arg_count {
                        args_nb.push(self.stack_take_raw(args_base + i));
                    }
                    // WB2.3 retain-on-read: move ownership of the
                    // closure HeapValue bits off the caller stack slot
                    // and onto the callee `CallFrame.closure_heap_bits`
                    // keep-alive. This guarantees the block outlives
                    // the callee's `OwnedMutable` / `Shared` pointer
                    // captures (which dereference raw `*mut` / `*const`
                    // into the block's allocation) and is released on
                    // frame pop via `op_return` / `op_return_value`.
                    let closure_bits = self.stack[callee_idx];
                    self.stack[callee_idx] = Self::NONE_BITS;
                    self.sp = callee_idx;
                    // Closure spec Phase G §5.4: record monomorphic target.
                    // The closure's underlying `function_id` is the guard
                    // key the Tier 2 JIT compares against. A single
                    // observed id -> `ICState::Monomorphic`; multiple
                    // observations promote to Polymorphic/Megamorphic
                    // via the existing state machine in
                    // `feedback::FeedbackVector::record_call`.
                    if let Some(fv) = self.current_feedback_vector() {
                        fv.record_call(call_ip, function_id);
                    }
                    self.call_closure_with_nb_args_keepalive(
                        function_id,
                        upvalues,
                        &args_nb,
                        Some(closure_bits),
                    )
                } else if let Some(shape_value::heap_value::HeapValue::HostClosure(callable)) =
                    callee_nb.as_heap_ref()
                {
                    let callable = callable.clone();
                    let _ = callee_nb;
                    let args_base = callee_idx + 1;
                    let mut args_nb: Vec<ValueWord> = Vec::with_capacity(arg_count);
                    for i in 0..arg_count {
                        args_nb.push(self.stack_take_raw(args_base + i));
                    }
                    // WB2.3: release the HostClosure HeapValue. The
                    // `callable.clone()` above retained an independent
                    // `Arc<dyn Fn>` share so the call survives the slot
                    // release.
                    vw_drop(self.stack[callee_idx]);
                    self.stack[callee_idx] = Self::NONE_BITS;
                    self.sp = callee_idx;
                    let result_nb = callable.call(&args_nb).map_err(VMError::RuntimeError)?;
                    self.push_raw_u64(result_nb)?;
                    Ok(())
                } else {
                    Err(VMError::InvalidCall)
                }
            }
            _ => Err(VMError::InvalidCall),
        }
    }

    pub(in crate::executor) fn op_call_value(&mut self) -> Result<(), VMError> {
        let arg_count = self
            .pop_raw_u64()?
            .as_number_coerce()
            .ok_or_else(|| VMError::RuntimeError("Expected number for arg count".to_string()))?
            as usize;

        // Stack layout: [callee, arg0, arg1, ..., argN-1]
        // Peek at the callee (below the args) to choose the dispatch path.
        let callee_idx = self
            .sp
            .checked_sub(arg_count + 1)
            .ok_or(VMError::StackUnderflow)?;
        let callee_nb = self.stack_read_raw(callee_idx);

        // Tag dispatch (no ValueWord materialization)
        let _bits = callee_nb.raw_bits();
        if !shape_value::tag_bits::is_tagged(_bits) {
            return Err(VMError::RuntimeError(
                "Cannot call non-function value".to_string(),
            ));
        }
        match shape_value::tag_bits::get_tag(_bits) {
            TAG_FUNCTION => {
                let func_id = callee_nb.as_function_id().ok_or(VMError::InvalidCall)?;
                let _ = callee_nb;
                // WB2.3: inline function-id callee — `vw_drop` no-op.
                vw_drop(self.stack[callee_idx]);
                for i in callee_idx..callee_idx + arg_count {
                    self.stack[i] = self.stack[i + 1];
                    // The source slot is now duplicated; clear it so we don't double-own.
                    self.stack[i + 1] = Self::NONE_BITS;
                }
                self.sp -= 1;
                self.call_function_from_stack(func_id, arg_count)
            }
            TAG_MODULE_FN => {
                let func_id = callee_nb.as_module_function().ok_or(VMError::InvalidCall)?;
                let _ = callee_nb;
                let args_base = callee_idx + 1;
                let mut args_nb: Vec<ValueWord> = Vec::with_capacity(arg_count);
                for i in 0..arg_count {
                    args_nb.push(self.stack_take_raw(args_base + i));
                }
                // WB2.3: inline module-fn callee — `vw_drop` no-op.
                vw_drop(self.stack[callee_idx]);
                self.stack[callee_idx] = Self::NONE_BITS;
                self.sp = callee_idx;
                let result_nb = self.invoke_module_fn_id(func_id, &args_nb)?;
                self.push_raw_u64(result_nb)?;
                Ok(())
            }
            TAG_HEAP => {
                // Extract closure info via raw_helpers
                if let Some((function_id, upvalues_slice)) =
                    raw_helpers::extract_closure_info(callee_nb.raw_bits())
                {
                    let function_id = function_id;
                    let upvalues = upvalues_slice.to_vec();
                    let _ = callee_nb;
                    // Collect args as ValueWord, then remove callee
                    let args_base = callee_idx + 1;
                    let mut args_nb = Vec::with_capacity(arg_count);
                    for i in 0..arg_count {
                        args_nb.push(self.stack_take_raw(args_base + i));
                    }
                    // WB2.3 retain-on-read: move ownership of the closure
                    // HeapValue onto the callee frame keep-alive (see
                    // `dispatch_call_closure_like` for the rationale — raw
                    // pointer captures must outlive the callee).
                    let closure_bits = self.stack[callee_idx];
                    self.stack[callee_idx] = Self::NONE_BITS;
                    self.sp = callee_idx;
                    self.call_closure_with_nb_args_keepalive(
                        function_id,
                        upvalues,
                        &args_nb,
                        Some(closure_bits),
                    )
                // cold-path: as_heap_ref retained — HostClosure fallback (no typed extractor)
                } else if let Some(shape_value::heap_value::HeapValue::HostClosure(callable)) =
                    callee_nb.as_heap_ref()
                {
                    // cold-path
                    let callable = callable.clone();
                    let _ = callee_nb;
                    let args_base = callee_idx + 1;
                    let mut args_nb: Vec<ValueWord> = Vec::with_capacity(arg_count);
                    for i in 0..arg_count {
                        args_nb.push(self.stack_take_raw(args_base + i));
                    }
                    // WB2.3: release the HostClosure HeapValue; the
                    // `callable.clone()` retained the underlying Fn Arc.
                    vw_drop(self.stack[callee_idx]);
                    self.stack[callee_idx] = Self::NONE_BITS;
                    self.sp = callee_idx;
                    let result_nb = callable.call(&args_nb).map_err(VMError::RuntimeError)?;
                    self.push_raw_u64(result_nb)?;
                    Ok(())
                } else {
                    Err(VMError::InvalidCall)
                }
            }
            _ => Err(VMError::InvalidCall),
        }
    }

    pub(in crate::executor) fn op_make_closure(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        // Closure spec H5: `MakeClosure` accepts two operand shapes:
        //   - `Operand::Function(fid)`            — non-escaping closure.
        //   - `Operand::ClosureAlloc { fid, .. }` — compiler-tagged with
        //     escape status (the VM path is identical; the JIT's MIR
        //     lowering reads `escapes` to pick stack vs. heap codegen).
        let func_id_opt = match instruction.operand {
            Some(Operand::Function(fid)) => Some(fid),
            Some(Operand::ClosureAlloc { fid, .. }) => Some(fid),
            _ => None,
        };
        if let Some(func_id) = func_id_opt {
            let function = self
                .program
                .functions
                .get(func_id.index())
                .ok_or(VMError::InvalidOperand)?;
            let capture_count = function.captures_count as usize;
            let mutable_captures = function.mutable_captures.clone();
            let mut upvalues = Vec::with_capacity(capture_count);
            for _ in 0..capture_count {
                let nb = self.pop_raw_u64()?;
                upvalues.push(nb);
            }
            upvalues.reverse();

            // Closure spec §13 H3: the legacy mutable-upvalue enum variant
            // has been retired. The shared-mutable-state carrier for
            // mutable captures is now the
            // `HeapValue::SharedCell` value sitting on the stack (emitted by
            // the legacy `BoxLocal` / `BoxModuleBinding` path, which H4
            // deletes). `Upvalue::new` stashes that SharedCell-bearing
            // ValueWord directly; `Upvalue::get` / `set` auto-deref through
            // the cell so every closure observing it sees the same slot.
            // Non-shared mutable captures (no SharedCell on the stack — e.g.
            // when BoxLocal was skipped) fall back to the closure-local
            // write semantics that the fresh-Arc branch provided pre-H3.
            let _ = &mutable_captures; // mutability flag remains for future frame-pointer work

            // Track A.5: every closure is emitted as `HeapValue::ClosureRaw`,
            // backed by a raw `TypedClosureHeader` block. The captures are
            // written at the layout's typed offsets via
            // `closure_raw::write_capture_typed`. Heap-typed captures
            // (`heap_capture_mask` bits) need an additional retain — the
            // block takes ownership of one share; the source stack slot
            // keeps its own share.
            //
            // A missing `ClosureLayout` is a compile-time / link-time bug
            // (A.5 retired the legacy fallback); see the error path
            // below.
            //
            // Track A.1B: when the registered layout marks any capture as
            // `CaptureKind::OwnedMutable` or `CaptureKind::Shared` (the
            // kinds A.1C's compiler emits for `let mut` / `var`
            // captures), the Raw path handles `Box<ValueWord>` /
            // `Arc<SharedCell>` allocation + release directly.
            let layout_opt: Option<std::sync::Arc<shape_value::v2::closure_layout::ClosureLayout>> =
                self.program
                    .closure_function_layouts
                    .get(func_id.index())
                    .and_then(|l| l.as_ref())
                    .cloned();
            let layout_has_mutable_cell_kinds = layout_opt
                .as_ref()
                .map(|l| l.owned_mutable_capture_mask != 0 || l.shared_capture_mask != 0)
                .unwrap_or(false);
            let any_mutable_capture = mutable_captures.iter().any(|b| *b);
            let layout_opt = if any_mutable_capture && !layout_has_mutable_cell_kinds {
                None
            } else {
                layout_opt
            };

            if let Some(layout) = layout_opt {
                if upvalues.len() == layout.capture_count() {
                    use shape_value::v2::closure_layout::{CaptureKind, SharedCell};
                    use shape_value::v2::closure_raw::{
                        OwnedClosureBlock, alloc_owned_mutable_bool, alloc_owned_mutable_f64,
                        alloc_owned_mutable_i8, alloc_owned_mutable_i16, alloc_owned_mutable_i32,
                        alloc_owned_mutable_i64, alloc_owned_mutable_ptr, alloc_owned_mutable_u8,
                        alloc_owned_mutable_u16, alloc_owned_mutable_u32, alloc_owned_mutable_u64,
                        alloc_typed_closure, write_capture_typed,
                    };
                    use shape_value::v2::struct_layout::FieldKind;
                    // SAFETY: `alloc_typed_closure` returns a freshly-
                    // allocated block with refcount=1; we write the
                    // captures at their typed offsets in-bounds, then
                    // transfer ownership to the `OwnedClosureBlock` below.
                    //
                    // Track A.1B: per-capture `CaptureKind` now selects
                    // the allocation discipline:
                    //   * Immutable: write the capture bits at the typed
                    //     offset via `write_capture_typed` (unchanged).
                    //   * OwnedMutable: allocate `Box::new(ValueWord)`,
                    //     write `Box::into_raw` into the 8-byte Ptr slot.
                    //     Released by `release_typed_closure` via
                    //     `Box::from_raw` on the closure's last refcount
                    //     release (see `release_typed_closure` +
                    //     `owned_mutable_capture_mask`).
                    //   * Shared: allocate
                    //     `Arc::new(parking_lot::Mutex::new(ValueWord))`,
                    //     write `Arc::into_raw` into the Ptr slot.
                    //     Released via `Arc::from_raw` on the closure's
                    //     last refcount release. For A.1B we construct a
                    //     fresh Arc per capture; A.1C refines the
                    //     compiler so multi-closure var-captures reuse
                    //     an existing `Arc<SharedCell>` (`Arc::clone` at
                    //     this site).
                    unsafe {
                        let ptr = alloc_typed_closure(func_id.0, 0, &layout);
                        for (i, bits) in upvalues.iter().enumerate() {
                            match layout.capture_storage_kind(i) {
                                CaptureKind::Immutable => {
                                    // Heap-typed captures: retain one
                                    // share for the block. The source
                                    // stack slot keeps its own share
                                    // (the capture ValueWord came from
                                    // the stack via `pop_raw_u64` which
                                    // did NOT drop).
                                    if layout.is_heap_capture(i) {
                                        // FR.2: clone_from_bits bumped the
                                        // refcount; the resulting u64 is
                                        // Copy so falling out of scope is a
                                        // no-op — share stays alive for the
                                        // block.
                                        let _dup = ValueWord::clone_from_bits(*bits);
                                        let _ = _dup;
                                    }
                                    write_capture_typed(ptr, &layout, i, *bits);
                                }
                                CaptureKind::OwnedMutable => {
                                    // Wave D (D.3): allocate the typed
                                    // OwnedMutable cell via Wave B's
                                    // per-FieldKind helpers — `Box<T>`
                                    // matching `capture_inner_kind(i)`,
                                    // not a uniform `Box<ValueWord>`.
                                    // `release_typed_closure` /
                                    // `drop_owned_mutable_capture`
                                    // already dispatches on the same
                                    // `capture_inner_kind` to reclaim
                                    // each typed box.
                                    //
                                    // Post Wave E+5/Unit B: typed-scalar
                                    // producers (`op_push_const` Int/Number/Bool,
                                    // typed `Load/StoreLocal<Kind>`, typed
                                    // arithmetic) push **native** bytes onto
                                    // the stack — no ValueWord tag. Decode the
                                    // popped 8-byte slot per-FieldKind using
                                    // the same raw-bit pattern as the typed
                                    // `op_store_shared_capture_<kind>`
                                    // handlers in variables/mod.rs:1703+.
                                    //
                                    // For `FieldKind::Ptr` the popped `*bits`
                                    // already carries the heap refcount share
                                    // verbatim (the stack slot's `pop_raw_u64`
                                    // did NOT drop), so the box swallows that
                                    // share — no retain/release pair at this
                                    // site.
                                    //
                                    // SAFETY: each `alloc_owned_mutable_<kind>`
                                    // returns `Box::into_raw(Box::new(...))`
                                    // — a non-null pointer with the alignment
                                    // of `T`. We cast to `u64` for storage in
                                    // the Ptr slot at
                                    // `layout.heap_capture_offset(i)`, which
                                    // is 8-byte aligned and in-bounds per
                                    // layout invariants.
                                    let inner = layout.capture_inner_kind(i);
                                    let cell_ptr_bits: u64 = match inner {
                                        FieldKind::I64 => alloc_owned_mutable_i64(
                                            *bits as i64,
                                        )
                                            as u64,
                                        FieldKind::F64 => alloc_owned_mutable_f64(
                                            f64::from_bits(*bits),
                                        )
                                            as u64,
                                        FieldKind::I32 => alloc_owned_mutable_i32(
                                            *bits as i64 as i32,
                                        )
                                            as u64,
                                        FieldKind::I16 => alloc_owned_mutable_i16(
                                            *bits as i64 as i16,
                                        )
                                            as u64,
                                        FieldKind::I8 => alloc_owned_mutable_i8(
                                            *bits as i64 as i8,
                                        )
                                            as u64,
                                        FieldKind::U64 => alloc_owned_mutable_u64(
                                            *bits,
                                        )
                                            as u64,
                                        FieldKind::U32 => alloc_owned_mutable_u32(
                                            *bits as u32,
                                        )
                                            as u64,
                                        FieldKind::U16 => alloc_owned_mutable_u16(
                                            *bits as u16,
                                        )
                                            as u64,
                                        FieldKind::U8 => alloc_owned_mutable_u8(
                                            *bits as u8,
                                        )
                                            as u64,
                                        FieldKind::Bool => alloc_owned_mutable_bool(
                                            *bits != 0,
                                        )
                                            as u64,
                                        FieldKind::Ptr => {
                                            // Pass-through: the cell
                                            // stores the raw 8-byte
                                            // heap-pointer bit pattern
                                            // verbatim (the popped
                                            // share moves into the
                                            // box). `release_typed_closure`
                                            // releases this share via
                                            // `release_raw_value_bits`
                                            // before reclaiming the
                                            // `Box<u64>`.
                                            alloc_owned_mutable_ptr(*bits) as u64
                                        }
                                    };
                                    let off = layout.heap_capture_offset(i);
                                    std::ptr::write(
                                        (ptr as *mut u8).add(off) as *mut u64,
                                        cell_ptr_bits,
                                    );
                                }
                                CaptureKind::Shared => {
                                    // Wave D (D.3): Shared cells use a
                                    // single 8-byte payload regardless
                                    // of declared FieldKind (D5
                                    // invariant); Wave B's per-FieldKind
                                    // read/write helpers reinterpret the
                                    // 8 bytes correctly. The legacy
                                    // generic Arc-pointer-bits
                                    // pass-through path remains until
                                    // follow-up #17 (atomic Shared
                                    // encoding flip) lands across BOTH
                                    // the JIT's outer-scope
                                    // shared_local_slots path AND the
                                    // closure-body shared_capture_slots
                                    // path.
                                    //
                                    // Track A.1C.2: the compiler emits
                                    // code that pushes the raw
                                    // `*const SharedCell` pointer bits
                                    // of a previously-promoted outer
                                    // slot (see `AllocSharedLocal` and
                                    // the closure-creation path in
                                    // `compile_expr_closure`). The
                                    // outer slot owns one Arc strong
                                    // share; the closure needs its own
                                    // share.
                                    //
                                    // `Arc::increment_strong_count` on
                                    // the raw pointer bumps the refcount
                                    // without reconstructing the Arc
                                    // (which would take ownership and
                                    // drop the share on return). The
                                    // same pointer bits are then
                                    // written into the Ptr slot.
                                    // `release_typed_closure` calls
                                    // `Arc::from_raw` +drop on
                                    // `shared_capture_mask` bits to
                                    // release this share.
                                    //
                                    // SAFETY: the bit pattern in
                                    // `*bits` was produced by
                                    // `AllocSharedLocal` via
                                    // `Arc::into_raw::<SharedCell>`;
                                    // it is non-null, 8-aligned, and
                                    // points at a live allocation for
                                    // the lifetime of the outer slot
                                    // (which only releases on
                                    // `DropSharedLocal`). The compiler
                                    // guarantees the outer slot is
                                    // still live at this point — the
                                    // shared-local lifecycle places
                                    // `AllocSharedLocal` before every
                                    // closure that captures the slot
                                    // and `DropSharedLocal` at the
                                    // scope-exit point that dominates
                                    // all such closure creations.
                                    let cell_ptr = *bits as *const SharedCell;
                                    std::sync::Arc::<SharedCell>::increment_strong_count(cell_ptr);
                                    let off = layout.heap_capture_offset(i);
                                    std::ptr::write(
                                        (ptr as *mut u8).add(off) as *mut *const SharedCell,
                                        cell_ptr,
                                    );
                                }
                            }
                        }
                        let owned = OwnedClosureBlock::from_raw(ptr as *const u8, layout);
                        self.push_raw_u64(ValueWord::from_heap_value(
                            shape_value::heap_value::HeapValue::ClosureRaw(owned),
                        ))?;
                    }
                    return Ok(());
                }
                // Capture-count mismatch: layout is stale. Prior to
                // A.1C.3 this fell through to a `HeapValue::Closure {
                // function_id, upvalues }` legacy producer. That
                // producer is retired — mismatch is a compile-time /
                // link-time bug, not a runtime-recoverable condition.
            }

            // Track A.5: every `op_make_closure` invocation must produce
            // a `HeapValue::ClosureRaw` via the `ClosureLayout` path
            // above. There is no legacy fallback — A.2A / A.3 / A.4 /
            // A.5 retired the remaining `HeapValue::Closure` producers.
            let _ = upvalues;
            Err(VMError::RuntimeError(format!(
                "internal error: MakeClosure for function {} has no registered ClosureLayout (layout registration is mandatory; Track A.5 retired every legacy closure producer)",
                func_id.0
            )))
        } else {
            Err(VMError::InvalidOperand)
        }
    }

    // Foreign function call

    pub(in crate::executor) fn op_call_foreign(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let foreign_idx = match instruction.operand {
            Some(Operand::ForeignFunction(idx)) => idx as usize,
            _ => return Err(VMError::InvalidOperand),
        };

        // Pop arg count (pushed by the stub as a constant)
        let arg_count = self
            .pop_raw_u64()?
            .as_number_coerce()
            .ok_or_else(|| VMError::RuntimeError("Expected number for arg count".to_string()))?
            as usize;

        // Pop args in reverse order then reverse for correct ordering
        let mut args = Vec::with_capacity(arg_count);
        for _ in 0..arg_count {
            args.push(self.pop_raw_u64()?);
        }
        args.reverse();

        // Look up foreign function entry
        let entry = self
            .program
            .foreign_functions
            .get(foreign_idx)
            .ok_or_else(|| {
                VMError::RuntimeError(format!(
                    "Foreign function index {} out of bounds",
                    foreign_idx
                ))
            })?;
        let language = entry.language.clone();
        let name = entry.name.clone();
        let dynamic_errors = entry.dynamic_errors;
        let return_type = entry.return_type.clone();
        let return_type_schema_id = entry.return_type_schema_id;
        let is_native_abi = entry.native_abi.is_some();

        // Look up compiled handle
        let handle = self
            .foreign_fn_handles
            .get(foreign_idx)
            .and_then(|h| h.as_ref())
            .ok_or_else(|| {
                if is_native_abi {
                    VMError::RuntimeError(format!(
                        "Native function '{}' is not linked. \
                         Verify [native-dependencies] and shared library availability.",
                        name
                    ))
                } else {
                    VMError::RuntimeError(format!(
                        "Foreign function '{}' (language '{}') not linked. \
                         Install the {} language runtime extension.",
                        name, language, language
                    ))
                }
            })?;

        match handle.clone() {
            ForeignFunctionHandle::Runtime { runtime, compiled } => {
                // Marshal args: convert Vec<ValueWord> -> msgpack bytes
                let args_msgpack =
                    foreign_marshal::marshal_args(&args, &self.program.type_schema_registry)?;

                // return_type is guaranteed Some(...) — the compiler rejects foreign functions
                // without an explicit return type annotation.
                let rt = return_type.as_ref().expect(
                    "ICE: foreign function missing return type (compiler should reject this)",
                );

                // Call runtime.invoke and handle result based on error model
                if dynamic_errors {
                    // Dynamic error model: wrap success in Ok, failure in Err.
                    // Two error kinds are distinguished by the `code` field:
                    //   - "RUNTIME_ERROR"  — the foreign code raised an exception
                    //   - "MARSHAL_ERROR"  — the return value couldn't be converted to T
                    match runtime.invoke(&compiled, &args_msgpack) {
                        Ok(result_msgpack) => {
                            match foreign_marshal::unmarshal_result(
                                &result_msgpack,
                                rt,
                                return_type_schema_id,
                                &self.program.type_schema_registry,
                            ) {
                                Ok(result) => {
                                    self.push_raw_u64(ValueWord::from_ok(result))?;
                                }
                                Err(marshal_err) => {
                                    let error_msg =
                                        format!("Foreign function '{}': {}", name, marshal_err);
                                    let error_payload =
                                        ValueWord::from_string(std::sync::Arc::new(error_msg));
                                    let trace = self.trace_info_full_nb();
                                    let err_obj = self.build_any_error_nb(
                                        error_payload,
                                        None,
                                        trace,
                                        Some("MARSHAL_ERROR"),
                                    );
                                    self.push_raw_u64(ValueWord::from_err(err_obj))?;
                                }
                            }
                        }
                        Err(e) => {
                            let error_msg = format!("{}", e);
                            let error_payload =
                                ValueWord::from_string(std::sync::Arc::new(error_msg));
                            let trace = self.trace_info_full_nb();
                            let err_obj = self.build_any_error_nb(
                                error_payload,
                                None,
                                trace,
                                Some("RUNTIME_ERROR"),
                            );
                            self.push_raw_u64(ValueWord::from_err(err_obj))?;
                        }
                    }
                } else {
                    // Static error model: errors are hard crashes (current behavior)
                    let result_msgpack = runtime.invoke(&compiled, &args_msgpack).map_err(|e| {
                        VMError::RuntimeError(format!("Foreign function '{}' error: {}", name, e))
                    })?;
                    let result = foreign_marshal::unmarshal_result(
                        &result_msgpack,
                        rt,
                        return_type_schema_id,
                        &self.program.type_schema_registry,
                    )?;
                    self.push_raw_u64(result)?;
                }
            }
            ForeignFunctionHandle::Native(linked) => {
                unsafe fn vm_callable_invoker(
                    ctx: *mut std::ffi::c_void,
                    callable: &ValueWord,
                    args: &[ValueWord],
                ) -> Result<ValueWord, String> {
                    let vm = unsafe { &mut *(ctx as *mut crate::executor::VirtualMachine) };
                    vm.call_value_immediate_nb(callable, args, None)
                        .map_err(|err| err.to_string())
                }

                let raw_invoker = shape_runtime::module_exports::RawCallableInvoker {
                    ctx: self as *mut crate::executor::VirtualMachine as *mut std::ffi::c_void,
                    invoke: vm_callable_invoker,
                };
                let live_stack_len = self.sp;
                // SAFETY: ValueWord is #[repr(transparent)] over u64, so
                // &mut [u64] and &mut [ValueWord] have identical layout.
                let vw_slice: &mut [ValueWord] = unsafe {
                    std::slice::from_raw_parts_mut(
                        self.stack[..live_stack_len].as_mut_ptr() as *mut ValueWord,
                        live_stack_len,
                    )
                };
                let result = native_abi::invoke_linked_function(
                    &linked,
                    &args,
                    Some(raw_invoker),
                    Some(vw_slice),
                )
                .map_err(|e| {
                    VMError::RuntimeError(format!("Native function '{}' error: {}", name, e))
                })?;
                self.push_raw_u64(result)?;
            }
        }

        Ok(())
    }

    // Return operations

    pub(in crate::executor) fn op_return(&mut self) -> Result<(), VMError> {
        if let Some(frame) = self.call_stack.pop() {
            // Restore instruction pointer
            self.ip = frame.return_ip;

            // Clean up register window: release each slot's share then
            // clear to NONE_BITS, finally restore sp to base_pointer.
            //
            // WB2.6 Phase 3: with retain-on-read in force across WB2.1–WB2.5,
            // every slot in `[bp..sp)` holds an **owning** share; releasing
            // per slot is now required to avoid leaks and is safe because no
            // caller aliases these bits without its own retain.
            let bp = frame.base_pointer;
            for i in bp..self.sp {
                let bits = self.stack[i];
                self.stack[i] = Self::NONE_BITS;
                shape_value::value_word_drop::vw_drop(bits);
            }
            self.sp = bp;
            // WB2.3 retain-on-read: release the closure keep-alive (if any)
            // now that the callee's `OwnedMutable` / `Shared` pointer
            // captures are no longer in scope.
            if let Some(bits) = frame.closure_heap_bits {
                vw_drop(bits);
            }
        } else {
            // Return from main
            self.ip = self.program.instructions.len();
        }
        Ok(())
    }

    pub(in crate::executor) fn op_return_value(&mut self) -> Result<(), VMError> {
        let return_value = self.pop_raw_u64()?;
        self.return_value_inner(return_value)
    }

    /// Shared inner body for `op_return_value` and the typed
    /// `op_return_value_<kind>` family (Wave E+3, opcodes 0x198..=0x1A2).
    ///
    /// The typed variants are *transport-neutral*: they pop the return
    /// value as raw u64 (same as the legacy `ReturnValue`) and feed it
    /// here for frame cleanup + caller-side push. The encoded `<Kind>`
    /// is a static annotation for the JIT and downstream consumers, not
    /// a runtime dispatch — so all 11 typed handlers and the legacy
    /// handler share this body verbatim.
    #[inline]
    fn return_value_inner(&mut self, return_value: u64) -> Result<(), VMError> {
        if let Some(frame) = self.call_stack.pop() {
            // Restore instruction pointer
            self.ip = frame.return_ip;

            // Clean up register window (see `op_return` for the
            // retain-on-read rationale). WB2.6 Phase 3 releases each
            // slot via `vw_drop`.
            let bp = frame.base_pointer;
            for i in bp..self.sp {
                let bits = self.stack[i];
                self.stack[i] = Self::NONE_BITS;
                shape_value::value_word_drop::vw_drop(bits);
            }
            self.sp = bp;

            // WB2.3 retain-on-read: release the closure keep-alive.
            if let Some(bits) = frame.closure_heap_bits {
                vw_drop(bits);
            }

            // Push return value
            self.push_raw_u64(return_value)?;
        } else {
            // Return from main
            self.push_raw_u64(return_value)?;
            self.ip = self.program.instructions.len();
        }
        Ok(())
    }

    // ─────────────────────────────────────────────────────────────────
    // Wave E+3: typed `ReturnValue<Kind>` handlers (opcodes 0x198..=0x1A2)
    //
    // Each typed handler is a thin wrapper around `return_value_inner`.
    // The handler bodies are identical at runtime — the encoded `<Kind>`
    // exists for static type information so the caller's stack
    // discipline is known at the call site (consumed by the JIT and
    // other downstream tooling).
    //
    // The legacy `op_return_value` (0x45) stays live for unproven-type
    // return positions.
    // ─────────────────────────────────────────────────────────────────

    /// Run `return_value_inner`, then if the pop landed at the top-level
    /// (call stack now empty), stamp `last_program_return_kind` so the
    /// host-boundary synthesizer knows how to re-tag the native bits.
    /// Skip the stamp for nested returns — only the LAST typed return on
    /// the path back to top-level should set the kind, and that's
    /// exactly the one whose pop empties the call stack.
    #[inline]
    fn typed_return_with_kind(
        &mut self,
        return_value: u64,
        kind: crate::type_tracking::SlotKind,
    ) -> Result<(), VMError> {
        self.return_value_inner(return_value)?;
        if self.call_stack.is_empty() {
            self.last_program_return_kind = Some(kind);
        }
        Ok(())
    }

    pub(in crate::executor) fn op_return_value_i64(&mut self) -> Result<(), VMError> {
        let return_value = self.pop_raw_u64()?;
        self.typed_return_with_kind(return_value, crate::type_tracking::SlotKind::Int64)
    }

    pub(in crate::executor) fn op_return_value_u64(&mut self) -> Result<(), VMError> {
        let return_value = self.pop_raw_u64()?;
        self.typed_return_with_kind(return_value, crate::type_tracking::SlotKind::UInt64)
    }

    pub(in crate::executor) fn op_return_value_f64(&mut self) -> Result<(), VMError> {
        let return_value = self.pop_raw_u64()?;
        self.typed_return_with_kind(return_value, crate::type_tracking::SlotKind::Float64)
    }

    pub(in crate::executor) fn op_return_value_i32(&mut self) -> Result<(), VMError> {
        let return_value = self.pop_raw_u64()?;
        self.typed_return_with_kind(return_value, crate::type_tracking::SlotKind::Int32)
    }

    pub(in crate::executor) fn op_return_value_u32(&mut self) -> Result<(), VMError> {
        let return_value = self.pop_raw_u64()?;
        self.typed_return_with_kind(return_value, crate::type_tracking::SlotKind::UInt32)
    }

    pub(in crate::executor) fn op_return_value_i16(&mut self) -> Result<(), VMError> {
        let return_value = self.pop_raw_u64()?;
        self.typed_return_with_kind(return_value, crate::type_tracking::SlotKind::Int16)
    }

    pub(in crate::executor) fn op_return_value_u16(&mut self) -> Result<(), VMError> {
        let return_value = self.pop_raw_u64()?;
        self.typed_return_with_kind(return_value, crate::type_tracking::SlotKind::UInt16)
    }

    pub(in crate::executor) fn op_return_value_i8(&mut self) -> Result<(), VMError> {
        let return_value = self.pop_raw_u64()?;
        self.typed_return_with_kind(return_value, crate::type_tracking::SlotKind::Int8)
    }

    pub(in crate::executor) fn op_return_value_u8(&mut self) -> Result<(), VMError> {
        let return_value = self.pop_raw_u64()?;
        self.typed_return_with_kind(return_value, crate::type_tracking::SlotKind::UInt8)
    }

    pub(in crate::executor) fn op_return_value_bool(&mut self) -> Result<(), VMError> {
        let return_value = self.pop_raw_u64()?;
        self.typed_return_with_kind(return_value, crate::type_tracking::SlotKind::Bool)
    }

    pub(in crate::executor) fn op_return_value_ptr(&mut self) -> Result<(), VMError> {
        // Ptr returns are heap-tagged ValueWord bits — synthesizer
        // passthrough is correct, no stamp needed.
        let return_value = self.pop_raw_u64()?;
        self.return_value_inner(return_value)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Closure spec §14.6 — H6.5 regression tests
// ─────────────────────────────────────────────────────────────────────────────
//
// H6.5 makes producers emit `HeapValue::ClosureRaw` (backed by an
// `OwnedClosureBlock` around a raw `TypedClosureHeader`) in place of
// `HeapValue::Closure { function_id, upvalues }`. These end-to-end tests
// exercise the new variant through the full compile → VM pipeline:
//
// 1. `h6_5_raw_alloc_destructor_correctness`: a simple escaping closure
//    with an immutable int capture; verifies the result decodes as
//    `ClosureRaw` via the shim, invoking it returns the expected value,
//    and subsequent drops do not leak or double-free.
// 2. `h6_5_cross_boundary_refcount_preservation`: stores a ClosureRaw
//    into an Array (emulating escape to a container), retrieves it, and
//    invokes it. Retain/release on the array path must keep the capture
//    alive through multiple calls.
#[cfg(test)]
mod h6_5_tests {
    use crate::test_utils::eval;
    use shape_value::ValueWordExt;

    #[test]
    fn h6_5_raw_alloc_destructor_correctness() {
        // Immutable int capture — exercises the layout-driven raw path
        // in `op_make_closure`. Result must equal `n + x = 15` and the
        // runtime must not leak (tests would crash under Miri / leak
        // detector; here we rely on tier-2 ASan-free execution).
        //
        // Follows the same shape as the H6.3 regression test — the
        // closure goes through the full escape path (return from
        // `make` → `let f` → `f(5)`).
        let val = eval(
            "fn make(n: int) -> any {\n\
                 return |x| x + n\n\
             }\n\
             let f = make(10)\n\
             f(5)",
        );
        assert_eq!(val.as_i64(), Some(15));
    }

    #[test]
    fn h6_5_cross_boundary_refcount_preservation() {
        // Store the closure in an Array and invoke it through element
        // access. Each array read/write copies the NaN-boxed bits, which
        // translates to an Arc<HeapValue::ClosureRaw> refcount bump. The
        // `OwnedClosureBlock`'s `Clone` bumps the `TypedClosureHeader`
        // refcount, and its Drop releases it. If the retain/release
        // protocol is off-by-one the closure body's capture read will
        // fault or return garbage.
        let val = eval(
            "fn make(n: int) -> any {\n\
                 return |x| x + n\n\
             }\n\
             let f = make(7)\n\
             let arr = [f]\n\
             let g = arr[0]\n\
             g(3) + g(4)",
        );
        // (7 + 3) + (7 + 4) = 10 + 11 = 21
        assert_eq!(val.as_i64(), Some(21));
    }

    #[test]
    fn h6_5_multiple_immutable_captures_raw_path() {
        // Multi-capture closure: two int captures both marked
        // immutable. Exercises the raw path's typed `write_capture_typed`
        // at distinct layout offsets.
        let val = eval(
            "fn make(a: int, b: int) -> any {\n\
                 return |x| x + a + b\n\
             }\n\
             let f = make(100, 2)\n\
             f(7)",
        );
        assert_eq!(val.as_i64(), Some(109));
    }

    /// Closure spec §14.6 (H6.5): the VM producer must emit a
    /// `HeapValue::ClosureRaw` when a layout is registered and no
    /// captures are marked mutable. This is the structural proxy for
    /// "hot loop allocates no `Arc<Vec<Upvalue>>`" — the `ClosureRaw`
    /// variant's storage is a raw `TypedClosureHeader` block, not a
    /// `Vec<Upvalue>`.
    ///
    /// Full Cranelift-IR inspection for `arr.map(|x| x + n)` is gated on
    /// the JIT test harness being teachable to expose its disassembled
    /// output; this structural check is the stable companion.
    #[test]
    fn h6_5_immutable_capture_produces_raw_variant() {
        // Return the closure as the top-level result and inspect its
        // backing variant. H6.5 producer must emit `ClosureRaw` when
        // (a) a `ClosureLayout` is registered for the function_id, and
        // (b) no captures are marked mutable.
        let result = eval(
            "fn make(n: int) -> any {\n\
                 return |x| x + n\n\
             }\n\
             make(10)",
        );
        let hv = result.as_heap_ref().expect("closure result is heap-tagged");
        assert!(
            matches!(hv, shape_value::HeapValue::ClosureRaw(..)),
            "H6.5 expects ClosureRaw variant for immutable captures, got {}",
            hv.type_name()
        );
        // Sanity: shim reads the capture correctly.
        let handle = hv.as_closure_handle().expect("handle");
        let caps = handle.captures_as_values();
        assert_eq!(caps.len(), 1);
        assert_eq!(caps[0].as_i64(), Some(10));
    }

    #[test]
    fn h6_5_mutable_capture_uses_owned_mutable_raw_path() {
        // Post-A.1C.2b: the H6.5 producer's legacy fallback no longer
        // fires for `let mut` local captures — they route through the
        // A.1B OwnedMutable Raw path with a `Box::into_raw` capture
        // cell. The closure's private Box accumulates across calls;
        // return it from the last invocation to observe.
        //
        // (Track A.5 retired the legacy `HeapValue::Closure` fallback
        // entirely; every `let mut` capture now routes through the
        // OwnedMutable Raw path.)
        let val = eval(
            "fn main() -> int {\n\
                 let mut n: int = 0\n\
                 let f = |x: int| { n = n + x; n }\n\
                 f(1)\n\
                 f(2)\n\
                 f(3)\n\
             }\n\
             main()",
        );
        assert_eq!(val.as_i64(), Some(6));
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Track A.1B: op_make_closure allocation for CaptureKind::OwnedMutable /
// CaptureKind::Shared. Exercises the extended Raw path in
// `op_make_closure` that allocates `Box::into_raw` / `Arc::into_raw`
// cells when the registered `ClosureLayout` marks captures with the new
// kinds.
// ─────────────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod a1b_make_closure_tests {
    use crate::bytecode::{BytecodeProgram, Constant, Function, Instruction, OpCode, Operand};
    use crate::executor::{VMConfig, VirtualMachine};
    use shape_value::heap_value::HeapValue;
    use shape_value::v2::closure_layout::{CaptureKind, ClosureLayout, SharedCell};
    use shape_value::v2::concrete_type::ConcreteType;
    use shape_value::{ValueWord, ValueWordExt};
    use std::sync::Arc;

    /// Register a closure function and a matching ClosureLayout in the
    /// program's tables. Returns the function id.
    fn register_closure_function(
        program: &mut BytecodeProgram,
        captures_count: u16,
        mutable_captures: Vec<bool>,
        layout: ClosureLayout,
        body: Vec<Instruction>,
    ) -> u16 {
        let fid = program.functions.len() as u16;
        // Entry point is the current end of the top-level instruction
        // stream — we append the body after the top-level code that has
        // already been written (tests write top-level first, then call
        // this).
        let entry_point = program.instructions.len();
        let body_length = body.len();
        program.instructions.extend(body);
        program.functions.push(Function {
            name: format!("a1b_test_{}", fid),
            arity: 0,
            param_names: vec![],
            locals_count: captures_count,
            entry_point,
            body_length,
            is_closure: true,
            captures_count,
            is_async: false,
            ref_params: vec![],
            ref_mutates: vec![],
            mutable_captures,
            frame_descriptor: None,
            osr_entry_points: vec![],
            mir_data: None,
        });
        // Pad layouts with None up to fid.
        while program.closure_function_layouts.len() < fid as usize + 1 {
            program.closure_function_layouts.push(None);
        }
        program.closure_function_layouts[fid as usize] = Some(Arc::new(layout));
        fid
    }

    #[test]
    fn a1b_make_closure_owned_mutable_allocates_box_and_releases() {
        // Set up a program with:
        //   - top-level instructions: PushConst 42; MakeClosure(F); Halt.
        //   - a closure function F (never called) whose layout marks
        //     capture 0 as OwnedMutable (single I64).
        // The MakeClosure handler must allocate Box::new(ValueWord(42)),
        // write the raw pointer into the ClosureRaw slot, and leave a
        // live ClosureRaw on the stack. Inspect the resulting closure
        // via VmClosureHandle to verify the capture reads back as 42.
        // When the VM / stack drops, release_typed_closure reclaims the
        // Box.
        let mut program = BytecodeProgram::default();
        let c0 = program.add_constant(Constant::Int(42));

        // Top-level first (IP starts at 0 — the executor runs from the
        // first instruction until Halt).
        program.instructions.push(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(c0)),
        ));
        // MakeClosure with Function operand carrying fid — we need the
        // fid before we emit this. Do it in two steps.
        let makeclosure_placeholder_idx = program.instructions.len();
        program.instructions.push(Instruction::simple(OpCode::Halt)); // placeholder
        program.instructions.push(Instruction::simple(OpCode::Halt));

        let body = vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c0))),
            Instruction::simple(OpCode::ReturnValue),
        ];
        let layout =
            ClosureLayout::from_capture_types(&[ConcreteType::I64], &[CaptureKind::OwnedMutable]);
        let fid = register_closure_function(
            &mut program,
            1,
            vec![true], // mutable_captures[0] = true — matches A.1C's expected shape
            layout,
            body,
        );

        // Patch in the MakeClosure instruction with the resolved fid.
        program.instructions[makeclosure_placeholder_idx] = Instruction::new(
            OpCode::MakeClosure,
            Some(Operand::Function(shape_value::FunctionId::new(fid))),
        );
        // makeclosure_placeholder_idx + 1 stays as Halt.
        program.top_level_locals_count = 0;

        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(program);
        let result = vm.execute(None).unwrap().clone();

        // Result is a ClosureRaw.
        let hv = result.as_heap_ref().expect("closure on stack");
        assert!(
            matches!(hv, HeapValue::ClosureRaw(_)),
            "A.1B expects OwnedMutable-capturing closure to be ClosureRaw, got {}",
            hv.type_name()
        );
        // Wave D (D.3): the slot now holds `*mut i64` from
        // `alloc_owned_mutable_i64`, not `*mut ValueWord`. The legacy
        // `capture_as_value` widening path reads 8 bytes verbatim which
        // is sound for I64 cells (8-byte allocation) but the resulting
        // `u64` is the native i64 bit pattern, not an i48-tagged
        // ValueWord — so `as_i64()` would return `None`. Read back via
        // Wave B's typed accessor instead.
        let handle = hv.as_closure_handle().expect("handle");
        assert_eq!(handle.capture_count(), 1);
        // capture_owned_mutable_ptr returns a non-null pointer.
        let cell_ptr = handle
            .capture_owned_mutable_ptr(0)
            .expect("OwnedMutable slot");
        assert!(!cell_ptr.is_null());
        // SAFETY: the slot was produced by `alloc_owned_mutable_i64`,
        // so reading the cell as `*mut i64` is the correctly-typed
        // access. The closure block's refcount keeps the box alive.
        let value = unsafe {
            shape_value::v2::closure_raw::read_owned_mutable_i64(cell_ptr as *mut i64)
        };
        assert_eq!(value, 42);

        // Drop the result and VM — release_typed_closure will reclaim
        // the Box. If the reclaim is wrong (leaked or double-freed),
        // miri / address sanitiser catches it; without those, the test
        // at least confirms no panic on drop.
        drop(result);
        drop(vm);
    }

    /// Wave D (D.3) deliverable test: a closure with two
    /// `OwnedMutable` captures of distinct interior `FieldKind`s
    /// (I64 and F64).
    ///
    /// Verifies that:
    /// 1. `op_make_closure` dispatches per-`capture_inner_kind` and
    ///    routes the two captures to the correct
    ///    `closure_raw::alloc_owned_mutable_<kind>` helper (the box
    ///    sizes differ — `*mut i64` vs `*mut f64` — but the slot
    ///    pointer bits are 8 bytes either way).
    /// 2. The native values are stored verbatim (no NaN-boxing) and
    ///    read back through Wave B's typed
    ///    `read_owned_mutable_<kind>` helpers.
    /// 3. `release_typed_closure` reclaims both typed boxes on the
    ///    closure's last refcount release without a leak or double-
    ///    free panic on Drop.
    #[test]
    fn d3_make_closure_owned_mutable_multi_kind_typed_alloc() {
        let mut program = BytecodeProgram::default();
        let c_int = program.add_constant(Constant::Int(7));
        let c_num = program.add_constant(Constant::Number(2.5));

        // Top-level: PushConst 7 (capture 0 = i64) ;
        //            PushConst 2.5 (capture 1 = f64) ;
        //            MakeClosure(F) ; Halt.
        program.instructions.push(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(c_int)),
        ));
        program.instructions.push(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(c_num)),
        ));
        let makeclosure_placeholder_idx = program.instructions.len();
        program.instructions.push(Instruction::simple(OpCode::Halt));
        program.instructions.push(Instruction::simple(OpCode::Halt));

        // Closure body is never executed (we only inspect the resulting
        // ClosureRaw) — placeholder PushConst + ReturnValue.
        let body = vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c_int))),
            Instruction::simple(OpCode::ReturnValue),
        ];
        let layout = ClosureLayout::from_capture_types(
            &[ConcreteType::I64, ConcreteType::F64],
            &[CaptureKind::OwnedMutable, CaptureKind::OwnedMutable],
        );
        let fid = register_closure_function(
            &mut program,
            2,
            vec![true, true],
            layout,
            body,
        );
        program.instructions[makeclosure_placeholder_idx] = Instruction::new(
            OpCode::MakeClosure,
            Some(Operand::Function(shape_value::FunctionId::new(fid))),
        );
        program.top_level_locals_count = 0;

        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(program);
        let result = vm.execute(None).unwrap().clone();

        // Result is a ClosureRaw with two OwnedMutable captures.
        let hv = result.as_heap_ref().expect("closure on stack");
        assert!(
            matches!(hv, HeapValue::ClosureRaw(_)),
            "D.3 expects multi-kind OwnedMutable closure to be ClosureRaw, got {}",
            hv.type_name()
        );
        let handle = hv.as_closure_handle().expect("handle");
        assert_eq!(handle.capture_count(), 2);

        // Capture 0: I64 — read back via the typed helper.
        let cell_0 = handle
            .capture_owned_mutable_ptr(0)
            .expect("OwnedMutable slot 0");
        assert!(!cell_0.is_null());
        // SAFETY: the slot was produced by `alloc_owned_mutable_i64`,
        // so reading the cell as `*mut i64` is the correctly-typed
        // access. The closure block's refcount keeps the box alive.
        let v0 = unsafe {
            shape_value::v2::closure_raw::read_owned_mutable_i64(cell_0 as *mut i64)
        };
        assert_eq!(v0, 7, "I64 capture should round-trip through the typed cell");

        // Capture 1: F64 — read back via the typed helper.
        let cell_1 = handle
            .capture_owned_mutable_ptr(1)
            .expect("OwnedMutable slot 1");
        assert!(!cell_1.is_null());
        // SAFETY: the slot was produced by `alloc_owned_mutable_f64`;
        // reading as `*mut f64` is the correctly-typed access.
        let v1 = unsafe {
            shape_value::v2::closure_raw::read_owned_mutable_f64(cell_1 as *mut f64)
        };
        assert_eq!(v1, 2.5, "F64 capture should round-trip through the typed cell");

        // Refcount before drop is 1 (the stack share). After dropping
        // result, `release_typed_closure` reclaims the block via
        // `drop_owned_mutable_capture` per slot — which dispatches on
        // `capture_inner_kind` and reconstructs the typed `Box<T>`.
        // A mismatched dispatch (e.g. `Box<u64>::from_raw` on a
        // `Box<f64>` allocation) would corrupt the allocator; running
        // under address sanitiser / miri catches it. Without those,
        // the test at least confirms no panic on drop.
        assert_eq!(handle.refcount(), 1);
        drop(result);
        drop(vm);
    }

    // Track A.1C.2: the A.1B `a1b_make_closure_shared_allocates_arc_and_
    // releases` test previously exercised op_make_closure's Shared
    // branch by pushing an initial VALUE and expecting the branch to
    // allocate a fresh Arc. A.1C.2 flipped the contract: the Shared
    // branch now expects the stack bits to be a pre-existing
    // `*const SharedCell` (produced by `AllocSharedLocal` at the
    // closure-capture site) and performs `Arc::increment_strong_count`
    // to give the closure its own strong-count share.
    //
    // End-to-end coverage for the new contract lives in
    // `test_a1c_var_multi_closure_e2e_shared_observes_writes` (two
    // closures capturing the same `var x` observe each other's writes
    // via the Arc).
}

// ─────────────────────────────────────────────────────────────────────────────
// Wave E+3: typed `ReturnValue<Kind>` round-trip tests
// ─────────────────────────────────────────────────────────────────────────────
//
// One test per FieldKind from the task brief: I64, F64, Bool, Ptr.
// Each test builds a small program of the form
//
//     PushConst <value>
//     PushConst <arg_count = 0>
//     Call func_0
//     Jump <past func body>
//     [func_0 body]:
//         PushConst <value>
//         <ReturnValue<Kind>>
//
// and verifies the value the typed handler pushed onto the caller's
// stack survives frame cleanup with the correct bits. Because the
// typed handler's body is identical to the legacy `op_return_value`
// (the encoded `<Kind>` is a static-only annotation), the test's
// purpose is to verify dispatch is wired correctly, not to test
// distinct runtime behaviour.
#[cfg(test)]
mod e3_typed_return_value_tests {
    use crate::bytecode::{BytecodeProgram, Constant, Function, Instruction, OpCode, Operand};
    use crate::executor::{VMConfig, VirtualMachine};
    use shape_value::heap_value::HeapValue;
    use shape_value::{FunctionId, ValueWord, ValueWordExt};

    /// Helper: build a tiny function that pushes `body_const_idx` and
    /// returns via the given typed-return opcode. The caller code is:
    /// PushConst(arg_count=0), Call(func_0), Halt.
    ///
    /// Returns the executed program's top-of-stack ValueWord.
    fn run_typed_return(typed_opcode: OpCode, body_const: Constant) -> ValueWord {
        let mut program = BytecodeProgram::default();
        // Caller-side constants:
        //   slot 0: arg_count = 0 (as Int — Call pops via as_number_coerce)
        let c_argc = program.add_constant(Constant::Int(0));
        // Function-body constant:
        //   slot 1: the value the function returns.
        let c_body = program.add_constant(body_const);

        // Emit caller-side instructions (top-level).
        //   0: PushConst(c_argc)         -- arg count
        //   1: Call func_0               -- return_ip = 2
        //   2: Jump +2                   -- skip over func body (3, 4)
        //   3..: func body
        //   <last>: Halt
        program
            .instructions
            .push(Instruction::new(OpCode::PushConst, Some(Operand::Const(c_argc))));
        program
            .instructions
            .push(Instruction::new(OpCode::Call, Some(Operand::Function(FunctionId(0)))));
        // Jump offset is computed relative to ip *after* dispatch (i.e.
        // ip=3 once Jump executes; we want to land at the Halt at ip=5).
        program
            .instructions
            .push(Instruction::new(OpCode::Jump, Some(Operand::Offset(2))));

        // Function body (entry_point = 3):
        //   3: PushConst(c_body)
        //   4: <typed_opcode>
        let entry_point = program.instructions.len();
        program
            .instructions
            .push(Instruction::new(OpCode::PushConst, Some(Operand::Const(c_body))));
        program.instructions.push(Instruction::simple(typed_opcode));
        let body_length = program.instructions.len() - entry_point;

        // Trailing Halt at top level.
        program.instructions.push(Instruction::simple(OpCode::Halt));

        program.functions.push(Function {
            name: "__e3_typed_return_test".to_string(),
            arity: 0,
            param_names: vec![],
            locals_count: 0,
            entry_point,
            body_length,
            is_closure: false,
            captures_count: 0,
            is_async: false,
            ref_params: vec![],
            ref_mutates: vec![],
            mutable_captures: vec![],
            frame_descriptor: None,
            osr_entry_points: vec![],
            mir_data: None,
        });
        program.top_level_locals_count = 0;

        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(program);
        vm.execute(None).expect("execute").clone()
    }

    #[test]
    fn return_value_i64_round_trip() {
        // ReturnValueI64 (0x198) — the function returns 12345 as an i64.
        // The handler shares its body with the legacy ReturnValue: pop 1
        // raw u64, frame-clean, push 1 onto the caller's stack. Verifies
        // dispatch fires and the bits survive frame teardown.
        let result = run_typed_return(OpCode::ReturnValueI64, Constant::Int(12345));
        assert_eq!(result.as_i64(), Some(12345));
    }

    #[test]
    fn return_value_f64_round_trip() {
        // ReturnValueF64 (0x19A) — the function returns 2.71828 as an f64.
        let result = run_typed_return(OpCode::ReturnValueF64, Constant::Number(2.71828));
        assert!(
            (result.as_f64().expect("f64") - 2.71828).abs() < 1e-12,
            "f64 should round-trip through ReturnValueF64"
        );
    }

    #[test]
    fn return_value_bool_round_trip() {
        // ReturnValueBool (0x1A1) — the function returns `true`.
        let result = run_typed_return(OpCode::ReturnValueBool, Constant::Bool(true));
        assert_eq!(result.as_bool(), Some(true));
    }

    #[test]
    fn return_value_ptr_round_trip() {
        // ReturnValuePtr (0x1A2) — the function returns a string heap
        // value. Ownership transfer is by raw bit-level pass-through,
        // so the caller-side ValueWord must still resolve to the same
        // string after frame cleanup. If frame cleanup dropped the bits
        // before pushing them back, the heap allocation would be freed
        // and the assertion would fail (or trip the leak detector under
        // miri / ASan).
        let result = run_typed_return(
            OpCode::ReturnValuePtr,
            Constant::String("e3-return-ptr".to_string()),
        );
        let hv = result
            .as_heap_ref()
            .expect("ReturnValuePtr should preserve the heap-tagged value");
        match hv {
            HeapValue::String(s) => assert_eq!(s.as_str(), "e3-return-ptr"),
            other => panic!("Expected HeapValue::String, got {}", other.type_name()),
        }
    }
}
