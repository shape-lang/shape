//! Matrix method handlers for the PHF method registry.
//!
//! All methods follow the MethodFn signature:
//! fn(&mut VirtualMachine, Vec<ValueWord>, Option<&mut ExecutionContext>) -> Result<ValueWord, VMError>

use crate::executor::VirtualMachine;
use shape_runtime::context::ExecutionContext;
use shape_runtime::intrinsics::matrix_kernels;
use shape_value::aligned_vec::AlignedVec;
use shape_value::heap_value::MatrixData;
use shape_value::typed_buffer::AlignedTypedBuffer;
use shape_value::{VMError, ValueWord};
use std::sync::Arc;
use std::mem::ManuallyDrop;

/// Borrow a `ValueWord` from a raw u64 without taking ownership.
///
/// The caller holds args by `&[u64]` — the underlying `Arc<HeapValue>` is
/// still owned by the VM stack/register file. Constructing a `ValueWord`
/// via `from_raw_bits` would create a second owner of the same Arc, causing
/// a double-free on drop. `ManuallyDrop` suppresses the extra drop.
#[inline]
fn borrow_vw(raw: u64) -> ManuallyDrop<ValueWord> {
    ManuallyDrop::new(ValueWord::from_raw_bits(raw))
}

fn args_to_vw(args: &mut [u64]) -> Vec<shape_value::ValueWord> {
    args.iter().map(|&raw| (*borrow_vw(raw)).clone()).collect()
}

fn extract_matrix_vw(vw: &ManuallyDrop<ValueWord>) -> Result<&MatrixData, VMError> {
    vw.as_matrix().ok_or_else(|| VMError::TypeError {
        expected: "Matrix",
        got: vw.type_name(),
    })
}

fn extract_matrix(nb: &ValueWord) -> Result<&MatrixData, VMError> {
    nb.as_matrix().ok_or_else(|| VMError::TypeError {
        expected: "Matrix",
        got: nb.type_name(),
    })
}

/// mat.transpose() -> Matrix
pub fn handle_transpose(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let m = extract_matrix(&args[0])?;
    let result = matrix_kernels::matrix_transpose(m);
    Ok(ValueWord::from_matrix(std::sync::Arc::new(result)))
}

/// mat.inverse() -> Matrix (errors if singular)
pub fn handle_inverse(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let m = extract_matrix(&args[0])?;
    let result = matrix_kernels::matrix_inverse(m).map_err(|e| VMError::RuntimeError(e))?;
    Ok(ValueWord::from_matrix(std::sync::Arc::new(result)))
}

/// mat.det() or mat.determinant() -> number
pub fn handle_determinant(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let m = extract_matrix(&args[0])?;
    let result = matrix_kernels::matrix_determinant(m).map_err(|e| VMError::RuntimeError(e))?;
    Ok(ValueWord::from_f64(result))
}

/// mat.trace() -> number
pub fn handle_trace(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let m = extract_matrix(&args[0])?;
    let result = matrix_kernels::matrix_trace(m).map_err(|e| VMError::RuntimeError(e))?;
    Ok(ValueWord::from_f64(result))
}

/// mat.shape() -> [rows, cols]
pub fn handle_shape(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let m = extract_matrix(&args[0])?;
    let (rows, cols) = m.shape();
    let pair = vec![
        ValueWord::from_i64(rows as i64),
        ValueWord::from_i64(cols as i64),
    ];
    Ok(ValueWord::from_array(Arc::new(pair)))
}

/// mat.reshape(rows, cols) -> Matrix
pub fn handle_reshape(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let m = extract_matrix(&args[0])?;
    let new_rows = args
        .get(1)
        .and_then(|nb| nb.as_number_coerce())
        .ok_or_else(|| VMError::RuntimeError("reshape requires rows argument".to_string()))?
        as u32;
    let new_cols = args
        .get(2)
        .and_then(|nb| nb.as_number_coerce())
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
    Ok(ValueWord::from_matrix(std::sync::Arc::new(
        MatrixData::from_flat(data, new_rows, new_cols),
    )))
}

/// mat.row(i) -> FloatArraySlice (zero-copy view into matrix row)
pub fn handle_row(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let m = extract_matrix(&args[0])?;
    let i = args
        .get(1)
        .and_then(|nb| nb.as_number_coerce())
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
    let parent_arc = match args[0].as_heap_ref() {
        Some(shape_value::heap_value::HeapValue::Matrix(arc)) => arc.clone(),
        _ => unreachable!("extract_matrix succeeded so this must be Matrix"),
    };

    let offset = actual as u32 * cols;
    let len = cols;
    Ok(ValueWord::from_heap_value(
        shape_value::heap_value::HeapValue::FloatArraySlice {
            parent: parent_arc,
            offset,
            len,
        },
    ))
}

/// mat.col(j) -> FloatArray
pub fn handle_col(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let m = extract_matrix(&args[0])?;
    let j = args
        .get(1)
        .and_then(|nb| nb.as_number_coerce())
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
    Ok(ValueWord::from_float_array(Arc::new(
        AlignedTypedBuffer::from_aligned(col_data),
    )))
}

/// mat.diag() -> FloatArray (main diagonal)
pub fn handle_diag(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let m = extract_matrix(&args[0])?;
    let n = m.rows.min(m.cols) as usize;
    let cols = m.cols as usize;
    let mut diag = AlignedVec::with_capacity(n);
    for i in 0..n {
        diag.push(m.data[i * cols + i]);
    }
    Ok(ValueWord::from_float_array(Arc::new(
        AlignedTypedBuffer::from_aligned(diag),
    )))
}

/// mat.flatten() -> FloatArray
pub fn handle_flatten(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let m = extract_matrix(&args[0])?;
    let mut flat = AlignedVec::with_capacity(m.data.len());
    for &v in m.data.iter() {
        flat.push(v);
    }
    Ok(ValueWord::from_float_array(Arc::new(
        AlignedTypedBuffer::from_aligned(flat),
    )))
}

/// mat.map(fn) -> Matrix
pub fn handle_map_legacy(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "Matrix.map requires a function argument".to_string(),
        ));
    }
    let m = extract_matrix(&args[0])?.clone();
    let callback = args[1].clone();

    let mut result = AlignedVec::with_capacity(m.data.len());
    for &v in m.data.iter() {
        let mapped = vm.call_value_immediate_nb(&callback, &[ValueWord::from_f64(v)], None)?;
        let val = mapped.as_number_coerce().ok_or_else(|| {
            VMError::RuntimeError("Matrix.map callback must return a number".to_string())
        })?;
        result.push(val);
    }

    Ok(ValueWord::from_matrix(std::sync::Arc::new(
        MatrixData::from_flat(result, m.rows, m.cols),
    )))
}

/// mat.sum() -> number
pub fn handle_sum(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let m = extract_matrix(&args[0])?;
    let sum: f64 = m.data.iter().sum();
    Ok(ValueWord::from_f64(sum))
}

/// mat.min() -> number
pub fn handle_min(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let m = extract_matrix(&args[0])?;
    if m.data.is_empty() {
        return Ok(ValueWord::none());
    }
    let min = m.data.iter().copied().fold(f64::INFINITY, f64::min);
    Ok(ValueWord::from_f64(min))
}

/// mat.max() -> number
pub fn handle_max(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let m = extract_matrix(&args[0])?;
    if m.data.is_empty() {
        return Ok(ValueWord::none());
    }
    let max = m.data.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    Ok(ValueWord::from_f64(max))
}

/// mat.mean() -> number
pub fn handle_mean(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let m = extract_matrix(&args[0])?;
    if m.data.is_empty() {
        return Ok(ValueWord::none());
    }
    let sum: f64 = m.data.iter().sum();
    Ok(ValueWord::from_f64(sum / m.data.len() as f64))
}

/// mat.rowSum() -> FloatArray
pub fn handle_row_sum(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let m = extract_matrix(&args[0])?;
    let mut result = AlignedVec::with_capacity(m.rows as usize);
    for i in 0..m.rows as usize {
        let sum: f64 = m.row_slice(i as u32).iter().sum();
        result.push(sum);
    }
    Ok(ValueWord::from_float_array(Arc::new(
        AlignedTypedBuffer::from_aligned(result),
    )))
}

/// mat.colSum() -> FloatArray
pub fn handle_col_sum(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let m = extract_matrix(&args[0])?;
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
    Ok(ValueWord::from_float_array(Arc::new(
        AlignedTypedBuffer::from_aligned(result),
    )))
}

// ---------------------------------------------------------------------------
// MethodFnV2 handlers (v2 ABI: fn(&mut VM, &[u64], ctx) -> Result<u64>)
// ---------------------------------------------------------------------------

/// mat.transpose() -> Matrix (v2)
pub fn v2_transpose(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_vw(args[0]);
    let m = extract_matrix_vw(&vw)?;
    let result = matrix_kernels::matrix_transpose(m);
    Ok(ValueWord::from_matrix(Arc::new(result)).into_raw_bits())
}

/// mat.inverse() -> Matrix (v2)
pub fn v2_inverse(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_vw(args[0]);
    let m = extract_matrix_vw(&vw)?;
    let result = matrix_kernels::matrix_inverse(m).map_err(|e| VMError::RuntimeError(e))?;
    Ok(ValueWord::from_matrix(Arc::new(result)).into_raw_bits())
}

/// mat.det() / mat.determinant() -> number (v2)
pub fn v2_determinant(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_vw(args[0]);
    let m = extract_matrix_vw(&vw)?;
    let result = matrix_kernels::matrix_determinant(m).map_err(|e| VMError::RuntimeError(e))?;
    Ok(ValueWord::from_f64(result).into_raw_bits())
}

/// mat.trace() -> number (v2)
pub fn v2_trace(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_vw(args[0]);
    let m = extract_matrix_vw(&vw)?;
    let result = matrix_kernels::matrix_trace(m).map_err(|e| VMError::RuntimeError(e))?;
    Ok(ValueWord::from_f64(result).into_raw_bits())
}

/// mat.shape() -> [rows, cols] (v2)
pub fn v2_shape(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_vw(args[0]);
    let m = extract_matrix_vw(&vw)?;
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
    let vw = borrow_vw(args[0]);
    let m = extract_matrix_vw(&vw)?;

    let rows_vw = args.get(1).map(|&r| borrow_vw(r));
    let new_rows = rows_vw
        .as_ref()
        .and_then(|nb| nb.as_number_coerce())
        .ok_or_else(|| VMError::RuntimeError("reshape requires rows argument".to_string()))?
        as u32;

    let cols_vw = args.get(2).map(|&r| borrow_vw(r));
    let new_cols = cols_vw
        .as_ref()
        .and_then(|nb| nb.as_number_coerce())
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
    let vw = borrow_vw(args[0]);
    let m = extract_matrix_vw(&vw)?;

    let idx_vw = args.get(1).map(|&r| borrow_vw(r));
    let i = idx_vw
        .as_ref()
        .and_then(|nb| nb.as_number_coerce())
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
    let parent_arc = match vw.as_heap_ref() {
        Some(shape_value::heap_value::HeapValue::Matrix(arc)) => arc.clone(),
        _ => unreachable!("extract_matrix_vw succeeded so this must be Matrix"),
    };

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
    let vw = borrow_vw(args[0]);
    let m = extract_matrix_vw(&vw)?;

    let idx_vw = args.get(1).map(|&r| borrow_vw(r));
    let j = idx_vw
        .as_ref()
        .and_then(|nb| nb.as_number_coerce())
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
    let vw = borrow_vw(args[0]);
    let m = extract_matrix_vw(&vw)?;
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
    let vw = borrow_vw(args[0]);
    let m = extract_matrix_vw(&vw)?;
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
    let vw = borrow_vw(args[0]);
    let m = extract_matrix_vw(&vw)?;
    let sum: f64 = m.data.iter().sum();
    Ok(ValueWord::from_f64(sum).into_raw_bits())
}

/// mat.min() -> number (v2)
pub fn v2_min(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_vw(args[0]);
    let m = extract_matrix_vw(&vw)?;
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
    let vw = borrow_vw(args[0]);
    let m = extract_matrix_vw(&vw)?;
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
    let vw = borrow_vw(args[0]);
    let m = extract_matrix_vw(&vw)?;
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
    let vw = borrow_vw(args[0]);
    let m = extract_matrix_vw(&vw)?;
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
    let vw = borrow_vw(args[0]);
    let m = extract_matrix_vw(&vw)?;
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

pub(crate) fn handle_map(
    vm: &mut crate::executor::VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, shape_value::VMError> {
    let vw_args = args_to_vw(args);
    let result = handle_map_legacy(vm, vw_args, _ctx)?;
    Ok(result.into_raw_bits())
}
