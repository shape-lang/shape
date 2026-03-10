//! Built-in function calls: math, array, type checking, higher-order functions
//!
//! This module is organized into sub-modules by function category for maintainability.
//! Builtins not handled by a dedicated JIT path fall through to the generic
//! builtin FFI trampoline (`jit_generic_builtin`), which dispatches to the
//! VM runtime at the cost of a single FFI call.

mod array;
mod array_builtins;
mod control;
mod math;
mod time;
mod types;

use crate::context::STACK_PTR_OFFSET;
use crate::translator::types::BytecodeToIR;
use shape_vm::bytecode::{Instruction, Operand};

use cranelift::prelude::{types as cl_types, *};

impl<'a, 'b> BytecodeToIR<'a, 'b> {
    /// Main dispatch for builtin function compilation
    #[inline(always)]
    pub(crate) fn compile_builtin_call(
        &mut self,
        instr: &Instruction,
        idx: usize,
    ) -> Result<(), String> {
        if let Some(Operand::Builtin(builtin)) = &instr.operand {
            // Try each category of builtins (dedicated JIT lowering)
            let handled = self.compile_math_builtin(builtin, idx)
                || self.compile_type_builtin(builtin)
                || self.compile_array_builtin(builtin, idx)
                || self.compile_series_builtin(builtin)
                || self.compile_time_builtin(builtin)
                || self.compile_control_builtin(builtin, idx);

            if !handled {
                // Fallback: dispatch via generic builtin FFI trampoline.
                // This handles ALL remaining builtins at the cost of one FFI call.
                self.compile_generic_builtin_call(builtin, idx)?;
            }
        }
        Ok(())
    }

    /// Compile a builtin call via the generic FFI trampoline.
    ///
    /// Protocol:
    /// 1. Pop arg_count (top of JIT typed stack)
    /// 2. Pop N args from JIT typed stack
    /// 3. Flush args to ctx.stack (so the FFI function can read them)
    /// 4. Call jit_generic_builtin(ctx, builtin_id, arg_count)
    /// 5. Push result onto JIT typed stack
    fn compile_generic_builtin_call(
        &mut self,
        builtin: &shape_vm::bytecode::BuiltinFunction,
        idx: usize,
    ) -> Result<(), String> {
        // Get compile-time arg count from the preceding PushConst
        let arg_count = self.get_arg_count_from_prev_instruction(idx);

        // Pop the arg_count value from the typed stack
        self.stack_pop();

        // Pop args from the typed stack
        let mut arg_vals = Vec::with_capacity(arg_count);
        for _ in 0..arg_count {
            if let Some(val) = self.stack_pop() {
                arg_vals.push(val);
            }
        }

        // Flush args to ctx.stack in original order (reverse of pop order)
        arg_vals.reverse();
        for &val in &arg_vals {
            self.emit_ctx_stack_push_inline(val);
        }

        // Also push arg_count as a number onto ctx.stack (the FFI function's
        // dispatch reads it the same way the interpreter does)
        let arg_count_boxed = crate::nan_boxing::box_number(arg_count as f64);
        let arg_count_val = self
            .builder
            .ins()
            .iconst(cl_types::I64, arg_count_boxed as i64);
        self.emit_ctx_stack_push_inline(arg_count_val);

        // Call jit_generic_builtin(ctx, builtin_id, arg_count)
        let builtin_id = *builtin as u16;
        let builtin_id_val = self.builder.ins().iconst(cl_types::I16, builtin_id as i64);
        let arg_count_i16 = self
            .builder
            .ins()
            .iconst(cl_types::I16, (arg_count + 1) as i64); // +1 for the arg_count value itself
        let call = self.builder.ins().call(
            self.ffi.generic_builtin,
            &[self.ctx_ptr, builtin_id_val, arg_count_i16],
        );
        let result = self.builder.inst_results(call)[0];

        self.stack_push(result);
        Ok(())
    }

    /// Push a value onto the JIT context stack (ctx.stack[ctx.stack_ptr++] = val).
    ///
    /// Used by the generic builtin trampoline to flush args from the compile-time
    /// stack to the runtime context stack before calling the FFI function.
    fn emit_ctx_stack_push_inline(&mut self, val: Value) {
        // Load ctx.stack_ptr
        let sp_addr = self
            .builder
            .ins()
            .iadd_imm(self.ctx_ptr, STACK_PTR_OFFSET as i64);
        let sp_val = self
            .builder
            .ins()
            .load(cl_types::I64, MemFlags::trusted(), sp_addr, 0);

        // Calculate stack slot address: ctx + STACK_OFFSET + sp * 8
        let stack_base = self
            .builder
            .ins()
            .iadd_imm(self.ctx_ptr, crate::context::STACK_OFFSET as i64);
        let byte_offset = self.builder.ins().imul_imm(sp_val, 8);
        let slot_addr = self.builder.ins().iadd(stack_base, byte_offset);

        // Store value
        self.builder
            .ins()
            .store(MemFlags::trusted(), val, slot_addr, 0);

        // Increment stack pointer
        let new_sp = self.builder.ins().iadd_imm(sp_val, 1);
        self.builder
            .ins()
            .store(MemFlags::trusted(), new_sp, sp_addr, 0);
    }
}
