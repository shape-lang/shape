//! Arithmetic operations for the VM executor
//!
//! Handles: Add, Sub, Mul, Div, Mod, Neg, Pow

use crate::{
    bytecode::{Instruction, NumericWidth, OpCode, Operand},
    executor::VirtualMachine,
};
use shape_ast::IntWidth;
use shape_value::heap_value::HeapValue;
use shape_value::{VMError, ValueWord};
use std::sync::Arc;

const EXACT_F64_INT_LIMIT: i128 = 9_007_199_254_740_992;

/// Check if an i64 result fits in the I48 inline range.
/// Values outside this range would be heap-boxed as BigInt, so we promote to f64 instead.
#[inline(always)]
fn fits_i48(v: i64) -> bool {
    v >= shape_value::tags::I48_MIN && v <= shape_value::tags::I48_MAX
}

#[derive(Clone, Copy)]
enum NumericDomain {
    Int(i128),
    Float(f64),
}

/// Unwrap TypeAnnotatedValue wrapper to get the inner value.
/// This is needed because `: number` annotations wrap values, and the
/// Heap tag doesn't match any arithmetic dispatch case.
#[inline(always)]
fn unwrap_annotated(nb: ValueWord) -> ValueWord {
    if let Some(HeapValue::TypeAnnotatedValue { value, .. }) = nb.as_heap_ref() {
        value.as_ref().clone()
    } else {
        nb
    }
}

macro_rules! define_compact_width_dispatch {
    ($( $fn_name:ident => $int_handler:ident, $float_handler:ident; )+) => {
        $(
            #[inline(always)]
            fn $fn_name(
                &mut self,
                width: crate::bytecode::NumericWidth,
            ) -> Result<(), VMError> {
                if width.is_integer() {
                    self.$int_handler(width)
                } else {
                    debug_assert!(width.is_float(), "unsupported NumericWidth: {:?}", width);
                    self.$float_handler()
                }
            }
        )+
    };
}

impl VirtualMachine {
    #[inline(always)]
    fn number_operand(nb: &ValueWord) -> Option<f64> {
        nb.as_number_strict()
    }

    /// Coerce a ValueWord to i64 for typed int opcodes.
    /// Accepts true i48 ints, native u64/i64 scalars, and f64 values that are
    /// exact whole numbers (handles compiler producing f64 constants for
    /// integer-looking literals).
    #[inline(always)]
    pub(in crate::executor) fn int_operand(nb: &ValueWord) -> Option<i64> {
        if let Some(i) = nb.as_i64() {
            return Some(i);
        }
        // Native u64 scalars (e.g. u64::MAX): reinterpret bits as i64 for
        // truncation to work correctly (all-ones pattern → -1 as i8).
        if let Some(u) = nb.as_u64() {
            return Some(u as i64);
        }
        // f64 whole-number coercion (e.g. array elements compiled as Number(1.0))
        if let Some(f) = nb.as_f64() {
            if f.is_finite() && f == f.trunc() && f.abs() < (i64::MAX as f64) {
                return Some(f as i64);
            }
        }
        None
    }

    #[inline(always)]
    fn arith_i128_to_lossless_f64(value: i128) -> Option<f64> {
        if (-EXACT_F64_INT_LIMIT..=EXACT_F64_INT_LIMIT).contains(&value) {
            Some(value as f64)
        } else {
            None
        }
    }

    #[inline(always)]
    fn integer_result_boxed(value: i128, op_name: &str) -> Result<ValueWord, VMError> {
        if (i64::MIN as i128..=i64::MAX as i128).contains(&value) {
            return Ok(ValueWord::from_i64(value as i64));
        }
        if (0..=u64::MAX as i128).contains(&value) {
            return Ok(ValueWord::from_native_u64(value as u64));
        }
        Err(VMError::RuntimeError(format!(
            "Integer overflow in '{}'",
            op_name
        )))
    }

    #[inline(always)]
    fn numeric_domain(nb: &ValueWord) -> Option<NumericDomain> {
        if let Some(i) = nb.as_i128_exact() {
            return Some(NumericDomain::Int(i));
        }
        nb.as_number_strict().map(NumericDomain::Float)
    }

    #[inline(always)]
    fn numeric_binary_result(
        a: &ValueWord,
        b: &ValueWord,
        op_name: &str,
        int_op: impl FnOnce(i128, i128) -> Option<i128>,
        float_op: impl FnOnce(f64, f64) -> f64,
    ) -> Result<Option<ValueWord>, VMError> {
        let a_num = match Self::numeric_domain(a) {
            Some(v) => v,
            None => return Ok(None),
        };
        let b_num = match Self::numeric_domain(b) {
            Some(v) => v,
            None => return Ok(None),
        };
        match (a_num, b_num) {
            (NumericDomain::Int(ai), NumericDomain::Int(bi)) => int_op(ai, bi)
                .ok_or_else(|| VMError::RuntimeError(format!("Integer overflow in '{}'", op_name)))
                .and_then(|v| Self::integer_result_boxed(v, op_name))
                .map(Some),
            (NumericDomain::Float(af), NumericDomain::Float(bf)) => {
                Ok(Some(ValueWord::from_f64(float_op(af, bf))))
            }
            (NumericDomain::Int(ai), NumericDomain::Float(bf)) => {
                let af = Self::arith_i128_to_lossless_f64(ai).ok_or_else(|| {
                    VMError::RuntimeError(format!(
                        "Cannot apply '{}' without explicit cast: {} is not losslessly representable as number",
                        op_name, ai
                    ))
                })?;
                Ok(Some(ValueWord::from_f64(float_op(af, bf))))
            }
            (NumericDomain::Float(af), NumericDomain::Int(bi)) => {
                let bf = Self::arith_i128_to_lossless_f64(bi).ok_or_else(|| {
                    VMError::RuntimeError(format!(
                        "Cannot apply '{}' without explicit cast: {} is not losslessly representable as number",
                        op_name, bi
                    ))
                })?;
                Ok(Some(ValueWord::from_f64(float_op(af, bf))))
            }
        }
    }

    #[inline(always)]
    fn numeric_div_result(a: &ValueWord, b: &ValueWord) -> Result<Option<ValueWord>, VMError> {
        let a_num = match Self::numeric_domain(a) {
            Some(v) => v,
            None => return Ok(None),
        };
        let b_num = match Self::numeric_domain(b) {
            Some(v) => v,
            None => return Ok(None),
        };
        match (a_num, b_num) {
            (NumericDomain::Int(ai), NumericDomain::Int(bi)) => {
                if bi == 0 {
                    return Err(VMError::DivisionByZero);
                }
                let out = ai
                    .checked_div(bi)
                    .ok_or_else(|| VMError::RuntimeError("Integer overflow in '/'".into()))?;
                Self::integer_result_boxed(out, "/").map(Some)
            }
            (NumericDomain::Float(af), NumericDomain::Float(bf)) => {
                if bf == 0.0 {
                    return Err(VMError::DivisionByZero);
                }
                Ok(Some(ValueWord::from_f64(af / bf)))
            }
            (NumericDomain::Int(ai), NumericDomain::Float(bf)) => {
                if bf == 0.0 {
                    return Err(VMError::DivisionByZero);
                }
                let af = Self::arith_i128_to_lossless_f64(ai).ok_or_else(|| {
                    VMError::RuntimeError(format!(
                        "Cannot apply '/' without explicit cast: {} is not losslessly representable as number",
                        ai
                    ))
                })?;
                Ok(Some(ValueWord::from_f64(af / bf)))
            }
            (NumericDomain::Float(af), NumericDomain::Int(bi)) => {
                let bf = Self::arith_i128_to_lossless_f64(bi).ok_or_else(|| {
                    VMError::RuntimeError(format!(
                        "Cannot apply '/' without explicit cast: {} is not losslessly representable as number",
                        bi
                    ))
                })?;
                if bf == 0.0 {
                    return Err(VMError::DivisionByZero);
                }
                Ok(Some(ValueWord::from_f64(af / bf)))
            }
        }
    }

    #[inline(always)]
    fn numeric_mod_result(a: &ValueWord, b: &ValueWord) -> Result<Option<ValueWord>, VMError> {
        let a_num = match Self::numeric_domain(a) {
            Some(v) => v,
            None => return Ok(None),
        };
        let b_num = match Self::numeric_domain(b) {
            Some(v) => v,
            None => return Ok(None),
        };
        match (a_num, b_num) {
            (NumericDomain::Int(ai), NumericDomain::Int(bi)) => {
                if bi == 0 {
                    return Err(VMError::DivisionByZero);
                }
                let out = ai
                    .checked_rem(bi)
                    .ok_or_else(|| VMError::RuntimeError("Integer overflow in '%'".into()))?;
                Self::integer_result_boxed(out, "%").map(Some)
            }
            (NumericDomain::Float(af), NumericDomain::Float(bf)) => {
                if bf == 0.0 {
                    return Err(VMError::DivisionByZero);
                }
                Ok(Some(ValueWord::from_f64(af % bf)))
            }
            (NumericDomain::Int(ai), NumericDomain::Float(bf)) => {
                if bf == 0.0 {
                    return Err(VMError::DivisionByZero);
                }
                let af = Self::arith_i128_to_lossless_f64(ai).ok_or_else(|| {
                    VMError::RuntimeError(format!(
                        "Cannot apply '%' without explicit cast: {} is not losslessly representable as number",
                        ai
                    ))
                })?;
                Ok(Some(ValueWord::from_f64(af % bf)))
            }
            (NumericDomain::Float(af), NumericDomain::Int(bi)) => {
                let bf = Self::arith_i128_to_lossless_f64(bi).ok_or_else(|| {
                    VMError::RuntimeError(format!(
                        "Cannot apply '%' without explicit cast: {} is not losslessly representable as number",
                        bi
                    ))
                })?;
                if bf == 0.0 {
                    return Err(VMError::DivisionByZero);
                }
                Ok(Some(ValueWord::from_f64(af % bf)))
            }
        }
    }

    #[inline(always)]
    fn checked_pow_i128(mut base: i128, mut exp: u32) -> Option<i128> {
        let mut out: i128 = 1;
        while exp > 0 {
            if (exp & 1) == 1 {
                out = out.checked_mul(base)?;
            }
            exp >>= 1;
            if exp > 0 {
                base = base.checked_mul(base)?;
            }
        }
        Some(out)
    }

    #[inline(always)]
    fn numeric_pow_result(a: &ValueWord, b: &ValueWord) -> Result<Option<ValueWord>, VMError> {
        let a_num = match Self::numeric_domain(a) {
            Some(v) => v,
            None => return Ok(None),
        };
        let b_num = match Self::numeric_domain(b) {
            Some(v) => v,
            None => return Ok(None),
        };
        match (a_num, b_num) {
            (NumericDomain::Int(base), NumericDomain::Int(exp)) => {
                if exp >= 0 && exp <= u32::MAX as i128 {
                    let out = Self::checked_pow_i128(base, exp as u32)
                        .ok_or_else(|| VMError::RuntimeError("Integer overflow in '**'".into()))?;
                    return Self::integer_result_boxed(out, "**").map(Some);
                }
                let base_f = Self::arith_i128_to_lossless_f64(base).ok_or_else(|| {
                    VMError::RuntimeError(format!(
                        "Cannot apply '**' without explicit cast: {} is not losslessly representable as number",
                        base
                    ))
                })?;
                let exp_f = Self::arith_i128_to_lossless_f64(exp).ok_or_else(|| {
                    VMError::RuntimeError(format!(
                        "Cannot apply '**' without explicit cast: {} is not losslessly representable as number",
                        exp
                    ))
                })?;
                Ok(Some(ValueWord::from_f64(base_f.powf(exp_f))))
            }
            (NumericDomain::Float(base), NumericDomain::Float(exp)) => {
                Ok(Some(ValueWord::from_f64(base.powf(exp))))
            }
            (NumericDomain::Int(base), NumericDomain::Float(exp)) => {
                let base_f = Self::arith_i128_to_lossless_f64(base).ok_or_else(|| {
                    VMError::RuntimeError(format!(
                        "Cannot apply '**' without explicit cast: {} is not losslessly representable as number",
                        base
                    ))
                })?;
                Ok(Some(ValueWord::from_f64(base_f.powf(exp))))
            }
            (NumericDomain::Float(base), NumericDomain::Int(exp)) => {
                let exp_f = Self::arith_i128_to_lossless_f64(exp).ok_or_else(|| {
                    VMError::RuntimeError(format!(
                        "Cannot apply '**' without explicit cast: {} is not losslessly representable as number",
                        exp
                    ))
                })?;
                Ok(Some(ValueWord::from_f64(base.powf(exp_f))))
            }
        }
    }

    // ===== Opcode Implementations =====

    /// Execute typed arithmetic opcodes (compiler-guaranteed types, zero dispatch)
    #[inline(always)]
    pub(in crate::executor) fn exec_typed_arithmetic(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        if let Some(ref mut metrics) = self.metrics {
            metrics.record_guarded_op();
        }
        use OpCode::*;
        match instruction.opcode {
            // ===== Typed Add (ValueWord fast path for Int/Number) =====
            AddInt => {
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                if a.is_i64() && b.is_i64() {
                    self.push_vw(unsafe { ValueWord::add_i64(&a, &b) })?;
                } else if let (Some(ai), Some(bi)) = (Self::int_operand(&a), Self::int_operand(&b))
                {
                    match ai.checked_add(bi) {
                        Some(result) if fits_i48(result) => {
                            self.push_vw(ValueWord::from_i64(result))?
                        }
                        _ => self.push_vw(ValueWord::from_f64(ai as f64 + bi as f64))?,
                    }
                } else {
                    return Err(VMError::TypeError {
                        expected: "int",
                        got: if Self::int_operand(&a).is_none() {
                            a.type_name()
                        } else {
                            b.type_name()
                        },
                    });
                }
            }
            AddNumber => {
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                let lhs = Self::number_operand(&a).ok_or_else(|| VMError::TypeError {
                    expected: "number",
                    got: a.type_name(),
                })?;
                let rhs = Self::number_operand(&b).ok_or_else(|| VMError::TypeError {
                    expected: "number",
                    got: b.type_name(),
                })?;
                self.push_vw(ValueWord::from_f64(lhs + rhs))?;
            }
            AddDecimal => {
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                self.push_vw(ValueWord::from_decimal(unsafe {
                    a.as_decimal_unchecked() + b.as_decimal_unchecked()
                }))?;
            }
            // ===== Typed Sub =====
            SubInt => {
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                if a.is_i64() && b.is_i64() {
                    self.push_vw(unsafe { ValueWord::sub_i64(&a, &b) })?;
                } else if let (Some(ai), Some(bi)) = (Self::int_operand(&a), Self::int_operand(&b))
                {
                    match ai.checked_sub(bi) {
                        Some(result) if fits_i48(result) => {
                            self.push_vw(ValueWord::from_i64(result))?
                        }
                        _ => self.push_vw(ValueWord::from_f64(ai as f64 - bi as f64))?,
                    }
                } else {
                    return Err(VMError::TypeError {
                        expected: "int",
                        got: if Self::int_operand(&a).is_none() {
                            a.type_name()
                        } else {
                            b.type_name()
                        },
                    });
                }
            }
            SubNumber => {
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                let lhs = Self::number_operand(&a).ok_or_else(|| VMError::TypeError {
                    expected: "number",
                    got: a.type_name(),
                })?;
                let rhs = Self::number_operand(&b).ok_or_else(|| VMError::TypeError {
                    expected: "number",
                    got: b.type_name(),
                })?;
                self.push_vw(ValueWord::from_f64(lhs - rhs))?;
            }
            SubDecimal => {
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                self.push_vw(ValueWord::from_decimal(unsafe {
                    a.as_decimal_unchecked() - b.as_decimal_unchecked()
                }))?;
            }
            // ===== Typed Mul =====
            MulInt => {
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                if a.is_i64() && b.is_i64() {
                    self.push_vw(unsafe { ValueWord::mul_i64(&a, &b) })?;
                } else if let (Some(ai), Some(bi)) = (Self::int_operand(&a), Self::int_operand(&b))
                {
                    match ai.checked_mul(bi) {
                        Some(result) if fits_i48(result) => {
                            self.push_vw(ValueWord::from_i64(result))?
                        }
                        _ => self.push_vw(ValueWord::from_f64(ai as f64 * bi as f64))?,
                    }
                } else {
                    return Err(VMError::TypeError {
                        expected: "int",
                        got: if Self::int_operand(&a).is_none() {
                            a.type_name()
                        } else {
                            b.type_name()
                        },
                    });
                }
            }
            MulNumber => {
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                let lhs = Self::number_operand(&a).ok_or_else(|| VMError::TypeError {
                    expected: "number",
                    got: a.type_name(),
                })?;
                let rhs = Self::number_operand(&b).ok_or_else(|| VMError::TypeError {
                    expected: "number",
                    got: b.type_name(),
                })?;
                self.push_vw(ValueWord::from_f64(lhs * rhs))?;
            }
            MulDecimal => {
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                self.push_vw(ValueWord::from_decimal(unsafe {
                    a.as_decimal_unchecked() * b.as_decimal_unchecked()
                }))?;
            }
            // ===== Typed Div (with zero-check) =====
            DivInt => {
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                let bi = Self::int_operand(&b).ok_or_else(|| VMError::TypeError {
                    expected: "int",
                    got: b.type_name(),
                })?;
                if bi == 0 {
                    return Err(VMError::DivisionByZero);
                }
                let ai = Self::int_operand(&a).ok_or_else(|| VMError::TypeError {
                    expected: "int",
                    got: a.type_name(),
                })?;
                self.push_vw(ValueWord::from_i64(ai / bi))?;
            }
            DivNumber => {
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                let divisor = Self::number_operand(&b).ok_or_else(|| VMError::TypeError {
                    expected: "number",
                    got: b.type_name(),
                })?;
                if divisor == 0.0 {
                    return Err(VMError::DivisionByZero);
                }
                let lhs = Self::number_operand(&a).ok_or_else(|| VMError::TypeError {
                    expected: "number",
                    got: a.type_name(),
                })?;
                self.push_vw(ValueWord::from_f64(lhs / divisor))?;
            }
            DivDecimal => {
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                let divisor = unsafe { b.as_decimal_unchecked() };
                if divisor.is_zero() {
                    return Err(VMError::DivisionByZero);
                }
                self.push_vw(ValueWord::from_decimal(
                    unsafe { a.as_decimal_unchecked() } / divisor,
                ))?;
            }
            // ===== Typed Mod (with zero-check) =====
            ModInt => {
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                let bi = Self::int_operand(&b).ok_or_else(|| VMError::TypeError {
                    expected: "int",
                    got: b.type_name(),
                })?;
                if bi == 0 {
                    return Err(VMError::DivisionByZero);
                }
                let ai = Self::int_operand(&a).ok_or_else(|| VMError::TypeError {
                    expected: "int",
                    got: a.type_name(),
                })?;
                self.push_vw(ValueWord::from_i64(ai % bi))?;
            }
            ModNumber => {
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                let divisor = Self::number_operand(&b).ok_or_else(|| VMError::TypeError {
                    expected: "number",
                    got: b.type_name(),
                })?;
                if divisor == 0.0 {
                    return Err(VMError::DivisionByZero);
                }
                let lhs = Self::number_operand(&a).ok_or_else(|| VMError::TypeError {
                    expected: "number",
                    got: a.type_name(),
                })?;
                self.push_vw(ValueWord::from_f64(lhs % divisor))?;
            }
            ModDecimal => {
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                let divisor = unsafe { b.as_decimal_unchecked() };
                if divisor.is_zero() {
                    return Err(VMError::DivisionByZero);
                }
                self.push_vw(ValueWord::from_decimal(
                    unsafe { a.as_decimal_unchecked() } % divisor,
                ))?;
            }
            // ===== Typed Pow =====
            PowInt => {
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                let base = Self::int_operand(&a).ok_or_else(|| VMError::TypeError {
                    expected: "int",
                    got: a.type_name(),
                })?;
                let exp = Self::int_operand(&b).ok_or_else(|| VMError::TypeError {
                    expected: "int",
                    got: b.type_name(),
                })?;
                if exp >= 0 && exp < u32::MAX as i64 {
                    self.push_vw(ValueWord::from_i64(base.pow(exp as u32)))?;
                } else {
                    self.push_vw(ValueWord::from_f64((base as f64).powf(exp as f64)))?;
                }
            }
            PowNumber => {
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                let base = Self::number_operand(&a).ok_or_else(|| VMError::TypeError {
                    expected: "number",
                    got: a.type_name(),
                })?;
                let exp = Self::number_operand(&b).ok_or_else(|| VMError::TypeError {
                    expected: "number",
                    got: b.type_name(),
                })?;
                self.push_vw(ValueWord::from_f64(base.powf(exp)))?;
            }
            PowDecimal => {
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                use rust_decimal::prelude::ToPrimitive;
                let base = unsafe { a.as_decimal_unchecked() };
                let exp = unsafe { b.as_decimal_unchecked() };
                let result = base
                    .to_f64()
                    .unwrap_or(0.0)
                    .powf(exp.to_f64().unwrap_or(0.0));
                use rust_decimal::prelude::FromPrimitive;
                self.push_vw(ValueWord::from_decimal(
                    rust_decimal::Decimal::from_f64(result).unwrap_or_default(),
                ))?;
            }
            // ===== Numeric Coercion =====
            IntToNumber => {
                let val = self.pop_vw()?;
                self.push_vw(ValueWord::from_f64(unsafe { val.as_i64_unchecked() } as f64))?;
            }
            NumberToInt => {
                let val = self.pop_vw()?;
                self.push_vw(ValueWord::from_i64(unsafe { val.as_f64_unchecked() } as i64))?;
            }
            _ => unreachable!(
                "exec_typed_arithmetic called with non-typed-arithmetic opcode: {:?}",
                instruction.opcode
            ),
        }
        Ok(())
    }

    // ---------------------------------------------------------------
    // Trusted typed opcodes (compiler-proved types, no runtime guard)
    // ---------------------------------------------------------------

    /// Execute trusted arithmetic opcodes. These skip all runtime type checks
    /// because the compiler has proved both operands have matching types via
    /// StorageHint analysis. In debug builds, debug_assert guards still fire.
    #[inline(always)]
    pub(in crate::executor) fn exec_trusted_arithmetic(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        if let Some(ref mut metrics) = self.metrics {
            metrics.record_trusted_op();
        }
        use OpCode::*;
        match instruction.opcode {
            AddIntTrusted => {
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                debug_assert!(
                    a.is_i64() && b.is_i64(),
                    "Trusted AddInt invariant violated"
                );
                let ai = unsafe { a.as_i64_unchecked() };
                let bi = unsafe { b.as_i64_unchecked() };
                match ai.checked_add(bi) {
                    Some(result) if fits_i48(result) => {
                        self.push_vw(ValueWord::from_i64(result))?
                    }
                    _ => self.push_vw(ValueWord::from_f64(ai as f64 + bi as f64))?,
                }
            }
            SubIntTrusted => {
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                debug_assert!(
                    a.is_i64() && b.is_i64(),
                    "Trusted SubInt invariant violated"
                );
                let ai = unsafe { a.as_i64_unchecked() };
                let bi = unsafe { b.as_i64_unchecked() };
                match ai.checked_sub(bi) {
                    Some(result) if fits_i48(result) => {
                        self.push_vw(ValueWord::from_i64(result))?
                    }
                    _ => self.push_vw(ValueWord::from_f64(ai as f64 - bi as f64))?,
                }
            }
            MulIntTrusted => {
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                debug_assert!(
                    a.is_i64() && b.is_i64(),
                    "Trusted MulInt invariant violated"
                );
                let ai = unsafe { a.as_i64_unchecked() };
                let bi = unsafe { b.as_i64_unchecked() };
                match ai.checked_mul(bi) {
                    Some(result) if fits_i48(result) => {
                        self.push_vw(ValueWord::from_i64(result))?
                    }
                    _ => self.push_vw(ValueWord::from_f64(ai as f64 * bi as f64))?,
                }
            }
            DivIntTrusted => {
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                debug_assert!(
                    a.is_i64() && b.is_i64(),
                    "Trusted DivInt invariant violated"
                );
                let bi = unsafe { b.as_i64_unchecked() };
                if bi == 0 {
                    return Err(VMError::DivisionByZero);
                }
                let ai = unsafe { a.as_i64_unchecked() };
                self.push_vw(ValueWord::from_i64(ai / bi))?;
            }
            AddNumberTrusted => {
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                debug_assert!(
                    a.as_number_coerce().is_some() && b.as_number_coerce().is_some(),
                    "Trusted AddNumber invariant violated"
                );
                self.push_vw(ValueWord::from_f64(unsafe {
                    a.as_f64_unchecked() + b.as_f64_unchecked()
                }))?;
            }
            SubNumberTrusted => {
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                debug_assert!(
                    a.as_number_coerce().is_some() && b.as_number_coerce().is_some(),
                    "Trusted SubNumber invariant violated"
                );
                self.push_vw(ValueWord::from_f64(unsafe {
                    a.as_f64_unchecked() - b.as_f64_unchecked()
                }))?;
            }
            MulNumberTrusted => {
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                debug_assert!(
                    a.as_number_coerce().is_some() && b.as_number_coerce().is_some(),
                    "Trusted MulNumber invariant violated"
                );
                self.push_vw(ValueWord::from_f64(unsafe {
                    a.as_f64_unchecked() * b.as_f64_unchecked()
                }))?;
            }
            DivNumberTrusted => {
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                debug_assert!(
                    a.as_number_coerce().is_some() && b.as_number_coerce().is_some(),
                    "Trusted DivNumber invariant violated"
                );
                let divisor = unsafe { b.as_f64_unchecked() };
                if divisor == 0.0 {
                    return Err(VMError::DivisionByZero);
                }
                self.push_vw(ValueWord::from_f64(
                    unsafe { a.as_f64_unchecked() } / divisor,
                ))?;
            }
            _ => unreachable!(
                "exec_trusted_arithmetic called with non-trusted opcode: {:?}",
                instruction.opcode
            ),
        }
        Ok(())
    }

    // ---------------------------------------------------------------
    // Compact typed opcodes (ABI-stable, width-parameterised)
    // ---------------------------------------------------------------

    /// Execute a compact typed arithmetic opcode (AddTyped .. ModTyped, CmpTyped).
    ///
    /// These opcodes carry an `Operand::Width(NumericWidth)` that selects the
    /// concrete numeric width.  At present the handler delegates to the existing
    /// per-type opcodes (AddInt, AddNumber, etc.) so semantics are identical;
    /// the value of the compact family is a stable, width-parameterised bytecode
    /// ABI that future backends can emit without knowing the per-legacy-opcode
    /// layout.
    pub(in crate::executor) fn exec_compact_typed_arithmetic(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use crate::bytecode::Operand;
        use OpCode::*;

        let width = match instruction.operand {
            Some(Operand::Width(w)) => w,
            _ => {
                return Err(VMError::InvalidOperand);
            }
        };

        match instruction.opcode {
            AddTyped => self.exec_compact_add(width),
            SubTyped => self.exec_compact_sub(width),
            MulTyped => self.exec_compact_mul(width),
            DivTyped => self.exec_compact_div(width),
            ModTyped => self.exec_compact_mod(width),
            CmpTyped => self.exec_compact_cmp(width),
            _ => unreachable!(
                "exec_compact_typed_arithmetic called with {:?}",
                instruction.opcode
            ),
        }
    }

    // --- compact opcode implementation helpers ---

    #[inline(always)]
    fn compact_int_type_error(a: &ValueWord, b: &ValueWord) -> VMError {
        VMError::TypeError {
            expected: "int",
            got: if Self::int_operand(a).is_none() {
                a.type_name()
            } else {
                b.type_name()
            },
        }
    }

    #[inline(always)]
    fn compact_number_type_error(a: &ValueWord, b: &ValueWord) -> VMError {
        VMError::TypeError {
            expected: "number",
            got: if Self::number_operand(a).is_none() {
                a.type_name()
            } else {
                b.type_name()
            },
        }
    }

    /// Width-aware wrapping for sub-64 and u64 widths. I64 returns value unchanged.
    #[inline(always)]
    fn width_wrap(value: i64, width: NumericWidth) -> ValueWord {
        match width.to_int_width() {
            Some(w) => ValueWord::from_i64(w.truncate(value)),
            None => {
                // I64 or F32/F64 — no truncation
                ValueWord::from_i64(value)
            }
        }
    }

    #[inline(always)]
    fn compact_int_checked_binop(
        &mut self,
        width: NumericWidth,
        wrapping_op: impl FnOnce(i64, i64) -> i64,
        checked: impl FnOnce(i64, i64) -> Option<i64>,
        overflow_fallback: impl FnOnce(i64, i64) -> f64,
    ) -> Result<(), VMError> {
        let b = self.pop_vw()?;
        let a = self.pop_vw()?;

        let (ai, bi) = if a.is_i64() && b.is_i64() {
            (unsafe { a.as_i64_unchecked() }, unsafe {
                b.as_i64_unchecked()
            })
        } else if let (Some(ai), Some(bi)) = (Self::int_operand(&a), Self::int_operand(&b)) {
            (ai, bi)
        } else {
            return Err(Self::compact_int_type_error(&a, &b));
        };

        // For sub-i64 widths: wrapping arithmetic + truncation
        if width.to_int_width().is_some() {
            let result = wrapping_op(ai, bi);
            return self.push_vw(Self::width_wrap(result, width));
        }

        // I64: checked with f64 fallback on overflow
        match checked(ai, bi) {
            Some(result) => self.push_vw(ValueWord::from_i64(result)),
            None => self.push_vw(ValueWord::from_f64(overflow_fallback(ai, bi))),
        }
    }

    #[inline(always)]
    fn compact_int_divmod(
        &mut self,
        width: NumericWidth,
        op: impl FnOnce(i64, i64) -> i64,
    ) -> Result<(), VMError> {
        let b = self.pop_vw()?;
        let a = self.pop_vw()?;

        let bi = Self::int_operand(&b).ok_or_else(|| VMError::TypeError {
            expected: "int",
            got: b.type_name(),
        })?;
        if bi == 0 {
            return Err(VMError::DivisionByZero);
        }
        let ai = Self::int_operand(&a).ok_or_else(|| VMError::TypeError {
            expected: "int",
            got: a.type_name(),
        })?;
        let result = op(ai, bi);
        self.push_vw(Self::width_wrap(result, width))
    }

    #[inline(always)]
    fn compact_float_binop(&mut self, op: impl FnOnce(f64, f64) -> f64) -> Result<(), VMError> {
        let b = self.pop_vw()?;
        let a = self.pop_vw()?;

        let lhs =
            Self::number_operand(&a).ok_or_else(|| Self::compact_number_type_error(&a, &b))?;
        let rhs =
            Self::number_operand(&b).ok_or_else(|| Self::compact_number_type_error(&a, &b))?;
        self.push_vw(ValueWord::from_f64(op(lhs, rhs)))
    }

    #[inline(always)]
    fn compact_float_divmod(&mut self, op: impl FnOnce(f64, f64) -> f64) -> Result<(), VMError> {
        let b = self.pop_vw()?;
        let a = self.pop_vw()?;

        let rhs = Self::number_operand(&b).ok_or_else(|| VMError::TypeError {
            expected: "number",
            got: b.type_name(),
        })?;
        if rhs == 0.0 {
            return Err(VMError::DivisionByZero);
        }
        let lhs = Self::number_operand(&a).ok_or_else(|| VMError::TypeError {
            expected: "number",
            got: a.type_name(),
        })?;
        self.push_vw(ValueWord::from_f64(op(lhs, rhs)))
    }

    #[inline(always)]
    fn compact_int_cmp(&mut self, width: NumericWidth) -> Result<(), VMError> {
        let b = self.pop_vw()?;
        let a = self.pop_vw()?;
        let ai = Self::int_operand(&a).ok_or_else(|| VMError::TypeError {
            expected: "int",
            got: a.type_name(),
        })?;
        let bi = Self::int_operand(&b).ok_or_else(|| VMError::TypeError {
            expected: "int",
            got: b.type_name(),
        })?;
        // For unsigned widths, compare as unsigned
        let ord = if width.is_unsigned() {
            (ai as u64).cmp(&(bi as u64)) as i64
        } else {
            ai.cmp(&bi) as i64
        };
        self.push_vw(ValueWord::from_i64(ord))
    }

    #[inline(always)]
    fn compact_float_cmp(&mut self) -> Result<(), VMError> {
        let b = self.pop_vw()?;
        let a = self.pop_vw()?;
        let lhs = Self::number_operand(&a).ok_or_else(|| VMError::TypeError {
            expected: "number",
            got: a.type_name(),
        })?;
        let rhs = Self::number_operand(&b).ok_or_else(|| VMError::TypeError {
            expected: "number",
            got: b.type_name(),
        })?;
        let ord = lhs.partial_cmp(&rhs).map_or(0i64, |o| o as i64);
        self.push_vw(ValueWord::from_i64(ord))
    }

    #[inline(always)]
    fn compact_int_add(&mut self, width: NumericWidth) -> Result<(), VMError> {
        self.compact_int_checked_binop(
            width,
            |a, b| a.wrapping_add(b),
            |a, b| a.checked_add(b),
            |a, b| a as f64 + b as f64,
        )
    }

    #[inline(always)]
    fn compact_float_add(&mut self) -> Result<(), VMError> {
        self.compact_float_binop(|a, b| a + b)
    }

    #[inline(always)]
    fn compact_int_sub(&mut self, width: NumericWidth) -> Result<(), VMError> {
        self.compact_int_checked_binop(
            width,
            |a, b| a.wrapping_sub(b),
            |a, b| a.checked_sub(b),
            |a, b| a as f64 - b as f64,
        )
    }

    #[inline(always)]
    fn compact_float_sub(&mut self) -> Result<(), VMError> {
        self.compact_float_binop(|a, b| a - b)
    }

    #[inline(always)]
    fn compact_int_mul(&mut self, width: NumericWidth) -> Result<(), VMError> {
        self.compact_int_checked_binop(
            width,
            |a, b| a.wrapping_mul(b),
            |a, b| a.checked_mul(b),
            |a, b| a as f64 * b as f64,
        )
    }

    #[inline(always)]
    fn compact_float_mul(&mut self) -> Result<(), VMError> {
        self.compact_float_binop(|a, b| a * b)
    }

    #[inline(always)]
    fn compact_int_div(&mut self, width: NumericWidth) -> Result<(), VMError> {
        self.compact_int_divmod(width, |a, b| a.wrapping_div(b))
    }

    #[inline(always)]
    fn compact_float_div(&mut self) -> Result<(), VMError> {
        self.compact_float_divmod(|a, b| a / b)
    }

    #[inline(always)]
    fn compact_int_mod(&mut self, width: NumericWidth) -> Result<(), VMError> {
        self.compact_int_divmod(width, |a, b| a.wrapping_rem(b))
    }

    #[inline(always)]
    fn compact_float_mod(&mut self) -> Result<(), VMError> {
        self.compact_float_divmod(|a, b| a % b)
    }

    define_compact_width_dispatch! {
        exec_compact_add => compact_int_add, compact_float_add;
        exec_compact_sub => compact_int_sub, compact_float_sub;
        exec_compact_mul => compact_int_mul, compact_float_mul;
        exec_compact_div => compact_int_div, compact_float_div;
        exec_compact_mod => compact_int_mod, compact_float_mod;
        exec_compact_cmp => compact_int_cmp, compact_float_cmp;
    }

    /// Execute CastWidth: pop value, truncate to declared width, push result.
    #[inline(always)]
    pub(in crate::executor) fn op_cast_width(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let width = match instruction.operand {
            Some(Operand::Width(w)) => w,
            _ => return Err(VMError::InvalidOperand),
        };
        let nb = self.pop_vw()?;
        let raw = Self::int_operand(&nb).unwrap_or_else(|| {
            // If not an int, try to extract from number
            nb.as_f64().map(|f| f as i64).unwrap_or(0)
        });
        if let Some(int_w) = width.to_int_width() {
            self.push_vw(ValueWord::from_i64(int_w.truncate(raw)))
        } else {
            // I64 or float: no truncation
            self.push_vw(ValueWord::from_i64(raw))
        }
    }

    #[inline(always)]
    pub(in crate::executor) fn exec_arithmetic(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use OpCode::*;
        match instruction.opcode {
            Add => {
                use shape_value::NanTag;
                // IC fast path: if monomorphic I48+I48 or F64+F64, try typed fast path
                // before the full generic dispatch.
                {
                    use crate::executor::ic_fast_paths::{ArithmeticIcHint, arithmetic_ic_check};
                    let hint = arithmetic_ic_check(self, self.ip);
                    if hint == ArithmeticIcHint::BothI48 {
                        // Peek at stack top two values without popping
                        if self.sp >= 2 {
                            let b = &self.stack[self.sp - 1];
                            let a = &self.stack[self.sp - 2];
                            if a.is_i64() && b.is_i64() {
                                let result = unsafe { ValueWord::add_i64(a, b) };
                                self.sp -= 2;
                                let ip = self.ip;
                                if let Some(fv) = self.current_feedback_vector() {
                                    fv.record_arithmetic(ip, NanTag::I48 as u8, NanTag::I48 as u8);
                                }
                                return self.push_vw(result);
                            }
                        }
                    } else if hint == ArithmeticIcHint::BothF64 {
                        if self.sp >= 2 {
                            let b = &self.stack[self.sp - 1];
                            let a = &self.stack[self.sp - 2];
                            if let (Some(af), Some(bf)) = (a.as_f64(), b.as_f64()) {
                                self.sp -= 2;
                                let ip = self.ip;
                                if let Some(fv) = self.current_feedback_vector() {
                                    fv.record_arithmetic(ip, NanTag::F64 as u8, NanTag::F64 as u8);
                                }
                                return self.push_vw(ValueWord::from_f64(af + bf));
                            }
                        }
                    }
                }
                // Generic path: pop, unwrap annotations, full dispatch.
                let b_nb = unwrap_annotated(self.pop_vw()?);
                let a_nb = unwrap_annotated(self.pop_vw()?);
                // Record operand types for IC profiling.
                {
                    let ip = self.ip;
                    if let Some(fv) = self.current_feedback_vector() {
                        fv.record_arithmetic(ip, a_nb.tag() as u8, b_nb.tag() as u8);
                    }
                }
                if let Some(result) = Self::numeric_binary_result(
                    &a_nb,
                    &b_nb,
                    "+",
                    |a, b| a.checked_add(b),
                    |a, b| a + b,
                )? {
                    return self.push_vw(result);
                }
                match (a_nb.tag(), b_nb.tag()) {
                    // Both inline numeric: int-preserving arithmetic
                    (NanTag::I48 | NanTag::F64, NanTag::I48 | NanTag::F64) => {
                        if let (Some(a_num), Some(b_num)) =
                            (a_nb.as_number_coerce(), b_nb.as_number_coerce())
                        {
                            return self.push_vw(ValueWord::binary_int_preserving(
                                &a_nb,
                                &b_nb,
                                a_num,
                                b_num,
                                |a, b| a.checked_add(b),
                                |a, b| a + b,
                            ));
                        }
                        return Err(VMError::RuntimeError(format!(
                            "Cannot apply '+' to {} and {}",
                            a_nb.type_name(),
                            b_nb.type_name()
                        )));
                    }
                    // Both heap: string concat, decimal, bigint, array concat, typed object merge
                    (NanTag::Heap, NanTag::Heap) => {
                        match (a_nb.as_heap_ref().unwrap(), b_nb.as_heap_ref().unwrap()) {
                            (HeapValue::BigInt(a_big), HeapValue::BigInt(b_big)) => {
                                return self.push_vw(ValueWord::from_i64(
                                    a_big.checked_add(*b_big).ok_or_else(|| {
                                        VMError::RuntimeError("Integer overflow".into())
                                    })?,
                                ));
                            }
                            (HeapValue::String(s_a), HeapValue::String(s_b)) => {
                                return self.push_vw(ValueWord::from_string(Arc::new(format!(
                                    "{}{}",
                                    s_a, s_b
                                ))));
                            }
                            (HeapValue::String(s), HeapValue::Char(c)) => {
                                return self.push_vw(ValueWord::from_string(Arc::new(format!(
                                    "{}{}",
                                    s, c
                                ))));
                            }
                            (HeapValue::Char(c), HeapValue::String(s)) => {
                                return self.push_vw(ValueWord::from_string(Arc::new(format!(
                                    "{}{}",
                                    c, s
                                ))));
                            }
                            (HeapValue::Char(a), HeapValue::Char(b)) => {
                                return self.push_vw(ValueWord::from_string(Arc::new(format!(
                                    "{}{}",
                                    a, b
                                ))));
                            }
                            (HeapValue::Decimal(a_dec), HeapValue::Decimal(b_dec)) => {
                                return self.push_vw(ValueWord::from_decimal(*a_dec + *b_dec));
                            }
                            // Time + TimeSpan => Time
                            (HeapValue::Time(dt), HeapValue::TimeSpan(dur)) => {
                                let result = dt.checked_add_signed(*dur).ok_or_else(|| {
                                    VMError::RuntimeError(
                                        "DateTime overflow in addition".to_string(),
                                    )
                                })?;
                                return self.push_vw(ValueWord::from_time(result));
                            }
                            // TimeSpan + Time => Time
                            (HeapValue::TimeSpan(dur), HeapValue::Time(dt)) => {
                                let result = dt.checked_add_signed(*dur).ok_or_else(|| {
                                    VMError::RuntimeError(
                                        "DateTime overflow in addition".to_string(),
                                    )
                                })?;
                                return self.push_vw(ValueWord::from_time(result));
                            }
                            // TimeSpan + TimeSpan => TimeSpan
                            (HeapValue::TimeSpan(a_dur), HeapValue::TimeSpan(b_dur)) => {
                                let result = a_dur.checked_add(b_dur).ok_or_else(|| {
                                    VMError::RuntimeError(
                                        "Duration overflow in addition".to_string(),
                                    )
                                })?;
                                return self.push_vw(ValueWord::from_timespan(result));
                            }
                            // Vec<number> + Vec<number> => element-wise SIMD add
                            (HeapValue::FloatArray(a_arr), HeapValue::FloatArray(b_arr)) => {
                                if a_arr.len() != b_arr.len() {
                                    return Err(VMError::RuntimeError(format!(
                                        "Vec<number> length mismatch: {} vs {}",
                                        a_arr.len(),
                                        b_arr.len()
                                    )));
                                }
                                let result = shape_runtime::intrinsics::vector::simd_vec_add_f64(
                                    a_arr.as_slice(),
                                    b_arr.as_slice(),
                                );
                                return self
                                    .push_vw(ValueWord::from_float_array(Arc::new(result.into())));
                            }
                            // Vec<int> + Vec<int> => element-wise with overflow check
                            (HeapValue::IntArray(a_arr), HeapValue::IntArray(b_arr)) => {
                                if a_arr.len() != b_arr.len() {
                                    return Err(VMError::RuntimeError(format!(
                                        "Vec<int> length mismatch: {} vs {}",
                                        a_arr.len(),
                                        b_arr.len()
                                    )));
                                }
                                match shape_runtime::intrinsics::vector::simd_vec_add_i64(
                                    a_arr.as_slice(),
                                    b_arr.as_slice(),
                                ) {
                                    Ok(result) => {
                                        return self.push_vw(ValueWord::from_int_array(Arc::new(
                                            result.into(),
                                        )));
                                    }
                                    Err(()) => {
                                        return Err(VMError::RuntimeError(
                                            "Integer overflow in Vec<int> element-wise addition"
                                                .into(),
                                        ));
                                    }
                                }
                            }
                            // Vec<int> + Vec<number> => coerce to f64, element-wise add
                            (HeapValue::IntArray(a_arr), HeapValue::FloatArray(b_arr)) => {
                                if a_arr.len() != b_arr.len() {
                                    return Err(VMError::RuntimeError(format!(
                                        "Vec length mismatch: {} vs {}",
                                        a_arr.len(),
                                        b_arr.len()
                                    )));
                                }
                                let a_f64 = shape_runtime::intrinsics::vector::i64_slice_to_f64(
                                    a_arr.as_slice(),
                                );
                                let result = shape_runtime::intrinsics::vector::simd_vec_add_f64(
                                    &a_f64,
                                    b_arr.as_slice(),
                                );
                                return self
                                    .push_vw(ValueWord::from_float_array(Arc::new(result.into())));
                            }
                            // Vec<number> + Vec<int> => coerce to f64, element-wise add
                            (HeapValue::FloatArray(a_arr), HeapValue::IntArray(b_arr)) => {
                                if a_arr.len() != b_arr.len() {
                                    return Err(VMError::RuntimeError(format!(
                                        "Vec length mismatch: {} vs {}",
                                        a_arr.len(),
                                        b_arr.len()
                                    )));
                                }
                                let b_f64 = shape_runtime::intrinsics::vector::i64_slice_to_f64(
                                    b_arr.as_slice(),
                                );
                                let result = shape_runtime::intrinsics::vector::simd_vec_add_f64(
                                    a_arr.as_slice(),
                                    &b_f64,
                                );
                                return self
                                    .push_vw(ValueWord::from_float_array(Arc::new(result.into())));
                            }
                            // Matrix + Matrix => element-wise add
                            (HeapValue::Matrix(a_mat), HeapValue::Matrix(b_mat)) => {
                                let result = shape_runtime::intrinsics::matrix_kernels::matrix_add(
                                    a_mat, b_mat,
                                )
                                .map_err(|e| VMError::RuntimeError(e))?;
                                return self.push_vw(ValueWord::from_matrix(Box::new(result)));
                            }
                            (HeapValue::Array(arr_a), HeapValue::Array(arr_b)) => {
                                let mut result_arr = Vec::with_capacity(arr_a.len() + arr_b.len());
                                result_arr.extend_from_slice(arr_a);
                                result_arr.extend_from_slice(arr_b);
                                return self.push_vw(ValueWord::from_array(Arc::new(result_arr)));
                            }
                            // Decimal + non-decimal heap (shouldn't happen but handle)
                            (HeapValue::Decimal(a_dec), _) => {
                                if let Some(b_num) = b_nb.as_number_coerce() {
                                    use rust_decimal::prelude::FromPrimitive;
                                    let b_dec =
                                        rust_decimal::Decimal::from_f64(b_num).unwrap_or_default();
                                    return self.push_vw(ValueWord::from_decimal(*a_dec + b_dec));
                                }
                            }
                            (_, HeapValue::Decimal(b_dec)) => {
                                if let Some(a_num) = a_nb.as_number_coerce() {
                                    use rust_decimal::prelude::FromPrimitive;
                                    let a_dec =
                                        rust_decimal::Decimal::from_f64(a_num).unwrap_or_default();
                                    return self.push_vw(ValueWord::from_decimal(a_dec + *b_dec));
                                }
                            }
                            // TypedObject + TypedObject: operator trait first, then merge
                            (
                                HeapValue::TypedObject {
                                    schema_id: id_a,
                                    slots: slots_a,
                                    heap_mask: mask_a,
                                },
                                HeapValue::TypedObject {
                                    schema_id: id_b,
                                    slots: slots_b,
                                    heap_mask: mask_b,
                                },
                            ) => {
                                // Try operator trait dispatch (impl Add for T)
                                if let Some(result) = self.try_binary_operator_trait(
                                    a_nb.clone(),
                                    b_nb.clone(),
                                    "add",
                                )? {
                                    return self.push_vw(result);
                                }
                                let merged_name = format!("__intersection_{}_{}", id_a, id_b);
                                let merged_id = self
                                    .program
                                    .type_schema_registry
                                    .get(&merged_name)
                                    .map(|s| s.id)
                                    .ok_or_else(|| {
                                        VMError::RuntimeError(format!(
                                            "Missing predeclared intersection schema '{}' for typed object addition",
                                            merged_name
                                        ))
                                    })?;
                                let mut merged_slots =
                                    Vec::with_capacity(slots_a.len() + slots_b.len());
                                let mut merged_mask: u64 = 0;
                                for i in 0..slots_a.len() {
                                    if *mask_a & (1u64 << i) != 0 {
                                        merged_slots.push(unsafe { slots_a[i].clone_heap() });
                                        merged_mask |= 1u64 << (merged_slots.len() - 1);
                                    } else {
                                        merged_slots.push(slots_a[i]);
                                    }
                                }
                                for i in 0..slots_b.len() {
                                    let idx = merged_slots.len();
                                    if *mask_b & (1u64 << i) != 0 {
                                        merged_slots.push(unsafe { slots_b[i].clone_heap() });
                                        merged_mask |= 1u64 << idx;
                                    } else {
                                        merged_slots.push(slots_b[i]);
                                    }
                                }
                                return self.push_vw(ValueWord::from_heap_value(
                                    HeapValue::TypedObject {
                                        schema_id: merged_id as u64,
                                        slots: merged_slots.into_boxed_slice(),
                                        heap_mask: merged_mask,
                                    },
                                ));
                            }
                            _ => {}
                        }
                        // Operator trait fallback (Add)
                        if let Some(result) =
                            self.try_binary_operator_trait(a_nb.clone(), b_nb.clone(), "add")?
                        {
                            return self.push_vw(result);
                        }
                        return Err(VMError::RuntimeError(format!(
                            "Cannot apply '+' to {} and {}",
                            a_nb.type_name(),
                            b_nb.type_name()
                        )));
                    }
                    // Mixed: one heap, one inline — bigint+num, string+num, decimal+num coercion
                    (NanTag::Heap, _) => {
                        // Vec<number> + scalar => broadcast add
                        if let Some(a_arr) = a_nb.as_float_array() {
                            if let Some(scalar) = b_nb.as_number_coerce() {
                                let result = shape_runtime::intrinsics::vector::simd_vec_add_f64(
                                    a_arr.as_slice(),
                                    &vec![scalar; a_arr.len()],
                                );
                                return self
                                    .push_vw(ValueWord::from_float_array(Arc::new(result.into())));
                            }
                        }
                        // Vec<int> + scalar int => broadcast add
                        if let Some(a_arr) = a_nb.as_int_array() {
                            if let Some(scalar) = b_nb.as_i64() {
                                let b_vec = vec![scalar; a_arr.len()];
                                match shape_runtime::intrinsics::vector::simd_vec_add_i64(
                                    a_arr.as_slice(),
                                    &b_vec,
                                ) {
                                    Ok(result) => {
                                        return self.push_vw(ValueWord::from_int_array(Arc::new(
                                            result.into(),
                                        )));
                                    }
                                    Err(()) => {
                                        return Err(VMError::RuntimeError(
                                            "Integer overflow in Vec<int> scalar addition".into(),
                                        ));
                                    }
                                }
                            }
                        }
                        if let Some(HeapValue::BigInt(a_big)) = a_nb.as_heap_ref() {
                            if let Some(b_i) = b_nb.as_i64() {
                                return self.push_vw(ValueWord::from_i64(
                                    a_big.checked_add(b_i).ok_or_else(|| {
                                        VMError::RuntimeError("Integer overflow".into())
                                    })?,
                                ));
                            }
                            if let Some(b_f) = b_nb.as_f64() {
                                return self.push_vw(ValueWord::from_f64(*a_big as f64 + b_f));
                            }
                        }
                        if let Some(s) = a_nb.as_str() {
                            if let Some(i) = b_nb.as_i64() {
                                return self.push_vw(ValueWord::from_string(Arc::new(format!(
                                    "{}{}",
                                    s, i
                                ))));
                            }
                            if let Some(n) = b_nb.as_f64() {
                                let n_str = if n.fract() == 0.0 {
                                    format!("{}", n as i64)
                                } else {
                                    format!("{}", n)
                                };
                                return self.push_vw(ValueWord::from_string(Arc::new(format!(
                                    "{}{}",
                                    s, n_str
                                ))));
                            }
                        }
                        if let Some(a_dec) = a_nb.as_decimal() {
                            if let Some(b_int) = b_nb.as_i64() {
                                return self.push_vw(ValueWord::from_decimal(
                                    a_dec + rust_decimal::Decimal::from(b_int),
                                ));
                            }
                            if let Some(b_num) = b_nb.as_f64() {
                                use rust_decimal::prelude::FromPrimitive;
                                let b_dec =
                                    rust_decimal::Decimal::from_f64(b_num).unwrap_or_default();
                                return self.push_vw(ValueWord::from_decimal(a_dec + b_dec));
                            }
                        }
                        // Operator trait fallback (Add)
                        if let Some(result) =
                            self.try_binary_operator_trait(a_nb.clone(), b_nb.clone(), "add")?
                        {
                            return self.push_vw(result);
                        }
                        return Err(VMError::RuntimeError(format!(
                            "Cannot apply '+' to {} and {}",
                            a_nb.type_name(),
                            b_nb.type_name()
                        )));
                    }
                    (_, NanTag::Heap) => {
                        // scalar + Vec<number> => broadcast add
                        if let Some(b_arr) = b_nb.as_float_array() {
                            if let Some(scalar) = a_nb.as_number_coerce() {
                                let result = shape_runtime::intrinsics::vector::simd_vec_add_f64(
                                    &vec![scalar; b_arr.len()],
                                    b_arr.as_slice(),
                                );
                                return self
                                    .push_vw(ValueWord::from_float_array(Arc::new(result.into())));
                            }
                        }
                        // scalar int + Vec<int> => broadcast add
                        if let Some(b_arr) = b_nb.as_int_array() {
                            if let Some(scalar) = a_nb.as_i64() {
                                let a_vec = vec![scalar; b_arr.len()];
                                match shape_runtime::intrinsics::vector::simd_vec_add_i64(
                                    &a_vec,
                                    b_arr.as_slice(),
                                ) {
                                    Ok(result) => {
                                        return self.push_vw(ValueWord::from_int_array(Arc::new(
                                            result.into(),
                                        )));
                                    }
                                    Err(()) => {
                                        return Err(VMError::RuntimeError(
                                            "Integer overflow in Vec<int> scalar addition".into(),
                                        ));
                                    }
                                }
                            }
                        }
                        if let Some(HeapValue::BigInt(b_big)) = b_nb.as_heap_ref() {
                            if let Some(a_i) = a_nb.as_i64() {
                                return self.push_vw(ValueWord::from_i64(
                                    a_i.checked_add(*b_big).ok_or_else(|| {
                                        VMError::RuntimeError("Integer overflow".into())
                                    })?,
                                ));
                            }
                            if let Some(a_f) = a_nb.as_f64() {
                                return self.push_vw(ValueWord::from_f64(a_f + *b_big as f64));
                            }
                        }
                        if let Some(s) = b_nb.as_str() {
                            if let Some(i) = a_nb.as_i64() {
                                return self.push_vw(ValueWord::from_string(Arc::new(format!(
                                    "{}{}",
                                    i, s
                                ))));
                            }
                            if let Some(n) = a_nb.as_f64() {
                                let n_str = if n.fract() == 0.0 {
                                    format!("{}", n as i64)
                                } else {
                                    format!("{}", n)
                                };
                                return self.push_vw(ValueWord::from_string(Arc::new(format!(
                                    "{}{}",
                                    n_str, s
                                ))));
                            }
                        }
                        if let Some(b_dec) = b_nb.as_decimal() {
                            if let Some(a_int) = a_nb.as_i64() {
                                return self.push_vw(ValueWord::from_decimal(
                                    rust_decimal::Decimal::from(a_int) + b_dec,
                                ));
                            }
                            if let Some(a_num) = a_nb.as_f64() {
                                use rust_decimal::prelude::FromPrimitive;
                                let a_dec =
                                    rust_decimal::Decimal::from_f64(a_num).unwrap_or_default();
                                return self.push_vw(ValueWord::from_decimal(a_dec + b_dec));
                            }
                        }
                        // Operator trait fallback (Add)
                        if let Some(result) =
                            self.try_binary_operator_trait(a_nb.clone(), b_nb.clone(), "add")?
                        {
                            return self.push_vw(result);
                        }
                        return Err(VMError::RuntimeError(format!(
                            "Cannot apply '+' to {} and {}",
                            a_nb.type_name(),
                            b_nb.type_name()
                        )));
                    }
                    // Neither numeric nor heap: bool+bool, null, etc.
                    _ => {
                        // Operator trait fallback (Add)
                        if let Some(result) =
                            self.try_binary_operator_trait(a_nb.clone(), b_nb.clone(), "add")?
                        {
                            return self.push_vw(result);
                        }
                        return Err(VMError::RuntimeError(format!(
                            "Cannot apply '+' to {} and {}",
                            a_nb.type_name(),
                            b_nb.type_name()
                        )));
                    }
                }
            }
            Sub => {
                use shape_value::NanTag;
                // IC fast path for Sub
                {
                    use crate::executor::ic_fast_paths::{ArithmeticIcHint, arithmetic_ic_check};
                    let hint = arithmetic_ic_check(self, self.ip);
                    if hint == ArithmeticIcHint::BothI48 && self.sp >= 2 {
                        let b = &self.stack[self.sp - 1];
                        let a = &self.stack[self.sp - 2];
                        if a.is_i64() && b.is_i64() {
                            let result = unsafe { ValueWord::sub_i64(a, b) };
                            self.sp -= 2;
                            let ip = self.ip;
                            if let Some(fv) = self.current_feedback_vector() {
                                fv.record_arithmetic(ip, NanTag::I48 as u8, NanTag::I48 as u8);
                            }
                            return self.push_vw(result);
                        }
                    } else if hint == ArithmeticIcHint::BothF64 && self.sp >= 2 {
                        let b = &self.stack[self.sp - 1];
                        let a = &self.stack[self.sp - 2];
                        if let (Some(af), Some(bf)) = (a.as_f64(), b.as_f64()) {
                            self.sp -= 2;
                            let ip = self.ip;
                            if let Some(fv) = self.current_feedback_vector() {
                                fv.record_arithmetic(ip, NanTag::F64 as u8, NanTag::F64 as u8);
                            }
                            return self.push_vw(ValueWord::from_f64(af - bf));
                        }
                    }
                }
                let b_nb = unwrap_annotated(self.pop_vw()?);
                let a_nb = unwrap_annotated(self.pop_vw()?);
                // Record operand types for IC profiling.
                {
                    let ip = self.ip;
                    if let Some(fv) = self.current_feedback_vector() {
                        fv.record_arithmetic(ip, a_nb.tag() as u8, b_nb.tag() as u8);
                    }
                }
                if let Some(result) = Self::numeric_binary_result(
                    &a_nb,
                    &b_nb,
                    "-",
                    |a, b| a.checked_sub(b),
                    |a, b| a - b,
                )? {
                    return self.push_vw(result);
                }
                match (a_nb.tag(), b_nb.tag()) {
                    (NanTag::I48 | NanTag::F64, NanTag::I48 | NanTag::F64) => {
                        if let (Some(a_num), Some(b_num)) =
                            (a_nb.as_number_coerce(), b_nb.as_number_coerce())
                        {
                            return self.push_vw(ValueWord::binary_int_preserving(
                                &a_nb,
                                &b_nb,
                                a_num,
                                b_num,
                                |a, b| a.checked_sub(b),
                                |a, b| a - b,
                            ));
                        }
                    }
                    (NanTag::Heap, NanTag::Heap) => {
                        match (a_nb.as_heap_ref().unwrap(), b_nb.as_heap_ref().unwrap()) {
                            (HeapValue::BigInt(a_big), HeapValue::BigInt(b_big)) => {
                                return self.push_vw(ValueWord::from_i64(
                                    a_big.checked_sub(*b_big).ok_or_else(|| {
                                        VMError::RuntimeError("Integer overflow".into())
                                    })?,
                                ));
                            }
                            (HeapValue::Decimal(a_dec), HeapValue::Decimal(b_dec)) => {
                                return self.push_vw(ValueWord::from_decimal(*a_dec - *b_dec));
                            }
                            // Time - Time => TimeSpan (duration between two instants)
                            (HeapValue::Time(a_dt), HeapValue::Time(b_dt)) => {
                                let diff = *a_dt - *b_dt;
                                return self.push_vw(ValueWord::from_timespan(diff));
                            }
                            // Time - TimeSpan => Time
                            (HeapValue::Time(dt), HeapValue::TimeSpan(dur)) => {
                                let result = dt.checked_sub_signed(*dur).ok_or_else(|| {
                                    VMError::RuntimeError(
                                        "DateTime overflow in subtraction".to_string(),
                                    )
                                })?;
                                return self.push_vw(ValueWord::from_time(result));
                            }
                            // TimeSpan - TimeSpan => TimeSpan
                            (HeapValue::TimeSpan(a_dur), HeapValue::TimeSpan(b_dur)) => {
                                let result = a_dur.checked_sub(b_dur).ok_or_else(|| {
                                    VMError::RuntimeError(
                                        "Duration overflow in subtraction".to_string(),
                                    )
                                })?;
                                return self.push_vw(ValueWord::from_timespan(result));
                            }
                            // Vec<number> - Vec<number>
                            (HeapValue::FloatArray(a_arr), HeapValue::FloatArray(b_arr)) => {
                                if a_arr.len() != b_arr.len() {
                                    return Err(VMError::RuntimeError(format!(
                                        "Vec<number> length mismatch: {} vs {}",
                                        a_arr.len(),
                                        b_arr.len()
                                    )));
                                }
                                let result = shape_runtime::intrinsics::vector::simd_vec_sub_f64(
                                    a_arr.as_slice(),
                                    b_arr.as_slice(),
                                );
                                return self
                                    .push_vw(ValueWord::from_float_array(Arc::new(result.into())));
                            }
                            // Vec<int> - Vec<int>
                            (HeapValue::IntArray(a_arr), HeapValue::IntArray(b_arr)) => {
                                if a_arr.len() != b_arr.len() {
                                    return Err(VMError::RuntimeError(format!(
                                        "Vec<int> length mismatch: {} vs {}",
                                        a_arr.len(),
                                        b_arr.len()
                                    )));
                                }
                                match shape_runtime::intrinsics::vector::simd_vec_sub_i64(
                                    a_arr.as_slice(),
                                    b_arr.as_slice(),
                                ) {
                                    Ok(result) => {
                                        return self.push_vw(ValueWord::from_int_array(Arc::new(
                                            result.into(),
                                        )));
                                    }
                                    Err(()) => {
                                        return Err(VMError::RuntimeError(
                                            "Integer overflow in Vec<int> element-wise subtraction"
                                                .into(),
                                        ));
                                    }
                                }
                            }
                            // Vec<int> - Vec<number> / Vec<number> - Vec<int>
                            (HeapValue::IntArray(a_arr), HeapValue::FloatArray(b_arr)) => {
                                if a_arr.len() != b_arr.len() {
                                    return Err(VMError::RuntimeError(format!(
                                        "Vec length mismatch: {} vs {}",
                                        a_arr.len(),
                                        b_arr.len()
                                    )));
                                }
                                let a_f64 = shape_runtime::intrinsics::vector::i64_slice_to_f64(
                                    a_arr.as_slice(),
                                );
                                let result = shape_runtime::intrinsics::vector::simd_vec_sub_f64(
                                    &a_f64,
                                    b_arr.as_slice(),
                                );
                                return self
                                    .push_vw(ValueWord::from_float_array(Arc::new(result.into())));
                            }
                            (HeapValue::FloatArray(a_arr), HeapValue::IntArray(b_arr)) => {
                                if a_arr.len() != b_arr.len() {
                                    return Err(VMError::RuntimeError(format!(
                                        "Vec length mismatch: {} vs {}",
                                        a_arr.len(),
                                        b_arr.len()
                                    )));
                                }
                                let b_f64 = shape_runtime::intrinsics::vector::i64_slice_to_f64(
                                    b_arr.as_slice(),
                                );
                                let result = shape_runtime::intrinsics::vector::simd_vec_sub_f64(
                                    a_arr.as_slice(),
                                    &b_f64,
                                );
                                return self
                                    .push_vw(ValueWord::from_float_array(Arc::new(result.into())));
                            }
                            // Matrix - Matrix => element-wise sub
                            (HeapValue::Matrix(a_mat), HeapValue::Matrix(b_mat)) => {
                                let result = shape_runtime::intrinsics::matrix_kernels::matrix_sub(
                                    a_mat, b_mat,
                                )
                                .map_err(|e| VMError::RuntimeError(e))?;
                                return self.push_vw(ValueWord::from_matrix(Box::new(result)));
                            }
                            _ => {}
                        }
                    }
                    (NanTag::Heap, _) => {
                        if let Some(HeapValue::BigInt(a_big)) = a_nb.as_heap_ref() {
                            if let Some(b_i) = b_nb.as_i64() {
                                return self.push_vw(ValueWord::from_i64(
                                    a_big.checked_sub(b_i).ok_or_else(|| {
                                        VMError::RuntimeError("Integer overflow".into())
                                    })?,
                                ));
                            }
                            if let Some(b_f) = b_nb.as_f64() {
                                return self.push_vw(ValueWord::from_f64(*a_big as f64 - b_f));
                            }
                        }
                        if let Some(a_dec) = a_nb.as_decimal() {
                            if let Some(b_int) = b_nb.as_i64() {
                                return self.push_vw(ValueWord::from_decimal(
                                    a_dec - rust_decimal::Decimal::from(b_int),
                                ));
                            }
                            if let Some(b_num) = b_nb.as_f64() {
                                use rust_decimal::prelude::FromPrimitive;
                                return self.push_vw(ValueWord::from_decimal(
                                    a_dec
                                        - rust_decimal::Decimal::from_f64(b_num)
                                            .unwrap_or_default(),
                                ));
                            }
                        }
                    }
                    (_, NanTag::Heap) => {
                        if let Some(HeapValue::BigInt(b_big)) = b_nb.as_heap_ref() {
                            if let Some(a_i) = a_nb.as_i64() {
                                return self.push_vw(ValueWord::from_i64(
                                    a_i.checked_sub(*b_big).ok_or_else(|| {
                                        VMError::RuntimeError("Integer overflow".into())
                                    })?,
                                ));
                            }
                            if let Some(a_f) = a_nb.as_f64() {
                                return self.push_vw(ValueWord::from_f64(a_f - *b_big as f64));
                            }
                        }
                        if let Some(b_dec) = b_nb.as_decimal() {
                            if let Some(a_int) = a_nb.as_i64() {
                                return self.push_vw(ValueWord::from_decimal(
                                    rust_decimal::Decimal::from(a_int) - b_dec,
                                ));
                            }
                            if let Some(a_num) = a_nb.as_f64() {
                                use rust_decimal::prelude::FromPrimitive;
                                return self.push_vw(ValueWord::from_decimal(
                                    rust_decimal::Decimal::from_f64(a_num).unwrap_or_default()
                                        - b_dec,
                                ));
                            }
                        }
                    }
                    _ => {}
                }
                // Operator trait fallback (Sub)
                if let Some(result) =
                    self.try_binary_operator_trait(a_nb.clone(), b_nb.clone(), "sub")?
                {
                    return self.push_vw(result);
                }
                return Err(VMError::RuntimeError(format!(
                    "Cannot apply '-' to {} and {}",
                    a_nb.type_name(),
                    b_nb.type_name()
                )));
            }
            Mul => {
                use shape_value::NanTag;
                // IC fast path for Mul
                {
                    use crate::executor::ic_fast_paths::{ArithmeticIcHint, arithmetic_ic_check};
                    let hint = arithmetic_ic_check(self, self.ip);
                    if hint == ArithmeticIcHint::BothI48 && self.sp >= 2 {
                        let b = &self.stack[self.sp - 1];
                        let a = &self.stack[self.sp - 2];
                        if a.is_i64() && b.is_i64() {
                            let result = unsafe { ValueWord::mul_i64(a, b) };
                            self.sp -= 2;
                            let ip = self.ip;
                            if let Some(fv) = self.current_feedback_vector() {
                                fv.record_arithmetic(ip, NanTag::I48 as u8, NanTag::I48 as u8);
                            }
                            return self.push_vw(result);
                        }
                    } else if hint == ArithmeticIcHint::BothF64 && self.sp >= 2 {
                        let b = &self.stack[self.sp - 1];
                        let a = &self.stack[self.sp - 2];
                        if let (Some(af), Some(bf)) = (a.as_f64(), b.as_f64()) {
                            self.sp -= 2;
                            let ip = self.ip;
                            if let Some(fv) = self.current_feedback_vector() {
                                fv.record_arithmetic(ip, NanTag::F64 as u8, NanTag::F64 as u8);
                            }
                            return self.push_vw(ValueWord::from_f64(af * bf));
                        }
                    }
                }
                let b_nb = unwrap_annotated(self.pop_vw()?);
                let a_nb = unwrap_annotated(self.pop_vw()?);
                // Record operand types for IC profiling.
                {
                    let ip = self.ip;
                    if let Some(fv) = self.current_feedback_vector() {
                        fv.record_arithmetic(ip, a_nb.tag() as u8, b_nb.tag() as u8);
                    }
                }
                if let Some(result) = Self::numeric_binary_result(
                    &a_nb,
                    &b_nb,
                    "*",
                    |a, b| a.checked_mul(b),
                    |a, b| a * b,
                )? {
                    return self.push_vw(result);
                }
                match (a_nb.tag(), b_nb.tag()) {
                    (NanTag::I48 | NanTag::F64, NanTag::I48 | NanTag::F64) => {
                        if let (Some(a_num), Some(b_num)) =
                            (a_nb.as_number_coerce(), b_nb.as_number_coerce())
                        {
                            return self.push_vw(ValueWord::binary_int_preserving(
                                &a_nb,
                                &b_nb,
                                a_num,
                                b_num,
                                |a, b| a.checked_mul(b),
                                |a, b| a * b,
                            ));
                        }
                    }
                    (NanTag::Heap, NanTag::Heap) => {
                        match (a_nb.as_heap_ref().unwrap(), b_nb.as_heap_ref().unwrap()) {
                            (HeapValue::BigInt(a_big), HeapValue::BigInt(b_big)) => {
                                return self.push_vw(ValueWord::from_i64(
                                    a_big.checked_mul(*b_big).ok_or_else(|| {
                                        VMError::RuntimeError("Integer overflow".into())
                                    })?,
                                ));
                            }
                            (HeapValue::Decimal(a_dec), HeapValue::Decimal(b_dec)) => {
                                return self.push_vw(ValueWord::from_decimal(*a_dec * *b_dec));
                            }
                            // Vec<number> * Vec<number>
                            (HeapValue::FloatArray(a_arr), HeapValue::FloatArray(b_arr)) => {
                                if a_arr.len() != b_arr.len() {
                                    return Err(VMError::RuntimeError(format!(
                                        "Vec<number> length mismatch: {} vs {}",
                                        a_arr.len(),
                                        b_arr.len()
                                    )));
                                }
                                let result = shape_runtime::intrinsics::vector::simd_vec_mul_f64(
                                    a_arr.as_slice(),
                                    b_arr.as_slice(),
                                );
                                return self
                                    .push_vw(ValueWord::from_float_array(Arc::new(result.into())));
                            }
                            // Vec<int> * Vec<int>
                            (HeapValue::IntArray(a_arr), HeapValue::IntArray(b_arr)) => {
                                if a_arr.len() != b_arr.len() {
                                    return Err(VMError::RuntimeError(format!(
                                        "Vec<int> length mismatch: {} vs {}",
                                        a_arr.len(),
                                        b_arr.len()
                                    )));
                                }
                                match shape_runtime::intrinsics::vector::simd_vec_mul_i64(
                                    a_arr.as_slice(),
                                    b_arr.as_slice(),
                                ) {
                                    Ok(result) => {
                                        return self.push_vw(ValueWord::from_int_array(Arc::new(
                                            result.into(),
                                        )));
                                    }
                                    Err(()) => return Err(VMError::RuntimeError(
                                        "Integer overflow in Vec<int> element-wise multiplication"
                                            .into(),
                                    )),
                                }
                            }
                            // Mixed int/float
                            (HeapValue::IntArray(a_arr), HeapValue::FloatArray(b_arr)) => {
                                if a_arr.len() != b_arr.len() {
                                    return Err(VMError::RuntimeError(format!(
                                        "Vec length mismatch: {} vs {}",
                                        a_arr.len(),
                                        b_arr.len()
                                    )));
                                }
                                let a_f64 = shape_runtime::intrinsics::vector::i64_slice_to_f64(
                                    a_arr.as_slice(),
                                );
                                let result = shape_runtime::intrinsics::vector::simd_vec_mul_f64(
                                    &a_f64,
                                    b_arr.as_slice(),
                                );
                                return self
                                    .push_vw(ValueWord::from_float_array(Arc::new(result.into())));
                            }
                            (HeapValue::FloatArray(a_arr), HeapValue::IntArray(b_arr)) => {
                                if a_arr.len() != b_arr.len() {
                                    return Err(VMError::RuntimeError(format!(
                                        "Vec length mismatch: {} vs {}",
                                        a_arr.len(),
                                        b_arr.len()
                                    )));
                                }
                                let b_f64 = shape_runtime::intrinsics::vector::i64_slice_to_f64(
                                    b_arr.as_slice(),
                                );
                                let result = shape_runtime::intrinsics::vector::simd_vec_mul_f64(
                                    a_arr.as_slice(),
                                    &b_f64,
                                );
                                return self
                                    .push_vw(ValueWord::from_float_array(Arc::new(result.into())));
                            }
                            // Matrix * Matrix => matmul
                            (HeapValue::Matrix(a_mat), HeapValue::Matrix(b_mat)) => {
                                let result =
                                    shape_runtime::intrinsics::matrix_kernels::matrix_matmul(
                                        a_mat, b_mat,
                                    )
                                    .map_err(|e| VMError::RuntimeError(e))?;
                                return self.push_vw(ValueWord::from_matrix(Box::new(result)));
                            }
                            // Matrix * FloatArray => matvec
                            (HeapValue::Matrix(mat), HeapValue::FloatArray(vec_data)) => {
                                let result =
                                    shape_runtime::intrinsics::matrix_kernels::matrix_matvec(
                                        mat,
                                        vec_data.as_slice(),
                                    )
                                    .map_err(|e| VMError::RuntimeError(e))?;
                                return self
                                    .push_vw(ValueWord::from_float_array(Arc::new(result.into())));
                            }
                            _ => {}
                        }
                    }
                    (NanTag::Heap, _) => {
                        // Vec<number> * scalar => broadcast scale
                        if let Some(a_arr) = a_nb.as_float_array() {
                            if let Some(scalar) = b_nb.as_number_coerce() {
                                let result = shape_runtime::intrinsics::vector::simd_vec_scale_f64(
                                    a_arr.as_slice(),
                                    scalar,
                                );
                                return self
                                    .push_vw(ValueWord::from_float_array(Arc::new(result.into())));
                            }
                        }
                        // Matrix * scalar => element-wise scale
                        if let Some(a_mat) = a_nb.as_matrix() {
                            if let Some(scalar) = b_nb.as_number_coerce() {
                                let result =
                                    shape_runtime::intrinsics::matrix_kernels::matrix_scale(
                                        a_mat, scalar,
                                    );
                                return self.push_vw(ValueWord::from_matrix(Box::new(result)));
                            }
                        }
                        if let Some(HeapValue::BigInt(a_big)) = a_nb.as_heap_ref() {
                            if let Some(b_i) = b_nb.as_i64() {
                                return self.push_vw(ValueWord::from_i64(
                                    a_big.checked_mul(b_i).ok_or_else(|| {
                                        VMError::RuntimeError("Integer overflow".into())
                                    })?,
                                ));
                            }
                            if let Some(b_f) = b_nb.as_f64() {
                                return self.push_vw(ValueWord::from_f64(*a_big as f64 * b_f));
                            }
                        }
                        if let Some(a_dec) = a_nb.as_decimal() {
                            if let Some(b_int) = b_nb.as_i64() {
                                return self.push_vw(ValueWord::from_decimal(
                                    a_dec * rust_decimal::Decimal::from(b_int),
                                ));
                            }
                            if let Some(b_num) = b_nb.as_f64() {
                                use rust_decimal::prelude::FromPrimitive;
                                return self.push_vw(ValueWord::from_decimal(
                                    a_dec
                                        * rust_decimal::Decimal::from_f64(b_num)
                                            .unwrap_or_default(),
                                ));
                            }
                        }
                    }
                    (_, NanTag::Heap) => {
                        // scalar * Vec<number> => broadcast scale
                        if let Some(b_arr) = b_nb.as_float_array() {
                            if let Some(scalar) = a_nb.as_number_coerce() {
                                let result = shape_runtime::intrinsics::vector::simd_vec_scale_f64(
                                    b_arr.as_slice(),
                                    scalar,
                                );
                                return self
                                    .push_vw(ValueWord::from_float_array(Arc::new(result.into())));
                            }
                        }
                        // scalar * Matrix => element-wise scale
                        if let Some(b_mat) = b_nb.as_matrix() {
                            if let Some(scalar) = a_nb.as_number_coerce() {
                                let result =
                                    shape_runtime::intrinsics::matrix_kernels::matrix_scale(
                                        b_mat, scalar,
                                    );
                                return self.push_vw(ValueWord::from_matrix(Box::new(result)));
                            }
                        }
                        if let Some(HeapValue::BigInt(b_big)) = b_nb.as_heap_ref() {
                            if let Some(a_i) = a_nb.as_i64() {
                                return self.push_vw(ValueWord::from_i64(
                                    a_i.checked_mul(*b_big).ok_or_else(|| {
                                        VMError::RuntimeError("Integer overflow".into())
                                    })?,
                                ));
                            }
                            if let Some(a_f) = a_nb.as_f64() {
                                return self.push_vw(ValueWord::from_f64(a_f * *b_big as f64));
                            }
                        }
                        if let Some(b_dec) = b_nb.as_decimal() {
                            if let Some(a_int) = a_nb.as_i64() {
                                return self.push_vw(ValueWord::from_decimal(
                                    rust_decimal::Decimal::from(a_int) * b_dec,
                                ));
                            }
                            if let Some(a_num) = a_nb.as_f64() {
                                use rust_decimal::prelude::FromPrimitive;
                                return self.push_vw(ValueWord::from_decimal(
                                    rust_decimal::Decimal::from_f64(a_num).unwrap_or_default()
                                        * b_dec,
                                ));
                            }
                        }
                    }
                    _ => {}
                }
                // Operator trait fallback (Mul)
                if let Some(result) =
                    self.try_binary_operator_trait(a_nb.clone(), b_nb.clone(), "mul")?
                {
                    return self.push_vw(result);
                }
                return Err(VMError::RuntimeError(format!(
                    "Cannot apply '*' to {} and {}",
                    a_nb.type_name(),
                    b_nb.type_name()
                )));
            }
            Div => {
                use shape_value::NanTag;
                // IC fast path for Div
                {
                    use crate::executor::ic_fast_paths::{ArithmeticIcHint, arithmetic_ic_check};
                    let hint = arithmetic_ic_check(self, self.ip);
                    if hint == ArithmeticIcHint::BothI48 && self.sp >= 2 {
                        let b = &self.stack[self.sp - 1];
                        let a = &self.stack[self.sp - 2];
                        if let (Some(ai), Some(bi)) = (Self::int_operand(a), Self::int_operand(b)) {
                            if bi == 0 {
                                return Err(VMError::DivisionByZero);
                            }
                            self.sp -= 2;
                            let ip = self.ip;
                            if let Some(fv) = self.current_feedback_vector() {
                                fv.record_arithmetic(ip, NanTag::I48 as u8, NanTag::I48 as u8);
                            }
                            return self.push_vw(ValueWord::from_i64(ai / bi));
                        }
                    } else if hint == ArithmeticIcHint::BothF64 && self.sp >= 2 {
                        let b = &self.stack[self.sp - 1];
                        let a = &self.stack[self.sp - 2];
                        if let (Some(af), Some(bf)) = (a.as_f64(), b.as_f64()) {
                            if bf == 0.0 {
                                return Err(VMError::DivisionByZero);
                            }
                            self.sp -= 2;
                            let ip = self.ip;
                            if let Some(fv) = self.current_feedback_vector() {
                                fv.record_arithmetic(ip, NanTag::F64 as u8, NanTag::F64 as u8);
                            }
                            return self.push_vw(ValueWord::from_f64(af / bf));
                        }
                    }
                }
                let b_nb = unwrap_annotated(self.pop_vw()?);
                let a_nb = unwrap_annotated(self.pop_vw()?);
                // Record operand types for IC profiling.
                {
                    let ip = self.ip;
                    if let Some(fv) = self.current_feedback_vector() {
                        fv.record_arithmetic(ip, a_nb.tag() as u8, b_nb.tag() as u8);
                    }
                }
                if let Some(result) = Self::numeric_div_result(&a_nb, &b_nb)? {
                    return self.push_vw(result);
                }
                match (a_nb.tag(), b_nb.tag()) {
                    (NanTag::I48 | NanTag::F64, NanTag::I48 | NanTag::F64) => {
                        if let (Some(a_num), Some(b_num)) =
                            (a_nb.as_number_coerce(), b_nb.as_number_coerce())
                        {
                            if b_num == 0.0 {
                                return Err(VMError::DivisionByZero);
                            }
                            return self.push_vw(ValueWord::binary_int_preserving(
                                &a_nb,
                                &b_nb,
                                a_num,
                                b_num,
                                |a, b| a.checked_div(b),
                                |a, b| a / b,
                            ));
                        }
                    }
                    (NanTag::Heap, NanTag::Heap) => {
                        match (a_nb.as_heap_ref().unwrap(), b_nb.as_heap_ref().unwrap()) {
                            (HeapValue::BigInt(a_big), HeapValue::BigInt(b_big)) => {
                                if *b_big == 0 {
                                    return Err(VMError::DivisionByZero);
                                }
                                return self.push_vw(ValueWord::from_i64(
                                    a_big.checked_div(*b_big).ok_or_else(|| {
                                        VMError::RuntimeError("Integer overflow".into())
                                    })?,
                                ));
                            }
                            (HeapValue::Decimal(a_dec), HeapValue::Decimal(b_dec)) => {
                                if b_dec.is_zero() {
                                    return Err(VMError::DivisionByZero);
                                }
                                return self.push_vw(ValueWord::from_decimal(*a_dec / *b_dec));
                            }
                            // Vec<number> / Vec<number>
                            (HeapValue::FloatArray(a_arr), HeapValue::FloatArray(b_arr)) => {
                                if a_arr.len() != b_arr.len() {
                                    return Err(VMError::RuntimeError(format!(
                                        "Vec<number> length mismatch: {} vs {}",
                                        a_arr.len(),
                                        b_arr.len()
                                    )));
                                }
                                let result = shape_runtime::intrinsics::vector::simd_vec_div_f64(
                                    a_arr.as_slice(),
                                    b_arr.as_slice(),
                                );
                                return self
                                    .push_vw(ValueWord::from_float_array(Arc::new(result.into())));
                            }
                            // Vec<int> / Vec<int>
                            (HeapValue::IntArray(a_arr), HeapValue::IntArray(b_arr)) => {
                                if a_arr.len() != b_arr.len() {
                                    return Err(VMError::RuntimeError(format!(
                                        "Vec<int> length mismatch: {} vs {}",
                                        a_arr.len(),
                                        b_arr.len()
                                    )));
                                }
                                match shape_runtime::intrinsics::vector::simd_vec_div_i64(a_arr.as_slice(), b_arr.as_slice()) {
                                    Ok(result) => return self.push_vw(ValueWord::from_int_array(Arc::new(result.into()))),
                                    Err(()) => return Err(VMError::RuntimeError(
                                        "Division by zero or overflow in Vec<int> element-wise division".into()
                                    )),
                                }
                            }
                            // Mixed int/float
                            (HeapValue::IntArray(a_arr), HeapValue::FloatArray(b_arr)) => {
                                if a_arr.len() != b_arr.len() {
                                    return Err(VMError::RuntimeError(format!(
                                        "Vec length mismatch: {} vs {}",
                                        a_arr.len(),
                                        b_arr.len()
                                    )));
                                }
                                let a_f64 = shape_runtime::intrinsics::vector::i64_slice_to_f64(
                                    a_arr.as_slice(),
                                );
                                let result = shape_runtime::intrinsics::vector::simd_vec_div_f64(
                                    &a_f64,
                                    b_arr.as_slice(),
                                );
                                return self
                                    .push_vw(ValueWord::from_float_array(Arc::new(result.into())));
                            }
                            (HeapValue::FloatArray(a_arr), HeapValue::IntArray(b_arr)) => {
                                if a_arr.len() != b_arr.len() {
                                    return Err(VMError::RuntimeError(format!(
                                        "Vec length mismatch: {} vs {}",
                                        a_arr.len(),
                                        b_arr.len()
                                    )));
                                }
                                let b_f64 = shape_runtime::intrinsics::vector::i64_slice_to_f64(
                                    b_arr.as_slice(),
                                );
                                let result = shape_runtime::intrinsics::vector::simd_vec_div_f64(
                                    a_arr.as_slice(),
                                    &b_f64,
                                );
                                return self
                                    .push_vw(ValueWord::from_float_array(Arc::new(result.into())));
                            }
                            _ => {}
                        }
                    }
                    (NanTag::Heap, _) => {
                        // Vec<number> / scalar => broadcast divide
                        if let Some(a_arr) = a_nb.as_float_array() {
                            if let Some(scalar) = b_nb.as_number_coerce() {
                                let result = shape_runtime::intrinsics::vector::simd_vec_scale_f64(
                                    a_arr.as_slice(),
                                    1.0 / scalar,
                                );
                                return self
                                    .push_vw(ValueWord::from_float_array(Arc::new(result.into())));
                            }
                        }
                        if let Some(HeapValue::BigInt(a_big)) = a_nb.as_heap_ref() {
                            if let Some(b_i) = b_nb.as_i64() {
                                if b_i == 0 {
                                    return Err(VMError::DivisionByZero);
                                }
                                return self.push_vw(ValueWord::from_i64(
                                    a_big.checked_div(b_i).ok_or_else(|| {
                                        VMError::RuntimeError("Integer overflow".into())
                                    })?,
                                ));
                            }
                            if let Some(b_f) = b_nb.as_f64() {
                                if b_f == 0.0 {
                                    return Err(VMError::DivisionByZero);
                                }
                                return self.push_vw(ValueWord::from_f64(*a_big as f64 / b_f));
                            }
                        }
                        if let Some(a_dec) = a_nb.as_decimal() {
                            if let Some(b_int) = b_nb.as_i64() {
                                if b_int == 0 {
                                    return Err(VMError::DivisionByZero);
                                }
                                return self.push_vw(ValueWord::from_decimal(
                                    a_dec / rust_decimal::Decimal::from(b_int),
                                ));
                            }
                            if let Some(b_num) = b_nb.as_f64() {
                                if b_num == 0.0 {
                                    return Err(VMError::DivisionByZero);
                                }
                                use rust_decimal::prelude::FromPrimitive;
                                return self.push_vw(ValueWord::from_decimal(
                                    a_dec
                                        / rust_decimal::Decimal::from_f64(b_num)
                                            .unwrap_or_default(),
                                ));
                            }
                        }
                    }
                    (_, NanTag::Heap) => {
                        if let Some(HeapValue::BigInt(b_big)) = b_nb.as_heap_ref() {
                            if *b_big == 0 {
                                return Err(VMError::DivisionByZero);
                            }
                            if let Some(a_i) = a_nb.as_i64() {
                                return self.push_vw(ValueWord::from_i64(
                                    a_i.checked_div(*b_big).ok_or_else(|| {
                                        VMError::RuntimeError("Integer overflow".into())
                                    })?,
                                ));
                            }
                            if let Some(a_f) = a_nb.as_f64() {
                                return self.push_vw(ValueWord::from_f64(a_f / *b_big as f64));
                            }
                        }
                        if let Some(b_dec) = b_nb.as_decimal() {
                            if b_dec.is_zero() {
                                return Err(VMError::DivisionByZero);
                            }
                            if let Some(a_int) = a_nb.as_i64() {
                                return self.push_vw(ValueWord::from_decimal(
                                    rust_decimal::Decimal::from(a_int) / b_dec,
                                ));
                            }
                            if let Some(a_num) = a_nb.as_f64() {
                                use rust_decimal::prelude::FromPrimitive;
                                return self.push_vw(ValueWord::from_decimal(
                                    rust_decimal::Decimal::from_f64(a_num).unwrap_or_default()
                                        / b_dec,
                                ));
                            }
                        }
                    }
                    _ => {}
                }
                // Operator trait fallback (Div)
                if let Some(result) =
                    self.try_binary_operator_trait(a_nb.clone(), b_nb.clone(), "div")?
                {
                    return self.push_vw(result);
                }
                return Err(VMError::RuntimeError(format!(
                    "Cannot apply '/' to {} and {}",
                    a_nb.type_name(),
                    b_nb.type_name()
                )));
            }
            Mod => {
                use shape_value::NanTag;
                let b_nb = unwrap_annotated(self.pop_vw()?);
                let a_nb = unwrap_annotated(self.pop_vw()?);
                if let Some(result) = Self::numeric_mod_result(&a_nb, &b_nb)? {
                    return self.push_vw(result);
                }
                match (a_nb.tag(), b_nb.tag()) {
                    (NanTag::I48 | NanTag::F64, NanTag::I48 | NanTag::F64) => {
                        if let (Some(a_num), Some(b_num)) =
                            (a_nb.as_number_coerce(), b_nb.as_number_coerce())
                        {
                            if b_num == 0.0 {
                                return Err(VMError::DivisionByZero);
                            }
                            return self.push_vw(ValueWord::binary_int_preserving(
                                &a_nb,
                                &b_nb,
                                a_num,
                                b_num,
                                |a, b| a.checked_rem(b),
                                |a, b| a % b,
                            ));
                        }
                    }
                    (NanTag::Heap, NanTag::Heap) => {
                        match (a_nb.as_heap_ref().unwrap(), b_nb.as_heap_ref().unwrap()) {
                            (HeapValue::BigInt(a_big), HeapValue::BigInt(b_big)) => {
                                if *b_big == 0 {
                                    return Err(VMError::DivisionByZero);
                                }
                                return self.push_vw(ValueWord::from_i64(
                                    a_big.checked_rem(*b_big).ok_or_else(|| {
                                        VMError::RuntimeError("Integer overflow".into())
                                    })?,
                                ));
                            }
                            (HeapValue::Decimal(a_dec), HeapValue::Decimal(b_dec)) => {
                                if b_dec.is_zero() {
                                    return Err(VMError::DivisionByZero);
                                }
                                return self.push_vw(ValueWord::from_decimal(*a_dec % *b_dec));
                            }
                            _ => {}
                        }
                    }
                    (NanTag::Heap, _) => {
                        if let Some(HeapValue::BigInt(a_big)) = a_nb.as_heap_ref() {
                            if let Some(b_i) = b_nb.as_i64() {
                                if b_i == 0 {
                                    return Err(VMError::DivisionByZero);
                                }
                                return self.push_vw(ValueWord::from_i64(
                                    a_big.checked_rem(b_i).ok_or_else(|| {
                                        VMError::RuntimeError("Integer overflow".into())
                                    })?,
                                ));
                            }
                        }
                        if let Some(a_dec) = a_nb.as_decimal() {
                            if let Some(b_int) = b_nb.as_i64() {
                                let b_dec = rust_decimal::Decimal::from(b_int);
                                if b_dec.is_zero() {
                                    return Err(VMError::DivisionByZero);
                                }
                                return self.push_vw(ValueWord::from_decimal(a_dec % b_dec));
                            }
                            if let Some(b_num) = b_nb.as_f64() {
                                if b_num == 0.0 {
                                    return Err(VMError::DivisionByZero);
                                }
                                use rust_decimal::prelude::ToPrimitive;
                                return self.push_vw(ValueWord::from_f64(
                                    a_dec.to_f64().unwrap_or(f64::NAN) % b_num,
                                ));
                            }
                        }
                    }
                    (_, NanTag::Heap) => {
                        if let Some(HeapValue::BigInt(b_big)) = b_nb.as_heap_ref() {
                            if *b_big == 0 {
                                return Err(VMError::DivisionByZero);
                            }
                            if let Some(a_i) = a_nb.as_i64() {
                                return self.push_vw(ValueWord::from_i64(
                                    a_i.checked_rem(*b_big).ok_or_else(|| {
                                        VMError::RuntimeError("Integer overflow".into())
                                    })?,
                                ));
                            }
                        }
                        if let Some(b_dec) = b_nb.as_decimal() {
                            if b_dec.is_zero() {
                                return Err(VMError::DivisionByZero);
                            }
                            if let Some(a_int) = a_nb.as_i64() {
                                return self.push_vw(ValueWord::from_decimal(
                                    rust_decimal::Decimal::from(a_int) % b_dec,
                                ));
                            }
                            if let Some(a_num) = a_nb.as_f64() {
                                use rust_decimal::prelude::ToPrimitive;
                                let b = b_dec.to_f64().unwrap_or(f64::NAN);
                                if b == 0.0 {
                                    return Err(VMError::DivisionByZero);
                                }
                                return self.push_vw(ValueWord::from_f64(a_num % b));
                            }
                        }
                    }
                    _ => {}
                }
                return Err(VMError::RuntimeError(format!(
                    "Cannot apply '%' to {} and {}",
                    a_nb.type_name(),
                    b_nb.type_name()
                )));
            }
            Neg => {
                use shape_value::NanTag;
                let val_nb = unwrap_annotated(self.pop_vw()?);
                match val_nb.tag() {
                    NanTag::I48 => {
                        return self
                            .push_vw(ValueWord::from_i64(-unsafe { val_nb.as_i64_unchecked() }));
                    }
                    NanTag::F64 => {
                        return self
                            .push_vw(ValueWord::from_f64(-unsafe { val_nb.as_f64_unchecked() }));
                    }
                    NanTag::Heap => {
                        if let Some(HeapValue::BigInt(big)) = val_nb.as_heap_ref() {
                            return self.push_vw(ValueWord::from_i64(
                                big.checked_neg().ok_or_else(|| {
                                    VMError::RuntimeError("Integer overflow".into())
                                })?,
                            ));
                        }
                        if let Some(d) = val_nb.as_decimal() {
                            return self.push_vw(ValueWord::from_decimal(-d));
                        }
                    }
                    _ => {}
                }
                // Operator trait fallback (Neg)
                if let Some(result) = self.try_unary_operator_trait(val_nb.clone(), "neg")? {
                    return self.push_vw(result);
                }
                return Err(VMError::TypeError {
                    expected: "number",
                    got: val_nb.type_name(),
                });
            }
            Pow => {
                use shape_value::NanTag;
                let b_nb = unwrap_annotated(self.pop_vw()?);
                let a_nb = unwrap_annotated(self.pop_vw()?);
                if let Some(result) = Self::numeric_pow_result(&a_nb, &b_nb)? {
                    return self.push_vw(result);
                }
                match (a_nb.tag(), b_nb.tag()) {
                    (NanTag::I48, NanTag::I48) => {
                        let base = unsafe { a_nb.as_i64_unchecked() };
                        let exp = unsafe { b_nb.as_i64_unchecked() };
                        if exp >= 0 && exp < u32::MAX as i64 {
                            return self.push_vw(ValueWord::from_i64(base.pow(exp as u32)));
                        }
                        return self.push_vw(ValueWord::from_f64((base as f64).powf(exp as f64)));
                    }
                    (NanTag::I48 | NanTag::F64, NanTag::I48 | NanTag::F64) => {
                        if let (Some(a_num), Some(b_num)) =
                            (a_nb.as_number_coerce(), b_nb.as_number_coerce())
                        {
                            return self.push_vw(ValueWord::from_f64(a_num.powf(b_num)));
                        }
                    }
                    (NanTag::Heap, _) => {
                        if let Some(HeapValue::BigInt(a_big)) = a_nb.as_heap_ref() {
                            if let Some(b_i) = b_nb.as_i64() {
                                if b_i >= 0 && b_i < u32::MAX as i64 {
                                    return self.push_vw(ValueWord::from_i64(
                                        a_big.checked_pow(b_i as u32).ok_or_else(|| {
                                            VMError::RuntimeError("Integer overflow".into())
                                        })?,
                                    ));
                                }
                                return self.push_vw(ValueWord::from_f64(
                                    (*a_big as f64).powf(b_i as f64),
                                ));
                            }
                            if let Some(b_f) = b_nb.as_f64() {
                                return self
                                    .push_vw(ValueWord::from_f64((*a_big as f64).powf(b_f)));
                            }
                        }
                        if let Some(a_dec) = a_nb.as_decimal() {
                            use rust_decimal::prelude::FromPrimitive;
                            use rust_decimal::prelude::ToPrimitive;
                            let base = a_dec.to_f64().unwrap_or(0.0);
                            if let Some(b_dec) = b_nb.as_decimal() {
                                let result = base.powf(b_dec.to_f64().unwrap_or(0.0));
                                return self.push_vw(ValueWord::from_decimal(
                                    rust_decimal::Decimal::from_f64(result).unwrap_or_default(),
                                ));
                            }
                            if let Some(b_int) = b_nb.as_i64() {
                                let result = base.powf(b_int as f64);
                                return self.push_vw(ValueWord::from_decimal(
                                    rust_decimal::Decimal::from_f64(result).unwrap_or_default(),
                                ));
                            }
                            if let Some(b_num) = b_nb.as_f64() {
                                return self.push_vw(ValueWord::from_f64(base.powf(b_num)));
                            }
                        }
                    }
                    (_, NanTag::Heap) => {
                        if let Some(HeapValue::BigInt(b_big)) = b_nb.as_heap_ref() {
                            if let Some(a_i) = a_nb.as_i64() {
                                if *b_big >= 0 && *b_big < u32::MAX as i64 {
                                    return self.push_vw(ValueWord::from_i64(
                                        a_i.checked_pow(*b_big as u32).ok_or_else(|| {
                                            VMError::RuntimeError("Integer overflow".into())
                                        })?,
                                    ));
                                }
                                return self.push_vw(ValueWord::from_f64(
                                    (a_i as f64).powf(*b_big as f64),
                                ));
                            }
                            if let Some(a_f) = a_nb.as_f64() {
                                return self.push_vw(ValueWord::from_f64(a_f.powf(*b_big as f64)));
                            }
                        }
                        if let Some(b_dec) = b_nb.as_decimal() {
                            use rust_decimal::prelude::FromPrimitive;
                            use rust_decimal::prelude::ToPrimitive;
                            let exp = b_dec.to_f64().unwrap_or(0.0);
                            if let Some(a_int) = a_nb.as_i64() {
                                let result = (a_int as f64).powf(exp);
                                return self.push_vw(ValueWord::from_decimal(
                                    rust_decimal::Decimal::from_f64(result).unwrap_or_default(),
                                ));
                            }
                            if let Some(a_num) = a_nb.as_f64() {
                                return self.push_vw(ValueWord::from_f64(a_num.powf(exp)));
                            }
                        }
                    }
                    _ => {}
                }
                return Err(VMError::RuntimeError(format!(
                    "Cannot apply '**' to {} and {}",
                    a_nb.type_name(),
                    b_nb.type_name()
                )));
            }
            BitXor => {
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                match (a.as_i64(), b.as_i64()) {
                    (Some(a_int), Some(b_int)) => {
                        self.push_vw(ValueWord::from_i64(a_int ^ b_int))?
                    }
                    _ => {
                        return Err(VMError::RuntimeError(format!(
                            "Bitwise XOR requires integer operands, got {} and {}",
                            a.type_name(),
                            b.type_name()
                        )));
                    }
                }
            }
            BitAnd => {
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                match (a.as_i64(), b.as_i64()) {
                    (Some(a_int), Some(b_int)) => {
                        self.push_vw(ValueWord::from_i64(a_int & b_int))?
                    }
                    _ => {
                        return Err(VMError::RuntimeError(format!(
                            "Bitwise AND requires integer operands, got {} and {}",
                            a.type_name(),
                            b.type_name()
                        )));
                    }
                }
            }
            BitOr => {
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                match (a.as_i64(), b.as_i64()) {
                    (Some(a_int), Some(b_int)) => {
                        self.push_vw(ValueWord::from_i64(a_int | b_int))?
                    }
                    _ => {
                        return Err(VMError::RuntimeError(format!(
                            "Bitwise OR requires integer operands, got {} and {}",
                            a.type_name(),
                            b.type_name()
                        )));
                    }
                }
            }
            BitShl => {
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                match (a.as_i64(), b.as_i64()) {
                    (Some(a_int), Some(b_int)) => {
                        self.push_vw(ValueWord::from_i64(a_int << b_int))?
                    }
                    _ => {
                        return Err(VMError::RuntimeError(format!(
                            "Bitwise shift left requires integer operands, got {} and {}",
                            a.type_name(),
                            b.type_name()
                        )));
                    }
                }
            }
            BitShr => {
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                match (a.as_i64(), b.as_i64()) {
                    (Some(a_int), Some(b_int)) => {
                        self.push_vw(ValueWord::from_i64(a_int >> b_int))?
                    }
                    _ => {
                        return Err(VMError::RuntimeError(format!(
                            "Bitwise shift right requires integer operands, got {} and {}",
                            a.type_name(),
                            b.type_name()
                        )));
                    }
                }
            }
            BitNot => {
                let a = self.pop_vw()?;
                match a.as_i64() {
                    Some(a_int) => self.push_vw(ValueWord::from_i64(!a_int))?,
                    _ => {
                        return Err(VMError::RuntimeError(format!(
                            "Bitwise NOT requires integer operand, got {}",
                            a.type_name()
                        )));
                    }
                }
            }
            _ => unreachable!(
                "exec_arithmetic called with non-arithmetic opcode: {:?}",
                instruction.opcode
            ),
        }
        Ok(())
    }

    // ---------------------------------------------------------------
    // Operator trait fallback helpers
    // ---------------------------------------------------------------

    /// Get the user-facing type name for a value, resolving TypedObject schema names.
    fn operator_type_name(&self, val: &ValueWord) -> String {
        if let Some((schema_id, _, _)) = val.as_typed_object() {
            if let Some(schema) = self.lookup_schema(schema_id as u32) {
                return schema.name.clone();
            }
        }
        val.type_name().to_string()
    }

    /// Try to dispatch a binary operator via trait method.
    /// Looks up `TypeName::method_name` in the function name index and calls it.
    /// Returns Ok(Some(result)) on success, Ok(None) if no impl found.
    fn try_binary_operator_trait(
        &mut self,
        a: ValueWord,
        b: ValueWord,
        method_name: &str,
    ) -> Result<Option<ValueWord>, VMError> {
        let type_name = self.operator_type_name(&a);
        let fn_name = format!("{}::{}", type_name, method_name);
        let func_idx = match self.function_name_index.get(&fn_name) {
            Some(&idx) => idx,
            None => return Ok(None),
        };
        // Push args: self, other
        self.push_vw(a)?;
        self.push_vw(b)?;
        self.call_function_from_stack(func_idx, 2)?;
        let result = self.pop_vw()?;
        Ok(Some(result))
    }

    /// Try to dispatch a unary operator via trait method.
    /// Returns Ok(Some(result)) on success, Ok(None) if no impl found.
    fn try_unary_operator_trait(
        &mut self,
        val: ValueWord,
        method_name: &str,
    ) -> Result<Option<ValueWord>, VMError> {
        let type_name = self.operator_type_name(&val);
        let fn_name = format!("{}::{}", type_name, method_name);
        let func_idx = match self.function_name_index.get(&fn_name) {
            Some(&idx) => idx,
            None => return Ok(None),
        };
        self.push_vw(val)?;
        self.call_function_from_stack(func_idx, 1)?;
        let result = self.pop_vw()?;
        Ok(Some(result))
    }
}

#[cfg(test)]
mod tests {
    use crate::VMConfig;
    use crate::bytecode::*;
    use crate::executor::VirtualMachine;
    use shape_value::ValueWord;

    #[test]
    fn test_string_concatenation() {
        let mut program = BytecodeProgram::default();
        let c0 = program.add_constant(Constant::String("hello ".to_string()));
        let c1 = program.add_constant(Constant::String("world".to_string()));
        program.instructions = vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c0))),
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c1))),
            Instruction::simple(OpCode::Add),
            Instruction::simple(OpCode::Halt),
        ];

        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(program);
        let result = vm.execute(None).unwrap().clone();
        {
            let s = result.as_arc_string().expect("Expected String");
            assert_eq!(s.as_ref(), "hello world");
        }
    }

    #[test]
    fn test_print_string_interpolation() {
        use crate::compiler::BytecodeCompiler;
        use shape_ast::parser::parse_program;

        let code = r#"
            function test() {
                let i = 10
                print("value is {i}")
            }
        "#;
        let program = parse_program(code).unwrap();
        let bytecode = BytecodeCompiler::new().compile(&program).unwrap();

        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(bytecode);
        vm.enable_output_capture();

        let result = vm.execute_function_by_name("test", vec![], None);
        assert!(
            result.is_ok(),
            "Print with interpolation should work: {:?}",
            result.err()
        );
        let output = vm.get_captured_output();
        assert_eq!(output, vec!["value is 10"]);
    }

    #[test]
    fn test_formatted_string_literal_in_general_expression() {
        use crate::compiler::BytecodeCompiler;
        use shape_ast::parser::parse_program;

        let code = r#"
            function test() {
                let i = 10
                let s = f"value is {i}"
                return s
            }
        "#;
        let program = parse_program(code).unwrap();
        let bytecode = BytecodeCompiler::new().compile(&program).unwrap();

        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(bytecode);
        let result = vm
            .execute_function_by_name("test", vec![], None)
            .expect("execution should succeed")
            .clone();
        {
            let s = result.as_arc_string().expect("Expected String");
            assert_eq!(s.as_ref(), "value is 10");
        }
    }

    #[test]
    fn test_formatted_triple_string_literal_dedents_and_interpolates() {
        use crate::compiler::BytecodeCompiler;
        use shape_ast::parser::parse_program;

        let code = r#"
            function test() {
                let i = 10
                let s = f"""
                    value is {i}
                    done
                    """
                return s
            }
        "#;
        let program = parse_program(code).unwrap();
        let bytecode = BytecodeCompiler::new().compile(&program).unwrap();

        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(bytecode);
        let result = vm
            .execute_function_by_name("test", vec![], None)
            .expect("execution should succeed")
            .clone();
        {
            let s = result.as_arc_string().expect("Expected String");
            assert_eq!(s.as_ref(), "value is 10\ndone");
        }
    }

    #[test]
    fn test_formatted_triple_string_literal_preserves_relative_indentation() {
        use crate::compiler::BytecodeCompiler;
        use shape_ast::parser::parse_program;

        let code = r#"
            function test() {
                let s = f"""
                    value:
                      {33+1}
                    """
                return s
            }
        "#;
        let program = parse_program(code).unwrap();
        let bytecode = BytecodeCompiler::new().compile(&program).unwrap();

        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(bytecode);
        let result = vm
            .execute_function_by_name("test", vec![], None)
            .expect("execution should succeed")
            .clone();
        {
            let s = result.as_arc_string().expect("Expected String");
            assert_eq!(s.as_ref(), "value:\n  34");
        }
    }

    #[test]
    fn test_formatted_string_literal_with_fixed_precision_spec() {
        use crate::compiler::BytecodeCompiler;
        use shape_ast::parser::parse_program;

        let code = r#"
            function test() {
                let p = 12.3456
                let s = f"price={p:fixed(2)}"
                return s
            }
        "#;
        let program = parse_program(code).unwrap();
        let bytecode = BytecodeCompiler::new().compile(&program).unwrap();

        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(bytecode);
        let result = vm
            .execute_function_by_name("test", vec![], None)
            .expect("execution should succeed")
            .clone();
        {
            let s = result.as_arc_string().expect("Expected String");
            assert_eq!(s.as_ref(), "price=12.35");
        }
    }

    #[test]
    fn test_bitwise_xor() {
        let mut program = BytecodeProgram::default();
        let c0 = program.add_constant(Constant::Int(0xFF));
        let c1 = program.add_constant(Constant::Int(0x0F));
        program.instructions = vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c0))),
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c1))),
            Instruction::simple(OpCode::BitXor),
            Instruction::simple(OpCode::Halt),
        ];
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(program);
        let result = vm.execute(None).unwrap().clone();
        assert_eq!(result, ValueWord::from_i64(0xF0));
    }

    #[test]
    fn test_bitwise_and() {
        let mut program = BytecodeProgram::default();
        let c0 = program.add_constant(Constant::Int(0xFF));
        let c1 = program.add_constant(Constant::Int(0x0F));
        program.instructions = vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c0))),
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c1))),
            Instruction::simple(OpCode::BitAnd),
            Instruction::simple(OpCode::Halt),
        ];
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(program);
        let result = vm.execute(None).unwrap().clone();
        assert_eq!(result, ValueWord::from_i64(0x0F));
    }

    #[test]
    fn test_bitwise_or() {
        let mut program = BytecodeProgram::default();
        let c0 = program.add_constant(Constant::Int(0xF0));
        let c1 = program.add_constant(Constant::Int(0x0F));
        program.instructions = vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c0))),
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c1))),
            Instruction::simple(OpCode::BitOr),
            Instruction::simple(OpCode::Halt),
        ];
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(program);
        let result = vm.execute(None).unwrap().clone();
        assert_eq!(result, ValueWord::from_i64(0xFF));
    }

    #[test]
    fn test_bitwise_shift() {
        // 3 << 2 == 12
        let mut program = BytecodeProgram::default();
        let c0 = program.add_constant(Constant::Int(3));
        let c1 = program.add_constant(Constant::Int(2));
        program.instructions = vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c0))),
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c1))),
            Instruction::simple(OpCode::BitShl),
            Instruction::simple(OpCode::Halt),
        ];
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(program);
        let result = vm.execute(None).unwrap().clone();
        assert_eq!(result, ValueWord::from_i64(12));

        // 12 >> 2 == 3
        let mut program2 = BytecodeProgram::default();
        let c2 = program2.add_constant(Constant::Int(12));
        let c3 = program2.add_constant(Constant::Int(2));
        program2.instructions = vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c2))),
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c3))),
            Instruction::simple(OpCode::BitShr),
            Instruction::simple(OpCode::Halt),
        ];
        let mut vm2 = VirtualMachine::new(VMConfig::default());
        vm2.load_program(program2);
        let result2 = vm2.execute(None).unwrap().clone();
        assert_eq!(result2, ValueWord::from_i64(3));
    }

    #[test]
    fn test_bitwise_not() {
        let mut program = BytecodeProgram::default();
        let c0 = program.add_constant(Constant::Int(0));
        program.instructions = vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c0))),
            Instruction::simple(OpCode::BitNot),
            Instruction::simple(OpCode::Halt),
        ];
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(program);
        let result = vm.execute(None).unwrap().clone();
        assert_eq!(result, ValueWord::from_i64(-1));
    }

    #[test]
    fn test_numeric_binary_result_preserves_u64_domain() {
        let a = ValueWord::from_native_u64(u64::MAX - 1);
        let b = ValueWord::from_native_u64(1);
        let result = VirtualMachine::numeric_binary_result(
            &a,
            &b,
            "+",
            |x, y| x.checked_add(y),
            |x, y| x + y,
        )
        .expect("numeric operation should succeed")
        .expect("numeric operation should produce value");
        assert_eq!(result.as_u64(), Some(u64::MAX));
    }

    #[test]
    fn test_numeric_binary_result_rejects_lossy_u64_to_number_mix() {
        let a = ValueWord::from_native_u64(u64::MAX);
        let b = ValueWord::from_f64(1.0);
        let err = VirtualMachine::numeric_binary_result(
            &a,
            &b,
            "+",
            |x, y| x.checked_add(y),
            |x, y| x + y,
        )
        .expect_err("u64 + number should require explicit cast");
        assert!(format!("{err}").contains("explicit cast"));
    }

    // ---- Compact typed opcodes (AddTyped .. CmpTyped) ----

    fn run_typed_op(op: OpCode, width: NumericWidth, a: Constant, b: Constant) -> ValueWord {
        let mut program = BytecodeProgram::default();
        let c0 = program.add_constant(a);
        let c1 = program.add_constant(b);
        program.instructions = vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c0))),
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c1))),
            Instruction::new(op, Some(Operand::Width(width))),
            Instruction::simple(OpCode::Halt),
        ];
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(program);
        vm.execute(None).unwrap()
    }

    #[test]
    fn test_add_typed_i64() {
        let result = run_typed_op(
            OpCode::AddTyped,
            NumericWidth::I64,
            Constant::Int(10),
            Constant::Int(20),
        );
        assert_eq!(result.as_i64(), Some(30));
    }

    #[test]
    fn test_add_typed_f64() {
        let result = run_typed_op(
            OpCode::AddTyped,
            NumericWidth::F64,
            Constant::Number(1.5),
            Constant::Number(2.5),
        );
        assert_eq!(result.as_f64(), Some(4.0));
    }

    #[test]
    fn test_sub_typed_i64() {
        let result = run_typed_op(
            OpCode::SubTyped,
            NumericWidth::I64,
            Constant::Int(50),
            Constant::Int(20),
        );
        assert_eq!(result.as_i64(), Some(30));
    }

    #[test]
    fn test_sub_typed_f64() {
        let result = run_typed_op(
            OpCode::SubTyped,
            NumericWidth::F64,
            Constant::Number(10.0),
            Constant::Number(3.5),
        );
        assert_eq!(result.as_f64(), Some(6.5));
    }

    #[test]
    fn test_mul_typed_i64() {
        let result = run_typed_op(
            OpCode::MulTyped,
            NumericWidth::I64,
            Constant::Int(6),
            Constant::Int(7),
        );
        assert_eq!(result.as_i64(), Some(42));
    }

    #[test]
    fn test_mul_typed_f64() {
        let result = run_typed_op(
            OpCode::MulTyped,
            NumericWidth::F64,
            Constant::Number(3.0),
            Constant::Number(4.5),
        );
        assert_eq!(result.as_f64(), Some(13.5));
    }

    #[test]
    fn test_div_typed_i64() {
        let result = run_typed_op(
            OpCode::DivTyped,
            NumericWidth::I64,
            Constant::Int(100),
            Constant::Int(4),
        );
        assert_eq!(result.as_i64(), Some(25));
    }

    #[test]
    fn test_div_typed_f64() {
        let result = run_typed_op(
            OpCode::DivTyped,
            NumericWidth::F64,
            Constant::Number(10.0),
            Constant::Number(4.0),
        );
        assert_eq!(result.as_f64(), Some(2.5));
    }

    #[test]
    fn test_div_typed_i64_zero() {
        let mut program = BytecodeProgram::default();
        let c0 = program.add_constant(Constant::Int(10));
        let c1 = program.add_constant(Constant::Int(0));
        program.instructions = vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c0))),
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c1))),
            Instruction::new(OpCode::DivTyped, Some(Operand::Width(NumericWidth::I64))),
            Instruction::simple(OpCode::Halt),
        ];
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(program);
        let err = vm.execute(None).unwrap_err();
        assert!(
            matches!(err, shape_value::VMError::DivisionByZero),
            "Expected DivisionByZero, got {err:?}"
        );
    }

    #[test]
    fn test_mod_typed_i64() {
        let result = run_typed_op(
            OpCode::ModTyped,
            NumericWidth::I64,
            Constant::Int(17),
            Constant::Int(5),
        );
        assert_eq!(result.as_i64(), Some(2));
    }

    #[test]
    fn test_mod_typed_f64() {
        let result = run_typed_op(
            OpCode::ModTyped,
            NumericWidth::F64,
            Constant::Number(10.5),
            Constant::Number(3.0),
        );
        let v = result.as_f64().unwrap();
        assert!((v - 1.5).abs() < 1e-10);
    }

    #[test]
    fn test_cmp_typed_i64_less() {
        let result = run_typed_op(
            OpCode::CmpTyped,
            NumericWidth::I64,
            Constant::Int(3),
            Constant::Int(10),
        );
        assert_eq!(result.as_i64(), Some(-1));
    }

    #[test]
    fn test_cmp_typed_i64_equal() {
        let result = run_typed_op(
            OpCode::CmpTyped,
            NumericWidth::I64,
            Constant::Int(7),
            Constant::Int(7),
        );
        assert_eq!(result.as_i64(), Some(0));
    }

    #[test]
    fn test_cmp_typed_i64_greater() {
        let result = run_typed_op(
            OpCode::CmpTyped,
            NumericWidth::I64,
            Constant::Int(10),
            Constant::Int(3),
        );
        assert_eq!(result.as_i64(), Some(1));
    }

    #[test]
    fn test_cmp_typed_f64_less() {
        let result = run_typed_op(
            OpCode::CmpTyped,
            NumericWidth::F64,
            Constant::Number(1.0),
            Constant::Number(2.0),
        );
        assert_eq!(result.as_i64(), Some(-1));
    }

    #[test]
    fn test_add_typed_i32_delegates_to_int_path() {
        // All integer widths (I8..U64) delegate to the same int path
        let result = run_typed_op(
            OpCode::AddTyped,
            NumericWidth::I32,
            Constant::Int(100),
            Constant::Int(200),
        );
        assert_eq!(result.as_i64(), Some(300));
    }

    #[test]
    fn test_add_typed_f32_delegates_to_number_path() {
        // F32 delegates to the same f64 number path
        let result = run_typed_op(
            OpCode::AddTyped,
            NumericWidth::F32,
            Constant::Number(1.25),
            Constant::Number(2.75),
        );
        assert_eq!(result.as_f64(), Some(4.0));
    }

    #[test]
    fn test_add_typed_missing_width_is_error() {
        // AddTyped without Width operand should return InvalidOperand
        let mut program = BytecodeProgram::default();
        let c0 = program.add_constant(Constant::Int(1));
        let c1 = program.add_constant(Constant::Int(2));
        program.instructions = vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c0))),
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c1))),
            Instruction::simple(OpCode::AddTyped), // no operand
            Instruction::simple(OpCode::Halt),
        ];
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(program);
        let err = vm.execute(None).unwrap_err();
        assert!(
            matches!(err, shape_value::VMError::InvalidOperand),
            "Expected InvalidOperand, got {err:?}"
        );
    }

    // ========================================================================
    // Width-aware arithmetic tests (Sprint 3)
    // ========================================================================

    /// Helper: run AddTyped with a given width on two integer constants.
    fn run_typed_add(a: i64, b: i64, width: NumericWidth) -> ValueWord {
        let mut program = BytecodeProgram::default();
        let c0 = program.add_constant(Constant::Int(a));
        let c1 = program.add_constant(Constant::Int(b));
        program.instructions = vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c0))),
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c1))),
            Instruction::new(OpCode::AddTyped, Some(Operand::Width(width))),
            Instruction::simple(OpCode::Halt),
        ];
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(program);
        vm.execute(None).unwrap().clone()
    }

    fn run_typed_sub(a: i64, b: i64, width: NumericWidth) -> ValueWord {
        let mut program = BytecodeProgram::default();
        let c0 = program.add_constant(Constant::Int(a));
        let c1 = program.add_constant(Constant::Int(b));
        program.instructions = vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c0))),
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c1))),
            Instruction::new(OpCode::SubTyped, Some(Operand::Width(width))),
            Instruction::simple(OpCode::Halt),
        ];
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(program);
        vm.execute(None).unwrap().clone()
    }

    fn run_typed_mul(a: i64, b: i64, width: NumericWidth) -> ValueWord {
        let mut program = BytecodeProgram::default();
        let c0 = program.add_constant(Constant::Int(a));
        let c1 = program.add_constant(Constant::Int(b));
        program.instructions = vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c0))),
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c1))),
            Instruction::new(OpCode::MulTyped, Some(Operand::Width(width))),
            Instruction::simple(OpCode::Halt),
        ];
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(program);
        vm.execute(None).unwrap().clone()
    }

    fn run_cast_width(value: i64, width: NumericWidth) -> ValueWord {
        let mut program = BytecodeProgram::default();
        let c0 = program.add_constant(Constant::Int(value));
        program.instructions = vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c0))),
            Instruction::new(OpCode::CastWidth, Some(Operand::Width(width))),
            Instruction::simple(OpCode::Halt),
        ];
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(program);
        vm.execute(None).unwrap().clone()
    }

    fn run_store_local_typed(value: i64, local: u16, width: NumericWidth) -> ValueWord {
        let mut program = BytecodeProgram::default();
        let c0 = program.add_constant(Constant::Int(value));
        program.instructions = vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c0))),
            Instruction::new(
                OpCode::StoreLocalTyped,
                Some(Operand::TypedLocal(local, width)),
            ),
            Instruction::new(OpCode::LoadLocal, Some(Operand::Local(local))),
            Instruction::simple(OpCode::Halt),
        ];
        program.top_level_locals_count = local + 1;
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(program);
        vm.execute(None).unwrap().clone()
    }

    // -- i8 wrapping --

    #[test]
    fn test_i8_add_wraps() {
        // 127 + 1 = -128 (wrapping)
        let result = run_typed_add(127, 1, NumericWidth::I8);
        assert_eq!(result.as_i64(), Some(-128));
    }

    #[test]
    fn test_i8_sub_wraps() {
        // -128 - 1 = 127 (wrapping)
        let result = run_typed_sub(-128, 1, NumericWidth::I8);
        assert_eq!(result.as_i64(), Some(127));
    }

    #[test]
    fn test_i8_mul_wraps() {
        // 64 * 3 = 192 → truncate to i8 = -64
        let result = run_typed_mul(64, 3, NumericWidth::I8);
        assert_eq!(result.as_i64(), Some(-64));
    }

    // -- u8 wrapping --

    #[test]
    fn test_u8_add_wraps() {
        // 255 + 1 = 0 (wrapping)
        let result = run_typed_add(255, 1, NumericWidth::U8);
        assert_eq!(result.as_i64(), Some(0));
    }

    #[test]
    fn test_u8_sub_wraps() {
        // 0 - 1 = 255 (wrapping unsigned)
        let result = run_typed_sub(0, 1, NumericWidth::U8);
        assert_eq!(result.as_i64(), Some(255));
    }

    // -- i16 wrapping --

    #[test]
    fn test_i16_add_wraps() {
        // 32767 + 1 = -32768
        let result = run_typed_add(32767, 1, NumericWidth::I16);
        assert_eq!(result.as_i64(), Some(-32768));
    }

    // -- u16 wrapping --

    #[test]
    fn test_u16_add_wraps() {
        // 65535 + 1 = 0
        let result = run_typed_add(65535, 1, NumericWidth::U16);
        assert_eq!(result.as_i64(), Some(0));
    }

    // -- i32 wrapping --

    #[test]
    fn test_i32_add_wraps() {
        // 2147483647 + 1 = -2147483648
        let result = run_typed_add(2147483647, 1, NumericWidth::I32);
        assert_eq!(result.as_i64(), Some(-2147483648));
    }

    // -- u32 wrapping --

    #[test]
    fn test_u32_add_wraps() {
        // 4294967295 + 1 = 0
        let result = run_typed_add(4294967295, 1, NumericWidth::U32);
        assert_eq!(result.as_i64(), Some(0));
    }

    // -- i64 default: checked with f64 fallback --

    #[test]
    fn test_i64_add_checked_no_overflow() {
        let result = run_typed_add(100, 200, NumericWidth::I64);
        assert_eq!(result.as_i64(), Some(300));
    }

    // -- CastWidth --

    #[test]
    fn test_cast_width_i8_truncation() {
        // 300 → i8: 300 & 0xFF = 44, sign-extend → 44
        let result = run_cast_width(300, NumericWidth::I8);
        assert_eq!(result.as_i64(), Some(44));
    }

    #[test]
    fn test_cast_width_i8_negative() {
        // -1 → u8: -1 & 0xFF = 255
        let result = run_cast_width(-1, NumericWidth::U8);
        assert_eq!(result.as_i64(), Some(255));
    }

    #[test]
    fn test_cast_width_i16() {
        // 70000 → i16: 70000 & 0xFFFF = 4464, sign-extend → 4464
        let result = run_cast_width(70000, NumericWidth::I16);
        assert_eq!(result.as_i64(), Some(4464));
    }

    // -- StoreLocalTyped --

    #[test]
    fn test_store_local_typed_i8_truncates() {
        // Store 300 into an i8 local → should truncate to 44
        let result = run_store_local_typed(300, 0, NumericWidth::I8);
        assert_eq!(result.as_i64(), Some(44));
    }

    #[test]
    fn test_store_local_typed_u8_truncates() {
        // Store 256 into a u8 local → should truncate to 0
        let result = run_store_local_typed(256, 0, NumericWidth::U8);
        assert_eq!(result.as_i64(), Some(0));
    }

    #[test]
    fn test_store_local_typed_i64_passthrough() {
        // i64 StoreLocalTyped: no truncation
        let result = run_store_local_typed(42, 0, NumericWidth::I64);
        assert_eq!(result.as_i64(), Some(42));
    }

    // -- LOW-7: u64 max as i8 should give -1 --

    #[test]
    fn test_cast_width_u64_max_to_i8() {
        // u64::MAX (all ones) cast to i8 should give -1.
        // Use Constant::UInt to push a native u64 value.
        let mut program = BytecodeProgram::default();
        let c0 = program.add_constant(Constant::UInt(u64::MAX));
        program.instructions = vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c0))),
            Instruction::new(OpCode::CastWidth, Some(Operand::Width(NumericWidth::I8))),
            Instruction::simple(OpCode::Halt),
        ];
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(program);
        let result = vm.execute(None).unwrap();
        assert_eq!(
            result.as_i64(),
            Some(-1),
            "u64::MAX truncated to i8 should be -1"
        );
    }

    #[test]
    fn test_cast_width_u64_max_to_u8() {
        // u64::MAX cast to u8 should give 255 (0xFF).
        let mut program = BytecodeProgram::default();
        let c0 = program.add_constant(Constant::UInt(u64::MAX));
        program.instructions = vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c0))),
            Instruction::new(OpCode::CastWidth, Some(Operand::Width(NumericWidth::U8))),
            Instruction::simple(OpCode::Halt),
        ];
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(program);
        let result = vm.execute(None).unwrap();
        assert_eq!(
            result.as_i64(),
            Some(255),
            "u64::MAX truncated to u8 should be 255"
        );
    }
}
