//! Arithmetic operations for the VM executor
//!
//! Handles: Add, Sub, Mul, Div, Mod, Neg, Pow
//!
//! # Dynamic fallback (post-R5.6)
//!
//! After R5.6, [`exec_arithmetic_dynamic_fallback`] handles only
//! legitimately-dynamic arithmetic: polyglot values, comptime unresolved
//! types, and unresolved type-vars / mixed-type arithmetic. Every typed
//! shape has a direct compile-time retarget:
//!
//! - Int+Int / Number+Number / Decimal+Decimal → typed opcodes in
//!   `exec_typed_arithmetic`.
//! - Bitwise int+int → `BitAndInt` / `BitOrInt` / ... (R5.1).
//! - User operator traits → `CallMethod` (R5.2).
//! - DateTime / Duration / TimeSpan arithmetic (let-locals and typed
//!   function parameters) → `CallMethod` into `datetime_methods.rs`
//!   (R5.3). Untyped function returns of temporal values still land on
//!   the fallback via the temporal arms in `try_heap_arithmetic`.
//! - `Vec<number> +/-/*// Vec<number>`, `Vec<int> + Vec<int>`,
//!   `Mat +/- Mat`, `Mat * Mat` → `BuiltinCall` intrinsics (R5.4).
//! - `string + int/number/bool` (proved string LHS) → `StringConcatInt`
//!   / `StringConcatNumber` / `StringConcatBool` (R5.5).
//!
//! The handler name is suffixed `_dynamic_fallback` to make that
//! contract visible at every call site.
//!
//! See §R5 in `/home/dev/.claude/plans/v2-residuals-closeout.md` and
//! `/home/dev/.claude/plans/v2-nanbox-removal-plan.md` for context.
//!
//! # Typed path
//!
//! Typed opcodes (`AddInt`, `AddNumber`, `AddDecimal`, ...) live in
//! `exec_typed_arithmetic`. That is the hot path for production code.
//! Compact/width-parameterised opcodes (`AddTyped`, ...) live in
//! `exec_compact_typed_arithmetic`.

use crate::{
    bytecode::{Instruction, NumericWidth, OpCode, Operand},
    executor::VirtualMachine,
    executor::objects::raw_helpers,
};
use shape_value::heap_value::HeapValue;
use shape_value::{TemporalData, TypedArrayData, VMError, ValueWord, ValueWordExt};
use std::sync::Arc;

use crate::constants::EXACT_F64_INT_LIMIT;

/// Get the IC profiling tag byte for a ValueWord.
/// F64 (untagged) returns 0xFF (sentinel), otherwise returns the 3-bit tag.
#[inline(always)]
fn ic_tag(v: &ValueWord) -> u8 {
    let bits = v.raw_bits();
    if !shape_value::tag_bits::is_tagged(bits) {
        0xFF
    } else {
        shape_value::tag_bits::get_tag(bits) as u8
    }
}

/// Heap-typed binary op tag for the `try_heap_arithmetic` helper.
/// Distinct from `OpCode` because the helper only cares about the
/// operator shape (Add/Sub/Mul/Div) and not about the dispatch-level
/// Dynamic variant.
#[derive(Clone, Copy, Debug)]
enum HeapBinOp {
    Add,
    Sub,
    Mul,
    Div,
}

/// Produce a `VMError::RuntimeError` for mixed int/float operations where the
/// integer operand is too large to convert losslessly to f64.
fn cannot_apply_without_cast(op: &str, value: i128) -> VMError {
    VMError::RuntimeError(format!(
        "Cannot apply '{}' without explicit cast: {} is not losslessly representable as number",
        op, value
    ))
}

/// Check if an i64 result fits in the I48 inline range.
/// Values outside this range would be heap-boxed as BigInt, so we promote to f64 instead.
#[inline(always)]
fn fits_i48(v: i64) -> bool {
    v >= shape_value::tag_bits::I48_MIN && v <= shape_value::tag_bits::I48_MAX
}

#[derive(Clone, Copy)]
enum NumericDomain {
    Int(i128),
    Float(f64),
    Decimal(rust_decimal::Decimal),
}

/// Unwrap TypeAnnotatedValue wrapper to get the inner value.
/// This is needed because `: number` annotations wrap values, and the
/// Heap tag doesn't match any arithmetic dispatch case.
#[inline(always)]
fn unwrap_annotated(nb: ValueWord) -> ValueWord {
    raw_helpers::unwrap_annotated_bits(nb.raw_bits())
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
        if let Some(u) = nb.as_u64_value() {
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
        if let Some(d) = nb.as_decimal() {
            return Some(NumericDomain::Decimal(d));
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
                let af = Self::arith_i128_to_lossless_f64(ai)
                    .ok_or_else(|| cannot_apply_without_cast(op_name, ai))?;
                Ok(Some(ValueWord::from_f64(float_op(af, bf))))
            }
            (NumericDomain::Float(af), NumericDomain::Int(bi)) => {
                let bf = Self::arith_i128_to_lossless_f64(bi)
                    .ok_or_else(|| cannot_apply_without_cast(op_name, bi))?;
                Ok(Some(ValueWord::from_f64(float_op(af, bf))))
            }
            // Decimal cases: promote the other operand to Decimal
            (NumericDomain::Decimal(ad), NumericDomain::Decimal(bd)) => {
                // Delegate to the float_op via f64 conversion for consistency;
                // callers that want exact decimal arithmetic already use the
                // typed Decimal opcodes (AddDecimal, etc.).
                use rust_decimal::prelude::ToPrimitive;
                let af = ad.to_f64().unwrap_or(0.0);
                let bf = bd.to_f64().unwrap_or(0.0);
                Ok(Some(ValueWord::from_decimal(
                    rust_decimal::Decimal::from_f64_retain(float_op(af, bf)).unwrap_or_default(),
                )))
            }
            (NumericDomain::Decimal(ad), NumericDomain::Int(bi)) => {
                let bd = rust_decimal::Decimal::from_i128_with_scale(bi, 0);
                use rust_decimal::prelude::ToPrimitive;
                let af = ad.to_f64().unwrap_or(0.0);
                let bf = bd.to_f64().unwrap_or(0.0);
                Ok(Some(ValueWord::from_decimal(
                    rust_decimal::Decimal::from_f64_retain(float_op(af, bf)).unwrap_or_default(),
                )))
            }
            (NumericDomain::Int(ai), NumericDomain::Decimal(bd)) => {
                let ad = rust_decimal::Decimal::from_i128_with_scale(ai, 0);
                use rust_decimal::prelude::ToPrimitive;
                let af = ad.to_f64().unwrap_or(0.0);
                let bf = bd.to_f64().unwrap_or(0.0);
                Ok(Some(ValueWord::from_decimal(
                    rust_decimal::Decimal::from_f64_retain(float_op(af, bf)).unwrap_or_default(),
                )))
            }
            (NumericDomain::Decimal(ad), NumericDomain::Float(bf)) => {
                use rust_decimal::prelude::ToPrimitive;
                let af = ad.to_f64().unwrap_or(0.0);
                Ok(Some(ValueWord::from_f64(float_op(af, bf))))
            }
            (NumericDomain::Float(af), NumericDomain::Decimal(bd)) => {
                use rust_decimal::prelude::ToPrimitive;
                let bf = bd.to_f64().unwrap_or(0.0);
                Ok(Some(ValueWord::from_f64(float_op(af, bf))))
            }
        }
    }

    /// Dispatch a numeric binary operation with zero-check on the divisor.
    ///
    /// Shared implementation for div and mod: handles Int/Float/Decimal domain
    /// dispatch, zero-check, int/float cross-coercion, and decimal promotion.
    #[inline(always)]
    fn dispatch_numeric_binary_with_zero_check(
        a: &ValueWord,
        b: &ValueWord,
        op_name: &str,
        int_op: impl FnOnce(i128, i128) -> Option<i128>,
        float_op: impl Fn(f64, f64) -> f64,
        decimal_op: impl FnOnce(rust_decimal::Decimal, rust_decimal::Decimal) -> rust_decimal::Decimal,
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
            (NumericDomain::Int(ai), NumericDomain::Int(bi)) => {
                if bi == 0 {
                    return Err(VMError::DivisionByZero);
                }
                let out = int_op(ai, bi)
                    .ok_or_else(|| VMError::RuntimeError(format!("Integer overflow in '{}'", op_name)))?;
                Self::integer_result_boxed(out, op_name).map(Some)
            }
            (NumericDomain::Float(af), NumericDomain::Float(bf)) => {
                if bf == 0.0 {
                    return Err(VMError::DivisionByZero);
                }
                Ok(Some(ValueWord::from_f64(float_op(af, bf))))
            }
            (NumericDomain::Int(ai), NumericDomain::Float(bf)) => {
                if bf == 0.0 {
                    return Err(VMError::DivisionByZero);
                }
                let af = Self::arith_i128_to_lossless_f64(ai)
                    .ok_or_else(|| cannot_apply_without_cast(op_name, ai))?;
                Ok(Some(ValueWord::from_f64(float_op(af, bf))))
            }
            (NumericDomain::Float(af), NumericDomain::Int(bi)) => {
                let bf = Self::arith_i128_to_lossless_f64(bi)
                    .ok_or_else(|| cannot_apply_without_cast(op_name, bi))?;
                if bf == 0.0 {
                    return Err(VMError::DivisionByZero);
                }
                Ok(Some(ValueWord::from_f64(float_op(af, bf))))
            }
            (NumericDomain::Decimal(ad), NumericDomain::Decimal(bd)) => {
                if bd.is_zero() {
                    return Err(VMError::DivisionByZero);
                }
                Ok(Some(ValueWord::from_decimal(decimal_op(ad, bd))))
            }
            (NumericDomain::Decimal(ad), NumericDomain::Int(bi)) => {
                let bd = rust_decimal::Decimal::from_i128_with_scale(bi, 0);
                if bd.is_zero() {
                    return Err(VMError::DivisionByZero);
                }
                Ok(Some(ValueWord::from_decimal(decimal_op(ad, bd))))
            }
            (NumericDomain::Int(ai), NumericDomain::Decimal(bd)) => {
                if bd.is_zero() {
                    return Err(VMError::DivisionByZero);
                }
                let ad = rust_decimal::Decimal::from_i128_with_scale(ai, 0);
                Ok(Some(ValueWord::from_decimal(decimal_op(ad, bd))))
            }
            (NumericDomain::Decimal(ad), NumericDomain::Float(bf)) => {
                if bf == 0.0 {
                    return Err(VMError::DivisionByZero);
                }
                use rust_decimal::prelude::ToPrimitive;
                let af = ad.to_f64().unwrap_or(0.0);
                Ok(Some(ValueWord::from_f64(float_op(af, bf))))
            }
            (NumericDomain::Float(af), NumericDomain::Decimal(bd)) => {
                use rust_decimal::prelude::ToPrimitive;
                let bf = bd.to_f64().unwrap_or(0.0);
                if bf == 0.0 {
                    return Err(VMError::DivisionByZero);
                }
                Ok(Some(ValueWord::from_f64(float_op(af, bf))))
            }
        }
    }

    #[inline(always)]
    fn numeric_div_result(a: &ValueWord, b: &ValueWord) -> Result<Option<ValueWord>, VMError> {
        Self::dispatch_numeric_binary_with_zero_check(
            a, b, "/",
            |a, b| a.checked_div(b),
            |a, b| a / b,
            |a, b| a / b,
        )
    }

    #[inline(always)]
    fn numeric_mod_result(a: &ValueWord, b: &ValueWord) -> Result<Option<ValueWord>, VMError> {
        Self::dispatch_numeric_binary_with_zero_check(
            a, b, "%",
            |a, b| a.checked_rem(b),
            |a, b| a % b,
            |a, b| a % b,
        )
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
                let base_f = Self::arith_i128_to_lossless_f64(base)
                    .ok_or_else(|| cannot_apply_without_cast("**", base))?;
                let exp_f = Self::arith_i128_to_lossless_f64(exp)
                    .ok_or_else(|| cannot_apply_without_cast("**", exp))?;
                Ok(Some(ValueWord::from_f64(base_f.powf(exp_f))))
            }
            (NumericDomain::Float(base), NumericDomain::Float(exp)) => {
                Ok(Some(ValueWord::from_f64(base.powf(exp))))
            }
            (NumericDomain::Int(base), NumericDomain::Float(exp)) => {
                let base_f = Self::arith_i128_to_lossless_f64(base)
                    .ok_or_else(|| cannot_apply_without_cast("**", base))?;
                Ok(Some(ValueWord::from_f64(base_f.powf(exp))))
            }
            (NumericDomain::Float(base), NumericDomain::Int(exp)) => {
                let exp_f = Self::arith_i128_to_lossless_f64(exp)
                    .ok_or_else(|| cannot_apply_without_cast("**", exp))?;
                Ok(Some(ValueWord::from_f64(base.powf(exp_f))))
            }
            // Decimal power — convert to f64 for the operation, return decimal
            (NumericDomain::Decimal(ad), NumericDomain::Decimal(bd)) => {
                use rust_decimal::prelude::ToPrimitive;
                let base_f = ad.to_f64().unwrap_or(0.0);
                let exp_f = bd.to_f64().unwrap_or(0.0);
                use rust_decimal::prelude::FromPrimitive;
                Ok(Some(ValueWord::from_decimal(
                    rust_decimal::Decimal::from_f64(base_f.powf(exp_f)).unwrap_or_default(),
                )))
            }
            (NumericDomain::Decimal(ad), NumericDomain::Int(exp)) => {
                use rust_decimal::prelude::ToPrimitive;
                let base_f = ad.to_f64().unwrap_or(0.0);
                let exp_f = exp as f64;
                use rust_decimal::prelude::FromPrimitive;
                Ok(Some(ValueWord::from_decimal(
                    rust_decimal::Decimal::from_f64(base_f.powf(exp_f)).unwrap_or_default(),
                )))
            }
            (NumericDomain::Int(base), NumericDomain::Decimal(bd)) => {
                use rust_decimal::prelude::ToPrimitive;
                let base_f = base as f64;
                let exp_f = bd.to_f64().unwrap_or(0.0);
                use rust_decimal::prelude::FromPrimitive;
                Ok(Some(ValueWord::from_decimal(
                    rust_decimal::Decimal::from_f64(base_f.powf(exp_f)).unwrap_or_default(),
                )))
            }
            (NumericDomain::Decimal(ad), NumericDomain::Float(exp)) => {
                use rust_decimal::prelude::ToPrimitive;
                let base_f = ad.to_f64().unwrap_or(0.0);
                Ok(Some(ValueWord::from_f64(base_f.powf(exp))))
            }
            (NumericDomain::Float(base), NumericDomain::Decimal(bd)) => {
                use rust_decimal::prelude::ToPrimitive;
                let exp_f = bd.to_f64().unwrap_or(0.0);
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
            // ===== Typed Add (raw typed stack API with fallback) =====
            AddInt => {
                if self.stack_top_both_i48() {
                    // Fast path: both operands are inline i48
                    let bi = self.pop_raw_i64()?;
                    let ai = self.pop_raw_i64()?;
                    match ai.checked_add(bi) {
                        Some(result) if fits_i48(result) => self.push_raw_i64(result)?,
                        _ => self.push_raw_f64(ai as f64 + bi as f64)?,
                    }
                } else {
                    // Slow path: BigInt or coercible values
                    let b = self.pop_raw_u64()?;
                    let a = self.pop_raw_u64()?;
                    let ai = Self::int_operand(&a).ok_or_else(|| VMError::TypeError {
                        expected: "int",
                        got: a.type_name(),
                    })?;
                    let bi = Self::int_operand(&b).ok_or_else(|| VMError::TypeError {
                        expected: "int",
                        got: b.type_name(),
                    })?;
                    match ai.checked_add(bi) {
                        Some(result) if fits_i48(result) => self.push_raw_i64(result)?,
                        _ => self.push_raw_f64(ai as f64 + bi as f64)?,
                    }
                }
            }
            AddNumber => {
                if self.stack_top_both_f64() {
                    let rhs = self.pop_raw_f64()?;
                    let lhs = self.pop_raw_f64()?;
                    self.push_raw_f64(lhs + rhs)?;
                } else {
                    let b = self.pop_raw_u64()?;
                    let a = self.pop_raw_u64()?;
                    let lhs = Self::number_operand(&a).ok_or_else(|| VMError::TypeError {
                        expected: "number",
                        got: a.type_name(),
                    })?;
                    let rhs = Self::number_operand(&b).ok_or_else(|| VMError::TypeError {
                        expected: "number",
                        got: b.type_name(),
                    })?;
                    self.push_raw_f64(lhs + rhs)?;
                }
            }
            AddDecimal => {
                let b = self.pop_raw_u64()?;
                let a = self.pop_raw_u64()?;
                self.push_raw_u64(ValueWord::from_decimal(unsafe {
                    a.as_decimal_unchecked() + b.as_decimal_unchecked()
                }))?;
            }
            // ===== Typed Sub (raw typed stack API with fallback) =====
            SubInt => {
                if self.stack_top_both_i48() {
                    let bi = self.pop_raw_i64()?;
                    let ai = self.pop_raw_i64()?;
                    match ai.checked_sub(bi) {
                        Some(result) if fits_i48(result) => self.push_raw_i64(result)?,
                        _ => self.push_raw_f64(ai as f64 - bi as f64)?,
                    }
                } else {
                    let b = self.pop_raw_u64()?;
                    let a = self.pop_raw_u64()?;
                    let ai = Self::int_operand(&a).ok_or_else(|| VMError::TypeError {
                        expected: "int",
                        got: a.type_name(),
                    })?;
                    let bi = Self::int_operand(&b).ok_or_else(|| VMError::TypeError {
                        expected: "int",
                        got: b.type_name(),
                    })?;
                    match ai.checked_sub(bi) {
                        Some(result) if fits_i48(result) => self.push_raw_i64(result)?,
                        _ => self.push_raw_f64(ai as f64 - bi as f64)?,
                    }
                }
            }
            SubNumber => {
                if self.stack_top_both_f64() {
                    let rhs = self.pop_raw_f64()?;
                    let lhs = self.pop_raw_f64()?;
                    self.push_raw_f64(lhs - rhs)?;
                } else {
                    let b = self.pop_raw_u64()?;
                    let a = self.pop_raw_u64()?;
                    let lhs = Self::number_operand(&a).ok_or_else(|| VMError::TypeError {
                        expected: "number",
                        got: a.type_name(),
                    })?;
                    let rhs = Self::number_operand(&b).ok_or_else(|| VMError::TypeError {
                        expected: "number",
                        got: b.type_name(),
                    })?;
                    self.push_raw_f64(lhs - rhs)?;
                }
            }
            SubDecimal => {
                let b = self.pop_raw_u64()?;
                let a = self.pop_raw_u64()?;
                self.push_raw_u64(ValueWord::from_decimal(unsafe {
                    a.as_decimal_unchecked() - b.as_decimal_unchecked()
                }))?;
            }
            // ===== Typed Mul (raw typed stack API with fallback) =====
            MulInt => {
                if self.stack_top_both_i48() {
                    let bi = self.pop_raw_i64()?;
                    let ai = self.pop_raw_i64()?;
                    match ai.checked_mul(bi) {
                        Some(result) if fits_i48(result) => self.push_raw_i64(result)?,
                        _ => self.push_raw_f64(ai as f64 * bi as f64)?,
                    }
                } else {
                    let b = self.pop_raw_u64()?;
                    let a = self.pop_raw_u64()?;
                    let ai = Self::int_operand(&a).ok_or_else(|| VMError::TypeError {
                        expected: "int",
                        got: a.type_name(),
                    })?;
                    let bi = Self::int_operand(&b).ok_or_else(|| VMError::TypeError {
                        expected: "int",
                        got: b.type_name(),
                    })?;
                    match ai.checked_mul(bi) {
                        Some(result) if fits_i48(result) => self.push_raw_i64(result)?,
                        _ => self.push_raw_f64(ai as f64 * bi as f64)?,
                    }
                }
            }
            MulNumber => {
                if self.stack_top_both_f64() {
                    let rhs = self.pop_raw_f64()?;
                    let lhs = self.pop_raw_f64()?;
                    self.push_raw_f64(lhs * rhs)?;
                } else {
                    let b = self.pop_raw_u64()?;
                    let a = self.pop_raw_u64()?;
                    let lhs = Self::number_operand(&a).ok_or_else(|| VMError::TypeError {
                        expected: "number",
                        got: a.type_name(),
                    })?;
                    let rhs = Self::number_operand(&b).ok_or_else(|| VMError::TypeError {
                        expected: "number",
                        got: b.type_name(),
                    })?;
                    self.push_raw_f64(lhs * rhs)?;
                }
            }
            MulDecimal => {
                let b = self.pop_raw_u64()?;
                let a = self.pop_raw_u64()?;
                self.push_raw_u64(ValueWord::from_decimal(unsafe {
                    a.as_decimal_unchecked() * b.as_decimal_unchecked()
                }))?;
            }
            // ===== Typed Div (raw typed stack API, with zero-check) =====
            DivInt => {
                if self.stack_top_both_i48() {
                    let bi = self.pop_raw_i64()?;
                    let ai = self.pop_raw_i64()?;
                    if bi == 0 {
                        return Err(VMError::DivisionByZero);
                    }
                    self.push_raw_i64(ai / bi)?;
                } else {
                    let b = self.pop_raw_u64()?;
                    let a = self.pop_raw_u64()?;
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
                    self.push_raw_i64(ai / bi)?;
                }
            }
            DivNumber => {
                if self.stack_top_both_f64() {
                    let divisor = self.pop_raw_f64()?;
                    let lhs = self.pop_raw_f64()?;
                    if divisor == 0.0 {
                        return Err(VMError::DivisionByZero);
                    }
                    self.push_raw_f64(lhs / divisor)?;
                } else {
                    let b = self.pop_raw_u64()?;
                    let a = self.pop_raw_u64()?;
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
                    self.push_raw_f64(lhs / divisor)?;
                }
            }
            DivDecimal => {
                let b = self.pop_raw_u64()?;
                let a = self.pop_raw_u64()?;
                let divisor = unsafe { b.as_decimal_unchecked() };
                if divisor.is_zero() {
                    return Err(VMError::DivisionByZero);
                }
                self.push_raw_u64(ValueWord::from_decimal(
                    unsafe { a.as_decimal_unchecked() } / divisor,
                ))?;
            }
            // ===== Typed Mod (raw typed stack API, with zero-check) =====
            ModInt => {
                if self.stack_top_both_i48() {
                    let bi = self.pop_raw_i64()?;
                    let ai = self.pop_raw_i64()?;
                    if bi == 0 {
                        return Err(VMError::DivisionByZero);
                    }
                    self.push_raw_i64(ai % bi)?;
                } else {
                    let b = self.pop_raw_u64()?;
                    let a = self.pop_raw_u64()?;
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
                    self.push_raw_i64(ai % bi)?;
                }
            }
            ModNumber => {
                if self.stack_top_both_f64() {
                    let divisor = self.pop_raw_f64()?;
                    let lhs = self.pop_raw_f64()?;
                    if divisor == 0.0 {
                        return Err(VMError::DivisionByZero);
                    }
                    self.push_raw_f64(lhs % divisor)?;
                } else {
                    let b = self.pop_raw_u64()?;
                    let a = self.pop_raw_u64()?;
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
                    self.push_raw_f64(lhs % divisor)?;
                }
            }
            ModDecimal => {
                let b = self.pop_raw_u64()?;
                let a = self.pop_raw_u64()?;
                let divisor = unsafe { b.as_decimal_unchecked() };
                if divisor.is_zero() {
                    return Err(VMError::DivisionByZero);
                }
                self.push_raw_u64(ValueWord::from_decimal(
                    unsafe { a.as_decimal_unchecked() } % divisor,
                ))?;
            }
            // ===== Typed Pow (raw typed stack API with fallback) =====
            PowInt => {
                if self.stack_top_both_i48() {
                    let exp = self.pop_raw_i64()?;
                    let base = self.pop_raw_i64()?;
                    if exp >= 0 && exp < u32::MAX as i64 {
                        let result = base.pow(exp as u32);
                        if fits_i48(result) {
                            self.push_raw_i64(result)?;
                        } else {
                            self.push_raw_f64(result as f64)?;
                        }
                    } else {
                        self.push_raw_f64((base as f64).powf(exp as f64))?;
                    }
                } else {
                    let b = self.pop_raw_u64()?;
                    let a = self.pop_raw_u64()?;
                    let base = Self::int_operand(&a).ok_or_else(|| VMError::TypeError {
                        expected: "int",
                        got: a.type_name(),
                    })?;
                    let exp = Self::int_operand(&b).ok_or_else(|| VMError::TypeError {
                        expected: "int",
                        got: b.type_name(),
                    })?;
                    if exp >= 0 && exp < u32::MAX as i64 {
                        let result = base.pow(exp as u32);
                        if fits_i48(result) {
                            self.push_raw_i64(result)?;
                        } else {
                            self.push_raw_f64(result as f64)?;
                        }
                    } else {
                        self.push_raw_f64((base as f64).powf(exp as f64))?;
                    }
                }
            }
            PowNumber => {
                if self.stack_top_both_f64() {
                    let exp = self.pop_raw_f64()?;
                    let base = self.pop_raw_f64()?;
                    self.push_raw_f64(base.powf(exp))?;
                } else {
                    let b = self.pop_raw_u64()?;
                    let a = self.pop_raw_u64()?;
                    let base = Self::number_operand(&a).ok_or_else(|| VMError::TypeError {
                        expected: "number",
                        got: a.type_name(),
                    })?;
                    let exp = Self::number_operand(&b).ok_or_else(|| VMError::TypeError {
                        expected: "number",
                        got: b.type_name(),
                    })?;
                    self.push_raw_f64(base.powf(exp))?;
                }
            }
            PowDecimal => {
                let b = self.pop_raw_u64()?;
                let a = self.pop_raw_u64()?;
                use rust_decimal::prelude::ToPrimitive;
                let base = unsafe { a.as_decimal_unchecked() };
                let exp = unsafe { b.as_decimal_unchecked() };
                let result = base
                    .to_f64()
                    .unwrap_or(0.0)
                    .powf(exp.to_f64().unwrap_or(0.0));
                use rust_decimal::prelude::FromPrimitive;
                self.push_raw_u64(ValueWord::from_decimal(
                    rust_decimal::Decimal::from_f64(result).unwrap_or_default(),
                ))?;
            }
            // ===== Numeric Coercion (raw typed stack API with fallback) =====
            IntToNumber => {
                if self.stack_top_is_i48() {
                    let v = self.pop_raw_i64()?;
                    self.push_raw_f64(v as f64)?;
                } else {
                    let val = self.pop_raw_u64()?;
                    self.push_raw_f64(unsafe { val.as_i64_unchecked() } as f64)?;
                }
            }
            NumberToInt => {
                if self.stack_top_is_f64() {
                    let v = self.pop_raw_f64()?;
                    self.push_raw_i64(v as i64)?;
                } else {
                    let val = self.pop_raw_u64()?;
                    self.push_raw_i64(unsafe { val.as_f64_unchecked() } as i64)?;
                }
            }
            // Stage 4.2: typed negation moved here from exec_arithmetic
            NegInt => {
                let val = self.pop_raw_i64()?;
                self.push_raw_i64(-val)?;
            }
            NegNumber => {
                // Mirror `MulNumber`'s fast/slow split (BUG5): the fast path
                // assumes the compiler proved the operand is a plain f64, but
                // `NegNumber` is also reached when a method body declared on
                // `Number` executes with a tagged `int` (i48) receiver —
                // `extend Number { method negate() { -self } }` called as
                // `(42).negate()`. Reinterpreting the tagged bits as f64 would
                // emit NaN; the fallback decodes the ValueWord and coerces
                // losslessly, matching `MulNumber`/`AddNumber`/etc.
                if self.stack_top_is_f64() {
                    let val = self.pop_raw_f64()?;
                    self.push_raw_f64(-val)?;
                } else {
                    let a = self.pop_raw_u64()?;
                    let v = Self::number_operand(&a).ok_or_else(|| VMError::TypeError {
                        expected: "number",
                        got: a.type_name(),
                    })?;
                    self.push_raw_f64(-v)?;
                }
            }
            NegDecimal => {
                // Decimal is heap-backed, use ValueWord path
                let val = self.pop_raw_u64()?;
                if let Some(d) = val.as_decimal() {
                    self.push_raw_u64(ValueWord::from_decimal(-d))?;
                } else {
                    self.push_raw_u64(val)?;
                }
            }
            // ===== R5.1B: Typed bitwise opcodes =====
            //
            // Int-typed siblings of the dynamic `BitAnd`/`BitOr`/`BitXor`/
            // `BitShl`/`BitShr`/`BitNot` handlers in
            // `exec_arithmetic_dynamic_fallback`. The compiler emits these
            // (R5.1C) when both operand types are proved to be `int`; at
            // this stage they are reachable only from hand-crafted
            // bytecode. Semantics match the dynamic fallback exactly
            // (plain i64 `&` / `|` / `^` / `<<` / `>>` / `!`) — no
            // rhs-masking, matching the documented `>>` / `<<` semantics
            // already shipped in Shape. Operands are raw i48-tagged int
            // slots per the v2 runtime spec; no tag checks, no coercion.
            BitAndInt => {
                let b = self.pop_raw_i64()?;
                let a = self.pop_raw_i64()?;
                self.push_raw_i64(a & b)?;
            }
            BitOrInt => {
                let b = self.pop_raw_i64()?;
                let a = self.pop_raw_i64()?;
                self.push_raw_i64(a | b)?;
            }
            BitXorInt => {
                let b = self.pop_raw_i64()?;
                let a = self.pop_raw_i64()?;
                self.push_raw_i64(a ^ b)?;
            }
            BitShlInt => {
                let b = self.pop_raw_i64()?;
                let a = self.pop_raw_i64()?;
                self.push_raw_i64(a << b)?;
            }
            BitShrInt => {
                let b = self.pop_raw_i64()?;
                let a = self.pop_raw_i64()?;
                self.push_raw_i64(a >> b)?;
            }
            BitNotInt => {
                let a = self.pop_raw_i64()?;
                self.push_raw_i64(!a)?;
            }
            _ => unreachable!(
                "exec_typed_arithmetic called with non-typed-arithmetic opcode: {:?}",
                instruction.opcode
            ),
        }
        Ok(())
    }

    // NOTE: exec_trusted_arithmetic was removed — trusted arithmetic opcodes
    // (AddIntTrusted, etc.) were consolidated into the typed variants.

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

    #[inline(always)]
    fn compact_int_checked_binop(
        &mut self,
        width: NumericWidth,
        wrapping_op: impl FnOnce(i64, i64) -> i64,
        checked: impl FnOnce(i64, i64) -> Option<i64>,
        overflow_fallback: impl FnOnce(i64, i64) -> f64,
    ) -> Result<(), VMError> {
        let b = self.pop_raw_u64()?;
        let a = self.pop_raw_u64()?;

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
        if let Some(int_w) = width.to_int_width() {
            let result = wrapping_op(ai, bi);
            return self.push_raw_i64(int_w.truncate(result));
        }

        // I64: checked with f64 fallback on overflow
        match checked(ai, bi) {
            Some(result) => self.push_raw_i64(result),
            None => self.push_raw_f64(overflow_fallback(ai, bi)),
        }
    }

    #[inline(always)]
    fn compact_int_divmod(
        &mut self,
        width: NumericWidth,
        op: impl FnOnce(i64, i64) -> i64,
    ) -> Result<(), VMError> {
        let b = self.pop_raw_u64()?;
        let a = self.pop_raw_u64()?;

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
        if let Some(int_w) = width.to_int_width() {
            self.push_raw_i64(int_w.truncate(result))
        } else {
            self.push_raw_i64(result)
        }
    }

    #[inline(always)]
    fn compact_float_binop(&mut self, op: impl FnOnce(f64, f64) -> f64) -> Result<(), VMError> {
        let b = self.pop_raw_u64()?;
        let a = self.pop_raw_u64()?;

        let lhs =
            Self::number_operand(&a).ok_or_else(|| Self::compact_number_type_error(&a, &b))?;
        let rhs =
            Self::number_operand(&b).ok_or_else(|| Self::compact_number_type_error(&a, &b))?;
        self.push_raw_f64(op(lhs, rhs))
    }

    #[inline(always)]
    fn compact_float_divmod(&mut self, op: impl FnOnce(f64, f64) -> f64) -> Result<(), VMError> {
        let b = self.pop_raw_u64()?;
        let a = self.pop_raw_u64()?;

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
        self.push_raw_f64(op(lhs, rhs))
    }

    #[inline(always)]
    fn compact_int_cmp(&mut self, width: NumericWidth) -> Result<(), VMError> {
        let b = self.pop_raw_u64()?;
        let a = self.pop_raw_u64()?;
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
        self.push_raw_i64(ord)
    }

    #[inline(always)]
    fn compact_float_cmp(&mut self) -> Result<(), VMError> {
        let b = self.pop_raw_u64()?;
        let a = self.pop_raw_u64()?;
        let lhs = Self::number_operand(&a).ok_or_else(|| VMError::TypeError {
            expected: "number",
            got: a.type_name(),
        })?;
        let rhs = Self::number_operand(&b).ok_or_else(|| VMError::TypeError {
            expected: "number",
            got: b.type_name(),
        })?;
        let ord = lhs.partial_cmp(&rhs).map_or(0i64, |o| o as i64);
        self.push_raw_i64(ord)
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
        let nb = self.pop_raw_u64()?;
        let raw = Self::int_operand(&nb).unwrap_or_else(|| {
            // If not an int, try to extract from number
            nb.as_f64().map(|f| f as i64).unwrap_or(0)
        });
        if let Some(int_w) = width.to_int_width() {
            self.push_raw_i64(int_w.truncate(raw))
        } else {
            // I64 or float: no truncation
            self.push_raw_i64(raw)
        }
    }

    /// Try the arithmetic IC fast path for a binary operation.
    ///
    /// If the feedback vector shows a monomorphic I48+I48 or F64+F64 pattern
    /// and the stack values match, execute the operation directly without
    /// going through the full generic dispatch. Returns `Some(())` on hit
    /// (result already pushed), `None` on miss.
    #[inline(always)]
    fn try_arithmetic_ic_fast_path(
        &mut self,
        i48_op: unsafe fn(&ValueWord, &ValueWord) -> ValueWord,
        f64_op: fn(f64, f64) -> f64,
    ) -> Result<Option<()>, VMError> {
        use crate::executor::ic_fast_paths::{ArithmeticIcHint, arithmetic_ic_check};
        use shape_value::tag_bits::TAG_INT;

        let hint = arithmetic_ic_check(self, self.ip);
        if hint == ArithmeticIcHint::BothI48 && self.sp >= 2 {
            let slice = self.stack_slice_raw((self.sp - 2)..self.sp);
            let a = &slice[0];
            let b = &slice[1];
            if a.is_i64() && b.is_i64() {
                let result = unsafe { i48_op(a, b) };
                self.sp -= 2;
                let ip = self.ip;
                if let Some(fv) = self.current_feedback_vector() {
                    fv.record_arithmetic(ip, TAG_INT as u8, TAG_INT as u8);
                }
                self.push_raw_u64(result)?;
                return Ok(Some(()));
            }
        } else if hint == ArithmeticIcHint::BothF64 && self.sp >= 2 {
            let slice = self.stack_slice_raw((self.sp - 2)..self.sp);
            let a = &slice[0];
            let b = &slice[1];
            if let (Some(af), Some(bf)) = (a.as_f64(), b.as_f64()) {
                self.sp -= 2;
                let ip = self.ip;
                if let Some(fv) = self.current_feedback_vector() {
                    fv.record_arithmetic(ip, 0xFF, 0xFF);
                }
                self.push_raw_u64(ValueWord::from_f64(f64_op(af, bf)))?;
                return Ok(Some(()));
            }
        }
        Ok(None)
    }

    /// Dynamic arithmetic dispatch for `*Dynamic` opcodes only.
    ///
    /// Post-R5.6: user operator traits, DateTime arithmetic, Matrix/Vec
    /// arithmetic, and typed string+scalar concat are all retargeted at
    /// compile time to dedicated opcodes / method dispatch / intrinsic
    /// calls, so the Dynamic path is only reached for:
    ///
    ///   * (int, int) / (f64, f64) / (decimal, decimal) numeric
    ///     arithmetic where the compiler could not prove types
    ///     (polyglot / comptime / unresolved type-vars).
    ///   * (string, string | char) concat on Add.
    ///   * Live mixed-heap shapes that have no typed retarget yet:
    ///     `Vec<int> + Vec<number>`, `Matrix * Vec<number>`, vector and
    ///     matrix scalar-broadcasts.
    ///   * The string+scalar residual paths documented on the heap-arm
    ///     match below (flag-off / commutative / untyped-param).
    ///
    /// See the module-level docstring for the full R5 retarget set.
    #[inline(always)]
    pub(in crate::executor) fn exec_arithmetic_dynamic_fallback(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use OpCode::*;
        match instruction.opcode {
            AddDynamic => {
                if self
                    .try_arithmetic_ic_fast_path(ValueWord::add_i64, |a, b| a + b)?
                    .is_some()
                {
                    return Ok(());
                }
                let b_nb = unwrap_annotated(self.pop_raw_u64()?);
                let a_nb = unwrap_annotated(self.pop_raw_u64()?);
                {
                    let ip = self.ip;
                    if let Some(fv) = self.current_feedback_vector() {
                        fv.record_arithmetic(ip, ic_tag(&a_nb), ic_tag(&b_nb));
                    }
                }
                if let Some(result) = Self::numeric_binary_result(
                    &a_nb,
                    &b_nb,
                    "+",
                    |a, b| a.checked_add(b),
                    |a, b| a + b,
                )? {
                    return self.push_raw_u64(result);
                }
                if let Some(result) = Self::try_string_concat(&a_nb, &b_nb) {
                    return self.push_raw_u64(result);
                }
                if let Some(result) = self.try_heap_arithmetic(HeapBinOp::Add, &a_nb, &b_nb)? {
                    return self.push_raw_u64(result);
                }
                Err(VMError::RuntimeError(format!(
                    "Cannot apply '+' to {} and {}",
                    a_nb.type_name(),
                    b_nb.type_name()
                )))
            }
            SubDynamic => {
                if self
                    .try_arithmetic_ic_fast_path(ValueWord::sub_i64, |a, b| a - b)?
                    .is_some()
                {
                    return Ok(());
                }
                let b_nb = unwrap_annotated(self.pop_raw_u64()?);
                let a_nb = unwrap_annotated(self.pop_raw_u64()?);
                {
                    let ip = self.ip;
                    if let Some(fv) = self.current_feedback_vector() {
                        fv.record_arithmetic(ip, ic_tag(&a_nb), ic_tag(&b_nb));
                    }
                }
                if let Some(result) = Self::numeric_binary_result(
                    &a_nb,
                    &b_nb,
                    "-",
                    |a, b| a.checked_sub(b),
                    |a, b| a - b,
                )? {
                    return self.push_raw_u64(result);
                }
                if let Some(result) = self.try_heap_arithmetic(HeapBinOp::Sub, &a_nb, &b_nb)? {
                    return self.push_raw_u64(result);
                }
                Err(VMError::RuntimeError(format!(
                    "Cannot apply '-' to {} and {}",
                    a_nb.type_name(),
                    b_nb.type_name()
                )))
            }
            MulDynamic => {
                if self
                    .try_arithmetic_ic_fast_path(ValueWord::mul_i64, |a, b| a * b)?
                    .is_some()
                {
                    return Ok(());
                }
                let b_nb = unwrap_annotated(self.pop_raw_u64()?);
                let a_nb = unwrap_annotated(self.pop_raw_u64()?);
                {
                    let ip = self.ip;
                    if let Some(fv) = self.current_feedback_vector() {
                        fv.record_arithmetic(ip, ic_tag(&a_nb), ic_tag(&b_nb));
                    }
                }
                if let Some(result) = Self::numeric_binary_result(
                    &a_nb,
                    &b_nb,
                    "*",
                    |a, b| a.checked_mul(b),
                    |a, b| a * b,
                )? {
                    return self.push_raw_u64(result);
                }
                if let Some(result) = self.try_heap_arithmetic(HeapBinOp::Mul, &a_nb, &b_nb)? {
                    return self.push_raw_u64(result);
                }
                Err(VMError::RuntimeError(format!(
                    "Cannot apply '*' to {} and {}",
                    a_nb.type_name(),
                    b_nb.type_name()
                )))
            }
            DivDynamic => {
                let b_nb = unwrap_annotated(self.pop_raw_u64()?);
                let a_nb = unwrap_annotated(self.pop_raw_u64()?);
                if let Some(result) = Self::numeric_div_result(&a_nb, &b_nb)? {
                    return self.push_raw_u64(result);
                }
                if let Some(result) = self.try_heap_arithmetic(HeapBinOp::Div, &a_nb, &b_nb)? {
                    return self.push_raw_u64(result);
                }
                Err(VMError::RuntimeError(format!(
                    "Cannot apply '/' to {} and {}",
                    a_nb.type_name(),
                    b_nb.type_name()
                )))
            }
            ModDynamic => {
                let b_nb = unwrap_annotated(self.pop_raw_u64()?);
                let a_nb = unwrap_annotated(self.pop_raw_u64()?);
                if let Some(result) = Self::numeric_mod_result(&a_nb, &b_nb)? {
                    return self.push_raw_u64(result);
                }
                Err(VMError::RuntimeError(format!(
                    "Cannot apply '%' to {} and {}",
                    a_nb.type_name(),
                    b_nb.type_name()
                )))
            }
            PowDynamic => {
                let b_nb = unwrap_annotated(self.pop_raw_u64()?);
                let a_nb = unwrap_annotated(self.pop_raw_u64()?);
                if let Some(result) = Self::numeric_pow_result(&a_nb, &b_nb)? {
                    return self.push_raw_u64(result);
                }
                Err(VMError::RuntimeError(format!(
                    "Cannot apply '**' to {} and {}",
                    a_nb.type_name(),
                    b_nb.type_name()
                )))
            }
            BitXor | BitAnd | BitOr | BitShl | BitShr => {
                self.exec_dyn_bit_binary(instruction.opcode)
            }
            BitNot => self.exec_dyn_bit_unary(),
            _ => unreachable!(
                "exec_arithmetic_dynamic_fallback called with non-arithmetic opcode: {:?}",
                instruction.opcode
            ),
        }
    }

    /// String / char concat fallback used by `AddDynamic`.
    fn try_string_concat(a: &ValueWord, b: &ValueWord) -> Option<ValueWord> {
        let (Some(a_heap), Some(b_heap)) = (unsafe {
            raw_helpers::extract_heap_ref(a.raw_bits())
        }, unsafe {
            raw_helpers::extract_heap_ref(b.raw_bits())
        }) else {
            return None;
        };
        let s = match (a_heap, b_heap) {
            (HeapValue::String(x), HeapValue::String(y)) => format!("{}{}", x, y),
            (HeapValue::String(x), HeapValue::Char(c)) => format!("{}{}", x, c),
            (HeapValue::Char(c), HeapValue::String(y)) => format!("{}{}", c, y),
            (HeapValue::Char(a), HeapValue::Char(b)) => format!("{}{}", a, b),
            _ => return None,
        };
        Some(ValueWord::from_string(Arc::new(s)))
    }

    /// Heap-typed arithmetic paths that remain live after R5.6.
    ///
    /// After R5.1-R5.5 retargeted the load-bearing typed cases (typed
    /// bitwise, user operator traits, DateTime, Matrix/Vec, string+scalar),
    /// the only arms that still legitimately fire here are the ones the
    /// compiler does not yet retarget:
    ///
    ///   * `Vec<int> + Vec<number>` / `Vec<number> + Vec<int>` — mixed-
    ///     kind promotion, not in R5.4's retarget set.
    ///   * `Matrix * Vec<number>` — mixed heap-kind dispatch.
    ///   * `Vec<number> op scalar` / `scalar op Vec<number>` broadcast.
    ///   * `Matrix * scalar` / `scalar * Matrix` broadcast.
    ///   * `string + scalar` residual paths (flag-off, commutative
    ///     `scalar + string`, and untyped-param sites); see R5.5.
    ///
    /// Reference: /home/dev/.claude/plans/v2-residuals-closeout.md §R5.6.
    fn try_heap_arithmetic(
        &mut self,
        op: HeapBinOp,
        a: &ValueWord,
        b: &ValueWord,
    ) -> Result<Option<ValueWord>, VMError> {
        use HeapBinOp::*;
        let ah = unsafe { raw_helpers::extract_heap_ref(a.raw_bits()) };
        let bh = unsafe { raw_helpers::extract_heap_ref(b.raw_bits()) };
        // Case 1: both heap — remaining live mixed-kind shapes.
        if let (Some(ah), Some(bh)) = (ah, bh) {
            match (op, ah, bh) {
                // DateTime +/- TimeSpan / TimeSpan +/- TimeSpan / DateTime - DateTime.
                //
                // R5.3B's typed-retarget fires for let-locals and typed function
                // parameters, but not for untyped function returns (where
                // `make_dt()` returns a DateTime without the compiler tracking
                // the return type). These arms catch that residual case.
                (
                    Add,
                    HeapValue::Temporal(TemporalData::DateTime(dt)),
                    HeapValue::Temporal(TemporalData::TimeSpan(dur)),
                )
                | (
                    Add,
                    HeapValue::Temporal(TemporalData::TimeSpan(dur)),
                    HeapValue::Temporal(TemporalData::DateTime(dt)),
                ) => {
                    let out = dt.checked_add_signed(*dur).ok_or_else(|| {
                        VMError::RuntimeError("DateTime overflow in addition".into())
                    })?;
                    return Ok(Some(ValueWord::from_time(out)));
                }
                (
                    Add,
                    HeapValue::Temporal(TemporalData::TimeSpan(a_dur)),
                    HeapValue::Temporal(TemporalData::TimeSpan(b_dur)),
                ) => {
                    let out = a_dur.checked_add(b_dur).ok_or_else(|| {
                        VMError::RuntimeError("Duration overflow in addition".into())
                    })?;
                    return Ok(Some(ValueWord::from_timespan(out)));
                }
                (
                    Sub,
                    HeapValue::Temporal(TemporalData::DateTime(dt)),
                    HeapValue::Temporal(TemporalData::TimeSpan(dur)),
                ) => {
                    let out = dt.checked_sub_signed(*dur).ok_or_else(|| {
                        VMError::RuntimeError("DateTime overflow in subtraction".into())
                    })?;
                    return Ok(Some(ValueWord::from_time(out)));
                }
                (
                    Sub,
                    HeapValue::Temporal(TemporalData::DateTime(a_dt)),
                    HeapValue::Temporal(TemporalData::DateTime(b_dt)),
                ) => {
                    return Ok(Some(ValueWord::from_timespan(a_dt.signed_duration_since(*b_dt))));
                }
                (
                    Sub,
                    HeapValue::Temporal(TemporalData::TimeSpan(a_dur)),
                    HeapValue::Temporal(TemporalData::TimeSpan(b_dur)),
                ) => {
                    let out = a_dur.checked_sub(b_dur).ok_or_else(|| {
                        VMError::RuntimeError("Duration overflow in subtraction".into())
                    })?;
                    return Ok(Some(ValueWord::from_timespan(out)));
                }
                // Vec<int> + Vec<number> / Vec<number> + Vec<int> — promote to f64
                (
                    Add,
                    HeapValue::TypedArray(TypedArrayData::I64(a_arr)),
                    HeapValue::TypedArray(TypedArrayData::F64(b_arr)),
                ) => {
                    if a_arr.len() != b_arr.len() {
                        return Err(VMError::RuntimeError(format!(
                            "Vec length mismatch: {} vs {}", a_arr.len(), b_arr.len()
                        )));
                    }
                    let af = shape_runtime::intrinsics::vector::i64_slice_to_f64(a_arr.as_slice());
                    let r = shape_runtime::intrinsics::vector::simd_vec_add_f64(&af, b_arr.as_slice());
                    return Ok(Some(ValueWord::from_float_array(Arc::new(r.into()))));
                }
                (
                    Add,
                    HeapValue::TypedArray(TypedArrayData::F64(a_arr)),
                    HeapValue::TypedArray(TypedArrayData::I64(b_arr)),
                ) => {
                    if a_arr.len() != b_arr.len() {
                        return Err(VMError::RuntimeError(format!(
                            "Vec length mismatch: {} vs {}", a_arr.len(), b_arr.len()
                        )));
                    }
                    let bf = shape_runtime::intrinsics::vector::i64_slice_to_f64(b_arr.as_slice());
                    let r = shape_runtime::intrinsics::vector::simd_vec_add_f64(a_arr.as_slice(), &bf);
                    return Ok(Some(ValueWord::from_float_array(Arc::new(r.into()))));
                }
                // Matrix * Vec<number>
                (
                    Mul,
                    HeapValue::TypedArray(TypedArrayData::Matrix(a_mat)),
                    HeapValue::TypedArray(TypedArrayData::F64(b_arr)),
                ) => {
                    let r = shape_runtime::intrinsics::matrix_kernels::matrix_matvec(
                        a_mat, b_arr.as_slice(),
                    )
                    .map_err(VMError::RuntimeError)?;
                    return Ok(Some(ValueWord::from_float_array(Arc::new(r.into()))));
                }
                _ => {}
            }
        }
        // Case 2: one heap, one scalar (broadcast / coerce).
        if let Some(ah) = ah {
            match (op, ah) {
                // Vec<number> op scalar — broadcast SIMD
                (_, HeapValue::TypedArray(TypedArrayData::F64(a_arr))) => {
                    if let Some(s) = b.as_number_coerce() {
                        let bv = vec![s; a_arr.len()];
                        let r = match op {
                            Add => shape_runtime::intrinsics::vector::simd_vec_add_f64(a_arr.as_slice(), &bv),
                            Sub => shape_runtime::intrinsics::vector::simd_vec_sub_f64(a_arr.as_slice(), &bv),
                            Mul => shape_runtime::intrinsics::vector::simd_vec_mul_f64(a_arr.as_slice(), &bv),
                            Div => shape_runtime::intrinsics::vector::simd_vec_div_f64(a_arr.as_slice(), &bv),
                            _ => return Ok(None),
                        };
                        return Ok(Some(ValueWord::from_float_array(Arc::new(r.into()))));
                    }
                }
                // Matrix op scalar (right scalar)
                (_, HeapValue::TypedArray(TypedArrayData::Matrix(a_mat))) => {
                    if let Some(s) = b.as_number_coerce() {
                        let r = match op {
                            Mul => shape_runtime::intrinsics::matrix_kernels::matrix_scale(a_mat, s),
                            _ => return Ok(None),
                        };
                        return Ok(Some(ValueWord::from_matrix(Arc::new(r))));
                    }
                }
                // string + scalar (int/number) concat.
                //
                // R5.5 retargeted the proved `string` LHS + `int` / `number`
                // / `bool` RHS path to dedicated `StringConcatInt` /
                // `StringConcatNumber` / `StringConcatBool` opcodes. This
                // arm remains reachable for:
                //   - `SHAPE_V2_STRING_COERCE_CONCAT=0` (flag-off fallback).
                //   - Commutative `scalar + string` (typed path only covers
                //     string-LHS).
                //   - Paths where the compiler fails to resolve the operand
                //     type name (untyped params / certain generic contexts).
                (Add, HeapValue::String(s)) => {
                    if let Some(i) = b.as_i64() {
                        return Ok(Some(ValueWord::from_string(Arc::new(format!("{}{}", s, i)))));
                    }
                    if let Some(n) = b.as_f64() {
                        let n_str = if n.fract() == 0.0 {
                            format!("{}", n as i64)
                        } else {
                            format!("{}", n)
                        };
                        return Ok(Some(ValueWord::from_string(Arc::new(format!("{}{}", s, n_str)))));
                    }
                }
                _ => {}
            }
        }
        if let Some(bh) = bh {
            match (op, bh) {
                // scalar op Vec<number> — broadcast SIMD
                (_, HeapValue::TypedArray(TypedArrayData::F64(b_arr))) => {
                    if let Some(s) = a.as_number_coerce() {
                        let av = vec![s; b_arr.len()];
                        let r = match op {
                            Add => shape_runtime::intrinsics::vector::simd_vec_add_f64(&av, b_arr.as_slice()),
                            Sub => shape_runtime::intrinsics::vector::simd_vec_sub_f64(&av, b_arr.as_slice()),
                            Mul => shape_runtime::intrinsics::vector::simd_vec_mul_f64(&av, b_arr.as_slice()),
                            Div => shape_runtime::intrinsics::vector::simd_vec_div_f64(&av, b_arr.as_slice()),
                            _ => return Ok(None),
                        };
                        return Ok(Some(ValueWord::from_float_array(Arc::new(r.into()))));
                    }
                }
                // scalar * Matrix (left scalar)
                (Mul, HeapValue::TypedArray(TypedArrayData::Matrix(b_mat))) => {
                    if let Some(s) = a.as_number_coerce() {
                        let r = shape_runtime::intrinsics::matrix_kernels::matrix_scale(b_mat, s);
                        return Ok(Some(ValueWord::from_matrix(Arc::new(r))));
                    }
                }
                _ => {}
            }
        }
        Ok(None)
    }

    /// Bitwise binary op fallback; int+int only.
    fn exec_dyn_bit_binary(&mut self, op: OpCode) -> Result<(), VMError> {
        use OpCode::*;
        let b = self.pop_raw_u64()?;
        let a = self.pop_raw_u64()?;
        let (Some(a_int), Some(b_int)) = (a.as_i64(), b.as_i64()) else {
            let name = match op {
                BitXor => "XOR",
                BitAnd => "AND",
                BitOr => "OR",
                BitShl => "shift left",
                BitShr => "shift right",
                _ => "bitwise op",
            };
            return Err(VMError::RuntimeError(format!(
                "Bitwise {} requires integer operands, got {} and {}",
                name,
                a.type_name(),
                b.type_name()
            )));
        };
        let result = match op {
            BitXor => a_int ^ b_int,
            BitAnd => a_int & b_int,
            BitOr => a_int | b_int,
            BitShl => a_int << b_int,
            BitShr => a_int >> b_int,
            _ => unreachable!(),
        };
        self.push_raw_u64(ValueWord::from_i64(result))
    }

    /// Bitwise NOT fallback; int only.
    fn exec_dyn_bit_unary(&mut self) -> Result<(), VMError> {
        let a = self.pop_raw_u64()?;
        let Some(a_int) = a.as_i64() else {
            return Err(VMError::RuntimeError(format!(
                "Bitwise NOT requires integer operand, got {}",
                a.type_name()
            )));
        };
        self.push_raw_u64(ValueWord::from_i64(!a_int))
    }

}

#[cfg(test)]
mod tests {
    use crate::VMConfig;
    use crate::bytecode::*;
    use crate::executor::VirtualMachine;
    use shape_value::{VMError, ValueWord, ValueWordExt};

    #[test]
    fn test_string_concatenation() {
        let mut program = BytecodeProgram::default();
        let c0 = program.add_constant(Constant::String("hello ".to_string()));
        let c1 = program.add_constant(Constant::String("world".to_string()));
        program.instructions = vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c0))),
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c1))),
            Instruction::simple(OpCode::AddDynamic),
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
        assert_eq!(result.as_u64_value(), Some(u64::MAX));
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

    // ===== Raw typed stack API tests =====

    /// Helper: create a VM with a dummy program loaded so the stack is usable.
    fn make_raw_vm() -> VirtualMachine {
        let program = BytecodeProgram::default();
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(program);
        vm
    }

    /// Helper: push two i64 values, execute a typed arithmetic instruction, pop the result.
    fn exec_typed_int_binop(a: i64, b: i64, opcode: OpCode) -> i64 {
        let mut vm = make_raw_vm();
        vm.push_raw_i64(a).unwrap();
        vm.push_raw_i64(b).unwrap();
        let instr = Instruction::simple(opcode);
        vm.exec_typed_arithmetic(&instr).unwrap();
        vm.pop_raw_i64().unwrap()
    }

    /// Helper: push two f64 values, execute a typed arithmetic instruction, pop the result.
    fn exec_typed_f64_binop(a: f64, b: f64, opcode: OpCode) -> f64 {
        let mut vm = make_raw_vm();
        vm.push_raw_f64(a).unwrap();
        vm.push_raw_f64(b).unwrap();
        let instr = Instruction::simple(opcode);
        vm.exec_typed_arithmetic(&instr).unwrap();
        vm.pop_raw_f64().unwrap()
    }

    #[test]
    fn test_raw_i64_roundtrip() {
        let mut vm = make_raw_vm();
        vm.push_raw_i64(42).unwrap();
        assert_eq!(vm.pop_raw_i64().unwrap(), 42);
    }

    #[test]
    fn test_raw_i64_negative_roundtrip() {
        let mut vm = make_raw_vm();
        vm.push_raw_i64(-123).unwrap();
        assert_eq!(vm.pop_raw_i64().unwrap(), -123);
    }

    #[test]
    fn test_raw_f64_roundtrip() {
        let mut vm = make_raw_vm();
        vm.push_raw_f64(3.14).unwrap();
        assert!((vm.pop_raw_f64().unwrap() - 3.14).abs() < 1e-15);
    }

    #[test]
    fn test_raw_f64_negative_roundtrip() {
        let mut vm = make_raw_vm();
        vm.push_raw_f64(-2.5).unwrap();
        assert!((vm.pop_raw_f64().unwrap() - (-2.5)).abs() < 1e-15);
    }

    #[test]
    fn test_raw_stack_underflow_i64() {
        let mut vm = make_raw_vm();
        assert!(vm.pop_raw_i64().is_err());
    }

    #[test]
    fn test_raw_stack_underflow_f64() {
        let mut vm = make_raw_vm();
        assert!(vm.pop_raw_f64().is_err());
    }

    // --- Typed AddInt via raw API ---

    #[test]
    fn test_typed_arithmetic_add_int() {
        assert_eq!(exec_typed_int_binop(5, 3, OpCode::AddInt), 8);
    }

    #[test]
    fn test_typed_arithmetic_add_int_negative() {
        assert_eq!(exec_typed_int_binop(-10, 7, OpCode::AddInt), -3);
    }

    #[test]
    fn test_typed_arithmetic_add_int_zero() {
        assert_eq!(exec_typed_int_binop(0, 0, OpCode::AddInt), 0);
    }

    // --- Typed SubInt via raw API ---

    #[test]
    fn test_typed_arithmetic_sub_int() {
        assert_eq!(exec_typed_int_binop(10, 4, OpCode::SubInt), 6);
    }

    #[test]
    fn test_typed_arithmetic_sub_int_negative_result() {
        assert_eq!(exec_typed_int_binop(3, 8, OpCode::SubInt), -5);
    }

    // --- Typed MulInt via raw API ---

    #[test]
    fn test_typed_arithmetic_mul_int() {
        assert_eq!(exec_typed_int_binop(6, 7, OpCode::MulInt), 42);
    }

    #[test]
    fn test_typed_arithmetic_mul_int_negative() {
        assert_eq!(exec_typed_int_binop(-3, 4, OpCode::MulInt), -12);
    }

    #[test]
    fn test_typed_arithmetic_mul_int_zero() {
        assert_eq!(exec_typed_int_binop(12345, 0, OpCode::MulInt), 0);
    }

    // --- Typed DivInt via raw API ---

    #[test]
    fn test_typed_arithmetic_div_int() {
        assert_eq!(exec_typed_int_binop(20, 4, OpCode::DivInt), 5);
    }

    #[test]
    fn test_typed_arithmetic_div_int_truncation() {
        assert_eq!(exec_typed_int_binop(7, 2, OpCode::DivInt), 3);
    }

    #[test]
    fn test_typed_arithmetic_div_int_by_zero() {
        let mut vm = make_raw_vm();
        vm.push_raw_i64(10).unwrap();
        vm.push_raw_i64(0).unwrap();
        let instr = Instruction::simple(OpCode::DivInt);
        let err = vm.exec_typed_arithmetic(&instr).unwrap_err();
        assert!(matches!(err, VMError::DivisionByZero));
    }

    // --- Typed ModInt via raw API ---

    #[test]
    fn test_typed_arithmetic_mod_int() {
        assert_eq!(exec_typed_int_binop(17, 5, OpCode::ModInt), 2);
    }

    #[test]
    fn test_typed_arithmetic_mod_int_by_zero() {
        let mut vm = make_raw_vm();
        vm.push_raw_i64(10).unwrap();
        vm.push_raw_i64(0).unwrap();
        let instr = Instruction::simple(OpCode::ModInt);
        let err = vm.exec_typed_arithmetic(&instr).unwrap_err();
        assert!(matches!(err, VMError::DivisionByZero));
    }

    // --- Typed PowInt via raw API ---

    #[test]
    fn test_typed_arithmetic_pow_int() {
        assert_eq!(exec_typed_int_binop(2, 10, OpCode::PowInt), 1024);
    }

    #[test]
    fn test_typed_arithmetic_pow_int_zero_exponent() {
        assert_eq!(exec_typed_int_binop(99, 0, OpCode::PowInt), 1);
    }

    // --- Typed AddNumber via raw API ---

    #[test]
    fn test_typed_arithmetic_add_number() {
        let result = exec_typed_f64_binop(2.5, 3.5, OpCode::AddNumber);
        assert!((result - 6.0).abs() < 1e-15);
    }

    // --- Typed SubNumber via raw API ---

    #[test]
    fn test_typed_arithmetic_sub_number() {
        let result = exec_typed_f64_binop(10.0, 3.5, OpCode::SubNumber);
        assert!((result - 6.5).abs() < 1e-15);
    }

    // --- Typed MulNumber via raw API ---

    #[test]
    fn test_typed_arithmetic_mul_number() {
        let result = exec_typed_f64_binop(3.0, 4.0, OpCode::MulNumber);
        assert!((result - 12.0).abs() < 1e-15);
    }

    // --- Typed DivNumber via raw API ---

    #[test]
    fn test_typed_arithmetic_div_number() {
        let result = exec_typed_f64_binop(10.0, 4.0, OpCode::DivNumber);
        assert!((result - 2.5).abs() < 1e-15);
    }

    #[test]
    fn test_typed_arithmetic_div_number_by_zero() {
        let mut vm = make_raw_vm();
        vm.push_raw_f64(10.0).unwrap();
        vm.push_raw_f64(0.0).unwrap();
        let instr = Instruction::simple(OpCode::DivNumber);
        let err = vm.exec_typed_arithmetic(&instr).unwrap_err();
        assert!(matches!(err, VMError::DivisionByZero));
    }

    // --- Typed ModNumber via raw API ---

    #[test]
    fn test_typed_arithmetic_mod_number() {
        let result = exec_typed_f64_binop(10.0, 3.0, OpCode::ModNumber);
        assert!((result - 1.0).abs() < 1e-15);
    }

    #[test]
    fn test_typed_arithmetic_mod_number_by_zero() {
        let mut vm = make_raw_vm();
        vm.push_raw_f64(10.0).unwrap();
        vm.push_raw_f64(0.0).unwrap();
        let instr = Instruction::simple(OpCode::ModNumber);
        let err = vm.exec_typed_arithmetic(&instr).unwrap_err();
        assert!(matches!(err, VMError::DivisionByZero));
    }

    // --- Typed PowNumber via raw API ---

    #[test]
    fn test_typed_arithmetic_pow_number() {
        let result = exec_typed_f64_binop(2.0, 10.0, OpCode::PowNumber);
        assert!((result - 1024.0).abs() < 1e-10);
    }

    // --- Coercion opcodes via raw API ---

    #[test]
    fn test_typed_arithmetic_int_to_number() {
        let mut vm = make_raw_vm();
        vm.push_raw_i64(42).unwrap();
        let instr = Instruction::simple(OpCode::IntToNumber);
        vm.exec_typed_arithmetic(&instr).unwrap();
        let result = vm.pop_raw_f64().unwrap();
        assert!((result - 42.0).abs() < 1e-15);
    }

    #[test]
    fn test_typed_arithmetic_number_to_int() {
        let mut vm = make_raw_vm();
        vm.push_raw_f64(7.9).unwrap();
        let instr = Instruction::simple(OpCode::NumberToInt);
        vm.exec_typed_arithmetic(&instr).unwrap();
        let result = vm.pop_raw_i64().unwrap();
        assert_eq!(result, 7);
    }

    // --- ValueWord compatibility: raw push produces valid ValueWord on pop ---

    #[test]
    fn test_raw_push_i64_pop_as_value_word() {
        let mut vm = make_raw_vm();
        vm.push_raw_i64(99).unwrap();
        let vw = vm.pop_raw_u64().unwrap();
        assert_eq!(vw.as_i64(), Some(99));
    }

    #[test]
    fn test_raw_push_f64_pop_as_value_word() {
        let mut vm = make_raw_vm();
        vm.push_raw_f64(1.5).unwrap();
        let vw = vm.pop_raw_u64().unwrap();
        assert!((vw.as_f64().unwrap() - 1.5).abs() < 1e-15);
    }

    #[test]
    fn test_value_word_push_pop_raw_i64() {
        let mut vm = make_raw_vm();
        vm.push_raw_u64(ValueWord::from_i64(77)).unwrap();
        assert_eq!(vm.pop_raw_i64().unwrap(), 77);
    }

    #[test]
    fn test_value_word_push_pop_raw_f64() {
        let mut vm = make_raw_vm();
        vm.push_raw_u64(ValueWord::from_f64(2.718)).unwrap();
        assert!((vm.pop_raw_f64().unwrap() - 2.718).abs() < 1e-15);
    }

    // ===== R5.1B: Typed bitwise opcodes via raw API =====
    //
    // These exercise the executor handlers added in R5.1B. Values stay
    // inside i48 range so `pop_raw_i64` / `push_raw_i64` round-trip
    // cleanly; the compiler will not emit these typed variants unless
    // both operands are proved to be `int` (i.e. i48-safe).

    // --- BitAndInt ---
    #[test]
    fn test_typed_arithmetic_bit_and_int() {
        assert_eq!(exec_typed_int_binop(0xF0, 0x0F, OpCode::BitAndInt), 0x00);
        assert_eq!(exec_typed_int_binop(0xFF, 0x0F, OpCode::BitAndInt), 0x0F);
    }

    #[test]
    fn test_typed_arithmetic_bit_and_int_negative() {
        // -1 & x == x for any x (two's-complement all-ones pattern).
        assert_eq!(exec_typed_int_binop(-1, 0x1234, OpCode::BitAndInt), 0x1234);
    }

    // --- BitOrInt ---
    #[test]
    fn test_typed_arithmetic_bit_or_int() {
        assert_eq!(exec_typed_int_binop(0xF0, 0x0F, OpCode::BitOrInt), 0xFF);
        assert_eq!(exec_typed_int_binop(0, 0, OpCode::BitOrInt), 0);
    }

    #[test]
    fn test_typed_arithmetic_bit_or_int_negative() {
        // 0 | -1 == -1.
        assert_eq!(exec_typed_int_binop(0, -1, OpCode::BitOrInt), -1);
    }

    // --- BitXorInt ---
    #[test]
    fn test_typed_arithmetic_bit_xor_int() {
        assert_eq!(exec_typed_int_binop(0xF0, 0x0F, OpCode::BitXorInt), 0xFF);
        assert_eq!(exec_typed_int_binop(0xFF, 0xFF, OpCode::BitXorInt), 0x00);
    }

    #[test]
    fn test_typed_arithmetic_bit_xor_int_self_is_zero() {
        // x ^ x == 0, even for negative x.
        assert_eq!(exec_typed_int_binop(-42, -42, OpCode::BitXorInt), 0);
    }

    // --- BitShlInt ---
    #[test]
    fn test_typed_arithmetic_bit_shl_int() {
        assert_eq!(exec_typed_int_binop(3, 2, OpCode::BitShlInt), 12);
        assert_eq!(exec_typed_int_binop(1, 10, OpCode::BitShlInt), 1024);
    }

    #[test]
    fn test_typed_arithmetic_bit_shl_int_zero_shift() {
        // << 0 is identity.
        assert_eq!(exec_typed_int_binop(12345, 0, OpCode::BitShlInt), 12345);
    }

    // --- BitShrInt (arithmetic right shift) ---
    #[test]
    fn test_typed_arithmetic_bit_shr_int() {
        assert_eq!(exec_typed_int_binop(12, 2, OpCode::BitShrInt), 3);
        assert_eq!(exec_typed_int_binop(1024, 10, OpCode::BitShrInt), 1);
    }

    #[test]
    fn test_typed_arithmetic_bit_shr_int_arithmetic_sign_extends() {
        // Arithmetic right shift on a negative value preserves the sign
        // bit -- matches the `a_int >> b_int` used by the dynamic BitShr
        // handler (i64 `>>` is arithmetic).
        assert_eq!(exec_typed_int_binop(-8, 1, OpCode::BitShrInt), -4);
        assert_eq!(exec_typed_int_binop(-1, 1, OpCode::BitShrInt), -1);
    }

    // --- BitNotInt (unary) ---
    #[test]
    fn test_typed_arithmetic_bit_not_int() {
        // !0 == -1, !(-1) == 0 (two's complement).
        let mut vm = make_raw_vm();
        vm.push_raw_i64(0).unwrap();
        let instr = Instruction::simple(OpCode::BitNotInt);
        vm.exec_typed_arithmetic(&instr).unwrap();
        assert_eq!(vm.pop_raw_i64().unwrap(), -1);

        let mut vm = make_raw_vm();
        vm.push_raw_i64(-1).unwrap();
        let instr = Instruction::simple(OpCode::BitNotInt);
        vm.exec_typed_arithmetic(&instr).unwrap();
        assert_eq!(vm.pop_raw_i64().unwrap(), 0);
    }

    #[test]
    fn test_typed_arithmetic_bit_not_int_involution() {
        // !!x == x for any x.
        let mut vm = make_raw_vm();
        vm.push_raw_i64(0x1234_5678).unwrap();
        let instr = Instruction::simple(OpCode::BitNotInt);
        vm.exec_typed_arithmetic(&instr).unwrap();
        vm.exec_typed_arithmetic(&instr).unwrap();
        assert_eq!(vm.pop_raw_i64().unwrap(), 0x1234_5678);
    }

    // --- End-to-end through dispatch (BytecodeProgram -> vm.execute) ---
    //
    // Mirrors the legacy `test_bitwise_*` tests further up in this file
    // but uses the new R5.1B typed opcodes. Confirms the dispatch arm in
    // `execute_instruction` routes the opcode to exec_typed_arithmetic.

    #[test]
    fn test_bit_and_int_end_to_end() {
        let mut program = BytecodeProgram::default();
        let c0 = program.add_constant(Constant::Int(0xF0));
        let c1 = program.add_constant(Constant::Int(0x0F));
        program.instructions = vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c0))),
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c1))),
            Instruction::simple(OpCode::BitAndInt),
            Instruction::simple(OpCode::Halt),
        ];
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(program);
        let result = vm.execute(None).unwrap().clone();
        assert_eq!(result, ValueWord::from_i64(0x00));
    }

    #[test]
    fn test_bit_shl_int_end_to_end() {
        let mut program = BytecodeProgram::default();
        let c0 = program.add_constant(Constant::Int(3));
        let c1 = program.add_constant(Constant::Int(2));
        program.instructions = vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c0))),
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c1))),
            Instruction::simple(OpCode::BitShlInt),
            Instruction::simple(OpCode::Halt),
        ];
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(program);
        let result = vm.execute(None).unwrap().clone();
        assert_eq!(result, ValueWord::from_i64(12));
    }

    #[test]
    fn test_bit_not_int_end_to_end() {
        let mut program = BytecodeProgram::default();
        let c0 = program.add_constant(Constant::Int(0));
        program.instructions = vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(c0))),
            Instruction::simple(OpCode::BitNotInt),
            Instruction::simple(OpCode::Halt),
        ];
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(program);
        let result = vm.execute(None).unwrap().clone();
        assert_eq!(result, ValueWord::from_i64(-1));
    }
}
