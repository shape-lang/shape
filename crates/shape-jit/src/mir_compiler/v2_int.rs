//! Sized integer (i32) codegen for MirToIR.
//!
//! Native 32-bit Cranelift instructions for i32 arithmetic and comparisons.
//! Input values are i64 (from NaN-boxed stack slots); we narrow to i32,
//! operate at native width, and widen back to i64 for storage.

use cranelift::prelude::*;

use super::MirToIR;
use shape_vm::mir::types::BinOp;

impl<'a, 'b> MirToIR<'a, 'b> {
    /// Compile i32 binary arithmetic — native 32-bit Cranelift instructions.
    ///
    /// Input values are i64 (from NaN-boxed stack slots), narrowed to i32 via
    /// `ireduce`, operated on natively, then sign-extended back to i64.
    /// v2-boundary: input/output are NaN-boxed I64; operates on extracted i32 payload
    pub(crate) fn compile_binop_i32(
        &mut self,
        op: &BinOp,
        lhs: Value,
        rhs: Value,
    ) -> Result<Value, String> {
        let l = self.builder.ins().ireduce(types::I32, lhs);
        let r = self.builder.ins().ireduce(types::I32, rhs);

        let result = match op {
            BinOp::Add => self.builder.ins().iadd(l, r),
            BinOp::Sub => self.builder.ins().isub(l, r),
            BinOp::Mul => self.builder.ins().imul(l, r),
            BinOp::Div => {
                let zero = self.builder.ins().iconst(types::I32, 0);
                let is_zero = self.builder.ins().icmp(IntCC::Equal, r, zero);
                self.builder.ins().trapnz(is_zero, TrapCode::User(0));
                self.builder.ins().sdiv(l, r)
            }
            BinOp::Mod => {
                let zero = self.builder.ins().iconst(types::I32, 0);
                let is_zero = self.builder.ins().icmp(IntCC::Equal, r, zero);
                self.builder.ins().trapnz(is_zero, TrapCode::User(0));
                self.builder.ins().srem(l, r)
            }
            _ => return Err(format!("unsupported i32 binop: {:?}", op)),
        };

        // Sign-extend back to i64, then NaN-box as integer.
        // NaN-boxed int = TAG_BASE | (TAG_INT << TAG_SHIFT) | (val & PAYLOAD_MASK)
        let extended = self.builder.ins().sextend(types::I64, result);
        let payload_mask = self.builder.ins().iconst(types::I64, shape_value::tags::PAYLOAD_MASK as i64);
        let payload = self.builder.ins().band(extended, payload_mask);
        let int_tag = self.builder.ins().iconst(
            types::I64,
            (shape_value::tags::TAG_BASE | (shape_value::tags::TAG_INT << shape_value::tags::TAG_SHIFT)) as i64,
        );
        Ok(self.builder.ins().bor(int_tag, payload))
    }

    /// Compile i32 comparison — returns NaN-boxed boolean.
    ///
    /// Narrows both operands to i32, performs signed integer comparison,
    /// and returns TAG_BOOL_TRUE or TAG_BOOL_FALSE.
    /// v2-boundary: returns NaN-boxed bool because callers expect I64 result
    pub(crate) fn compile_cmp_i32(
        &mut self,
        op: &BinOp,
        lhs: Value,
        rhs: Value,
    ) -> Result<Value, String> {
        let l = self.builder.ins().ireduce(types::I32, lhs);
        let r = self.builder.ins().ireduce(types::I32, rhs);

        let cc = match op {
            BinOp::Eq => IntCC::Equal,
            BinOp::Ne => IntCC::NotEqual,
            BinOp::Lt => IntCC::SignedLessThan,
            BinOp::Le => IntCC::SignedLessThanOrEqual,
            BinOp::Gt => IntCC::SignedGreaterThan,
            BinOp::Ge => IntCC::SignedGreaterThanOrEqual,
            _ => return Err(format!("unsupported i32 cmp: {:?}", op)),
        };

        let cmp_result = self.builder.ins().icmp(cc, l, r);

        let true_val = self
            .builder
            .ins()
            .iconst(types::I64, crate::nan_boxing::TAG_BOOL_TRUE as i64);
        let false_val = self
            .builder
            .ins()
            .iconst(types::I64, crate::nan_boxing::TAG_BOOL_FALSE as i64);
        Ok(self.builder.ins().select(cmp_result, true_val, false_val))
    }
}

#[cfg(test)]
mod tests {
    use cranelift::prelude::*;
    use cranelift_jit::{JITBuilder, JITModule};
    use cranelift_module::Module;

    /// Build a minimal JIT function: (i64, i64) -> i64 using the i32 arithmetic
    /// pattern: ireduce i32, operate, sextend i64.
    fn jit_i32_binop(op: &str, a: i64, b: i64) -> i64 {
        let mut flag_builder = settings::builder();
        flag_builder.set("opt_level", "speed_and_size").unwrap();
        let isa_builder = cranelift_native::builder().unwrap();
        let isa = isa_builder
            .finish(settings::Flags::new(flag_builder))
            .unwrap();
        let builder = JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());
        let mut module = JITModule::new(builder);
        let mut ctx = module.make_context();

        // fn(i64, i64) -> i64
        let ptr_type = types::I64;
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(ptr_type));
        sig.params.push(AbiParam::new(ptr_type));
        sig.returns.push(AbiParam::new(ptr_type));

        let func_id = module
            .declare_function("test_fn", cranelift_module::Linkage::Local, &sig)
            .unwrap();
        ctx.func.signature = sig;

        let mut fn_builder_ctx = FunctionBuilderContext::new();
        let mut builder = FunctionBuilder::new(&mut ctx.func, &mut fn_builder_ctx);
        let block = builder.create_block();
        builder.append_block_params_for_function_params(block);
        builder.switch_to_block(block);
        builder.seal_block(block);

        let lhs = builder.block_params(block)[0];
        let rhs = builder.block_params(block)[1];

        // Same pattern as compile_binop_i32: ireduce, operate, sextend
        let l = builder.ins().ireduce(types::I32, lhs);
        let r = builder.ins().ireduce(types::I32, rhs);

        let result = match op {
            "add" => builder.ins().iadd(l, r),
            "sub" => builder.ins().isub(l, r),
            "mul" => builder.ins().imul(l, r),
            "div" => builder.ins().sdiv(l, r),
            "mod" => builder.ins().srem(l, r),
            _ => panic!("unknown op: {}", op),
        };

        let result_i64 = builder.ins().sextend(types::I64, result);
        builder.ins().return_(&[result_i64]);
        builder.finalize();

        module.define_function(func_id, &mut ctx).unwrap();
        module.clear_context(&mut ctx);
        module.finalize_definitions().unwrap();

        let code_ptr = module.get_finalized_function(func_id);
        let func: fn(i64, i64) -> i64 = unsafe { std::mem::transmute(code_ptr) };
        func(a, b)
    }

    /// Build a JIT function that performs i32 comparison: (i64, i64) -> i64
    /// Returns TAG_BOOL_TRUE or TAG_BOOL_FALSE (same as compile_cmp_i32).
    fn jit_i32_cmp(op: &str, a: i64, b: i64) -> u64 {
        let mut flag_builder = settings::builder();
        flag_builder.set("opt_level", "speed_and_size").unwrap();
        let isa_builder = cranelift_native::builder().unwrap();
        let isa = isa_builder
            .finish(settings::Flags::new(flag_builder))
            .unwrap();
        let builder = JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());
        let mut module = JITModule::new(builder);
        let mut ctx = module.make_context();

        let ptr_type = types::I64;
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(ptr_type));
        sig.params.push(AbiParam::new(ptr_type));
        sig.returns.push(AbiParam::new(ptr_type));

        let func_id = module
            .declare_function("test_cmp", cranelift_module::Linkage::Local, &sig)
            .unwrap();
        ctx.func.signature = sig;

        let mut fn_builder_ctx = FunctionBuilderContext::new();
        let mut builder = FunctionBuilder::new(&mut ctx.func, &mut fn_builder_ctx);
        let block = builder.create_block();
        builder.append_block_params_for_function_params(block);
        builder.switch_to_block(block);
        builder.seal_block(block);

        let lhs = builder.block_params(block)[0];
        let rhs = builder.block_params(block)[1];

        // Same pattern as compile_cmp_i32
        let l = builder.ins().ireduce(types::I32, lhs);
        let r = builder.ins().ireduce(types::I32, rhs);

        let cc = match op {
            "eq" => IntCC::Equal,
            "ne" => IntCC::NotEqual,
            "lt" => IntCC::SignedLessThan,
            "le" => IntCC::SignedLessThanOrEqual,
            "gt" => IntCC::SignedGreaterThan,
            "ge" => IntCC::SignedGreaterThanOrEqual,
            _ => panic!("unknown cmp: {}", op),
        };

        let cmp_result = builder.ins().icmp(cc, l, r);
        let true_val = builder
            .ins()
            .iconst(types::I64, crate::nan_boxing::TAG_BOOL_TRUE as i64);
        let false_val = builder
            .ins()
            .iconst(types::I64, crate::nan_boxing::TAG_BOOL_FALSE as i64);
        let result = builder.ins().select(cmp_result, true_val, false_val);
        builder.ins().return_(&[result]);
        builder.finalize();

        module.define_function(func_id, &mut ctx).unwrap();
        module.clear_context(&mut ctx);
        module.finalize_definitions().unwrap();

        let code_ptr = module.get_finalized_function(func_id);
        let func: fn(i64, i64) -> u64 = unsafe { std::mem::transmute(code_ptr) };
        func(a, b)
    }

    #[test]
    fn test_i32_add_codegen() {
        assert_eq!(jit_i32_binop("add", 100, 200), 300);
    }

    #[test]
    fn test_i32_sub_codegen() {
        assert_eq!(jit_i32_binop("sub", 500, 200), 300);
    }

    #[test]
    fn test_i32_mul_codegen() {
        assert_eq!(jit_i32_binop("mul", 7, 6), 42);
    }

    #[test]
    fn test_i32_div_codegen() {
        assert_eq!(jit_i32_binop("div", 17, 5), 3);
    }

    #[test]
    fn test_i32_mod_codegen() {
        assert_eq!(jit_i32_binop("mod", 17, 5), 2);
    }

    #[test]
    fn test_i32_add_wrapping_overflow() {
        // i32::MAX + 1 should wrap to i32::MIN, then sign-extend to i64
        let result = jit_i32_binop("add", i32::MAX as i64, 1);
        assert_eq!(result, i32::MIN as i64);
    }

    #[test]
    fn test_i32_mul_wrapping_overflow() {
        // 100000 * 100000 = 10_000_000_000, wraps at i32
        let expected = (100000_i32).wrapping_mul(100000_i32) as i64;
        assert_eq!(jit_i32_binop("mul", 100000, 100000), expected);
    }

    #[test]
    fn test_i32_cmp_eq_true() {
        assert_eq!(
            jit_i32_cmp("eq", 42, 42),
            crate::nan_boxing::TAG_BOOL_TRUE
        );
    }

    #[test]
    fn test_i32_cmp_eq_false() {
        assert_eq!(
            jit_i32_cmp("eq", 42, 43),
            crate::nan_boxing::TAG_BOOL_FALSE
        );
    }

    #[test]
    fn test_i32_cmp_lt() {
        assert_eq!(
            jit_i32_cmp("lt", 10, 20),
            crate::nan_boxing::TAG_BOOL_TRUE
        );
        assert_eq!(
            jit_i32_cmp("lt", 20, 10),
            crate::nan_boxing::TAG_BOOL_FALSE
        );
    }

    #[test]
    fn test_i32_cmp_gt() {
        assert_eq!(
            jit_i32_cmp("gt", 20, 10),
            crate::nan_boxing::TAG_BOOL_TRUE
        );
        assert_eq!(
            jit_i32_cmp("gt", 10, 20),
            crate::nan_boxing::TAG_BOOL_FALSE
        );
    }

    #[test]
    fn test_i32_negative_values() {
        // -5 + 3 = -2, sign-extended back to i64
        assert_eq!(jit_i32_binop("add", -5, 3), -2);
        // -10 < 5 should be true
        assert_eq!(
            jit_i32_cmp("lt", -10, 5),
            crate::nan_boxing::TAG_BOOL_TRUE
        );
    }
}
