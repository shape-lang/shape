//! Sized integer (i32) codegen for MirToIR.
//!
//! Native 32-bit Cranelift instructions for i32 arithmetic and comparisons.
//! Per ADR-006 §2.7.5 the JIT emits raw native results; the slot's
//! `NativeKind::Int32` is stamped at JIT compile time from the MIR static
//! type info, not encoded in the bits. Input values are i64 raw payloads
//! (callers widened them into the I64 ABI slot); we narrow to i32, operate
//! at native width, and sign-extend back to i64 for storage. No NaN-box,
//! no `tag_bits` dispatch, no payload masking.

use cranelift::prelude::*;

use super::MirToIR;
use shape_vm::mir::types::BinOp;

impl<'a, 'b> MirToIR<'a, 'b> {
    /// Compile i32 binary arithmetic — native 32-bit Cranelift instructions.
    ///
    /// Input values are i64 raw payloads (callers widened them into the I64
    /// ABI slot), narrowed to i32 via `ireduce`, operated on natively, then
    /// sign-extended back to i64. Per ADR-006 §2.7.5 the result is raw
    /// native bits — `NativeKind::Int32` is stamped at the JIT-FFI carrier
    /// from the MIR static type info, not encoded in the bits.
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
            // r5c-2-gz-cp2-jit-div: VM-equivalent trap-free i32 div/mod —
            // div-by-zero → clean `Division by zero`; `i32::MIN / -1` →
            // wrapping `i32::MIN` (mod → 0). `compile_int_divmod_guarded`
            // (in `rvalues.rs`) operates at the `I32` width; the result is
            // sign-extended back to the I64 ABI slot below.
            BinOp::Div => {
                self.compile_int_divmod_guarded(l, r, types::I32, true, false)?
            }
            BinOp::Mod => {
                self.compile_int_divmod_guarded(l, r, types::I32, true, true)?
            }
            _ => return Err(format!("unsupported i32 binop: {:?}", op)),
        };

        // Sign-extend the raw i32 payload back into the I64 ABI slot. No
        // NaN-box: kind flows on the parallel JitFfiCarrier companion.
        Ok(self.builder.ins().sextend(types::I64, result))
    }

    /// Compile i32 comparison — returns a raw bool payload in the I64 ABI slot.
    ///
    /// Narrows both operands to i32, performs signed integer comparison,
    /// and selects 1u64 / 0u64. Per ADR-006 §2.7.5 the result is raw bits;
    /// `NativeKind::Bool` is stamped on the parallel JitFfiCarrier.
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
            .iconst(types::I64, 1i64);
        let false_val = self
            .builder
            .ins()
            .iconst(types::I64, 0i64);
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
            .iconst(types::I64, 1i64);
        let false_val = builder
            .ins()
            .iconst(types::I64, 0i64);
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
            1u64
        );
    }

    #[test]
    fn test_i32_cmp_eq_false() {
        assert_eq!(
            jit_i32_cmp("eq", 42, 43),
            0u64
        );
    }

    #[test]
    fn test_i32_cmp_lt() {
        assert_eq!(
            jit_i32_cmp("lt", 10, 20),
            1u64
        );
        assert_eq!(
            jit_i32_cmp("lt", 20, 10),
            0u64
        );
    }

    #[test]
    fn test_i32_cmp_gt() {
        assert_eq!(
            jit_i32_cmp("gt", 20, 10),
            1u64
        );
        assert_eq!(
            jit_i32_cmp("gt", 10, 20),
            0u64
        );
    }

    #[test]
    fn test_i32_negative_values() {
        // -5 + 3 = -2, sign-extended back to i64
        assert_eq!(jit_i32_binop("add", -5, 3), -2);
        // -10 < 5 should be true
        assert_eq!(
            jit_i32_cmp("lt", -10, 5),
            1u64
        );
    }

    // ── R5c-2-β-γ (c) jit-narrow-wrap regression tests ──────────────
    //
    // These mirror `compile_binop_narrow_int`'s codegen exactly:
    // coerce each I64 operand to the narrow Cranelift width via
    // `ireduce`, then `iadd`/`isub`/`imul` at that width — which wraps
    // two's-complement natively — matching the bytecode VM's
    // `AddI32`/`AddTyped` truncating opcodes. Before this checkpoint the
    // JIT operated at 64-bit width and never truncated, so overflow did
    // not wrap (e.g. `100i8 + 100i8` produced 200 instead of -56).
    //
    // Standalone Cranelift fns (no stdlib JIT-compilation) — fast,
    // deterministic, and not subject to the `deep-tests` gating that
    // covers JIT end-to-end stdlib-execution suites.

    /// Build a JIT fn `(i64, i64) -> i64` using the narrow-int codegen
    /// pattern: `ireduce` both operands to `narrow`, apply `op`, then
    /// `sextend`/`uextend` the narrow result back to i64 (the same shape
    /// `compile_binop_narrow_int` produces feeding `store_to_place`'s
    /// `ensure_kind` widen). `unsigned` selects the result extension.
    fn jit_narrow_binop(
        op: &str,
        narrow: types::Type,
        unsigned: bool,
        a: i64,
        b: i64,
    ) -> i64 {
        let mut flag_builder = settings::builder();
        flag_builder.set("opt_level", "speed_and_size").unwrap();
        let isa = cranelift_native::builder()
            .unwrap()
            .finish(settings::Flags::new(flag_builder))
            .unwrap();
        let builder =
            JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());
        let mut module = JITModule::new(builder);
        let mut ctx = module.make_context();

        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));

        let func_id = module
            .declare_function(
                "narrow_fn",
                cranelift_module::Linkage::Local,
                &sig,
            )
            .unwrap();
        ctx.func.signature = sig;

        let mut fbc = FunctionBuilderContext::new();
        let mut builder = FunctionBuilder::new(&mut ctx.func, &mut fbc);
        let block = builder.create_block();
        builder.append_block_params_for_function_params(block);
        builder.switch_to_block(block);
        builder.seal_block(block);

        let lhs = builder.block_params(block)[0];
        let rhs = builder.block_params(block)[1];
        let l = builder.ins().ireduce(narrow, lhs);
        let r = builder.ins().ireduce(narrow, rhs);
        let result = match op {
            "add" => builder.ins().iadd(l, r),
            "sub" => builder.ins().isub(l, r),
            "mul" => builder.ins().imul(l, r),
            _ => panic!("unknown op: {}", op),
        };
        let widened = if unsigned {
            builder.ins().uextend(types::I64, result)
        } else {
            builder.ins().sextend(types::I64, result)
        };
        builder.ins().return_(&[widened]);
        builder.finalize();

        module.define_function(func_id, &mut ctx).unwrap();
        module.clear_context(&mut ctx);
        module.finalize_definitions().unwrap();
        let code_ptr = module.get_finalized_function(func_id);
        let func: fn(i64, i64) -> i64 = unsafe { std::mem::transmute(code_ptr) };
        func(a, b)
    }

    #[test]
    fn narrow_i8_add_overflow_wraps() {
        // 100i8 + 100i8 = 200 wraps to -56 (the canonical reproducer).
        assert_eq!(
            jit_narrow_binop("add", types::I8, false, 100, 100),
            (100i8).wrapping_add(100) as i64,
        );
        assert_eq!(jit_narrow_binop("add", types::I8, false, 100, 100), -56);
    }

    #[test]
    fn narrow_i8_sub_overflow_wraps() {
        // -100i8 - 100i8 = -200 wraps to 56.
        assert_eq!(
            jit_narrow_binop("sub", types::I8, false, -100, 100),
            (-100i8).wrapping_sub(100) as i64,
        );
        assert_eq!(jit_narrow_binop("sub", types::I8, false, -100, 100), 56);
    }

    #[test]
    fn narrow_i8_mul_overflow_wraps() {
        // 20i8 * 20i8 = 400 wraps to -112.
        assert_eq!(
            jit_narrow_binop("mul", types::I8, false, 20, 20),
            (20i8).wrapping_mul(20) as i64,
        );
        assert_eq!(jit_narrow_binop("mul", types::I8, false, 20, 20), -112);
    }

    #[test]
    fn narrow_i16_add_overflow_wraps() {
        // 30000i16 + 30000i16 = 60000 wraps to -5536.
        assert_eq!(
            jit_narrow_binop("add", types::I16, false, 30000, 30000),
            (30000i16).wrapping_add(30000) as i64,
        );
        assert_eq!(jit_narrow_binop("add", types::I16, false, 30000, 30000), -5536);
    }

    #[test]
    fn narrow_i16_sub_overflow_wraps() {
        assert_eq!(
            jit_narrow_binop("sub", types::I16, false, -30000, 30000),
            (-30000i16).wrapping_sub(30000) as i64,
        );
        assert_eq!(jit_narrow_binop("sub", types::I16, false, -30000, 30000), 5536);
    }

    #[test]
    fn narrow_i16_mul_overflow_wraps() {
        assert_eq!(
            jit_narrow_binop("mul", types::I16, false, 1000, 1000),
            (1000i16).wrapping_mul(1000) as i64,
        );
        assert_eq!(jit_narrow_binop("mul", types::I16, false, 1000, 1000), 16960);
    }

    #[test]
    fn narrow_i32_add_overflow_wraps() {
        // 2_000_000_000i32 + 2_000_000_000i32 = 4e9 wraps to -294967296.
        assert_eq!(
            jit_narrow_binop("add", types::I32, false, 2_000_000_000, 2_000_000_000),
            (2_000_000_000i32).wrapping_add(2_000_000_000) as i64,
        );
        assert_eq!(
            jit_narrow_binop("add", types::I32, false, 2_000_000_000, 2_000_000_000),
            -294_967_296,
        );
    }

    #[test]
    fn narrow_i32_sub_overflow_wraps() {
        assert_eq!(
            jit_narrow_binop("sub", types::I32, false, -2_000_000_000, 2_000_000_000),
            (-2_000_000_000i32).wrapping_sub(2_000_000_000) as i64,
        );
        assert_eq!(
            jit_narrow_binop("sub", types::I32, false, -2_000_000_000, 2_000_000_000),
            294_967_296,
        );
    }

    #[test]
    fn narrow_i32_mul_overflow_wraps() {
        assert_eq!(
            jit_narrow_binop("mul", types::I32, false, 100_000, 100_000),
            (100_000i32).wrapping_mul(100_000) as i64,
        );
        assert_eq!(
            jit_narrow_binop("mul", types::I32, false, 100_000, 100_000),
            1_410_065_408,
        );
    }

    #[test]
    fn narrow_u8_add_overflow_wraps() {
        // 200u8 + 200u8 = 400 wraps to 144 (unsigned-extended result).
        assert_eq!(
            jit_narrow_binop("add", types::I8, true, 200, 200),
            (200u8).wrapping_add(200) as i64,
        );
        assert_eq!(jit_narrow_binop("add", types::I8, true, 200, 200), 144);
    }

    #[test]
    fn narrow_u8_sub_overflow_wraps() {
        // 50u8 - 200u8 underflows; 50.wrapping_sub(200) = 106.
        assert_eq!(
            jit_narrow_binop("sub", types::I8, true, 50, 200),
            (50u8).wrapping_sub(200) as i64,
        );
        assert_eq!(jit_narrow_binop("sub", types::I8, true, 50, 200), 106);
    }

    #[test]
    fn narrow_u8_mul_overflow_wraps() {
        assert_eq!(
            jit_narrow_binop("mul", types::I8, true, 30, 30),
            (30u8).wrapping_mul(30) as i64,
        );
        assert_eq!(jit_narrow_binop("mul", types::I8, true, 30, 30), 132);
    }

    #[test]
    fn narrow_u16_add_overflow_wraps() {
        assert_eq!(
            jit_narrow_binop("add", types::I16, true, 60000, 60000),
            (60000u16).wrapping_add(60000) as i64,
        );
        assert_eq!(jit_narrow_binop("add", types::I16, true, 60000, 60000), 54464);
    }

    #[test]
    fn narrow_u16_sub_overflow_wraps() {
        assert_eq!(
            jit_narrow_binop("sub", types::I16, true, 10000, 60000),
            (10000u16).wrapping_sub(60000) as i64,
        );
        assert_eq!(jit_narrow_binop("sub", types::I16, true, 10000, 60000), 15536);
    }

    #[test]
    fn narrow_u16_mul_overflow_wraps() {
        assert_eq!(
            jit_narrow_binop("mul", types::I16, true, 1000, 1000),
            (1000u16).wrapping_mul(1000) as i64,
        );
        assert_eq!(jit_narrow_binop("mul", types::I16, true, 1000, 1000), 16960);
    }

    #[test]
    fn narrow_u32_add_overflow_wraps() {
        // 4e9u32 + 4e9u32 = 8e9 wraps to 3_705_032_704.
        assert_eq!(
            jit_narrow_binop("add", types::I32, true, 4_000_000_000, 4_000_000_000),
            (4_000_000_000u32).wrapping_add(4_000_000_000) as i64,
        );
        assert_eq!(
            jit_narrow_binop("add", types::I32, true, 4_000_000_000, 4_000_000_000),
            3_705_032_704,
        );
    }

    #[test]
    fn narrow_u32_sub_overflow_wraps() {
        assert_eq!(
            jit_narrow_binop("sub", types::I32, true, 1_000_000_000, 4_000_000_000),
            (1_000_000_000u32).wrapping_sub(4_000_000_000) as i64,
        );
        assert_eq!(
            jit_narrow_binop("sub", types::I32, true, 1_000_000_000, 4_000_000_000),
            1_294_967_296,
        );
    }

    #[test]
    fn narrow_u32_mul_overflow_wraps() {
        assert_eq!(
            jit_narrow_binop("mul", types::I32, true, 100_000, 100_000),
            (100_000u32).wrapping_mul(100_000) as i64,
        );
        assert_eq!(
            jit_narrow_binop("mul", types::I32, true, 100_000, 100_000),
            1_410_065_408,
        );
    }

    #[test]
    fn narrow_no_overflow_is_exact() {
        // In-range narrow arithmetic is unaffected by the truncation.
        assert_eq!(jit_narrow_binop("add", types::I8, false, 5, 7), 12);
        assert_eq!(jit_narrow_binop("sub", types::I16, false, 100, 40), 60);
        assert_eq!(jit_narrow_binop("mul", types::I32, false, 7, 6), 42);
        assert_eq!(jit_narrow_binop("add", types::I8, true, 100, 50), 150);
    }

    // ── R5c-2-β-γ (b) u64-carrier JIT-codegen regression tests ──────────
    //
    // These mirror `compile_binop_uint64`'s codegen exactly: `u64` and
    // `i64` share the 64-bit Cranelift `I64` width (no `ireduce`), so the
    // operands flow through unchanged. Add/Sub/Mul are `iadd`/`isub`/
    // `imul` (two's-complement — signedness-agnostic, wrap at 2^64);
    // Div/Mod are `udiv`/`urem` (UNSIGNED — a `sdiv` would reinterpret
    // `u64::MAX` as `-1`); comparisons use the `Unsigned*` condition
    // codes. The JIT must match the bytecode VM's u64 arithmetic
    // (`compact_int_divmod_u64`, `int_cmp_is_unsigned`) byte-for-byte.
    //
    // Standalone Cranelift fns — fast, deterministic, not `deep-tests`-gated.

    /// Build a JIT fn `(i64, i64) -> i64` using `compile_binop_uint64`'s
    /// codegen pattern. Operands and result are raw 64-bit values; `udiv`/
    /// `urem` decode the full unsigned range. Caller passes `u64` operands
    /// reinterpreted through `as i64` and reads the result back the same way.
    fn jit_u64_binop(op: &str, a: u64, b: u64) -> u64 {
        let mut flag_builder = settings::builder();
        flag_builder.set("opt_level", "speed_and_size").unwrap();
        let isa = cranelift_native::builder()
            .unwrap()
            .finish(settings::Flags::new(flag_builder))
            .unwrap();
        let builder =
            JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());
        let mut module = JITModule::new(builder);
        let mut ctx = module.make_context();

        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));

        let func_id = module
            .declare_function("u64_fn", cranelift_module::Linkage::Local, &sig)
            .unwrap();
        ctx.func.signature = sig;

        let mut fbc = FunctionBuilderContext::new();
        let mut builder = FunctionBuilder::new(&mut ctx.func, &mut fbc);
        let block = builder.create_block();
        builder.append_block_params_for_function_params(block);
        builder.switch_to_block(block);
        builder.seal_block(block);

        let l = builder.block_params(block)[0];
        let r = builder.block_params(block)[1];
        let result = match op {
            "add" => builder.ins().iadd(l, r),
            "sub" => builder.ins().isub(l, r),
            "mul" => builder.ins().imul(l, r),
            "div" => builder.ins().udiv(l, r),
            "mod" => builder.ins().urem(l, r),
            _ => panic!("unknown op: {}", op),
        };
        builder.ins().return_(&[result]);
        builder.finalize();

        module.define_function(func_id, &mut ctx).unwrap();
        module.clear_context(&mut ctx);
        module.finalize_definitions().unwrap();

        let code_ptr = module.get_finalized_function(func_id);
        let func: fn(u64, u64) -> u64 = unsafe { std::mem::transmute(code_ptr) };
        func(a, b)
    }

    /// Build a JIT fn `(i64, i64) -> i64` using `compile_binop_uint64`'s
    /// comparison codegen — `Unsigned*` condition codes.
    fn jit_u64_cmp(op: &str, a: u64, b: u64) -> bool {
        let mut flag_builder = settings::builder();
        flag_builder.set("opt_level", "speed_and_size").unwrap();
        let isa = cranelift_native::builder()
            .unwrap()
            .finish(settings::Flags::new(flag_builder))
            .unwrap();
        let builder =
            JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());
        let mut module = JITModule::new(builder);
        let mut ctx = module.make_context();

        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));

        let func_id = module
            .declare_function("u64_cmp", cranelift_module::Linkage::Local, &sig)
            .unwrap();
        ctx.func.signature = sig;

        let mut fbc = FunctionBuilderContext::new();
        let mut builder = FunctionBuilder::new(&mut ctx.func, &mut fbc);
        let block = builder.create_block();
        builder.append_block_params_for_function_params(block);
        builder.switch_to_block(block);
        builder.seal_block(block);

        let l = builder.block_params(block)[0];
        let r = builder.block_params(block)[1];
        let cc = match op {
            "lt" => IntCC::UnsignedLessThan,
            "le" => IntCC::UnsignedLessThanOrEqual,
            "gt" => IntCC::UnsignedGreaterThan,
            "ge" => IntCC::UnsignedGreaterThanOrEqual,
            "eq" => IntCC::Equal,
            "ne" => IntCC::NotEqual,
            _ => panic!("unknown cmp: {}", op),
        };
        let cmp = builder.ins().icmp(cc, l, r);
        let t = builder.ins().iconst(types::I64, 1);
        let f = builder.ins().iconst(types::I64, 0);
        let result = builder.ins().select(cmp, t, f);
        builder.ins().return_(&[result]);
        builder.finalize();

        module.define_function(func_id, &mut ctx).unwrap();
        module.clear_context(&mut ctx);
        module.finalize_definitions().unwrap();

        let code_ptr = module.get_finalized_function(func_id);
        let func: fn(u64, u64) -> u64 = unsafe { std::mem::transmute(code_ptr) };
        func(a, b) != 0
    }

    #[test]
    fn u64_jit_add_exact() {
        assert_eq!(
            jit_u64_binop("add", 10_000_000_000_000_000_000, 5),
            10_000_000_000_000_000_005
        );
    }

    #[test]
    fn u64_jit_add_wraps_at_2_pow_64() {
        assert_eq!(jit_u64_binop("add", u64::MAX, 1), 0);
    }

    #[test]
    fn u64_jit_sub_wraps_below_zero() {
        assert_eq!(jit_u64_binop("sub", 0, 1), u64::MAX);
    }

    #[test]
    fn u64_jit_mul_wraps_at_2_pow_64() {
        assert_eq!(
            jit_u64_binop("mul", 10_000_000_000_000_000_000, 10_000_000_000_000_000_000),
            10_000_000_000_000_000_000u64.wrapping_mul(10_000_000_000_000_000_000),
        );
    }

    #[test]
    fn u64_jit_div_is_unsigned() {
        // u64::MAX / 2 — unsigned udiv: 9223372036854775807.
        // A signed sdiv would compute (-1) / 2 == 0.
        assert_eq!(jit_u64_binop("div", u64::MAX, 2), 9_223_372_036_854_775_807);
    }

    #[test]
    fn u64_jit_mod_is_unsigned() {
        // u64::MAX % 10 == 5 (unsigned urem). Signed srem gives -1.
        assert_eq!(jit_u64_binop("mod", u64::MAX, 10), 5);
    }

    #[test]
    fn u64_jit_div_full_range_operands() {
        // Both operands above i64::MAX: (2^64-2) / (2^63) == 1.
        assert_eq!(jit_u64_binop("div", u64::MAX - 1, 1u64 << 63), 1);
    }

    #[test]
    fn u64_jit_gt_above_i64_max_is_greater() {
        // u64::MAX > 2 — true under unsigned compare.
        assert!(jit_u64_cmp("gt", u64::MAX, 2));
        assert!(!jit_u64_cmp("lt", u64::MAX, 2));
    }

    #[test]
    fn u64_jit_cmp_full_range() {
        assert!(jit_u64_cmp("ge", u64::MAX, u64::MAX));
        assert!(jit_u64_cmp("le", 1u64 << 63, u64::MAX));
        assert!(jit_u64_cmp("eq", u64::MAX, u64::MAX));
        assert!(jit_u64_cmp("ne", u64::MAX, u64::MAX - 1));
    }

    // ── r5c-2-gz-cp2-jit-div: trap-free integer div/mod regression tests ──
    //
    // These mirror `compile_int_divmod_guarded` (in `rvalues.rs`) codegen
    // exactly, for every integer width:
    //
    //   1. Divisor-is-zero guard. The production helper does an immediate
    //      `return_` of `JIT_SIGNAL_DIVISION_BY_ZERO`; a standalone test fn
    //      returning the division result cannot early-return an i32 signal,
    //      so the harness returns a distinct sentinel (`SENTINEL_DIVZERO`) on
    //      the zero-divisor branch. The assertion proves the branch is taken
    //      and — crucially — that NO `ud2`/SIGILL trap fires (the test
    //      process survives; the prior `trapnz` would have aborted it).
    //
    //   2. `INT_MIN / -1` signed-overflow substitution. The helper replaces
    //      the divisor with `1` when `divisor == -1 && dividend == INT_MIN`,
    //      so `sdiv(INT_MIN, 1) == INT_MIN` (== `wrapping_div`) and
    //      `srem(INT_MIN, 1) == 0` (== `wrapping_rem`). The harness replicates
    //      the `select` substitution; the prior raw `sdiv` would SIGFPE here.
    //
    //   3. Ordinary division/modulo is bit-identical to the prior `sdiv`/
    //      `srem`/`udiv`/`urem` — the guard and substitution never trigger.
    //
    // Standalone Cranelift fns — fast, deterministic, not `deep-tests`-gated.

    /// Sentinel returned by the test harness on the zero-divisor branch.
    /// In production this branch instead does `return_(&[i32 signal])`.
    const SENTINEL_DIVZERO: i64 = 0x7EAD_BEEF_DEAD_BEEFu64 as i64;

    /// Build a JIT fn `(i64, i64) -> i64` replicating `compile_int_divmod_guarded`
    /// codegen at `narrow` Cranelift width. `is_signed` selects `sdiv`/`srem` +
    /// the `INT_MIN / -1` substitution vs `udiv`/`urem`; `is_mod` selects
    /// remainder vs quotient. The zero-divisor branch returns
    /// `SENTINEL_DIVZERO` (the test analog of the production i32 signal
    /// early-return — proving the branch is taken without a trap). The
    /// operands are `ireduce`d to `narrow` and the result widened back to i64.
    fn jit_guarded_divmod(
        narrow: types::Type,
        is_signed: bool,
        is_mod: bool,
        dividend: i64,
        divisor: i64,
    ) -> i64 {
        let mut flag_builder = settings::builder();
        flag_builder.set("opt_level", "speed_and_size").unwrap();
        let isa = cranelift_native::builder()
            .unwrap()
            .finish(settings::Flags::new(flag_builder))
            .unwrap();
        let builder =
            JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());
        let mut module = JITModule::new(builder);
        let mut ctx = module.make_context();

        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));

        let func_id = module
            .declare_function("guarded_divmod", cranelift_module::Linkage::Local, &sig)
            .unwrap();
        ctx.func.signature = sig;

        let mut fbc = FunctionBuilderContext::new();
        let mut builder = FunctionBuilder::new(&mut ctx.func, &mut fbc);
        let entry = builder.create_block();
        builder.append_block_params_for_function_params(entry);
        builder.switch_to_block(entry);
        builder.seal_block(entry);

        // Cranelift's `iconst` rejects a raw negative `i64` for sub-`I64`
        // types — the immediate must be the zero-extended bit pattern. This
        // mirrors `MirToIR::narrow_iconst` in `rvalues.rs`.
        let narrow_imm = |value: i64| -> i64 {
            let bits = narrow.bits();
            if bits >= 64 {
                value
            } else {
                (value as u64 & ((1u64 << bits) - 1)) as i64
            }
        };

        let lhs_i64 = builder.block_params(entry)[0];
        let rhs_i64 = builder.block_params(entry)[1];
        let l = if narrow == types::I64 {
            lhs_i64
        } else {
            builder.ins().ireduce(narrow, lhs_i64)
        };
        let r = if narrow == types::I64 {
            rhs_i64
        } else {
            builder.ins().ireduce(narrow, rhs_i64)
        };

        // ── 1. Divisor-is-zero guard ─────────────────────────────────────
        let zero = builder.ins().iconst(narrow, 0);
        let is_zero = builder.ins().icmp(IntCC::Equal, r, zero);
        let div_by_zero_block = builder.create_block();
        let continue_block = builder.create_block();
        builder
            .ins()
            .brif(is_zero, div_by_zero_block, &[], continue_block, &[]);

        builder.switch_to_block(div_by_zero_block);
        builder.seal_block(div_by_zero_block);
        let sentinel = builder.ins().iconst(types::I64, SENTINEL_DIVZERO);
        builder.ins().return_(&[sentinel]);

        builder.switch_to_block(continue_block);
        builder.seal_block(continue_block);

        let narrow_result = if !is_signed {
            if is_mod {
                builder.ins().urem(l, r)
            } else {
                builder.ins().udiv(l, r)
            }
        } else {
            // ── 2. INT_MIN / -1 substitution ─────────────────────────────
            let neg_one = builder.ins().iconst(narrow, narrow_imm(-1));
            let int_min = builder
                .ins()
                .iconst(narrow, narrow_imm(i64::MIN >> (64 - narrow.bits())));
            let div_is_neg_one = builder.ins().icmp(IntCC::Equal, r, neg_one);
            let dividend_is_min = builder.ins().icmp(IntCC::Equal, l, int_min);
            let is_overflow = builder.ins().band(div_is_neg_one, dividend_is_min);
            let one = builder.ins().iconst(narrow, narrow_imm(1));
            let safe_divisor = builder.ins().select(is_overflow, one, r);
            if is_mod {
                builder.ins().srem(l, safe_divisor)
            } else {
                builder.ins().sdiv(l, safe_divisor)
            }
        };

        let widened = if narrow == types::I64 {
            narrow_result
        } else if is_signed {
            builder.ins().sextend(types::I64, narrow_result)
        } else {
            builder.ins().uextend(types::I64, narrow_result)
        };
        builder.ins().return_(&[widened]);
        builder.finalize();

        module.define_function(func_id, &mut ctx).unwrap();
        module.clear_context(&mut ctx);
        module.finalize_definitions().unwrap();
        let code_ptr = module.get_finalized_function(func_id);
        let func: fn(i64, i64) -> i64 = unsafe { std::mem::transmute(code_ptr) };
        func(dividend, divisor)
    }

    // ── i64: division/modulo by zero is guarded, not trapped ────────────

    #[test]
    fn i64_div_by_zero_is_guarded_no_sigill() {
        // Prior codegen: `trapnz` → `ud2` → SIGILL aborting the process.
        // Now: the guard branch returns the sentinel; the process survives.
        assert_eq!(jit_guarded_divmod(types::I64, true, false, 1, 0), SENTINEL_DIVZERO);
        assert_eq!(jit_guarded_divmod(types::I64, true, false, -42, 0), SENTINEL_DIVZERO);
    }

    #[test]
    fn i64_mod_by_zero_is_guarded_no_sigill() {
        assert_eq!(jit_guarded_divmod(types::I64, true, true, 10, 0), SENTINEL_DIVZERO);
    }

    #[test]
    fn i64_div_int_min_by_neg_one_wraps_no_sigfpe() {
        // Prior codegen: raw `sdiv(i64::MIN, -1)` → SIGFPE. Now: the divisor
        // substitution yields `sdiv(i64::MIN, 1) == i64::MIN` — the exact
        // `i64::MIN.wrapping_div(-1)` result the VM produces.
        assert_eq!(
            jit_guarded_divmod(types::I64, true, false, i64::MIN, -1),
            i64::MIN,
        );
        assert_eq!(
            jit_guarded_divmod(types::I64, true, false, i64::MIN, -1),
            i64::MIN.wrapping_div(-1),
        );
    }

    #[test]
    fn i64_mod_int_min_by_neg_one_is_zero_no_sigfpe() {
        // `srem(i64::MIN, 1) == 0` == `i64::MIN.wrapping_rem(-1)`.
        assert_eq!(jit_guarded_divmod(types::I64, true, true, i64::MIN, -1), 0);
        assert_eq!(
            jit_guarded_divmod(types::I64, true, true, i64::MIN, -1),
            i64::MIN.wrapping_rem(-1),
        );
    }

    #[test]
    fn i64_ordinary_div_mod_unaffected() {
        assert_eq!(jit_guarded_divmod(types::I64, true, false, 17, 5), 3);
        assert_eq!(jit_guarded_divmod(types::I64, true, true, 17, 5), 2);
        // -5 / -1 must still be 5 — the substitution only fires for INT_MIN.
        assert_eq!(jit_guarded_divmod(types::I64, true, false, -5, -1), 5);
        assert_eq!(jit_guarded_divmod(types::I64, true, false, -20, 4), -5);
        assert_eq!(jit_guarded_divmod(types::I64, true, true, -20, 6), -2);
    }

    // ── i32: same guard + substitution at 32-bit width ──────────────────

    #[test]
    fn i32_div_by_zero_is_guarded_no_sigill() {
        assert_eq!(jit_guarded_divmod(types::I32, true, false, 7, 0), SENTINEL_DIVZERO);
        assert_eq!(jit_guarded_divmod(types::I32, true, true, 7, 0), SENTINEL_DIVZERO);
    }

    #[test]
    fn i32_div_int_min_by_neg_one_wraps_no_sigfpe() {
        assert_eq!(
            jit_guarded_divmod(types::I32, true, false, i32::MIN as i64, -1),
            i32::MIN as i64,
        );
        assert_eq!(
            jit_guarded_divmod(types::I32, true, true, i32::MIN as i64, -1),
            0,
        );
    }

    #[test]
    fn i32_ordinary_div_mod_unaffected() {
        assert_eq!(jit_guarded_divmod(types::I32, true, false, 17, 5), 3);
        assert_eq!(jit_guarded_divmod(types::I32, true, true, 17, 5), 2);
        assert_eq!(jit_guarded_divmod(types::I32, true, false, -100, -1), 100);
    }

    // ── narrow signed (i8 / i16): same guard + substitution ─────────────

    #[test]
    fn narrow_i8_div_by_zero_is_guarded_no_sigill() {
        assert_eq!(jit_guarded_divmod(types::I8, true, false, 9, 0), SENTINEL_DIVZERO);
        assert_eq!(jit_guarded_divmod(types::I16, true, true, 9, 0), SENTINEL_DIVZERO);
    }

    #[test]
    fn narrow_i8_div_int_min_by_neg_one_wraps_no_sigfpe() {
        // i8::MIN / -1 wraps to i8::MIN (-128); i8::MIN % -1 == 0.
        assert_eq!(
            jit_guarded_divmod(types::I8, true, false, i8::MIN as i64, -1),
            i8::MIN as i64,
        );
        assert_eq!(
            jit_guarded_divmod(types::I8, true, true, i8::MIN as i64, -1),
            0,
        );
    }

    #[test]
    fn narrow_i16_div_int_min_by_neg_one_wraps_no_sigfpe() {
        assert_eq!(
            jit_guarded_divmod(types::I16, true, false, i16::MIN as i64, -1),
            i16::MIN as i64,
        );
        assert_eq!(
            jit_guarded_divmod(types::I16, true, true, i16::MIN as i64, -1),
            0,
        );
    }

    #[test]
    fn narrow_signed_ordinary_div_mod_unaffected() {
        assert_eq!(jit_guarded_divmod(types::I8, true, false, 100, 7), 14);
        assert_eq!(jit_guarded_divmod(types::I8, true, true, 100, 7), 2);
        // -50 / -1 == 50 — substitution must not fire for non-INT_MIN.
        assert_eq!(jit_guarded_divmod(types::I8, true, false, -50, -1), 50);
        assert_eq!(jit_guarded_divmod(types::I16, true, false, 30000, 3), 10000);
    }

    // ── u64 / narrow unsigned: only the zero-divisor guard applies ──────

    #[test]
    fn u64_div_by_zero_is_guarded_no_sigill() {
        assert_eq!(jit_guarded_divmod(types::I64, false, false, 100, 0), SENTINEL_DIVZERO);
        assert_eq!(jit_guarded_divmod(types::I64, false, true, 100, 0), SENTINEL_DIVZERO);
    }

    #[test]
    fn u64_div_full_range_is_unsigned() {
        // u64::MAX / 2 — unsigned udiv (a signed sdiv would compute 0).
        // No overflow case exists for unsigned division.
        let q = jit_guarded_divmod(types::I64, false, false, u64::MAX as i64, 2) as u64;
        assert_eq!(q, 9_223_372_036_854_775_807);
        let r = jit_guarded_divmod(types::I64, false, true, u64::MAX as i64, 10) as u64;
        assert_eq!(r, 5);
    }

    #[test]
    fn narrow_unsigned_div_by_zero_is_guarded_no_sigill() {
        assert_eq!(jit_guarded_divmod(types::I8, false, false, 200, 0), SENTINEL_DIVZERO);
        assert_eq!(jit_guarded_divmod(types::I32, false, true, 4_000_000_000, 0), SENTINEL_DIVZERO);
    }

    #[test]
    fn narrow_unsigned_ordinary_div_mod_unaffected() {
        // 200u8 / 3 == 66; 200u8 % 3 == 2.
        assert_eq!(jit_guarded_divmod(types::I8, false, false, 200, 3), 66);
        assert_eq!(jit_guarded_divmod(types::I8, false, true, 200, 3), 2);
        // 4_000_000_000u32 / 7 — unsigned udiv at 32-bit width.
        assert_eq!(
            jit_guarded_divmod(types::I32, false, false, 4_000_000_000, 7),
            (4_000_000_000u32 / 7) as i64,
        );
    }

    // ── r5c-2-gz-cp6 narrow-neg-literal regression tests ────────────────
    //
    // These mirror `compile_binop_narrow_int`'s COMPARISON codegen exactly:
    // each operand is widened to I64 via `extend_narrow_to_i64` (sextend for
    // the signed narrow widths, uextend for the unsigned narrow widths) and
    // the `icmp` runs at I64 — byte-equal to the VM's `compact_int_cmp`,
    // which compares the full sign-/zero-extended i64 slot bits and never
    // re-truncates.
    //
    // The bug they pin: pre-cp6 a `(narrow-var, Int64-literal)` comparison
    // fell to the kind-blind generic `compile_binop_dynamic_cmp`, whose
    // `to_i64_bits` ZERO-extends an `I8` operand. For a NEGATIVE narrow
    // value the zero-extend produced a large positive i64 that mis-compared
    // against the (sign-extended) literal — `(a+b) == -56` for `i8` gave
    // JIT `false` against VM `true`. Positive values coincided.
    //
    // Standalone Cranelift fns — fast, deterministic, not `deep-tests`-gated.

    /// Build a JIT fn `(i64, i64) -> u64` using `compile_binop_narrow_int`'s
    /// COMPARISON codegen pattern for the given narrow Cranelift width.
    ///
    /// The first operand (`a`) models a narrow variable: it is `ireduce`d to
    /// `narrow` (mirroring how a narrow local is read at its declared width)
    /// then `extend_narrow_to_i64`'d back per `unsigned`. The second operand
    /// (`b`) models the width-polymorphic literal / `int` partner: it stays
    /// at the full I64 width. `icmp` then runs at I64 with the signed /
    /// unsigned condition code selected by `unsigned`. Returns 1 / 0.
    fn jit_narrow_cmp(
        op: &str,
        narrow: types::Type,
        unsigned: bool,
        a: i64,
        b: i64,
    ) -> u64 {
        let mut flag_builder = settings::builder();
        flag_builder.set("opt_level", "speed_and_size").unwrap();
        let isa = cranelift_native::builder()
            .unwrap()
            .finish(settings::Flags::new(flag_builder))
            .unwrap();
        let builder =
            JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());
        let mut module = JITModule::new(builder);
        let mut ctx = module.make_context();

        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));

        let func_id = module
            .declare_function("narrow_cmp_fn", cranelift_module::Linkage::Local, &sig)
            .unwrap();
        ctx.func.signature = sig;

        let mut fbc = FunctionBuilderContext::new();
        let mut builder = FunctionBuilder::new(&mut ctx.func, &mut fbc);
        let block = builder.create_block();
        builder.append_block_params_for_function_params(block);
        builder.switch_to_block(block);
        builder.seal_block(block);

        let lhs = builder.block_params(block)[0];
        let rhs = builder.block_params(block)[1];
        // `a` → narrow variable: truncate to the declared narrow width …
        let l_narrow = builder.ins().ireduce(narrow, lhs);
        // … then re-extend to I64 per the kind's signedness, exactly as
        // `extend_narrow_to_i64` does in `compile_binop_narrow_int`.
        let l = if unsigned {
            builder.ins().uextend(types::I64, l_narrow)
        } else {
            builder.ins().sextend(types::I64, l_narrow)
        };
        // `b` → width-polymorphic literal / `int`: kept at full I64 width.
        let r = rhs;
        let cc = match (op, unsigned) {
            ("eq", _) => IntCC::Equal,
            ("ne", _) => IntCC::NotEqual,
            ("lt", false) => IntCC::SignedLessThan,
            ("lt", true) => IntCC::UnsignedLessThan,
            ("le", false) => IntCC::SignedLessThanOrEqual,
            ("le", true) => IntCC::UnsignedLessThanOrEqual,
            ("gt", false) => IntCC::SignedGreaterThan,
            ("gt", true) => IntCC::UnsignedGreaterThan,
            ("ge", false) => IntCC::SignedGreaterThanOrEqual,
            ("ge", true) => IntCC::UnsignedGreaterThanOrEqual,
            _ => panic!("unknown cmp: {}", op),
        };
        let cmp = builder.ins().icmp(cc, l, r);
        let true_val = builder.ins().iconst(types::I64, 1);
        let false_val = builder.ins().iconst(types::I64, 0);
        let result = builder.ins().select(cmp, true_val, false_val);
        builder.ins().return_(&[result]);
        builder.finalize();

        module.define_function(func_id, &mut ctx).unwrap();
        module.clear_context(&mut ctx);
        module.finalize_definitions().unwrap();
        let code_ptr = module.get_finalized_function(func_id);
        let func: fn(i64, i64) -> u64 = unsafe { std::mem::transmute(code_ptr) };
        func(a, b)
    }

    // ── i8 == against a negative literal — the canonical reproducer ──

    #[test]
    fn narrow_i8_eq_negative_literal_canonical_repro() {
        // `let a:i8=100; let b:i8=100; (a+b) == -56`: i8 add wraps to -56.
        // The narrow `a+b` temp holds the i8 bit-pattern 0xC8; comparing it
        // against the literal -56 must be `true` — VM-equal.
        let sum: i64 = (100i8).wrapping_add(100) as i64; // -56
        assert_eq!(jit_narrow_cmp("eq", types::I8, false, sum, -56), 1);
        assert_eq!(jit_narrow_cmp("eq", types::I8, false, -56, -56), 1);
        // Sanity: positive literal still works (zero/sign-extension agree).
        assert_eq!(jit_narrow_cmp("eq", types::I8, false, 56, 56), 1);
    }

    #[test]
    fn narrow_i8_eq_ne_negative_and_positive_literal() {
        // -56 == -56 true; -56 == -10 false; -56 != -10 true.
        assert_eq!(jit_narrow_cmp("eq", types::I8, false, -56, -56), 1);
        assert_eq!(jit_narrow_cmp("eq", types::I8, false, -56, -10), 0);
        assert_eq!(jit_narrow_cmp("ne", types::I8, false, -56, -10), 1);
        assert_eq!(jit_narrow_cmp("ne", types::I8, false, -56, -56), 0);
        // Positive literals.
        assert_eq!(jit_narrow_cmp("eq", types::I8, false, 42, 42), 1);
        assert_eq!(jit_narrow_cmp("ne", types::I8, false, 42, 7), 1);
    }

    #[test]
    fn narrow_i8_ordered_cmp_negative_literal() {
        // -56 < -10 true; -56 <= -56 true; -56 > -100 true; -56 >= -10 false.
        assert_eq!(jit_narrow_cmp("lt", types::I8, false, -56, -10), 1);
        assert_eq!(jit_narrow_cmp("le", types::I8, false, -56, -56), 1);
        assert_eq!(jit_narrow_cmp("gt", types::I8, false, -56, -100), 1);
        assert_eq!(jit_narrow_cmp("ge", types::I8, false, -56, -10), 0);
        // Mixed-sign: -56 < 10 true; -56 > 10 false.
        assert_eq!(jit_narrow_cmp("lt", types::I8, false, -56, 10), 1);
        assert_eq!(jit_narrow_cmp("gt", types::I8, false, -56, 10), 0);
    }

    #[test]
    fn narrow_i16_cmp_negative_and_positive_literal() {
        let v: i64 = -5536; // 30000i16 + 30000i16 wraps to -5536
        assert_eq!(jit_narrow_cmp("eq", types::I16, false, v, -5536), 1);
        assert_eq!(jit_narrow_cmp("ne", types::I16, false, v, -5536), 0);
        assert_eq!(jit_narrow_cmp("lt", types::I16, false, v, -100), 1);
        assert_eq!(jit_narrow_cmp("le", types::I16, false, v, v), 1);
        assert_eq!(jit_narrow_cmp("gt", types::I16, false, v, -10000), 1);
        assert_eq!(jit_narrow_cmp("ge", types::I16, false, v, -10000), 1);
        // Positive literal.
        assert_eq!(jit_narrow_cmp("eq", types::I16, false, 12345, 12345), 1);
    }

    #[test]
    fn narrow_i32_cmp_negative_and_positive_literal() {
        let v: i64 = (2_000_000_000i32).wrapping_add(2_000_000_000) as i64; // -294967296
        assert_eq!(jit_narrow_cmp("eq", types::I32, false, v, -294_967_296), 1);
        assert_eq!(jit_narrow_cmp("ne", types::I32, false, v, 0), 1);
        assert_eq!(jit_narrow_cmp("lt", types::I32, false, v, -1), 1);
        assert_eq!(jit_narrow_cmp("le", types::I32, false, v, v), 1);
        assert_eq!(jit_narrow_cmp("gt", types::I32, false, v, -1_000_000_000), 1);
        assert_eq!(jit_narrow_cmp("ge", types::I32, false, v, v), 1);
        // Positive literal.
        assert_eq!(jit_narrow_cmp("eq", types::I32, false, 123_456, 123_456), 1);
    }

    #[test]
    fn narrow_unsigned_cmp_uses_uextend_not_sextend() {
        // A `u8` value above the signed-i8 boundary (200) must zero-extend:
        // 200 == 200 true (sextend would make it -56 and mis-compare).
        assert_eq!(jit_narrow_cmp("eq", types::I8, true, 200, 200), 1);
        assert_eq!(jit_narrow_cmp("ne", types::I8, true, 200, 100), 1);
        // Unsigned ordering: 200u8 > 100 true; 200u8 < 100 false.
        assert_eq!(jit_narrow_cmp("gt", types::I8, true, 200, 100), 1);
        assert_eq!(jit_narrow_cmp("lt", types::I8, true, 200, 100), 0);
        assert_eq!(jit_narrow_cmp("ge", types::I8, true, 200, 200), 1);
        assert_eq!(jit_narrow_cmp("le", types::I8, true, 100, 200), 1);
        // u16 / u32 above their signed boundaries.
        assert_eq!(jit_narrow_cmp("eq", types::I16, true, 60_000, 60_000), 1);
        assert_eq!(jit_narrow_cmp("gt", types::I16, true, 60_000, 1_000), 1);
        assert_eq!(jit_narrow_cmp("eq", types::I32, true, 4_000_000_000, 4_000_000_000), 1);
        assert_eq!(jit_narrow_cmp("gt", types::I32, true, 4_000_000_000, 1), 1);
    }

    #[test]
    fn narrow_cmp_out_of_range_literal_not_truncated() {
        // `let c:i8=44; c == 300` — the VM compares 44 against the FULL
        // literal 300 (`44 != 300` → false). Comparing at I64 width (the
        // partner kept at its real value) reproduces that; truncating the
        // literal to the i8 window would mask 300 → 44 and wrongly report
        // equal. The narrow operand `a` is in-range (44); the I64 partner
        // `b` is the out-of-range 300.
        assert_eq!(jit_narrow_cmp("eq", types::I8, false, 44, 300), 0);
        assert_eq!(jit_narrow_cmp("ne", types::I8, false, 44, 300), 1);
        assert_eq!(jit_narrow_cmp("lt", types::I8, false, 44, 300), 1);
        // i16 out-of-range partner.
        assert_eq!(jit_narrow_cmp("eq", types::I16, false, 100, 100_000), 0);
        assert_eq!(jit_narrow_cmp("lt", types::I16, false, 100, 100_000), 1);
    }

    #[test]
    fn narrow_cmp_against_int_variable_signed_extends() {
        // `let a:i8=-1; let n:int=5; a < n` — signed compare, -1 < 5 true.
        // The narrow operand sign-extends; the `int` partner stays I64.
        assert_eq!(jit_narrow_cmp("lt", types::I8, false, -1, 5), 1);
        assert_eq!(jit_narrow_cmp("eq", types::I8, false, -1, 5), 0);
        assert_eq!(jit_narrow_cmp("gt", types::I8, false, -1, -100), 1);
        // i32 narrow vs a large positive `int`.
        assert_eq!(jit_narrow_cmp("lt", types::I32, false, -1, 9_000_000_000), 1);
    }
}
