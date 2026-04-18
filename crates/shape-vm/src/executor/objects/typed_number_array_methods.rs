//! Method handlers for v2 `TypedArray<f64>` (native typed number arrays).
//!
//! These handlers extract the receiver as a `V2TypedArrayView` over an
//! `TypedArray<f64>` and delegate to the typed element primitives exposed by
//! `v2_handlers::v2_array_detect` (read/write/push/pop/sum, …).
//!
//! ## Status (V2.a — wired)
//!
//! Registered in the [`TYPED_NUMBER_ARRAY_METHODS`] PHF map in
//! [`method_registry`](super::method_registry) and wired into the dispatch
//! cascade in [`objects`](super): when the receiver is a native v2
//! `TypedArray<f64>`, the PHF is consulted before the bespoke match in
//! `dispatch_v2_typed_array_method` and before the generic `ARRAY_METHODS`
//! lookup. Method names not in the PHF (e.g. higher-order `map/filter/reduce`)
//! fall through to the bespoke path, which in turn falls through to the
//! generic `ARRAY_METHODS` handler via element materialization.
//!
//! The legacy `HeapKind::FloatArray` → `FLOAT_ARRAY_METHODS` cascade still
//! handles v1 `Arc<AlignedTypedBuffer>` receivers on the slow path.

use crate::executor::VirtualMachine;
use crate::executor::v2_handlers::v2_array_detect::{
    self as v2, V2ElemType, V2TypedArrayView,
};
use shape_runtime::context::ExecutionContext;
use shape_value::v2::typed_array::TypedArray;
use shape_value::{VMError, ValueWord, ValueWordExt};
use std::mem::ManuallyDrop;

/// Borrow a `ValueWord` from raw `u64` bits without taking ownership.
#[inline]
fn borrow_vw(raw: u64) -> ManuallyDrop<ValueWord> {
    ManuallyDrop::new(ValueWord::from_raw_bits(raw))
}

/// Extract the receiver as a v2 `TypedArray<f64>` view.
#[inline]
fn extract_number_view(args: &mut [u64]) -> Result<V2TypedArrayView, VMError> {
    let vw = borrow_vw(args[0]);
    let view = v2::as_v2_typed_array(&vw).ok_or_else(|| VMError::TypeError {
        expected: "TypedArray<number>",
        got: vw.type_name(),
    })?;
    if view.elem_type != V2ElemType::F64 {
        return Err(VMError::TypeError {
            expected: "TypedArray<number>",
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
    let view = extract_number_view(args)?;
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
    let view = extract_number_view(args)?;
    let val_vw = borrow_vw(args[1]);
    v2::push_element(&view, &val_vw).map_err(|e| VMError::RuntimeError(e.to_string()))?;
    let new_len = unsafe {
        let arr = view.ptr as *const TypedArray<f64>;
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
    let view = extract_number_view(args)?;
    let val = v2::pop_element(&view).unwrap_or_else(ValueWord::none);
    Ok(val.into_raw_bits())
}

/// `arr.sum()` — sum all elements.
pub fn sum(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let view = extract_number_view(args)?;
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
    let view = extract_number_view(args)?;
    if view.len == 0 {
        return Err(VMError::RuntimeError(
            "first() called on empty Vec<number>".into(),
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
    let view = extract_number_view(args)?;
    if view.len == 0 {
        return Err(VMError::RuntimeError(
            "last() called on empty Vec<number>".into(),
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
    let view = extract_number_view(args)?;
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
    let view = extract_number_view(args)?;
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
    use crate::executor::v2_handlers::v2_array_detect::{ELEM_TYPE_F64, stamp_elem_type};
    use shape_value::v2::typed_array::TypedArray;

    fn make_number_array(values: &[f64]) -> (*mut TypedArray<f64>, u64) {
        let arr = TypedArray::<f64>::from_slice(values);
        unsafe {
            stamp_elem_type(arr as *mut u8, ELEM_TYPE_F64);
        }
        let bits = ValueWord::from_native_ptr(arr as usize).into_raw_bits();
        (arr, bits)
    }

    fn dummy_vm() -> VirtualMachine {
        VirtualMachine::new(VMConfig::default())
    }

    #[test]
    fn test_typed_number_array_len() {
        let (arr, bits) = make_number_array(&[1.0, 2.0, 3.0, 4.0]);
        let mut vm = dummy_vm();
        let mut args = [bits];
        let result = len(&mut vm, &mut args, None).expect("len");
        assert_eq!(ValueWord::from_raw_bits(result).as_i64(), Some(4));
        unsafe {
            TypedArray::drop_array(arr);
        }
    }

    #[test]
    fn test_typed_number_array_first_last() {
        let (arr, bits) = make_number_array(&[1.5, 2.5, 3.5]);
        let mut vm = dummy_vm();

        let mut args = [bits];
        let f = first(&mut vm, &mut args, None).expect("first");
        assert_eq!(ValueWord::from_raw_bits(f).as_f64(), Some(1.5));

        let mut args = [bits];
        let l = last(&mut vm, &mut args, None).expect("last");
        assert_eq!(ValueWord::from_raw_bits(l).as_f64(), Some(3.5));

        unsafe {
            TypedArray::drop_array(arr);
        }
    }

    #[test]
    fn test_typed_number_array_last_empty_errors() {
        let (arr, bits) = make_number_array(&[]);
        let mut vm = dummy_vm();
        let mut args = [bits];
        let err = last(&mut vm, &mut args, None).unwrap_err();
        match err {
            VMError::RuntimeError(msg) => assert!(msg.contains("empty")),
            other => panic!("unexpected error: {:?}", other),
        }
        unsafe {
            TypedArray::drop_array(arr);
        }
    }

    #[test]
    fn test_typed_number_array_sum() {
        let (arr, bits) = make_number_array(&[1.0, 2.0, 3.0, 4.0]);
        let mut vm = dummy_vm();
        let mut args = [bits];
        let result = sum(&mut vm, &mut args, None).expect("sum");
        assert_eq!(ValueWord::from_raw_bits(result).as_f64(), Some(10.0));
        unsafe {
            TypedArray::drop_array(arr);
        }
    }

    #[test]
    fn test_typed_number_array_get_set() {
        let (arr, bits) = make_number_array(&[1.0, 2.0, 3.0]);
        let mut vm = dummy_vm();

        // get(2)
        let idx_bits = ValueWord::from_i64(2).into_raw_bits();
        let mut args = [bits, idx_bits];
        let got = get(&mut vm, &mut args, None).expect("get");
        assert_eq!(ValueWord::from_raw_bits(got).as_f64(), Some(3.0));

        // set(0, 99.5)
        let idx_bits = ValueWord::from_i64(0).into_raw_bits();
        let val_bits = ValueWord::from_f64(99.5).into_raw_bits();
        let mut args = [bits, idx_bits, val_bits];
        set(&mut vm, &mut args, None).expect("set");

        // get(0) -> 99.5
        let idx_bits = ValueWord::from_i64(0).into_raw_bits();
        let mut args = [bits, idx_bits];
        let got = get(&mut vm, &mut args, None).expect("get after set");
        assert_eq!(ValueWord::from_raw_bits(got).as_f64(), Some(99.5));

        unsafe {
            TypedArray::drop_array(arr);
        }
    }

    #[test]
    fn test_typed_number_array_set_oob() {
        let (arr, bits) = make_number_array(&[1.0]);
        let mut vm = dummy_vm();
        let idx_bits = ValueWord::from_i64(10).into_raw_bits();
        let val_bits = ValueWord::from_f64(0.0).into_raw_bits();
        let mut args = [bits, idx_bits, val_bits];
        let err = set(&mut vm, &mut args, None).unwrap_err();
        assert!(matches!(err, VMError::IndexOutOfBounds { .. }));
        unsafe {
            TypedArray::drop_array(arr);
        }
    }

    #[test]
    fn test_typed_number_array_push_pop() {
        let (arr, bits) = make_number_array(&[1.0, 2.0]);
        let mut vm = dummy_vm();

        let val_bits = ValueWord::from_f64(3.0).into_raw_bits();
        let mut args = [bits, val_bits];
        let new_len = push(&mut vm, &mut args, None).expect("push");
        assert_eq!(ValueWord::from_raw_bits(new_len).as_i64(), Some(3));

        let mut args = [bits];
        let popped = pop(&mut vm, &mut args, None).expect("pop");
        assert_eq!(ValueWord::from_raw_bits(popped).as_f64(), Some(3.0));

        let mut args = [bits];
        let l = len(&mut vm, &mut args, None).expect("len");
        assert_eq!(ValueWord::from_raw_bits(l).as_i64(), Some(2));

        unsafe {
            TypedArray::drop_array(arr);
        }
    }

    #[test]
    fn test_typed_number_array_pop_empty_returns_none() {
        let (arr, bits) = make_number_array(&[]);
        let mut vm = dummy_vm();
        let mut args = [bits];
        let popped = pop(&mut vm, &mut args, None).expect("pop empty");
        assert!(ValueWord::from_raw_bits(popped).is_none());
        unsafe {
            TypedArray::drop_array(arr);
        }
    }
}
