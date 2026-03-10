//! Arithmetic, comparison, and logical operations

use cranelift::prelude::*;

use crate::nan_boxing::*;
use shape_vm::bytecode::{Instruction, NumericWidth, OpCode, Operand};
use shape_vm::type_tracking::StorageHint;

use crate::translator::types::BytecodeToIR;

impl<'a, 'b> BytecodeToIR<'a, 'b> {
    // Arithmetic operations
    pub(crate) fn compile_add(&mut self) -> Result<(), String> {
        if let Some(hint) = self
            .top_two_integer_result_hint()
            .filter(|hint| hint.is_integer_family() && !hint.is_default_int_family())
            .map(|hint| hint.non_nullable())
        {
            self.width_integer_binary_op(hint, |b, a, c| b.ins().iadd(a, c));
            return Ok(());
        }
        // Raw i64 fast path: inside unboxed integer loops
        if self.typed_stack.either_top_i64() {
            if self.typed_stack.both_top_i64() {
                self.raw_int64_binary_op(|b, a, c| b.ins().iadd(a, c));
            } else {
                // Generic Add uses numeric (f64) semantics.
                self.mixed_numeric_binary_op(|b, a, c| b.ins().fadd(a, c));
            }
            return Ok(());
        }
        // Mixed/raw f64 fast path: one side is a float-unboxed local/result.
        if self.typed_stack.either_top_f64() {
            if self.typed_stack.both_top_f64() {
                self.raw_f64_binary_op(|b, a, c| b.ins().fadd(a, c));
            } else {
                self.mixed_numeric_binary_op(|b, a, c| b.ins().fadd(a, c));
            }
            return Ok(());
        }
        // Known non-numeric (String, Bool, etc.): dispatch to generic_add FFI
        // which handles string concatenation, Time+Duration, etc.
        if self.either_operand_non_numeric() {
            if self.stack_len() >= 2 {
                let b = self.stack_pop().unwrap();
                let a = self.stack_pop().unwrap();
                let inst = self.builder.ins().call(self.ffi.generic_add, &[a, b]);
                let result = self.builder.inst_results(inst)[0];
                self.stack_push(result);
            }
            return Ok(());
        }
        // Feedback-guided speculation: if we have monomorphic type feedback
        // for this instruction, emit a guarded typed fast path.
        if self.has_feedback() && self.try_speculative_add(self.current_instr_idx) {
            return Ok(());
        }
        // Unknown types: runtime check — numeric fast path with generic_add fallback
        self.generic_binary_op_with_fallback(|b, a_f64, b_f64| b.ins().fadd(a_f64, b_f64), true);
        Ok(())
    }

    pub(crate) fn compile_sub(&mut self) -> Result<(), String> {
        if let Some(hint) = self
            .top_two_integer_result_hint()
            .filter(|hint| hint.is_integer_family() && !hint.is_default_int_family())
            .map(|hint| hint.non_nullable())
        {
            self.width_integer_binary_op(hint, |b, a, c| b.ins().isub(a, c));
            return Ok(());
        }
        if self.typed_stack.either_top_i64() {
            if self.typed_stack.both_top_i64() {
                self.raw_int64_binary_op(|b, a, c| b.ins().isub(a, c));
            } else {
                // Generic Sub uses numeric (f64) semantics.
                self.mixed_numeric_binary_op(|b, a, c| b.ins().fsub(a, c));
            }
            return Ok(());
        }
        if self.typed_stack.either_top_f64() {
            if self.typed_stack.both_top_f64() {
                self.raw_f64_binary_op(|b, a, c| b.ins().fsub(a, c));
            } else {
                self.mixed_numeric_binary_op(|b, a, c| b.ins().fsub(a, c));
            }
            return Ok(());
        }
        // Feedback-guided speculation
        if self.has_feedback() && self.try_speculative_sub(self.current_instr_idx) {
            return Ok(());
        }
        self.nullable_float64_binary_op(|b, a_f64, b_f64| b.ins().fsub(a_f64, b_f64));
        Ok(())
    }

    pub(crate) fn compile_mul(&mut self) -> Result<(), String> {
        if let Some(hint) = self
            .top_two_integer_result_hint()
            .filter(|hint| hint.is_integer_family() && !hint.is_default_int_family())
            .map(|hint| hint.non_nullable())
        {
            self.width_integer_binary_op(hint, |b, a, c| b.ins().imul(a, c));
            return Ok(());
        }
        if self.typed_stack.either_top_i64() {
            if self.typed_stack.both_top_i64() {
                self.raw_int64_binary_op(|b, a, c| b.ins().imul(a, c));
            } else {
                // Generic Mul uses numeric (f64) semantics.
                self.mixed_numeric_binary_op(|b, a, c| b.ins().fmul(a, c));
            }
            return Ok(());
        }
        if self.typed_stack.either_top_f64() {
            if self.typed_stack.both_top_f64() {
                self.raw_f64_binary_op(|b, a, c| b.ins().fmul(a, c));
            } else {
                self.mixed_numeric_binary_op(|b, a, c| b.ins().fmul(a, c));
            }
            return Ok(());
        }
        // Feedback-guided speculation
        if self.has_feedback() && self.try_speculative_mul(self.current_instr_idx) {
            return Ok(());
        }
        self.nullable_float64_binary_op(|b, a_f64, b_f64| b.ins().fmul(a_f64, b_f64));
        Ok(())
    }

    /// Check if the preceding instruction is a PushConst with a Number whose
    /// reciprocal is exactly representable in f64. Returns Some(reciprocal) if
    /// strength reduction from fdiv to fmul is safe.
    /// Check if the divisor (top of stack) is a compile-time constant with an
    /// exact reciprocal. Scans backwards through bytecode to find the PushConst
    /// that produced the divisor, correctly handling Swap reordering.
    fn div_const_reciprocal_from_stack(&self) -> Option<f64> {
        use shape_vm::bytecode::Constant;
        if self.current_instr_idx == 0 {
            return None;
        }

        // Scan backwards to find the PushConst that produced the divisor (TOS).
        // After all preceding instructions have been compiled, the divisor is
        // at TOS position. We trace stack effects backwards to find its producer.
        let mut pos_from_top: i32 = 0; // 0 = TOS
        for k in (0..self.current_instr_idx).rev() {
            let instr_k = &self.program.instructions[k];

            // Swap is pass-through: it just reorders the top 2 values.
            // Going backwards through Swap: pos 0 ↔ pos 1.
            if instr_k.opcode == OpCode::Swap {
                if pos_from_top == 0 {
                    pos_from_top = 1;
                } else if pos_from_top == 1 {
                    pos_from_top = 0;
                }
                // positions ≥ 2 are unaffected by Swap
                continue;
            }

            let eff = Self::div_stack_effect(instr_k.opcode);
            if eff.is_none() {
                return None; // unknown opcode, give up
            }
            let (pops, pushes) = eff.unwrap();
            if pos_from_top < pushes {
                // This instruction produced our target value
                if instr_k.opcode == OpCode::PushConst {
                    let const_idx = match &instr_k.operand {
                        Some(Operand::Const(idx)) => *idx as usize,
                        _ => return None,
                    };
                    let val = match self.program.constants.get(const_idx)? {
                        Constant::Number(n) => *n,
                        Constant::Int(i) => *i as f64,
                        _ => return None,
                    };
                    if val == 0.0 || !val.is_finite() {
                        return None;
                    }
                    let recip = 1.0 / val;
                    if recip.is_finite() && Self::has_exact_reciprocal(val) {
                        return Some(recip);
                    }
                }
                return None; // producer is not PushConst
            }
            pos_from_top = pos_from_top - pushes + pops;
            if pos_from_top < 0 {
                return None; // stack underflow
            }
        }
        None
    }

    /// Returns true if val is a power of 2 (the only f64 values with exact reciprocals).
    fn has_exact_reciprocal(val: f64) -> bool {
        if !val.is_finite() || val == 0.0 {
            return false;
        }
        let bits = val.to_bits();
        let exponent = (bits >> 52) & 0x7FF;
        let mantissa = bits & 0x000F_FFFF_FFFF_FFFF;
        // Normalized number with zero mantissa = power of 2
        exponent != 0 && mantissa == 0
    }

    /// Stack effect for bytecode instructions, used by div_const_reciprocal_from_stack.
    fn div_stack_effect(op: OpCode) -> Option<(i32, i32)> {
        let eff = match op {
            // Push-only
            OpCode::LoadLocal
            | OpCode::LoadLocalTrusted
            | OpCode::LoadModuleBinding
            | OpCode::LoadClosure
            | OpCode::PushConst
            | OpCode::PushNull
            | OpCode::DerefLoad => (0, 1),
            // Unary (1→1)
            OpCode::IntToNumber
            | OpCode::NumberToInt
            | OpCode::CastWidth
            | OpCode::Neg
            | OpCode::Not => (1, 1),
            // Binary ops (2→1)
            OpCode::Add | OpCode::Sub | OpCode::Mul | OpCode::Div
            | OpCode::Mod | OpCode::Pow
            | OpCode::AddInt | OpCode::SubInt | OpCode::MulInt
            | OpCode::DivInt | OpCode::ModInt | OpCode::PowInt
            | OpCode::AddIntTrusted | OpCode::SubIntTrusted
            | OpCode::MulIntTrusted | OpCode::DivIntTrusted
            | OpCode::AddNumber | OpCode::SubNumber | OpCode::MulNumber
            | OpCode::DivNumber | OpCode::ModNumber | OpCode::PowNumber
            | OpCode::AddNumberTrusted | OpCode::SubNumberTrusted
            | OpCode::MulNumberTrusted | OpCode::DivNumberTrusted
            | OpCode::Gt | OpCode::Lt | OpCode::Gte | OpCode::Lte
            | OpCode::Eq | OpCode::Neq
            | OpCode::GtInt | OpCode::LtInt | OpCode::GteInt | OpCode::LteInt
            | OpCode::EqInt | OpCode::NeqInt
            | OpCode::GtIntTrusted | OpCode::LtIntTrusted
            | OpCode::GteIntTrusted | OpCode::LteIntTrusted
            | OpCode::GtNumber | OpCode::LtNumber | OpCode::GteNumber | OpCode::LteNumber
            | OpCode::EqNumber | OpCode::NeqNumber
            | OpCode::GtNumberTrusted | OpCode::LtNumberTrusted
            | OpCode::GteNumberTrusted | OpCode::LteNumberTrusted
            | OpCode::GetProp => (2, 1),
            // Stack manipulation
            OpCode::Dup => (1, 2),
            OpCode::Swap => (2, 2),
            // Store (1→0)
            OpCode::StoreLocal | OpCode::StoreLocalTyped
            | OpCode::StoreModuleBinding | OpCode::StoreModuleBindingTyped
            | OpCode::StoreClosure | OpCode::Pop => (1, 0),
            _ => return None,
        };
        Some(eff)
    }

    pub(crate) fn compile_div(&mut self) -> Result<(), String> {
        if let Some(hint) = self
            .top_two_integer_result_hint()
            .filter(|hint| hint.is_integer_family() && !hint.is_default_int_family())
            .map(|hint| hint.non_nullable())
        {
            if hint.is_signed_integer().unwrap_or(true) {
                self.width_integer_binary_op(hint, |b, a, c| b.ins().sdiv(a, c));
            } else {
                self.width_integer_binary_op(hint, |b, a, c| b.ins().udiv(a, c));
            }
            return Ok(());
        }
        // Strength reduction: fdiv by exact-reciprocal constant → fmul.
        // fdiv throughput: ~4 cycles; fmul throughput: ~0.5 cycles on modern x86-64.
        if let Some(recip) = self.div_const_reciprocal_from_stack() {
            if self.typed_stack.either_top_i64() {
                // Check if both operands are integers BEFORE modifying the stack,
                // so we know to truncate the result (int / int -> int).
                let both_int = self.typed_stack.both_top_i64();
                // Pop divisor (the constant) and replace with reciprocal
                let _ = self.stack_pop();
                let recip_f64 = self.builder.ins().f64const(recip);
                let recip_boxed = self.f64_to_i64(recip_f64);
                self.stack_push(recip_boxed);
                self.typed_stack
                    .replace_top(crate::translator::storage::TypedValue::f64(recip_f64));
                if both_int {
                    // int / int -> int: multiply by reciprocal then truncate toward zero
                    self.mixed_numeric_binary_op(|b, a, c| {
                        let prod = b.ins().fmul(a, c);
                        b.ins().trunc(prod)
                    });
                } else {
                    self.mixed_numeric_binary_op(|b, a, c| b.ins().fmul(a, c));
                }
                return Ok(());
            }
            if self.typed_stack.either_top_f64() {
                let _ = self.stack_pop();
                let recip_f64 = self.builder.ins().f64const(recip);
                let recip_boxed = self.f64_to_i64(recip_f64);
                self.stack_push(recip_boxed);
                self.typed_stack
                    .replace_top(crate::translator::storage::TypedValue::f64(recip_f64));
                if self.typed_stack.both_top_f64() {
                    self.raw_f64_binary_op(|b, a, c| b.ins().fmul(a, c));
                } else {
                    self.mixed_numeric_binary_op(|b, a, c| b.ins().fmul(a, c));
                }
                return Ok(());
            }
        }
        if self.typed_stack.both_top_i64() {
            // Both operands are integers: int / int -> int (truncated toward zero),
            // matching VM semantics (checked_div on i64 values).
            self.mixed_numeric_binary_op(|b, a, c| {
                let div = b.ins().fdiv(a, c);
                b.ins().trunc(div)
            });
            return Ok(());
        }
        if self.typed_stack.either_top_i64() {
            // Mixed int/float: promote to f64, result is float.
            self.mixed_numeric_binary_op(|b, a, c| b.ins().fdiv(a, c));
            return Ok(());
        }
        if self.typed_stack.either_top_f64() {
            if self.typed_stack.both_top_f64() {
                self.raw_f64_binary_op(|b, a, c| b.ins().fdiv(a, c));
            } else {
                self.mixed_numeric_binary_op(|b, a, c| b.ins().fdiv(a, c));
            }
            return Ok(());
        }
        // Feedback-guided speculation
        if self.has_feedback() && self.try_speculative_div(self.current_instr_idx) {
            return Ok(());
        }
        self.nullable_float64_binary_op(|b, a_f64, b_f64| b.ins().fdiv(a_f64, b_f64));
        Ok(())
    }

    pub(crate) fn compile_mod(&mut self) -> Result<(), String> {
        if let Some(hint) = self
            .top_two_integer_result_hint()
            .filter(|hint| hint.is_integer_family() && !hint.is_default_int_family())
            .map(|hint| hint.non_nullable())
        {
            if hint.is_signed_integer().unwrap_or(true) {
                self.width_integer_binary_op(hint, |b, a, c| b.ins().srem(a, c));
            } else {
                self.width_integer_binary_op(hint, |b, a, c| b.ins().urem(a, c));
            }
            return Ok(());
        }
        if self.typed_stack.either_top_i64() {
            // Generic Mod uses numeric (f64) semantics.
            self.mixed_numeric_binary_op(|b, a_f64, b_f64| {
                let div = b.ins().fdiv(a_f64, b_f64);
                let trunced = b.ins().trunc(div);
                let product = b.ins().fmul(trunced, b_f64);
                b.ins().fsub(a_f64, product)
            });
            return Ok(());
        }
        if self.typed_stack.either_top_f64() {
            if self.typed_stack.both_top_f64() {
                self.raw_f64_binary_op(|b, a_f64, b_f64| {
                    let div = b.ins().fdiv(a_f64, b_f64);
                    let trunced = b.ins().trunc(div);
                    let product = b.ins().fmul(trunced, b_f64);
                    b.ins().fsub(a_f64, product)
                });
            } else {
                self.mixed_numeric_binary_op(|b, a_f64, b_f64| {
                    let div = b.ins().fdiv(a_f64, b_f64);
                    let trunced = b.ins().trunc(div);
                    let product = b.ins().fmul(trunced, b_f64);
                    b.ins().fsub(a_f64, product)
                });
            }
            return Ok(());
        }
        self.nullable_float64_binary_op(|b, a_f64, b_f64| {
            let div = b.ins().fdiv(a_f64, b_f64);
            let trunced = b.ins().trunc(div);
            let product = b.ins().fmul(trunced, b_f64);
            b.ins().fsub(a_f64, product)
        });
        Ok(())
    }

    pub(crate) fn compile_neg(&mut self) -> Result<(), String> {
        self.nullable_float64_unary_op(|b, a_f64| b.ins().fneg(a_f64));
        Ok(())
    }

    pub(crate) fn compile_pow(&mut self) -> Result<(), String> {
        if self.stack_len() >= 2 {
            let exp = self.stack_pop().unwrap();
            let base = self.stack_pop().unwrap();
            let inst = self.builder.ins().call(self.ffi.pow, &[base, exp]);
            let result = self.builder.inst_results(inst)[0];
            self.stack_push(result);
        }
        Ok(())
    }

    // Comparison operations - fast path for numeric, FFI fallback for Series
    pub(crate) fn compile_gt(&mut self) -> Result<(), String> {
        if let Some(hint) = self
            .top_two_integer_result_hint()
            .filter(|hint| hint.is_integer_family() && !hint.is_default_int_family())
            .map(|hint| hint.non_nullable())
        {
            self.width_integer_comparison(hint, IntCC::SignedGreaterThan);
            return Ok(());
        }
        if self.typed_stack.either_top_i64() {
            if self.typed_stack.both_top_i64() {
                self.raw_int64_comparison(IntCC::SignedGreaterThan);
            } else {
                self.mixed_numeric_comparison(FloatCC::GreaterThan);
            }
            return Ok(());
        }
        if self.typed_stack.either_top_f64() {
            if self.typed_stack.both_top_f64() {
                self.raw_f64_comparison(FloatCC::GreaterThan);
            } else {
                self.mixed_numeric_comparison(FloatCC::GreaterThan);
            }
            return Ok(());
        }
        self.typed_comparison(FloatCC::GreaterThan);
        Ok(())
    }

    pub(crate) fn compile_lt(&mut self) -> Result<(), String> {
        if let Some(hint) = self
            .top_two_integer_result_hint()
            .filter(|hint| hint.is_integer_family() && !hint.is_default_int_family())
            .map(|hint| hint.non_nullable())
        {
            self.width_integer_comparison(hint, IntCC::SignedLessThan);
            return Ok(());
        }
        if self.typed_stack.either_top_i64() {
            if self.typed_stack.both_top_i64() {
                self.raw_int64_comparison(IntCC::SignedLessThan);
            } else {
                self.mixed_numeric_comparison(FloatCC::LessThan);
            }
            return Ok(());
        }
        if self.typed_stack.either_top_f64() {
            if self.typed_stack.both_top_f64() {
                self.raw_f64_comparison(FloatCC::LessThan);
            } else {
                self.mixed_numeric_comparison(FloatCC::LessThan);
            }
            return Ok(());
        }
        self.typed_comparison(FloatCC::LessThan);
        Ok(())
    }

    pub(crate) fn compile_gte(&mut self) -> Result<(), String> {
        if let Some(hint) = self
            .top_two_integer_result_hint()
            .filter(|hint| hint.is_integer_family() && !hint.is_default_int_family())
            .map(|hint| hint.non_nullable())
        {
            self.width_integer_comparison(hint, IntCC::SignedGreaterThanOrEqual);
            return Ok(());
        }
        if self.typed_stack.either_top_i64() {
            if self.typed_stack.both_top_i64() {
                self.raw_int64_comparison(IntCC::SignedGreaterThanOrEqual);
            } else {
                self.mixed_numeric_comparison(FloatCC::GreaterThanOrEqual);
            }
            return Ok(());
        }
        if self.typed_stack.either_top_f64() {
            if self.typed_stack.both_top_f64() {
                self.raw_f64_comparison(FloatCC::GreaterThanOrEqual);
            } else {
                self.mixed_numeric_comparison(FloatCC::GreaterThanOrEqual);
            }
            return Ok(());
        }
        self.typed_comparison(FloatCC::GreaterThanOrEqual);
        Ok(())
    }

    pub(crate) fn compile_lte(&mut self) -> Result<(), String> {
        if let Some(hint) = self
            .top_two_integer_result_hint()
            .filter(|hint| hint.is_integer_family() && !hint.is_default_int_family())
            .map(|hint| hint.non_nullable())
        {
            self.width_integer_comparison(hint, IntCC::SignedLessThanOrEqual);
            return Ok(());
        }
        if self.typed_stack.either_top_i64() {
            if self.typed_stack.both_top_i64() {
                self.raw_int64_comparison(IntCC::SignedLessThanOrEqual);
            } else {
                self.mixed_numeric_comparison(FloatCC::LessThanOrEqual);
            }
            return Ok(());
        }
        if self.typed_stack.either_top_f64() {
            if self.typed_stack.both_top_f64() {
                self.raw_f64_comparison(FloatCC::LessThanOrEqual);
            } else {
                self.mixed_numeric_comparison(FloatCC::LessThanOrEqual);
            }
            return Ok(());
        }
        self.typed_comparison(FloatCC::LessThanOrEqual);
        Ok(())
    }

    pub(crate) fn compile_eq(&mut self) -> Result<(), String> {
        if let Some(hint) = self
            .top_two_integer_result_hint()
            .filter(|hint| hint.is_integer_family() && !hint.is_default_int_family())
            .map(|hint| hint.non_nullable())
        {
            self.width_integer_comparison(hint, IntCC::Equal);
            return Ok(());
        }
        if self.typed_stack.either_top_i64() {
            if self.typed_stack.both_top_i64() {
                self.raw_int64_comparison(IntCC::Equal);
            } else {
                self.mixed_numeric_comparison(FloatCC::Equal);
            }
            return Ok(());
        }
        // Known non-numeric (String, Bool): dispatch to generic_eq FFI
        // which compares string contents, not pointer identity.
        if self.either_operand_non_numeric() {
            if self.stack_len() >= 2 {
                let b = self.stack_pop().unwrap();
                let a = self.stack_pop().unwrap();
                let inst = self.builder.ins().call(self.ffi.generic_eq, &[a, b]);
                let result = self.builder.inst_results(inst)[0];
                self.stack_push(result);
            }
            return Ok(());
        }
        // Unknown types: runtime check — numeric fast path with generic_eq fallback
        self.generic_comparison_with_fallback(FloatCC::Equal);
        Ok(())
    }

    pub(crate) fn compile_neq(&mut self) -> Result<(), String> {
        if let Some(hint) = self
            .top_two_integer_result_hint()
            .filter(|hint| hint.is_integer_family() && !hint.is_default_int_family())
            .map(|hint| hint.non_nullable())
        {
            self.width_integer_comparison(hint, IntCC::NotEqual);
            return Ok(());
        }
        if self.typed_stack.either_top_i64() {
            if self.typed_stack.both_top_i64() {
                self.raw_int64_comparison(IntCC::NotEqual);
            } else {
                self.mixed_numeric_comparison(FloatCC::NotEqual);
            }
            return Ok(());
        }
        // Known non-numeric (String, Bool): dispatch to generic_neq FFI
        if self.either_operand_non_numeric() {
            if self.stack_len() >= 2 {
                let b = self.stack_pop().unwrap();
                let a = self.stack_pop().unwrap();
                let inst = self.builder.ins().call(self.ffi.generic_neq, &[a, b]);
                let result = self.builder.inst_results(inst)[0];
                self.stack_push(result);
            }
            return Ok(());
        }
        // Unknown types: runtime check — numeric fast path with generic_neq fallback
        self.generic_comparison_with_fallback(FloatCC::NotEqual);
        Ok(())
    }

    // Logical operations
    pub(crate) fn compile_and(&mut self) -> Result<(), String> {
        if self.stack_len() >= 2 {
            let b = self.stack_pop().unwrap();
            let a = self.stack_pop().unwrap();
            let true_val = self.builder.ins().iconst(types::I64, TAG_BOOL_TRUE as i64);
            let false_val = self.builder.ins().iconst(types::I64, TAG_BOOL_FALSE as i64);
            let a_true = self.is_truthy(a);
            let b_true = self.is_truthy(b);
            let both = self.builder.ins().band(a_true, b_true);
            let result = self.builder.ins().select(both, true_val, false_val);
            self.stack_push(result);
        }
        Ok(())
    }

    pub(crate) fn compile_or(&mut self) -> Result<(), String> {
        if self.stack_len() >= 2 {
            let b = self.stack_pop().unwrap();
            let a = self.stack_pop().unwrap();
            let true_val = self.builder.ins().iconst(types::I64, TAG_BOOL_TRUE as i64);
            let false_val = self.builder.ins().iconst(types::I64, TAG_BOOL_FALSE as i64);
            let a_true = self.is_truthy(a);
            let b_true = self.is_truthy(b);
            let either = self.builder.ins().bor(a_true, b_true);
            let result = self.builder.ins().select(either, true_val, false_val);
            self.stack_push(result);
        }
        Ok(())
    }

    pub(crate) fn compile_not(&mut self) -> Result<(), String> {
        if let Some(a) = self.stack_pop() {
            let true_val = self.builder.ins().iconst(types::I64, TAG_BOOL_TRUE as i64);
            let false_val = self.builder.ins().iconst(types::I64, TAG_BOOL_FALSE as i64);
            let is_true = self.is_truthy(a);
            let result = self.builder.ins().select(is_true, false_val, true_val);
            self.stack_push(result);
        }
        Ok(())
    }

    // ========================================================================
    // Typed arithmetic — compiler-guaranteed types, no runtime dispatch
    // ========================================================================

    fn width_integer_binary_op<F>(&mut self, result_hint: StorageHint, op: F)
    where
        F: FnOnce(&mut FunctionBuilder, Value, Value) -> Value,
    {
        if self.stack_len() < 2 {
            return;
        }
        let rhs_hint = self.get_stack_type_at(0);
        let lhs_hint = self.get_stack_type_at(1);
        let (rhs_raw, lhs_raw) = self.typed_stack.top_two_i64_flags();

        let rhs_val = self.stack_pop().unwrap();
        let lhs_val = self.stack_pop().unwrap();

        let lhs_i64 = if lhs_raw {
            self.normalize_i64_to_hint(lhs_val, lhs_hint)
        } else {
            self.boxed_to_i64_for_hint(lhs_val, lhs_hint)
        };
        let rhs_i64 = if rhs_raw {
            self.normalize_i64_to_hint(rhs_val, rhs_hint)
        } else {
            self.boxed_to_i64_for_hint(rhs_val, rhs_hint)
        };

        let Some((int_ty, signed)) = Self::integer_clif_type_and_signed(result_hint) else {
            return;
        };
        let lhs_typed = if int_ty == types::I64 {
            lhs_i64
        } else {
            self.builder.ins().ireduce(int_ty, lhs_i64)
        };
        let rhs_typed = if int_ty == types::I64 {
            rhs_i64
        } else {
            self.builder.ins().ireduce(int_ty, rhs_i64)
        };
        let result_typed = op(self.builder, lhs_typed, rhs_typed);
        let widened = if int_ty == types::I64 {
            result_typed
        } else if signed {
            self.builder.ins().sextend(types::I64, result_typed)
        } else {
            self.builder.ins().uextend(types::I64, result_typed)
        };
        let normalized = self.normalize_i64_to_hint(widened, result_hint);
        self.stack_push(normalized);
        self.typed_stack
            .replace_top(crate::translator::storage::TypedValue::i64(normalized));
        self.propagate_result_type(result_hint);
    }

    fn width_integer_comparison(&mut self, hint: StorageHint, cc: IntCC) {
        if self.stack_len() < 2 {
            return;
        }
        let rhs_hint = self.get_stack_type_at(0);
        let lhs_hint = self.get_stack_type_at(1);
        let (rhs_raw, lhs_raw) = self.typed_stack.top_two_i64_flags();

        let rhs_val = self.stack_pop().unwrap();
        let lhs_val = self.stack_pop().unwrap();

        let lhs_i64 = if lhs_raw {
            self.normalize_i64_to_hint(lhs_val, lhs_hint)
        } else {
            self.boxed_to_i64_for_hint(lhs_val, lhs_hint)
        };
        let rhs_i64 = if rhs_raw {
            self.normalize_i64_to_hint(rhs_val, rhs_hint)
        } else {
            self.boxed_to_i64_for_hint(rhs_val, rhs_hint)
        };

        let Some((int_ty, _)) = Self::integer_clif_type_and_signed(hint) else {
            return;
        };
        let lhs_typed = if int_ty == types::I64 {
            lhs_i64
        } else {
            self.builder.ins().ireduce(int_ty, lhs_i64)
        };
        let rhs_typed = if int_ty == types::I64 {
            rhs_i64
        } else {
            self.builder.ins().ireduce(int_ty, rhs_i64)
        };

        let cmp = self
            .builder
            .ins()
            .icmp(Self::intcc_for_hint(cc, hint), lhs_typed, rhs_typed);
        self.push_cmp_bool_result(cmp);
    }

    /// Typed integer arithmetic — uses f64 operations since integers are
    /// stored as exact f64 in NaN-boxing. Compiler guarantees both operands
    /// are integers, so no type checks needed. Uses f64 cache from typed_stack
    /// when available to skip input bitcasts.
    ///
    /// When inside an integer-unboxed loop (unboxed_int_locals non-empty) AND
    /// both operands are raw i64, uses native integer ops (iadd, isub, imul)
    /// for ~3x lower latency than the f64 path.
    pub(crate) fn compile_int_arith(&mut self, op: OpCode) -> Result<(), String> {
        // Map trusted variants to their guarded equivalents — the JIT generates
        // the same code either way since it operates on typed IR.
        let op = match op {
            OpCode::AddIntTrusted => OpCode::AddInt,
            OpCode::SubIntTrusted => OpCode::SubInt,
            OpCode::MulIntTrusted => OpCode::MulInt,
            OpCode::DivIntTrusted => OpCode::DivInt,
            _ => op,
        };
        let width_hint = self
            .top_two_integer_result_hint()
            .filter(|hint| hint.is_integer_family() && !hint.is_default_int_family())
            .map(|hint| hint.non_nullable());
        if let Some(result_hint) = width_hint {
            match op {
                OpCode::AddInt => {
                    self.width_integer_binary_op(result_hint, |b, a, c| b.ins().iadd(a, c));
                }
                OpCode::SubInt => {
                    self.width_integer_binary_op(result_hint, |b, a, c| b.ins().isub(a, c));
                }
                OpCode::MulInt => {
                    self.width_integer_binary_op(result_hint, |b, a, c| b.ins().imul(a, c));
                }
                OpCode::DivInt => {
                    let signed = result_hint.is_signed_integer().unwrap_or(true);
                    if signed {
                        self.width_integer_binary_op(result_hint, |b, a, c| b.ins().sdiv(a, c));
                    } else {
                        self.width_integer_binary_op(result_hint, |b, a, c| b.ins().udiv(a, c));
                    }
                }
                OpCode::ModInt => {
                    let signed = result_hint.is_signed_integer().unwrap_or(true);
                    if signed {
                        self.width_integer_binary_op(result_hint, |b, a, c| b.ins().srem(a, c));
                    } else {
                        self.width_integer_binary_op(result_hint, |b, a, c| b.ins().urem(a, c));
                    }
                }
                OpCode::PowInt => {
                    // Pow uses FFI path for now.
                    if self.stack_len() >= 2 {
                        let exp = self.stack_pop_boxed().unwrap();
                        let base = self.stack_pop_boxed().unwrap();
                        let inst = self.builder.ins().call(self.ffi.pow, &[base, exp]);
                        let result = self.builder.inst_results(inst)[0];
                        self.stack_push_typed(result, StorageHint::Float64);
                    }
                }
                _ => {}
            }
            return Ok(());
        }

        // Fast path: native i64 arithmetic inside unboxed integer loops
        if self.typed_stack.either_top_i64() {
            let use_raw = self.typed_stack.both_top_i64();
            macro_rules! int_op {
                ($op_fn:ident) => {
                    if use_raw {
                        self.raw_int64_binary_op(|b, a, c| b.ins().$op_fn(a, c));
                    } else {
                        self.mixed_int64_binary_op(|b, a, c| b.ins().$op_fn(a, c));
                    }
                    self.propagate_result_type(StorageHint::Int64);
                };
            }
            match op {
                OpCode::AddInt => {
                    int_op!(iadd);
                }
                OpCode::SubInt => {
                    int_op!(isub);
                }
                OpCode::MulInt => {
                    int_op!(imul);
                }
                OpCode::DivInt => {
                    int_op!(sdiv);
                }
                OpCode::ModInt => {
                    int_op!(srem);
                }
                OpCode::PowInt => {
                    // Pow requires FFI — rebox operands for the call
                    if self.stack_len() >= 2 {
                        let exp_raw = self.stack_pop().unwrap();
                        let base_raw = self.stack_pop().unwrap();
                        let exp = self.box_from_int(exp_raw);
                        let base = self.box_from_int(base_raw);
                        let inst = self.builder.ins().call(self.ffi.pow, &[base, exp]);
                        let result = self.builder.inst_results(inst)[0];
                        // Pow returns NaN-boxed; convert back to raw i64
                        let f64_val = self.i64_to_f64(result);
                        let raw = self.builder.ins().fcvt_to_sint_sat(types::I64, f64_val);
                        self.stack_push(raw);
                        self.typed_stack
                            .replace_top(crate::translator::storage::TypedValue::i64(raw));
                    }
                }
                _ => {}
            }
            return Ok(());
        }

        // Standard path: NaN-boxed f64 arithmetic
        match op {
            OpCode::AddInt => {
                self.nullable_float64_binary_op(|b, a, c| b.ins().fadd(a, c));
                self.propagate_result_type(StorageHint::Int64);
            }
            OpCode::SubInt => {
                self.nullable_float64_binary_op(|b, a, c| b.ins().fsub(a, c));
                self.propagate_result_type(StorageHint::Int64);
            }
            OpCode::MulInt => {
                self.nullable_float64_binary_op(|b, a, c| b.ins().fmul(a, c));
                self.propagate_result_type(StorageHint::Int64);
            }
            OpCode::DivInt => {
                // int / int -> int (truncated toward zero), matching VM semantics.
                self.nullable_float64_binary_op(|b, a, c| {
                    let div = b.ins().fdiv(a, c);
                    b.ins().trunc(div)
                });
                self.propagate_result_type(StorageHint::Int64);
            }
            OpCode::ModInt => {
                self.nullable_float64_binary_op(|b, a, c| {
                    let div = b.ins().fdiv(a, c);
                    let trunced = b.ins().trunc(div);
                    let product = b.ins().fmul(trunced, c);
                    b.ins().fsub(a, product)
                });
                self.propagate_result_type(StorageHint::Int64);
            }
            OpCode::PowInt => {
                if self.stack_len() >= 2 {
                    let exp = self.stack_pop().unwrap();
                    let base = self.stack_pop().unwrap();
                    let inst = self.builder.ins().call(self.ffi.pow, &[base, exp]);
                    let result = self.builder.inst_results(inst)[0];
                    self.stack_push_typed(result, StorageHint::Float64);
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Typed float arithmetic — same as int in NaN-boxing (both are f64).
    /// Compiler guarantees both operands are numbers — unconditionally use fast path.
    pub(crate) fn compile_float_arith(&mut self, op: OpCode) -> Result<(), String> {
        // Map trusted variants to their guarded equivalents — the JIT generates
        // the same code either way since it operates on typed IR.
        let op = match op {
            OpCode::AddNumberTrusted => OpCode::AddNumber,
            OpCode::SubNumberTrusted => OpCode::SubNumber,
            OpCode::MulNumberTrusted => OpCode::MulNumber,
            OpCode::DivNumberTrusted => OpCode::DivNumber,
            _ => op,
        };
        let result_hint = StorageHint::Float64;

        match op {
            OpCode::AddNumber => {
                self.nullable_float64_binary_op(|b, a, c| b.ins().fadd(a, c));
                self.propagate_result_type(result_hint);
            }
            OpCode::SubNumber => {
                self.nullable_float64_binary_op(|b, a, c| b.ins().fsub(a, c));
                self.propagate_result_type(result_hint);
            }
            OpCode::MulNumber => {
                self.nullable_float64_binary_op(|b, a, c| b.ins().fmul(a, c));
                self.propagate_result_type(result_hint);
            }
            OpCode::DivNumber => {
                // Strength reduction: fdiv by exact-reciprocal constant → fmul
                if let Some(recip) = self.div_const_reciprocal_from_stack() {
                    let _ = self.stack_pop_f64();
                    let recip_f64 = self.builder.ins().f64const(recip);
                    let recip_boxed = self.f64_to_i64(recip_f64);
                    self.stack_push(recip_boxed);
                    self.typed_stack
                        .replace_top(crate::translator::storage::TypedValue::f64(recip_f64));
                    self.nullable_float64_binary_op(|b, a, c| b.ins().fmul(a, c));
                } else {
                    self.nullable_float64_binary_op(|b, a, c| b.ins().fdiv(a, c));
                }
                self.propagate_result_type(result_hint);
            }
            OpCode::ModNumber => {
                self.nullable_float64_binary_op(|b, a, c| {
                    let div = b.ins().fdiv(a, c);
                    let trunced = b.ins().trunc(div);
                    let product = b.ins().fmul(trunced, c);
                    b.ins().fsub(a, product)
                });
                self.propagate_result_type(result_hint);
            }
            OpCode::PowNumber => {
                if self.stack_len() >= 2 {
                    let exp = self.stack_pop().unwrap();
                    let base = self.stack_pop().unwrap();
                    let inst = self.builder.ins().call(self.ffi.pow, &[base, exp]);
                    let result = self.builder.inst_results(inst)[0];
                    self.stack_push_typed(result, result_hint);
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Typed decimal arithmetic — decimals are not JIT-compiled yet, fall through.
    pub(crate) fn compile_decimal_arith(&mut self, _op: OpCode) -> Result<(), String> {
        // Decimal types are not yet supported in JIT — handled by interpreter fallback
        Ok(())
    }

    // ========================================================================
    // Typed comparisons — compiler-guaranteed types, no runtime dispatch
    // ========================================================================

    /// Typed integer comparison — uses f64 fcmp since integers are
    /// stored as exact f64 in NaN-boxing. Uses f64 cache from typed_stack
    /// when available to skip input bitcasts.
    ///
    /// When inside an integer-unboxed loop with both operands as raw i64,
    /// uses native icmp for ~3x lower latency than fcmp.
    pub(crate) fn compile_int_cmp(&mut self, op: OpCode) -> Result<(), String> {
        // Map trusted variants to their guarded equivalents — the JIT generates
        // the same code either way since it operates on typed IR.
        let op = match op {
            OpCode::GtIntTrusted => OpCode::GtInt,
            OpCode::LtIntTrusted => OpCode::LtInt,
            OpCode::GteIntTrusted => OpCode::GteInt,
            OpCode::LteIntTrusted => OpCode::LteInt,
            _ => op,
        };
        let width_hint = self
            .top_two_integer_result_hint()
            .filter(|hint| hint.is_integer_family() && !hint.is_default_int_family())
            .map(|hint| hint.non_nullable());
        if let Some(hint) = width_hint {
            let cc = match op {
                OpCode::GtInt => IntCC::SignedGreaterThan,
                OpCode::LtInt => IntCC::SignedLessThan,
                OpCode::GteInt => IntCC::SignedGreaterThanOrEqual,
                OpCode::LteInt => IntCC::SignedLessThanOrEqual,
                OpCode::EqInt => IntCC::Equal,
                OpCode::NeqInt => IntCC::NotEqual,
                _ => return Ok(()),
            };
            self.width_integer_comparison(hint, cc);
            return Ok(());
        }

        // Fast path: native i64 comparison inside unboxed integer loops
        if self.typed_stack.either_top_i64() {
            let cc = match op {
                OpCode::GtInt => IntCC::SignedGreaterThan,
                OpCode::LtInt => IntCC::SignedLessThan,
                OpCode::GteInt => IntCC::SignedGreaterThanOrEqual,
                OpCode::LteInt => IntCC::SignedLessThanOrEqual,
                OpCode::EqInt => IntCC::Equal,
                OpCode::NeqInt => IntCC::NotEqual,
                _ => return Ok(()),
            };
            if self.typed_stack.both_top_i64() {
                self.raw_int64_comparison(cc);
            } else {
                self.mixed_int64_comparison(cc);
            }
            return Ok(());
        }

        // Standard path: NaN-boxed f64 comparison
        let cc = match op {
            OpCode::GtInt => FloatCC::GreaterThan,
            OpCode::LtInt => FloatCC::LessThan,
            OpCode::GteInt => FloatCC::GreaterThanOrEqual,
            OpCode::LteInt => FloatCC::LessThanOrEqual,
            OpCode::EqInt => FloatCC::Equal,
            OpCode::NeqInt => FloatCC::NotEqual,
            _ => return Ok(()),
        };
        self.typed_comparison(cc);
        Ok(())
    }

    /// Typed float comparison — direct f64 comparison, no type checks.
    pub(crate) fn compile_float_cmp(&mut self, op: OpCode) -> Result<(), String> {
        // Map trusted variants to their guarded equivalents — the JIT generates
        // the same code either way since it operates on typed IR.
        let op = match op {
            OpCode::GtNumberTrusted => OpCode::GtNumber,
            OpCode::LtNumberTrusted => OpCode::LtNumber,
            OpCode::GteNumberTrusted => OpCode::GteNumber,
            OpCode::LteNumberTrusted => OpCode::LteNumber,
            _ => op,
        };
        let cc = match op {
            OpCode::GtNumber => FloatCC::GreaterThan,
            OpCode::LtNumber => FloatCC::LessThan,
            OpCode::GteNumber => FloatCC::GreaterThanOrEqual,
            OpCode::LteNumber => FloatCC::LessThanOrEqual,
            OpCode::EqNumber => FloatCC::Equal,
            OpCode::NeqNumber => FloatCC::NotEqual,
            _ => return Ok(()),
        };
        self.typed_comparison(cc);
        Ok(())
    }

    /// Typed decimal comparison — not JIT-compiled yet.
    pub(crate) fn compile_decimal_cmp(&mut self, _op: OpCode) -> Result<(), String> {
        Ok(())
    }

    /// Direct f64 comparison without type checking — used by typed comparison opcodes.
    /// Uses typed_stack f64 shadows when available to skip input bitcasts.
    /// When the next instruction is a conditional jump, skips boolean materialization
    /// and caches the raw i1 fcmp for a fused comparison-branch.
    fn typed_comparison(&mut self, cc: FloatCC) {
        if self.stack_len() >= 2 {
            let b_f64 = self.stack_pop_f64().unwrap();
            let a_f64 = self.stack_pop_f64().unwrap();
            let cmp = self.builder.ins().fcmp(cc, a_f64, b_f64);
            self.push_cmp_bool_result(cmp);
        }
    }

    // ========================================================================
    // Bitwise operations — convert f64→i64, perform bitwise op, convert back
    // ========================================================================

    /// Helper: convert a NaN-boxed f64 value to an integer for bitwise ops.
    /// Unboxes f64, then truncates to i64 via fcvt_to_sint.
    fn unbox_to_int(&mut self, boxed: Value) -> Value {
        let f64_val = self.i64_to_f64(boxed);
        self.builder.ins().fcvt_to_sint_sat(types::I64, f64_val)
    }

    /// Helper: convert an integer result back to NaN-boxed f64.
    /// Converts i64 to f64 via fcvt_from_sint, then boxes as NaN-boxed i64.
    fn box_from_int(&mut self, int_val: Value) -> Value {
        let f64_val = self.builder.ins().fcvt_from_sint(types::F64, int_val);
        self.f64_to_i64(f64_val)
    }

    pub(crate) fn compile_bitwise_binary(&mut self, op: OpCode) -> Result<(), String> {
        if self.stack_len() >= 2 {
            let b_boxed = self.stack_pop().unwrap();
            let a_boxed = self.stack_pop().unwrap();
            let a_int = self.unbox_to_int(a_boxed);
            let b_int = self.unbox_to_int(b_boxed);
            let result_int = match op {
                OpCode::BitAnd => self.builder.ins().band(a_int, b_int),
                OpCode::BitOr => self.builder.ins().bor(a_int, b_int),
                OpCode::BitXor => self.builder.ins().bxor(a_int, b_int),
                OpCode::BitShl => self.builder.ins().ishl(a_int, b_int),
                OpCode::BitShr => self.builder.ins().sshr(a_int, b_int),
                _ => return Ok(()),
            };
            let result_boxed = self.box_from_int(result_int);
            self.stack_push(result_boxed);
        }
        Ok(())
    }

    pub(crate) fn compile_bit_not(&mut self) -> Result<(), String> {
        if let Some(a_boxed) = self.stack_pop() {
            let a_int = self.unbox_to_int(a_boxed);
            let result_int = self.builder.ins().bnot(a_int);
            let result_boxed = self.box_from_int(result_int);
            self.stack_push(result_boxed);
        }
        Ok(())
    }

    // ========================================================================
    // Type coercion — IntToNumber / NumberToInt
    // ========================================================================

    /// IntToNumber: In NaN-boxing, ints are already stored as f64, so this is
    /// effectively a no-op for boxed values. For raw i64 values from unboxed
    /// integer loops, convert to f64 and re-box so downstream float ops see
    /// canonical NaN-boxed numbers.
    ///
    /// Peephole: when the source is a loop-invariant int local with a
    /// precomputed f64, use the cached value instead of re-emitting
    /// fcvt_from_sint every iteration.
    pub(crate) fn compile_int_to_number(&mut self) -> Result<(), String> {
        use crate::translator::storage::CraneliftRepr;
        use shape_vm::bytecode::{OpCode, Operand};

        // LICM peephole: check if the preceding LoadLocal feeds a precomputed f64.
        if !self.precomputed_f64_for_invariant_int.is_empty() {
            let prev_idx = self.current_instr_idx.wrapping_sub(1);
            if prev_idx < self.program.instructions.len() {
                let prev_instr = &self.program.instructions[prev_idx];
                if matches!(
                    prev_instr.opcode,
                    OpCode::LoadLocal | OpCode::LoadLocalTrusted
                ) {
                    if let Some(Operand::Local(local_idx)) = &prev_instr.operand {
                        if let Some(&f64_var) =
                            self.precomputed_f64_for_invariant_int.get(local_idx)
                        {
                            // Use precomputed f64 instead of re-converting
                            let _ = self.stack_pop();
                            let f64_val = self.builder.use_var(f64_var);
                            let boxed = self.f64_to_i64(f64_val);
                            self.stack_push(boxed);
                            self.typed_stack.replace_top(
                                crate::translator::storage::TypedValue::f64(f64_val),
                            );
                            return Ok(());
                        }
                    }
                }
            }
        }

        let top_repr = self.typed_stack.peek().map(|tv| tv.repr);
        if top_repr == Some(CraneliftRepr::I64) {
            if let Some(raw_i64) = self.stack_pop() {
                let as_f64 = self.builder.ins().fcvt_from_sint(types::F64, raw_i64);
                let boxed = self.f64_to_i64(as_f64);
                self.stack_push(boxed);
                self.typed_stack
                    .replace_top(crate::translator::storage::TypedValue::f64(as_f64));
            }
        }
        Ok(())
    }

    /// NumberToInt: Truncate f64 to integer, then convert back to f64.
    /// This strips the fractional part (e.g., 3.7 -> 3.0, -2.3 -> -2.0).
    pub(crate) fn compile_number_to_int(&mut self) -> Result<(), String> {
        use crate::translator::storage::CraneliftRepr;

        let top_repr = self.typed_stack.peek().map(|tv| tv.repr);
        if top_repr == Some(CraneliftRepr::I64) {
            // Already an integer in raw i64 form.
            return Ok(());
        }

        if let Some(f64_val) = self.stack_pop_f64() {
            let truncated = self.builder.ins().trunc(f64_val);
            let result_boxed = self.f64_to_i64(truncated);
            self.stack_push(result_boxed);
            self.typed_stack
                .replace_top(crate::translator::storage::TypedValue::f64(truncated));
        }
        Ok(())
    }

    // ========================================================================
    // Compact typed opcodes (AddTyped, SubTyped, etc.)
    // ========================================================================

    /// Compile a compact typed arithmetic opcode by extracting the NumericWidth
    /// operand and dispatching to the appropriate int/float arithmetic path.
    pub(crate) fn compile_typed_arith(&mut self, instr: &Instruction) -> Result<(), String> {
        let width = match &instr.operand {
            Some(Operand::Width(w)) => *w,
            _ => return Err(format!("{:?} requires Width operand", instr.opcode)),
        };

        let equiv_op = match instr.opcode {
            OpCode::AddTyped => {
                if width.is_integer() {
                    OpCode::AddInt
                } else {
                    OpCode::AddNumber
                }
            }
            OpCode::SubTyped => {
                if width.is_integer() {
                    OpCode::SubInt
                } else {
                    OpCode::SubNumber
                }
            }
            OpCode::MulTyped => {
                if width.is_integer() {
                    OpCode::MulInt
                } else {
                    OpCode::MulNumber
                }
            }
            OpCode::DivTyped => {
                if width.is_integer() {
                    OpCode::DivInt
                } else {
                    OpCode::DivNumber
                }
            }
            OpCode::ModTyped => {
                if width.is_integer() {
                    OpCode::ModInt
                } else {
                    OpCode::ModNumber
                }
            }
            OpCode::CmpTyped => {
                if width.is_integer() {
                    OpCode::GtInt
                } else {
                    OpCode::GtNumber
                }
            }
            _ => return Ok(()),
        };

        // CmpTyped has special 3-way result semantics (-1, 0, 1).
        // Implement as two comparisons: first gt, then eq.
        if instr.opcode == OpCode::CmpTyped {
            return self.compile_typed_cmp(width);
        }

        if width.is_integer() {
            self.compile_int_arith(equiv_op)
        } else {
            debug_assert!(width.is_float(), "unsupported NumericWidth: {:?}", width);
            self.compile_float_arith(equiv_op)
        }
    }

    /// Compile CastWidth: truncate TOS to the declared integer width via bitmask.
    pub(crate) fn compile_cast_width(&mut self, instr: &Instruction) -> Result<(), String> {
        let width = match &instr.operand {
            Some(Operand::Width(w)) => *w,
            _ => return Err("CastWidth requires Width operand".to_string()),
        };

        if self.stack_len() < 1 {
            return Ok(());
        }

        if let Some(int_width) = width.to_int_width() {
            // Integer width: pop value, mask to target width
            let val = self.stack_pop().unwrap();
            let mask = int_width.mask() as i64;
            let mask_val = self.builder.ins().iconst(types::I64, mask);
            let truncated = self.builder.ins().band(val, mask_val);

            // For signed widths, sign-extend from the bit width
            let result = if int_width.is_signed() {
                let bits = int_width.bits() as i64;
                let shift = 64 - bits;
                let shift_val = self.builder.ins().iconst(types::I64, shift);
                let shifted_left = self.builder.ins().ishl(truncated, shift_val);
                self.builder.ins().sshr(shifted_left, shift_val)
            } else {
                truncated
            };

            self.stack_push(result);
        } else {
            // Float width — no-op for cast (f32 truncation could go here later)
        }
        Ok(())
    }

    /// Compile CmpTyped: 3-way comparison returning -1, 0, or 1.
    fn compile_typed_cmp(&mut self, width: NumericWidth) -> Result<(), String> {
        if self.stack_len() < 2 {
            return Ok(());
        }

        if width.is_integer() {
            // Integer comparison path: use icmp with signed/unsigned CC
            let rhs_val = self.stack_pop().unwrap();
            let lhs_val = self.stack_pop().unwrap();

            // Choose signed vs unsigned comparison
            let (gt_cc, eq_cc) = if width.is_signed() {
                (IntCC::SignedGreaterThan, IntCC::Equal)
            } else {
                (IntCC::UnsignedGreaterThan, IntCC::Equal)
            };

            let is_gt = self.builder.ins().icmp(gt_cc, lhs_val, rhs_val);
            let is_eq = self.builder.ins().icmp(eq_cc, lhs_val, rhs_val);

            // result = is_gt ? 1 : (is_eq ? 0 : -1), then box as NaN-boxed f64
            let neg_one = self.builder.ins().f64const(-1.0);
            let zero = self.builder.ins().f64const(0.0);
            let one = self.builder.ins().f64const(1.0);

            let eq_or_lt = self.builder.ins().select(is_eq, zero, neg_one);
            let result_f64 = self.builder.ins().select(is_gt, one, eq_or_lt);

            let result_bits = self
                .builder
                .ins()
                .bitcast(types::I64, MemFlags::new(), result_f64);
            self.stack_push(result_bits);
        } else {
            // Float comparison path (existing)
            let rhs_f64 = self.stack_pop_f64().unwrap();
            let lhs_f64 = self.stack_pop_f64().unwrap();

            let is_gt = self
                .builder
                .ins()
                .fcmp(FloatCC::GreaterThan, lhs_f64, rhs_f64);
            let is_eq = self.builder.ins().fcmp(FloatCC::Equal, lhs_f64, rhs_f64);

            let neg_one = self.builder.ins().f64const(-1.0);
            let zero = self.builder.ins().f64const(0.0);
            let one = self.builder.ins().f64const(1.0);

            let eq_or_lt = self.builder.ins().select(is_eq, zero, neg_one);
            let result_f64 = self.builder.ins().select(is_gt, one, eq_or_lt);

            let result_bits = self
                .builder
                .ins()
                .bitcast(types::I64, MemFlags::new(), result_f64);
            self.stack_push(result_bits);
        }
        Ok(())
    }
}
