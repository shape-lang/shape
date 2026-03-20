//! Matrix method dispatch for JIT.

use crate::jit_matrix::JitMatrix;
use crate::nan_boxing::*;
use shape_value::aligned_vec::AlignedVec;
use shape_value::heap_value::MatrixData;
use std::sync::Arc;

/// Dispatch a method call on a Matrix receiver.
pub fn call_matrix_method(receiver_bits: u64, method_name: &str, args: &[u64]) -> u64 {
    if !is_heap_kind(receiver_bits, HK_MATRIX) {
        return TAG_NULL;
    }
    let jm = unsafe { jit_unbox::<JitMatrix>(receiver_bits) };

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
    jit_box(HK_MATRIX, new_jm)
}

fn matrix_flatten(jm: &JitMatrix) -> u64 {
    let data = unsafe { std::slice::from_raw_parts(jm.data, jm.total_len as usize) };
    use crate::jit_array::JitArray;
    let mut arr = JitArray::new();
    for &val in data {
        arr.push(box_number(val));
    }
    arr.heap_box()
}

fn matrix_shape(jm: &JitMatrix) -> u64 {
    use crate::jit_array::JitArray;
    let mut arr = JitArray::new();
    arr.push(box_number(jm.rows as f64));
    arr.push(box_number(jm.cols as f64));
    arr.heap_box()
}

fn matrix_row(jm: &JitMatrix, idx: usize) -> u64 {
    let rows = jm.rows as usize;
    let cols = jm.cols as usize;
    if idx >= rows {
        return TAG_NULL;
    }
    let data = unsafe { std::slice::from_raw_parts(jm.data, jm.total_len as usize) };
    use crate::jit_array::JitArray;
    let mut arr = JitArray::new();
    let start = idx * cols;
    for j in 0..cols {
        arr.push(box_number(data[start + j]));
    }
    arr.heap_box()
}

fn matrix_col(jm: &JitMatrix, idx: usize) -> u64 {
    let _rows = jm.rows as usize;
    let cols = jm.cols as usize;
    if idx >= cols {
        return TAG_NULL;
    }
    let data = unsafe { std::slice::from_raw_parts(jm.data, jm.total_len as usize) };
    use crate::jit_array::JitArray;
    let mut arr = JitArray::new();
    let rows = jm.rows as usize;
    for i in 0..rows {
        arr.push(box_number(data[i * cols + idx]));
    }
    arr.heap_box()
}
