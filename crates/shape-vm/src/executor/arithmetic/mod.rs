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
    executor::VirtualMachine,
    executor::vm_impl::stack::drop_with_kind,
};
use shape_value::{
    NativeKind, VMError,
    heap_value::{HeapKind, HeapValue},
};
use std::sync::Arc;

use crate::constants::EXACT_F64_INT_LIMIT;

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
            // ===== Typed Add/Sub/Mul (exact native i64, CHECKED overflow) =====
            //
            // `int` is i64 (full 64-bit range). Per THE RULE (user
            // 2026-06-01 / numeric-conversion D3): arithmetic is EXACT across
            // the full i64 range and overflow is a structured RUNTIME error
            // (no silent f64 promotion, no silent two's-complement wrap). The
            // programmer widens explicitly via `as number` / `as bigint`. We
            // compute directly on i64 with `checked_*`; the result kind is
            // unconditionally `NativeKind::Int64`. (This supersedes the
            // 2026-05-20 wrapping ruling — a silent wrap is the same class of
            // hidden-data-loss defect as a silent narrowing cast.)
            AddInt => self.binop_int_checked(i64::checked_add, "addition")?,
            SubInt => self.binop_int_checked(i64::checked_sub, "subtraction")?,
            MulInt => self.binop_int_checked(i64::checked_mul, "multiplication")?,
            DivInt => {
                let (b_bits, _b_kind) = self.pop_kinded()?;
                let (a_bits, _a_kind) = self.pop_kinded()?;
                let bi = b_bits as i64;
                let ai = a_bits as i64;
                if bi == 0 {
                    return Err(VMError::DivisionByZero);
                }
                // `wrapping_div`: exact i64 quotient; the single overflow
                // case `i64::MIN / -1` wraps to `i64::MIN` per ruling #3
                // (plain `/` would panic in debug). No f64 round-trip.
                self.push_kinded(ai.wrapping_div(bi) as u64, NativeKind::Int64)?;
            }
            ModInt => {
                let (b_bits, _b_kind) = self.pop_kinded()?;
                let (a_bits, _a_kind) = self.pop_kinded()?;
                let bi = b_bits as i64;
                let ai = a_bits as i64;
                if bi == 0 {
                    return Err(VMError::DivisionByZero);
                }
                // `wrapping_rem`: exact i64 remainder; `i64::MIN % -1`
                // wraps to `0` per ruling #3. No f64 round-trip.
                self.push_kinded(ai.wrapping_rem(bi) as u64, NativeKind::Int64)?;
            }
            PowInt => {
                let (b_bits, _b_kind) = self.pop_kinded()?;
                let (a_bits, _a_kind) = self.pop_kinded()?;
                let exp = b_bits as i64;
                let base = a_bits as i64;
                if exp >= 0 && exp <= u32::MAX as i64 {
                    // `wrapping_pow`: exact i64 power with two's-complement
                    // wrapping on overflow per ruling #3 — no f64 promotion.
                    self.push_kinded(base.wrapping_pow(exp as u32) as u64, NativeKind::Int64)?;
                } else {
                    // Negative exponent has no i64 representation; fall back
                    // to f64 power (this branch never overflows i64).
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
                        let r = base
                            .to_f64()
                            .unwrap_or(0.0)
                            .powf(exp.to_f64().unwrap_or(0.0));
                        rust_decimal::Decimal::from_f64(r).unwrap_or_default()
                    }
                    _ => rust_decimal::Decimal::default(),
                };
                drop_with_kind(a_bits, a_kind);
                drop_with_kind(b_bits, b_kind);
                push_decimal(self, result)?;
            }
            // ===== Numeric Coercion =====
            //
            // PB3 (2026-05-29) — kind-aware coercion.
            //
            // Pre-PB3 `IntToNumber` / `NumberToInt` discarded the source kind
            // and treated the popped bits as i64 / f64 respectively. With the
            // PB3 typed-LoadLocal fix preserving the slot's actual kind from
            // the §2.7.7 parallel-kind track, these coercion opcodes are now
            // emitted at sites where the runtime kind may already match the
            // target (e.g. `LoadLocalI64` on a slot that was re-stored as
            // Float64 by a prior typed-Number-arith result whose StoreLocal
            // preserved Float64 per JF-1b). When the runtime kind already
            // matches the target, the coercion is an identity — passing the
            // bits through. When the runtime kind is the source-tier kind,
            // we coerce via `coerce_to_f64_kinded` (which handles all
            // numeric-tier kinds: Int8..Int64, UInt8..UInt64, Float64).
            //
            // This mirrors the post-JF-1b reality that compile-time picker
            // and runtime kind can diverge across typed-Store boundaries;
            // the coercion site must reconcile rather than blind-reinterpret.
            IntToNumber => {
                let (bits, kind) = self.pop_kinded()?;
                let v = coerce_to_f64_kinded(bits, kind).ok_or_else(|| VMError::TypeError {
                    expected: "number",
                    got: kind_type_name(kind),
                })?;
                drop_with_kind(bits, kind);
                self.push_kinded(v.to_bits(), NativeKind::Float64)?;
            }
            NumberToInt => {
                let (bits, kind) = self.pop_kinded()?;
                let v = match kind {
                    NativeKind::Int8
                    | NativeKind::Int16
                    | NativeKind::Int32
                    | NativeKind::Int64
                    | NativeKind::IntSize => bits as i64,
                    NativeKind::UInt8
                    | NativeKind::UInt16
                    | NativeKind::UInt32
                    | NativeKind::UInt64
                    | NativeKind::UIntSize => bits as i64,
                    NativeKind::Float64 | NativeKind::NullableFloat64 => {
                        f64::from_bits(bits) as i64
                    }
                    _ => {
                        return Err(VMError::TypeError {
                            expected: "int",
                            got: kind_type_name(kind),
                        });
                    }
                };
                drop_with_kind(bits, kind);
                self.push_kinded(v as u64, NativeKind::Int64)?;
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

    /// Int-int binary op with exact CHECKED i64 semantics (Add/Sub/Mul).
    ///
    /// `int` is i64 across its full range. Per THE RULE (user 2026-06-01 /
    /// numeric-conversion D3): overflow is a structured RUNTIME error — never
    /// a silent two's-complement wrap and never a silent f64 promotion. The
    /// programmer widens explicitly via `as number` / `as bigint`. Result
    /// kind is always `NativeKind::Int64`. `op_name` ("addition" /
    /// "subtraction" / "multiplication") names the operation in the error.
    #[inline(always)]
    fn binop_int_checked(
        &mut self,
        op: impl FnOnce(i64, i64) -> Option<i64>,
        op_name: &'static str,
    ) -> Result<(), VMError> {
        let (b_bits, _b_kind) = self.pop_kinded()?;
        let (a_bits, _a_kind) = self.pop_kinded()?;
        let bi = b_bits as i64;
        let ai = a_bits as i64;
        match op(ai, bi) {
            Some(r) => self.push_kinded(r as u64, NativeKind::Int64),
            None => Err(VMError::RuntimeError(format!(
                "integer {op_name} overflow: result of {ai} and {bi} exceeds the int (i64) range; \
                 widen explicitly with `as number` or `as bigint`"
            ))),
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
            self.compact_int_checked_binop(width, |a, b| a.wrapping_add(b))
        } else {
            self.compact_float_binop(|a, b| a + b)
        }
    }
    fn exec_compact_sub(&mut self, width: NumericWidth) -> Result<(), VMError> {
        if width.is_integer() {
            self.compact_int_checked_binop(width, |a, b| a.wrapping_sub(b))
        } else {
            self.compact_float_binop(|a, b| a - b)
        }
    }
    fn exec_compact_mul(&mut self, width: NumericWidth) -> Result<(), VMError> {
        if width.is_integer() {
            self.compact_int_checked_binop(width, |a, b| a.wrapping_mul(b))
        } else {
            self.compact_float_binop(|a, b| a * b)
        }
    }
    fn exec_compact_div(&mut self, width: NumericWidth) -> Result<(), VMError> {
        if width == NumericWidth::U64 {
            // R5c-2-β-γ checkpoint (b) u64-carrier: full-range unsigned
            // division — `u64::wrapping_div` on the raw bits decoded as u64.
            self.compact_int_divmod_u64(|a, b| a.wrapping_div(b))
        } else if width.is_integer() {
            self.compact_int_divmod(width, |a, b| a.wrapping_div(b))
        } else {
            self.compact_float_divmod(|a, b| a / b)
        }
    }
    fn exec_compact_mod(&mut self, width: NumericWidth) -> Result<(), VMError> {
        if width == NumericWidth::U64 {
            // R5c-2-β-γ checkpoint (b) u64-carrier: full-range unsigned
            // remainder — `u64::wrapping_rem` on the raw bits decoded as u64.
            self.compact_int_divmod_u64(|a, b| a.wrapping_rem(b))
        } else if width.is_integer() {
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
    ) -> Result<(), VMError> {
        let (b_bits, _b_kind) = self.pop_kinded()?;
        let (a_bits, _a_kind) = self.pop_kinded()?;
        let bi = b_bits as i64;
        let ai = a_bits as i64;

        // All compact integer opcodes wrap on overflow per the 2026-05-20
        // integer-semantics ruling #3. Sub-64-bit widths truncate to their
        // declared width; the i64 path (`to_int_width()` == None) wraps at
        // 64 bits. No f64 promotion — `int` arithmetic is exact across the
        // full i64 range (ruling #1).
        //
        // R5c-2-β-γ checkpoint (b) u64-carrier: Add/Sub/Mul are
        // signedness-agnostic (two's-complement — `wrapping_add` on `i64`
        // and `u64` produce the same bit pattern), so the same `wrapping_op`
        // is reused for `u64`. The result KIND, however, must reflect the
        // declared width: a `u64` result is stamped `NativeKind::UInt64` so
        // downstream `print()` / storage / wire renders it as a full-range
        // unsigned value (`u64::MAX` prints `18446744073709551615`, not
        // `-1`). Sub-64 unsigned values are non-negative in i64 and keep the
        // `Int64` carrier (pre-existing decision; out of u64-carrier scope).
        let result = wrapping_op(ai, bi);
        match width.to_int_width() {
            Some(int_w) => {
                self.push_kinded(int_w.truncate(result) as u64, result_kind_for_width(width))
            }
            None => self.push_kinded(result as u64, NativeKind::Int64),
        }
    }

    /// Signed integer division / remainder for the compact-typed widths
    /// other than `U64`.
    ///
    /// R5c-2-β-γ checkpoint (b) u64-carrier: division and remainder are NOT
    /// signedness-agnostic — `i64`'s `wrapping_div` interprets the operands
    /// as signed two's-complement, so `u64::MAX / 2` would compute
    /// `(-1) / 2 == 0` instead of `9223372036854775807`. The full-range
    /// `U64` width is therefore routed to `compact_int_divmod_u64` by
    /// `exec_compact_div` / `exec_compact_mod`; this function handles only
    /// the signed widths (`i8`/`i16`/`i32`/`i64`) and the sub-64 unsigned
    /// widths (`u8`/`u16`/`u32`), whose values are non-negative in `i64` so
    /// signed division is correct for them.
    #[inline(always)]
    fn compact_int_divmod(
        &mut self,
        width: NumericWidth,
        op: impl FnOnce(i64, i64) -> i64,
    ) -> Result<(), VMError> {
        debug_assert_ne!(
            width,
            NumericWidth::U64,
            "U64 width must route through compact_int_divmod_u64 (unsigned div)"
        );
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

    /// Unsigned 64-bit division / remainder for the `U64` compact width.
    ///
    /// R5c-2-β-γ checkpoint (b) u64-carrier: `u64` div/mod operate on the
    /// raw bits decoded as `u64`. `u64::wrapping_div` / `wrapping_rem` never
    /// overflow (the `i64::MIN / -1` case has no unsigned analogue), so the
    /// `wrapping_*` form is exact two's-complement-free unsigned arithmetic.
    /// Division by zero is a clean `VMError::DivisionByZero` (mirrors the
    /// signed path). The result is stamped `NativeKind::UInt64`.
    #[inline(always)]
    fn compact_int_divmod_u64(&mut self, op: impl FnOnce(u64, u64) -> u64) -> Result<(), VMError> {
        let (b_bits, _b_kind) = self.pop_kinded()?;
        let (a_bits, _a_kind) = self.pop_kinded()?;
        if b_bits == 0 {
            return Err(VMError::DivisionByZero);
        }
        self.push_kinded(op(a_bits, b_bits), NativeKind::UInt64)
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
    fn compact_float_divmod(&mut self, op: impl FnOnce(f64, f64) -> f64) -> Result<(), VMError> {
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

    // c5 Phase B (v0.3.3, 2026-05-28) — DELETED `exec_dyn_bit_dispatch`,
    // `exec_dyn_bit_binary`, `exec_dyn_bit_unary`. The three helpers
    // discarded operand kinds at `pop_kinded()?` (`(b_bits, _b_kind)`)
    // and reinterpreted the slot bits as i64 — the 3-line DISCARDS class
    // surfaced by the Phase A pop_kinded() sweep at audit doc 05a §c5
    // anchor sites. With the producer-side compile-time gate in
    // `compiler/expressions/binary_ops.rs:1403` + `unary_ops.rs:28`
    // refusing every non-int operand, the dynamic `BitAnd`/`BitOr`/
    // `BitXor`/`BitShl`/`BitShr`/`BitNot` opcodes are dead code: they
    // had no producer. Deletion-not-deprecation per CLAUDE.md
    // §Forbidden-Code "`exec_*_dynamic_fallback` handlers. Deleted."
    //
    // The typed `BitAndInt`/`BitOrInt`/`BitXorInt`/`BitShlInt`/`BitShrInt`/
    // `BitNotInt` arms (kept) remain the only bitwise emit path; they
    // statically pin operand kind via opcode suffix per ADR-006 §2.7.5
    // (see `exec_typed_arithmetic` arms at L121-228 in this file).
}

/// Result `NativeKind` for a compact-typed integer Add/Sub/Mul opcode.
///
/// R5c-2-β-γ checkpoint (b) u64-carrier: a `u64`-width result must be
/// stamped `NativeKind::UInt64` so it is rendered / stored / wired as a
/// full-range unsigned value (`u64::MAX` → `18446744073709551615`, not the
/// signed-reinterpret `-1`). All other integer widths keep the `Int64`
/// carrier: sub-64 unsigned values are non-negative in `i64`, and the
/// signed widths are `i64` already. This is the single place the U64
/// carrier diverges from the pre-checkpoint-(b) uniform `Int64` stamp.
#[inline]
fn result_kind_for_width(width: NumericWidth) -> NativeKind {
    if width == NumericWidth::U64 {
        NativeKind::UInt64
    } else {
        NativeKind::Int64
    }
}

/// `&'static str` description of a `NativeKind` for `VMError::TypeError`.
#[inline]
fn kind_type_name(kind: NativeKind) -> &'static str {
    match kind {
        // R5b-2-bool-null-sentinel-cluster (ADR-006 §2.7 + §2.7.7/Q9,
        // 2026-05-19): canonical absence-of-value discriminator.
        NativeKind::Null => "null",
        NativeKind::Bool => "bool",
        NativeKind::Float64 | NativeKind::NullableFloat64 => "number",
        // Round 19 S1.5 W12-nativekind-scalar-additions (2026-05-14):
        // ADR-006 §2.7.5 amendment adds F32 + Char as scalar variants.
        NativeKind::Float32 => "f32",
        NativeKind::Char => "char",
        // Wave 2 Agent B W12-StringV2-DecimalV2-NativeKind-additions
        // (2026-05-14): ADR-006 §2.7.5 amendment adds StringV2 +
        // DecimalV2 as v2-raw heap-pointer variants. Type-name surfaces
        // are the same as their Arc-wrapped siblings (`string` /
        // `decimal`) — the carrier-shape distinction is at the
        // refcount-dispatch layer, not the surface error message.
        NativeKind::StringV2 => "string",
        NativeKind::DecimalV2 => "decimal",
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
        // Wave 15 W15-deque (ADR-006 §2.7.19 / Q20, 2026-05-10).
        NativeKind::Ptr(HeapKind::Deque) => "deque",
        // Wave 15 W15-channel-rebuild (ADR-006 §2.7.20 / Q21, 2026-05-10).
        NativeKind::Ptr(HeapKind::Channel) => "channel",
        // Wave 15 W15-priority-queue (ADR-006 §2.7.18 / Q19, 2026-05-10).
        NativeKind::Ptr(HeapKind::PriorityQueue) => "priority_queue",
        // W15-range (ADR-006 §2.7.23 / Q24, 2026-05-10).
        NativeKind::Ptr(HeapKind::Range) => "range",
        // Wave 14 W14-variant-codegen (ADR-006 §2.7.17 / Q18, 2026-05-10).
        NativeKind::Ptr(HeapKind::Result) => "result",
        NativeKind::Ptr(HeapKind::Option) => "option",
        // W17-concurrency (ADR-006 §2.7.25, 2026-05-11).
        NativeKind::Ptr(HeapKind::Mutex) => "mutex",
        NativeKind::Ptr(HeapKind::Atomic) => "atomic",
        NativeKind::Ptr(HeapKind::Lazy) => "lazy",
        // W17-trait-object-storage (ADR-006 §2.7.24 / Q25.C, 2026-05-11).
        NativeKind::Ptr(HeapKind::TraitObject) => "trait_object",
        // W17-comptime-vm-dispatch (ADR-006 §2.7.26, 2026-05-12).
        NativeKind::Ptr(HeapKind::ModuleFn) => "module_fn",
        // ADR-006 §2.7.22 amendment (Round 18 S3, 2026-05-13).
        NativeKind::Ptr(HeapKind::Matrix) => "matrix",
        NativeKind::Ptr(HeapKind::MatrixSlice) => "matrix_slice",
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

    // ── i64 integer-semantics: exact arithmetic + CHECKED overflow ────────
    //
    // Per THE RULE (user 2026-06-01 / numeric-conversion D3): `int` is i64;
    // arithmetic is EXACT across the full i64 range; overflow is a structured
    // RUNTIME error — never a silent two's-complement wrap and never a silent
    // f64 promotion. (This supersedes the 2026-05-20 wrapping ruling, which
    // these tests previously pinned: a silent wrap is the same hidden-data-
    // loss defect class as a silent narrowing cast.) The exact-arithmetic
    // tests below 2^63 still pin EXACT i64 results (matching the Cranelift
    // JIT `iadd`/`isub`/`imul`); the boundary tests now assert the overflow
    // RUNTIME error.

    /// Returns the raw result of an `int`-typed binop together with its kind,
    /// so tests can assert the result stays `NativeKind::Int64` (no f64
    /// promotion). Panics if the op errors (use `exec_typed_int_binop_err`
    /// for the overflow cases).
    fn exec_typed_int_binop_kinded(a: i64, b: i64, opcode: OpCode) -> (i64, NativeKind) {
        let mut vm = make_vm();
        push_int(&mut vm, a);
        push_int(&mut vm, b);
        let instr = Instruction::simple(opcode);
        vm.exec_typed_arithmetic(&instr).unwrap();
        let (bits, kind) = vm.pop_kinded().unwrap();
        (bits as i64, kind)
    }

    /// Runs an `int`-typed binop expecting an overflow RUNTIME error (D3).
    fn exec_typed_int_binop_err(a: i64, b: i64, opcode: OpCode) -> VMError {
        let mut vm = make_vm();
        push_int(&mut vm, a);
        push_int(&mut vm, b);
        let instr = Instruction::simple(opcode);
        vm.exec_typed_arithmetic(&instr).unwrap_err()
    }

    #[test]
    fn int_add_exact_crossing_2_pow_53() {
        // 2^53 + 1 — the smallest add that the old f64 route rounded away.
        // Reproducer /tmp/intchk_b.shape.
        let (v, k) = exec_typed_int_binop_kinded(9007199254740992, 1, OpCode::AddInt);
        assert_eq!(v, 9007199254740993);
        assert_eq!(k, NativeKind::Int64);
    }

    #[test]
    fn int_add_exact_far_above_2_pow_53() {
        // Both the operand and the sum lie above 2^53, where f64 cannot
        // represent every integer exactly; the old route lost precision.
        let (v, k) =
            exec_typed_int_binop_kinded(9_007_199_254_740_993, 1_000_000_001, OpCode::AddInt);
        assert_eq!(v, 9_007_200_254_740_994);
        assert_eq!(k, NativeKind::Int64);
    }

    #[test]
    fn int_add_overflow_is_runtime_error() {
        // 2^62 + 2^62 == 2^63 overflows i64 — D3 runtime error, no wrap.
        let half = 4_611_686_018_427_387_904; // 2^62
        let err = exec_typed_int_binop_err(half, half, OpCode::AddInt);
        assert!(matches!(err, VMError::RuntimeError(ref m) if m.contains("overflow")));
    }

    #[test]
    fn int_add_at_i64_max_boundary_is_runtime_error() {
        let err = exec_typed_int_binop_err(i64::MAX, 1, OpCode::AddInt);
        assert!(matches!(err, VMError::RuntimeError(ref m) if m.contains("overflow")));
    }

    #[test]
    fn int_sub_at_i64_min_boundary_is_runtime_error() {
        let err = exec_typed_int_binop_err(i64::MIN, 1, OpCode::SubInt);
        assert!(matches!(err, VMError::RuntimeError(ref m) if m.contains("overflow")));
    }

    #[test]
    fn int_mul_exact_crossing_2_pow_53() {
        // Product is 9_007_199_705_687_823 — above 2^53 (9_007_199_254_740_992)
        // and odd, so it is NOT a representable f64. The old route rounded it.
        let (v, k) = exec_typed_int_binop_kinded(94_906_267, 94_906_269, OpCode::MulInt);
        assert_eq!(v, 9_007_199_705_687_823);
        assert!(v > 9_007_199_254_740_992); // genuinely above 2^53
        assert_eq!(k, NativeKind::Int64);
    }

    #[test]
    fn int_mul_overflow_is_runtime_error() {
        // 3037000500^2 overflows i64 — D3 runtime error, no wrap.
        let err = exec_typed_int_binop_err(3_037_000_500, 3_037_000_500, OpCode::MulInt);
        assert!(matches!(err, VMError::RuntimeError(ref m) if m.contains("overflow")));
    }

    #[test]
    fn int_mul_i64_min_by_neg_one_is_runtime_error() {
        // i64::MIN * -1 has no i64 representation — D3 runtime error.
        let err = exec_typed_int_binop_err(i64::MIN, -1, OpCode::MulInt);
        assert!(matches!(err, VMError::RuntimeError(ref m) if m.contains("overflow")));
    }

    #[test]
    fn int_div_exact_above_2_pow_53() {
        // Large exact quotient — old f64 route would lose precision.
        let (v, k) = exec_typed_int_binop_kinded(9_007_199_254_740_993_000, 1000, OpCode::DivInt);
        assert_eq!(v, 9_007_199_254_740_993);
        assert_eq!(k, NativeKind::Int64);
    }

    #[test]
    fn int_div_i64_min_by_neg_one_wraps() {
        // The single i64 division-overflow case. Per ruling #3 it wraps to
        // i64::MIN (plain `/` panics in debug; the VM uses `wrapping_div`).
        let (v, k) = exec_typed_int_binop_kinded(i64::MIN, -1, OpCode::DivInt);
        assert_eq!(v, i64::MIN);
        assert_eq!(k, NativeKind::Int64);
    }

    #[test]
    fn int_div_by_zero_is_clean_error() {
        let mut vm = make_vm();
        push_int(&mut vm, i64::MAX);
        push_int(&mut vm, 0);
        let instr = Instruction::simple(OpCode::DivInt);
        let err = vm.exec_typed_arithmetic(&instr).unwrap_err();
        assert!(matches!(err, VMError::DivisionByZero));
    }

    #[test]
    fn int_mod_i64_min_by_neg_one_wraps() {
        // i64::MIN % -1 wraps to 0 per ruling #3 (`wrapping_rem`).
        let (v, k) = exec_typed_int_binop_kinded(i64::MIN, -1, OpCode::ModInt);
        assert_eq!(v, 0);
        assert_eq!(k, NativeKind::Int64);
    }

    #[test]
    fn int_mod_exact_above_2_pow_53() {
        let (v, k) = exec_typed_int_binop_kinded(9_007_199_254_740_993_007, 1000, OpCode::ModInt);
        assert_eq!(v, 7);
        assert_eq!(k, NativeKind::Int64);
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

    fn run_typed_op_int(opcode: OpCode, width: NumericWidth, a: i64, b: i64) -> i64 {
        let mut vm = make_vm();
        push_int(&mut vm, a);
        push_int(&mut vm, b);
        let instr = Instruction::new(opcode, Some(Operand::Width(width)));
        vm.exec_compact_typed_arithmetic(&instr).unwrap();
        pop_int(&mut vm)
    }

    fn run_typed_op_f64(opcode: OpCode, width: NumericWidth, a: f64, b: f64) -> f64 {
        let mut vm = make_vm();
        push_f64(&mut vm, a);
        push_f64(&mut vm, b);
        let instr = Instruction::new(opcode, Some(Operand::Width(width)));
        vm.exec_compact_typed_arithmetic(&instr).unwrap();
        pop_f64(&mut vm)
    }

    #[test]
    fn add_typed_i64() {
        assert_eq!(
            run_typed_op_int(OpCode::AddTyped, NumericWidth::I64, 10, 20),
            30
        );
    }

    #[test]
    fn add_typed_f64() {
        let result = run_typed_op_f64(OpCode::AddTyped, NumericWidth::F64, 1.5, 2.5);
        assert!((result - 4.0).abs() < 1e-15);
    }

    #[test]
    fn sub_typed_i64() {
        assert_eq!(
            run_typed_op_int(OpCode::SubTyped, NumericWidth::I64, 50, 20),
            30
        );
    }

    #[test]
    fn mul_typed_i64() {
        assert_eq!(
            run_typed_op_int(OpCode::MulTyped, NumericWidth::I64, 6, 7),
            42
        );
    }

    #[test]
    fn div_typed_i64() {
        assert_eq!(
            run_typed_op_int(OpCode::DivTyped, NumericWidth::I64, 100, 4),
            25
        );
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
        assert_eq!(
            run_typed_op_int(OpCode::ModTyped, NumericWidth::I64, 17, 5),
            2
        );
    }

    #[test]
    fn cmp_typed_i64_less() {
        assert_eq!(
            run_typed_op_int(OpCode::CmpTyped, NumericWidth::I64, 3, 10),
            -1
        );
    }

    #[test]
    fn cmp_typed_i64_equal() {
        assert_eq!(
            run_typed_op_int(OpCode::CmpTyped, NumericWidth::I64, 7, 7),
            0
        );
    }

    #[test]
    fn cmp_typed_i64_greater() {
        assert_eq!(
            run_typed_op_int(OpCode::CmpTyped, NumericWidth::I64, 10, 3),
            1
        );
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
        assert_eq!(
            run_typed_op_int(OpCode::AddTyped, NumericWidth::I8, 127, 1),
            -128
        );
    }

    #[test]
    fn u8_add_wraps() {
        // 255 + 1 = 0 (wrapping)
        assert_eq!(
            run_typed_op_int(OpCode::AddTyped, NumericWidth::U8, 255, 1),
            0
        );
    }

    #[test]
    fn i16_add_wraps() {
        assert_eq!(
            run_typed_op_int(OpCode::AddTyped, NumericWidth::I16, 32767, 1),
            -32768
        );
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
        vm.push_kinded(bits, NativeKind::Ptr(HeapKind::Decimal))
            .unwrap();
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
        assert_eq!(
            pop_decimal_test(&mut vm),
            Decimal::from_str("3.75").unwrap()
        );
    }

    #[test]
    fn neg_decimal() {
        use rust_decimal::Decimal;
        use std::str::FromStr;
        let mut vm = make_vm();
        push_decimal_test(&mut vm, Decimal::from_str("3.14").unwrap());
        let instr = Instruction::simple(OpCode::NegDecimal);
        vm.exec_typed_arithmetic(&instr).unwrap();
        assert_eq!(
            pop_decimal_test(&mut vm),
            Decimal::from_str("-3.14").unwrap()
        );
    }

    // ── u64 full-range carrier — R5c-2-β-γ checkpoint (b) ─────────────────
    //
    // `u64` is a REAL full-range `0..2^64` carrier (integer-semantics
    // ruling 2026-05-20 #2). The compact-typed opcode family carries the
    // `NumericWidth::U64` operand; Add/Sub/Mul wrap two's-complement at
    // 2^64 (#3) and reuse the signedness-agnostic `wrapping_*` ops, but
    // the result KIND must be `UInt64` so the value renders / stores /
    // wires as a full-range unsigned number. Div/Mod are signedness-
    // DEPENDENT — they decode the raw bits as `u64` and use unsigned
    // `u64::wrapping_div`/`_rem` (a signed reinterpret would compute
    // `u64::MAX / 2 == (-1)/2 == 0`).

    /// Push raw `u64` bits with the `UInt64` carrier kind.
    fn push_u64(vm: &mut VirtualMachine, v: u64) {
        vm.push_kinded(v, NativeKind::UInt64).unwrap();
    }

    /// Run a compact-typed U64 opcode, returning `(result_bits_as_u64, kind)`.
    fn run_u64_op(opcode: OpCode, a: u64, b: u64) -> (u64, NativeKind) {
        let mut vm = make_vm();
        push_u64(&mut vm, a);
        push_u64(&mut vm, b);
        let instr = Instruction::new(opcode, Some(Operand::Width(NumericWidth::U64)));
        vm.exec_compact_typed_arithmetic(&instr).unwrap();
        vm.pop_kinded().unwrap()
    }

    #[test]
    fn u64_add_exact() {
        let (v, k) = run_u64_op(OpCode::AddTyped, 10_000_000_000_000_000_000, 5);
        assert_eq!(v, 10_000_000_000_000_000_005);
        assert_eq!(k, NativeKind::UInt64);
    }

    #[test]
    fn u64_add_wraps_at_2_pow_64() {
        // u64::MAX + 1 wraps to 0.
        let (v, k) = run_u64_op(OpCode::AddTyped, u64::MAX, 1);
        assert_eq!(v, 0);
        assert_eq!(k, NativeKind::UInt64);
    }

    #[test]
    fn u64_mul_wraps_at_2_pow_64() {
        // 1e19 * 1e19 wraps mod 2^64.
        let (v, k) = run_u64_op(
            OpCode::MulTyped,
            10_000_000_000_000_000_000,
            10_000_000_000_000_000_000,
        );
        assert_eq!(
            v,
            10_000_000_000_000_000_000u64.wrapping_mul(10_000_000_000_000_000_000)
        );
        assert_eq!(k, NativeKind::UInt64);
    }

    #[test]
    fn u64_sub_wraps_below_zero() {
        // 0 - 1 wraps to u64::MAX.
        let (v, k) = run_u64_op(OpCode::SubTyped, 0, 1);
        assert_eq!(v, u64::MAX);
        assert_eq!(k, NativeKind::UInt64);
    }

    #[test]
    fn u64_div_is_unsigned() {
        // u64::MAX / 2 — unsigned: 9223372036854775807. A signed
        // reinterpret would compute (-1) / 2 == 0.
        let (v, k) = run_u64_op(OpCode::DivTyped, u64::MAX, 2);
        assert_eq!(v, 9_223_372_036_854_775_807);
        assert_eq!(k, NativeKind::UInt64);
    }

    #[test]
    fn u64_div_full_range_dividend() {
        // (2^64 - 2) / (2^63) == 1 — both operands above i64::MAX.
        let (v, _) = run_u64_op(OpCode::DivTyped, u64::MAX - 1, 1u64 << 63);
        assert_eq!(v, 1);
    }

    #[test]
    fn u64_mod_is_unsigned() {
        // u64::MAX % 10 == 5 (unsigned). Signed would give -1.
        let (v, k) = run_u64_op(OpCode::ModTyped, u64::MAX, 10);
        assert_eq!(v, 5);
        assert_eq!(k, NativeKind::UInt64);
    }

    #[test]
    fn u64_div_by_zero_is_clean_error() {
        let mut vm = make_vm();
        push_u64(&mut vm, u64::MAX);
        push_u64(&mut vm, 0);
        let instr = Instruction::new(OpCode::DivTyped, Some(Operand::Width(NumericWidth::U64)));
        let err = vm.exec_compact_typed_arithmetic(&instr).unwrap_err();
        assert!(matches!(err, VMError::DivisionByZero));
    }

    #[test]
    fn u64_mod_by_zero_is_clean_error() {
        let mut vm = make_vm();
        push_u64(&mut vm, u64::MAX);
        push_u64(&mut vm, 0);
        let instr = Instruction::new(OpCode::ModTyped, Some(Operand::Width(NumericWidth::U64)));
        let err = vm.exec_compact_typed_arithmetic(&instr).unwrap_err();
        assert!(matches!(err, VMError::DivisionByZero));
    }
}
