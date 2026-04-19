//! Matrix intrinsics - dense numeric matrix multiplication kernels.
//!
//! Matrices are represented as `Vec<Vec<number>>` at runtime.
//! This module validates matrix shape once, flattens to contiguous row-major
//! buffers, and runs tight numeric kernels.

use super::{extract_f64_array, f64_vec_to_nb_array};
use crate::context::ExecutionContext;
use crate::intrinsics::matrix_kernels;
use shape_ast::error::{Result, ShapeError};
use shape_value::aligned_vec::AlignedVec;
use shape_value::heap_value::MatrixData;
use shape_value::{ValueWord, ValueWordExt};
use std::sync::Arc;

fn extract_matrix_f64(nb: &ValueWord, label: &str) -> Result<(Vec<f64>, usize, usize)> {
    let rows_view = nb.as_any_array().ok_or_else(|| ShapeError::RuntimeError {
        message: format!("{} must be a matrix (array of numeric arrays)", label),
        location: None,
    })?;

    if rows_view.is_empty() {
        return Ok((Vec::new(), 0, 0));
    }

    let rows = rows_view.to_generic();
    let num_rows = rows.len();
    let mut cols: Option<usize> = None;
    let mut flat = Vec::new();

    for (row_idx, row_nb) in rows.iter().enumerate() {
        let row_view = row_nb
            .as_any_array()
            .ok_or_else(|| ShapeError::RuntimeError {
                message: format!(
                    "{} row {} must be an array of numeric values",
                    label, row_idx
                ),
                location: None,
            })?;

        let row_len = row_view.len();
        match cols {
            Some(expected) if row_len != expected => {
                return Err(ShapeError::RuntimeError {
                    message: format!(
                        "{} has non-rectangular rows: expected {}, got {} at row {}",
                        label, expected, row_len, row_idx
                    ),
                    location: None,
                });
            }
            None => cols = Some(row_len),
            _ => {}
        }

        if let Some(slice) = row_view.as_f64_slice() {
            flat.extend_from_slice(slice);
        } else if let Some(slice) = row_view.as_i64_slice() {
            flat.extend(slice.iter().map(|&v| v as f64));
        } else {
            let row = row_view.to_generic();
            for value in row.iter() {
                let n = value
                    .as_number_coerce()
                    .ok_or_else(|| ShapeError::RuntimeError {
                        message: format!("{} must contain only numeric values", label),
                        location: None,
                    })?;
                flat.push(n);
            }
        }
    }

    Ok((flat, num_rows, cols.unwrap_or(0)))
}

fn matrix_to_nb(flat: &[f64], rows: usize, cols: usize) -> ValueWord {
    if rows == 0 {
        return ValueWord::from_array(shape_value::vmarray_from_vec(Vec::new()));
    }
    let mut out_rows = Vec::with_capacity(rows);
    for i in 0..rows {
        let base = i * cols;
        let row = (0..cols)
            .map(|j| ValueWord::from_f64(flat[base + j]))
            .collect::<Vec<_>>();
        out_rows.push(ValueWord::from_array(shape_value::vmarray_from_vec(row)));
    }
    ValueWord::from_array(shape_value::vmarray_from_vec(out_rows))
}

/// Core matrix-vector multiplication: `Mat<number> * Vec<number> -> Vec<number>`.
pub fn intrinsic_matmul_vec(args: &[ValueWord], _ctx: &mut ExecutionContext) -> Result<ValueWord> {
    if args.len() != 2 {
        return Err(ShapeError::RuntimeError {
            message: "matmul_vec requires 2 arguments".into(),
            location: None,
        });
    }

    let (a, rows, inner) = extract_matrix_f64(&args[0], "Left matrix")?;
    let b = extract_f64_array(&args[1], "Right vector")?;
    if inner != b.len() {
        return Err(ShapeError::RuntimeError {
            message: format!(
                "Matrix/vector dimension mismatch: matrix is {}x{}, vector is length {}",
                rows,
                inner,
                b.len()
            ),
            location: None,
        });
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

    Ok(f64_vec_to_nb_array(out))
}

/// Core matrix-matrix multiplication: `Mat<number> * Mat<number> -> Mat<number>`.
pub fn intrinsic_matmul_mat(args: &[ValueWord], _ctx: &mut ExecutionContext) -> Result<ValueWord> {
    if args.len() != 2 {
        return Err(ShapeError::RuntimeError {
            message: "matmul_mat requires 2 arguments".into(),
            location: None,
        });
    }

    let (a, a_rows, a_cols) = extract_matrix_f64(&args[0], "Left matrix")?;
    let (b, b_rows, b_cols) = extract_matrix_f64(&args[1], "Right matrix")?;
    if a_cols != b_rows {
        return Err(ShapeError::RuntimeError {
            message: format!(
                "Matrix dimension mismatch: left is {}x{}, right is {}x{}",
                a_rows, a_cols, b_rows, b_cols
            ),
            location: None,
        });
    }

    if a_rows == 0 || b_cols == 0 {
        return Ok(matrix_to_nb(&[], a_rows, b_cols));
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

    Ok(matrix_to_nb(&out, a_rows, b_cols))
}

/// Build a `MatrixData` from the nested-array representation produced
/// by R5.4B's `Mat<number>` literals. Shared by `intrinsic_mat_add` and
/// `intrinsic_mat_sub` so their dimension-check error paths stay
/// identical to `intrinsic_matmul_mat`.
fn matrix_data_from_nb(nb: &ValueWord, label: &str) -> Result<MatrixData> {
    let (flat, rows, cols) = extract_matrix_f64(nb, label)?;
    let aligned = if flat.is_empty() {
        AlignedVec::new()
    } else {
        AlignedVec::from_vec(flat)
    };
    Ok(MatrixData::from_flat(aligned, rows as u32, cols as u32))
}

/// Convert a kernel-produced `MatrixData` back to the nested-array
/// shape that post-R5.4B `Mat<number>` literals produce. Keeps input
/// and output representations symmetric so downstream method dispatch
/// doesn't diverge between the fallback and intrinsic paths.
fn matrix_data_to_nb(mat: &MatrixData) -> ValueWord {
    matrix_to_nb(mat.data.as_slice(), mat.rows as usize, mat.cols as usize)
}

/// Core element-wise matrix addition: `Mat<number> + Mat<number>` (R5.4D).
///
/// Dispatches to `matrix_kernels::matrix_add` after extracting the
/// nested-array inputs via `extract_matrix_f64`. Shape errors (non-
/// rectangular / non-numeric rows) come from the extractor; dimension
/// mismatch surfaces from the kernel. Result is the nested-array
/// representation produced by R5.4B's `Mat<number>` literals.
pub fn intrinsic_mat_add(args: &[ValueWord], _ctx: &mut ExecutionContext) -> Result<ValueWord> {
    if args.len() != 2 {
        return Err(ShapeError::RuntimeError {
            message: "mat_add requires 2 arguments".into(),
            location: None,
        });
    }
    let a = matrix_data_from_nb(&args[0], "Left matrix")?;
    let b = matrix_data_from_nb(&args[1], "Right matrix")?;
    let out =
        matrix_kernels::matrix_add(&a, &b).map_err(|msg| ShapeError::RuntimeError {
            message: msg,
            location: None,
        })?;
    Ok(matrix_data_to_nb(&out))
}

/// Core element-wise matrix subtraction: `Mat<number> - Mat<number>` (R5.4D).
/// Companion to `intrinsic_mat_add`, dispatching to
/// `matrix_kernels::matrix_sub` with the same input / output shape
/// contract.
pub fn intrinsic_mat_sub(args: &[ValueWord], _ctx: &mut ExecutionContext) -> Result<ValueWord> {
    if args.len() != 2 {
        return Err(ShapeError::RuntimeError {
            message: "mat_sub requires 2 arguments".into(),
            location: None,
        });
    }
    let a = matrix_data_from_nb(&args[0], "Left matrix")?;
    let b = matrix_data_from_nb(&args[1], "Right matrix")?;
    let out =
        matrix_kernels::matrix_sub(&a, &b).map_err(|msg| ShapeError::RuntimeError {
            message: msg,
            location: None,
        })?;
    Ok(matrix_data_to_nb(&out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::ExecutionContext;

    fn nb_vec(values: &[f64]) -> ValueWord {
        ValueWord::from_array(shape_value::vmarray_from_value_words(
            values.iter().copied().map(ValueWord::from_f64),
        ))
    }

    fn nb_mat(rows: &[&[f64]]) -> ValueWord {
        let out = rows
            .iter()
            .map(|row| nb_vec(row))
            .collect::<Vec<ValueWord>>();
        ValueWord::from_array(shape_value::vmarray_from_vec(out))
    }

    fn unwrap_vec(nb: ValueWord) -> Vec<f64> {
        let arr = nb.as_any_array().expect("expected array").to_generic();
        arr.iter()
            .map(|x| x.as_number_coerce().expect("expected number"))
            .collect()
    }

    fn unwrap_mat(nb: ValueWord) -> Vec<Vec<f64>> {
        let outer = nb.as_any_array().expect("expected matrix").to_generic();
        outer
            .iter()
            .map(|row| {
                let inner = row.as_any_array().expect("expected row").to_generic();
                inner
                    .iter()
                    .map(|x| x.as_number_coerce().expect("expected number"))
                    .collect()
            })
            .collect()
    }

    #[test]
    fn test_matmul_vec() {
        let a = nb_mat(&[&[1.0, 2.0], &[3.0, 4.0]]);
        let b = nb_vec(&[5.0, 6.0]);
        let mut ctx = ExecutionContext::new_empty();
        let out = intrinsic_matmul_vec(&[a, b], &mut ctx).expect("matmul_vec");
        assert_eq!(unwrap_vec(out), vec![17.0, 39.0]);
    }

    #[test]
    fn test_matmul_mat() {
        let a = nb_mat(&[&[1.0, 2.0], &[3.0, 4.0]]);
        let b = nb_mat(&[&[5.0, 6.0], &[7.0, 8.0]]);
        let mut ctx = ExecutionContext::new_empty();
        let out = intrinsic_matmul_mat(&[a, b], &mut ctx).expect("matmul_mat");
        assert_eq!(unwrap_mat(out), vec![vec![19.0, 22.0], vec![43.0, 50.0]]);
    }

    // ===== R5.4D: intrinsic_mat_add / intrinsic_mat_sub =====

    #[test]
    fn test_r5_4d_intrinsic_mat_add_happy_path() {
        let a = nb_mat(&[&[1.0, 2.0], &[3.0, 4.0]]);
        let b = nb_mat(&[&[10.0, 20.0], &[30.0, 40.0]]);
        let mut ctx = ExecutionContext::new_empty();
        let out = intrinsic_mat_add(&[a, b], &mut ctx).expect("mat_add");
        assert_eq!(unwrap_mat(out), vec![vec![11.0, 22.0], vec![33.0, 44.0]]);
    }

    #[test]
    fn test_r5_4d_intrinsic_mat_add_dimension_mismatch() {
        let a = nb_mat(&[&[1.0, 2.0], &[3.0, 4.0]]);
        let b = nb_mat(&[&[1.0, 2.0, 3.0]]);
        let mut ctx = ExecutionContext::new_empty();
        let err = intrinsic_mat_add(&[a, b], &mut ctx).unwrap_err();
        let msg = format!("{:?}", err);
        assert!(
            msg.contains("dimension mismatch") || msg.contains("Matrix"),
            "got: {}",
            msg
        );
    }

    #[test]
    fn test_r5_4d_intrinsic_mat_sub_happy_path() {
        let a = nb_mat(&[&[10.0, 20.0], &[30.0, 40.0]]);
        let b = nb_mat(&[&[1.0, 2.0], &[3.0, 4.0]]);
        let mut ctx = ExecutionContext::new_empty();
        let out = intrinsic_mat_sub(&[a, b], &mut ctx).expect("mat_sub");
        assert_eq!(unwrap_mat(out), vec![vec![9.0, 18.0], vec![27.0, 36.0]]);
    }

    #[test]
    fn test_r5_4d_intrinsic_mat_sub_dimension_mismatch() {
        let a = nb_mat(&[&[1.0, 2.0], &[3.0, 4.0]]);
        let b = nb_mat(&[&[1.0], &[2.0]]);
        let mut ctx = ExecutionContext::new_empty();
        let err = intrinsic_mat_sub(&[a, b], &mut ctx).unwrap_err();
        let msg = format!("{:?}", err);
        assert!(
            msg.contains("dimension mismatch") || msg.contains("Matrix"),
            "got: {}",
            msg
        );
    }

    #[test]
    fn test_r5_4d_intrinsic_mat_add_arity_error() {
        let a = nb_mat(&[&[1.0]]);
        let mut ctx = ExecutionContext::new_empty();
        let err = intrinsic_mat_add(&[a], &mut ctx).unwrap_err();
        let msg = format!("{:?}", err);
        assert!(msg.contains("2 arguments"), "got: {}", msg);
    }
}
