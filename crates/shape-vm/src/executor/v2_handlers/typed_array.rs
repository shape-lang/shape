#![allow(unsafe_op_in_unsafe_fn)]
//! v2 handler functions for typed array operations.
//!
//! These handlers operate on `TypedArrayHeader` from `shape_value::v2_typed_array`
//! using direct native memory access -- no NaN-boxing for the array data itself.
//!
//! ## Stack encoding conventions
//!
//! The VM stack uses `[u64]` slots (`ValueWord` is `repr(transparent)` over `u64`).
//! For v2 native values:
//! - **f64**: stored as `f64::to_bits()` in a u64 slot, loaded with `f64::from_bits()`
//! - **i64**: stored directly in a u64 slot (reinterpret cast)
//! - **i32**: stored zero-extended in a u64 slot
//! - **pointers**: stored as `usize` cast to `u64` in a slot
//!
//! Array pointers on the stack are raw `*mut TypedArrayHeader` values stored as u64.
//! This means they bypass NaN-boxing entirely -- the dispatch table must ensure these
//! handlers are only called when the compiler has proven the types at compile time.

use shape_value::v2_typed_array::{
    ElemTag, TypedArrayHeader, typed_array_alloc, typed_array_free, typed_array_get_f64,
    typed_array_get_i32, typed_array_get_i64, typed_array_len, typed_array_push_f64,
    typed_array_push_i32, typed_array_push_i64, typed_array_set_f64, typed_array_set_i32,
    typed_array_set_i64,
};
use shape_value::{VMError, ValueWord};

use crate::executor::VirtualMachine;

// ---------------------------------------------------------------------------
// Helpers: raw u64 <-> pointer / native value conversion
// ---------------------------------------------------------------------------

/// Encode a raw pointer as a u64 for storage on the VM stack.
#[inline(always)]
fn ptr_to_u64(ptr: *mut TypedArrayHeader) -> u64 {
    ptr as usize as u64
}

/// Decode a u64 from the VM stack back to a raw pointer.
#[inline(always)]
fn u64_to_ptr(bits: u64) -> *mut TypedArrayHeader {
    bits as usize as *mut TypedArrayHeader
}

/// Create a `ValueWord` that holds a raw u64 bit pattern.
///
/// # Safety
/// The resulting ValueWord must only be consumed by v2 handlers that understand
/// the raw encoding. It is NOT a valid NaN-boxed value for v1 dispatch.
#[inline(always)]
unsafe fn value_word_from_raw_u64(bits: u64) -> ValueWord {
    // Safety: ValueWord is repr(transparent) over u64.
    std::mem::transmute::<u64, ValueWord>(bits)
}

/// Extract the raw u64 bit pattern from a ValueWord.
///
/// # Safety
/// The ValueWord must contain a raw u64 value placed by a v2 handler, not a
/// normal NaN-boxed value (unless the caller knows how to interpret it).
#[inline(always)]
unsafe fn value_word_to_raw_u64(vw: &ValueWord) -> u64 {
    // Safety: ValueWord is repr(transparent) over u64.
    std::mem::transmute_copy::<ValueWord, u64>(vw)
}

// ---------------------------------------------------------------------------
// Alloc / Free
// ---------------------------------------------------------------------------

/// Handle TypedArrayAllocF64: push a new f64 typed array pointer onto the stack.
///
/// Stack: [...] -> [..., array_ptr:u64]
/// Operand: capacity (u32), encoded in the instruction operand.
pub fn op_typed_array_alloc_f64(vm: &mut VirtualMachine, capacity: u32) -> Result<(), VMError> {
    let ptr = typed_array_alloc(ElemTag::F64, capacity);
    let bits = ptr_to_u64(ptr);
    unsafe { vm.push_vw(value_word_from_raw_u64(bits)) }
}

/// Handle TypedArrayAllocI64: push a new i64 typed array pointer onto the stack.
///
/// Stack: [...] -> [..., array_ptr:u64]
pub fn op_typed_array_alloc_i64(vm: &mut VirtualMachine, capacity: u32) -> Result<(), VMError> {
    let ptr = typed_array_alloc(ElemTag::I64, capacity);
    let bits = ptr_to_u64(ptr);
    unsafe { vm.push_vw(value_word_from_raw_u64(bits)) }
}

/// Handle TypedArrayAllocI32: push a new i32 typed array pointer onto the stack.
///
/// Stack: [...] -> [..., array_ptr:u64]
pub fn op_typed_array_alloc_i32(vm: &mut VirtualMachine, capacity: u32) -> Result<(), VMError> {
    let ptr = typed_array_alloc(ElemTag::I32, capacity);
    let bits = ptr_to_u64(ptr);
    unsafe { vm.push_vw(value_word_from_raw_u64(bits)) }
}

/// Handle TypedArrayFree: pop array pointer and deallocate.
///
/// Stack: [..., array_ptr:u64] -> [...]
pub fn op_typed_array_free(vm: &mut VirtualMachine) -> Result<(), VMError> {
    let vw = vm.pop_vw()?;
    let bits = unsafe { value_word_to_raw_u64(&vw) };
    let ptr = u64_to_ptr(bits);
    unsafe { typed_array_free(ptr) };
    // Prevent ValueWord from running Drop on whatever bits we stored
    std::mem::forget(vw);
    Ok(())
}

// ---------------------------------------------------------------------------
// Get (element access)
// ---------------------------------------------------------------------------

/// Handle TypedArrayGetF64: pop index (i64), pop array_ptr (u64), push f64.
///
/// Stack: [..., array_ptr:u64, index:i64] -> [..., value:f64]
pub fn op_typed_array_get_f64(vm: &mut VirtualMachine) -> Result<(), VMError> {
    let index_vw = vm.pop_vw()?;
    let arr_vw = vm.pop_vw()?;
    let index = index_vw.as_i64().ok_or_else(|| VMError::RuntimeError(
        "TypedArrayGetF64: index is not an integer".into(),
    ))?;
    let arr_bits = unsafe { value_word_to_raw_u64(&arr_vw) };
    let arr_ptr = u64_to_ptr(arr_bits);
    let len = unsafe { typed_array_len(arr_ptr) };
    if index < 0 || index as u32 >= len {
        return Err(VMError::RuntimeError(format!(
            "TypedArrayGetF64: index {} out of bounds (len={})",
            index, len
        )));
    }
    let val = unsafe { typed_array_get_f64(arr_ptr, index as u32) };
    // Prevent Drop on the raw-bits ValueWords
    std::mem::forget(arr_vw);
    vm.push_vw(ValueWord::from_f64(val))
}

/// Handle TypedArrayGetI64: pop index (i64), pop array_ptr (u64), push i64.
///
/// Stack: [..., array_ptr:u64, index:i64] -> [..., value:i64]
pub fn op_typed_array_get_i64(vm: &mut VirtualMachine) -> Result<(), VMError> {
    let index_vw = vm.pop_vw()?;
    let arr_vw = vm.pop_vw()?;
    let index = index_vw.as_i64().ok_or_else(|| VMError::RuntimeError(
        "TypedArrayGetI64: index is not an integer".into(),
    ))?;
    let arr_bits = unsafe { value_word_to_raw_u64(&arr_vw) };
    let arr_ptr = u64_to_ptr(arr_bits);
    let len = unsafe { typed_array_len(arr_ptr) };
    if index < 0 || index as u32 >= len {
        return Err(VMError::RuntimeError(format!(
            "TypedArrayGetI64: index {} out of bounds (len={})",
            index, len
        )));
    }
    let val = unsafe { typed_array_get_i64(arr_ptr, index as u32) };
    std::mem::forget(arr_vw);
    vm.push_vw(ValueWord::from_i64(val))
}

/// Handle TypedArrayGetI32: pop index (i64), pop array_ptr (u64), push i32 (as i64).
///
/// Stack: [..., array_ptr:u64, index:i64] -> [..., value:i64]
pub fn op_typed_array_get_i32(vm: &mut VirtualMachine) -> Result<(), VMError> {
    let index_vw = vm.pop_vw()?;
    let arr_vw = vm.pop_vw()?;
    let index = index_vw.as_i64().ok_or_else(|| VMError::RuntimeError(
        "TypedArrayGetI32: index is not an integer".into(),
    ))?;
    let arr_bits = unsafe { value_word_to_raw_u64(&arr_vw) };
    let arr_ptr = u64_to_ptr(arr_bits);
    let len = unsafe { typed_array_len(arr_ptr) };
    if index < 0 || index as u32 >= len {
        return Err(VMError::RuntimeError(format!(
            "TypedArrayGetI32: index {} out of bounds (len={})",
            index, len
        )));
    }
    let val = unsafe { typed_array_get_i32(arr_ptr, index as u32) };
    std::mem::forget(arr_vw);
    // i32 is widened to i64 on the stack
    vm.push_vw(ValueWord::from_i64(val as i64))
}

// ---------------------------------------------------------------------------
// Set (element mutation)
// ---------------------------------------------------------------------------

/// Handle TypedArraySetF64: pop value (f64), pop index (i64), pop array_ptr (u64).
///
/// Stack: [..., array_ptr:u64, index:i64, value:f64] -> [...]
pub fn op_typed_array_set_f64(vm: &mut VirtualMachine) -> Result<(), VMError> {
    let val_vw = vm.pop_vw()?;
    let index_vw = vm.pop_vw()?;
    let arr_vw = vm.pop_vw()?;
    let val = val_vw.as_f64().ok_or_else(|| VMError::RuntimeError(
        "TypedArraySetF64: value is not a number".into(),
    ))?;
    let index = index_vw.as_i64().ok_or_else(|| VMError::RuntimeError(
        "TypedArraySetF64: index is not an integer".into(),
    ))?;
    let arr_bits = unsafe { value_word_to_raw_u64(&arr_vw) };
    let arr_ptr = u64_to_ptr(arr_bits);
    let len = unsafe { typed_array_len(arr_ptr) };
    if index < 0 || index as u32 >= len {
        return Err(VMError::RuntimeError(format!(
            "TypedArraySetF64: index {} out of bounds (len={})",
            index, len
        )));
    }
    unsafe { typed_array_set_f64(arr_ptr, index as u32, val) };
    std::mem::forget(arr_vw);
    Ok(())
}

/// Handle TypedArraySetI64: pop value (i64), pop index (i64), pop array_ptr (u64).
///
/// Stack: [..., array_ptr:u64, index:i64, value:i64] -> [...]
pub fn op_typed_array_set_i64(vm: &mut VirtualMachine) -> Result<(), VMError> {
    let val_vw = vm.pop_vw()?;
    let index_vw = vm.pop_vw()?;
    let arr_vw = vm.pop_vw()?;
    let val = val_vw.as_i64().ok_or_else(|| VMError::RuntimeError(
        "TypedArraySetI64: value is not an integer".into(),
    ))?;
    let index = index_vw.as_i64().ok_or_else(|| VMError::RuntimeError(
        "TypedArraySetI64: index is not an integer".into(),
    ))?;
    let arr_bits = unsafe { value_word_to_raw_u64(&arr_vw) };
    let arr_ptr = u64_to_ptr(arr_bits);
    let len = unsafe { typed_array_len(arr_ptr) };
    if index < 0 || index as u32 >= len {
        return Err(VMError::RuntimeError(format!(
            "TypedArraySetI64: index {} out of bounds (len={})",
            index, len
        )));
    }
    unsafe { typed_array_set_i64(arr_ptr, index as u32, val) };
    std::mem::forget(arr_vw);
    Ok(())
}

/// Handle TypedArraySetI32: pop value (i64->i32), pop index (i64), pop array_ptr (u64).
///
/// Stack: [..., array_ptr:u64, index:i64, value:i64] -> [...]
pub fn op_typed_array_set_i32(vm: &mut VirtualMachine) -> Result<(), VMError> {
    let val_vw = vm.pop_vw()?;
    let index_vw = vm.pop_vw()?;
    let arr_vw = vm.pop_vw()?;
    let val = val_vw.as_i64().ok_or_else(|| VMError::RuntimeError(
        "TypedArraySetI32: value is not an integer".into(),
    ))?;
    let index = index_vw.as_i64().ok_or_else(|| VMError::RuntimeError(
        "TypedArraySetI32: index is not an integer".into(),
    ))?;
    let arr_bits = unsafe { value_word_to_raw_u64(&arr_vw) };
    let arr_ptr = u64_to_ptr(arr_bits);
    let len = unsafe { typed_array_len(arr_ptr) };
    if index < 0 || index as u32 >= len {
        return Err(VMError::RuntimeError(format!(
            "TypedArraySetI32: index {} out of bounds (len={})",
            index, len
        )));
    }
    unsafe { typed_array_set_i32(arr_ptr, index as u32, val as i32) };
    std::mem::forget(arr_vw);
    Ok(())
}

// ---------------------------------------------------------------------------
// Push (append element)
// ---------------------------------------------------------------------------

/// Handle TypedArrayPushF64: pop value (f64), pop array_ptr (u64), push updated array_ptr.
///
/// The array pointer may change due to reallocation, so the new pointer is pushed back.
///
/// Stack: [..., array_ptr:u64, value:f64] -> [..., array_ptr:u64]
pub fn op_typed_array_push_f64(vm: &mut VirtualMachine) -> Result<(), VMError> {
    let val_vw = vm.pop_vw()?;
    let arr_vw = vm.pop_vw()?;
    let val = val_vw.as_f64().ok_or_else(|| VMError::RuntimeError(
        "TypedArrayPushF64: value is not a number".into(),
    ))?;
    let arr_bits = unsafe { value_word_to_raw_u64(&arr_vw) };
    let mut arr_ptr = u64_to_ptr(arr_bits);
    unsafe { typed_array_push_f64(&mut arr_ptr, val) };
    std::mem::forget(arr_vw);
    let new_bits = ptr_to_u64(arr_ptr);
    unsafe { vm.push_vw(value_word_from_raw_u64(new_bits)) }
}

/// Handle TypedArrayPushI64: pop value (i64), pop array_ptr (u64), push updated array_ptr.
///
/// Stack: [..., array_ptr:u64, value:i64] -> [..., array_ptr:u64]
pub fn op_typed_array_push_i64(vm: &mut VirtualMachine) -> Result<(), VMError> {
    let val_vw = vm.pop_vw()?;
    let arr_vw = vm.pop_vw()?;
    let val = val_vw.as_i64().ok_or_else(|| VMError::RuntimeError(
        "TypedArrayPushI64: value is not an integer".into(),
    ))?;
    let arr_bits = unsafe { value_word_to_raw_u64(&arr_vw) };
    let mut arr_ptr = u64_to_ptr(arr_bits);
    unsafe { typed_array_push_i64(&mut arr_ptr, val) };
    std::mem::forget(arr_vw);
    let new_bits = ptr_to_u64(arr_ptr);
    unsafe { vm.push_vw(value_word_from_raw_u64(new_bits)) }
}

/// Handle TypedArrayPushI32: pop value (i64->i32), pop array_ptr (u64), push updated array_ptr.
///
/// Stack: [..., array_ptr:u64, value:i64] -> [..., array_ptr:u64]
pub fn op_typed_array_push_i32(vm: &mut VirtualMachine) -> Result<(), VMError> {
    let val_vw = vm.pop_vw()?;
    let arr_vw = vm.pop_vw()?;
    let val = val_vw.as_i64().ok_or_else(|| VMError::RuntimeError(
        "TypedArrayPushI32: value is not an integer".into(),
    ))?;
    let arr_bits = unsafe { value_word_to_raw_u64(&arr_vw) };
    let mut arr_ptr = u64_to_ptr(arr_bits);
    unsafe { typed_array_push_i32(&mut arr_ptr, val as i32) };
    std::mem::forget(arr_vw);
    let new_bits = ptr_to_u64(arr_ptr);
    unsafe { vm.push_vw(value_word_from_raw_u64(new_bits)) }
}

// ---------------------------------------------------------------------------
// Len
// ---------------------------------------------------------------------------

/// Handle TypedArrayLen: pop array_ptr (u64), push len (i64).
///
/// Stack: [..., array_ptr:u64] -> [..., len:i64]
pub fn op_typed_array_len(vm: &mut VirtualMachine) -> Result<(), VMError> {
    let arr_vw = vm.pop_vw()?;
    let arr_bits = unsafe { value_word_to_raw_u64(&arr_vw) };
    let arr_ptr = u64_to_ptr(arr_bits);
    let len = unsafe { typed_array_len(arr_ptr) };
    std::mem::forget(arr_vw);
    vm.push_vw(ValueWord::from_i64(len as i64))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::{VMConfig, VirtualMachine};

    /// Create a minimal VM for testing handler functions.
    fn make_test_vm() -> VirtualMachine {
        VirtualMachine::new(VMConfig::default())
    }

    // ---- Alloc + Len ----

    #[test]
    fn test_alloc_f64_and_len() {
        let mut vm = make_test_vm();
        // Allocate an f64 array with capacity 8
        op_typed_array_alloc_f64(&mut vm, 8).unwrap();
        // The array pointer is on the stack; duplicate it for len check
        // Pop it, check len, then free
        let arr_vw = vm.pop_vw().unwrap();
        let arr_bits = unsafe { value_word_to_raw_u64(&arr_vw) };
        let ptr = u64_to_ptr(arr_bits);
        assert_eq!(unsafe { typed_array_len(ptr) }, 0);
        // Clean up
        unsafe { typed_array_free(ptr) };
        std::mem::forget(arr_vw);
    }

    #[test]
    fn test_alloc_i64_and_len() {
        let mut vm = make_test_vm();
        op_typed_array_alloc_i64(&mut vm, 4).unwrap();
        op_typed_array_len(&mut vm).unwrap();
        let len_vw = vm.pop_vw().unwrap();
        assert_eq!(len_vw.as_i64(), Some(0));
    }

    // ---- Push + Get (f64) ----

    #[test]
    fn test_push_and_get_f64() {
        let mut vm = make_test_vm();

        // Allocate
        op_typed_array_alloc_f64(&mut vm, 4).unwrap();

        // Push 3 values: each push pops (arr, val) and pushes updated arr
        vm.push_vw(ValueWord::from_f64(1.5)).unwrap();
        // Stack: [arr, 1.5] -- but push expects [arr, val]
        // We need to swap since alloc leaves arr on stack, then we push val on top
        // Actually the stack is: [..., arr_ptr, 1.5] which is correct for push
        op_typed_array_push_f64(&mut vm).unwrap();
        // Stack: [arr_ptr (possibly new)]

        vm.push_vw(ValueWord::from_f64(2.7)).unwrap();
        op_typed_array_push_f64(&mut vm).unwrap();

        vm.push_vw(ValueWord::from_f64(3.14)).unwrap();
        op_typed_array_push_f64(&mut vm).unwrap();

        // Check length: duplicate the arr pointer first
        // We need the pointer for both len and get, so let's peek
        let arr_vw = vm.pop_vw().unwrap();
        let arr_bits = unsafe { value_word_to_raw_u64(&arr_vw) };
        std::mem::forget(arr_vw);

        // Push arr back for len
        unsafe { vm.push_vw(value_word_from_raw_u64(arr_bits)).unwrap() };
        op_typed_array_len(&mut vm).unwrap();
        let len = vm.pop_vw().unwrap();
        assert_eq!(len.as_i64(), Some(3));

        // Get element 0
        unsafe { vm.push_vw(value_word_from_raw_u64(arr_bits)).unwrap() };
        vm.push_vw(ValueWord::from_i64(0)).unwrap();
        op_typed_array_get_f64(&mut vm).unwrap();
        let v0 = vm.pop_vw().unwrap();
        assert_eq!(v0.as_f64(), Some(1.5));

        // Get element 1
        unsafe { vm.push_vw(value_word_from_raw_u64(arr_bits)).unwrap() };
        vm.push_vw(ValueWord::from_i64(1)).unwrap();
        op_typed_array_get_f64(&mut vm).unwrap();
        let v1 = vm.pop_vw().unwrap();
        assert_eq!(v1.as_f64(), Some(2.7));

        // Get element 2
        unsafe { vm.push_vw(value_word_from_raw_u64(arr_bits)).unwrap() };
        vm.push_vw(ValueWord::from_i64(2)).unwrap();
        op_typed_array_get_f64(&mut vm).unwrap();
        let v2 = vm.pop_vw().unwrap();
        assert_eq!(v2.as_f64(), Some(3.14));

        // Out of bounds
        unsafe { vm.push_vw(value_word_from_raw_u64(arr_bits)).unwrap() };
        vm.push_vw(ValueWord::from_i64(3)).unwrap();
        let err = op_typed_array_get_f64(&mut vm);
        assert!(err.is_err());

        // Negative index
        unsafe { vm.push_vw(value_word_from_raw_u64(arr_bits)).unwrap() };
        vm.push_vw(ValueWord::from_i64(-1)).unwrap();
        let err = op_typed_array_get_f64(&mut vm);
        assert!(err.is_err());

        // Free
        let ptr = u64_to_ptr(arr_bits);
        unsafe { typed_array_free(ptr) };
    }

    // ---- Push + Get (i64) ----

    #[test]
    fn test_push_and_get_i64() {
        let mut vm = make_test_vm();

        op_typed_array_alloc_i64(&mut vm, 4).unwrap();

        vm.push_vw(ValueWord::from_i64(100)).unwrap();
        op_typed_array_push_i64(&mut vm).unwrap();

        vm.push_vw(ValueWord::from_i64(-200)).unwrap();
        op_typed_array_push_i64(&mut vm).unwrap();

        vm.push_vw(ValueWord::from_i64(i64::MAX)).unwrap();
        op_typed_array_push_i64(&mut vm).unwrap();

        let arr_vw = vm.pop_vw().unwrap();
        let arr_bits = unsafe { value_word_to_raw_u64(&arr_vw) };
        std::mem::forget(arr_vw);

        // Len
        unsafe { vm.push_vw(value_word_from_raw_u64(arr_bits)).unwrap() };
        op_typed_array_len(&mut vm).unwrap();
        assert_eq!(vm.pop_vw().unwrap().as_i64(), Some(3));

        // Get elements
        unsafe { vm.push_vw(value_word_from_raw_u64(arr_bits)).unwrap() };
        vm.push_vw(ValueWord::from_i64(0)).unwrap();
        op_typed_array_get_i64(&mut vm).unwrap();
        assert_eq!(vm.pop_vw().unwrap().as_i64(), Some(100));

        unsafe { vm.push_vw(value_word_from_raw_u64(arr_bits)).unwrap() };
        vm.push_vw(ValueWord::from_i64(1)).unwrap();
        op_typed_array_get_i64(&mut vm).unwrap();
        assert_eq!(vm.pop_vw().unwrap().as_i64(), Some(-200));

        // Note: i64::MAX exceeds i48 range, so ValueWord::from_i64 will heap-box it.
        // The push handler extracts via as_i64() which works for heap-boxed BigInt too.
        unsafe { vm.push_vw(value_word_from_raw_u64(arr_bits)).unwrap() };
        vm.push_vw(ValueWord::from_i64(2)).unwrap();
        op_typed_array_get_i64(&mut vm).unwrap();
        let v2 = vm.pop_vw().unwrap();
        assert_eq!(v2.as_i64(), Some(i64::MAX));

        let ptr = u64_to_ptr(arr_bits);
        unsafe { typed_array_free(ptr) };
    }

    // ---- Push + Get (i32) ----

    #[test]
    fn test_push_and_get_i32() {
        let mut vm = make_test_vm();

        op_typed_array_alloc_i32(&mut vm, 4).unwrap();

        vm.push_vw(ValueWord::from_i64(42)).unwrap();
        op_typed_array_push_i32(&mut vm).unwrap();

        vm.push_vw(ValueWord::from_i64(-99)).unwrap();
        op_typed_array_push_i32(&mut vm).unwrap();

        let arr_vw = vm.pop_vw().unwrap();
        let arr_bits = unsafe { value_word_to_raw_u64(&arr_vw) };
        std::mem::forget(arr_vw);

        // Len
        unsafe { vm.push_vw(value_word_from_raw_u64(arr_bits)).unwrap() };
        op_typed_array_len(&mut vm).unwrap();
        assert_eq!(vm.pop_vw().unwrap().as_i64(), Some(2));

        // Get elements (i32 widened to i64)
        unsafe { vm.push_vw(value_word_from_raw_u64(arr_bits)).unwrap() };
        vm.push_vw(ValueWord::from_i64(0)).unwrap();
        op_typed_array_get_i32(&mut vm).unwrap();
        assert_eq!(vm.pop_vw().unwrap().as_i64(), Some(42));

        unsafe { vm.push_vw(value_word_from_raw_u64(arr_bits)).unwrap() };
        vm.push_vw(ValueWord::from_i64(1)).unwrap();
        op_typed_array_get_i32(&mut vm).unwrap();
        assert_eq!(vm.pop_vw().unwrap().as_i64(), Some(-99));

        let ptr = u64_to_ptr(arr_bits);
        unsafe { typed_array_free(ptr) };
    }

    // ---- Set ----

    #[test]
    fn test_set_f64() {
        let mut vm = make_test_vm();

        op_typed_array_alloc_f64(&mut vm, 4).unwrap();

        // Push two zeros
        vm.push_vw(ValueWord::from_f64(0.0)).unwrap();
        op_typed_array_push_f64(&mut vm).unwrap();
        vm.push_vw(ValueWord::from_f64(0.0)).unwrap();
        op_typed_array_push_f64(&mut vm).unwrap();

        let arr_vw = vm.pop_vw().unwrap();
        let arr_bits = unsafe { value_word_to_raw_u64(&arr_vw) };
        std::mem::forget(arr_vw);

        // Set index 0 to 42.0
        unsafe { vm.push_vw(value_word_from_raw_u64(arr_bits)).unwrap() };
        vm.push_vw(ValueWord::from_i64(0)).unwrap();
        vm.push_vw(ValueWord::from_f64(42.0)).unwrap();
        op_typed_array_set_f64(&mut vm).unwrap();

        // Set index 1 to -1.5
        unsafe { vm.push_vw(value_word_from_raw_u64(arr_bits)).unwrap() };
        vm.push_vw(ValueWord::from_i64(1)).unwrap();
        vm.push_vw(ValueWord::from_f64(-1.5)).unwrap();
        op_typed_array_set_f64(&mut vm).unwrap();

        // Verify
        unsafe { vm.push_vw(value_word_from_raw_u64(arr_bits)).unwrap() };
        vm.push_vw(ValueWord::from_i64(0)).unwrap();
        op_typed_array_get_f64(&mut vm).unwrap();
        assert_eq!(vm.pop_vw().unwrap().as_f64(), Some(42.0));

        unsafe { vm.push_vw(value_word_from_raw_u64(arr_bits)).unwrap() };
        vm.push_vw(ValueWord::from_i64(1)).unwrap();
        op_typed_array_get_f64(&mut vm).unwrap();
        assert_eq!(vm.pop_vw().unwrap().as_f64(), Some(-1.5));

        let ptr = u64_to_ptr(arr_bits);
        unsafe { typed_array_free(ptr) };
    }

    #[test]
    fn test_set_i64() {
        let mut vm = make_test_vm();

        op_typed_array_alloc_i64(&mut vm, 4).unwrap();

        vm.push_vw(ValueWord::from_i64(0)).unwrap();
        op_typed_array_push_i64(&mut vm).unwrap();
        vm.push_vw(ValueWord::from_i64(0)).unwrap();
        op_typed_array_push_i64(&mut vm).unwrap();

        let arr_vw = vm.pop_vw().unwrap();
        let arr_bits = unsafe { value_word_to_raw_u64(&arr_vw) };
        std::mem::forget(arr_vw);

        // Set index 0
        unsafe { vm.push_vw(value_word_from_raw_u64(arr_bits)).unwrap() };
        vm.push_vw(ValueWord::from_i64(0)).unwrap();
        vm.push_vw(ValueWord::from_i64(999)).unwrap();
        op_typed_array_set_i64(&mut vm).unwrap();

        // Set index 1
        unsafe { vm.push_vw(value_word_from_raw_u64(arr_bits)).unwrap() };
        vm.push_vw(ValueWord::from_i64(1)).unwrap();
        vm.push_vw(ValueWord::from_i64(-888)).unwrap();
        op_typed_array_set_i64(&mut vm).unwrap();

        // Verify
        unsafe { vm.push_vw(value_word_from_raw_u64(arr_bits)).unwrap() };
        vm.push_vw(ValueWord::from_i64(0)).unwrap();
        op_typed_array_get_i64(&mut vm).unwrap();
        assert_eq!(vm.pop_vw().unwrap().as_i64(), Some(999));

        unsafe { vm.push_vw(value_word_from_raw_u64(arr_bits)).unwrap() };
        vm.push_vw(ValueWord::from_i64(1)).unwrap();
        op_typed_array_get_i64(&mut vm).unwrap();
        assert_eq!(vm.pop_vw().unwrap().as_i64(), Some(-888));

        let ptr = u64_to_ptr(arr_bits);
        unsafe { typed_array_free(ptr) };
    }

    #[test]
    fn test_set_i32() {
        let mut vm = make_test_vm();

        op_typed_array_alloc_i32(&mut vm, 4).unwrap();

        vm.push_vw(ValueWord::from_i64(0)).unwrap();
        op_typed_array_push_i32(&mut vm).unwrap();

        let arr_vw = vm.pop_vw().unwrap();
        let arr_bits = unsafe { value_word_to_raw_u64(&arr_vw) };
        std::mem::forget(arr_vw);

        // Set index 0 to 777
        unsafe { vm.push_vw(value_word_from_raw_u64(arr_bits)).unwrap() };
        vm.push_vw(ValueWord::from_i64(0)).unwrap();
        vm.push_vw(ValueWord::from_i64(777)).unwrap();
        op_typed_array_set_i32(&mut vm).unwrap();

        // Verify
        unsafe { vm.push_vw(value_word_from_raw_u64(arr_bits)).unwrap() };
        vm.push_vw(ValueWord::from_i64(0)).unwrap();
        op_typed_array_get_i32(&mut vm).unwrap();
        assert_eq!(vm.pop_vw().unwrap().as_i64(), Some(777));

        let ptr = u64_to_ptr(arr_bits);
        unsafe { typed_array_free(ptr) };
    }

    // ---- Free via handler ----

    #[test]
    fn test_free_handler() {
        let mut vm = make_test_vm();
        op_typed_array_alloc_f64(&mut vm, 4).unwrap();
        // Free via handler (pops the pointer and deallocates)
        op_typed_array_free(&mut vm).unwrap();
        // Stack should be empty
        assert!(vm.pop_vw().is_err());
    }

    // ---- Growth under push ----

    #[test]
    fn test_push_triggers_growth() {
        let mut vm = make_test_vm();

        // Allocate with capacity 2
        op_typed_array_alloc_f64(&mut vm, 2).unwrap();

        // Push 5 elements (forces at least one realloc)
        for i in 0..5 {
            vm.push_vw(ValueWord::from_f64(i as f64 * 1.1)).unwrap();
            op_typed_array_push_f64(&mut vm).unwrap();
        }

        // Check length
        let arr_vw = vm.pop_vw().unwrap();
        let arr_bits = unsafe { value_word_to_raw_u64(&arr_vw) };
        std::mem::forget(arr_vw);

        unsafe { vm.push_vw(value_word_from_raw_u64(arr_bits)).unwrap() };
        op_typed_array_len(&mut vm).unwrap();
        assert_eq!(vm.pop_vw().unwrap().as_i64(), Some(5));

        // Verify all values survived realloc
        for i in 0..5 {
            unsafe { vm.push_vw(value_word_from_raw_u64(arr_bits)).unwrap() };
            vm.push_vw(ValueWord::from_i64(i)).unwrap();
            op_typed_array_get_f64(&mut vm).unwrap();
            let val = vm.pop_vw().unwrap().as_f64().unwrap();
            let expected = i as f64 * 1.1;
            assert!(
                (val - expected).abs() < 1e-12,
                "mismatch at index {}: got {}, expected {}",
                i,
                val,
                expected
            );
        }

        let ptr = u64_to_ptr(arr_bits);
        unsafe { typed_array_free(ptr) };
    }

    // ---- Set out-of-bounds ----

    #[test]
    fn test_set_out_of_bounds() {
        let mut vm = make_test_vm();

        op_typed_array_alloc_f64(&mut vm, 4).unwrap();

        vm.push_vw(ValueWord::from_f64(1.0)).unwrap();
        op_typed_array_push_f64(&mut vm).unwrap();

        let arr_vw = vm.pop_vw().unwrap();
        let arr_bits = unsafe { value_word_to_raw_u64(&arr_vw) };
        std::mem::forget(arr_vw);

        // Try to set index 1 (len is 1, so out of bounds)
        unsafe { vm.push_vw(value_word_from_raw_u64(arr_bits)).unwrap() };
        vm.push_vw(ValueWord::from_i64(1)).unwrap();
        vm.push_vw(ValueWord::from_f64(99.0)).unwrap();
        let err = op_typed_array_set_f64(&mut vm);
        assert!(err.is_err());

        let ptr = u64_to_ptr(arr_bits);
        unsafe { typed_array_free(ptr) };
    }

    // ---- Roundtrip: alloc, push, get, set, len, free (integration) ----

    #[test]
    fn test_full_lifecycle_f64() {
        let mut vm = make_test_vm();

        // Allocate
        op_typed_array_alloc_f64(&mut vm, 8).unwrap();

        // Push 4 elements
        let values = [1.0, 2.0, 3.0, 4.0];
        for &v in &values {
            vm.push_vw(ValueWord::from_f64(v)).unwrap();
            op_typed_array_push_f64(&mut vm).unwrap();
        }

        // Stash the pointer
        let arr_vw = vm.pop_vw().unwrap();
        let arr_bits = unsafe { value_word_to_raw_u64(&arr_vw) };
        std::mem::forget(arr_vw);

        // Check len == 4
        unsafe { vm.push_vw(value_word_from_raw_u64(arr_bits)).unwrap() };
        op_typed_array_len(&mut vm).unwrap();
        assert_eq!(vm.pop_vw().unwrap().as_i64(), Some(4));

        // Modify index 2: 3.0 -> 30.0
        unsafe { vm.push_vw(value_word_from_raw_u64(arr_bits)).unwrap() };
        vm.push_vw(ValueWord::from_i64(2)).unwrap();
        vm.push_vw(ValueWord::from_f64(30.0)).unwrap();
        op_typed_array_set_f64(&mut vm).unwrap();

        // Read back all values
        let expected = [1.0, 2.0, 30.0, 4.0];
        for (i, &exp) in expected.iter().enumerate() {
            unsafe { vm.push_vw(value_word_from_raw_u64(arr_bits)).unwrap() };
            vm.push_vw(ValueWord::from_i64(i as i64)).unwrap();
            op_typed_array_get_f64(&mut vm).unwrap();
            let got = vm.pop_vw().unwrap().as_f64().unwrap();
            assert_eq!(got, exp, "mismatch at index {}", i);
        }

        // Free
        let ptr = u64_to_ptr(arr_bits);
        unsafe { typed_array_free(ptr) };
    }

    #[test]
    fn test_full_lifecycle_i64() {
        let mut vm = make_test_vm();

        op_typed_array_alloc_i64(&mut vm, 8).unwrap();

        let values: [i64; 4] = [10, 20, 30, 40];
        for &v in &values {
            vm.push_vw(ValueWord::from_i64(v)).unwrap();
            op_typed_array_push_i64(&mut vm).unwrap();
        }

        let arr_vw = vm.pop_vw().unwrap();
        let arr_bits = unsafe { value_word_to_raw_u64(&arr_vw) };
        std::mem::forget(arr_vw);

        // Set index 1: 20 -> -200
        unsafe { vm.push_vw(value_word_from_raw_u64(arr_bits)).unwrap() };
        vm.push_vw(ValueWord::from_i64(1)).unwrap();
        vm.push_vw(ValueWord::from_i64(-200)).unwrap();
        op_typed_array_set_i64(&mut vm).unwrap();

        // Verify
        let expected: [i64; 4] = [10, -200, 30, 40];
        for (i, &exp) in expected.iter().enumerate() {
            unsafe { vm.push_vw(value_word_from_raw_u64(arr_bits)).unwrap() };
            vm.push_vw(ValueWord::from_i64(i as i64)).unwrap();
            op_typed_array_get_i64(&mut vm).unwrap();
            assert_eq!(vm.pop_vw().unwrap().as_i64(), Some(exp), "mismatch at i={}", i);
        }

        let ptr = u64_to_ptr(arr_bits);
        unsafe { typed_array_free(ptr) };
    }
}
