//! Arithmetic operations for the VM executor (ADR-006 §2.7.7 / Q9 — kinded stack).
//!
//! Handles: Add, Sub, Mul, Div, Mod, Neg, Pow (typed variants per primitive
//! kind: Int/Number/Decimal), bitwise int ops, numeric coercion
//! (`IntToNumber` / `NumberToInt`), and the compact / width-parameterised
//! opcode family (`AddTyped` .. `CmpTyped`, `CastWidth`).
//!
//! Wave 6.5 substep-2 (Cluster A): every push/pop now threads through the
//! kinded API (`push_kinded(bits, kind)` / `pop_kinded()`). Result kind for
//! each opcode is sourced from the opcode-name suffix per playbook §2:
//!
//! - `*Int` family → `NativeKind::Int64`
//! - `*Number` family → `NativeKind::Float64`
//! - `*Decimal` family → `NativeKind::Ptr(HeapKind::Decimal)`
//! - `Bit*Int` family → `NativeKind::Int64`
//! - `IntToNumber` → `NativeKind::Float64`; `NumberToInt` → `NativeKind::Int64`
//! - `*Typed` (compact) family → kind from the operand `Width` (integer
//!   widths → `Int64`; F32/F64 → `Float64`).
//! - `CmpTyped` → always `NativeKind::Int64` (ordinal -1/0/1, not a bool).
//! - `CastWidth` → `NativeKind::Int64` (truncated to declared width).
//!
//! The pre-Wave-6 dual-path tag detectors (the i48 / f64 stack-top probes)
//! and ValueWord-based mixed-domain coercion
//! (`numeric_binary_result`) are gone — the compiler emits typed opcodes
//! when types are proven; cross-domain mixing arrives only at the Number
//! family, where Int operands are widened to f64 via `coerce_to_f64_kinded`.
//! Decimal arithmetic always operates on heap-backed `Arc<Decimal>` per
//! ADR-005 §1 single-discriminator.

use crate::{
    bytecode::{Instruction, NumericWidth, OpCode, Operand},
    executor::vm_impl::stack::drop_with_kind,
    executor::VirtualMachine,
};
use shape_value::{
    NativeKind, VMError,
    heap_value::{HeapKind, HeapValue},
};
use std::sync::Arc;

use crate::constants::EXACT_F64_INT_LIMIT;

/// Check if an i64 result fits in the I48 inline range. Values outside this
/// range would have been heap-boxed as BigInt under the v1 `ValueWord`
/// encoding; under the kinded ABI we promote to f64 instead, preserving the
/// legacy overflow semantics.
#[inline(always)]
fn fits_i48(v: i64) -> bool {
    // I48_MIN = -(1<<47), I48_MAX = (1<<47)-1
    const I48_MIN: i64 = -(1i64 << 47);
    const I48_MAX: i64 = (1i64 << 47) - 1;
    (I48_MIN..=I48_MAX).contains(&v)
}

#[inline(always)]
fn arith_i128_to_lossless_f64(value: i128) -> Option<f64> {
    if (-EXACT_F64_INT_LIMIT..=EXACT_F64_INT_LIMIT).contains(&value) {
        Some(value as f64)
    } else {
        None
    }
}

/// Coerce a `(bits, kind)` pair to `f64` if the kind is `Float64` or any
/// integer-family. Returns `None` for non-numeric kinds.
#[inline]
fn coerce_to_f64_kinded(bits: u64, kind: NativeKind) -> Option<f64> {
    match kind {
        NativeKind::Float64 | NativeKind::NullableFloat64 => Some(f64::from_bits(bits)),
        NativeKind::Int8
        | NativeKind::Int16
        | NativeKind::Int32
        | NativeKind::Int64
        | NativeKind::IntSize => Some(bits as i64 as f64),
        NativeKind::UInt8
        | NativeKind::UInt16
        | NativeKind::UInt32
        | NativeKind::UInt64
        | NativeKind::UIntSize => Some(bits as f64),
        _ => None,
    }
}

/// Read the borrowed `Decimal` payload behind a `Ptr(HeapKind::Decimal)`-kinded
/// operand. The slot's `bits` are `Arc::into_raw(Arc<rust_decimal::Decimal>)`
/// per `KindedSlot::from_decimal`.
#[inline]
fn decimal_ref<'a>(bits: u64, kind: NativeKind) -> Option<&'a rust_decimal::Decimal> {
    if !matches!(kind, NativeKind::Ptr(HeapKind::Decimal)) || bits == 0 {
        return None;
    }
    let ptr = bits as *const rust_decimal::Decimal;
    Some(unsafe { &*ptr })
}

/// Push a freshly-constructed `Arc<Decimal>` as a `Ptr(HeapKind::Decimal)`
/// kinded slot. The caller transfers one strong-count share.
#[inline]
fn push_decimal(vm: &mut VirtualMachine, d: rust_decimal::Decimal) -> Result<(), VMError> {
    let arc = Arc::new(d);
    let bits = Arc::into_raw(arc) as u64;
    vm.push_kinded(bits, NativeKind::Ptr(HeapKind::Decimal))
}

impl VirtualMachine {
    /// Execute typed arithmetic opcodes (compiler-guaranteed types, zero dispatch).
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
            // ===== Typed Add (single-path native i64) =====
            //
            // The pre-Wave-6 dual-path tag-probe detector is gone.
            // AddInt unconditionally pops two Int64-kinded slots,
            // checks for overflow against the i48 inline range, and
            // promotes to f64 on overflow.
            AddInt => self.binop_int_with_f64_overflow(|a, b| a.checked_add(b), |a, b| a as f64 + b as f64)?,
            SubInt => self.binop_int_with_f64_overflow(|a, b| a.checked_sub(b), |a, b| a as f64 - b as f64)?,
            MulInt => self.binop_int_with_f64_overflow(|a, b| a.checked_mul(b), |a, b| a as f64 * b as f64)?,
            DivInt => {
                let (b_bits, _b_kind) = self.pop_kinded()?;
                let (a_bits, _a_kind) = self.pop_kinded()?;
                let bi = b_bits as i64;
                let ai = a_bits as i64;
                if bi == 0 {
                    return Err(VMError::DivisionByZero);
                }
                self.push_kinded((ai / bi) as u64, NativeKind::Int64)?;
            }
            ModInt => {
                let (b_bits, _b_kind) = self.pop_kinded()?;
                let (a_bits, _a_kind) = self.pop_kinded()?;
                let bi = b_bits as i64;
                let ai = a_bits as i64;
                if bi == 0 {
                    return Err(VMError::DivisionByZero);
                }
                self.push_kinded((ai % bi) as u64, NativeKind::Int64)?;
            }
            PowInt => {
                let (b_bits, _b_kind) = self.pop_kinded()?;
                let (a_bits, _a_kind) = self.pop_kinded()?;
                let exp = b_bits as i64;
                let base = a_bits as i64;
                if exp >= 0 && exp < u32::MAX as i64 {
                    let result = base.pow(exp as u32);
                    if fits_i48(result) {
                        self.push_kinded(result as u64, NativeKind::Int64)?;
                    } else {
                        self.push_kinded((result as f64).to_bits(), NativeKind::Float64)?;
                    }
                } else {
                    let result = (base as f64).powf(exp as f64);
                    self.push_kinded(result.to_bits(), NativeKind::Float64)?;
                }
            }
            // ===== Typed Number family — kind-aware Int→f64 widen =====
            AddNumber => self.binop_number_kinded(|a, b| a + b)?,
            SubNumber => self.binop_number_kinded(|a, b| a - b)?,
            MulNumber => self.binop_number_kinded(|a, b| a * b)?,
            DivNumber => self.divmod_number_kinded(|a, b| a / b)?,
            ModNumber => self.divmod_number_kinded(|a, b| a % b)?,
            PowNumber => self.binop_number_kinded(|a, b| a.powf(b))?,
            // ===== Typed Decimal family — heap-backed Arc<Decimal> =====
            AddDecimal => self.binop_decimal_kinded(|a, b| a + b)?,
            SubDecimal => self.binop_decimal_kinded(|a, b| a - b)?,
            MulDecimal => self.binop_decimal_kinded(|a, b| a * b)?,
            DivDecimal => self.divmod_decimal_kinded(|a, b| a / b)?,
            ModDecimal => self.divmod_decimal_kinded(|a, b| a % b)?,
            PowDecimal => {
                let (b_bits, b_kind) = self.pop_kinded()?;
                let (a_bits, a_kind) = self.pop_kinded()?;
                use rust_decimal::prelude::{FromPrimitive, ToPrimitive};
                let result = match (decimal_ref(a_bits, a_kind), decimal_ref(b_bits, b_kind)) {
                    (Some(base), Some(exp)) => {
                        let r = base.to_f64().unwrap_or(0.0).powf(exp.to_f64().unwrap_or(0.0));
                        rust_decimal::Decimal::from_f64(r).unwrap_or_default()
                    }
                    _ => rust_decimal::Decimal::default(),
                };
                drop_with_kind(a_bits, a_kind);
                drop_with_kind(b_bits, b_kind);
                push_decimal(self, result)?;
            }
            // ===== Numeric Coercion =====
            IntToNumber => {
                let (bits, _kind) = self.pop_kinded()?;
                let v = bits as i64;
                self.push_kinded((v as f64).to_bits(), NativeKind::Float64)?;
            }
            NumberToInt => {
                let (bits, _kind) = self.pop_kinded()?;
                let v = f64::from_bits(bits);
                self.push_kinded((v as i64) as u64, NativeKind::Int64)?;
            }
            // ===== Negation =====
            NegInt => {
                let (bits, _kind) = self.pop_kinded()?;
                let v = bits as i64;
                self.push_kinded((-v) as u64, NativeKind::Int64)?;
            }
            NegNumber => {
                let (bits, kind) = self.pop_kinded()?;
                let v = coerce_to_f64_kinded(bits, kind).ok_or_else(|| VMError::TypeError {
                    expected: "number",
                    got: kind_type_name(kind),
                })?;
                drop_with_kind(bits, kind);
                self.push_kinded((-v).to_bits(), NativeKind::Float64)?;
            }
            NegDecimal => {
                let (bits, kind) = self.pop_kinded()?;
                let result = decimal_ref(bits, kind).map(|d| -*d).unwrap_or_default();
                drop_with_kind(bits, kind);
                push_decimal(self, result)?;
            }
            // ===== Typed bitwise =====
            BitAndInt => self.binop_int_simple(|a, b| a & b)?,
            BitOrInt => self.binop_int_simple(|a, b| a | b)?,
            BitXorInt => self.binop_int_simple(|a, b| a ^ b)?,
            BitShlInt => self.binop_int_simple(|a, b| a << b)?,
            BitShrInt => self.binop_int_simple(|a, b| a >> b)?,
            BitNotInt => {
                let (bits, _kind) = self.pop_kinded()?;
                let a = bits as i64;
                self.push_kinded((!a) as u64, NativeKind::Int64)?;
            }
            _ => unreachable!(
                "exec_typed_arithmetic called with non-typed-arithmetic opcode: {:?}",
                instruction.opcode
            ),
        }
        Ok(())
    }

    /// Int-int binary op with checked-overflow → f64 fallback (Add/Sub/Mul).
    #[inline(always)]
    fn binop_int_with_f64_overflow(
        &mut self,
        checked: impl FnOnce(i64, i64) -> Option<i64>,
        overflow: impl FnOnce(i64, i64) -> f64,
    ) -> Result<(), VMError> {
        let (b_bits, _b_kind) = self.pop_kinded()?;
        let (a_bits, _a_kind) = self.pop_kinded()?;
        let bi = b_bits as i64;
        let ai = a_bits as i64;
        match checked(ai, bi) {
            Some(result) if fits_i48(result) => {
                self.push_kinded(result as u64, NativeKind::Int64)
            }
            _ => self.push_kinded(overflow(ai, bi).to_bits(), NativeKind::Float64),
        }
    }

    /// Int-int binary op with no overflow gate (BitAnd/BitOr/BitXor/BitShl/BitShr).
    #[inline(always)]
    fn binop_int_simple(&mut self, op: impl FnOnce(i64, i64) -> i64) -> Result<(), VMError> {
        let (b_bits, _b_kind) = self.pop_kinded()?;
        let (a_bits, _a_kind) = self.pop_kinded()?;
        let bi = b_bits as i64;
        let ai = a_bits as i64;
        self.push_kinded(op(ai, bi) as u64, NativeKind::Int64)
    }

    /// Number-family binary op: kind-aware coercion (Int→f64 widen).
    #[inline(always)]
    fn binop_number_kinded(&mut self, op: impl FnOnce(f64, f64) -> f64) -> Result<(), VMError> {
        let (b_bits, b_kind) = self.pop_kinded()?;
        let (a_bits, a_kind) = self.pop_kinded()?;
        let lhs = coerce_to_f64_kinded(a_bits, a_kind).ok_or_else(|| VMError::TypeError {
            expected: "number",
            got: kind_type_name(a_kind),
        });
        let rhs = coerce_to_f64_kinded(b_bits, b_kind).ok_or_else(|| VMError::TypeError {
            expected: "number",
            got: kind_type_name(b_kind),
        });
        drop_with_kind(a_bits, a_kind);
        drop_with_kind(b_bits, b_kind);
        let result = op(lhs?, rhs?);
        self.push_kinded(result.to_bits(), NativeKind::Float64)
    }

    /// Number-family div/mod with zero check.
    #[inline(always)]
    fn divmod_number_kinded(&mut self, op: impl FnOnce(f64, f64) -> f64) -> Result<(), VMError> {
        let (b_bits, b_kind) = self.pop_kinded()?;
        let (a_bits, a_kind) = self.pop_kinded()?;
        let lhs = coerce_to_f64_kinded(a_bits, a_kind).ok_or_else(|| VMError::TypeError {
            expected: "number",
            got: kind_type_name(a_kind),
        });
        let rhs = coerce_to_f64_kinded(b_bits, b_kind).ok_or_else(|| VMError::TypeError {
            expected: "number",
            got: kind_type_name(b_kind),
        });
        drop_with_kind(a_bits, a_kind);
        drop_with_kind(b_bits, b_kind);
        let l = lhs?;
        let r = rhs?;
        if r == 0.0 {
            return Err(VMError::DivisionByZero);
        }
        self.push_kinded(op(l, r).to_bits(), NativeKind::Float64)
    }

    /// Decimal-family binary op (Add/Sub/Mul).
    #[inline(always)]
    fn binop_decimal_kinded(
        &mut self,
        op: impl FnOnce(rust_decimal::Decimal, rust_decimal::Decimal) -> rust_decimal::Decimal,
    ) -> Result<(), VMError> {
        let (b_bits, b_kind) = self.pop_kinded()?;
        let (a_bits, a_kind) = self.pop_kinded()?;
        let result = match (decimal_ref(a_bits, a_kind), decimal_ref(b_bits, b_kind)) {
            (Some(ad), Some(bd)) => op(*ad, *bd),
            _ => rust_decimal::Decimal::default(),
        };
        drop_with_kind(a_bits, a_kind);
        drop_with_kind(b_bits, b_kind);
        push_decimal(self, result)
    }

    /// Decimal-family div/mod with zero-check.
    #[inline(always)]
    fn divmod_decimal_kinded(
        &mut self,
        op: impl FnOnce(rust_decimal::Decimal, rust_decimal::Decimal) -> rust_decimal::Decimal,
    ) -> Result<(), VMError> {
        let (b_bits, b_kind) = self.pop_kinded()?;
        let (a_bits, a_kind) = self.pop_kinded()?;
        let result_or_err = match (decimal_ref(a_bits, a_kind), decimal_ref(b_bits, b_kind)) {
            (Some(ad), Some(bd)) => {
                if bd.is_zero() {
                    Err(VMError::DivisionByZero)
                } else {
                    Ok(op(*ad, *bd))
                }
            }
            _ => Ok(rust_decimal::Decimal::default()),
        };
        drop_with_kind(a_bits, a_kind);
        drop_with_kind(b_bits, b_kind);
        push_decimal(self, result_or_err?)
    }

    // ---------------------------------------------------------------
    // Compact typed opcodes (ABI-stable, width-parameterised)
    // ---------------------------------------------------------------

    /// Execute a compact typed arithmetic opcode (`AddTyped` .. `ModTyped`,
    /// `CmpTyped`).
    pub(in crate::executor) fn exec_compact_typed_arithmetic(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
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

    fn exec_compact_add(&mut self, width: NumericWidth) -> Result<(), VMError> {
        if width.is_integer() {
            self.compact_int_checked_binop(
                width,
                |a, b| a.wrapping_add(b),
                |a, b| a.checked_add(b),
                |a, b| a as f64 + b as f64,
            )
        } else {
            self.compact_float_binop(|a, b| a + b)
        }
    }
    fn exec_compact_sub(&mut self, width: NumericWidth) -> Result<(), VMError> {
        if width.is_integer() {
            self.compact_int_checked_binop(
                width,
                |a, b| a.wrapping_sub(b),
                |a, b| a.checked_sub(b),
                |a, b| a as f64 - b as f64,
            )
        } else {
            self.compact_float_binop(|a, b| a - b)
        }
    }
    fn exec_compact_mul(&mut self, width: NumericWidth) -> Result<(), VMError> {
        if width.is_integer() {
            self.compact_int_checked_binop(
                width,
                |a, b| a.wrapping_mul(b),
                |a, b| a.checked_mul(b),
                |a, b| a as f64 * b as f64,
            )
        } else {
            self.compact_float_binop(|a, b| a * b)
        }
    }
    fn exec_compact_div(&mut self, width: NumericWidth) -> Result<(), VMError> {
        if width.is_integer() {
            self.compact_int_divmod(width, |a, b| a.wrapping_div(b))
        } else {
            self.compact_float_divmod(|a, b| a / b)
        }
    }
    fn exec_compact_mod(&mut self, width: NumericWidth) -> Result<(), VMError> {
        if width.is_integer() {
            self.compact_int_divmod(width, |a, b| a.wrapping_rem(b))
        } else {
            self.compact_float_divmod(|a, b| a % b)
        }
    }
    fn exec_compact_cmp(&mut self, width: NumericWidth) -> Result<(), VMError> {
        if width.is_integer() {
            self.compact_int_cmp(width)
        } else {
            self.compact_float_cmp()
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
        let (b_bits, _b_kind) = self.pop_kinded()?;
        let (a_bits, _a_kind) = self.pop_kinded()?;
        let bi = b_bits as i64;
        let ai = a_bits as i64;

        if let Some(int_w) = width.to_int_width() {
            let result = wrapping_op(ai, bi);
            return self.push_kinded(int_w.truncate(result) as u64, NativeKind::Int64);
        }
        match checked(ai, bi) {
            Some(result) => self.push_kinded(result as u64, NativeKind::Int64),
            None => self.push_kinded(overflow_fallback(ai, bi).to_bits(), NativeKind::Float64),
        }
    }

    #[inline(always)]
    fn compact_int_divmod(
        &mut self,
        width: NumericWidth,
        op: impl FnOnce(i64, i64) -> i64,
    ) -> Result<(), VMError> {
        let (b_bits, _b_kind) = self.pop_kinded()?;
        let (a_bits, _a_kind) = self.pop_kinded()?;
        let bi = b_bits as i64;
        let ai = a_bits as i64;
        if bi == 0 {
            return Err(VMError::DivisionByZero);
        }
        let result = op(ai, bi);
        if let Some(int_w) = width.to_int_width() {
            self.push_kinded(int_w.truncate(result) as u64, NativeKind::Int64)
        } else {
            self.push_kinded(result as u64, NativeKind::Int64)
        }
    }

    #[inline(always)]
    fn compact_float_binop(&mut self, op: impl FnOnce(f64, f64) -> f64) -> Result<(), VMError> {
        let (b_bits, b_kind) = self.pop_kinded()?;
        let (a_bits, a_kind) = self.pop_kinded()?;
        let lhs = coerce_to_f64_kinded(a_bits, a_kind).ok_or_else(|| VMError::TypeError {
            expected: "number",
            got: kind_type_name(a_kind),
        });
        let rhs = coerce_to_f64_kinded(b_bits, b_kind).ok_or_else(|| VMError::TypeError {
            expected: "number",
            got: kind_type_name(b_kind),
        });
        drop_with_kind(a_bits, a_kind);
        drop_with_kind(b_bits, b_kind);
        self.push_kinded(op(lhs?, rhs?).to_bits(), NativeKind::Float64)
    }

    #[inline(always)]
    fn compact_float_divmod(
        &mut self,
        op: impl FnOnce(f64, f64) -> f64,
    ) -> Result<(), VMError> {
        let (b_bits, b_kind) = self.pop_kinded()?;
        let (a_bits, a_kind) = self.pop_kinded()?;
        let lhs = coerce_to_f64_kinded(a_bits, a_kind).ok_or_else(|| VMError::TypeError {
            expected: "number",
            got: kind_type_name(a_kind),
        });
        let rhs = coerce_to_f64_kinded(b_bits, b_kind).ok_or_else(|| VMError::TypeError {
            expected: "number",
            got: kind_type_name(b_kind),
        });
        drop_with_kind(a_bits, a_kind);
        drop_with_kind(b_bits, b_kind);
        let l = lhs?;
        let r = rhs?;
        if r == 0.0 {
            return Err(VMError::DivisionByZero);
        }
        self.push_kinded(op(l, r).to_bits(), NativeKind::Float64)
    }

    #[inline(always)]
    fn compact_int_cmp(&mut self, width: NumericWidth) -> Result<(), VMError> {
        // CmpTyped's i64 ordinal output (-1/0/1) — pushed as Int64.
        let (b_bits, _b_kind) = self.pop_kinded()?;
        let (a_bits, _a_kind) = self.pop_kinded()?;
        let ai = a_bits as i64;
        let bi = b_bits as i64;
        let ord = if width.is_unsigned() {
            (ai as u64).cmp(&(bi as u64)) as i64
        } else {
            ai.cmp(&bi) as i64
        };
        self.push_kinded(ord as u64, NativeKind::Int64)
    }

    #[inline(always)]
    fn compact_float_cmp(&mut self) -> Result<(), VMError> {
        let (b_bits, b_kind) = self.pop_kinded()?;
        let (a_bits, a_kind) = self.pop_kinded()?;
        let lhs = coerce_to_f64_kinded(a_bits, a_kind).ok_or_else(|| VMError::TypeError {
            expected: "number",
            got: kind_type_name(a_kind),
        });
        let rhs = coerce_to_f64_kinded(b_bits, b_kind).ok_or_else(|| VMError::TypeError {
            expected: "number",
            got: kind_type_name(b_kind),
        });
        drop_with_kind(a_bits, a_kind);
        drop_with_kind(b_bits, b_kind);
        let ord = lhs?.partial_cmp(&rhs?).map_or(0i64, |o| o as i64);
        self.push_kinded(ord as u64, NativeKind::Int64)
    }

    /// Execute `CastWidth`: pop value, truncate to declared width, push result.
    /// Wave 6.5: native i64 transport in/out via the kinded API.
    #[inline(always)]
    pub(in crate::executor) fn op_cast_width(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let width = match instruction.operand {
            Some(Operand::Width(w)) => w,
            _ => return Err(VMError::InvalidOperand),
        };
        let (bits, _kind) = self.pop_kinded()?;
        let raw = bits as i64;
        if let Some(int_w) = width.to_int_width() {
            self.push_kinded(int_w.truncate(raw) as u64, NativeKind::Int64)
        } else {
            self.push_kinded(raw as u64, NativeKind::Int64)
        }
    }

    /// Bitwise dynamic op dispatch: routes BitAnd/BitOr/BitXor/BitShl/BitShr
    /// to the binary helper and BitNot to the unary helper.
    #[inline(always)]
    pub(in crate::executor) fn exec_dyn_bit_dispatch(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use OpCode::*;
        match instruction.opcode {
            BitXor | BitAnd | BitOr | BitShl | BitShr => {
                self.exec_dyn_bit_binary(instruction.opcode)
            }
            BitNot => self.exec_dyn_bit_unary(),
            _ => unreachable!(
                "exec_dyn_bit_dispatch called with non-bitwise opcode: {:?}",
                instruction.opcode
            ),
        }
    }

    fn exec_dyn_bit_binary(&mut self, op: OpCode) -> Result<(), VMError> {
        use OpCode::*;
        let (b_bits, _b_kind) = self.pop_kinded()?;
        let (a_bits, _a_kind) = self.pop_kinded()?;
        let b_int = b_bits as i64;
        let a_int = a_bits as i64;
        let result = match op {
            BitXor => a_int ^ b_int,
            BitAnd => a_int & b_int,
            BitOr => a_int | b_int,
            BitShl => a_int << b_int,
            BitShr => a_int >> b_int,
            _ => unreachable!(),
        };
        self.push_kinded(result as u64, NativeKind::Int64)
    }

    fn exec_dyn_bit_unary(&mut self) -> Result<(), VMError> {
        let (bits, _kind) = self.pop_kinded()?;
        let a_int = bits as i64;
        self.push_kinded((!a_int) as u64, NativeKind::Int64)
    }
}

/// `&'static str` description of a `NativeKind` for `VMError::TypeError`.
#[inline]
fn kind_type_name(kind: NativeKind) -> &'static str {
    match kind {
        NativeKind::Bool => "bool",
        NativeKind::Float64 | NativeKind::NullableFloat64 => "number",
        NativeKind::Int8 | NativeKind::NullableInt8 => "i8",
        NativeKind::Int16 | NativeKind::NullableInt16 => "i16",
        NativeKind::Int32 | NativeKind::NullableInt32 => "i32",
        NativeKind::Int64 | NativeKind::NullableInt64 => "int",
        NativeKind::IntSize | NativeKind::NullableIntSize => "isize",
        NativeKind::UInt8 | NativeKind::NullableUInt8 => "u8",
        NativeKind::UInt16 | NativeKind::NullableUInt16 => "u16",
        NativeKind::UInt32 | NativeKind::NullableUInt32 => "u32",
        NativeKind::UInt64 | NativeKind::NullableUInt64 => "u64",
        NativeKind::UIntSize | NativeKind::NullableUIntSize => "usize",
        NativeKind::String => "string",
        NativeKind::Ptr(HeapKind::String) => "string",
        NativeKind::Ptr(HeapKind::TypedArray) => "array",
        NativeKind::Ptr(HeapKind::TypedObject) => "object",
        NativeKind::Ptr(HeapKind::HashMap) => "map",
        NativeKind::Ptr(HeapKind::Decimal) => "decimal",
        NativeKind::Ptr(HeapKind::BigInt) => "bigint",
        NativeKind::Ptr(HeapKind::DataTable) => "table",
        NativeKind::Ptr(HeapKind::IoHandle) => "io_handle",
        NativeKind::Ptr(HeapKind::NativeView) => "native_view",
        NativeKind::Ptr(HeapKind::Content) => "content",
        NativeKind::Ptr(HeapKind::Instant) => "instant",
        NativeKind::Ptr(HeapKind::Temporal) => "temporal",
        NativeKind::Ptr(HeapKind::TableView) => "table_view",
        NativeKind::Ptr(HeapKind::TaskGroup) => "task_group",
        NativeKind::Ptr(HeapKind::Char) => "char",
        NativeKind::Ptr(HeapKind::Closure) => "closure",
        NativeKind::Ptr(HeapKind::Future) => "future",
        NativeKind::Ptr(HeapKind::NativeScalar) => "native_scalar",
        // Wave-γ G-heap-filter-expr (ADR-006 §2.3 / Q8 amendment).
        NativeKind::Ptr(HeapKind::FilterExpr) => "filter_expr",
        // ADR-006 §2.7.13 / Q14 (Wave 8 W8-T26).
        NativeKind::Ptr(HeapKind::Reference) => "ref",
        // Wave 8 W8-T25 (ADR-006 §2.7.12 / Q13 amendment, 2026-05-10).
        NativeKind::Ptr(HeapKind::SharedCell) => "shared_cell",
        // Wave 13 W13-hashset-rebuild (ADR-006 §2.7.15 / Q16, 2026-05-10).
        NativeKind::Ptr(HeapKind::HashSet) => "set",
        // W13-iterator-state (ADR-006 §2.7.16 / Q17, 2026-05-10).
        NativeKind::Ptr(HeapKind::Iterator) => "iterator",
        // Wave 14 W14-variant-codegen (ADR-006 §2.7.17 / Q18, 2026-05-10).
        NativeKind::Ptr(HeapKind::Result) => "result",
        NativeKind::Ptr(HeapKind::Option) => "option",
    }
}

// HeapValue is referenced by the `decimal_ref` doc commentary above (the
// dispatch path mirrors ADR-005 §1) but the local reads use direct
// `Arc`-as-raw pointer access matching `KindedSlot::from_decimal`. The
// alias keeps the `use` expression stable for downstream test additions
// that need HeapValue dispatch.
#[allow(unused_imports)]
use HeapValue as _HeapValue;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::VMConfig;
    use crate::bytecode::*;
    use crate::executor::VirtualMachine;

    fn make_vm() -> VirtualMachine {
        VirtualMachine::new(VMConfig::default())
    }

    fn push_int(vm: &mut VirtualMachine, v: i64) {
        vm.push_kinded(v as u64, NativeKind::Int64).unwrap();
    }

    fn push_f64(vm: &mut VirtualMachine, v: f64) {
        vm.push_kinded(v.to_bits(), NativeKind::Float64).unwrap();
    }

    fn pop_int(vm: &mut VirtualMachine) -> i64 {
        let (bits, _kind) = vm.pop_kinded().unwrap();
        bits as i64
    }

    fn pop_f64(vm: &mut VirtualMachine) -> f64 {
        let (bits, _kind) = vm.pop_kinded().unwrap();
        f64::from_bits(bits)
    }

    fn exec_typed_int_binop(a: i64, b: i64, opcode: OpCode) -> i64 {
        let mut vm = make_vm();
        push_int(&mut vm, a);
        push_int(&mut vm, b);
        let instr = Instruction::simple(opcode);
        vm.exec_typed_arithmetic(&instr).unwrap();
        pop_int(&mut vm)
    }

    fn exec_typed_f64_binop(a: f64, b: f64, opcode: OpCode) -> f64 {
        let mut vm = make_vm();
        push_f64(&mut vm, a);
        push_f64(&mut vm, b);
        let instr = Instruction::simple(opcode);
        vm.exec_typed_arithmetic(&instr).unwrap();
        pop_f64(&mut vm)
    }

    // ── Typed Int family ──────────────────────────────────────────────────

    #[test]
    fn typed_arithmetic_add_int() {
        assert_eq!(exec_typed_int_binop(5, 3, OpCode::AddInt), 8);
    }

    #[test]
    fn typed_arithmetic_add_int_negative() {
        assert_eq!(exec_typed_int_binop(-10, 7, OpCode::AddInt), -3);
    }

    #[test]
    fn typed_arithmetic_sub_int() {
        assert_eq!(exec_typed_int_binop(10, 4, OpCode::SubInt), 6);
    }

    #[test]
    fn typed_arithmetic_mul_int() {
        assert_eq!(exec_typed_int_binop(6, 7, OpCode::MulInt), 42);
    }

    #[test]
    fn typed_arithmetic_div_int() {
        assert_eq!(exec_typed_int_binop(20, 4, OpCode::DivInt), 5);
    }

    #[test]
    fn typed_arithmetic_div_int_truncation() {
        assert_eq!(exec_typed_int_binop(7, 2, OpCode::DivInt), 3);
    }

    #[test]
    fn typed_arithmetic_div_int_by_zero() {
        let mut vm = make_vm();
        push_int(&mut vm, 10);
        push_int(&mut vm, 0);
        let instr = Instruction::simple(OpCode::DivInt);
        let err = vm.exec_typed_arithmetic(&instr).unwrap_err();
        assert!(matches!(err, VMError::DivisionByZero));
    }

    #[test]
    fn typed_arithmetic_mod_int() {
        assert_eq!(exec_typed_int_binop(17, 5, OpCode::ModInt), 2);
    }

    #[test]
    fn typed_arithmetic_pow_int() {
        assert_eq!(exec_typed_int_binop(2, 10, OpCode::PowInt), 1024);
    }

    // ── Typed Number family ───────────────────────────────────────────────

    #[test]
    fn typed_arithmetic_add_number() {
        let result = exec_typed_f64_binop(2.5, 3.5, OpCode::AddNumber);
        assert!((result - 6.0).abs() < 1e-15);
    }

    #[test]
    fn typed_arithmetic_sub_number() {
        let result = exec_typed_f64_binop(10.0, 3.5, OpCode::SubNumber);
        assert!((result - 6.5).abs() < 1e-15);
    }

    #[test]
    fn typed_arithmetic_mul_number() {
        let result = exec_typed_f64_binop(3.0, 4.0, OpCode::MulNumber);
        assert!((result - 12.0).abs() < 1e-15);
    }

    #[test]
    fn typed_arithmetic_div_number() {
        let result = exec_typed_f64_binop(10.0, 4.0, OpCode::DivNumber);
        assert!((result - 2.5).abs() < 1e-15);
    }

    #[test]
    fn typed_arithmetic_div_number_by_zero() {
        let mut vm = make_vm();
        push_f64(&mut vm, 10.0);
        push_f64(&mut vm, 0.0);
        let instr = Instruction::simple(OpCode::DivNumber);
        let err = vm.exec_typed_arithmetic(&instr).unwrap_err();
        assert!(matches!(err, VMError::DivisionByZero));
    }

    #[test]
    fn typed_arithmetic_mod_number() {
        let result = exec_typed_f64_binop(10.0, 3.0, OpCode::ModNumber);
        assert!((result - 1.0).abs() < 1e-15);
    }

    #[test]
    fn typed_arithmetic_pow_number() {
        let result = exec_typed_f64_binop(2.0, 10.0, OpCode::PowNumber);
        assert!((result - 1024.0).abs() < 1e-10);
    }

    // ── Coercion ──────────────────────────────────────────────────────────

    #[test]
    fn typed_arithmetic_int_to_number() {
        let mut vm = make_vm();
        push_int(&mut vm, 42);
        let instr = Instruction::simple(OpCode::IntToNumber);
        vm.exec_typed_arithmetic(&instr).unwrap();
        let result = pop_f64(&mut vm);
        assert!((result - 42.0).abs() < 1e-15);
    }

    #[test]
    fn typed_arithmetic_number_to_int() {
        let mut vm = make_vm();
        push_f64(&mut vm, 7.9);
        let instr = Instruction::simple(OpCode::NumberToInt);
        vm.exec_typed_arithmetic(&instr).unwrap();
        let result = pop_int(&mut vm);
        assert_eq!(result, 7);
    }

    // ── Bitwise int ───────────────────────────────────────────────────────

    #[test]
    fn typed_arithmetic_bit_and_int() {
        assert_eq!(exec_typed_int_binop(0xF0, 0x0F, OpCode::BitAndInt), 0x00);
        assert_eq!(exec_typed_int_binop(0xFF, 0x0F, OpCode::BitAndInt), 0x0F);
    }

    #[test]
    fn typed_arithmetic_bit_or_int() {
        assert_eq!(exec_typed_int_binop(0xF0, 0x0F, OpCode::BitOrInt), 0xFF);
    }

    #[test]
    fn typed_arithmetic_bit_xor_int() {
        assert_eq!(exec_typed_int_binop(0xF0, 0x0F, OpCode::BitXorInt), 0xFF);
        assert_eq!(exec_typed_int_binop(0xFF, 0xFF, OpCode::BitXorInt), 0x00);
    }

    #[test]
    fn typed_arithmetic_bit_shl_int() {
        assert_eq!(exec_typed_int_binop(3, 2, OpCode::BitShlInt), 12);
    }

    #[test]
    fn typed_arithmetic_bit_shr_int() {
        assert_eq!(exec_typed_int_binop(12, 2, OpCode::BitShrInt), 3);
    }

    #[test]
    fn typed_arithmetic_bit_not_int() {
        let mut vm = make_vm();
        push_int(&mut vm, 0);
        let instr = Instruction::simple(OpCode::BitNotInt);
        vm.exec_typed_arithmetic(&instr).unwrap();
        assert_eq!(pop_int(&mut vm), -1);
    }

    // ── CastWidth ─────────────────────────────────────────────────────────

    fn run_cast_width(value: i64, width: NumericWidth) -> i64 {
        let mut vm = make_vm();
        push_int(&mut vm, value);
        let instr = Instruction::new(OpCode::CastWidth, Some(Operand::Width(width)));
        vm.op_cast_width(&instr).unwrap();
        pop_int(&mut vm)
    }

    #[test]
    fn cast_width_i8_truncation() {
        // 300 → i8: 300 & 0xFF = 44, sign-extend → 44
        assert_eq!(run_cast_width(300, NumericWidth::I8), 44);
    }

    #[test]
    fn cast_width_i8_negative() {
        // -1 → u8: 255
        assert_eq!(run_cast_width(-1, NumericWidth::U8), 255);
    }

    #[test]
    fn cast_width_u64_max_to_i8() {
        // u64::MAX (all-ones) cast to i8 → -1
        assert_eq!(run_cast_width(u64::MAX as i64, NumericWidth::I8), -1);
    }

    // ── Compact typed family ─────────────────────────────────────────────

    fn run_typed_op_int(
        opcode: OpCode,
        width: NumericWidth,
        a: i64,
        b: i64,
    ) -> i64 {
        let mut vm = make_vm();
        push_int(&mut vm, a);
        push_int(&mut vm, b);
        let instr = Instruction::new(opcode, Some(Operand::Width(width)));
        vm.exec_compact_typed_arithmetic(&instr).unwrap();
        pop_int(&mut vm)
    }

    fn run_typed_op_f64(
        opcode: OpCode,
        width: NumericWidth,
        a: f64,
        b: f64,
    ) -> f64 {
        let mut vm = make_vm();
        push_f64(&mut vm, a);
        push_f64(&mut vm, b);
        let instr = Instruction::new(opcode, Some(Operand::Width(width)));
        vm.exec_compact_typed_arithmetic(&instr).unwrap();
        pop_f64(&mut vm)
    }

    #[test]
    fn add_typed_i64() {
        assert_eq!(run_typed_op_int(OpCode::AddTyped, NumericWidth::I64, 10, 20), 30);
    }

    #[test]
    fn add_typed_f64() {
        let result = run_typed_op_f64(OpCode::AddTyped, NumericWidth::F64, 1.5, 2.5);
        assert!((result - 4.0).abs() < 1e-15);
    }

    #[test]
    fn sub_typed_i64() {
        assert_eq!(run_typed_op_int(OpCode::SubTyped, NumericWidth::I64, 50, 20), 30);
    }

    #[test]
    fn mul_typed_i64() {
        assert_eq!(run_typed_op_int(OpCode::MulTyped, NumericWidth::I64, 6, 7), 42);
    }

    #[test]
    fn div_typed_i64() {
        assert_eq!(run_typed_op_int(OpCode::DivTyped, NumericWidth::I64, 100, 4), 25);
    }

    #[test]
    fn div_typed_i64_zero_errors() {
        let mut vm = make_vm();
        push_int(&mut vm, 10);
        push_int(&mut vm, 0);
        let instr = Instruction::new(OpCode::DivTyped, Some(Operand::Width(NumericWidth::I64)));
        let err = vm.exec_compact_typed_arithmetic(&instr).unwrap_err();
        assert!(matches!(err, VMError::DivisionByZero));
    }

    #[test]
    fn mod_typed_i64() {
        assert_eq!(run_typed_op_int(OpCode::ModTyped, NumericWidth::I64, 17, 5), 2);
    }

    #[test]
    fn cmp_typed_i64_less() {
        assert_eq!(run_typed_op_int(OpCode::CmpTyped, NumericWidth::I64, 3, 10), -1);
    }

    #[test]
    fn cmp_typed_i64_equal() {
        assert_eq!(run_typed_op_int(OpCode::CmpTyped, NumericWidth::I64, 7, 7), 0);
    }

    #[test]
    fn cmp_typed_i64_greater() {
        assert_eq!(run_typed_op_int(OpCode::CmpTyped, NumericWidth::I64, 10, 3), 1);
    }

    #[test]
    fn add_typed_missing_width_is_error() {
        let mut vm = make_vm();
        push_int(&mut vm, 1);
        push_int(&mut vm, 2);
        let instr = Instruction::simple(OpCode::AddTyped);
        let err = vm.exec_compact_typed_arithmetic(&instr).unwrap_err();
        assert!(matches!(err, VMError::InvalidOperand));
    }

    // ── Width-aware wrapping (sub-i64) ─────────────────────────────────────

    #[test]
    fn i8_add_wraps() {
        // 127 + 1 = -128 (wrapping)
        assert_eq!(run_typed_op_int(OpCode::AddTyped, NumericWidth::I8, 127, 1), -128);
    }

    #[test]
    fn u8_add_wraps() {
        // 255 + 1 = 0 (wrapping)
        assert_eq!(run_typed_op_int(OpCode::AddTyped, NumericWidth::U8, 255, 1), 0);
    }

    #[test]
    fn i16_add_wraps() {
        assert_eq!(run_typed_op_int(OpCode::AddTyped, NumericWidth::I16, 32767, 1), -32768);
    }

    #[test]
    fn i32_add_wraps() {
        assert_eq!(
            run_typed_op_int(OpCode::AddTyped, NumericWidth::I32, 2147483647, 1),
            -2147483648
        );
    }

    // ── Decimal family ─────────────────────────────────────────────────────

    fn push_decimal_test(vm: &mut VirtualMachine, d: rust_decimal::Decimal) {
        let arc = std::sync::Arc::new(d);
        let bits = std::sync::Arc::into_raw(arc) as u64;
        vm.push_kinded(bits, NativeKind::Ptr(HeapKind::Decimal)).unwrap();
    }

    fn pop_decimal_test(vm: &mut VirtualMachine) -> rust_decimal::Decimal {
        let (bits, kind) = vm.pop_kinded().unwrap();
        assert_eq!(kind, NativeKind::Ptr(HeapKind::Decimal));
        // SAFETY: we pushed an `Arc::into_raw(Arc<Decimal>)` above.
        let arc: std::sync::Arc<rust_decimal::Decimal> =
            unsafe { std::sync::Arc::from_raw(bits as *const rust_decimal::Decimal) };
        *arc
    }

    #[test]
    fn add_decimal() {
        use rust_decimal::Decimal;
        use std::str::FromStr;
        let mut vm = make_vm();
        push_decimal_test(&mut vm, Decimal::from_str("1.5").unwrap());
        push_decimal_test(&mut vm, Decimal::from_str("2.25").unwrap());
        let instr = Instruction::simple(OpCode::AddDecimal);
        vm.exec_typed_arithmetic(&instr).unwrap();
        assert_eq!(pop_decimal_test(&mut vm), Decimal::from_str("3.75").unwrap());
    }

    #[test]
    fn neg_decimal() {
        use rust_decimal::Decimal;
        use std::str::FromStr;
        let mut vm = make_vm();
        push_decimal_test(&mut vm, Decimal::from_str("3.14").unwrap());
        let instr = Instruction::simple(OpCode::NegDecimal);
        vm.exec_typed_arithmetic(&instr).unwrap();
        assert_eq!(pop_decimal_test(&mut vm), Decimal::from_str("-3.14").unwrap());
    }
}
