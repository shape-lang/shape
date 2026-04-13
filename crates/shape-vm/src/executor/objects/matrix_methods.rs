//! Matrix method handlers for the PHF method registry.

use crate::executor::VirtualMachine;
use shape_runtime::context::ExecutionContext;
use shape_runtime::intrinsics::matrix_kernels;
use shape_value::aligned_vec::AlignedVec;
use shape_value::heap_value::MatrixData;
use shape_value::typed_buffer::AlignedTypedBuffer;
use shape_value::{VMError, ValueWord};
use std::sync::Arc;

use super::raw_helpers::{extract_matrix, extract_matrix_arc, extract_number_coerce};

// ---------------------------------------------------------------------------
// MethodFnV2 handlers (v2 ABI: fn(&mut VM, &[u64], ctx) -> Result<u64>)
// ---------------------------------------------------------------------------

/// mat.transpose() -> Matrix (v2)
pub fn v2_transpose(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let m = extract_matrix(args[0]).ok_or_else(|| VMError::TypeError {
        expected: "Matrix",
        got: super::raw_helpers::type_name_from_bits(args[0]),
    })?;
    let result = matrix_kernels::matrix_transpose(m);
    Ok(ValueWord::from_matrix(Arc::new(result)).into_raw_bits())
}

/// mat.inverse() -> Matrix (v2)
pub fn v2_inverse(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let m = extract_matrix(args[0]).ok_or_else(|| VMError::TypeError {
        expected: "Matrix",
        got: super::raw_helpers::type_name_from_bits(args[0]),
    })?;
    let result = matrix_kernels::matrix_inverse(m).map_err(|e| VMError::RuntimeError(e))?;
    Ok(ValueWord::from_matrix(Arc::new(result)).into_raw_bits())
}

/// mat.det() / mat.determinant() -> number (v2)
pub fn v2_determinant(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let m = extract_matrix(args[0]).ok_or_else(|| VMError::TypeError {
        expected: "Matrix",
        got: super::raw_helpers::type_name_from_bits(args[0]),
    })?;
    let result = matrix_kernels::matrix_determinant(m).map_err(|e| VMError::RuntimeError(e))?;
    Ok(ValueWord::from_f64(result).into_raw_bits())
}

/// mat.trace() -> number (v2)
pub fn v2_trace(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let m = extract_matrix(args[0]).ok_or_else(|| VMError::TypeError {
        expected: "Matrix",
        got: super::raw_helpers::type_name_from_bits(args[0]),
    })?;
    let result = matrix_kernels::matrix_trace(m).map_err(|e| VMError::RuntimeError(e))?;
    Ok(ValueWord::from_f64(result).into_raw_bits())
}

/// mat.shape() -> [rows, cols] (v2)
pub fn v2_shape(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let m = extract_matrix(args[0]).ok_or_else(|| VMError::TypeError {
        expected: "Matrix",
        got: super::raw_helpers::type_name_from_bits(args[0]),
    })?;
    let (rows, cols) = m.shape();
    let pair = vec![
        ValueWord::from_i64(rows as i64),
        ValueWord::from_i64(cols as i64),
    ];
    Ok(ValueWord::from_array(Arc::new(pair)).into_raw_bits())
}

/// mat.reshape(rows, cols) -> Matrix (v2)
pub fn v2_reshape(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let m = extract_matrix(args[0]).ok_or_else(|| VMError::TypeError {
        expected: "Matrix",
        got: super::raw_helpers::type_name_from_bits(args[0]),
    })?;

    let new_rows = args
        .get(1)
        .and_then(|&r| extract_number_coerce(r))
        .ok_or_else(|| VMError::RuntimeError("reshape requires rows argument".to_string()))?
        as u32;

    let new_cols = args
        .get(2)
        .and_then(|&r| extract_number_coerce(r))
        .ok_or_else(|| VMError::RuntimeError("reshape requires cols argument".to_string()))?
        as u32;

    let total = (new_rows as usize) * (new_cols as usize);
    if total != m.data.len() {
        return Err(VMError::RuntimeError(format!(
            "Cannot reshape {}x{} matrix to {}x{}: element count mismatch ({} vs {})",
            m.rows,
            m.cols,
            new_rows,
            new_cols,
            m.data.len(),
            total
        )));
    }

    let mut data = AlignedVec::with_capacity(total);
    for v in m.data.iter() {
        data.push(*v);
    }
    Ok(ValueWord::from_matrix(Arc::new(MatrixData::from_flat(data, new_rows, new_cols))).into_raw_bits())
}

/// mat.row(i) -> FloatArraySlice (v2)
pub fn v2_row(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let m = extract_matrix(args[0]).ok_or_else(|| VMError::TypeError {
        expected: "Matrix",
        got: super::raw_helpers::type_name_from_bits(args[0]),
    })?;

    let i = args
        .get(1)
        .and_then(|&r| extract_number_coerce(r))
        .ok_or_else(|| VMError::RuntimeError("row requires an index argument".to_string()))?
        as i64;

    let rows = m.rows as i64;
    let cols = m.cols;
    let actual = if i < 0 { rows + i } else { i };
    if actual < 0 || actual >= rows {
        return Err(VMError::RuntimeError(format!(
            "Row index {} out of bounds for {}x{} matrix",
            i, m.rows, m.cols
        )));
    }

    // Extract the Arc<MatrixData> from the receiver HeapValue
    let parent_arc = extract_matrix_arc(args[0])
        .expect("extract_matrix succeeded so this must be Matrix");

    let offset = actual as u32 * cols;
    let len = cols;
    Ok(ValueWord::from_heap_value(
        shape_value::heap_value::HeapValue::FloatArraySlice {
            parent: parent_arc,
            offset,
            len,
        },
    )
    .into_raw_bits())
}

/// mat.col(j) -> FloatArray (v2)
pub fn v2_col(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let m = extract_matrix(args[0]).ok_or_else(|| VMError::TypeError {
        expected: "Matrix",
        got: super::raw_helpers::type_name_from_bits(args[0]),
    })?;

    let j = args
        .get(1)
        .and_then(|&r| extract_number_coerce(r))
        .ok_or_else(|| VMError::RuntimeError("col requires an index argument".to_string()))?
        as i64;

    let cols = m.cols as i64;
    let actual = if j < 0 { cols + j } else { j };
    if actual < 0 || actual >= cols {
        return Err(VMError::RuntimeError(format!(
            "Column index {} out of bounds for {}x{} matrix",
            j, m.rows, m.cols
        )));
    }

    let col_idx = actual as usize;
    let n_cols = m.cols as usize;
    let mut col_data = AlignedVec::with_capacity(m.rows as usize);
    for i in 0..m.rows as usize {
        col_data.push(m.data[i * n_cols + col_idx]);
    }
    Ok(ValueWord::from_float_array(Arc::new(AlignedTypedBuffer::from_aligned(col_data))).into_raw_bits())
}

/// mat.diag() -> FloatArray (v2)
pub fn v2_diag(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let m = extract_matrix(args[0]).ok_or_else(|| VMError::TypeError {
        expected: "Matrix",
        got: super::raw_helpers::type_name_from_bits(args[0]),
    })?;
    let n = m.rows.min(m.cols) as usize;
    let cols = m.cols as usize;
    let mut diag = AlignedVec::with_capacity(n);
    for i in 0..n {
        diag.push(m.data[i * cols + i]);
    }
    Ok(ValueWord::from_float_array(Arc::new(AlignedTypedBuffer::from_aligned(diag))).into_raw_bits())
}

/// mat.flatten() -> FloatArray (v2)
pub fn v2_flatten(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let m = extract_matrix(args[0]).ok_or_else(|| VMError::TypeError {
        expected: "Matrix",
        got: super::raw_helpers::type_name_from_bits(args[0]),
    })?;
    let mut flat = AlignedVec::with_capacity(m.data.len());
    for &v in m.data.iter() {
        flat.push(v);
    }
    Ok(ValueWord::from_float_array(Arc::new(AlignedTypedBuffer::from_aligned(flat))).into_raw_bits())
}

/// mat.sum() -> number (v2)
pub fn v2_sum(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let m = extract_matrix(args[0]).ok_or_else(|| VMError::TypeError {
        expected: "Matrix",
        got: super::raw_helpers::type_name_from_bits(args[0]),
    })?;
    let sum: f64 = m.data.iter().sum();
    Ok(ValueWord::from_f64(sum).into_raw_bits())
}

/// mat.min() -> number (v2)
pub fn v2_min(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let m = extract_matrix(args[0]).ok_or_else(|| VMError::TypeError {
        expected: "Matrix",
        got: super::raw_helpers::type_name_from_bits(args[0]),
    })?;
    if m.data.is_empty() {
        return Ok(ValueWord::none().into_raw_bits());
    }
    let min = m.data.iter().copied().fold(f64::INFINITY, f64::min);
    Ok(ValueWord::from_f64(min).into_raw_bits())
}

/// mat.max() -> number (v2)
pub fn v2_max(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let m = extract_matrix(args[0]).ok_or_else(|| VMError::TypeError {
        expected: "Matrix",
        got: super::raw_helpers::type_name_from_bits(args[0]),
    })?;
    if m.data.is_empty() {
        return Ok(ValueWord::none().into_raw_bits());
    }
    let max = m.data.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    Ok(ValueWord::from_f64(max).into_raw_bits())
}

/// mat.mean() -> number (v2)
pub fn v2_mean(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let m = extract_matrix(args[0]).ok_or_else(|| VMError::TypeError {
        expected: "Matrix",
        got: super::raw_helpers::type_name_from_bits(args[0]),
    })?;
    if m.data.is_empty() {
        return Ok(ValueWord::none().into_raw_bits());
    }
    let sum: f64 = m.data.iter().sum();
    Ok(ValueWord::from_f64(sum / m.data.len() as f64).into_raw_bits())
}

/// mat.rowSum() -> FloatArray (v2)
pub fn v2_row_sum(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let m = extract_matrix(args[0]).ok_or_else(|| VMError::TypeError {
        expected: "Matrix",
        got: super::raw_helpers::type_name_from_bits(args[0]),
    })?;
    let mut result = AlignedVec::with_capacity(m.rows as usize);
    for i in 0..m.rows as usize {
        let sum: f64 = m.row_slice(i as u32).iter().sum();
        result.push(sum);
    }
    Ok(ValueWord::from_float_array(Arc::new(AlignedTypedBuffer::from_aligned(result))).into_raw_bits())
}

/// mat.colSum() -> FloatArray (v2)
pub fn v2_col_sum(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let m = extract_matrix(args[0]).ok_or_else(|| VMError::TypeError {
        expected: "Matrix",
        got: super::raw_helpers::type_name_from_bits(args[0]),
    })?;
    let rows = m.rows as usize;
    let cols = m.cols as usize;
    let mut result = AlignedVec::with_capacity(cols);
    for j in 0..cols {
        let mut sum = 0.0;
        for i in 0..rows {
            sum += m.data[i * cols + j];
        }
        result.push(sum);
    }
    Ok(ValueWord::from_float_array(Arc::new(AlignedTypedBuffer::from_aligned(result))).into_raw_bits())
}

/// mat.map(fn) -> Matrix (v2 — native u64 ABI)
pub(crate) fn handle_map(
    vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "Matrix.map requires a function argument".to_string(),
        ));
    }
    let m = extract_matrix(args[0]).ok_or_else(|| VMError::TypeError {
        expected: "Matrix",
        got: super::raw_helpers::type_name_from_bits(args[0]),
    })?.clone();

    let mut result = AlignedVec::with_capacity(m.data.len());
    for &v in m.data.iter() {
        let elem_bits = ValueWord::from_f64(v).into_raw_bits();
        let result_bits = vm.call_value_immediate_raw(args[1], &[elem_bits], None)?;
        let val = extract_number_coerce(result_bits);
        drop(ValueWord::from_raw_bits(result_bits));
        let val = val.ok_or_else(|| {
            VMError::RuntimeError("Matrix.map callback must return a number".to_string())
        })?;
        result.push(val);
    }

    Ok(ValueWord::from_matrix(Arc::new(MatrixData::from_flat(result, m.rows, m.cols))).into_raw_bits())
}
