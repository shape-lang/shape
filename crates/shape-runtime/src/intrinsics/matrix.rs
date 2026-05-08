//! Matrix intrinsics — full migration to typed marshal layer.
//!
//! Per the intrinsics-typed-CC migration's per-file table, all 4 matrix
//! intrinsics (`matmul_vec`, `matmul_mat`, `mat_add`, `mat_sub`) migrate to
//! `register_typed_fn_2` typed entries via [`create_matrix_intrinsics_module`].
//!
//! Inputs use the existing Phase 2d Array `Vec<Arc<HeapValue>>` FromSlot for
//! nested `Array<Array<number>>` matrix arguments and `Arc<AlignedTypedBuffer>`
//! for flat `Array<number>` vector arguments. Outputs project through
//! `ConcreteReturn::ArrayHeapValue(Vec<Arc<HeapValue>>)` for nested-array
//! returns (Phase 2d Array landed; production-active per arrow_module /
//! csv_module / process_ops migrations) and `ConcreteReturn::ArrayF64` for
//! flat returns.
//!
//! Body-side row extraction goes via direct `Arc<HeapValue>` pattern-match
//! (mirrors `Arc<DataTable>`'s shape at `marshal.rs:200-217` and Dev 2's
//! HashMap-marshal P1(b) body access pattern). Each row extracts to a
//! borrowed `&[f64]` if `HeapValue::TypedArray(TypedArrayData::F64(buf))`,
//! widens i64 to f64 if `TypedArrayData::I64(...)`, otherwise rejects with
//! a marshal-contract error.
//!
//! Matrices are represented as `Array<Array<number>>` at runtime.
//! This module validates matrix shape once, flattens to contiguous row-major
//! buffers, and runs tight numeric kernels.

use crate::intrinsics::matrix_kernels;
use crate::marshal::register_typed_fn_2;
use crate::module_exports::ModuleExports;
use crate::typed_module_exports::{ConcreteReturn, ConcreteType, TypedReturn};
use shape_value::aligned_vec::AlignedVec;
use shape_value::heap_value::{HeapValue, MatrixData, TypedArrayData};
use shape_value::{AlignedTypedBuffer, TypedBuffer};
use std::sync::Arc;

// ───────────────────── Module factory (4 typed entries) ─────────────────────

/// Create the matrix intrinsics module with all 4 typed-marshal entry points.
pub fn create_matrix_intrinsics_module() -> ModuleExports {
    let mut module = ModuleExports::new("std::core::intrinsics::matrix");
    module.description =
        "Matrix intrinsics (matmul_vec, matmul_mat, mat_add, mat_sub)".to_string();

    register_typed_fn_2::<_, Vec<Arc<HeapValue>>, Arc<AlignedTypedBuffer>>(
        &mut module,
        "__intrinsic_matmul_vec",
        "Matrix-vector multiplication: `Mat<number> * Vec<number> -> Vec<number>`",
        [("matrix", "Array<Array<number>>"), ("vector", "Array<number>")],
        ConcreteType::ArrayNumber,
        |matrix, vector, _ctx| {
            let (a, rows, inner) = extract_matrix(&matrix, "Left matrix")?;
            let b = vector.as_slice();
            if inner != b.len() {
                return Err(format!(
                    "Matrix/vector dimension mismatch: matrix is {}x{}, vector is length {}",
                    rows,
                    inner,
                    b.len()
                ));
            }
            let mut out = vec![0.0; rows];
            for i in 0..rows {
                let row_base = i * inner;
                let mut acc = 0.0;
                for k in 0..inner {
                    acc += a[row_base + k] * b[k];
                }
                out[i] = acc;
            }
            Ok(TypedReturn::Concrete(ConcreteReturn::ArrayF64(out)))
        },
    );

    register_typed_fn_2::<_, Vec<Arc<HeapValue>>, Vec<Arc<HeapValue>>>(
        &mut module,
        "__intrinsic_matmul_mat",
        "Matrix-matrix multiplication: `Mat<number> * Mat<number> -> Mat<number>`",
        [
            ("a", "Array<Array<number>>"),
            ("b", "Array<Array<number>>"),
        ],
        ConcreteType::ArrayHeapValue("Array<Array<number>>".to_string()),
        |a_rows_arc, b_rows_arc, _ctx| {
            let (a, a_rows, a_cols) = extract_matrix(&a_rows_arc, "Left matrix")?;
            let (b, b_rows, b_cols) = extract_matrix(&b_rows_arc, "Right matrix")?;
            if a_cols != b_rows {
                return Err(format!(
                    "Matrix dimension mismatch: left is {}x{}, right is {}x{}",
                    a_rows, a_cols, b_rows, b_cols
                ));
            }
            if a_rows == 0 || b_cols == 0 {
                return Ok(TypedReturn::Concrete(ConcreteReturn::ArrayHeapValue(
                    matrix_to_heap_value_vec(&[], a_rows, b_cols),
                )));
            }
            let mut out = vec![0.0; a_rows * b_cols];
            for i in 0..a_rows {
                let a_row_base = i * a_cols;
                let out_row_base = i * b_cols;
                for k in 0..a_cols {
                    let a_ik = a[a_row_base + k];
                    let b_row_base = k * b_cols;
                    for j in 0..b_cols {
                        out[out_row_base + j] += a_ik * b[b_row_base + j];
                    }
                }
            }
            Ok(TypedReturn::Concrete(ConcreteReturn::ArrayHeapValue(
                matrix_to_heap_value_vec(&out, a_rows, b_cols),
            )))
        },
    );

    register_typed_fn_2::<_, Vec<Arc<HeapValue>>, Vec<Arc<HeapValue>>>(
        &mut module,
        "__intrinsic_mat_add",
        "Element-wise matrix addition: `Mat<number> + Mat<number>`",
        [
            ("a", "Array<Array<number>>"),
            ("b", "Array<Array<number>>"),
        ],
        ConcreteType::ArrayHeapValue("Array<Array<number>>".to_string()),
        |a_rows_arc, b_rows_arc, _ctx| {
            let a = matrix_data_from_heap_value_vec(&a_rows_arc, "Left matrix")?;
            let b = matrix_data_from_heap_value_vec(&b_rows_arc, "Right matrix")?;
            let out = matrix_kernels::matrix_add(&a, &b)?;
            Ok(TypedReturn::Concrete(ConcreteReturn::ArrayHeapValue(
                matrix_data_to_heap_value_vec(&out),
            )))
        },
    );

    register_typed_fn_2::<_, Vec<Arc<HeapValue>>, Vec<Arc<HeapValue>>>(
        &mut module,
        "__intrinsic_mat_sub",
        "Element-wise matrix subtraction: `Mat<number> - Mat<number>`",
        [
            ("a", "Array<Array<number>>"),
            ("b", "Array<Array<number>>"),
        ],
        ConcreteType::ArrayHeapValue("Array<Array<number>>".to_string()),
        |a_rows_arc, b_rows_arc, _ctx| {
            let a = matrix_data_from_heap_value_vec(&a_rows_arc, "Left matrix")?;
            let b = matrix_data_from_heap_value_vec(&b_rows_arc, "Right matrix")?;
            let out = matrix_kernels::matrix_sub(&a, &b)?;
            Ok(TypedReturn::Concrete(ConcreteReturn::ArrayHeapValue(
                matrix_data_to_heap_value_vec(&out),
            )))
        },
    );

    module
}

// ───────────────────── Body-side helpers ─────────────────────

/// Extract a row-`&[f64]`-equivalent from a single `Arc<HeapValue>` row element.
///
/// Pattern-match shape mirrors `marshal.rs:200-217`'s `FromSlot for Arc<DataTable>`
/// and Dev 2's HashMap-marshal P1(b) body access. Returns owned `Vec<f64>` if
/// the row is `TypedArrayData::I64` (widen needed) or borrowed-then-cloned
/// `Vec<f64>` if `TypedArrayData::F64`. Reject other variants per marshal
/// contract.
fn row_to_f64_vec(hv: &Arc<HeapValue>, label: &str, row_idx: usize) -> Result<Vec<f64>, String> {
    match &**hv {
        HeapValue::TypedArray(arc) => match &**arc {
            TypedArrayData::F64(buf) => Ok(buf.as_slice().to_vec()),
            TypedArrayData::I64(buf) => {
                Ok(buf.as_slice().iter().map(|&v| v as f64).collect())
            }
            other => Err(format!(
                "{} row {} must be a numeric array; got TypedArray::{}",
                label,
                row_idx,
                other.type_name()
            )),
        },
        other => Err(format!(
            "{} row {} must be an array of numeric values; got {:?}",
            label,
            row_idx,
            other.kind()
        )),
    }
}

/// Walk a `Vec<Arc<HeapValue>>` of rows; produce a flat row-major `Vec<f64>`
/// + dimensions. Validates rectangularity and rejects non-numeric rows.
fn extract_matrix(
    rows: &[Arc<HeapValue>],
    label: &str,
) -> Result<(Vec<f64>, usize, usize), String> {
    if rows.is_empty() {
        return Ok((Vec::new(), 0, 0));
    }
    let mut cols: Option<usize> = None;
    let mut flat = Vec::new();
    for (row_idx, hv) in rows.iter().enumerate() {
        let row = row_to_f64_vec(hv, label, row_idx)?;
        match cols {
            Some(expected) if row.len() != expected => {
                return Err(format!(
                    "{} has non-rectangular rows: expected {}, got {} at row {}",
                    label,
                    expected,
                    row.len(),
                    row_idx
                ));
            }
            None => cols = Some(row.len()),
            _ => {}
        }
        flat.extend_from_slice(&row);
    }
    Ok((flat, rows.len(), cols.unwrap_or(0)))
}

/// Build a `MatrixData` from the nested `Vec<Arc<HeapValue>>` row representation.
/// Used by `mat_add` / `mat_sub` so their dimension-check error paths stay
/// identical to `matmul_mat`.
fn matrix_data_from_heap_value_vec(
    rows: &[Arc<HeapValue>],
    label: &str,
) -> Result<MatrixData, String> {
    let (flat, num_rows, cols) = extract_matrix(rows, label)?;
    let aligned = if flat.is_empty() {
        AlignedVec::new()
    } else {
        AlignedVec::from_vec(flat)
    };
    Ok(MatrixData::from_flat(aligned, num_rows as u32, cols as u32))
}

/// Convert a flat row-major `&[f64]` of dimensions `rows`x`cols` into
/// `Vec<Arc<HeapValue>>` rows where each inner element is
/// `HeapValue::TypedArray(TypedArrayData::F64(...))`.
fn matrix_to_heap_value_vec(flat: &[f64], rows: usize, cols: usize) -> Vec<Arc<HeapValue>> {
    if rows == 0 {
        return Vec::new();
    }
    let mut out_rows = Vec::with_capacity(rows);
    for i in 0..rows {
        let base = i * cols;
        let row_data: Vec<f64> = flat[base..base + cols].to_vec();
        let buf = AlignedTypedBuffer::from(AlignedVec::from_vec(row_data));
        let data = Arc::new(TypedArrayData::F64(Arc::new(buf)));
        out_rows.push(Arc::new(HeapValue::TypedArray(data)));
    }
    out_rows
}

/// Convert a kernel-produced `MatrixData` back to the nested-array
/// representation.
fn matrix_data_to_heap_value_vec(mat: &MatrixData) -> Vec<Arc<HeapValue>> {
    matrix_to_heap_value_vec(mat.data.as_slice(), mat.rows as usize, mat.cols as usize)
}

// Suppress unused-import warnings for TypedBuffer (kept for forward consistency
// with potential i64-buffer construction in matrix_to_heap_value_vec variants).
#[allow(dead_code)]
type _TypedBufferI64 = TypedBuffer<i64>;
