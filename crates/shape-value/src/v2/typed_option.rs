//! Typed Option representations for v2 runtime.
//!
//! For heap types: `Option<*const T>` = nullable pointer.
//! `None` = null (0x0), `Some(v)` = non-null pointer. Zero overhead.
//!
//! For primitive types: tagged struct with `has_value` discriminant.

/// Option for primitive (non-pointer) types.
///
/// ## Memory layout (16 bytes for T=f64/i64, 8 bytes could suffice for smaller T)
///
/// ```text
/// Offset  Size  Field
/// ------  ----  -----
///   0       1   has_value (0 = None, 1 = Some)
///   1       7   _pad (align to 8)
///   8    sz(T)  value
/// ```
#[repr(C)]
pub struct PrimitiveOption<T: Copy> {
    /// 0 = None, 1 = Some.
    pub has_value: u8,
    /// Padding to align value to 8 bytes.
    pub _pad: [u8; 7],
    /// The value (only valid when has_value == 1).
    pub value: T,
}

impl<T: Copy> PrimitiveOption<T> {
    /// Create a `Some` variant.
    #[inline]
    pub fn some(value: T) -> Self {
        Self {
            has_value: 1,
            _pad: [0; 7],
            value,
        }
    }

    /// Create a `None` variant.
    ///
    /// # Safety
    /// The `value` field is left zeroed and must not be read.
    #[inline]
    pub fn none() -> Self
    where
        T: Default,
    {
        Self {
            has_value: 0,
            _pad: [0; 7],
            value: T::default(),
        }
    }

    /// Whether this is `Some`.
    #[inline]
    pub fn is_some(&self) -> bool {
        self.has_value != 0
    }

    /// Whether this is `None`.
    #[inline]
    pub fn is_none(&self) -> bool {
        self.has_value == 0
    }

    /// Get the value, if present.
    #[inline]
    pub fn get(&self) -> Option<T> {
        if self.is_some() {
            Some(self.value)
        } else {
            None
        }
    }
}

/// Option for heap pointer types — just a nullable pointer.
/// `None` = null (0x0), `Some` = non-null pointer. Zero overhead.
///
/// This is a type alias to make intent clear in signatures.
/// The actual representation is just `*const T`.
pub type HeapOption<T> = *const T;

/// Check if a heap option is None (null pointer).
#[inline]
pub fn heap_option_is_none<T>(ptr: *const T) -> bool {
    ptr.is_null()
}

/// Check if a heap option is Some (non-null pointer).
#[inline]
pub fn heap_option_is_some<T>(ptr: *const T) -> bool {
    !ptr.is_null()
}

/// Byte offset constants for JIT codegen.
pub const PRIMITIVE_OPTION_OFFSET_HAS_VALUE: usize = 0;
pub const PRIMITIVE_OPTION_OFFSET_VALUE: usize = 8;

#[cfg(test)]
mod tests {
    use super::*;
use crate::value_word::ValueWordExt;

    #[test]
    fn test_primitive_option_f64_some() {
        let opt = PrimitiveOption::some(3.14f64);
        assert!(opt.is_some());
        assert!(!opt.is_none());
        assert_eq!(opt.get(), Some(3.14f64));
    }

    #[test]
    fn test_primitive_option_f64_none() {
        let opt = PrimitiveOption::<f64>::none();
        assert!(opt.is_none());
        assert!(!opt.is_some());
        assert_eq!(opt.get(), None);
    }

    #[test]
    fn test_primitive_option_i64_some() {
        let opt = PrimitiveOption::some(42i64);
        assert!(opt.is_some());
        assert_eq!(opt.get(), Some(42i64));
    }

    #[test]
    fn test_primitive_option_i64_none() {
        let opt = PrimitiveOption::<i64>::none();
        assert!(opt.is_none());
        assert_eq!(opt.get(), None);
    }

    #[test]
    fn test_primitive_option_i32_some() {
        let opt = PrimitiveOption::some(99i32);
        assert!(opt.is_some());
        assert_eq!(opt.get(), Some(99i32));
    }

    #[test]
    fn test_primitive_option_i32_none() {
        let opt = PrimitiveOption::<i32>::none();
        assert!(opt.is_none());
        assert_eq!(opt.get(), None);
    }

    #[test]
    fn test_primitive_option_bool_some() {
        let opt = PrimitiveOption::some(true);
        assert!(opt.is_some());
        assert_eq!(opt.get(), Some(true));

        let opt_false = PrimitiveOption::some(false);
        assert!(opt_false.is_some());
        assert_eq!(opt_false.get(), Some(false));
    }

    #[test]
    fn test_primitive_option_bool_none() {
        let opt = PrimitiveOption::<bool>::none();
        assert!(opt.is_none());
        assert_eq!(opt.get(), None);
    }

    #[test]
    fn test_size_of_primitive_option_f64() {
        assert_eq!(std::mem::size_of::<PrimitiveOption<f64>>(), 16);
    }

    #[test]
    fn test_size_of_primitive_option_i64() {
        assert_eq!(std::mem::size_of::<PrimitiveOption<i64>>(), 16);
    }

    #[test]
    fn test_size_of_primitive_option_i32() {
        // 8 (tag+pad) + 4 (i32) = 12, but repr(C) pads to alignment of largest field
        // Largest field alignment is 4 (i32), so 12 is valid (no extra padding needed)
        assert_eq!(std::mem::size_of::<PrimitiveOption<i32>>(), 12);
    }

    #[test]
    fn test_size_of_primitive_option_bool() {
        // 8 (tag+pad) + 1 (bool) = 9, padded to alignment of largest = 1, so 9
        assert_eq!(std::mem::size_of::<PrimitiveOption<bool>>(), 9);
    }

    #[test]
    fn test_field_offsets_f64() {
        let opt = PrimitiveOption::some(1.0f64);
        let base = &opt as *const _ as usize;
        let has_value_offset = &opt.has_value as *const _ as usize - base;
        let value_offset = &opt.value as *const _ as usize - base;

        assert_eq!(has_value_offset, PRIMITIVE_OPTION_OFFSET_HAS_VALUE);
        assert_eq!(value_offset, PRIMITIVE_OPTION_OFFSET_VALUE);
    }

    #[test]
    fn test_heap_option_none() {
        let ptr: HeapOption<u8> = std::ptr::null();
        assert!(heap_option_is_none(ptr));
        assert!(!heap_option_is_some(ptr));
    }

    #[test]
    fn test_heap_option_some() {
        let val: u8 = 42;
        let ptr: HeapOption<u8> = &val as *const u8;
        assert!(heap_option_is_some(ptr));
        assert!(!heap_option_is_none(ptr));
    }

    #[test]
    fn test_heap_option_is_pointer_sized() {
        // HeapOption<T> is just *const T, should be 8 bytes
        assert_eq!(std::mem::size_of::<HeapOption<u8>>(), 8);
        assert_eq!(std::mem::size_of::<HeapOption<f64>>(), 8);
    }
}
