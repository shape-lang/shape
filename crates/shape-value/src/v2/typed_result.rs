//! Typed Result representations for v2 runtime.
//!
//! Result<T, E> is a tagged union: tag (0=Ok, 1=Err) + payload.
//! Size = 8 + max(sizeof(T), sizeof(E)). Monomorphized per instantiation.
//!
//! Since Rust `#[repr(C)]` unions with generics are awkward, we provide
//! concrete instantiations for common Shape result types.

use super::string_obj::StringObj;

/// Tag value for Ok variant.
pub const RESULT_TAG_OK: u8 = 0;
/// Tag value for Err variant.
pub const RESULT_TAG_ERR: u8 = 1;

/// Result<f64, *const StringObj> — Ok is f64, Err is string pointer.
///
/// ## Memory layout (16 bytes)
/// ```text
/// Offset  Size  Field
/// ------  ----  -----
///   0       1   tag (0=Ok, 1=Err)
///   1       7   _pad
///   8       8   payload (f64 or *const StringObj)
/// ```
#[repr(C)]
pub struct ResultF64Str {
    pub tag: u8,
    pub _pad: [u8; 7],
    payload: u64, // union: f64 bits or pointer
}

impl ResultF64Str {
    /// Create an Ok(f64) result.
    #[inline]
    pub fn ok(value: f64) -> Self {
        Self {
            tag: RESULT_TAG_OK,
            _pad: [0; 7],
            payload: value.to_bits(),
        }
    }

    /// Create an Err(*const StringObj) result.
    #[inline]
    pub fn err(err_ptr: *const StringObj) -> Self {
        Self {
            tag: RESULT_TAG_ERR,
            _pad: [0; 7],
            payload: err_ptr as u64,
        }
    }

    #[inline]
    pub fn is_ok(&self) -> bool {
        self.tag == RESULT_TAG_OK
    }

    #[inline]
    pub fn is_err(&self) -> bool {
        self.tag == RESULT_TAG_ERR
    }

    /// Get the Ok value. Caller must check `is_ok()` first.
    #[inline]
    pub fn unwrap_ok(&self) -> f64 {
        debug_assert!(self.is_ok());
        f64::from_bits(self.payload)
    }

    /// Get the Err pointer. Caller must check `is_err()` first.
    #[inline]
    pub fn unwrap_err(&self) -> *const StringObj {
        debug_assert!(self.is_err());
        self.payload as *const StringObj
    }
}

/// Result<i64, *const StringObj> — Ok is i64, Err is string pointer.
///
/// ## Memory layout (16 bytes)
/// ```text
/// Offset  Size  Field
/// ------  ----  -----
///   0       1   tag (0=Ok, 1=Err)
///   1       7   _pad
///   8       8   payload (i64 or *const StringObj)
/// ```
#[repr(C)]
pub struct ResultI64Str {
    pub tag: u8,
    pub _pad: [u8; 7],
    payload: u64, // union: i64 bits or pointer
}

impl ResultI64Str {
    /// Create an Ok(i64) result.
    #[inline]
    pub fn ok(value: i64) -> Self {
        Self {
            tag: RESULT_TAG_OK,
            _pad: [0; 7],
            payload: value as u64,
        }
    }

    /// Create an Err(*const StringObj) result.
    #[inline]
    pub fn err(err_ptr: *const StringObj) -> Self {
        Self {
            tag: RESULT_TAG_ERR,
            _pad: [0; 7],
            payload: err_ptr as u64,
        }
    }

    #[inline]
    pub fn is_ok(&self) -> bool {
        self.tag == RESULT_TAG_OK
    }

    #[inline]
    pub fn is_err(&self) -> bool {
        self.tag == RESULT_TAG_ERR
    }

    /// Get the Ok value. Caller must check `is_ok()` first.
    #[inline]
    pub fn unwrap_ok(&self) -> i64 {
        debug_assert!(self.is_ok());
        self.payload as i64
    }

    /// Get the Err pointer. Caller must check `is_err()` first.
    #[inline]
    pub fn unwrap_err(&self) -> *const StringObj {
        debug_assert!(self.is_err());
        self.payload as *const StringObj
    }
}

/// Result<*const T, *const StringObj> — Ok is a heap pointer, Err is string pointer.
///
/// ## Memory layout (16 bytes)
/// ```text
/// Offset  Size  Field
/// ------  ----  -----
///   0       1   tag (0=Ok, 1=Err)
///   1       7   _pad
///   8       8   payload (*const T or *const StringObj)
/// ```
#[repr(C)]
pub struct ResultPtrStr {
    pub tag: u8,
    pub _pad: [u8; 7],
    payload: u64, // union: ok pointer or err pointer
}

impl ResultPtrStr {
    /// Create an Ok(*const u8) result (generic heap pointer).
    #[inline]
    pub fn ok(ptr: *const u8) -> Self {
        Self {
            tag: RESULT_TAG_OK,
            _pad: [0; 7],
            payload: ptr as u64,
        }
    }

    /// Create an Err(*const StringObj) result.
    #[inline]
    pub fn err(err_ptr: *const StringObj) -> Self {
        Self {
            tag: RESULT_TAG_ERR,
            _pad: [0; 7],
            payload: err_ptr as u64,
        }
    }

    #[inline]
    pub fn is_ok(&self) -> bool {
        self.tag == RESULT_TAG_OK
    }

    #[inline]
    pub fn is_err(&self) -> bool {
        self.tag == RESULT_TAG_ERR
    }

    /// Get the Ok pointer. Caller must check `is_ok()` first.
    #[inline]
    pub fn unwrap_ok(&self) -> *const u8 {
        debug_assert!(self.is_ok());
        self.payload as *const u8
    }

    /// Get the Err pointer. Caller must check `is_err()` first.
    #[inline]
    pub fn unwrap_err(&self) -> *const StringObj {
        debug_assert!(self.is_err());
        self.payload as *const StringObj
    }
}

/// Byte offset constants for JIT codegen.
pub const RESULT_OFFSET_TAG: usize = 0;
pub const RESULT_OFFSET_PAYLOAD: usize = 8;

#[cfg(test)]
mod tests {
    use super::*;

    // --- ResultF64Str ---

    #[test]
    fn test_result_f64_ok() {
        let r = ResultF64Str::ok(3.14);
        assert!(r.is_ok());
        assert!(!r.is_err());
        assert!((r.unwrap_ok() - 3.14).abs() < f64::EPSILON);
    }

    #[test]
    fn test_result_f64_err() {
        let err = StringObj::new("something went wrong");
        let r = ResultF64Str::err(err);
        assert!(r.is_err());
        assert!(!r.is_ok());
        assert_eq!(r.unwrap_err(), err as *const StringObj);
        unsafe { StringObj::drop(err) };
    }

    #[test]
    fn test_size_of_result_f64_str() {
        assert_eq!(std::mem::size_of::<ResultF64Str>(), 16);
    }

    // --- ResultI64Str ---

    #[test]
    fn test_result_i64_ok() {
        let r = ResultI64Str::ok(42);
        assert!(r.is_ok());
        assert_eq!(r.unwrap_ok(), 42);
    }

    #[test]
    fn test_result_i64_ok_negative() {
        let r = ResultI64Str::ok(-100);
        assert!(r.is_ok());
        assert_eq!(r.unwrap_ok(), -100);
    }

    #[test]
    fn test_result_i64_err() {
        let err = StringObj::new("i64 error");
        let r = ResultI64Str::err(err);
        assert!(r.is_err());
        assert_eq!(r.unwrap_err(), err as *const StringObj);
        unsafe { StringObj::drop(err) };
    }

    #[test]
    fn test_size_of_result_i64_str() {
        assert_eq!(std::mem::size_of::<ResultI64Str>(), 16);
    }

    // --- ResultPtrStr ---

    #[test]
    fn test_result_ptr_ok() {
        let val: u8 = 99;
        let r = ResultPtrStr::ok(&val as *const u8);
        assert!(r.is_ok());
        assert_eq!(r.unwrap_ok(), &val as *const u8);
    }

    #[test]
    fn test_result_ptr_err() {
        let err = StringObj::new("ptr error");
        let r = ResultPtrStr::err(err);
        assert!(r.is_err());
        assert_eq!(r.unwrap_err(), err as *const StringObj);
        unsafe { StringObj::drop(err) };
    }

    #[test]
    fn test_size_of_result_ptr_str() {
        assert_eq!(std::mem::size_of::<ResultPtrStr>(), 16);
    }

    // --- Offset constants ---

    #[test]
    fn test_result_field_offsets() {
        let r = ResultF64Str::ok(1.0);
        let base = &r as *const _ as usize;
        let tag_offset = &r.tag as *const _ as usize - base;
        let payload_offset = &r.payload as *const _ as usize - base;

        assert_eq!(tag_offset, RESULT_OFFSET_TAG);
        assert_eq!(payload_offset, RESULT_OFFSET_PAYLOAD);
    }

    // --- Tag constants ---

    #[test]
    fn test_tag_constants() {
        assert_eq!(RESULT_TAG_OK, 0);
        assert_eq!(RESULT_TAG_ERR, 1);
        assert_ne!(RESULT_TAG_OK, RESULT_TAG_ERR);
    }
}
