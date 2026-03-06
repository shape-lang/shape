//! Math and vector intrinsic builtin functions for JIT compilation

use cranelift::prelude::*;

use crate::translator::types::BytecodeToIR;
use shape_vm::bytecode::BuiltinFunction;

impl<'a, 'b> BytecodeToIR<'a, 'b> {
    /// Compile math builtin functions
    #[inline(always)]
    pub(super) fn compile_math_builtin(&mut self, builtin: &BuiltinFunction, idx: usize) -> bool {
        match builtin {
            // Math functions - single argument
            BuiltinFunction::Abs => {
                self.stack_pop(); // arg_count
                if let Some(a_boxed) = self.stack_pop() {
                    let a_f64 = self.i64_to_f64(a_boxed);
                    let result_f64 = self.builder.ins().fabs(a_f64);
                    let result_boxed = self.f64_to_i64(result_f64);
                    self.stack_push(result_boxed);
                }
                true
            }
            BuiltinFunction::Sqrt => {
                self.stack_pop();
                if let Some(a_boxed) = self.stack_pop() {
                    let a_f64 = self.i64_to_f64(a_boxed);
                    let result_f64 = self.builder.ins().sqrt(a_f64);
                    let result_boxed = self.f64_to_i64(result_f64);
                    self.stack_push(result_boxed);
                }
                true
            }
            BuiltinFunction::Floor => {
                self.stack_pop();
                if let Some(a_boxed) = self.stack_pop() {
                    let a_f64 = self.i64_to_f64(a_boxed);
                    let result_f64 = self.builder.ins().floor(a_f64);
                    let result_boxed = self.f64_to_i64(result_f64);
                    self.stack_push(result_boxed);
                }
                true
            }
            BuiltinFunction::Ceil => {
                self.stack_pop();
                if let Some(a_boxed) = self.stack_pop() {
                    let a_f64 = self.i64_to_f64(a_boxed);
                    let result_f64 = self.builder.ins().ceil(a_f64);
                    let result_boxed = self.f64_to_i64(result_f64);
                    self.stack_push(result_boxed);
                }
                true
            }
            BuiltinFunction::Round => {
                self.stack_pop();
                if let Some(a_boxed) = self.stack_pop() {
                    let a_f64 = self.i64_to_f64(a_boxed);
                    let result_f64 = self.builder.ins().nearest(a_f64);
                    let result_boxed = self.f64_to_i64(result_f64);
                    self.stack_push(result_boxed);
                }
                true
            }

            // Binary math functions
            BuiltinFunction::Min => {
                let arg_count = self.get_arg_count_from_prev_instruction(idx);
                self.stack_pop();
                if arg_count >= 2 && self.stack_len() >= 2 {
                    let b_boxed = self.stack_pop().unwrap();
                    let a_boxed = self.stack_pop().unwrap();
                    let a_f64 = self.i64_to_f64(a_boxed);
                    let b_f64 = self.i64_to_f64(b_boxed);
                    let result_f64 = self.builder.ins().fmin(a_f64, b_f64);
                    let result_boxed = self.f64_to_i64(result_f64);
                    self.stack_push(result_boxed);
                } else if arg_count == 1 && self.stack_len() >= 1 {
                    let arr_boxed = self.stack_pop().unwrap();
                    let inst = self.builder.ins().call(self.ffi.array_min, &[arr_boxed]);
                    let result = self.builder.inst_results(inst)[0];
                    self.stack_push(result);
                }
                true
            }
            BuiltinFunction::Max => {
                let arg_count = self.get_arg_count_from_prev_instruction(idx);
                self.stack_pop();
                if arg_count >= 2 && self.stack_len() >= 2 {
                    let b_boxed = self.stack_pop().unwrap();
                    let a_boxed = self.stack_pop().unwrap();
                    let a_f64 = self.i64_to_f64(a_boxed);
                    let b_f64 = self.i64_to_f64(b_boxed);
                    let result_f64 = self.builder.ins().fmax(a_f64, b_f64);
                    let result_boxed = self.f64_to_i64(result_f64);
                    self.stack_push(result_boxed);
                } else if arg_count == 1 && self.stack_len() >= 1 {
                    let arr_boxed = self.stack_pop().unwrap();
                    let inst = self.builder.ins().call(self.ffi.array_max, &[arr_boxed]);
                    let result = self.builder.inst_results(inst)[0];
                    self.stack_push(result);
                }
                true
            }

            // Trig and transcendental functions (via FFI)
            BuiltinFunction::Sin => {
                self.stack_pop();
                if let Some(val) = self.stack_pop() {
                    let inst = self.builder.ins().call(self.ffi.sin, &[val]);
                    let result = self.builder.inst_results(inst)[0];
                    self.stack_push(result);
                }
                true
            }
            BuiltinFunction::Cos => {
                self.stack_pop();
                if let Some(val) = self.stack_pop() {
                    let inst = self.builder.ins().call(self.ffi.cos, &[val]);
                    let result = self.builder.inst_results(inst)[0];
                    self.stack_push(result);
                }
                true
            }
            BuiltinFunction::Tan => {
                self.stack_pop();
                if let Some(val) = self.stack_pop() {
                    let inst = self.builder.ins().call(self.ffi.tan, &[val]);
                    let result = self.builder.inst_results(inst)[0];
                    self.stack_push(result);
                }
                true
            }
            BuiltinFunction::Asin => {
                self.stack_pop();
                if let Some(val) = self.stack_pop() {
                    let inst = self.builder.ins().call(self.ffi.asin, &[val]);
                    let result = self.builder.inst_results(inst)[0];
                    self.stack_push(result);
                }
                true
            }
            BuiltinFunction::Acos => {
                self.stack_pop();
                if let Some(val) = self.stack_pop() {
                    let inst = self.builder.ins().call(self.ffi.acos, &[val]);
                    let result = self.builder.inst_results(inst)[0];
                    self.stack_push(result);
                }
                true
            }
            BuiltinFunction::Atan => {
                self.stack_pop();
                if let Some(val) = self.stack_pop() {
                    let inst = self.builder.ins().call(self.ffi.atan, &[val]);
                    let result = self.builder.inst_results(inst)[0];
                    self.stack_push(result);
                }
                true
            }
            BuiltinFunction::Exp => {
                self.stack_pop();
                if let Some(val) = self.stack_pop() {
                    let inst = self.builder.ins().call(self.ffi.exp, &[val]);
                    let result = self.builder.inst_results(inst)[0];
                    self.stack_push(result);
                }
                true
            }
            BuiltinFunction::Ln => {
                self.stack_pop();
                if let Some(val) = self.stack_pop() {
                    let inst = self.builder.ins().call(self.ffi.ln, &[val]);
                    let result = self.builder.inst_results(inst)[0];
                    self.stack_push(result);
                }
                true
            }
            BuiltinFunction::Log => {
                self.stack_pop();
                if self.stack_len() >= 2 {
                    let base = self.stack_pop().unwrap();
                    let val = self.stack_pop().unwrap();
                    let inst = self.builder.ins().call(self.ffi.log, &[val, base]);
                    let result = self.builder.inst_results(inst)[0];
                    self.stack_push(result);
                }
                true
            }
            BuiltinFunction::Pow => {
                self.stack_pop();
                if self.stack_len() >= 2 {
                    let exp = self.stack_pop().unwrap();
                    let base = self.stack_pop().unwrap();
                    let inst = self.builder.ins().call(self.ffi.pow, &[base, exp]);
                    let result = self.builder.inst_results(inst)[0];
                    self.stack_push(result);
                }
                true
            }

            // Vector intrinsics
            BuiltinFunction::IntrinsicVecAbs => {
                self.stack_pop(); // arg_count
                if let Some(arg) = self.stack_pop() {
                    let inst = self
                        .builder
                        .ins()
                        .call(self.ffi.intrinsic_vec_abs, &[self.ctx_ptr, arg]);
                    let result = self.builder.inst_results(inst)[0];
                    self.stack_push(result);
                } else {
                    let null_val = self
                        .builder
                        .ins()
                        .iconst(types::I64, crate::nan_boxing::TAG_NULL as i64);
                    self.stack_push(null_val);
                }
                true
            }
            BuiltinFunction::IntrinsicVecSqrt => {
                self.stack_pop(); // arg_count
                if let Some(arg) = self.stack_pop() {
                    let inst = self
                        .builder
                        .ins()
                        .call(self.ffi.intrinsic_vec_sqrt, &[self.ctx_ptr, arg]);
                    let result = self.builder.inst_results(inst)[0];
                    self.stack_push(result);
                } else {
                    let null_val = self
                        .builder
                        .ins()
                        .iconst(types::I64, crate::nan_boxing::TAG_NULL as i64);
                    self.stack_push(null_val);
                }
                true
            }
            BuiltinFunction::IntrinsicVecLn => {
                self.stack_pop(); // arg_count
                if let Some(arg) = self.stack_pop() {
                    let inst = self
                        .builder
                        .ins()
                        .call(self.ffi.intrinsic_vec_ln, &[self.ctx_ptr, arg]);
                    let result = self.builder.inst_results(inst)[0];
                    self.stack_push(result);
                } else {
                    let null_val = self
                        .builder
                        .ins()
                        .iconst(types::I64, crate::nan_boxing::TAG_NULL as i64);
                    self.stack_push(null_val);
                }
                true
            }
            BuiltinFunction::IntrinsicVecExp => {
                self.stack_pop(); // arg_count
                if let Some(arg) = self.stack_pop() {
                    let inst = self
                        .builder
                        .ins()
                        .call(self.ffi.intrinsic_vec_exp, &[self.ctx_ptr, arg]);
                    let result = self.builder.inst_results(inst)[0];
                    self.stack_push(result);
                } else {
                    let null_val = self
                        .builder
                        .ins()
                        .iconst(types::I64, crate::nan_boxing::TAG_NULL as i64);
                    self.stack_push(null_val);
                }
                true
            }
            BuiltinFunction::IntrinsicVecAdd => {
                self.stack_pop(); // arg_count
                if self.stack_len() >= 2 {
                    let b = self.stack_pop().unwrap();
                    let a = self.stack_pop().unwrap();
                    let inst = self
                        .builder
                        .ins()
                        .call(self.ffi.intrinsic_vec_add, &[self.ctx_ptr, a, b]);
                    let result = self.builder.inst_results(inst)[0];
                    self.stack_push(result);
                } else {
                    let null_val = self
                        .builder
                        .ins()
                        .iconst(types::I64, crate::nan_boxing::TAG_NULL as i64);
                    self.stack_push(null_val);
                }
                true
            }
            BuiltinFunction::IntrinsicVecSub => {
                self.stack_pop(); // arg_count
                if self.stack_len() >= 2 {
                    let b = self.stack_pop().unwrap();
                    let a = self.stack_pop().unwrap();
                    let inst = self
                        .builder
                        .ins()
                        .call(self.ffi.intrinsic_vec_sub, &[self.ctx_ptr, a, b]);
                    let result = self.builder.inst_results(inst)[0];
                    self.stack_push(result);
                } else {
                    let null_val = self
                        .builder
                        .ins()
                        .iconst(types::I64, crate::nan_boxing::TAG_NULL as i64);
                    self.stack_push(null_val);
                }
                true
            }
            BuiltinFunction::IntrinsicVecMul => {
                self.stack_pop(); // arg_count
                if self.stack_len() >= 2 {
                    let b = self.stack_pop().unwrap();
                    let a = self.stack_pop().unwrap();
                    let inst = self
                        .builder
                        .ins()
                        .call(self.ffi.intrinsic_vec_mul, &[self.ctx_ptr, a, b]);
                    let result = self.builder.inst_results(inst)[0];
                    self.stack_push(result);
                } else {
                    let null_val = self
                        .builder
                        .ins()
                        .iconst(types::I64, crate::nan_boxing::TAG_NULL as i64);
                    self.stack_push(null_val);
                }
                true
            }
            BuiltinFunction::IntrinsicVecDiv => {
                self.stack_pop(); // arg_count
                if self.stack_len() >= 2 {
                    let b = self.stack_pop().unwrap();
                    let a = self.stack_pop().unwrap();
                    let inst = self
                        .builder
                        .ins()
                        .call(self.ffi.intrinsic_vec_div, &[self.ctx_ptr, a, b]);
                    let result = self.builder.inst_results(inst)[0];
                    self.stack_push(result);
                } else {
                    let null_val = self
                        .builder
                        .ins()
                        .iconst(types::I64, crate::nan_boxing::TAG_NULL as i64);
                    self.stack_push(null_val);
                }
                true
            }
            BuiltinFunction::IntrinsicVecMax => {
                self.stack_pop(); // arg_count
                if self.stack_len() >= 2 {
                    let b = self.stack_pop().unwrap();
                    let a = self.stack_pop().unwrap();
                    let inst = self
                        .builder
                        .ins()
                        .call(self.ffi.intrinsic_vec_max, &[self.ctx_ptr, a, b]);
                    let result = self.builder.inst_results(inst)[0];
                    self.stack_push(result);
                } else {
                    let null_val = self
                        .builder
                        .ins()
                        .iconst(types::I64, crate::nan_boxing::TAG_NULL as i64);
                    self.stack_push(null_val);
                }
                true
            }
            BuiltinFunction::IntrinsicVecMin => {
                self.stack_pop(); // arg_count
                if self.stack_len() >= 2 {
                    let b = self.stack_pop().unwrap();
                    let a = self.stack_pop().unwrap();
                    let inst = self
                        .builder
                        .ins()
                        .call(self.ffi.intrinsic_vec_min, &[self.ctx_ptr, a, b]);
                    let result = self.builder.inst_results(inst)[0];
                    self.stack_push(result);
                } else {
                    let null_val = self
                        .builder
                        .ins()
                        .iconst(types::I64, crate::nan_boxing::TAG_NULL as i64);
                    self.stack_push(null_val);
                }
                true
            }
            BuiltinFunction::IntrinsicMatMulVec => {
                self.stack_pop(); // arg_count
                if self.stack_len() >= 2 {
                    let b = self.stack_pop().unwrap();
                    let a = self.stack_pop().unwrap();
                    let inst = self
                        .builder
                        .ins()
                        .call(self.ffi.intrinsic_matmul_vec, &[self.ctx_ptr, a, b]);
                    let result = self.builder.inst_results(inst)[0];
                    self.stack_push(result);
                } else {
                    let null_val = self
                        .builder
                        .ins()
                        .iconst(types::I64, crate::nan_boxing::TAG_NULL as i64);
                    self.stack_push(null_val);
                }
                true
            }
            BuiltinFunction::IntrinsicMatMulMat => {
                self.stack_pop(); // arg_count
                if self.stack_len() >= 2 {
                    let b = self.stack_pop().unwrap();
                    let a = self.stack_pop().unwrap();
                    let inst = self
                        .builder
                        .ins()
                        .call(self.ffi.intrinsic_matmul_mat, &[self.ctx_ptr, a, b]);
                    let result = self.builder.inst_results(inst)[0];
                    self.stack_push(result);
                } else {
                    let null_val = self
                        .builder
                        .ins()
                        .iconst(types::I64, crate::nan_boxing::TAG_NULL as i64);
                    self.stack_push(null_val);
                }
                true
            }

            _ => false,
        }
    }
}
