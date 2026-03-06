//! Result/Option helper opcodes used by control-flow lowering.

use cranelift::prelude::*;

use crate::nan_boxing::*;
use crate::translator::types::BytecodeToIR;

impl<'a, 'b> BytecodeToIR<'a, 'b> {
    /// IsOk: check if value is Ok, push boolean result.
    pub(crate) fn compile_is_ok(&mut self) -> Result<(), String> {
        if let Some(value) = self.stack_pop() {
            let inst = self.builder.ins().call(self.ffi.is_ok, &[value]);
            let result = self.builder.inst_results(inst)[0];
            self.stack_push_typed(result, shape_vm::type_tracking::StorageHint::Bool);
        } else {
            let false_val = self.builder.ins().iconst(types::I64, TAG_BOOL_FALSE as i64);
            self.stack_push_typed(false_val, shape_vm::type_tracking::StorageHint::Bool);
        }
        Ok(())
    }

    /// IsErr: check if value is Err, push boolean result.
    pub(crate) fn compile_is_err(&mut self) -> Result<(), String> {
        if let Some(value) = self.stack_pop() {
            let inst = self.builder.ins().call(self.ffi.is_err, &[value]);
            let result = self.builder.inst_results(inst)[0];
            self.stack_push_typed(result, shape_vm::type_tracking::StorageHint::Bool);
        } else {
            let false_val = self.builder.ins().iconst(types::I64, TAG_BOOL_FALSE as i64);
            self.stack_push_typed(false_val, shape_vm::type_tracking::StorageHint::Bool);
        }
        Ok(())
    }

    /// UnwrapOk: extract inner value from Ok, return NULL if not Ok.
    pub(crate) fn compile_unwrap_ok(&mut self) -> Result<(), String> {
        if let Some(value) = self.stack_pop() {
            let inst = self.builder.ins().call(self.ffi.unwrap_ok, &[value]);
            let result = self.builder.inst_results(inst)[0];
            self.stack_push(result);
        } else {
            let null_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
            self.stack_push(null_val);
        }
        Ok(())
    }

    /// UnwrapErr: extract inner value from Err, return NULL if not Err.
    pub(crate) fn compile_unwrap_err(&mut self) -> Result<(), String> {
        if let Some(value) = self.stack_pop() {
            let inst = self.builder.ins().call(self.ffi.unwrap_err, &[value]);
            let result = self.builder.inst_results(inst)[0];
            self.stack_push(result);
        } else {
            let null_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
            self.stack_push(null_val);
        }
        Ok(())
    }

    /// UnwrapOption: extract inner value from Some, return NULL for None.
    /// Used for pattern matching in match expressions.
    pub(crate) fn compile_unwrap_option(&mut self) -> Result<(), String> {
        if let Some(value) = self.stack_pop() {
            // Returns TAG_NULL when not a Some value.
            let inst = self.builder.ins().call(self.ffi.unwrap_some, &[value]);
            let inner_value = self.builder.inst_results(inst)[0];
            self.stack_push(inner_value);
        } else {
            let null_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
            self.stack_push(null_val);
        }

        Ok(())
    }
}
