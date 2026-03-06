//! StorageType to Cranelift IR Mapping
//!
//! This module bridges Shape's type system with Cranelift's IR generation.
//! Key insight: we generate DIFFERENT code based on StorageType:
//!
//! - `Float64` / `NullableFloat64` → raw f64 operations (no boxing)
//! - `Int64` / `NullableInt64` → raw i64 operations (no boxing)
//! - `Dynamic` → NaN-boxed u64 with runtime type checks
//!
//! This is the core of typed JIT compilation.

use cranelift::prelude::*;
use shape_runtime::type_system::StorageType;

/// Cranelift type representation for a StorageType
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum CraneliftRepr {
    /// Raw f64 value (for Float64, NullableFloat64)
    /// NullableFloat64 uses NaN as sentinel - IEEE 754 propagation handles null
    F64,

    /// Raw i64 value (for Int64)
    I64,

    /// NaN-boxed u64 representation for dynamic/complex types
    NanBoxed,

    /// Boolean as i8 (0 = false, 1 = true, 2 = None for nullable)
    I8,

    /// Series: raw pointer to f64 data buffer + length
    /// Enables zero-copy SIMD operations: simd_add(ptr_a, ptr_b, len)
    /// The TypedValue stores (data_ptr: Value, len: Value) separately
    Series,
}

#[allow(dead_code)]
impl CraneliftRepr {
    /// Get the Cranelift type for this representation
    pub fn cranelift_type(&self) -> types::Type {
        match self {
            CraneliftRepr::F64 => types::F64,
            CraneliftRepr::I64 => types::I64,
            CraneliftRepr::NanBoxed => types::I64, // NaN-boxed representation
            CraneliftRepr::I8 => types::I8,
            CraneliftRepr::Series => types::I64, // Pointer type
        }
    }

    /// Check if this representation can use direct operations (no boxing)
    pub fn is_unboxed(&self) -> bool {
        matches!(
            self,
            CraneliftRepr::F64 | CraneliftRepr::I64 | CraneliftRepr::I8
        )
    }

    /// Check if this representation supports NaN-sentinel null semantics
    pub fn uses_nan_null(&self) -> bool {
        matches!(self, CraneliftRepr::F64)
    }

    /// Check if this is a Series (vector) type
    pub fn is_series(&self) -> bool {
        matches!(self, CraneliftRepr::Series)
    }
}

/// Map StorageType to CraneliftRepr
#[allow(dead_code)]
pub fn storage_to_repr(storage: &StorageType) -> CraneliftRepr {
    match storage {
        // Float types → raw f64
        StorageType::Float64 | StorageType::NullableFloat64 => CraneliftRepr::F64,

        // Integer types → raw i64
        StorageType::Int64 | StorageType::NullableInt64 => CraneliftRepr::I64,

        // Boolean → i8
        StorageType::Bool | StorageType::NullableBool => CraneliftRepr::I8,

        // Table → raw pointer + length for zero-copy SIMD
        StorageType::Table { .. } => CraneliftRepr::Series,

        // Everything else → NaN-boxed (dynamic dispatch)
        StorageType::String
        | StorageType::Array(_)
        | StorageType::Object
        | StorageType::Result { .. }
        | StorageType::TaggedUnion { .. }
        | StorageType::Function
        | StorageType::Struct(_)
        | StorageType::Dynamic => CraneliftRepr::NanBoxed,
    }
}

/// Typed value tracking for JIT compilation
///
/// This tracks both the Cranelift Value and its storage type,
/// enabling type-aware code generation.
///
/// For Series, we track both the data pointer and length to enable
/// zero-copy SIMD operations with signature: simd_op(ptr_a, ptr_b, len)
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub struct TypedValue {
    /// The Cranelift IR value (for Series: data pointer)
    pub value: Value,
    /// The storage representation
    pub repr: CraneliftRepr,
    /// Whether this value can be None (for nullable types)
    pub nullable: bool,
    /// For Series: the length value (number of f64 elements)
    /// None for non-Series types
    pub series_len: Option<Value>,
    /// For Bool from typed_comparison: the raw i1 fcmp result
    /// Enables fused comparison-branch (skip boolean boxing/unboxing)
    pub raw_cmp: Option<Value>,
    /// Hoisted array metadata for values known to be invariant arrays.
    /// When present, `GetProp` can skip tag/payload extraction and directly
    /// use preloaded `(data_ptr, len)` from loop preheader.
    pub hoisted_array_info: Option<(Value, Value)>,
}

#[allow(dead_code)]
impl TypedValue {
    /// Create a new typed value
    pub fn new(value: Value, repr: CraneliftRepr, nullable: bool) -> Self {
        Self {
            value,
            repr,
            nullable,
            series_len: None,
            raw_cmp: None,
            hoisted_array_info: None,
        }
    }

    /// Create a typed value for a known f64
    pub fn f64(value: Value) -> Self {
        Self::new(value, CraneliftRepr::F64, false)
    }

    /// Create a typed value for a nullable f64 (Option<f64>)
    pub fn nullable_f64(value: Value) -> Self {
        Self::new(value, CraneliftRepr::F64, true)
    }

    /// Create a typed value for a known i64
    pub fn i64(value: Value) -> Self {
        Self::new(value, CraneliftRepr::I64, false)
    }

    /// Create a typed value for a boxed/dynamic value
    pub fn boxed(value: Value) -> Self {
        Self::new(value, CraneliftRepr::NanBoxed, false)
    }

    /// Create a typed value for a Series with raw data pointer + length
    /// Enables zero-copy SIMD: simd_add(data_ptr_a, data_ptr_b, len)
    pub fn series(data_ptr: Value, len: Value) -> Self {
        Self {
            value: data_ptr,
            repr: CraneliftRepr::Series,
            nullable: false,
            series_len: Some(len),
            raw_cmp: None,
            hoisted_array_info: None,
        }
    }

    /// Create a Bool typed value with the raw i1 comparison result cached.
    /// Enables fused comparison-branch: JumpIfFalse/JumpIfTrue can use
    /// the raw fcmp directly instead of unboxing the boolean.
    pub fn bool_with_raw_cmp(boxed_value: Value, raw_cmp: Value) -> Self {
        Self {
            value: boxed_value,
            repr: CraneliftRepr::NanBoxed,
            nullable: false,
            series_len: None,
            raw_cmp: Some(raw_cmp),
            hoisted_array_info: None,
        }
    }

    /// Attach hoisted array metadata to this typed value.
    pub fn with_hoisted_array_info(mut self, data_ptr: Value, len: Value) -> Self {
        self.hoisted_array_info = Some((data_ptr, len));
        self
    }

    /// Check if this is a Series type
    pub fn is_series(&self) -> bool {
        self.repr.is_series()
    }

    /// Get Series length (panics if not a Series)
    pub fn get_series_len(&self) -> Value {
        self.series_len
            .expect("get_series_len called on non-Series TypedValue")
    }

    /// Check if binary operation can use direct unboxed ops
    pub fn can_unbox_binary(a: &TypedValue, b: &TypedValue) -> bool {
        a.repr.is_unboxed() && b.repr.is_unboxed() && a.repr == b.repr
    }

    /// Check if binary operation can use SIMD (both are Series)
    pub fn can_simd_binary(a: &TypedValue, b: &TypedValue) -> bool {
        a.repr.is_series() && b.repr.is_series()
    }

    /// Compute result type for binary operation
    pub fn binary_result_type(a: &TypedValue, b: &TypedValue) -> CraneliftRepr {
        if a.repr == b.repr && a.repr.is_unboxed() {
            a.repr
        } else if a.repr.is_series() && b.repr.is_series() {
            CraneliftRepr::Series
        } else {
            CraneliftRepr::NanBoxed
        }
    }

    /// Compute result nullability for binary operation
    /// Any nullable operand → nullable result
    pub fn binary_result_nullable(a: &TypedValue, b: &TypedValue) -> bool {
        a.nullable || b.nullable
    }
}

/// Typed stack for JIT compilation
///
/// Tracks both values and their types through operations.
/// This enables generating optimal code based on known types.
#[derive(Debug, Default)]
pub struct TypedStack {
    /// Stack of typed values
    stack: Vec<TypedValue>,
}

#[allow(dead_code)]
impl TypedStack {
    pub fn new() -> Self {
        Self { stack: Vec::new() }
    }

    pub fn push(&mut self, tv: TypedValue) {
        self.stack.push(tv);
    }

    pub fn pop(&mut self) -> Option<TypedValue> {
        self.stack.pop()
    }

    pub fn peek(&self) -> Option<&TypedValue> {
        self.stack.last()
    }

    pub fn second(&self) -> Option<&TypedValue> {
        if self.stack.len() < 2 {
            return None;
        }
        self.stack.get(self.stack.len() - 2)
    }

    /// Hoisted array metadata for the second value from top (used by GetProp:
    /// top is key, second is object).
    pub fn second_hoisted_array_info(&self) -> Option<(Value, Value)> {
        self.second().and_then(|tv| tv.hoisted_array_info)
    }

    pub fn len(&self) -> usize {
        self.stack.len()
    }

    pub fn is_empty(&self) -> bool {
        self.stack.is_empty()
    }

    /// Replace the top entry (used after stack_push to upgrade from boxed to typed)
    pub fn replace_top(&mut self, tv: TypedValue) {
        if let Some(top) = self.stack.last_mut() {
            *top = tv;
        }
    }

    /// Clear all entries (used at block boundaries where SSA values may not dominate)
    pub fn clear(&mut self) {
        self.stack.clear();
    }

    /// Check if top two values can use unboxed binary operation
    pub fn can_unbox_binary_top(&self) -> bool {
        if self.stack.len() < 2 {
            return false;
        }
        let a = &self.stack[self.stack.len() - 1];
        let b = &self.stack[self.stack.len() - 2];
        TypedValue::can_unbox_binary(a, b)
    }

    /// Check if top two values can use SIMD binary operation (both Series)
    pub fn can_simd_binary_top(&self) -> bool {
        if self.stack.len() < 2 {
            return false;
        }
        let a = &self.stack[self.stack.len() - 1];
        let b = &self.stack[self.stack.len() - 2];
        TypedValue::can_simd_binary(a, b)
    }

    /// Get representation of top value
    pub fn top_repr(&self) -> CraneliftRepr {
        self.stack
            .last()
            .map(|v| v.repr)
            .unwrap_or(CraneliftRepr::NanBoxed)
    }

    /// Check if top value is a Series
    pub fn top_is_series(&self) -> bool {
        self.stack.last().map(|v| v.is_series()).unwrap_or(false)
    }

    /// Check if the top two values are both raw i64 (for integer unboxing fast path)
    pub fn both_top_i64(&self) -> bool {
        if self.stack.len() < 2 {
            return false;
        }
        let a = &self.stack[self.stack.len() - 1];
        let b = &self.stack[self.stack.len() - 2];
        a.repr == CraneliftRepr::I64 && b.repr == CraneliftRepr::I64
    }

    /// Check if at least one of the top two values is raw i64.
    /// Used for mixed-type operations where one operand is from an unboxed local
    /// and the other is from PushConst (NaN-boxed).
    pub fn either_top_i64(&self) -> bool {
        if self.stack.len() < 2 {
            return false;
        }
        let a = &self.stack[self.stack.len() - 1];
        let b = &self.stack[self.stack.len() - 2];
        a.repr == CraneliftRepr::I64 || b.repr == CraneliftRepr::I64
    }

    /// Returns (top_is_i64, second_is_i64) for the top two stack entries.
    pub fn top_two_i64_flags(&self) -> (bool, bool) {
        if self.stack.len() < 2 {
            return (false, false);
        }
        let top = self.stack[self.stack.len() - 1].repr == CraneliftRepr::I64;
        let second = self.stack[self.stack.len() - 2].repr == CraneliftRepr::I64;
        (top, second)
    }

    /// Check if both top two values are raw f64 (for float unboxing fast path)
    pub fn both_top_f64(&self) -> bool {
        if self.stack.len() < 2 {
            return false;
        }
        let a = &self.stack[self.stack.len() - 1];
        let b = &self.stack[self.stack.len() - 2];
        a.repr == CraneliftRepr::F64 && b.repr == CraneliftRepr::F64
    }

    /// Check if at least one of the top two values is raw f64.
    pub fn either_top_f64(&self) -> bool {
        if self.stack.len() < 2 {
            return false;
        }
        let a = &self.stack[self.stack.len() - 1];
        let b = &self.stack[self.stack.len() - 2];
        a.repr == CraneliftRepr::F64 || b.repr == CraneliftRepr::F64
    }

    /// Returns (top_is_f64, second_is_f64) for the top two stack entries.
    pub fn top_two_f64_flags(&self) -> (bool, bool) {
        if self.stack.len() < 2 {
            return (false, false);
        }
        let top = self.stack[self.stack.len() - 1].repr == CraneliftRepr::F64;
        let second = self.stack[self.stack.len() - 2].repr == CraneliftRepr::F64;
        (top, second)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_storage_to_repr() {
        assert_eq!(storage_to_repr(&StorageType::Float64), CraneliftRepr::F64);
        assert_eq!(
            storage_to_repr(&StorageType::NullableFloat64),
            CraneliftRepr::F64
        );
        assert_eq!(storage_to_repr(&StorageType::Int64), CraneliftRepr::I64);
        assert_eq!(
            storage_to_repr(&StorageType::Dynamic),
            CraneliftRepr::NanBoxed
        );
    }

    #[test]
    fn test_repr_properties() {
        assert!(CraneliftRepr::F64.is_unboxed());
        assert!(CraneliftRepr::F64.uses_nan_null());
        assert!(CraneliftRepr::I64.is_unboxed());
        assert!(!CraneliftRepr::I64.uses_nan_null());
        assert!(!CraneliftRepr::NanBoxed.is_unboxed());
    }
}
