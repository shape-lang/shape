//! Matrix intrinsic builtin implementations.
//!
//! `IntrinsicMatAdd` and `IntrinsicMatSub` are operator-retarget runtime
//! endpoints for statically-proven matrix arithmetic. The entry point stays
//! on the kinded carrier ABI: the dispatcher pops `KindedSlot` args, this
//! module projects only explicit matrix carriers or already-proven nested
//! `TypedArray<TypedArray<f64>>` values, and the result is returned as
//! `KindedSlot::from_matrix`.

use crate::bytecode::BuiltinFunction;
use crate::executor::v2_handlers::v2_array_detect::{V2ElemType, as_v2_typed_array, read_element};
use shape_value::aligned_vec::AlignedVec;
use shape_value::heap_value::{HeapKind, MatrixData};
use shape_value::v2::typed_array::TypedArray;
use shape_value::{KindedSlot, NativeKind, VMError};
use std::sync::Arc;

#[inline]
fn type_error(msg: impl Into<String>) -> VMError {
    VMError::RuntimeError(msg.into())
}

fn matrix_arg(args: &[KindedSlot], idx: usize, builtin: &str) -> Result<MatrixData, VMError> {
    let slot = args
        .get(idx)
        .ok_or_else(|| type_error(format!("{builtin} missing matrix argument {}", idx + 1)))?;
    match slot.kind {
        NativeKind::Ptr(HeapKind::Matrix) => {
            if slot.raw() == 0 {
                return Err(type_error(format!("{builtin} matrix argument is null")));
            }
            // SAFETY: `KindedSlot::from_matrix` stores `Arc::into_raw::<MatrixData>`
            // under `Ptr(HeapKind::Matrix)`. This slot owns a live matrix share for
            // the duration of the borrow; cloning the `MatrixData` avoids taking
            // ownership of the raw Arc pointer here.
            let matrix: &MatrixData = unsafe { &*(slot.raw() as *const MatrixData) };
            Ok(matrix.clone())
        }
        NativeKind::Ptr(HeapKind::TypedArray) => nested_typed_array_matrix(slot, builtin, idx),
        other => Err(type_error(format!(
            "{builtin} expects Mat<number> argument {}, got kind {:?}",
            idx + 1,
            other
        ))),
    }
}

fn nested_typed_array_matrix(
    slot: &KindedSlot,
    builtin: &str,
    arg_idx: usize,
) -> Result<MatrixData, VMError> {
    let outer = as_v2_typed_array(slot.raw(), slot.kind).ok_or_else(|| {
        type_error(format!(
            "{builtin} argument {} is not a v2 typed-array matrix",
            arg_idx + 1
        ))
    })?;
    if outer.elem_type != V2ElemType::TypedArray {
        return Err(type_error(format!(
            "{builtin} argument {} must be Mat<number> (nested typed-array rows), got {:?}",
            arg_idx + 1,
            outer.elem_type
        )));
    }

    let rows = outer.len as usize;
    let mut cols: Option<usize> = None;
    let mut flat = AlignedVec::<f64>::new();
    for row_idx in 0..rows {
        let (row_bits, row_kind) = read_element(&outer, row_idx as u32).ok_or_else(|| {
            type_error(format!(
                "{builtin} argument {} missing row {}",
                arg_idx + 1,
                row_idx
            ))
        })?;
        let row_result = (|| {
            let row_view = as_v2_typed_array(row_bits, row_kind).ok_or_else(|| {
                type_error(format!(
                    "{builtin} argument {} row {} is not a typed array",
                    arg_idx + 1,
                    row_idx
                ))
            })?;
            if row_view.elem_type != V2ElemType::F64 {
                return Err(type_error(format!(
                    "{builtin} argument {} row {} must be Vec<number>, got {:?}",
                    arg_idx + 1,
                    row_idx,
                    row_view.elem_type
                )));
            }
            let row_cols = row_view.len as usize;
            match cols {
                Some(expected) if expected != row_cols => {
                    return Err(type_error(format!(
                        "{builtin} argument {} is not rectangular: row {} has {}, expected {}",
                        arg_idx + 1,
                        row_idx,
                        row_cols,
                        expected
                    )));
                }
                None => cols = Some(row_cols),
                _ => {}
            }
            let row_ptr = row_view.ptr as *const TypedArray<f64>;
            // SAFETY: `row_view` comes from `as_v2_typed_array`, and the explicit
            // `elem_type == V2ElemType::F64` check above proves that `ptr` points
            // to a live `TypedArray<f64>`. `read_element` returned a retained row
            // share, which is dropped after this block with `drop_with_kind`.
            let row = unsafe { TypedArray::<f64>::as_slice(row_ptr) };
            for value in row {
                flat.push(*value);
            }
            Ok(())
        })();
        crate::executor::vm_impl::stack::drop_with_kind(row_bits, row_kind);
        row_result?;
    }

    Ok(MatrixData::from_flat(
        flat,
        rows as u32,
        cols.unwrap_or(0) as u32,
    ))
}

pub(in crate::executor) fn builtin_matrix_arithmetic(
    builtin: BuiltinFunction,
    args: &[KindedSlot],
) -> Result<KindedSlot, VMError> {
    if args.len() != 2 {
        return Err(type_error(format!(
            "{:?} expects exactly 2 matrix arguments, got {}",
            builtin,
            args.len()
        )));
    }
    let name = match builtin {
        BuiltinFunction::IntrinsicMatAdd => "IntrinsicMatAdd",
        BuiltinFunction::IntrinsicMatSub => "IntrinsicMatSub",
        _ => {
            return Err(type_error(format!(
                "{:?} is not a matrix arithmetic builtin",
                builtin
            )));
        }
    };
    let left = matrix_arg(args, 0, name)?;
    let right = matrix_arg(args, 1, name)?;
    let out = match builtin {
        BuiltinFunction::IntrinsicMatAdd => {
            shape_runtime::intrinsics::matrix_kernels::matrix_add(&left, &right)
        }
        BuiltinFunction::IntrinsicMatSub => {
            shape_runtime::intrinsics::matrix_kernels::matrix_sub(&left, &right)
        }
        _ => unreachable!(),
    }
    .map_err(VMError::RuntimeError)?;
    Ok(KindedSlot::from_matrix(Arc::new(out)))
}
