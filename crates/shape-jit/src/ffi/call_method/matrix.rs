//! Matrix method dispatch for JIT.

use crate::jit_matrix::JitMatrix;
use crate::ffi::jit_kinds::*;
use crate::ffi::value_ffi::*;
use shape_value::aligned_vec::AlignedVec;
use shape_value::heap_value::MatrixData;
use std::sync::Arc;

/// Dispatch a method call on a Matrix receiver.
pub fn call_matrix_method(receiver_bits: u64, method_name: &str, args: &[u64]) -> u64 {
    if !is_heap_kind(receiver_bits, HK_MATRIX) {
        return TAG_NULL;
    }
    let jm = unsafe { unified_unbox::<JitMatrix>(receiver_bits) };

    match method_name {
        "transpose" => matrix_transpose(jm),
        "flatten" => matrix_flatten(jm),
        "shape" => matrix_shape(jm),
        "row" => {
            let idx = if !args.is_empty() && is_number(args[0]) {
                unbox_number(args[0]) as usize
            } else {
                return TAG_NULL;
            };
            matrix_row(jm, idx)
        }
        "col" => {
            let idx = if !args.is_empty() && is_number(args[0]) {
                unbox_number(args[0]) as usize
            } else {
                return TAG_NULL;
            };
            matrix_col(jm, idx)
        }
        "rows" => box_number(jm.rows as f64),
        "cols" => box_number(jm.cols as f64),
        _ => TAG_NULL,
    }
}

fn matrix_transpose(jm: &JitMatrix) -> u64 {
    let rows = jm.rows as usize;
    let cols = jm.cols as usize;
    let data = unsafe { std::slice::from_raw_parts(jm.data, jm.total_len as usize) };
    let mut result = vec![0.0f64; rows * cols];
    for i in 0..rows {
        for j in 0..cols {
            result[j * rows + i] = data[i * cols + j];
        }
    }
    let aligned = AlignedVec::from_vec(result);
    let mat_data = MatrixData::from_flat(aligned, cols as u32, rows as u32);
    let arc = Arc::new(mat_data);
    let new_jm = JitMatrix::from_arc(&arc);
    std::mem::forget(arc); // JitMatrix::from_arc already cloned the Arc
    unified_box(HK_MATRIX, new_jm)
}

// SURFACE (W10 jit-playbook §5 / ADR-006 §2.7.4): matrix_flatten /
// matrix_shape / matrix_row / matrix_col all returned a freshly
// allocated `JitArray` of `box_number` elements via the deleted
// `JitArray::new()` / `push` / `heap_box` constructors. The kinded
// rebuild allocates a `TypedArray<f64>` for the result and returns
// it through the §2.7.6/Q8 carrier with `kind =
// NativeKind::Ptr(HeapKind::TypedArray)` (element kind =
// NativeKind::Float64). Every caller in `call_matrix_method` flows
// through the surface below.

fn matrix_flatten(_jm: &JitMatrix) -> u64 {
    todo!(
        "phase-2c §2.7.4 / W10 jit-playbook §5: JitArray rebuild — \
         matrix_flatten. Result allocation needs `TypedArray<f64>` \
         per ADR-006 §2.7.6/Q8."
    )
}

fn matrix_shape(_jm: &JitMatrix) -> u64 {
    todo!(
        "phase-2c §2.7.4 / W10 jit-playbook §5: JitArray rebuild — \
         matrix_shape. Result allocation needs `TypedArray<f64>` \
         (or `TypedArray<i64>`) per ADR-006 §2.7.6/Q8."
    )
}

fn matrix_row(_jm: &JitMatrix, _idx: usize) -> u64 {
    todo!(
        "phase-2c §2.7.4 / W10 jit-playbook §5: JitArray rebuild — \
         matrix_row. Result allocation needs `TypedArray<f64>` \
         per ADR-006 §2.7.6/Q8."
    )
}

fn matrix_col(_jm: &JitMatrix, _idx: usize) -> u64 {
    todo!(
        "phase-2c §2.7.4 / W10 jit-playbook §5: JitArray rebuild — \
         matrix_col. Result allocation needs `TypedArray<f64>` \
         per ADR-006 §2.7.6/Q8."
    )
}
