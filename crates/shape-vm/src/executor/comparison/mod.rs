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

use crate::constants::EXACT_F64_INT_LIMIT;

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
            // Raw typed fast paths for Int/Number comparisons. These typed
            // opcodes are emitted when the compiler has proven both
            // operands are i48-inline ints (resp. plain f64 numbers), so
            // the fast path can read raw bits without dispatching through
            // ValueWord. The slow path tolerates coercible operands
            // (matching the legacy `*_unchecked` behavior).
            // (Decimal comparisons remain on the heap path.)
            GtInt => {
                if self.stack_top_both_i48() {
                    let bi = self.pop_raw_i64()?;
                    let ai = self.pop_raw_i64()?;
                    self.push_raw_bool(ai > bi)?;
                } else {
                    let b = self.pop_vw()?;
                    let a = self.pop_vw()?;
                    self.push_raw_bool(unsafe {
                        a.as_i64_unchecked() > b.as_i64_unchecked()
                    })?;
                }
            }
            GtNumber => {
                if self.stack_top_both_f64() {
                    let b = self.pop_raw_f64()?;
                    let a = self.pop_raw_f64()?;
                    self.push_raw_bool(a > b)?;
                } else {
                    let b = self.pop_vw()?;
                    let a = self.pop_vw()?;
                    self.push_raw_bool(unsafe {
                        a.as_f64_unchecked() > b.as_f64_unchecked()
                    })?;
                }
            }
            GtDecimal => {
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                self.push_raw_bool(unsafe {
                    a.as_decimal_unchecked() > b.as_decimal_unchecked()
                })?;
            }
            LtInt => {
                if self.stack_top_both_i48() {
                    let bi = self.pop_raw_i64()?;
                    let ai = self.pop_raw_i64()?;
                    self.push_raw_bool(ai < bi)?;
                } else {
                    let b = self.pop_vw()?;
                    let a = self.pop_vw()?;
                    self.push_raw_bool(unsafe {
                        a.as_i64_unchecked() < b.as_i64_unchecked()
                    })?;
                }
            }
            LtNumber => {
                if self.stack_top_both_f64() {
                    let b = self.pop_raw_f64()?;
                    let a = self.pop_raw_f64()?;
                    self.push_raw_bool(a < b)?;
                } else {
                    let b = self.pop_vw()?;
                    let a = self.pop_vw()?;
                    self.push_raw_bool(unsafe {
                        a.as_f64_unchecked() < b.as_f64_unchecked()
                    })?;
                }
            }
            LtDecimal => {
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                self.push_raw_bool(unsafe {
                    a.as_decimal_unchecked() < b.as_decimal_unchecked()
                })?;
            }
            GteInt => {
                if self.stack_top_both_i48() {
                    let bi = self.pop_raw_i64()?;
                    let ai = self.pop_raw_i64()?;
                    self.push_raw_bool(ai >= bi)?;
                } else {
                    let b = self.pop_vw()?;
                    let a = self.pop_vw()?;
                    self.push_raw_bool(unsafe {
                        a.as_i64_unchecked() >= b.as_i64_unchecked()
                    })?;
                }
            }
            GteNumber => {
                if self.stack_top_both_f64() {
                    let b = self.pop_raw_f64()?;
                    let a = self.pop_raw_f64()?;
                    self.push_raw_bool(a >= b)?;
                } else {
                    let b = self.pop_vw()?;
                    let a = self.pop_vw()?;
                    self.push_raw_bool(unsafe {
                        a.as_f64_unchecked() >= b.as_f64_unchecked()
                    })?;
                }
            }
            GteDecimal => {
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                self.push_raw_bool(unsafe {
                    a.as_decimal_unchecked() >= b.as_decimal_unchecked()
                })?;
            }
            LteInt => {
                if self.stack_top_both_i48() {
                    let bi = self.pop_raw_i64()?;
                    let ai = self.pop_raw_i64()?;
                    self.push_raw_bool(ai <= bi)?;
                } else {
                    let b = self.pop_vw()?;
                    let a = self.pop_vw()?;
                    self.push_raw_bool(unsafe {
                        a.as_i64_unchecked() <= b.as_i64_unchecked()
                    })?;
                }
            }
            LteNumber => {
                if self.stack_top_both_f64() {
                    let b = self.pop_raw_f64()?;
                    let a = self.pop_raw_f64()?;
                    self.push_raw_bool(a <= b)?;
                } else {
                    let b = self.pop_vw()?;
                    let a = self.pop_vw()?;
                    self.push_raw_bool(unsafe {
                        a.as_f64_unchecked() <= b.as_f64_unchecked()
                    })?;
                }
            }
            LteDecimal => {
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                self.push_raw_bool(unsafe {
                    a.as_decimal_unchecked() <= b.as_decimal_unchecked()
                })?;
            }
            EqInt => {
                if self.stack_top_both_i48() {
                    let bi = self.pop_raw_i64()?;
                    let ai = self.pop_raw_i64()?;
                    self.push_raw_bool(ai == bi)?;
                } else {
                    let b = self.pop_vw()?;
                    let a = self.pop_vw()?;
                    self.push_raw_bool(unsafe {
                        a.as_i64_unchecked() == b.as_i64_unchecked()
                    })?;
                }
            }
            EqNumber => {
                // NOTE: NaN != NaN per IEEE 754 — both fast and slow paths
                // preserve this semantics via direct f64 == compare.
                if self.stack_top_both_f64() {
                    let b = self.pop_raw_f64()?;
                    let a = self.pop_raw_f64()?;
                    self.push_raw_bool(a == b)?;
                } else {
                    let b = self.pop_vw()?;
                    let a = self.pop_vw()?;
                    self.push_raw_bool(unsafe {
                        a.as_f64_unchecked() == b.as_f64_unchecked()
                    })?;
                }
            }
            NeqInt => {
                if self.stack_top_both_i48() {
                    let bi = self.pop_raw_i64()?;
                    let ai = self.pop_raw_i64()?;
                    self.push_raw_bool(ai != bi)?;
                } else {
                    let b = self.pop_vw()?;
                    let a = self.pop_vw()?;
                    self.push_raw_bool(unsafe {
                        a.as_i64_unchecked() != b.as_i64_unchecked()
                    })?;
                }
            }
            NeqNumber => {
                // NaN != NaN per IEEE 754 — preserved by direct f64 compare.
                if self.stack_top_both_f64() {
                    let b = self.pop_raw_f64()?;
                    let a = self.pop_raw_f64()?;
                    self.push_raw_bool(a != b)?;
                } else {
                    let b = self.pop_vw()?;
                    let a = self.pop_vw()?;
                    self.push_raw_bool(unsafe {
                        a.as_f64_unchecked() != b.as_f64_unchecked()
                    })?;
                }
            }
            // Stage 2.6.3: typed equality for heap-backed string and
            // decimal types. Compiler emits these only when both operands
            // are statically proven to be the matching type.
            EqString => {
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                let eq = match (a.as_str(), b.as_str()) {
                    (Some(a_str), Some(b_str)) => a_str == b_str,
                    _ => false,
                };
                self.push_raw_bool(eq)?;
            }
            EqDecimal => {
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                let eq = match (a.as_decimal(), b.as_decimal()) {
                    (Some(ad), Some(bd)) => ad == bd,
                    _ => false,
                };
                self.push_raw_bool(eq)?;
            }
            // Stage 4.2: typed ordered comparison for strings (lexicographic).
            GtString => {
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                let result = a.as_str().unwrap_or("") > b.as_str().unwrap_or("");
                self.push_raw_bool(result)?;
            }
            LtString => {
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                let result = a.as_str().unwrap_or("") < b.as_str().unwrap_or("");
                self.push_raw_bool(result)?;
            }
            GteString => {
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                let result = a.as_str().unwrap_or("") >= b.as_str().unwrap_or("");
                self.push_raw_bool(result)?;
            }
            LteString => {
                let b = self.pop_vw()?;
                let a = self.pop_vw()?;
                let result = a.as_str().unwrap_or("") <= b.as_str().unwrap_or("");
                self.push_raw_bool(result)?;
            }
            // Stage 2.6.5.1: typed absence check. Pops one value, pushes a
            // bool that is true iff the value is the None or Unit sentinel.
            // Both are absence-of-value markers; the optional-chaining and
            // null-coalescing desugarings short-circuit on either, so this
            // opcode covers both cases. Replaces the legacy `PushNull; Eq`
            // and `emit_unit; Eq` patterns at the 16 sentinel sites in the
            // compiler.
            IsNull => {
                // Compute the absence flag from the popped value's tag
                // BEFORE the ValueWord goes out of scope, so the borrow
                // can't outlive the Drop. Avoids the SIGABRT regression
                // from the original Phase 2.6.5 bigbang attempt.
                let v = self.pop_vw()?;
                let is_absent = v.is_none() || v.is_unit();
                drop(v);
                self.push_raw_bool(is_absent)?;
            }
            // NOTE: Trusted comparison variants removed — consolidated into
            // the typed variants above (GtInt, LtInt, etc.).
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
            GtDynamic => {
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
            LtDynamic => {
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
            GteDynamic => {
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
            LteDynamic => {
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
            EqDynamic => {
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
            NeqDynamic => {
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
    use crate::bytecode::{Instruction, Operand};
    use crate::executor::{VMConfig, VirtualMachine};
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

    fn make_vm() -> VirtualMachine {
        VirtualMachine::new(VMConfig::default())
    }

    fn run_typed_cmp(vm: &mut VirtualMachine, opcode: OpCode) -> bool {
        let instr = Instruction { opcode, operand: None };
        vm.exec_typed_comparison(&instr).unwrap();
        unsafe { vm.pop_vw().unwrap().as_bool_unchecked() }
    }

    // ----- Raw Int comparison fast paths -----

    #[test]
    fn typed_int_eq_uses_raw_fast_path() {
        let mut vm = make_vm();
        vm.push_raw_i64(42).unwrap();
        vm.push_raw_i64(42).unwrap();
        assert!(run_typed_cmp(&mut vm, OpCode::EqInt));
    }

    #[test]
    fn typed_int_neq_uses_raw_fast_path() {
        let mut vm = make_vm();
        vm.push_raw_i64(1).unwrap();
        vm.push_raw_i64(2).unwrap();
        assert!(run_typed_cmp(&mut vm, OpCode::NeqInt));
    }

    #[test]
    fn typed_int_lt_uses_raw_fast_path() {
        let mut vm = make_vm();
        vm.push_raw_i64(-5).unwrap();
        vm.push_raw_i64(3).unwrap();
        assert!(run_typed_cmp(&mut vm, OpCode::LtInt));
    }

    #[test]
    fn typed_int_gt_uses_raw_fast_path() {
        let mut vm = make_vm();
        vm.push_raw_i64(7).unwrap();
        vm.push_raw_i64(3).unwrap();
        assert!(run_typed_cmp(&mut vm, OpCode::GtInt));
    }

    #[test]
    fn typed_int_gte_lte_boundary_equal() {
        let mut vm = make_vm();
        vm.push_raw_i64(10).unwrap();
        vm.push_raw_i64(10).unwrap();
        assert!(run_typed_cmp(&mut vm, OpCode::GteInt));
        let mut vm = make_vm();
        vm.push_raw_i64(10).unwrap();
        vm.push_raw_i64(10).unwrap();
        assert!(run_typed_cmp(&mut vm, OpCode::LteInt));
    }

    // ----- Raw Number comparison fast paths -----

    #[test]
    fn typed_number_eq_uses_raw_fast_path() {
        let mut vm = make_vm();
        vm.push_raw_f64(1.5).unwrap();
        vm.push_raw_f64(1.5).unwrap();
        assert!(run_typed_cmp(&mut vm, OpCode::EqNumber));
    }

    #[test]
    fn typed_number_lt_uses_raw_fast_path() {
        let mut vm = make_vm();
        vm.push_raw_f64(-1.0).unwrap();
        vm.push_raw_f64(0.5).unwrap();
        assert!(run_typed_cmp(&mut vm, OpCode::LtNumber));
    }

    #[test]
    fn typed_number_gt_uses_raw_fast_path() {
        let mut vm = make_vm();
        vm.push_raw_f64(3.14).unwrap();
        vm.push_raw_f64(2.71).unwrap();
        assert!(run_typed_cmp(&mut vm, OpCode::GtNumber));
    }

    // ----- NaN semantics: IEEE 754 says NaN != NaN, NaN !< NaN, etc. -----

    #[test]
    fn typed_number_eq_nan_is_false() {
        // f64::NAN gets canonicalized on push but the result must still be NaN.
        let mut vm = make_vm();
        vm.push_raw_f64(f64::NAN).unwrap();
        vm.push_raw_f64(f64::NAN).unwrap();
        // EqNumber: NaN == NaN must be false (IEEE 754)
        assert!(!run_typed_cmp(&mut vm, OpCode::EqNumber));
    }

    #[test]
    fn typed_number_neq_nan_is_true() {
        let mut vm = make_vm();
        vm.push_raw_f64(f64::NAN).unwrap();
        vm.push_raw_f64(f64::NAN).unwrap();
        // NeqNumber: NaN != NaN must be true (IEEE 754)
        assert!(run_typed_cmp(&mut vm, OpCode::NeqNumber));
    }

    #[test]
    fn typed_number_lt_nan_is_false() {
        let mut vm = make_vm();
        vm.push_raw_f64(1.0).unwrap();
        vm.push_raw_f64(f64::NAN).unwrap();
        // 1.0 < NaN must be false
        assert!(!run_typed_cmp(&mut vm, OpCode::LtNumber));
    }

    #[test]
    fn typed_number_gt_nan_is_false() {
        let mut vm = make_vm();
        vm.push_raw_f64(1.0).unwrap();
        vm.push_raw_f64(f64::NAN).unwrap();
        // 1.0 > NaN must be false
        assert!(!run_typed_cmp(&mut vm, OpCode::GtNumber));
    }

    #[test]
    fn typed_number_eq_nan_vs_number_is_false() {
        let mut vm = make_vm();
        vm.push_raw_f64(0.0).unwrap();
        vm.push_raw_f64(f64::NAN).unwrap();
        assert!(!run_typed_cmp(&mut vm, OpCode::EqNumber));
    }

    // ----- Negative zero edge case -----

    #[test]
    fn typed_number_eq_treats_neg_zero_as_zero() {
        // IEEE 754: -0.0 == 0.0 (the only case where bit-equality differs from
        // numerical equality for non-NaN floats)
        let mut vm = make_vm();
        vm.push_raw_f64(-0.0).unwrap();
        vm.push_raw_f64(0.0).unwrap();
        assert!(run_typed_cmp(&mut vm, OpCode::EqNumber));
    }

    // ----- Slow path: ensures vw fallback still works for mixed types -----

    #[test]
    fn typed_int_eq_slow_path_handles_legacy_vw() {
        // Push via the legacy ValueWord path so the fast-path detector misses;
        // the slow path (as_i64_unchecked) must still produce correct results.
        let mut vm = make_vm();
        vm.push_vw(ValueWord::from_i64(100)).unwrap();
        vm.push_vw(ValueWord::from_i64(100)).unwrap();
        assert!(run_typed_cmp(&mut vm, OpCode::EqInt));
    }

    #[test]
    fn typed_number_eq_slow_path_handles_legacy_vw() {
        let mut vm = make_vm();
        vm.push_vw(ValueWord::from_f64(2.5)).unwrap();
        vm.push_vw(ValueWord::from_f64(2.5)).unwrap();
        assert!(run_typed_cmp(&mut vm, OpCode::EqNumber));
    }

    // ----- Stage 2.6.3: typed equality for heap-backed types -----

    #[test]
    fn typed_string_eq_same_content_is_true() {
        let mut vm = make_vm();
        vm.push_vw(ValueWord::from_string(std::sync::Arc::new("hello".to_string())))
            .unwrap();
        vm.push_vw(ValueWord::from_string(std::sync::Arc::new("hello".to_string())))
            .unwrap();
        assert!(run_typed_cmp(&mut vm, OpCode::EqString));
    }

    #[test]
    fn typed_string_eq_different_content_is_false() {
        let mut vm = make_vm();
        vm.push_vw(ValueWord::from_string(std::sync::Arc::new("foo".to_string())))
            .unwrap();
        vm.push_vw(ValueWord::from_string(std::sync::Arc::new("bar".to_string())))
            .unwrap();
        assert!(!run_typed_cmp(&mut vm, OpCode::EqString));
    }

    #[test]
    fn typed_string_eq_empty_strings_are_equal() {
        let mut vm = make_vm();
        vm.push_vw(ValueWord::from_string(std::sync::Arc::new(String::new())))
            .unwrap();
        vm.push_vw(ValueWord::from_string(std::sync::Arc::new(String::new())))
            .unwrap();
        assert!(run_typed_cmp(&mut vm, OpCode::EqString));
    }

    #[test]
    fn typed_decimal_eq_same_value_is_true() {
        use rust_decimal::Decimal;
        use std::str::FromStr;
        let mut vm = make_vm();
        vm.push_vw(ValueWord::from_decimal(Decimal::from_str("12.34").unwrap()))
            .unwrap();
        vm.push_vw(ValueWord::from_decimal(Decimal::from_str("12.34").unwrap()))
            .unwrap();
        assert!(run_typed_cmp(&mut vm, OpCode::EqDecimal));
    }

    #[test]
    fn typed_decimal_eq_different_value_is_false() {
        use rust_decimal::Decimal;
        use std::str::FromStr;
        let mut vm = make_vm();
        vm.push_vw(ValueWord::from_decimal(Decimal::from_str("1.0").unwrap()))
            .unwrap();
        vm.push_vw(ValueWord::from_decimal(Decimal::from_str("2.0").unwrap()))
            .unwrap();
        assert!(!run_typed_cmp(&mut vm, OpCode::EqDecimal));
    }

    // ----- Stage 2.6.5.1: IsNull typed absence check -----

    fn run_is_null(vm: &mut VirtualMachine) -> bool {
        let instr = Instruction { opcode: OpCode::IsNull, operand: None };
        vm.exec_typed_comparison(&instr).unwrap();
        unsafe { vm.pop_vw().unwrap().as_bool_unchecked() }
    }

    #[test]
    fn is_null_on_none_returns_true() {
        let mut vm = make_vm();
        vm.push_vw(ValueWord::none()).unwrap();
        assert!(run_is_null(&mut vm));
    }

    #[test]
    fn is_null_on_unit_returns_true() {
        let mut vm = make_vm();
        vm.push_vw(ValueWord::unit()).unwrap();
        assert!(run_is_null(&mut vm));
    }

    #[test]
    fn is_null_on_int_returns_false() {
        let mut vm = make_vm();
        vm.push_vw(ValueWord::from_i64(42)).unwrap();
        assert!(!run_is_null(&mut vm));
    }

    #[test]
    fn is_null_on_zero_int_returns_false() {
        // Critical: int 0 is NOT null, even though some null encodings
        // use raw zero. IsNull must check the tag, not the bit pattern.
        let mut vm = make_vm();
        vm.push_vw(ValueWord::from_i64(0)).unwrap();
        assert!(!run_is_null(&mut vm));
    }

    #[test]
    fn is_null_on_string_returns_false() {
        let mut vm = make_vm();
        vm.push_vw(ValueWord::from_string(std::sync::Arc::new("hello".to_string())))
            .unwrap();
        assert!(!run_is_null(&mut vm));
    }

    #[test]
    fn is_null_on_false_bool_returns_false() {
        // Critical: bool false is NOT null. The is_truthy check would
        // conflate them but is_none() / is_unit() correctly distinguish.
        let mut vm = make_vm();
        vm.push_vw(ValueWord::from_bool(false)).unwrap();
        assert!(!run_is_null(&mut vm));
    }
}
