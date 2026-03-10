//! Type checking builtin functions for JIT compilation

use cranelift::prelude::*;

use crate::nan_boxing::*;
use crate::translator::types::BytecodeToIR;
use shape_vm::bytecode::BuiltinFunction;

impl<'a, 'b> BytecodeToIR<'a, 'b> {
    /// Compile type checking builtin functions
    #[inline(always)]
    pub(super) fn compile_type_builtin(&mut self, builtin: &BuiltinFunction) -> bool {
        match builtin {
            BuiltinFunction::TypeOf => {
                if let Some(val) = self.stack_pop() {
                    let inst = self.builder.ins().call(self.ffi.type_of, &[val]);
                    let result = self.builder.inst_results(inst)[0];
                    self.stack_push(result);
                }
                true
            }
            BuiltinFunction::IsNumber => {
                self.stack_pop();
                if let Some(val) = self.stack_pop() {
                    let nan_base = self.builder.ins().iconst(types::I64, NAN_BASE as i64);
                    let masked = self.builder.ins().band(val, nan_base);
                    let is_num = self.builder.ins().icmp(IntCC::NotEqual, masked, nan_base);
                    let true_val = self.builder.ins().iconst(types::I64, TAG_BOOL_TRUE as i64);
                    let false_val = self.builder.ins().iconst(types::I64, TAG_BOOL_FALSE as i64);
                    let result = self.builder.ins().select(is_num, true_val, false_val);
                    self.stack_push(result);
                }
                true
            }
            BuiltinFunction::IsString => {
                self.stack_pop();
                if let Some(val) = self.stack_pop() {
                    let is_string = self.emit_is_heap_kind(val, HK_STRING);
                    let result = self.emit_boxed_bool_from_i1(is_string);
                    self.stack_push(result);
                }
                true
            }
            BuiltinFunction::IsBool => {
                self.stack_pop();
                if let Some(val) = self.stack_pop() {
                    let true_tag = self.builder.ins().iconst(types::I64, TAG_BOOL_TRUE as i64);
                    let false_tag = self.builder.ins().iconst(types::I64, TAG_BOOL_FALSE as i64);
                    let is_true = self.builder.ins().icmp(IntCC::Equal, val, true_tag);
                    let is_false = self.builder.ins().icmp(IntCC::Equal, val, false_tag);
                    let is_bool = self.builder.ins().bor(is_true, is_false);
                    let true_val = self.builder.ins().iconst(types::I64, TAG_BOOL_TRUE as i64);
                    let false_val = self.builder.ins().iconst(types::I64, TAG_BOOL_FALSE as i64);
                    let result = self.builder.ins().select(is_bool, true_val, false_val);
                    self.stack_push(result);
                }
                true
            }
            BuiltinFunction::IsArray => {
                self.stack_pop();
                if let Some(val) = self.stack_pop() {
                    let is_array = self.emit_is_heap_kind(val, HK_ARRAY);
                    let result = self.emit_boxed_bool_from_i1(is_array);
                    self.stack_push(result);
                }
                true
            }
            BuiltinFunction::IsObject => {
                self.stack_pop();
                if let Some(val) = self.stack_pop() {
                    let is_object = self.emit_is_heap_kind(val, HK_TYPED_OBJECT);
                    let result = self.emit_boxed_bool_from_i1(is_object);
                    self.stack_push(result);
                }
                true
            }

            BuiltinFunction::OkCtor => {
                self.stack_pop(); // arg count
                if let Some(val) = self.stack_pop() {
                    let inst = self.builder.ins().call(self.ffi.make_ok, &[val]);
                    let result = self.builder.inst_results(inst)[0];
                    self.stack_push(result);
                }
                true
            }
            BuiltinFunction::ErrCtor => {
                self.stack_pop(); // arg count
                if let Some(val) = self.stack_pop() {
                    let inst = self.builder.ins().call(self.ffi.make_err, &[val]);
                    let result = self.builder.inst_results(inst)[0];
                    self.stack_push(result);
                }
                true
            }
            BuiltinFunction::SomeCtor => {
                self.stack_pop(); // arg count
                if let Some(val) = self.stack_pop() {
                    // Some(x) just returns x — identity wrapper
                    self.stack_push(val);
                }
                true
            }

            BuiltinFunction::FormatValueWithMeta => {
                // FormatValueWithMeta(value) -> string representation of value
                // Used by f-string interpolation: f"text {expr}"
                // In JIT, we handle this as a simple toString conversion.
                self.stack_pop(); // pop arg_count
                if let Some(val) = self.stack_pop_boxed() {
                    let inst = self.builder.ins().call(self.ffi.to_string, &[val]);
                    let result = self.builder.inst_results(inst)[0];
                    self.stack_push(result);
                }
                true
            }

            _ => false,
        }
    }
}
