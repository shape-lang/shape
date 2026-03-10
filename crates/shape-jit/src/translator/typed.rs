//! Typed Code Generation for JIT
//!
//! This module generates type-specialized Cranelift IR based on StorageType.
//!
//! ## Key Optimization: Unboxed Operations
//!
//! When operand types are known at compile time, we generate unboxed operations:
//!
//! ```text
//! // Boxed (dynamic) - current approach:
//! a_boxed: i64 = load from stack
//! b_boxed: i64 = load from stack
//! type_check(a_boxed, b_boxed)     // Runtime check
//! a_f64 = bitcast(a_boxed)          // Convert
//! b_f64 = bitcast(b_boxed)          // Convert
//! result_f64 = fadd(a_f64, b_f64)   // Operation
//! result_boxed = bitcast(result_f64) // Convert back
//!
//! // Unboxed (typed) - new approach:
//! a_f64: f64 = load from typed stack
//! b_f64: f64 = load from typed stack
//! result_f64 = fadd(a_f64, b_f64)   // Just the operation!
//! ```
//!
//! The unboxed approach eliminates type checks, boxing/unboxing, and branches.

use cranelift::prelude::*;

use super::storage::{CraneliftRepr, TypedValue};
use super::types::BytecodeToIR;

/// Extension trait for BytecodeToIR to add typed operations
#[allow(dead_code)]
impl<'a, 'b> BytecodeToIR<'a, 'b> {
    // ========================================================================
    // Typed Stack Operations
    // ========================================================================

    /// Push a typed f64 constant (unboxed)
    pub(crate) fn push_typed_f64(&mut self, value: f64) {
        let f64_val = self.builder.ins().f64const(value);
        self.typed_stack.push(TypedValue::f64(f64_val));
    }

    /// Push a typed i64 constant (unboxed)
    pub(crate) fn push_typed_i64(&mut self, value: i64) {
        let i64_val = self.builder.ins().iconst(types::I64, value);
        self.typed_stack.push(TypedValue::i64(i64_val));
    }

    /// Push a boxed value (for dynamic types)
    pub(crate) fn push_boxed(&mut self, value: Value) {
        self.typed_stack.push(TypedValue::boxed(value));
    }

    /// Pop a typed value
    pub(crate) fn pop_typed(&mut self) -> Option<TypedValue> {
        self.typed_stack.pop()
    }

    /// Check if we can use unboxed binary operation
    pub(crate) fn can_unbox_binary(&self) -> bool {
        self.typed_stack.can_unbox_binary_top()
    }

    // ========================================================================
    // Typed Binary Operations
    // ========================================================================

    /// Perform typed addition
    ///
    /// - If both operands are unboxed f64: direct fadd
    /// - If both operands are unboxed i64: direct iadd
    /// - Otherwise: fall back to boxed operation
    pub(crate) fn typed_add(&mut self) -> Result<TypedValue, String> {
        let b = self.pop_typed().ok_or("Stack underflow")?;
        let a = self.pop_typed().ok_or("Stack underflow")?;

        let result = match (a.repr, b.repr) {
            (CraneliftRepr::F64, CraneliftRepr::F64) => {
                // Unboxed f64 add - NaN propagates for nullable
                let result = self.builder.ins().fadd(a.value, b.value);
                TypedValue::new(result, CraneliftRepr::F64, a.nullable || b.nullable)
            }
            (CraneliftRepr::I64, CraneliftRepr::I64) => {
                // Unboxed i64 add
                let result = self.builder.ins().iadd(a.value, b.value);
                TypedValue::new(result, CraneliftRepr::I64, a.nullable || b.nullable)
            }
            _ => {
                // Fall back to boxed operation
                let a_boxed = self.ensure_boxed(a);
                let b_boxed = self.ensure_boxed(b);
                let inst = self
                    .builder
                    .ins()
                    .call(self.ffi.generic_add, &[a_boxed, b_boxed]);
                let result = self.builder.inst_results(inst)[0];
                TypedValue::boxed(result)
            }
        };

        self.typed_stack.push(result);
        Ok(result)
    }

    /// Perform typed subtraction
    pub(crate) fn typed_sub(&mut self) -> Result<TypedValue, String> {
        let b = self.pop_typed().ok_or("Stack underflow")?;
        let a = self.pop_typed().ok_or("Stack underflow")?;

        let result = match (a.repr, b.repr) {
            (CraneliftRepr::F64, CraneliftRepr::F64) => {
                let result = self.builder.ins().fsub(a.value, b.value);
                TypedValue::new(result, CraneliftRepr::F64, a.nullable || b.nullable)
            }
            (CraneliftRepr::I64, CraneliftRepr::I64) => {
                let result = self.builder.ins().isub(a.value, b.value);
                TypedValue::new(result, CraneliftRepr::I64, a.nullable || b.nullable)
            }
            _ => {
                let a_boxed = self.ensure_boxed(a);
                let b_boxed = self.ensure_boxed(b);
                let inst = self
                    .builder
                    .ins()
                    .call(self.ffi.generic_sub, &[a_boxed, b_boxed]);
                let result = self.builder.inst_results(inst)[0];
                TypedValue::boxed(result)
            }
        };

        self.typed_stack.push(result);
        Ok(result)
    }

    /// Perform typed multiplication
    pub(crate) fn typed_mul(&mut self) -> Result<TypedValue, String> {
        let b = self.pop_typed().ok_or("Stack underflow")?;
        let a = self.pop_typed().ok_or("Stack underflow")?;

        let result = match (a.repr, b.repr) {
            (CraneliftRepr::F64, CraneliftRepr::F64) => {
                let result = self.builder.ins().fmul(a.value, b.value);
                TypedValue::new(result, CraneliftRepr::F64, a.nullable || b.nullable)
            }
            (CraneliftRepr::I64, CraneliftRepr::I64) => {
                let result = self.builder.ins().imul(a.value, b.value);
                TypedValue::new(result, CraneliftRepr::I64, a.nullable || b.nullable)
            }
            _ => {
                let a_boxed = self.ensure_boxed(a);
                let b_boxed = self.ensure_boxed(b);
                let inst = self
                    .builder
                    .ins()
                    .call(self.ffi.generic_mul, &[a_boxed, b_boxed]);
                let result = self.builder.inst_results(inst)[0];
                TypedValue::boxed(result)
            }
        };

        self.typed_stack.push(result);
        Ok(result)
    }

    /// Perform typed division
    pub(crate) fn typed_div(&mut self) -> Result<TypedValue, String> {
        let b = self.pop_typed().ok_or("Stack underflow")?;
        let a = self.pop_typed().ok_or("Stack underflow")?;

        let result = match (a.repr, b.repr) {
            (CraneliftRepr::F64, CraneliftRepr::F64) => {
                // f64 division - produces NaN on 0/0, Inf on x/0
                let result = self.builder.ins().fdiv(a.value, b.value);
                TypedValue::new(result, CraneliftRepr::F64, a.nullable || b.nullable)
            }
            (CraneliftRepr::I64, CraneliftRepr::I64) => {
                // i64 division - truncated toward zero, matching VM semantics.
                let result = self.builder.ins().sdiv(a.value, b.value);
                TypedValue::new(result, CraneliftRepr::I64, a.nullable || b.nullable)
            }
            _ => {
                let a_boxed = self.ensure_boxed(a);
                let b_boxed = self.ensure_boxed(b);
                let inst = self
                    .builder
                    .ins()
                    .call(self.ffi.generic_div, &[a_boxed, b_boxed]);
                let result = self.builder.inst_results(inst)[0];
                TypedValue::boxed(result)
            }
        };

        self.typed_stack.push(result);
        Ok(result)
    }

    // ========================================================================
    // Typed Unary Operations
    // ========================================================================

    /// Perform typed negation
    pub(crate) fn typed_neg(&mut self) -> Result<TypedValue, String> {
        let a = self.pop_typed().ok_or("Stack underflow")?;

        let result = match a.repr {
            CraneliftRepr::F64 => {
                let result = self.builder.ins().fneg(a.value);
                TypedValue::new(result, CraneliftRepr::F64, a.nullable)
            }
            CraneliftRepr::I64 => {
                let result = self.builder.ins().ineg(a.value);
                TypedValue::new(result, CraneliftRepr::I64, a.nullable)
            }
            _ => {
                // Fall back to boxed operation
                let boxed = self.ensure_boxed(a);
                let f64_val = self.i64_to_f64(boxed);
                let neg = self.builder.ins().fneg(f64_val);
                let result = self.f64_to_i64(neg);
                TypedValue::boxed(result)
            }
        };

        self.typed_stack.push(result);
        Ok(result)
    }

    // ========================================================================
    // Series SIMD Operations (Zero-Copy, High Performance)
    // ========================================================================

    /// Perform SIMD addition on two Series: result[i] = a[i] + b[i]
    /// Returns a new Series with data_ptr and len
    #[allow(dead_code)]
    pub(crate) fn typed_series_add(
        &mut self,
        a: TypedValue,
        b: TypedValue,
    ) -> Result<TypedValue, String> {
        self.typed_series_binary_op(a, b, self.ffi.simd_add)
    }

    /// Perform SIMD subtraction on two Series
    #[allow(dead_code)]
    pub(crate) fn typed_series_sub(
        &mut self,
        a: TypedValue,
        b: TypedValue,
    ) -> Result<TypedValue, String> {
        self.typed_series_binary_op(a, b, self.ffi.simd_sub)
    }

    /// Perform SIMD multiplication on two Series
    #[allow(dead_code)]
    pub(crate) fn typed_series_mul(
        &mut self,
        a: TypedValue,
        b: TypedValue,
    ) -> Result<TypedValue, String> {
        self.typed_series_binary_op(a, b, self.ffi.simd_mul)
    }

    /// Perform SIMD division on two Series
    #[allow(dead_code)]
    pub(crate) fn typed_series_div(
        &mut self,
        a: TypedValue,
        b: TypedValue,
    ) -> Result<TypedValue, String> {
        self.typed_series_binary_op(a, b, self.ffi.simd_div)
    }

    /// Perform SIMD greater-than comparison on two Series
    #[allow(dead_code)]
    pub(crate) fn typed_series_gt(
        &mut self,
        a: TypedValue,
        b: TypedValue,
    ) -> Result<TypedValue, String> {
        self.typed_series_binary_op(a, b, self.ffi.simd_gt)
    }

    /// Perform SIMD less-than comparison on two Series
    #[allow(dead_code)]
    pub(crate) fn typed_series_lt(
        &mut self,
        a: TypedValue,
        b: TypedValue,
    ) -> Result<TypedValue, String> {
        self.typed_series_binary_op(a, b, self.ffi.simd_lt)
    }

    /// Helper: emit a SIMD binary operation call
    /// simd_op(ptr_a, ptr_b, len) -> result_ptr
    fn typed_series_binary_op(
        &mut self,
        a: TypedValue,
        b: TypedValue,
        simd_func: cranelift::codegen::ir::FuncRef,
    ) -> Result<TypedValue, String> {
        if !a.is_series() || !b.is_series() {
            return Err("typed_series_binary_op requires two Series".to_string());
        }

        let a_ptr = a.value;
        let b_ptr = b.value;
        let a_len = a.get_series_len();
        let b_len = b.get_series_len();

        // Use minimum length (for safety)
        let len = self.builder.ins().umin(a_len, b_len);

        // Call SIMD function: simd_op(ptr_a, ptr_b, len) -> result_ptr
        let inst = self.builder.ins().call(simd_func, &[a_ptr, b_ptr, len]);
        let result_ptr = self.builder.inst_results(inst)[0];

        // Result is a new Series with same length
        Ok(TypedValue::series(result_ptr, len))
    }

    // ========================================================================
    // Type Conversion Helpers
    // ========================================================================

    /// Ensure a typed value is boxed (for FFI calls)
    fn ensure_boxed(&mut self, tv: TypedValue) -> Value {
        match tv.repr {
            CraneliftRepr::F64 => {
                // Box f64 as NaN-boxed value
                self.f64_to_i64(tv.value)
            }
            CraneliftRepr::I64 => {
                // Box i64 - convert to f64 first (Shape numbers are f64)
                let f64_val = self.builder.ins().fcvt_from_sint(types::F64, tv.value);
                self.f64_to_i64(f64_val)
            }
            CraneliftRepr::I8 => {
                // Box bool
                use crate::nan_boxing::{TAG_BOOL_FALSE, TAG_BOOL_TRUE};
                let true_val = self.builder.ins().iconst(types::I64, TAG_BOOL_TRUE as i64);
                let false_val = self.builder.ins().iconst(types::I64, TAG_BOOL_FALSE as i64);
                let zero = self.builder.ins().iconst(types::I8, 0);
                let is_true = self.builder.ins().icmp(IntCC::NotEqual, tv.value, zero);
                self.builder.ins().select(is_true, true_val, false_val)
            }
            CraneliftRepr::NanBoxed => {
                // Already boxed
                tv.value
            }
            CraneliftRepr::Series => {
                // Series is stored as raw pointer - pass through
                // Note: This should not normally be called for Series
                // as Series ops use direct SIMD calls
                tv.value
            }
        }
    }

    /// Convert boxed value to typed (unbox)
    #[allow(dead_code)]
    fn unbox_to_typed(&mut self, boxed: Value, target: CraneliftRepr) -> TypedValue {
        match target {
            CraneliftRepr::F64 => {
                let f64_val = self.i64_to_f64(boxed);
                TypedValue::f64(f64_val)
            }
            CraneliftRepr::I64 => {
                let f64_val = self.i64_to_f64(boxed);
                let i64_val = self.builder.ins().fcvt_to_sint_sat(types::I64, f64_val);
                TypedValue::i64(i64_val)
            }
            CraneliftRepr::I8 => {
                // Unbox bool - check against TAG_BOOL_TRUE
                use crate::nan_boxing::TAG_BOOL_TRUE;
                let true_tag = self.builder.ins().iconst(types::I64, TAG_BOOL_TRUE as i64);
                let is_true = self.builder.ins().icmp(IntCC::Equal, boxed, true_tag);
                let one = self.builder.ins().iconst(types::I8, 1);
                let zero = self.builder.ins().iconst(types::I8, 0);
                let result = self.builder.ins().select(is_true, one, zero);
                TypedValue::new(result, CraneliftRepr::I8, false)
            }
            CraneliftRepr::NanBoxed => TypedValue::boxed(boxed),
            CraneliftRepr::Series => {
                // Series cannot be unboxed this way - it needs ptr + len extraction
                // This is a placeholder; proper Series unboxing needs special handling
                TypedValue::boxed(boxed)
            }
        }
    }
}
