//! Comparison operations for the VM executor
//!
//! Handles: Gt, Lt, Gte, Lte, Eq, Neq

use crate::{
    bytecode::{Instruction, OpCode},
    executor::VirtualMachine,
};
use shape_value::{FilterLiteral, FilterNode, FilterOp, NanTag, VMError, ValueWord};
use std::cmp::Ordering;
use std::sync::Arc;

const EXACT_F64_INT_LIMIT: i128 = 9_007_199_254_740_992;

impl VirtualMachine {
    #[inline(always)]
    fn i128_to_lossless_f64(v: i128) -> Option<f64> {
        if (-EXACT_F64_INT_LIMIT..=EXACT_F64_INT_LIMIT).contains(&v) {
            Some(v as f64)
        } else {
            None
        }
    }

    /// Compare two ValueWord numeric values without lossy integer->float coercion.
    #[inline(always)]
    fn nb_compare_numeric(a: &ValueWord, b: &ValueWord) -> Option<Ordering> {
        if let (Some(ai), Some(bi)) = (a.as_i128_exact(), b.as_i128_exact()) {
            return Some(ai.cmp(&bi));
        }

        match (a.as_decimal(), b.as_decimal()) {
            (Some(ad), Some(bd)) => return Some(ad.cmp(&bd)),
            (Some(ad), None) => {
                if let Some(bi) = b.as_i128_exact() {
                    let b_dec = rust_decimal::Decimal::from_i128_with_scale(bi, 0);
                    return Some(ad.cmp(&b_dec));
                }
                if let Some(bf) = b.as_number_strict() {
                    let b_dec = rust_decimal::Decimal::from_f64_retain(bf)?;
                    return Some(ad.cmp(&b_dec));
                }
            }
            (None, Some(bd)) => {
                if let Some(ai) = a.as_i128_exact() {
                    let a_dec = rust_decimal::Decimal::from_i128_with_scale(ai, 0);
                    return Some(a_dec.cmp(&bd));
                }
                if let Some(af) = a.as_number_strict() {
                    let a_dec = rust_decimal::Decimal::from_f64_retain(af)?;
                    return Some(a_dec.cmp(&bd));
                }
            }
            _ => {}
        }

        if let (Some(af), Some(bf)) = (a.as_number_strict(), b.as_number_strict()) {
            return af.partial_cmp(&bf);
        }

        if let (Some(ai), Some(bf)) = (a.as_i128_exact(), b.as_number_strict()) {
            let af = Self::i128_to_lossless_f64(ai)?;
            return af.partial_cmp(&bf);
        }
        if let (Some(af), Some(bi)) = (a.as_number_strict(), b.as_i128_exact()) {
            let bf = Self::i128_to_lossless_f64(bi)?;
            return af.partial_cmp(&bf);
        }

        None
    }

    /// Try to build a FilterExpr from an ExprProxy comparison (ValueWord-native).
    /// Returns Some(ValueWord) if one operand is ExprProxy and the other is a literal.
    fn try_nb_expr_proxy_compare(a: &ValueWord, b: &ValueWord, op: FilterOp) -> Option<ValueWord> {
        // ExprProxy op Literal
        if let Some(col) = a.as_expr_proxy() {
            if let Some(lit) = Self::nb_to_filter_literal(b) {
                return Some(ValueWord::from_filter_expr(Arc::new(FilterNode::Compare {
                    column: col.as_ref().clone(),
                    op,
                    value: lit,
                })));
            }
        }
        // Literal op ExprProxy -> flip
        if let Some(col) = b.as_expr_proxy() {
            if let Some(lit) = Self::nb_to_filter_literal(a) {
                let flipped_op = match op {
                    FilterOp::Gt => FilterOp::Lt,
                    FilterOp::Lt => FilterOp::Gt,
                    FilterOp::Gte => FilterOp::Lte,
                    FilterOp::Lte => FilterOp::Gte,
                    FilterOp::Eq => FilterOp::Eq,
                    FilterOp::Neq => FilterOp::Neq,
                };
                return Some(ValueWord::from_filter_expr(Arc::new(FilterNode::Compare {
                    column: col.as_ref().clone(),
                    op: flipped_op,
                    value: lit,
                })));
            }
        }
        None
    }

    /// Convert a ValueWord to a FilterLiteral (for SQL pushdown)
    fn nb_to_filter_literal(value: &ValueWord) -> Option<FilterLiteral> {
        match value.tag() {
            NanTag::I48 => Some(FilterLiteral::Int(value.as_i64().unwrap_or(0))),
            NanTag::F64 => value.as_f64().map(FilterLiteral::Float),
            NanTag::Bool => Some(FilterLiteral::Bool(value.as_bool().unwrap_or(false))),
            NanTag::None => Some(FilterLiteral::Null),
            NanTag::Heap => value.as_str().map(|s| FilterLiteral::String(s.to_string())),
            NanTag::Ref => None,
            _ => None,
        }
    }

    /// Execute typed comparison opcodes (compiler-guaranteed types, zero dispatch)
    #[inline(always)]
    pub(in crate::executor) fn exec_typed_comparison(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        if let Some(ref mut metrics) = self.metrics {
            if instruction.opcode.is_trusted() {
                metrics.record_trusted_op();
            } else {
                metrics.record_guarded_op();
            }
        }
        use OpCode::*;
        match instruction.opcode {
            // ValueWord fast path for Int/Number comparisons
            GtInt => {
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                self.push_vw(unsafe { ValueWord::gt_i64(&a, &b) })?;
            }
            GtNumber => {
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                self.push_vw(ValueWord::from_bool(unsafe {
                    a.as_f64_unchecked() > b.as_f64_unchecked()
                }))?;
            }
            GtDecimal => {
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                self.push_vw(ValueWord::from_bool(unsafe {
                    a.as_decimal_unchecked() > b.as_decimal_unchecked()
                }))?;
            }
            LtInt => {
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                self.push_vw(unsafe { ValueWord::lt_i64(&a, &b) })?;
            }
            LtNumber => {
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                self.push_vw(ValueWord::from_bool(unsafe {
                    a.as_f64_unchecked() < b.as_f64_unchecked()
                }))?;
            }
            LtDecimal => {
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                self.push_vw(ValueWord::from_bool(unsafe {
                    a.as_decimal_unchecked() < b.as_decimal_unchecked()
                }))?;
            }
            GteInt => {
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                self.push_vw(ValueWord::from_bool(unsafe {
                    a.as_i64_unchecked() >= b.as_i64_unchecked()
                }))?;
            }
            GteNumber => {
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                self.push_vw(ValueWord::from_bool(unsafe {
                    a.as_f64_unchecked() >= b.as_f64_unchecked()
                }))?;
            }
            GteDecimal => {
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                self.push_vw(ValueWord::from_bool(unsafe {
                    a.as_decimal_unchecked() >= b.as_decimal_unchecked()
                }))?;
            }
            LteInt => {
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                self.push_vw(ValueWord::from_bool(unsafe {
                    a.as_i64_unchecked() <= b.as_i64_unchecked()
                }))?;
            }
            LteNumber => {
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                self.push_vw(ValueWord::from_bool(unsafe {
                    a.as_f64_unchecked() <= b.as_f64_unchecked()
                }))?;
            }
            LteDecimal => {
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                self.push_vw(ValueWord::from_bool(unsafe {
                    a.as_decimal_unchecked() <= b.as_decimal_unchecked()
                }))?;
            }
            EqInt => {
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                self.push_vw(ValueWord::from_bool(unsafe {
                    a.as_i64_unchecked() == b.as_i64_unchecked()
                }))?;
            }
            EqNumber => {
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                self.push_vw(ValueWord::from_bool(unsafe {
                    a.as_f64_unchecked() == b.as_f64_unchecked()
                }))?;
            }
            NeqInt => {
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                self.push_vw(ValueWord::from_bool(unsafe {
                    a.as_i64_unchecked() != b.as_i64_unchecked()
                }))?;
            }
            NeqNumber => {
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                self.push_vw(ValueWord::from_bool(unsafe {
                    a.as_f64_unchecked() != b.as_f64_unchecked()
                }))?;
            }
            // Trusted comparison variants (compiler-proved types, no runtime guard)
            GtIntTrusted => {
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                debug_assert!(a.is_i64() && b.is_i64(), "Trusted GtInt invariant violated");
                self.push_vw(ValueWord::from_bool(unsafe {
                    a.as_i64_unchecked() > b.as_i64_unchecked()
                }))?;
            }
            LtIntTrusted => {
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                debug_assert!(a.is_i64() && b.is_i64(), "Trusted LtInt invariant violated");
                self.push_vw(ValueWord::from_bool(unsafe {
                    a.as_i64_unchecked() < b.as_i64_unchecked()
                }))?;
            }
            GteIntTrusted => {
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                debug_assert!(
                    a.is_i64() && b.is_i64(),
                    "Trusted GteInt invariant violated"
                );
                self.push_vw(ValueWord::from_bool(unsafe {
                    a.as_i64_unchecked() >= b.as_i64_unchecked()
                }))?;
            }
            LteIntTrusted => {
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                debug_assert!(
                    a.is_i64() && b.is_i64(),
                    "Trusted LteInt invariant violated"
                );
                self.push_vw(ValueWord::from_bool(unsafe {
                    a.as_i64_unchecked() <= b.as_i64_unchecked()
                }))?;
            }
            GtNumberTrusted => {
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                debug_assert!(
                    a.as_number_coerce().is_some() && b.as_number_coerce().is_some(),
                    "Trusted GtNumber invariant violated"
                );
                self.push_vw(ValueWord::from_bool(unsafe {
                    a.as_f64_unchecked() > b.as_f64_unchecked()
                }))?;
            }
            LtNumberTrusted => {
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                debug_assert!(
                    a.as_number_coerce().is_some() && b.as_number_coerce().is_some(),
                    "Trusted LtNumber invariant violated"
                );
                self.push_vw(ValueWord::from_bool(unsafe {
                    a.as_f64_unchecked() < b.as_f64_unchecked()
                }))?;
            }
            GteNumberTrusted => {
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                debug_assert!(
                    a.as_number_coerce().is_some() && b.as_number_coerce().is_some(),
                    "Trusted GteNumber invariant violated"
                );
                self.push_vw(ValueWord::from_bool(unsafe {
                    a.as_f64_unchecked() >= b.as_f64_unchecked()
                }))?;
            }
            LteNumberTrusted => {
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                debug_assert!(
                    a.as_number_coerce().is_some() && b.as_number_coerce().is_some(),
                    "Trusted LteNumber invariant violated"
                );
                self.push_vw(ValueWord::from_bool(unsafe {
                    a.as_f64_unchecked() <= b.as_f64_unchecked()
                }))?;
            }
            _ => unreachable!(
                "exec_typed_comparison called with non-typed-comparison opcode: {:?}",
                instruction.opcode
            ),
        }
        Ok(())
    }

    #[inline(always)]
    pub(in crate::executor) fn exec_comparison(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use OpCode::*;
        match instruction.opcode {
            Gt => {
                let b_nb = self.pop_vw()?;
                let a_nb = self.pop_vw()?;
                // ValueWord-native: try ExprProxy, then numeric comparison
                if let Some(expr) = Self::try_nb_expr_proxy_compare(&a_nb, &b_nb, FilterOp::Gt) {
                    self.push_vw(expr)?;
                } else if let Some(ord) = Self::nb_compare_numeric(&a_nb, &b_nb) {
                    self.push_vw(ValueWord::from_bool(ord == Ordering::Greater))?;
                } else {
                    // String comparison
                    if let (Some(a_str), Some(b_str)) = (a_nb.as_str(), b_nb.as_str()) {
                        self.push_vw(ValueWord::from_bool(a_str > b_str))?;
                    } else {
                        return Err(VMError::RuntimeError(format!(
                            "Cannot compare '>' on {} and {}",
                            a_nb.type_name(),
                            b_nb.type_name()
                        )));
                    }
                }
            }
            Lt => {
                let b_nb = self.pop_vw()?;
                let a_nb = self.pop_vw()?;
                if let Some(expr) = Self::try_nb_expr_proxy_compare(&a_nb, &b_nb, FilterOp::Lt) {
                    self.push_vw(expr)?;
                } else if let Some(ord) = Self::nb_compare_numeric(&a_nb, &b_nb) {
                    self.push_vw(ValueWord::from_bool(ord == Ordering::Less))?;
                } else {
                    if let (Some(a_str), Some(b_str)) = (a_nb.as_str(), b_nb.as_str()) {
                        self.push_vw(ValueWord::from_bool(a_str < b_str))?;
                    } else {
                        return Err(VMError::RuntimeError(format!(
                            "Cannot compare '<' on {} and {}",
                            a_nb.type_name(),
                            b_nb.type_name()
                        )));
                    }
                }
            }
            Gte => {
                let b_nb = self.pop_vw()?;
                let a_nb = self.pop_vw()?;
                if let Some(expr) = Self::try_nb_expr_proxy_compare(&a_nb, &b_nb, FilterOp::Gte) {
                    self.push_vw(expr)?;
                } else if let Some(ord) = Self::nb_compare_numeric(&a_nb, &b_nb) {
                    self.push_vw(ValueWord::from_bool(
                        ord == Ordering::Greater || ord == Ordering::Equal,
                    ))?;
                } else {
                    if let (Some(a_str), Some(b_str)) = (a_nb.as_str(), b_nb.as_str()) {
                        self.push_vw(ValueWord::from_bool(a_str >= b_str))?;
                    } else {
                        return Err(VMError::RuntimeError(format!(
                            "Cannot compare '>=' on {} and {}",
                            a_nb.type_name(),
                            b_nb.type_name()
                        )));
                    }
                }
            }
            Lte => {
                let b_nb = self.pop_vw()?;
                let a_nb = self.pop_vw()?;
                if let Some(expr) = Self::try_nb_expr_proxy_compare(&a_nb, &b_nb, FilterOp::Lte) {
                    self.push_vw(expr)?;
                } else if let Some(ord) = Self::nb_compare_numeric(&a_nb, &b_nb) {
                    self.push_vw(ValueWord::from_bool(
                        ord == Ordering::Less || ord == Ordering::Equal,
                    ))?;
                } else {
                    if let (Some(a_str), Some(b_str)) = (a_nb.as_str(), b_nb.as_str()) {
                        self.push_vw(ValueWord::from_bool(a_str <= b_str))?;
                    } else {
                        return Err(VMError::RuntimeError(format!(
                            "Cannot compare '<=' on {} and {}",
                            a_nb.type_name(),
                            b_nb.type_name()
                        )));
                    }
                }
            }
            Eq => {
                let b_nb = self.pop_vw()?;
                let a_nb = self.pop_vw()?;
                // Check ExprProxy first (rare SQL pushdown path)
                if let Some(expr) = Self::try_nb_expr_proxy_compare(&a_nb, &b_nb, FilterOp::Eq) {
                    self.push_vw(expr)?;
                } else {
                    // vw_equals handles all types including heap (String, Array, Decimal, etc.)
                    self.push_vw(ValueWord::from_bool(a_nb.vw_equals(&b_nb)))?;
                }
            }
            Neq => {
                let b_nb = self.pop_vw()?;
                let a_nb = self.pop_vw()?;
                if let Some(expr) = Self::try_nb_expr_proxy_compare(&a_nb, &b_nb, FilterOp::Neq) {
                    self.push_vw(expr)?;
                } else {
                    self.push_vw(ValueWord::from_bool(!a_nb.vw_equals(&b_nb)))?;
                }
            }
            _ => unreachable!(
                "exec_comparison called with non-comparison opcode: {:?}",
                instruction.opcode
            ),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering;

    #[test]
    fn compare_numeric_handles_u64_exactly() {
        let a = ValueWord::from_native_u64(u64::MAX);
        let b = ValueWord::from_native_u64(u64::MAX - 1);
        assert_eq!(
            VirtualMachine::nb_compare_numeric(&a, &b),
            Some(Ordering::Greater)
        );
    }

    #[test]
    fn compare_numeric_rejects_lossy_u64_vs_number() {
        let a = ValueWord::from_native_u64(u64::MAX);
        let b = ValueWord::from_f64(1.0);
        assert_eq!(VirtualMachine::nb_compare_numeric(&a, &b), None);
    }
}
