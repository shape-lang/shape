//! Control flow operations for the VM executor
//!
//! Handles: Jump, JumpIfFalse, JumpIfTrue, Call, CallValue, CallForeign, Return, ReturnValue

pub mod foreign_marshal;
pub mod jit_abi;
pub mod native_abi;

use crate::{
    bytecode::{Instruction, OpCode, Operand},
    executor::{ForeignFunctionHandle, VirtualMachine},
};
use shape_value::{VMError, ValueWord};

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
            CallForeign => self.op_call_foreign(instruction)?,
            Return => self.op_return()?,
            ReturnValue => self.op_return_value()?,
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
            let condition = self.pop_vw()?.is_truthy();
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
    /// Uses direct bool check instead of full `is_truthy()` dispatch.
    #[inline(always)]
    pub(in crate::executor) fn op_jump_if_false_trusted(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        if let Some(Operand::Offset(offset)) = instruction.operand {
            let cond = self.pop_vw()?;
            debug_assert!(cond.is_bool(), "Trusted JumpIfFalse invariant violated");
            if !unsafe { cond.as_bool_unchecked() } {
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
            let condition = self.pop_vw()?.is_truthy();
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
            .pop_vw()?
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
                    let mut param_kinds = [crate::type_tracking::SlotKind::Unknown; 64];
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
                    let mut jit_locals = [0u64; 64];
                    let args_base = self.sp.saturating_sub(arg_count);
                    for i in 0..copy_count {
                        jit_locals[i] =
                            jit_abi::marshal_arg_to_jit(&self.stack[args_base + i], param_kinds[i]);
                    }

                    // Build a minimal JIT context on the stack matching the
                    // JITContext layout (from shape-jit/src/context.rs):
                    //   locals:    byte 64  (u64 index 8)
                    //   stack:     byte 576 (u64 index 72)
                    //   stack_ptr: byte 1600 (u64 index 200)
                    //   total: ~1728 bytes = 216 u64s
                    const CTX_U64_SIZE: usize = 216;
                    const LOCALS_U64_OFFSET: usize = 8; // byte 64 / 8
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
                                    self.stack.resize_with(needed * 2 + 1, ValueWord::none);
                                }
                                // Zero-init local slots (deopt_with_info overwrites the live ones)
                                for i in 0..locals_count {
                                    self.stack[bp + i] = ValueWord::none();
                                }

                                let blob_hash = self.blob_hash_for_function(func_id_u16);
                                self.call_stack.push(super::super::CallFrame {
                                    return_ip: self.ip,
                                    base_pointer: bp,
                                    locals_count,
                                    function_id: Some(func_id_u16),
                                    upvalues: None,
                                    blob_hash,
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
                        self.stack[i] = ValueWord::none();
                    }
                    self.sp = args_base;

                    self.push_vw(result_vw)?;
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

    pub(in crate::executor) fn op_call_value(&mut self) -> Result<(), VMError> {
        let arg_count = self
            .pop_vw()?
            .as_number_coerce()
            .ok_or_else(|| VMError::RuntimeError("Expected number for arg count".to_string()))?
            as usize;

        // Stack layout: [callee, arg0, arg1, ..., argN-1]
        // Peek at the callee (below the args) to choose the dispatch path.
        let callee_idx = self
            .sp
            .checked_sub(arg_count + 1)
            .ok_or(VMError::StackUnderflow)?;
        let callee_nb = self.stack[callee_idx].clone();

        // NanTag dispatch (no ValueWord materialization)
        use shape_value::NanTag;
        match callee_nb.tag() {
            NanTag::Function => {
                let func_id = callee_nb.as_function().ok_or(VMError::InvalidCall)?;
                // Swap-down: shift args down by one to overwrite the callee slot
                for i in callee_idx..callee_idx + arg_count {
                    self.stack[i] = self.stack[i + 1].clone();
                }
                self.sp -= 1;
                self.stack[self.sp] = ValueWord::none();
                self.call_function_from_stack(func_id, arg_count)
            }
            NanTag::ModuleFunction => {
                let func_id = callee_nb.as_module_function().ok_or(VMError::InvalidCall)?;
                let args_base = callee_idx + 1;
                let mut args_nb: Vec<ValueWord> = Vec::with_capacity(arg_count);
                for i in 0..arg_count {
                    args_nb.push(std::mem::replace(
                        &mut self.stack[args_base + i],
                        ValueWord::none(),
                    ));
                }
                self.stack[callee_idx] = ValueWord::none();
                self.sp = callee_idx;
                let module_fn = self.module_fn_table.get(func_id).cloned().ok_or_else(|| {
                    VMError::RuntimeError(format!(
                        "Module function ID {} not found in registry",
                        func_id
                    ))
                })?;
                let result_nb = self.invoke_module_fn(&module_fn, &args_nb)?;
                self.push_vw(result_nb)?;
                Ok(())
            }
            NanTag::Heap => {
                use shape_value::heap_value::HeapValue;
                match callee_nb.as_heap_ref() {
                    Some(HeapValue::Closure {
                        function_id,
                        upvalues,
                    }) => {
                        let function_id = *function_id;
                        let upvalues = upvalues.clone();
                        // Collect args as ValueWord, then remove callee
                        let args_base = callee_idx + 1;
                        let mut args_nb = Vec::with_capacity(arg_count);
                        for i in 0..arg_count {
                            args_nb.push(std::mem::replace(
                                &mut self.stack[args_base + i],
                                ValueWord::none(),
                            ));
                        }
                        self.stack[callee_idx] = ValueWord::none();
                        self.sp = callee_idx;
                        self.call_closure_with_nb_args(function_id, upvalues, &args_nb)
                    }
                    Some(HeapValue::HostClosure(callable)) => {
                        let callable = callable.clone();
                        let args_base = callee_idx + 1;
                        let mut args_nb: Vec<ValueWord> = Vec::with_capacity(arg_count);
                        for i in 0..arg_count {
                            args_nb.push(std::mem::replace(
                                &mut self.stack[args_base + i],
                                ValueWord::none(),
                            ));
                        }
                        self.stack[callee_idx] = ValueWord::none();
                        self.sp = callee_idx;
                        let result_nb = callable.call(&args_nb).map_err(VMError::RuntimeError)?;
                        self.push_vw(result_nb)?;
                        Ok(())
                    }
                    _ => Err(VMError::InvalidCall),
                }
            }
            _ => Err(VMError::InvalidCall),
        }
    }

    pub(in crate::executor) fn op_make_closure(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use shape_value::Upvalue;

        if let Some(Operand::Function(func_id)) = instruction.operand {
            let function = self
                .program
                .functions
                .get(func_id.index())
                .ok_or(VMError::InvalidOperand)?;
            let capture_count = function.captures_count as usize;
            let mutable_captures = function.mutable_captures.clone();
            let mut upvalues = Vec::with_capacity(capture_count);
            for _ in 0..capture_count {
                let nb = self.pop_vw()?;
                upvalues.push(nb);
            }
            upvalues.reverse();

            // Create upvalues: mutable captures reuse SharedCell Arcs for shared state,
            // immutable captures get direct ValueWord values (no overhead).
            let upvalues: Vec<Upvalue> = upvalues
                .into_iter()
                .enumerate()
                .map(|(i, nb)| {
                    if mutable_captures.get(i).copied().unwrap_or(false) {
                        // If the captured value is a SharedCell (boxed by BoxLocal),
                        // extract and reuse its Arc so the closure and enclosing scope
                        // share the same mutable cell.
                        if let Some(shape_value::heap_value::HeapValue::SharedCell(arc)) =
                            nb.as_heap_ref()
                        {
                            Upvalue::Mutable(arc.clone())
                        } else {
                            Upvalue::new_mutable(nb)
                        }
                    } else {
                        Upvalue::new(nb)
                    }
                })
                .collect();

            self.push_vw(ValueWord::from_heap_value(
                shape_value::heap_value::HeapValue::Closure {
                    function_id: func_id.0,
                    upvalues,
                },
            ))?;
            Ok(())
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
            .pop_vw()?
            .as_number_coerce()
            .ok_or_else(|| VMError::RuntimeError("Expected number for arg count".to_string()))?
            as usize;

        // Pop args in reverse order then reverse for correct ordering
        let mut args = Vec::with_capacity(arg_count);
        for _ in 0..arg_count {
            args.push(self.pop_vw()?);
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
                                    self.push_vw(ValueWord::from_ok(result))?;
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
                                    self.push_vw(ValueWord::from_err(err_obj))?;
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
                            self.push_vw(ValueWord::from_err(err_obj))?;
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
                    self.push_vw(result)?;
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
                let result = native_abi::invoke_linked_function(
                    &linked,
                    &args,
                    Some(raw_invoker),
                    Some(&mut self.stack[..live_stack_len]),
                )
                .map_err(|e| {
                    VMError::RuntimeError(format!("Native function '{}' error: {}", name, e))
                })?;
                self.push_vw(result)?;
            }
        }

        Ok(())
    }

    // Return operations

    pub(in crate::executor) fn op_return(&mut self) -> Result<(), VMError> {
        if let Some(frame) = self.call_stack.pop() {
            // Restore instruction pointer
            self.ip = frame.return_ip;

            // Clean up register window: clear slots and restore sp to base_pointer
            let bp = frame.base_pointer;
            for i in bp..self.sp {
                self.stack[i] = ValueWord::none();
            }
            self.sp = bp;
        } else {
            // Return from main
            self.ip = self.program.instructions.len();
        }
        Ok(())
    }

    pub(in crate::executor) fn op_return_value(&mut self) -> Result<(), VMError> {
        let return_value = self.pop_vw()?;

        if let Some(frame) = self.call_stack.pop() {
            // Restore instruction pointer
            self.ip = frame.return_ip;

            // Clean up register window: clear slots and restore sp to base_pointer
            let bp = frame.base_pointer;
            for i in bp..self.sp {
                self.stack[i] = ValueWord::none();
            }
            self.sp = bp;

            // Push return value
            self.push_vw(return_value)?;
        } else {
            // Return from main
            self.push_vw(return_value)?;
            self.ip = self.program.instructions.len();
        }
        Ok(())
    }
}
