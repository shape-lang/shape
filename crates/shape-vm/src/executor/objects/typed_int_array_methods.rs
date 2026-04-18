//! Method handlers for v2 `TypedArray<i64>` (native typed int arrays).
//!
//! These handlers extract the receiver as a `V2TypedArrayView` over an
//! `TypedArray<i64>` and delegate to the typed element primitives exposed by
//! `v2_handlers::v2_array_detect` (read/write/push/pop/sum, …).
//!
//! ## Status (V0.c scaffolding)
//!
//! Registered in the [`TYPED_INT_ARRAY_METHODS`] PHF map in
//! [`method_registry`](super::method_registry) but **not wired into the
//! dispatch cascade yet**. The v2 typed-array dispatch path in
//! `objects/mod.rs` currently handles `Vec<int>` receivers via the legacy
//! `HeapKind::IntArray` → `INT_ARRAY_METHODS` cascade (for v1 `Arc<TypedBuffer>`
//! receivers) and via `dispatch_v2_typed_array_method` (for native v2
//! `TypedArray<i64>` receivers).
//!
//! Phase V2.a of the plan at
//! `/home/dev/.claude/plans/i-want-a-complete-foamy-eich.md` wires these
//! handlers into the cascade ahead of the generic `ARRAY_METHODS` lookup so
//! that calls on a native-typed `TypedArray<i64>` dispatch straight here,
//! eliminating the runtime HeapKind match.
//!
//! Until then, the entries in [`TYPED_INT_ARRAY_METHODS`] are unreachable
//! dead code. That is intentional — this file exists so that V2.a becomes a
//! one-file wiring change rather than a mixed scaffolding + wiring commit.

use crate::executor::VirtualMachine;
use crate::executor::v2_handlers::v2_array_detect::{
    self as v2, V2ElemType, V2TypedArrayView,
};
use shape_runtime::context::ExecutionContext;
use shape_value::v2::typed_array::TypedArray;
use shape_value::{VMError, ValueWord, ValueWordExt};
use std::mem::ManuallyDrop;

/// Borrow a `ValueWord` from raw `u64` bits without taking ownership.
/// Mirrors the helper in `typed_array_methods.rs` — the dispatcher already
/// owns the bits, so wrapping them in a `ManuallyDrop` avoids a double-free.
#[inline]
fn borrow_vw(raw: u64) -> ManuallyDrop<ValueWord> {
    ManuallyDrop::new(ValueWord::from_raw_bits(raw))
}

/// Extract the receiver as a v2 `TypedArray<i64>` view. Returns a typed
/// error when the receiver is not a v2 typed array of i64 elements.
#[inline]
fn extract_int_view(args: &mut [u64]) -> Result<V2TypedArrayView, VMError> {
    let vw = borrow_vw(args[0]);
    let view = v2::as_v2_typed_array(&vw).ok_or_else(|| VMError::TypeError {
        expected: "TypedArray<int>",
        got: vw.type_name(),
    })?;
    if view.elem_type != V2ElemType::I64 {
        return Err(VMError::TypeError {
            expected: "TypedArray<int>",
            got: vw.type_name(),
        });
    }
    Ok(view)
}

// ═════════════════════════════════════════════════════════════════════════════
// MethodFnV2 handlers
// ═════════════════════════════════════════════════════════════════════════════

/// `arr.len()` — return the number of elements.
pub fn len(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let view = extract_int_view(args)?;
    Ok(ValueWord::from_i64(view.len as i64).into_raw_bits())
}

/// `arr.push(x)` — append an element, return the new length.
pub fn push(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    if args.len() != 2 {
        return Err(VMError::ArityMismatch {
            function: "push".to_string(),
            expected: 1,
            got: args.len().saturating_sub(1),
        });
    }
    let view = extract_int_view(args)?;
    let val_vw = borrow_vw(args[1]);
    v2::push_element(&view, &val_vw).map_err(|e| VMError::RuntimeError(e.to_string()))?;
    // push_element grew the array: re-read length from the pointer.
    let new_len = unsafe {
        let arr = view.ptr as *const TypedArray<i64>;
        (*arr).len
    };
    Ok(ValueWord::from_i64(new_len as i64).into_raw_bits())
}

/// `arr.pop()` — remove and return the last element, or `none` if empty.
pub fn pop(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let view = extract_int_view(args)?;
    let val = v2::pop_element(&view).unwrap_or_else(ValueWord::none);
    Ok(val.into_raw_bits())
}

/// `arr.sum()` — sum all elements.
pub fn sum(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let view = extract_int_view(args)?;
    let val = v2::sum_elements(&view).ok_or_else(|| {
        VMError::RuntimeError("sum() unsupported on this typed array".into())
    })?;
    Ok(val.into_raw_bits())
}

/// `arr.first()` — first element, or error if empty.
pub fn first(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let view = extract_int_view(args)?;
    if view.len == 0 {
        return Err(VMError::RuntimeError(
            "first() called on empty Vec<int>".into(),
        ));
    }
    let val = v2::read_element(&view, 0).unwrap_or_else(ValueWord::none);
    Ok(val.into_raw_bits())
}

/// `arr.last()` — last element, or error if empty.
pub fn last(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let view = extract_int_view(args)?;
    if view.len == 0 {
        return Err(VMError::RuntimeError(
            "last() called on empty Vec<int>".into(),
        ));
    }
    let val = v2::read_element(&view, view.len - 1).unwrap_or_else(ValueWord::none);
    Ok(val.into_raw_bits())
}

/// `arr.get(i)` — element at index `i`, error if out of bounds.
pub fn get(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    if args.len() != 2 {
        return Err(VMError::ArityMismatch {
            function: "get".to_string(),
            expected: 1,
            got: args.len().saturating_sub(1),
        });
    }
    let view = extract_int_view(args)?;
    let idx_vw = borrow_vw(args[1]);
    let idx = idx_vw.as_i64().ok_or_else(|| VMError::TypeError {
        expected: "int",
        got: idx_vw.type_name(),
    })?;
    if idx < 0 || (idx as u64) >= view.len as u64 {
        return Err(VMError::IndexOutOfBounds {
            index: idx as i32,
            length: view.len as usize,
        });
    }
    let val = v2::read_element(&view, idx as u32).unwrap_or_else(ValueWord::none);
    Ok(val.into_raw_bits())
}

/// `arr.set(i, x)` — set element at index; returns `none`.
pub fn set(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    if args.len() != 3 {
        return Err(VMError::ArityMismatch {
            function: "set".to_string(),
            expected: 2,
            got: args.len().saturating_sub(1),
        });
    }
    let view = extract_int_view(args)?;
    let idx_vw = borrow_vw(args[1]);
    let idx = idx_vw.as_i64().ok_or_else(|| VMError::TypeError {
        expected: "int",
        got: idx_vw.type_name(),
    })?;
    if idx < 0 || (idx as u64) >= view.len as u64 {
        return Err(VMError::IndexOutOfBounds {
            index: idx as i32,
            length: view.len as usize,
        });
    }
    let val_vw = borrow_vw(args[2]);
    v2::write_element(&view, idx as u32, &val_vw)
        .map_err(|e| VMError::RuntimeError(e.to_string()))?;
    Ok(ValueWord::none().into_raw_bits())
}

// ═════════════════════════════════════════════════════════════════════════════
// Tests
// ═════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::{VMConfig, VirtualMachine};
    use crate::executor::v2_handlers::v2_array_detect::{ELEM_TYPE_I64, stamp_elem_type};
    use shape_value::v2::typed_array::TypedArray;

    /// Allocate a v2 TypedArray<i64> from a slice and wrap as a ValueWord
    /// native pointer with the i64 elem-type stamped. Returns the raw u64
    /// bits. Caller must drop_array after tests.
    fn make_int_array(values: &[i64]) -> (*mut TypedArray<i64>, u64) {
        let arr = TypedArray::<i64>::from_slice(values);
        unsafe {
            stamp_elem_type(arr as *mut u8, ELEM_TYPE_I64);
        }
        let bits = ValueWord::from_native_ptr(arr as usize).into_raw_bits();
        (arr, bits)
    }

    fn dummy_vm() -> VirtualMachine {
        VirtualMachine::new(VMConfig::default())
    }

    #[test]
    fn test_typed_int_array_len() {
        let (arr, bits) = make_int_array(&[10, 20, 30, 40]);
        let mut vm = dummy_vm();
        let mut args = [bits];
        let result = len(&mut vm, &mut args, None).expect("len");
        let vw = ValueWord::from_raw_bits(result);
        assert_eq!(vw.as_i64(), Some(4));
        unsafe {
            TypedArray::drop_array(arr);
        }
    }

    #[test]
    fn test_typed_int_array_first_last() {
        let (arr, bits) = make_int_array(&[100, 200, 300]);
        let mut vm = dummy_vm();

        let mut args = [bits];
        let f = first(&mut vm, &mut args, None).expect("first");
        assert_eq!(ValueWord::from_raw_bits(f).as_i64(), Some(100));

        let mut args = [bits];
        let l = last(&mut vm, &mut args, None).expect("last");
        assert_eq!(ValueWord::from_raw_bits(l).as_i64(), Some(300));

        unsafe {
            TypedArray::drop_array(arr);
        }
    }

    #[test]
    fn test_typed_int_array_first_empty_errors() {
        let (arr, bits) = make_int_array(&[]);
        let mut vm = dummy_vm();
        let mut args = [bits];
        let err = first(&mut vm, &mut args, None).unwrap_err();
        match err {
            VMError::RuntimeError(msg) => assert!(msg.contains("empty")),
            other => panic!("unexpected error: {:?}", other),
        }
        unsafe {
            TypedArray::drop_array(arr);
        }
    }

    #[test]
    fn test_typed_int_array_sum() {
        let (arr, bits) = make_int_array(&[1, 2, 3, 4, 5]);
        let mut vm = dummy_vm();
        let mut args = [bits];
        let result = sum(&mut vm, &mut args, None).expect("sum");
        let vw = ValueWord::from_raw_bits(result);
        assert_eq!(vw.as_i64(), Some(15));
        unsafe {
            TypedArray::drop_array(arr);
        }
    }

    #[test]
    fn test_typed_int_array_get_set() {
        let (arr, bits) = make_int_array(&[10, 20, 30]);
        let mut vm = dummy_vm();

        // get(1) -> 20
        let idx_bits = ValueWord::from_i64(1).into_raw_bits();
        let mut args = [bits, idx_bits];
        let got = get(&mut vm, &mut args, None).expect("get");
        assert_eq!(ValueWord::from_raw_bits(got).as_i64(), Some(20));

        // set(1, 99)
        let idx_bits = ValueWord::from_i64(1).into_raw_bits();
        let val_bits = ValueWord::from_i64(99).into_raw_bits();
        let mut args = [bits, idx_bits, val_bits];
        set(&mut vm, &mut args, None).expect("set");

        // get(1) now -> 99
        let idx_bits = ValueWord::from_i64(1).into_raw_bits();
        let mut args = [bits, idx_bits];
        let got = get(&mut vm, &mut args, None).expect("get after set");
        assert_eq!(ValueWord::from_raw_bits(got).as_i64(), Some(99));

        unsafe {
            TypedArray::drop_array(arr);
        }
    }

    #[test]
    fn test_typed_int_array_get_oob() {
        let (arr, bits) = make_int_array(&[1, 2]);
        let mut vm = dummy_vm();
        let idx_bits = ValueWord::from_i64(5).into_raw_bits();
        let mut args = [bits, idx_bits];
        let err = get(&mut vm, &mut args, None).unwrap_err();
        assert!(matches!(err, VMError::IndexOutOfBounds { .. }));
        unsafe {
            TypedArray::drop_array(arr);
        }
    }

    #[test]
    fn test_typed_int_array_push_pop() {
        let (arr, bits) = make_int_array(&[1, 2, 3]);
        let mut vm = dummy_vm();

        // push(4) -> new len 4
        let val_bits = ValueWord::from_i64(4).into_raw_bits();
        let mut args = [bits, val_bits];
        let new_len = push(&mut vm, &mut args, None).expect("push");
        assert_eq!(ValueWord::from_raw_bits(new_len).as_i64(), Some(4));

        // pop -> 4
        let mut args = [bits];
        let popped = pop(&mut vm, &mut args, None).expect("pop");
        assert_eq!(ValueWord::from_raw_bits(popped).as_i64(), Some(4));

        // len -> 3
        let mut args = [bits];
        let l = len(&mut vm, &mut args, None).expect("len");
        assert_eq!(ValueWord::from_raw_bits(l).as_i64(), Some(3));

        unsafe {
            TypedArray::drop_array(arr);
        }
    }

    #[test]
    fn test_typed_int_array_pop_empty_returns_none() {
        let (arr, bits) = make_int_array(&[]);
        let mut vm = dummy_vm();
        let mut args = [bits];
        let popped = pop(&mut vm, &mut args, None).expect("pop empty");
        assert!(ValueWord::from_raw_bits(popped).is_none());
        unsafe {
            TypedArray::drop_array(arr);
        }
    }
}
