//! Method handlers for v2 typed arrays (`TypedArray<T>` receivers — Vec<int>,
//! Vec<number>, Vec<bool>, ...).
//!
//! The old `TypedArrayData` heap carrier was deleted during strict-typing
//! migration. These handlers consume the stamped v2-raw `TypedArray<T>`
//! carrier through `v2_array_detect`, preserving the compile-time element kind
//! instead of reconstructing a legacy `ValueWord`/`Constant::Value` carrier.
//!
//! Typed-specialized names live here. Names that are semantically identical
//! for all element kinds still fall through to `ARRAY_METHODS`.

use crate::executor::VirtualMachine;
use crate::executor::v2_handlers::v2_array_detect::{
    V2ElemType, V2TypedArrayView, all_elements, any_elements, as_v2_typed_array, clone_array,
    count_true_elements, diff_f64, dot_elements, max_elements, min_elements, norm_elements,
    std_elements, sum_elements, variance_elements,
};
use shape_runtime::context::ExecutionContext;
use shape_value::heap_value::HeapKind;
use shape_value::v2::typed_array::{ELEM_TYPE_F64, ELEM_TYPE_I64, TypedArray};
use shape_value::{KindedSlot, NativeKind, VMError, ValueSlot};

#[inline]
fn extract_view(op: &str, args: &[KindedSlot]) -> Result<V2TypedArrayView, VMError> {
    let receiver = args
        .first()
        .ok_or_else(|| VMError::RuntimeError(format!("{op}: missing receiver")))?;
    if receiver.kind != NativeKind::Ptr(HeapKind::TypedArray) {
        return Err(VMError::RuntimeError(format!(
            "{op}: expected v2 TypedArray receiver, got kind {:?}",
            receiver.kind
        )));
    }
    as_v2_typed_array(receiver.slot.raw(), receiver.kind).ok_or_else(|| {
        VMError::RuntimeError(format!(
            "{op}: receiver bits failed v2 TypedArray detection"
        ))
    })
}

#[inline]
fn require_arity(op: &str, args: &[KindedSlot], expected_call_args: usize) -> Result<(), VMError> {
    let actual = args.len().saturating_sub(1);
    if actual == expected_call_args {
        Ok(())
    } else {
        Err(VMError::RuntimeError(format!(
            "{op}: expected {expected_call_args} argument(s), got {actual}"
        )))
    }
}

#[inline]
fn pair_to_slot((bits, kind): (u64, NativeKind)) -> KindedSlot {
    KindedSlot::new(ValueSlot::from_raw(bits), kind)
}

#[inline]
fn typed_array_slot(ptr: *mut u8) -> KindedSlot {
    KindedSlot::new(
        ValueSlot::from_raw(ptr as usize as u64),
        NativeKind::Ptr(HeapKind::TypedArray),
    )
}

fn numeric_pair(
    op: &str,
    args: &[KindedSlot],
    f: fn(&V2TypedArrayView) -> Option<(u64, NativeKind)>,
) -> Result<KindedSlot, VMError> {
    require_arity(op, args, 0)?;
    let view = extract_view(op, args)?;
    f(&view).map(pair_to_slot).ok_or_else(|| {
        VMError::RuntimeError(format!(
            "{op}: not defined for element kind {:?}",
            view.elem_type
        ))
    })
}

fn f64_result_array(
    op: &str,
    args: &[KindedSlot],
    transform: impl Fn(&V2TypedArrayView, *mut f64) -> Result<u32, VMError>,
) -> Result<KindedSlot, VMError> {
    require_arity(op, args, 0)?;
    let view = extract_view(op, args)?;
    if view.elem_type != V2ElemType::F64 {
        return Err(VMError::RuntimeError(format!(
            "{op}: expected Vec<number>, got element kind {:?}",
            view.elem_type
        )));
    }
    let out = TypedArray::<f64>::with_capacity(view.len);
    let out_len = unsafe {
        let dst = (*out).data;
        transform(&view, dst)?
    };
    unsafe {
        (*out).len = out_len;
        crate::executor::v2_handlers::v2_array_detect::stamp_elem_type(
            out as *mut u8,
            ELEM_TYPE_F64,
        );
    }
    Ok(typed_array_slot(out as *mut u8))
}

fn f64_map_array(
    op: &str,
    args: &[KindedSlot],
    f: impl Fn(f64) -> f64,
) -> Result<KindedSlot, VMError> {
    f64_result_array(op, args, |view, dst| {
        let src = view.ptr as *const TypedArray<f64>;
        for i in 0..view.len {
            let value = unsafe { TypedArray::<f64>::get_unchecked(src, i) };
            unsafe {
                *dst.add(i as usize) = f(value);
            }
        }
        Ok(view.len)
    })
}

// ═════════════════════════════════════════════════════════════════════════════
// MethodFnV2 handlers
// ═════════════════════════════════════════════════════════════════════════════

/// `arr.len() / arr.length()` — element count.
pub fn v2_len(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    require_arity("Vec.len", args, 0)?;
    let view = extract_view("Vec.len", args)?;
    Ok(KindedSlot::from_int(view.len as i64))
}

/// `Vec<number>.sum()` — float aggregation.
pub fn v2_float_sum(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    numeric_pair("Vec<number>.sum", args, sum_elements)
}

/// `Vec<int>.sum()` — int aggregation.
pub fn v2_int_sum(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    numeric_pair("Vec<int>.sum", args, sum_elements)
}

/// `Vec<number>.avg() / Vec<number>.mean()`.
pub fn v2_float_avg(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    numeric_pair(
        "Vec<number>.avg",
        args,
        crate::executor::v2_handlers::v2_array_detect::avg_elements,
    )
}

/// `Vec<int>.avg() / Vec<int>.mean()`.
pub fn v2_int_avg(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    numeric_pair(
        "Vec<int>.avg",
        args,
        crate::executor::v2_handlers::v2_array_detect::avg_elements,
    )
}

/// `Vec<number>.min()`.
pub fn v2_float_min(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    numeric_pair("Vec<number>.min", args, min_elements)
}

/// `Vec<int>.min()`.
pub fn v2_int_min(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    numeric_pair("Vec<int>.min", args, min_elements)
}

/// `Vec<number>.max()`.
pub fn v2_float_max(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    numeric_pair("Vec<number>.max", args, max_elements)
}

/// `Vec<int>.max()`.
pub fn v2_int_max(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    numeric_pair("Vec<int>.max", args, max_elements)
}

/// `Vec<number>.variance()`.
pub fn v2_float_variance(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    numeric_pair("Vec<number>.variance", args, variance_elements)
}

/// `Vec<number>.std()`.
pub fn v2_float_std(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    numeric_pair("Vec<number>.std", args, std_elements)
}

/// `Vec<number>.dot(other)`.
pub fn v2_float_dot(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    require_arity("Vec<number>.dot", args, 1)?;
    let lhs = extract_view("Vec<number>.dot", args)?;
    let rhs = extract_view("Vec<number>.dot argument", &args[1..])?;
    if lhs.len != rhs.len {
        return Err(VMError::RuntimeError(format!(
            "Vec<number>.dot: length mismatch {} vs {}",
            lhs.len, rhs.len
        )));
    }
    dot_elements(&lhs, &rhs).map(pair_to_slot).ok_or_else(|| {
        VMError::RuntimeError("Vec<number>.dot: expected two Vec<number> receivers".into())
    })
}

/// `Vec<number>.norm()`.
pub fn v2_float_norm(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    numeric_pair("Vec<number>.norm", args, norm_elements)
}

/// `Vec<bool>.count()`.
pub fn v2_bool_count(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() > 1 {
        return crate::executor::objects::array_aggregation::handle_count_v2(vm, args, ctx);
    }
    let view = extract_view("Vec<bool>.count", args)?;
    count_true_elements(&view)
        .map(pair_to_slot)
        .ok_or_else(|| VMError::RuntimeError("Vec<bool>.count: expected Vec<bool> receiver".into()))
}

/// `Vec<bool>.any()`.
pub fn v2_bool_any(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() > 1 {
        return crate::executor::objects::array_query::handle_any_v2(vm, args, ctx);
    }
    let view = extract_view("Vec<bool>.any", args)?;
    any_elements(&view)
        .map(pair_to_slot)
        .ok_or_else(|| VMError::RuntimeError("Vec<bool>.any: expected Vec<bool> receiver".into()))
}

/// `Vec<bool>.all()`.
pub fn v2_bool_all(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() > 1 {
        return crate::executor::objects::array_query::handle_all_v2(vm, args, ctx);
    }
    let view = extract_view("Vec<bool>.all", args)?;
    all_elements(&view)
        .map(pair_to_slot)
        .ok_or_else(|| VMError::RuntimeError("Vec<bool>.all: expected Vec<bool> receiver".into()))
}

// ═════════════════════════════════════════════════════════════════════════════
// Float unary transforms
// ═════════════════════════════════════════════════════════════════════════════

/// `Vec<number>.normalize()`.
pub(crate) fn handle_float_normalize(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let norm = {
        require_arity("Vec<number>.normalize", args, 0)?;
        let view = extract_view("Vec<number>.normalize", args)?;
        let (bits, kind) = norm_elements(&view).ok_or_else(|| {
            VMError::RuntimeError(format!(
                "Vec<number>.normalize: expected Vec<number>, got {:?}",
                view.elem_type
            ))
        })?;
        debug_assert_eq!(kind, NativeKind::Float64);
        f64::from_bits(bits)
    };
    f64_map_array("Vec<number>.normalize", args, |x| x / norm)
}

/// `Vec<number>.cumsum()`.
pub(crate) fn handle_float_cumsum(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    f64_result_array("Vec<number>.cumsum", args, |view, dst| {
        let src = view.ptr as *const TypedArray<f64>;
        let mut acc = 0.0_f64;
        for i in 0..view.len {
            acc += unsafe { TypedArray::<f64>::get_unchecked(src, i) };
            unsafe {
                *dst.add(i as usize) = acc;
            }
        }
        Ok(view.len)
    })
}

/// `Vec<number>.diff()`.
pub(crate) fn handle_float_diff(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    require_arity("Vec<number>.diff", args, 0)?;
    let view = extract_view("Vec<number>.diff", args)?;
    diff_f64(&view).map(typed_array_slot).ok_or_else(|| {
        VMError::RuntimeError("Vec<number>.diff: expected Vec<number> receiver".into())
    })
}

/// `Vec<number>.abs()`.
pub(crate) fn handle_float_abs(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    f64_map_array("Vec<number>.abs", args, f64::abs)
}

/// `Vec<number>.sqrt()`.
pub(crate) fn handle_float_sqrt(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    f64_map_array("Vec<number>.sqrt", args, f64::sqrt)
}

/// `Vec<number>.ln()`.
pub(crate) fn handle_float_ln(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    f64_map_array("Vec<number>.ln", args, f64::ln)
}

/// `Vec<number>.exp()`.
pub(crate) fn handle_float_exp(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    f64_map_array("Vec<number>.exp", args, f64::exp)
}

// ═════════════════════════════════════════════════════════════════════════════
// Float higher-order
// ═════════════════════════════════════════════════════════════════════════════

/// `Vec<number>.map(|x| ...)`.
pub(crate) fn handle_float_map(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    crate::executor::objects::array_transform::handle_map_v2(vm, args, ctx)
}

/// `Vec<number>.filter(|x| ...)`.
pub(crate) fn handle_float_filter(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    crate::executor::objects::array_transform::handle_filter_v2(vm, args, ctx)
}

/// `Vec<number>.forEach(|x| ...)`.
pub(crate) fn handle_float_for_each(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    crate::executor::objects::array_query::handle_for_each_v2(vm, args, ctx)
}

/// `Vec<number>.reduce(|acc, x| ...) / .fold(init, |acc, x| ...)`.
pub(crate) fn handle_float_reduce(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    crate::executor::objects::array_aggregation::handle_reduce_v2(vm, args, ctx)
}

/// `Vec<number>.find(|x| ...)`.
pub(crate) fn handle_float_find(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    crate::executor::objects::array_query::handle_find_v2(vm, args, ctx)
}

/// `Vec<number>.some(|x| ...)`.
pub(crate) fn handle_float_some(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    crate::executor::objects::array_query::handle_some_v2(vm, args, ctx)
}

/// `Vec<number>.every(|x| ...)`.
pub(crate) fn handle_float_every(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    crate::executor::objects::array_query::handle_every_v2(vm, args, ctx)
}

/// `Vec<number>.toArray()`.
pub(crate) fn handle_float_to_array(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    require_arity("Vec<number>.toArray", args, 0)?;
    let view = extract_view("Vec<number>.toArray", args)?;
    Ok(typed_array_slot(clone_array(&view)))
}

// ═════════════════════════════════════════════════════════════════════════════
// Int handlers
// ═════════════════════════════════════════════════════════════════════════════

/// `Vec<int>.abs()`.
pub(crate) fn handle_int_abs(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    require_arity("Vec<int>.abs", args, 0)?;
    let view = extract_view("Vec<int>.abs", args)?;
    if view.elem_type != V2ElemType::I64 {
        return Err(VMError::RuntimeError(format!(
            "Vec<int>.abs: expected Vec<int>, got element kind {:?}",
            view.elem_type
        )));
    }
    let out = TypedArray::<i64>::with_capacity(view.len);
    unsafe {
        let src = view.ptr as *const TypedArray<i64>;
        let dst = (*out).data;
        for i in 0..view.len {
            let value = TypedArray::<i64>::get_unchecked(src, i);
            let abs = value.checked_abs().ok_or_else(|| {
                VMError::RuntimeError("Vec<int>.abs: i64::MIN cannot be represented".into())
            })?;
            *dst.add(i as usize) = abs;
        }
        (*out).len = view.len;
        crate::executor::v2_handlers::v2_array_detect::stamp_elem_type(
            out as *mut u8,
            ELEM_TYPE_I64,
        );
    }
    Ok(typed_array_slot(out as *mut u8))
}

/// `Vec<int>.map(|x| ...)`.
pub(crate) fn handle_int_map(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    crate::executor::objects::array_transform::handle_map_v2(vm, args, ctx)
}

/// `Vec<int>.filter(|x| ...)`.
pub(crate) fn handle_int_filter(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    crate::executor::objects::array_transform::handle_filter_v2(vm, args, ctx)
}

/// `Vec<int>.forEach(|x| ...)`.
pub(crate) fn handle_int_for_each(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    crate::executor::objects::array_query::handle_for_each_v2(vm, args, ctx)
}

/// `Vec<int>.reduce(|acc, x| ...) / .fold(init, |acc, x| ...)`.
pub(crate) fn handle_int_reduce(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    crate::executor::objects::array_aggregation::handle_reduce_v2(vm, args, ctx)
}

/// `Vec<int>.find(|x| ...)`.
pub(crate) fn handle_int_find(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    crate::executor::objects::array_query::handle_find_v2(vm, args, ctx)
}

/// `Vec<int>.some(|x| ...)`.
pub(crate) fn handle_int_some(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    crate::executor::objects::array_query::handle_some_v2(vm, args, ctx)
}

/// `Vec<int>.every(|x| ...)`.
pub(crate) fn handle_int_every(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    crate::executor::objects::array_query::handle_every_v2(vm, args, ctx)
}

/// `Vec<int>.toArray()`.
pub(crate) fn handle_int_to_array(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    require_arity("Vec<int>.toArray", args, 0)?;
    let view = extract_view("Vec<int>.toArray", args)?;
    Ok(typed_array_slot(clone_array(&view)))
}

/// `Vec<bool>.toArray()`.
pub(crate) fn handle_bool_to_array(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    require_arity("Vec<bool>.toArray", args, 0)?;
    let view = extract_view("Vec<bool>.toArray", args)?;
    Ok(typed_array_slot(clone_array(&view)))
}
