#![allow(unsafe_op_in_unsafe_fn, dead_code, unused_unsafe)]
//! Handler functions for typed array operations.
//!
//! These handlers operate on `TypedArrayHeader` from `shape_value::typed_array_header`
//! using direct native memory access -- no NaN-boxing for the array data itself.
//!
//! ## Stack encoding conventions
//!
//! The VM stack uses `[u64]` slots (`ValueWord` is a type alias for `u64`).
//! For native values:
//! - **f64**: stored as `f64::to_bits()` in a u64 slot, loaded with `f64::from_bits()`
//! - **i64**: stored directly in a u64 slot (reinterpret cast)
//! - **i32**: stored zero-extended in a u64 slot
//! - **pointers**: stored as `usize` cast to `u64` in a slot
//!
//! Array pointers on the stack are raw `*mut TypedArrayHeader` values stored as u64.
//! This means they bypass NaN-boxing entirely -- the dispatch table must ensure these
//! handlers are only called when the compiler has proven the types at compile time.

use shape_value::typed_array_header::{
    ElemType, TypedArrayHeader, typed_array_alloc, typed_array_free, typed_array_get_bool,
    typed_array_get_f64, typed_array_get_i32, typed_array_get_i64, typed_array_len,
    typed_array_push_bool, typed_array_push_f64, typed_array_push_i32, typed_array_push_i64,
    typed_array_set_bool, typed_array_set_f64, typed_array_set_i32, typed_array_set_i64,
};
use shape_value::{VMError, ValueWord, ValueWordExt};

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

/// Create a `ValueWord` from raw u64 bits. Since `ValueWord = u64`, this is
/// an identity conversion. Kept as a named function for documentation.
#[inline(always)]
fn value_word_from_raw_u64(bits: u64) -> ValueWord {
    bits
}

/// Extract the raw u64 bit pattern from a ValueWord. Since `ValueWord = u64`,
/// this is an identity conversion.
#[inline(always)]
fn value_word_to_raw_u64(vw: &ValueWord) -> u64 {
    *vw
}

// ---------------------------------------------------------------------------
// Alloc / Free
// ---------------------------------------------------------------------------

/// Handle TypedArrayAllocF64: push a new f64 typed array pointer onto the stack.
///
/// Stack: [...] -> [..., array_ptr:u64]
/// Operand: capacity (u32), encoded in the instruction operand.
pub fn op_typed_array_alloc_f64(vm: &mut VirtualMachine, capacity: u32) -> Result<(), VMError> {
    let ptr = typed_array_alloc(ElemType::F64 as u8, capacity);
    let bits = ptr_to_u64(ptr);
    vm.push_raw_u64(bits)
}

/// Handle TypedArrayAllocI64: push a new i64 typed array pointer onto the stack.
///
/// Stack: [...] -> [..., array_ptr:u64]
pub fn op_typed_array_alloc_i64(vm: &mut VirtualMachine, capacity: u32) -> Result<(), VMError> {
    let ptr = typed_array_alloc(ElemType::I64 as u8, capacity);
    let bits = ptr_to_u64(ptr);
    vm.push_raw_u64(bits)
}

/// Handle TypedArrayAllocI32: push a new i32 typed array pointer onto the stack.
///
/// Stack: [...] -> [..., array_ptr:u64]
pub fn op_typed_array_alloc_i32(vm: &mut VirtualMachine, capacity: u32) -> Result<(), VMError> {
    let ptr = typed_array_alloc(ElemType::I32 as u8, capacity);
    let bits = ptr_to_u64(ptr);
    vm.push_raw_u64(bits)
}

/// Handle TypedArrayAllocBool: push a new bool typed array pointer onto the stack.
///
/// Stack: [...] -> [..., array_ptr:u64]
pub fn op_typed_array_alloc_bool(vm: &mut VirtualMachine, capacity: u32) -> Result<(), VMError> {
    let ptr = typed_array_alloc(ElemType::Bool as u8, capacity);
    let bits = ptr_to_u64(ptr);
    vm.push_raw_u64(bits)
}

/// Handle TypedArrayFree: pop array pointer and deallocate.
///
/// Stack: [..., array_ptr:u64] -> [...]
pub fn op_typed_array_free(vm: &mut VirtualMachine) -> Result<(), VMError> {
    let bits = vm.pop_raw_u64()?;
    let ptr = u64_to_ptr(bits);
    typed_array_free(ptr);
    Ok(())
}

// ---------------------------------------------------------------------------
// Get (element access)
// ---------------------------------------------------------------------------

/// Handle TypedArrayGetF64: pop index (i64), pop array_ptr (u64), push f64.
///
/// Stack: [..., array_ptr:u64, index:i64] -> [..., value:f64]
pub fn op_typed_array_get_f64(vm: &mut VirtualMachine) -> Result<(), VMError> {
    let index = vm.pop_raw_i64()?;
    let arr_bits = vm.pop_raw_u64()?;
    let arr_ptr = u64_to_ptr(arr_bits);
    let len = typed_array_len(arr_ptr);
    if index < 0 || index as u32 >= len {
        return Err(VMError::RuntimeError(format!(
            "TypedArrayGetF64: index {} out of bounds (len={})",
            index, len
        )));
    }
    let val = typed_array_get_f64(arr_ptr, index as u32);
    // Prevent Drop on the raw-bits ValueWords
    vm.push_raw_f64(val)
}

/// Handle TypedArrayGetI64: pop index (i64), pop array_ptr (u64), push i64.
///
/// Stack: [..., array_ptr:u64, index:i64] -> [..., value:i64]
pub fn op_typed_array_get_i64(vm: &mut VirtualMachine) -> Result<(), VMError> {
    let index = vm.pop_raw_i64()?;
    let arr_bits = vm.pop_raw_u64()?;
    let arr_ptr = u64_to_ptr(arr_bits);
    let len = typed_array_len(arr_ptr);
    if index < 0 || index as u32 >= len {
        return Err(VMError::RuntimeError(format!(
            "TypedArrayGetI64: index {} out of bounds (len={})",
            index, len
        )));
    }
    let val = typed_array_get_i64(arr_ptr, index as u32);
    vm.push_raw_i64(val)
}

/// Handle TypedArrayGetI32: pop index (i64), pop array_ptr (u64), push i32 (as i64).
///
/// Stack: [..., array_ptr:u64, index:i64] -> [..., value:i64]
pub fn op_typed_array_get_i32(vm: &mut VirtualMachine) -> Result<(), VMError> {
    let index = vm.pop_raw_i64()?;
    let arr_bits = vm.pop_raw_u64()?;
    let arr_ptr = u64_to_ptr(arr_bits);
    let len = typed_array_len(arr_ptr);
    if index < 0 || index as u32 >= len {
        return Err(VMError::RuntimeError(format!(
            "TypedArrayGetI32: index {} out of bounds (len={})",
            index, len
        )));
    }
    let val = typed_array_get_i32(arr_ptr, index as u32);
    // i32 is widened to i64 on the stack
    vm.push_raw_i64(val as i64)
}

/// Handle TypedArrayGetBool: pop index (i64), pop array_ptr (u64), push bool.
///
/// Stack: [..., array_ptr:u64, index:i64] -> [..., value:u64(0|1)]
pub fn op_typed_array_get_bool(vm: &mut VirtualMachine) -> Result<(), VMError> {
    let index = vm.pop_raw_i64()?;
    let arr_bits = vm.pop_raw_u64()?;
    let arr_ptr = u64_to_ptr(arr_bits);
    let len = typed_array_len(arr_ptr);
    if index < 0 || index as u32 >= len {
        return Err(VMError::RuntimeError(format!(
            "TypedArrayGetBool: index {} out of bounds (len={})",
            index, len
        )));
    }
    let val = typed_array_get_bool(arr_ptr, index as u32);
    vm.push_raw_bool(val)
}

// ---------------------------------------------------------------------------
// Set (element mutation)
// ---------------------------------------------------------------------------

/// Handle TypedArraySetF64: pop value (f64), pop index (i64), pop array_ptr (u64).
///
/// Stack: [..., array_ptr:u64, index:i64, value:f64] -> [...]
pub fn op_typed_array_set_f64(vm: &mut VirtualMachine) -> Result<(), VMError> {
    let val = vm.pop_raw_f64()?;
    let index = vm.pop_raw_i64()?;
    let arr_bits = vm.pop_raw_u64()?;
    let arr_ptr = u64_to_ptr(arr_bits);
    let len = typed_array_len(arr_ptr);
    if index < 0 || index as u32 >= len {
        return Err(VMError::RuntimeError(format!(
            "TypedArraySetF64: index {} out of bounds (len={})",
            index, len
        )));
    }
    typed_array_set_f64(arr_ptr, index as u32, val);
    Ok(())
}

/// Handle TypedArraySetI64: pop value (i64), pop index (i64), pop array_ptr (u64).
///
/// Stack: [..., array_ptr:u64, index:i64, value:i64] -> [...]
pub fn op_typed_array_set_i64(vm: &mut VirtualMachine) -> Result<(), VMError> {
    let val = vm.pop_raw_i64()?;
    let index = vm.pop_raw_i64()?;
    let arr_bits = vm.pop_raw_u64()?;
    let arr_ptr = u64_to_ptr(arr_bits);
    let len = typed_array_len(arr_ptr);
    if index < 0 || index as u32 >= len {
        return Err(VMError::RuntimeError(format!(
            "TypedArraySetI64: index {} out of bounds (len={})",
            index, len
        )));
    }
    typed_array_set_i64(arr_ptr, index as u32, val);
    Ok(())
}

/// Handle TypedArraySetI32: pop value (i64->i32), pop index (i64), pop array_ptr (u64).
///
/// Stack: [..., array_ptr:u64, index:i64, value:i64] -> [...]
pub fn op_typed_array_set_i32(vm: &mut VirtualMachine) -> Result<(), VMError> {
    let val = vm.pop_raw_i64()? as i32;
    let index = vm.pop_raw_i64()?;
    let arr_bits = vm.pop_raw_u64()?;
    let arr_ptr = u64_to_ptr(arr_bits);
    let len = typed_array_len(arr_ptr);
    if index < 0 || index as u32 >= len {
        return Err(VMError::RuntimeError(format!(
            "TypedArraySetI32: index {} out of bounds (len={})",
            index, len
        )));
    }
    typed_array_set_i32(arr_ptr, index as u32, val);
    Ok(())
}

/// Handle TypedArraySetBool: pop value (u64->bool), pop index (i64), pop array_ptr (u64).
///
/// Stack: [..., array_ptr:u64, index:i64, value:u64(0|1)] -> [...]
pub fn op_typed_array_set_bool(vm: &mut VirtualMachine) -> Result<(), VMError> {
    let val = vm.pop_raw_u64()? != 0;
    let index = vm.pop_raw_i64()?;
    let arr_bits = vm.pop_raw_u64()?;
    let arr_ptr = u64_to_ptr(arr_bits);
    let len = typed_array_len(arr_ptr);
    if index < 0 || index as u32 >= len {
        return Err(VMError::RuntimeError(format!(
            "TypedArraySetBool: index {} out of bounds (len={})",
            index, len
        )));
    }
    typed_array_set_bool(arr_ptr, index as u32, val);
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
    let val = vm.pop_raw_f64()?;
    let arr_bits = vm.pop_raw_u64()?;
    let arr_ptr = u64_to_ptr(arr_bits);
    let new_ptr = typed_array_push_f64(arr_ptr, val);
    let new_bits = ptr_to_u64(new_ptr);
    vm.push_raw_u64(new_bits)
}

/// Handle TypedArrayPushI64: pop value (i64), pop array_ptr (u64), push updated array_ptr.
///
/// Stack: [..., array_ptr:u64, value:i64] -> [..., array_ptr:u64]
pub fn op_typed_array_push_i64(vm: &mut VirtualMachine) -> Result<(), VMError> {
    let val = vm.pop_raw_i64()?;
    let arr_bits = vm.pop_raw_u64()?;
    let arr_ptr = u64_to_ptr(arr_bits);
    let new_ptr = typed_array_push_i64(arr_ptr, val);
    let new_bits = ptr_to_u64(new_ptr);
    vm.push_raw_u64(new_bits)
}

/// Handle TypedArrayPushI32: pop value (i64->i32), pop array_ptr (u64), push updated array_ptr.
///
/// Stack: [..., array_ptr:u64, value:i64] -> [..., array_ptr:u64]
pub fn op_typed_array_push_i32(vm: &mut VirtualMachine) -> Result<(), VMError> {
    let val = vm.pop_raw_i64()? as i32;
    let arr_bits = vm.pop_raw_u64()?;
    let arr_ptr = u64_to_ptr(arr_bits);
    let new_ptr = typed_array_push_i32(arr_ptr, val);
    let new_bits = ptr_to_u64(new_ptr);
    vm.push_raw_u64(new_bits)
}

/// Handle TypedArrayPushBool: pop value (u64->bool), pop array_ptr (u64), push updated array_ptr.
///
/// Stack: [..., array_ptr:u64, value:u64(0|1)] -> [..., array_ptr:u64]
pub fn op_typed_array_push_bool(vm: &mut VirtualMachine) -> Result<(), VMError> {
    let val = vm.pop_raw_u64()? != 0;
    let arr_bits = vm.pop_raw_u64()?;
    let arr_ptr = u64_to_ptr(arr_bits);
    let new_ptr = typed_array_push_bool(arr_ptr, val);
    let new_bits = ptr_to_u64(new_ptr);
    vm.push_raw_u64(new_bits)
}

// ---------------------------------------------------------------------------
// Len
// ---------------------------------------------------------------------------

/// Handle TypedArrayLen: pop array_ptr (u64), push len (i64).
///
/// Stack: [..., array_ptr:u64] -> [..., len:i64]
pub fn op_typed_array_len(vm: &mut VirtualMachine) -> Result<(), VMError> {
    let arr_bits = vm.pop_raw_u64()?;
    let arr_ptr = u64_to_ptr(arr_bits);
    let len = typed_array_len(arr_ptr);
    vm.push_raw_i64(len as i64)
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

    // All push/pop in these tests use the raw stack ops (push_raw_u64/i64/f64,
    // pop_raw_u64/i64/f64) because the typed handlers operate on raw native
    // values, NOT NaN-boxed ValueWords. f64 values happen to be compatible
    // between both encodings (non-NaN doubles are stored as-is), but i64
    // values are NOT (NaN-boxed i48 uses tag bits, raw i64 is reinterpret).

    // ---- Alloc + Len ----

    #[test]
    fn test_alloc_f64_and_len() {
        let mut vm = make_test_vm();
        op_typed_array_alloc_f64(&mut vm, 8).unwrap();
        let arr_bits = vm.pop_raw_u64().unwrap();
        let ptr = u64_to_ptr(arr_bits);
        assert_eq!(typed_array_len(ptr), 0);
        typed_array_free(ptr);
    }

    #[test]
    fn test_alloc_i64_and_len() {
        let mut vm = make_test_vm();
        op_typed_array_alloc_i64(&mut vm, 4).unwrap();
        op_typed_array_len(&mut vm).unwrap();
        assert_eq!(vm.pop_raw_i64().unwrap(), 0);
    }

    // ---- Push + Get (f64) ----

    #[test]
    fn test_push_and_get_f64() {
        let mut vm = make_test_vm();

        // Allocate
        op_typed_array_alloc_f64(&mut vm, 4).unwrap();

        // Push 3 values: each push pops (arr, val) and pushes updated arr
        vm.push_raw_f64(1.5).unwrap();
        op_typed_array_push_f64(&mut vm).unwrap();

        vm.push_raw_f64(2.7).unwrap();
        op_typed_array_push_f64(&mut vm).unwrap();

        vm.push_raw_f64(3.14).unwrap();
        op_typed_array_push_f64(&mut vm).unwrap();

        // Stash the array pointer
        let arr_bits = vm.pop_raw_u64().unwrap();

        // Check length
        vm.push_raw_u64(arr_bits).unwrap();
        op_typed_array_len(&mut vm).unwrap();
        assert_eq!(vm.pop_raw_i64().unwrap(), 3);

        // Get element 0
        vm.push_raw_u64(arr_bits).unwrap();
        vm.push_raw_i64(0).unwrap();
        op_typed_array_get_f64(&mut vm).unwrap();
        assert_eq!(vm.pop_raw_f64().unwrap(), 1.5);

        // Get element 1
        vm.push_raw_u64(arr_bits).unwrap();
        vm.push_raw_i64(1).unwrap();
        op_typed_array_get_f64(&mut vm).unwrap();
        assert_eq!(vm.pop_raw_f64().unwrap(), 2.7);

        // Get element 2
        vm.push_raw_u64(arr_bits).unwrap();
        vm.push_raw_i64(2).unwrap();
        op_typed_array_get_f64(&mut vm).unwrap();
        assert_eq!(vm.pop_raw_f64().unwrap(), 3.14);

        // Out of bounds
        vm.push_raw_u64(arr_bits).unwrap();
        vm.push_raw_i64(3).unwrap();
        assert!(op_typed_array_get_f64(&mut vm).is_err());

        // Negative index
        vm.push_raw_u64(arr_bits).unwrap();
        vm.push_raw_i64(-1).unwrap();
        assert!(op_typed_array_get_f64(&mut vm).is_err());

        // Free
        typed_array_free(u64_to_ptr(arr_bits));
    }

    // ---- Push + Get (i64) ----

    #[test]
    fn test_push_and_get_i64() {
        let mut vm = make_test_vm();

        op_typed_array_alloc_i64(&mut vm, 4).unwrap();

        vm.push_raw_i64(100).unwrap();
        op_typed_array_push_i64(&mut vm).unwrap();

        vm.push_raw_i64(-200).unwrap();
        op_typed_array_push_i64(&mut vm).unwrap();

        vm.push_raw_i64(i64::MAX).unwrap();
        op_typed_array_push_i64(&mut vm).unwrap();

        let arr_bits = vm.pop_raw_u64().unwrap();

        // Len
        vm.push_raw_u64(arr_bits).unwrap();
        op_typed_array_len(&mut vm).unwrap();
        assert_eq!(vm.pop_raw_i64().unwrap(), 3);

        // Get elements
        vm.push_raw_u64(arr_bits).unwrap();
        vm.push_raw_i64(0).unwrap();
        op_typed_array_get_i64(&mut vm).unwrap();
        assert_eq!(vm.pop_raw_i64().unwrap(), 100);

        vm.push_raw_u64(arr_bits).unwrap();
        vm.push_raw_i64(1).unwrap();
        op_typed_array_get_i64(&mut vm).unwrap();
        assert_eq!(vm.pop_raw_i64().unwrap(), -200);

        vm.push_raw_u64(arr_bits).unwrap();
        vm.push_raw_i64(2).unwrap();
        op_typed_array_get_i64(&mut vm).unwrap();
        assert_eq!(vm.pop_raw_i64().unwrap(), i64::MAX);

        typed_array_free(u64_to_ptr(arr_bits));
    }

    // ---- Push + Get (i32) ----

    #[test]
    fn test_push_and_get_i32() {
        let mut vm = make_test_vm();

        op_typed_array_alloc_i32(&mut vm, 4).unwrap();

        vm.push_raw_i64(42).unwrap();
        op_typed_array_push_i32(&mut vm).unwrap();

        vm.push_raw_i64(-99).unwrap();
        op_typed_array_push_i32(&mut vm).unwrap();

        let arr_bits = vm.pop_raw_u64().unwrap();

        // Len
        vm.push_raw_u64(arr_bits).unwrap();
        op_typed_array_len(&mut vm).unwrap();
        assert_eq!(vm.pop_raw_i64().unwrap(), 2);

        // Get elements (i32 widened to i64)
        vm.push_raw_u64(arr_bits).unwrap();
        vm.push_raw_i64(0).unwrap();
        op_typed_array_get_i32(&mut vm).unwrap();
        assert_eq!(vm.pop_raw_i64().unwrap(), 42);

        vm.push_raw_u64(arr_bits).unwrap();
        vm.push_raw_i64(1).unwrap();
        op_typed_array_get_i32(&mut vm).unwrap();
        assert_eq!(vm.pop_raw_i64().unwrap(), -99);

        typed_array_free(u64_to_ptr(arr_bits));
    }

    // ---- Set ----

    #[test]
    fn test_set_f64() {
        let mut vm = make_test_vm();

        op_typed_array_alloc_f64(&mut vm, 4).unwrap();

        // Push two zeros
        vm.push_raw_f64(0.0).unwrap();
        op_typed_array_push_f64(&mut vm).unwrap();
        vm.push_raw_f64(0.0).unwrap();
        op_typed_array_push_f64(&mut vm).unwrap();

        let arr_bits = vm.pop_raw_u64().unwrap();

        // Set index 0 to 42.0
        vm.push_raw_u64(arr_bits).unwrap();
        vm.push_raw_i64(0).unwrap();
        vm.push_raw_f64(42.0).unwrap();
        op_typed_array_set_f64(&mut vm).unwrap();

        // Set index 1 to -1.5
        vm.push_raw_u64(arr_bits).unwrap();
        vm.push_raw_i64(1).unwrap();
        vm.push_raw_f64(-1.5).unwrap();
        op_typed_array_set_f64(&mut vm).unwrap();

        // Verify
        vm.push_raw_u64(arr_bits).unwrap();
        vm.push_raw_i64(0).unwrap();
        op_typed_array_get_f64(&mut vm).unwrap();
        assert_eq!(vm.pop_raw_f64().unwrap(), 42.0);

        vm.push_raw_u64(arr_bits).unwrap();
        vm.push_raw_i64(1).unwrap();
        op_typed_array_get_f64(&mut vm).unwrap();
        assert_eq!(vm.pop_raw_f64().unwrap(), -1.5);

        typed_array_free(u64_to_ptr(arr_bits));
    }

    #[test]
    fn test_set_i64() {
        let mut vm = make_test_vm();

        op_typed_array_alloc_i64(&mut vm, 4).unwrap();

        vm.push_raw_i64(0).unwrap();
        op_typed_array_push_i64(&mut vm).unwrap();
        vm.push_raw_i64(0).unwrap();
        op_typed_array_push_i64(&mut vm).unwrap();

        let arr_bits = vm.pop_raw_u64().unwrap();

        // Set index 0
        vm.push_raw_u64(arr_bits).unwrap();
        vm.push_raw_i64(0).unwrap();
        vm.push_raw_i64(999).unwrap();
        op_typed_array_set_i64(&mut vm).unwrap();

        // Set index 1
        vm.push_raw_u64(arr_bits).unwrap();
        vm.push_raw_i64(1).unwrap();
        vm.push_raw_i64(-888).unwrap();
        op_typed_array_set_i64(&mut vm).unwrap();

        // Verify
        vm.push_raw_u64(arr_bits).unwrap();
        vm.push_raw_i64(0).unwrap();
        op_typed_array_get_i64(&mut vm).unwrap();
        assert_eq!(vm.pop_raw_i64().unwrap(), 999);

        vm.push_raw_u64(arr_bits).unwrap();
        vm.push_raw_i64(1).unwrap();
        op_typed_array_get_i64(&mut vm).unwrap();
        assert_eq!(vm.pop_raw_i64().unwrap(), -888);

        typed_array_free(u64_to_ptr(arr_bits));
    }

    #[test]
    fn test_set_i32() {
        let mut vm = make_test_vm();

        op_typed_array_alloc_i32(&mut vm, 4).unwrap();

        vm.push_raw_i64(0).unwrap();
        op_typed_array_push_i32(&mut vm).unwrap();

        let arr_bits = vm.pop_raw_u64().unwrap();

        // Set index 0 to 777
        vm.push_raw_u64(arr_bits).unwrap();
        vm.push_raw_i64(0).unwrap();
        vm.push_raw_i64(777).unwrap();
        op_typed_array_set_i32(&mut vm).unwrap();

        // Verify
        vm.push_raw_u64(arr_bits).unwrap();
        vm.push_raw_i64(0).unwrap();
        op_typed_array_get_i32(&mut vm).unwrap();
        assert_eq!(vm.pop_raw_i64().unwrap(), 777);

        typed_array_free(u64_to_ptr(arr_bits));
    }

    // ---- Free via handler ----

    #[test]
    fn test_free_handler() {
        let mut vm = make_test_vm();
        op_typed_array_alloc_f64(&mut vm, 4).unwrap();
        // Free via handler (pops the pointer and deallocates)
        op_typed_array_free(&mut vm).unwrap();
        // Stack should be empty
        assert!(vm.pop_raw_u64().is_err());
    }

    // ---- Growth under push ----

    #[test]
    fn test_push_triggers_growth() {
        let mut vm = make_test_vm();

        // Allocate with capacity 2
        op_typed_array_alloc_f64(&mut vm, 2).unwrap();

        // Push 5 elements (forces at least one realloc)
        for i in 0..5 {
            vm.push_raw_f64(i as f64 * 1.1).unwrap();
            op_typed_array_push_f64(&mut vm).unwrap();
        }

        // Check length
        let arr_bits = vm.pop_raw_u64().unwrap();

        vm.push_raw_u64(arr_bits).unwrap();
        op_typed_array_len(&mut vm).unwrap();
        assert_eq!(vm.pop_raw_i64().unwrap(), 5);

        // Verify all values survived realloc
        for i in 0..5 {
            vm.push_raw_u64(arr_bits).unwrap();
            vm.push_raw_i64(i).unwrap();
            op_typed_array_get_f64(&mut vm).unwrap();
            let val = vm.pop_raw_f64().unwrap();
            let expected = i as f64 * 1.1;
            assert!(
                (val - expected).abs() < 1e-12,
                "mismatch at index {}: got {}, expected {}",
                i,
                val,
                expected
            );
        }

        typed_array_free(u64_to_ptr(arr_bits));
    }

    // ---- Set out-of-bounds ----

    #[test]
    fn test_set_out_of_bounds() {
        let mut vm = make_test_vm();

        op_typed_array_alloc_f64(&mut vm, 4).unwrap();

        vm.push_raw_f64(1.0).unwrap();
        op_typed_array_push_f64(&mut vm).unwrap();

        let arr_bits = vm.pop_raw_u64().unwrap();

        // Try to set index 1 (len is 1, so out of bounds)
        vm.push_raw_u64(arr_bits).unwrap();
        vm.push_raw_i64(1).unwrap();
        vm.push_raw_f64(99.0).unwrap();
        assert!(op_typed_array_set_f64(&mut vm).is_err());

        typed_array_free(u64_to_ptr(arr_bits));
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
            vm.push_raw_f64(v).unwrap();
            op_typed_array_push_f64(&mut vm).unwrap();
        }

        let arr_bits = vm.pop_raw_u64().unwrap();

        // Check len == 4
        vm.push_raw_u64(arr_bits).unwrap();
        op_typed_array_len(&mut vm).unwrap();
        assert_eq!(vm.pop_raw_i64().unwrap(), 4);

        // Modify index 2: 3.0 -> 30.0
        vm.push_raw_u64(arr_bits).unwrap();
        vm.push_raw_i64(2).unwrap();
        vm.push_raw_f64(30.0).unwrap();
        op_typed_array_set_f64(&mut vm).unwrap();

        // Read back all values
        let expected = [1.0, 2.0, 30.0, 4.0];
        for (i, &exp) in expected.iter().enumerate() {
            vm.push_raw_u64(arr_bits).unwrap();
            vm.push_raw_i64(i as i64).unwrap();
            op_typed_array_get_f64(&mut vm).unwrap();
            let got = vm.pop_raw_f64().unwrap();
            assert_eq!(got, exp, "mismatch at index {}", i);
        }

        typed_array_free(u64_to_ptr(arr_bits));
    }

    #[test]
    fn test_full_lifecycle_i64() {
        let mut vm = make_test_vm();

        op_typed_array_alloc_i64(&mut vm, 8).unwrap();

        let values: [i64; 4] = [10, 20, 30, 40];
        for &v in &values {
            vm.push_raw_i64(v).unwrap();
            op_typed_array_push_i64(&mut vm).unwrap();
        }

        let arr_bits = vm.pop_raw_u64().unwrap();

        // Set index 1: 20 -> -200
        vm.push_raw_u64(arr_bits).unwrap();
        vm.push_raw_i64(1).unwrap();
        vm.push_raw_i64(-200).unwrap();
        op_typed_array_set_i64(&mut vm).unwrap();

        // Verify
        let expected: [i64; 4] = [10, -200, 30, 40];
        for (i, &exp) in expected.iter().enumerate() {
            vm.push_raw_u64(arr_bits).unwrap();
            vm.push_raw_i64(i as i64).unwrap();
            op_typed_array_get_i64(&mut vm).unwrap();
            assert_eq!(vm.pop_raw_i64().unwrap(), exp, "mismatch at i={}", i);
        }

        typed_array_free(u64_to_ptr(arr_bits));
    }
}
