//! TypedScalar: type-preserving scalar boundary contract for VM↔JIT exchange.
//!
//! When scalar values cross the VM/JIT boundary, their type identity (int vs
//! float) must be preserved. `TypedScalar` carries an explicit `ScalarKind`
//! discriminator alongside the raw payload bits.

use crate::slot::ValueSlot;

/// Scalar type discriminator.
///
/// Covers all width-specific numeric types plus bool/none/unit sentinels.
/// Discriminant values are part of the ABI — do not reorder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ScalarKind {
    I8 = 0,
    U8 = 1,
    I16 = 2,
    U16 = 3,
    I32 = 4,
    U32 = 5,
    I64 = 6,
    U64 = 7,
    I128 = 8,
    U128 = 9,
    F32 = 10,
    F64 = 11,
    Bool = 12,
    None = 13,
    Unit = 14,
}

impl ScalarKind {
    /// True if this kind represents an integer type (signed or unsigned).
    #[inline]
    pub fn is_integer(self) -> bool {
        matches!(
            self,
            ScalarKind::I8
                | ScalarKind::U8
                | ScalarKind::I16
                | ScalarKind::U16
                | ScalarKind::I32
                | ScalarKind::U32
                | ScalarKind::I64
                | ScalarKind::U64
                | ScalarKind::I128
                | ScalarKind::U128
        )
    }

    /// True if this kind represents a floating-point type.
    #[inline]
    pub fn is_float(self) -> bool {
        matches!(self, ScalarKind::F32 | ScalarKind::F64)
    }

    /// True if this kind represents a numeric type (integer or float).
    #[inline]
    pub fn is_numeric(self) -> bool {
        self.is_integer() || self.is_float()
    }

    /// True if this kind represents an unsigned integer type.
    #[inline]
    pub fn is_unsigned_integer(self) -> bool {
        matches!(
            self,
            ScalarKind::U8 | ScalarKind::U16 | ScalarKind::U32 | ScalarKind::U64 | ScalarKind::U128
        )
    }

    /// True if this kind represents a signed integer type.
    #[inline]
    pub fn is_signed_integer(self) -> bool {
        matches!(
            self,
            ScalarKind::I8 | ScalarKind::I16 | ScalarKind::I32 | ScalarKind::I64 | ScalarKind::I128
        )
    }
}

/// Type-preserving scalar value for VM↔JIT boundary exchange.
///
/// Carries an explicit type discriminator (`kind`) so that integer 42 and
/// float 42.0 are distinguishable even when their f64 bit patterns would
/// be identical.
///
/// `payload_hi` is zero for all types smaller than 128 bits.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(C)]
pub struct TypedScalar {
    pub kind: ScalarKind,
    pub payload_lo: u64,
    /// Second 64-bit word — only used for I128/U128. Zero otherwise.
    pub payload_hi: u64,
}

impl TypedScalar {
    /// Create an I64 scalar.
    #[inline]
    pub fn i64(v: i64) -> Self {
        Self {
            kind: ScalarKind::I64,
            payload_lo: v as u64,
            payload_hi: 0,
        }
    }

    /// Create an F64 scalar from a value.
    #[inline]
    pub fn f64(v: f64) -> Self {
        Self {
            kind: ScalarKind::F64,
            payload_lo: v.to_bits(),
            payload_hi: 0,
        }
    }

    /// Create an F64 scalar from raw bits.
    #[inline]
    pub fn f64_from_bits(bits: u64) -> Self {
        Self {
            kind: ScalarKind::F64,
            payload_lo: bits,
            payload_hi: 0,
        }
    }

    /// Create a Bool scalar.
    #[inline]
    pub fn bool(v: bool) -> Self {
        Self {
            kind: ScalarKind::Bool,
            payload_lo: v as u64,
            payload_hi: 0,
        }
    }

    /// Create a None scalar.
    #[inline]
    pub fn none() -> Self {
        Self {
            kind: ScalarKind::None,
            payload_lo: 0,
            payload_hi: 0,
        }
    }

    /// Create a Unit scalar.
    #[inline]
    pub fn unit() -> Self {
        Self {
            kind: ScalarKind::Unit,
            payload_lo: 0,
            payload_hi: 0,
        }
    }

    /// Create an I8 scalar.
    #[inline]
    pub fn i8(v: i8) -> Self {
        Self {
            kind: ScalarKind::I8,
            payload_lo: v as i64 as u64,
            payload_hi: 0,
        }
    }

    /// Create a U8 scalar.
    #[inline]
    pub fn u8(v: u8) -> Self {
        Self {
            kind: ScalarKind::U8,
            payload_lo: v as u64,
            payload_hi: 0,
        }
    }

    /// Create an I16 scalar.
    #[inline]
    pub fn i16(v: i16) -> Self {
        Self {
            kind: ScalarKind::I16,
            payload_lo: v as i64 as u64,
            payload_hi: 0,
        }
    }

    /// Create a U16 scalar.
    #[inline]
    pub fn u16(v: u16) -> Self {
        Self {
            kind: ScalarKind::U16,
            payload_lo: v as u64,
            payload_hi: 0,
        }
    }

    /// Create an I32 scalar.
    #[inline]
    pub fn i32(v: i32) -> Self {
        Self {
            kind: ScalarKind::I32,
            payload_lo: v as i64 as u64,
            payload_hi: 0,
        }
    }

    /// Create a U32 scalar.
    #[inline]
    pub fn u32(v: u32) -> Self {
        Self {
            kind: ScalarKind::U32,
            payload_lo: v as u64,
            payload_hi: 0,
        }
    }

    /// Create a U64 scalar.
    #[inline]
    pub fn u64(v: u64) -> Self {
        Self {
            kind: ScalarKind::U64,
            payload_lo: v,
            payload_hi: 0,
        }
    }

    /// Create an F32 scalar.
    #[inline]
    pub fn f32(v: f32) -> Self {
        Self {
            kind: ScalarKind::F32,
            payload_lo: f64::from(v).to_bits(),
            payload_hi: 0,
        }
    }

    /// Extract as i64 (only valid if kind is an integer type).
    /// Returns None for U64 values > i64::MAX (use `as_u64()` instead).
    #[inline]
    pub fn as_i64(&self) -> Option<i64> {
        if self.kind == ScalarKind::U64 {
            i64::try_from(self.payload_lo).ok()
        } else if self.kind.is_integer() {
            Some(self.payload_lo as i64)
        } else {
            Option::None
        }
    }

    /// Extract as u64 (only valid if kind is an unsigned integer type).
    #[inline]
    pub fn as_u64(&self) -> Option<u64> {
        if self.kind.is_unsigned_integer() {
            Some(self.payload_lo)
        } else if self.kind.is_signed_integer() {
            // Signed → u64: return raw bits (caller interprets)
            Some(self.payload_lo)
        } else {
            Option::None
        }
    }

    /// Extract as f64 (only valid if kind is F64 or F32).
    #[inline]
    pub fn as_f64(&self) -> Option<f64> {
        match self.kind {
            ScalarKind::F64 => Some(f64::from_bits(self.payload_lo)),
            ScalarKind::F32 => Some(f64::from_bits(self.payload_lo)),
            _ => Option::None,
        }
    }

    /// Extract as bool (only valid if kind is Bool).
    #[inline]
    pub fn as_bool(&self) -> Option<bool> {
        if self.kind == ScalarKind::Bool {
            Some(self.payload_lo != 0)
        } else {
            Option::None
        }
    }

    /// Interpret this scalar as an f64 regardless of kind (for numeric comparison).
    /// Integer kinds are cast; float kinds use their stored value; non-numeric returns None.
    #[inline]
    pub fn to_f64_lossy(&self) -> Option<f64> {
        match self.kind {
            ScalarKind::F64 | ScalarKind::F32 => Some(f64::from_bits(self.payload_lo)),
            k if k.is_unsigned_integer() => Some(self.payload_lo as f64),
            k if k.is_signed_integer() => Some(self.payload_lo as i64 as f64),
            _ => Option::None,
        }
    }
}

// ============================================================================
// NumericWidth mapping
// ============================================================================

// Note: `From<NumericWidth> for ScalarKind` is implemented in shape-vm
// (where NumericWidth is defined) since shape-value cannot depend on shape-vm.

// ============================================================================
// ValueSlot <- TypedScalar conversion
// ============================================================================

impl ValueSlot {
    /// Create a ValueSlot from a TypedScalar.
    ///
    /// Returns `(slot, is_heap)` — `is_heap` is always false for scalars
    /// since all scalar values fit in 8 bytes without heap allocation.
    #[inline]
    pub fn from_typed_scalar(ts: TypedScalar) -> (Self, bool) {
        match ts.kind {
            ScalarKind::I8 | ScalarKind::I16 | ScalarKind::I32 | ScalarKind::I64 => {
                (ValueSlot::from_int(ts.payload_lo as i64), false)
            }
            ScalarKind::U8 | ScalarKind::U16 | ScalarKind::U32 => {
                (ValueSlot::from_int(ts.payload_lo as i64), false)
            }
            ScalarKind::U64 => (ValueSlot::from_u64(ts.payload_lo), false),
            ScalarKind::I128 | ScalarKind::U128 => {
                (ValueSlot::from_int(ts.payload_lo as i64), false)
            }
            ScalarKind::F64 | ScalarKind::F32 => {
                (ValueSlot::from_number(f64::from_bits(ts.payload_lo)), false)
            }
            ScalarKind::Bool => (ValueSlot::from_bool(ts.payload_lo != 0), false),
            ScalarKind::None | ScalarKind::Unit => (ValueSlot::none(), false),
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_scalar_convenience_constructors() {
        assert_eq!(TypedScalar::i64(42).kind, ScalarKind::I64);
        assert_eq!(TypedScalar::i64(42).payload_lo, 42);
        assert_eq!(TypedScalar::f64(1.5).kind, ScalarKind::F64);
        assert_eq!(TypedScalar::bool(true).payload_lo, 1);
        assert_eq!(TypedScalar::none().kind, ScalarKind::None);
        assert_eq!(TypedScalar::unit().kind, ScalarKind::Unit);
    }

    #[test]
    fn scalar_kind_classification() {
        assert!(ScalarKind::I64.is_integer());
        assert!(ScalarKind::U32.is_integer());
        assert!(!ScalarKind::F64.is_integer());
        assert!(!ScalarKind::Bool.is_integer());

        assert!(ScalarKind::F64.is_float());
        assert!(ScalarKind::F32.is_float());
        assert!(!ScalarKind::I64.is_float());

        assert!(ScalarKind::I64.is_numeric());
        assert!(ScalarKind::F64.is_numeric());
        assert!(!ScalarKind::Bool.is_numeric());
        assert!(!ScalarKind::None.is_numeric());
    }

    #[test]
    fn value_slot_from_typed_scalar() {
        // Integer
        let (slot, is_heap) = ValueSlot::from_typed_scalar(TypedScalar::i64(-42));
        assert!(!is_heap);
        assert_eq!(slot.as_i64(), -42);

        // Float
        let (slot, is_heap) = ValueSlot::from_typed_scalar(TypedScalar::f64(3.14));
        assert!(!is_heap);
        assert_eq!(slot.as_f64(), 3.14);

        // Bool
        let (slot, is_heap) = ValueSlot::from_typed_scalar(TypedScalar::bool(true));
        assert!(!is_heap);
        assert!(slot.as_bool());

        // None
        let (slot, is_heap) = ValueSlot::from_typed_scalar(TypedScalar::none());
        assert!(!is_heap);
        assert_eq!(slot.raw(), 0);
    }

    #[test]
    fn to_f64_lossy_works() {
        assert_eq!(TypedScalar::f64(3.14).to_f64_lossy(), Some(3.14));
        assert_eq!(TypedScalar::i64(42).to_f64_lossy(), Some(42.0));
        assert_eq!(TypedScalar::bool(true).to_f64_lossy(), Option::None);
        assert_eq!(TypedScalar::none().to_f64_lossy(), Option::None);
    }

    #[test]
    fn typed_scalar_extraction_methods() {
        assert_eq!(TypedScalar::i64(42).as_i64(), Some(42));
        assert_eq!(TypedScalar::f64(3.14).as_i64(), Option::None);
        assert_eq!(TypedScalar::f64(3.14).as_f64(), Some(3.14));
        assert_eq!(TypedScalar::i64(42).as_f64(), Option::None);
        assert_eq!(TypedScalar::bool(true).as_bool(), Some(true));
        assert_eq!(TypedScalar::i64(1).as_bool(), Option::None);
    }
}
