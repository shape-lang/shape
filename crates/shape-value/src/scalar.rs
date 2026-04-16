//! TypedScalar: type-preserving scalar boundary contract for VM↔JIT exchange.
//!
//! When scalar values cross the VM/JIT boundary, their type identity (int vs float)
//! must be preserved. `TypedScalar` carries an explicit `ScalarKind` discriminator
//! alongside the raw payload bits, eliminating the ambiguity between NaN-boxed I48
//! integers and plain f64 numbers.

use crate::slot::ValueSlot;
use crate::tags::{is_tagged, get_tag, TAG_INT, TAG_BOOL, TAG_NONE, TAG_UNIT, TAG_FUNCTION, TAG_MODULE_FN, TAG_HEAP, TAG_REF};
use crate::value_word::{ValueWord, ValueWordExt};

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
// ValueWord <-> TypedScalar conversions
// ============================================================================

/// Extension: scalar conversion methods for ValueWord.
pub trait ValueWordScalarExt {
    fn to_typed_scalar(&self) -> Option<TypedScalar>;
    fn from_typed_scalar(ts: TypedScalar) -> ValueWord;
}
impl ValueWordScalarExt for u64 {
    /// Convert this ValueWord to a TypedScalar, preserving type identity.
    ///
    /// Returns `None` for heap-allocated values (strings, arrays, objects, etc.)
    /// which are not representable as scalars.
    #[inline]
    fn to_typed_scalar(&self) -> Option<TypedScalar> {
        let bits = self.raw_bits();
        if !is_tagged(bits) {
            return Some(TypedScalar {
                kind: ScalarKind::F64,
                payload_lo: bits,
                payload_hi: 0,
            });
        }
        match get_tag(bits) {
            TAG_INT => {
                let i = unsafe { self.as_i64_unchecked() };
                Some(TypedScalar::i64(i))
            }
            TAG_BOOL => {
                let b = unsafe { self.as_bool_unchecked() };
                Some(TypedScalar::bool(b))
            }
            TAG_NONE => Some(TypedScalar::none()),
            TAG_UNIT => Some(TypedScalar::unit()),
            TAG_HEAP | TAG_FUNCTION | TAG_MODULE_FN | TAG_REF => Option::None,
            _ => Option::None,
        }
    }

    /// Create a ValueWord from a TypedScalar.
    ///
    /// Integer kinds are stored as I48 (clamped to the 48-bit range; values
    /// outside [-2^47, 2^47-1] are heap-boxed as BigInt via `from_i64`).
    /// Float kinds are stored as plain f64. Bool/None/Unit use their direct
    /// ValueWord constructors.
    #[inline]
    fn from_typed_scalar(ts: TypedScalar) -> ValueWord {
        match ts.kind {
            ScalarKind::I64 => ValueWord::from_i64(ts.payload_lo as i64),
            ScalarKind::I8 | ScalarKind::I16 | ScalarKind::I32 => {
                // Sign-extend to i64, then use from_i64
                ValueWord::from_i64(ts.payload_lo as i64)
            }
            ScalarKind::U8 | ScalarKind::U16 | ScalarKind::U32 => {
                // Unsigned sub-64: always fits in i64
                ValueWord::from_i64(ts.payload_lo as i64)
            }
            ScalarKind::U64 => {
                if ts.payload_lo <= i64::MAX as u64 {
                    ValueWord::from_i64(ts.payload_lo as i64)
                } else {
                    ValueWord::from_native_u64(ts.payload_lo)
                }
            }
            ScalarKind::I128 | ScalarKind::U128 => {
                // Truncate to i64 (best effort for 128-bit)
                ValueWord::from_i64(ts.payload_lo as i64)
            }
            ScalarKind::F64 => ValueWord::from_f64(f64::from_bits(ts.payload_lo)),
            ScalarKind::F32 => ValueWord::from_f64(f64::from_bits(ts.payload_lo)),
            ScalarKind::Bool => ValueWord::from_bool(ts.payload_lo != 0),
            ScalarKind::None => ValueWord::none(),
            ScalarKind::Unit => ValueWord::unit(),
        }
    }
}

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
    use crate::tags::{I48_MAX, I48_MIN};

    #[test]
    fn round_trip_i64() {
        let vw = ValueWord::from_i64(42);
        let ts = vw.to_typed_scalar().unwrap();
        assert_eq!(ts.kind, ScalarKind::I64);
        assert_eq!(ts.payload_lo, 42u64);
        assert_eq!(ts.payload_hi, 0);
        let vw2 = ValueWord::from_typed_scalar(ts);
        assert_eq!(vw.raw_bits(), vw2.raw_bits());
    }

    #[test]
    fn round_trip_negative_i64() {
        let vw = ValueWord::from_i64(-99);
        let ts = vw.to_typed_scalar().unwrap();
        assert_eq!(ts.kind, ScalarKind::I64);
        assert_eq!(ts.payload_lo as i64, -99);
        let vw2 = ValueWord::from_typed_scalar(ts);
        assert_eq!(vw.raw_bits(), vw2.raw_bits());
    }

    #[test]
    fn round_trip_i48_max() {
        let vw = ValueWord::from_i64(I48_MAX);
        let ts = vw.to_typed_scalar().unwrap();
        assert_eq!(ts.kind, ScalarKind::I64);
        assert_eq!(ts.payload_lo as i64, I48_MAX);
        let vw2 = ValueWord::from_typed_scalar(ts);
        assert_eq!(vw.raw_bits(), vw2.raw_bits());
    }

    #[test]
    fn round_trip_i48_min() {
        let vw = ValueWord::from_i64(I48_MIN);
        let ts = vw.to_typed_scalar().unwrap();
        assert_eq!(ts.kind, ScalarKind::I64);
        assert_eq!(ts.payload_lo as i64, I48_MIN);
        let vw2 = ValueWord::from_typed_scalar(ts);
        assert_eq!(vw.raw_bits(), vw2.raw_bits());
    }

    #[test]
    fn round_trip_f64() {
        let vw = ValueWord::from_f64(3.14);
        let ts = vw.to_typed_scalar().unwrap();
        assert_eq!(ts.kind, ScalarKind::F64);
        assert_eq!(f64::from_bits(ts.payload_lo), 3.14);
        let vw2 = ValueWord::from_typed_scalar(ts);
        assert_eq!(vw.raw_bits(), vw2.raw_bits());
    }

    #[test]
    fn round_trip_f64_nan() {
        let vw = ValueWord::from_f64(f64::NAN);
        let ts = vw.to_typed_scalar().unwrap();
        assert_eq!(ts.kind, ScalarKind::F64);
        assert!(f64::from_bits(ts.payload_lo).is_nan());
        let vw2 = ValueWord::from_typed_scalar(ts);
        // Both should be canonical NaN
        assert_eq!(vw.raw_bits(), vw2.raw_bits());
    }

    #[test]
    fn round_trip_f64_infinity() {
        let vw = ValueWord::from_f64(f64::INFINITY);
        let ts = vw.to_typed_scalar().unwrap();
        assert_eq!(ts.kind, ScalarKind::F64);
        assert_eq!(f64::from_bits(ts.payload_lo), f64::INFINITY);
        let vw2 = ValueWord::from_typed_scalar(ts);
        assert_eq!(vw.raw_bits(), vw2.raw_bits());
    }

    #[test]
    fn round_trip_f64_neg_zero() {
        let vw = ValueWord::from_f64(-0.0);
        let ts = vw.to_typed_scalar().unwrap();
        assert_eq!(ts.kind, ScalarKind::F64);
        // -0.0 has specific bit pattern (sign bit set)
        assert_eq!(ts.payload_lo, (-0.0f64).to_bits());
        let vw2 = ValueWord::from_typed_scalar(ts);
        assert_eq!(vw.raw_bits(), vw2.raw_bits());
    }

    #[test]
    fn round_trip_bool_true() {
        let vw = ValueWord::from_bool(true);
        let ts = vw.to_typed_scalar().unwrap();
        assert_eq!(ts.kind, ScalarKind::Bool);
        assert_eq!(ts.payload_lo, 1);
        let vw2 = ValueWord::from_typed_scalar(ts);
        assert_eq!(vw.raw_bits(), vw2.raw_bits());
    }

    #[test]
    fn round_trip_bool_false() {
        let vw = ValueWord::from_bool(false);
        let ts = vw.to_typed_scalar().unwrap();
        assert_eq!(ts.kind, ScalarKind::Bool);
        assert_eq!(ts.payload_lo, 0);
        let vw2 = ValueWord::from_typed_scalar(ts);
        assert_eq!(vw.raw_bits(), vw2.raw_bits());
    }

    #[test]
    fn round_trip_none() {
        let vw = ValueWord::none();
        let ts = vw.to_typed_scalar().unwrap();
        assert_eq!(ts.kind, ScalarKind::None);
        let vw2 = ValueWord::from_typed_scalar(ts);
        assert_eq!(vw.raw_bits(), vw2.raw_bits());
    }

    #[test]
    fn round_trip_unit() {
        let vw = ValueWord::unit();
        let ts = vw.to_typed_scalar().unwrap();
        assert_eq!(ts.kind, ScalarKind::Unit);
        let vw2 = ValueWord::from_typed_scalar(ts);
        assert_eq!(vw.raw_bits(), vw2.raw_bits());
    }

    #[test]
    fn heap_value_returns_none() {
        let vw = ValueWord::from_string(std::sync::Arc::new("hello".to_string()));
        assert!(vw.to_typed_scalar().is_none());
    }

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
