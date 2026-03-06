//! Array, higher-order function, and object builtin functions for JIT compilation

use cranelift::prelude::*;

use crate::nan_boxing::*;
use crate::translator::types::BytecodeToIR;
use shape_vm::bytecode::BuiltinFunction;

impl<'a, 'b> BytecodeToIR<'a, 'b> {
    /// Compile array and object builtin functions
    #[inline(always)]
    pub(super) fn compile_array_builtin(&mut self, builtin: &BuiltinFunction, idx: usize) -> bool {
        match builtin {
            // Array functions
            BuiltinFunction::First => {
                self.stack_pop(); // arg_count
                if let Some(arr) = self.stack_pop() {
                    // OPTIMIZATION: Inline for arrays — load data_ptr[0]
                    let is_array = self.emit_is_heap_kind(arr, HK_ARRAY);

                    let inline_block = self.builder.create_block();
                    let ffi_block = self.builder.create_block();
                    let merge_block = self.builder.create_block();
                    self.builder.append_block_param(merge_block, types::I64);

                    self.builder
                        .ins()
                        .brif(is_array, inline_block, &[], ffi_block, &[]);

                    // Inline: load first element
                    self.builder.switch_to_block(inline_block);
                    self.builder.seal_block(inline_block);
                    let (data_ptr, length) = self.emit_array_data_ptr(arr);
                    let zero = self.builder.ins().iconst(types::I64, 0);
                    let is_empty = self.builder.ins().icmp(IntCC::Equal, length, zero);
                    let null_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
                    let first_elem =
                        self.builder
                            .ins()
                            .load(types::I64, MemFlags::trusted(), data_ptr, 0);
                    let inline_result = self.builder.ins().select(is_empty, null_val, first_elem);
                    self.builder.ins().jump(merge_block, &[inline_result]);

                    // FFI fallback
                    self.builder.switch_to_block(ffi_block);
                    self.builder.seal_block(ffi_block);
                    let inst = self.builder.ins().call(self.ffi.array_first, &[arr]);
                    let ffi_result = self.builder.inst_results(inst)[0];
                    self.builder.ins().jump(merge_block, &[ffi_result]);

                    self.builder.switch_to_block(merge_block);
                    self.builder.seal_block(merge_block);
                    let result = self.builder.block_params(merge_block)[0];
                    self.stack_push(result);
                }
                true
            }
            BuiltinFunction::Last => {
                self.stack_pop(); // arg_count
                if let Some(arr) = self.stack_pop() {
                    // OPTIMIZATION: Inline for arrays — load data_ptr[(len-1)*8]
                    let is_array = self.emit_is_heap_kind(arr, HK_ARRAY);

                    let inline_block = self.builder.create_block();
                    let ffi_block = self.builder.create_block();
                    let merge_block = self.builder.create_block();
                    self.builder.append_block_param(merge_block, types::I64);

                    self.builder
                        .ins()
                        .brif(is_array, inline_block, &[], ffi_block, &[]);

                    // Inline: load last element
                    self.builder.switch_to_block(inline_block);
                    self.builder.seal_block(inline_block);
                    let (data_ptr, length) = self.emit_array_data_ptr(arr);
                    let zero = self.builder.ins().iconst(types::I64, 0);
                    let is_empty = self.builder.ins().icmp(IntCC::Equal, length, zero);
                    let null_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
                    let one = self.builder.ins().iconst(types::I64, 1);
                    let last_idx = self.builder.ins().isub(length, one);
                    let eight = self.builder.ins().iconst(types::I64, 8);
                    let byte_offset = self.builder.ins().imul(last_idx, eight);
                    let elem_addr = self.builder.ins().iadd(data_ptr, byte_offset);
                    let last_elem =
                        self.builder
                            .ins()
                            .load(types::I64, MemFlags::trusted(), elem_addr, 0);
                    let inline_result = self.builder.ins().select(is_empty, null_val, last_elem);
                    self.builder.ins().jump(merge_block, &[inline_result]);

                    // FFI fallback
                    self.builder.switch_to_block(ffi_block);
                    self.builder.seal_block(ffi_block);
                    let inst = self.builder.ins().call(self.ffi.array_last, &[arr]);
                    let ffi_result = self.builder.inst_results(inst)[0];
                    self.builder.ins().jump(merge_block, &[ffi_result]);

                    self.builder.switch_to_block(merge_block);
                    self.builder.seal_block(merge_block);
                    let result = self.builder.block_params(merge_block)[0];
                    self.stack_push(result);
                }
                true
            }
            BuiltinFunction::Len => {
                self.stack_pop(); // arg_count
                if let Some(val) = self.stack_pop() {
                    // OPTIMIZATION: Inline for arrays — load Vec.len directly
                    let is_array = self.emit_is_heap_kind(val, HK_ARRAY);

                    let inline_block = self.builder.create_block();
                    let ffi_block = self.builder.create_block();
                    let merge_block = self.builder.create_block();
                    self.builder.append_block_param(merge_block, types::I64);

                    self.builder
                        .ins()
                        .brif(is_array, inline_block, &[], ffi_block, &[]);

                    // Inline: load Vec.len
                    self.builder.switch_to_block(inline_block);
                    self.builder.seal_block(inline_block);
                    let inline_result = self.inline_array_length(val);
                    self.builder.ins().jump(merge_block, &[inline_result]);

                    // FFI fallback
                    self.builder.switch_to_block(ffi_block);
                    self.builder.seal_block(ffi_block);
                    let inst = self.builder.ins().call(self.ffi.length, &[val]);
                    let ffi_result = self.builder.inst_results(inst)[0];
                    self.builder.ins().jump(merge_block, &[ffi_result]);

                    self.builder.switch_to_block(merge_block);
                    self.builder.seal_block(merge_block);
                    let result = self.builder.block_params(merge_block)[0];
                    self.stack_push(result);
                }
                true
            }
            BuiltinFunction::Slice => {
                self.stack_pop();
                if self.stack_len() >= 3 {
                    let end = self.stack_pop().unwrap();
                    let start = self.stack_pop().unwrap();
                    let arr = self.stack_pop().unwrap();
                    let inst = self.builder.ins().call(self.ffi.slice, &[arr, start, end]);
                    let result = self.builder.inst_results(inst)[0];
                    self.stack_push(result);
                }
                true
            }
            BuiltinFunction::Range => {
                self.stack_pop();
                if self.stack_len() >= 2 {
                    let end = self.stack_pop().unwrap();
                    let start = self.stack_pop().unwrap();
                    let inst = self.builder.ins().call(self.ffi.range, &[start, end]);
                    let result = self.builder.inst_results(inst)[0];
                    self.stack_push(result);
                } else {
                    let empty_arr = jit_box(HK_ARRAY, crate::jit_array::JitArray::new());
                    let val = self.builder.ins().iconst(types::I64, empty_arr as i64);
                    self.stack_push(val);
                }
                true
            }
            BuiltinFunction::ToString => {
                self.stack_pop();
                if let Some(val) = self.stack_pop() {
                    let inst = self.builder.ins().call(self.ffi.to_string, &[val]);
                    let result = self.builder.inst_results(inst)[0];
                    self.stack_push(result);
                }
                true
            }
            BuiltinFunction::ToNumber => {
                self.stack_pop();
                if let Some(val) = self.stack_pop() {
                    let inst = self.builder.ins().call(self.ffi.to_number, &[val]);
                    let result = self.builder.inst_results(inst)[0];
                    self.stack_push(result);
                }
                true
            }
            BuiltinFunction::Print => {
                self.stack_pop(); // arg count
                if let Some(val) = self.stack_pop() {
                    self.builder.ins().call(self.ffi.print, &[val]);
                }
                let null_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
                self.stack_push(null_val);
                true
            }

            // Higher-order functions
            BuiltinFunction::ControlFold => {
                let needed = 4;
                if self.stack_len() >= needed {
                    self.materialize_to_stack(needed);
                    let inst = self
                        .builder
                        .ins()
                        .call(self.ffi.control_fold, &[self.ctx_ptr]);
                    let result = self.builder.inst_results(inst)[0];
                    self.update_sp_after_ffi(needed, 0);
                    self.stack_push(result);
                } else {
                    let null_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
                    self.stack_push(null_val);
                }
                true
            }
            BuiltinFunction::Map => {
                let needed = 3;
                if self.stack_len() >= needed {
                    self.materialize_to_stack(needed);
                    let inst = self
                        .builder
                        .ins()
                        .call(self.ffi.control_map, &[self.ctx_ptr]);
                    let result = self.builder.inst_results(inst)[0];
                    self.update_sp_after_ffi(needed, 0);
                    self.stack_push(result);
                } else {
                    let null_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
                    self.stack_push(null_val);
                }
                true
            }
            BuiltinFunction::Filter => {
                let needed = 3;
                if self.stack_len() >= needed {
                    self.materialize_to_stack(needed);
                    let inst = self
                        .builder
                        .ins()
                        .call(self.ffi.control_filter, &[self.ctx_ptr]);
                    let result = self.builder.inst_results(inst)[0];
                    self.update_sp_after_ffi(needed, 0);
                    self.stack_push(result);
                } else {
                    let null_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
                    self.stack_push(null_val);
                }
                true
            }
            BuiltinFunction::Reduce => {
                let needed = 4;
                if self.stack_len() >= needed {
                    self.materialize_to_stack(needed);
                    let inst = self
                        .builder
                        .ins()
                        .call(self.ffi.control_reduce, &[self.ctx_ptr]);
                    let result = self.builder.inst_results(inst)[0];
                    self.update_sp_after_ffi(needed, 0);
                    self.stack_push(result);
                } else {
                    let null_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
                    self.stack_push(null_val);
                }
                true
            }
            BuiltinFunction::ForEach => {
                let arg_count = self.get_arg_count_from_prev_instruction(idx);
                let needed = arg_count + 1;
                if self.stack_len() >= needed {
                    self.materialize_to_stack(needed);
                    let count_val = self.builder.ins().iconst(types::I64, needed as i64);
                    let inst = self
                        .builder
                        .ins()
                        .call(self.ffi.control_foreach, &[self.ctx_ptr, count_val]);
                    let result = self.builder.inst_results(inst)[0];
                    self.update_sp_after_ffi(needed, 0);
                    self.stack_push(result);
                } else {
                    let null_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
                    self.stack_push(null_val);
                }
                true
            }
            BuiltinFunction::Find => {
                // Stack: [array, predicate, arg_count]
                let needed = 3;
                if self.stack_len() >= needed {
                    self.materialize_to_stack(needed);
                    let inst = self
                        .builder
                        .ins()
                        .call(self.ffi.control_find, &[self.ctx_ptr]);
                    let result = self.builder.inst_results(inst)[0];
                    self.update_sp_after_ffi(needed, 0);
                    self.stack_push(result);
                } else {
                    let null_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
                    self.stack_push(null_val);
                }
                true
            }
            BuiltinFunction::FindIndex => {
                // Stack: [array, predicate, arg_count]
                let needed = 3;
                if self.stack_len() >= needed {
                    self.materialize_to_stack(needed);
                    let inst = self
                        .builder
                        .ins()
                        .call(self.ffi.control_find_index, &[self.ctx_ptr]);
                    let result = self.builder.inst_results(inst)[0];
                    self.update_sp_after_ffi(needed, 0);
                    self.stack_push(result);
                } else {
                    let null_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
                    self.stack_push(null_val);
                }
                true
            }
            BuiltinFunction::Some => {
                // Stack: [array, predicate, arg_count]
                let needed = 3;
                if self.stack_len() >= needed {
                    self.materialize_to_stack(needed);
                    let inst = self
                        .builder
                        .ins()
                        .call(self.ffi.control_some, &[self.ctx_ptr]);
                    let result = self.builder.inst_results(inst)[0];
                    self.update_sp_after_ffi(needed, 0);
                    self.stack_push(result);
                } else {
                    let null_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
                    self.stack_push(null_val);
                }
                true
            }
            BuiltinFunction::Every => {
                // Stack: [array, predicate, arg_count]
                let needed = 3;
                if self.stack_len() >= needed {
                    self.materialize_to_stack(needed);
                    let inst = self
                        .builder
                        .ins()
                        .call(self.ffi.control_every, &[self.ctx_ptr]);
                    let result = self.builder.inst_results(inst)[0];
                    self.update_sp_after_ffi(needed, 0);
                    self.stack_push(result);
                } else {
                    let null_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
                    self.stack_push(null_val);
                }
                true
            }
            BuiltinFunction::Push => {
                let arg_count = self.get_arg_count_from_prev_instruction(idx);
                let needed = arg_count + 1;
                if self.stack_len() >= needed {
                    self.materialize_to_stack(needed);
                    let count_val = self.builder.ins().iconst(types::I64, needed as i64);
                    let inst = self
                        .builder
                        .ins()
                        .call(self.ffi.array_push, &[self.ctx_ptr, count_val]);
                    let result = self.builder.inst_results(inst)[0];
                    self.update_sp_after_ffi(needed, 0);
                    self.stack_push(result);
                } else {
                    let null_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
                    self.stack_push(null_val);
                }
                true
            }
            BuiltinFunction::Pop => {
                self.stack_pop();
                if let Some(arr) = self.stack_pop() {
                    let inst = self.builder.ins().call(self.ffi.array_pop, &[arr]);
                    let result = self.builder.inst_results(inst)[0];
                    self.stack_push(result);
                } else {
                    let null_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
                    self.stack_push(null_val);
                }
                true
            }
            BuiltinFunction::Zip => {
                self.stack_pop();
                if self.stack_len() >= 2 {
                    let arr2 = self.stack_pop().unwrap();
                    let arr1 = self.stack_pop().unwrap();
                    let inst = self.builder.ins().call(self.ffi.array_zip, &[arr1, arr2]);
                    let result = self.builder.inst_results(inst)[0];
                    self.stack_push(result);
                } else {
                    let null_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
                    self.stack_push(null_val);
                }
                true
            }

            BuiltinFunction::Filled => {
                // Array.filled(size, value) — pop arg_count, pop size and value, call FFI
                self.stack_pop(); // pop arg_count
                if self.stack_len() >= 2 {
                    let value = self.stack_pop().unwrap();
                    let size = self.stack_pop().unwrap();
                    let inst = self
                        .builder
                        .ins()
                        .call(self.ffi.array_filled, &[size, value]);
                    let result = self.builder.inst_results(inst)[0];
                    self.stack_push(result);
                } else {
                    let null_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
                    self.stack_push(null_val);
                }
                true
            }

            // Object operations
            BuiltinFunction::ObjectRest => {
                self.stack_pop();
                if self.stack_len() >= 2 {
                    let keys = self.stack_pop().unwrap();
                    let obj = self.stack_pop().unwrap();
                    let inst = self.builder.ins().call(self.ffi.object_rest, &[obj, keys]);
                    let result = self.builder.inst_results(inst)[0];
                    self.stack_push(result);
                } else {
                    let null_val = self.builder.ins().iconst(types::I64, TAG_NULL as i64);
                    self.stack_push(null_val);
                }
                true
            }

            _ => false,
        }
    }
}
