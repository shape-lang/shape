//! Async opcode translation: SpawnTask, JoinInit, JoinAwait, CancelTask,
//! AsyncScopeEnter, AsyncScopeExit.
//!
//! Each opcode is compiled to a call to the corresponding FFI function.
//! JoinAwait additionally checks the suspension state to determine whether
//! the JIT function should return early (handing control to the interpreter).

use cranelift::prelude::*;
use shape_vm::bytecode::{Instruction, Operand};

use crate::context::STACK_PTR_OFFSET;
use crate::ffi::async_ops::SUSPENSION_ASYNC_WAIT;
use crate::translator::types::BytecodeToIR;

impl<'a, 'b> BytecodeToIR<'a, 'b> {
    /// SpawnTask: pop callable, push Future(task_id)
    ///
    /// Bytecode: pops 1 (callable), pushes 1 (future)
    pub(crate) fn compile_spawn_task(&mut self, _instr: &Instruction) -> Result<(), String> {
        // Pop callable from typed stack
        let callable_val = self.stack_pop().unwrap();

        // Call jit_spawn_task(ctx, callable_bits) -> future_bits
        let call = self
            .builder
            .ins()
            .call(self.ffi.spawn_task, &[self.ctx_ptr, callable_val]);
        let result = self.builder.inst_results(call)[0];

        self.stack_push(result);
        Ok(())
    }

    /// JoinInit: pop `arity` futures from stack (via FFI), push TaskGroup
    ///
    /// Bytecode: pops 0 (FFI pops from JIT stack directly), pushes 1 (TaskGroup)
    /// Operand: Count(packed_u16) where high 2 bits = join kind, low 14 bits = arity
    pub(crate) fn compile_join_init(&mut self, instr: &Instruction) -> Result<(), String> {
        let packed = match &instr.operand {
            Some(Operand::Count(n)) => *n,
            _ => {
                return Err("JoinInit requires Count operand".to_string());
            }
        };

        let arity = (packed & 0x3FFF) as usize;

        // The FFI function reads futures directly from ctx.stack, so we need to
        // flush our compile-time stack to the context stack first.
        // Pop `arity` values from our typed stack and push them onto ctx.stack.
        let mut future_vals = Vec::with_capacity(arity);
        for _ in 0..arity {
            if let Some(val) = self.stack_pop() {
                future_vals.push(val);
            }
        }
        // Push them to ctx.stack in original order (reverse of pop order)
        future_vals.reverse();
        for &val in &future_vals {
            self.emit_ctx_stack_push(val);
        }

        // Call jit_join_init(ctx, packed) -> TaskGroup bits
        let packed_val = self.builder.ins().iconst(types::I16, packed as i64);
        let call = self
            .builder
            .ins()
            .call(self.ffi.join_init, &[self.ctx_ptr, packed_val]);
        let result = self.builder.inst_results(call)[0];

        self.stack_push(result);
        Ok(())
    }

    /// JoinAwait: pop TaskGroup, suspend if needed, push result
    ///
    /// Bytecode: pops 1 (TaskGroup), pushes 1 (result or suspension)
    ///
    /// If the FFI function sets suspension_state to SUSPENSION_ASYNC_WAIT,
    /// the JIT function returns a special signal code so the execution loop
    /// can hand control back to the interpreter.
    pub(crate) fn compile_join_await(&mut self, _instr: &Instruction) -> Result<(), String> {
        // Pop task group from typed stack
        let tg_val = self.stack_pop().unwrap();

        // Call jit_join_await(ctx, task_group_bits) -> result
        let call = self
            .builder
            .ins()
            .call(self.ffi.join_await, &[self.ctx_ptr, tg_val]);
        let result = self.builder.inst_results(call)[0];

        // Check suspension_state in ctx for SUSPENSION_ASYNC_WAIT
        // If suspended, return special signal from the JIT function.
        let suspension_offset = offset_of_suspension_state();
        let suspension_addr = self
            .builder
            .ins()
            .iadd_imm(self.ctx_ptr, suspension_offset as i64);
        let suspension_val =
            self.builder
                .ins()
                .load(types::I32, MemFlags::trusted(), suspension_addr, 0);
        let async_wait_const = self
            .builder
            .ins()
            .iconst(types::I32, SUSPENSION_ASYNC_WAIT as i64);
        let is_suspended = self
            .builder
            .ins()
            .icmp(IntCC::Equal, suspension_val, async_wait_const);

        // Create continuation and suspension blocks
        let continue_block = self.builder.create_block();
        let suspend_block = self.builder.create_block();

        self.builder
            .ins()
            .brif(is_suspended, suspend_block, &[], continue_block, &[]);

        // Suspend block: return special signal (negative value signals suspension)
        self.builder.switch_to_block(suspend_block);
        self.builder.seal_block(suspend_block);
        // Return -2 to signal async suspension to the execution loop
        let suspend_signal = self.builder.ins().iconst(types::I32, -2i64);
        self.builder.ins().return_(&[suspend_signal]);

        // Continue block: push result and continue
        self.builder.switch_to_block(continue_block);
        self.builder.seal_block(continue_block);

        self.stack_push(result);
        Ok(())
    }

    /// CancelTask: pop Future, cancel it
    ///
    /// Bytecode: pops 1 (future), pushes 0
    pub(crate) fn compile_cancel_task(&mut self, _instr: &Instruction) -> Result<(), String> {
        let future_val = self.stack_pop().unwrap();

        // Call jit_cancel_task(ctx, future_bits) -> i32
        self.builder
            .ins()
            .call(self.ffi.cancel_task, &[self.ctx_ptr, future_val]);

        Ok(())
    }

    /// AsyncScopeEnter: push new scope onto async_scope_stack
    ///
    /// Bytecode: pops 0, pushes 0
    pub(crate) fn compile_async_scope_enter(&mut self, _instr: &Instruction) -> Result<(), String> {
        // Call jit_async_scope_enter(ctx) -> i32
        self.builder
            .ins()
            .call(self.ffi.async_scope_enter, &[self.ctx_ptr]);

        Ok(())
    }

    /// AsyncScopeExit: pop scope, cancel remaining tasks
    ///
    /// Bytecode: pops 0, pushes 0
    pub(crate) fn compile_async_scope_exit(&mut self, _instr: &Instruction) -> Result<(), String> {
        // Call jit_async_scope_exit(ctx) -> i32
        self.builder
            .ins()
            .call(self.ffi.async_scope_exit, &[self.ctx_ptr]);

        Ok(())
    }

    /// Push a value onto the JIT context stack (ctx.stack[ctx.stack_ptr++] = val).
    ///
    /// Used by JoinInit to flush futures from the compile-time stack to the
    /// runtime context stack before calling the FFI function.
    fn emit_ctx_stack_push(&mut self, val: Value) {
        // Load ctx.stack_ptr
        let sp_addr = self
            .builder
            .ins()
            .iadd_imm(self.ctx_ptr, STACK_PTR_OFFSET as i64);
        let sp_val = self
            .builder
            .ins()
            .load(types::I64, MemFlags::trusted(), sp_addr, 0);

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

/// Calculate the byte offset of `suspension_state` within JITContext.
fn offset_of_suspension_state() -> i32 {
    std::mem::offset_of!(crate::context::JITContext, suspension_state) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_suspension_state_offset() {
        // Verify the offset is reasonable (should be somewhere in the struct)
        let offset = offset_of_suspension_state();
        assert!(offset > 0);
        assert!((offset as usize) < std::mem::size_of::<crate::context::JITContext>());
    }
}
