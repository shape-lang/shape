//! SIMD-accelerated matrix kernels operating on `MatrixData`.
//!
//! All functions operate directly on `MatrixData` (row-major, SIMD-aligned)
//! to avoid the overhead of nested-array extraction in the old `matrix.rs`.

use shape_value::aligned_vec::AlignedVec;
use shape_value::heap_value::MatrixData;
use wide::f64x4;

const SIMD_THRESHOLD: usize = 16;

/// Element-wise matrix addition: C = A + B (same dimensions).
pub fn matrix_add(a: &MatrixData, b: &MatrixData) -> Result<MatrixData, String> {
    if a.rows != b.rows || a.cols != b.cols {
        return Err(format!(
            "Matrix dimension mismatch for add: {}x{} vs {}x{}",
            a.rows, a.cols, b.rows, b.cols
        ));
    }
    let len = a.data.len();
    let mut result = AlignedVec::with_capacity(len);

    if len >= SIMD_THRESHOLD {
        let chunks = len / 4;
        let a_ptr = a.data.as_ptr();
        let b_ptr = b.data.as_ptr();
        for i in 0..chunks {
            let offset = i * 4;
            let va = f64x4::from(unsafe { *(a_ptr.add(offset) as *const [f64; 4]) });
            let vb = f64x4::from(unsafe { *(b_ptr.add(offset) as *const [f64; 4]) });
            let vc = va + vb;
            let arr: [f64; 4] = vc.into();
            for v in arr {
                result.push(v);
            }
        }
        for i in (chunks * 4)..len {
            result.push(a.data[i] + b.data[i]);
        }
    } else {
        for i in 0..len {
            result.push(a.data[i] + b.data[i]);
        }
    }

    Ok(MatrixData::from_flat(result, a.rows, a.cols))
}

/// Element-wise matrix subtraction: C = A - B (same dimensions).
pub fn matrix_sub(a: &MatrixData, b: &MatrixData) -> Result<MatrixData, String> {
    if a.rows != b.rows || a.cols != b.cols {
        return Err(format!(
            "Matrix dimension mismatch for sub: {}x{} vs {}x{}",
            a.rows, a.cols, b.rows, b.cols
        ));
    }
    let len = a.data.len();
    let mut result = AlignedVec::with_capacity(len);

    if len >= SIMD_THRESHOLD {
        let chunks = len / 4;
        let a_ptr = a.data.as_ptr();
        let b_ptr = b.data.as_ptr();
        for i in 0..chunks {
            let offset = i * 4;
            let va = f64x4::from(unsafe { *(a_ptr.add(offset) as *const [f64; 4]) });
            let vb = f64x4::from(unsafe { *(b_ptr.add(offset) as *const [f64; 4]) });
            let vc = va - vb;
            let arr: [f64; 4] = vc.into();
            for v in arr {
                result.push(v);
            }
        }
        for i in (chunks * 4)..len {
            result.push(a.data[i] - b.data[i]);
        }
    } else {
        for i in 0..len {
            result.push(a.data[i] - b.data[i]);
        }
    }

    Ok(MatrixData::from_flat(result, a.rows, a.cols))
}

/// Scalar multiplication: C = A * scalar.
pub fn matrix_scale(a: &MatrixData, scalar: f64) -> MatrixData {
    let len = a.data.len();
    let mut result = AlignedVec::with_capacity(len);

    if len >= SIMD_THRESHOLD {
        let chunks = len / 4;
        let s = f64x4::splat(scalar);
        let a_ptr = a.data.as_ptr();
        for i in 0..chunks {
            let offset = i * 4;
            let va = f64x4::from(unsafe { *(a_ptr.add(offset) as *const [f64; 4]) });
            let vc = va * s;
            let arr: [f64; 4] = vc.into();
            for v in arr {
                result.push(v);
            }
        }
        for i in (chunks * 4)..len {
            result.push(a.data[i] * scalar);
        }
    } else {
        for i in 0..len {
            result.push(a.data[i] * scalar);
        }
    }

    MatrixData::from_flat(result, a.rows, a.cols)
}

/// Element-wise (Hadamard) multiplication: C[i,j] = A[i,j] * B[i,j].
pub fn matrix_element_mul(a: &MatrixData, b: &MatrixData) -> Result<MatrixData, String> {
    if a.rows != b.rows || a.cols != b.cols {
        return Err(format!(
            "Matrix dimension mismatch for element-wise mul: {}x{} vs {}x{}",
            a.rows, a.cols, b.rows, b.cols
        ));
    }
    let len = a.data.len();
    let mut result = AlignedVec::with_capacity(len);

    if len >= SIMD_THRESHOLD {
        let chunks = len / 4;
        let a_ptr = a.data.as_ptr();
        let b_ptr = b.data.as_ptr();
        for i in 0..chunks {
            let offset = i * 4;
            let va = f64x4::from(unsafe { *(a_ptr.add(offset) as *const [f64; 4]) });
            let vb = f64x4::from(unsafe { *(b_ptr.add(offset) as *const [f64; 4]) });
            let vc = va * vb;
            let arr: [f64; 4] = vc.into();
            for v in arr {
                result.push(v);
            }
        }
        for i in (chunks * 4)..len {
            result.push(a.data[i] * b.data[i]);
        }
    } else {
        for i in 0..len {
            result.push(a.data[i] * b.data[i]);
        }
    }

    Ok(MatrixData::from_flat(result, a.rows, a.cols))
}

/// Matrix multiplication: C = A * B.
/// A is (m x k), B is (k x n), result is (m x n).
pub fn matrix_matmul(a: &MatrixData, b: &MatrixData) -> Result<MatrixData, String> {
    if a.cols != b.rows {
        return Err(format!(
            "Matrix dimension mismatch for matmul: {}x{} * {}x{}",
            a.rows, a.cols, b.rows, b.cols
        ));
    }
    let m = a.rows as usize;
    let k = a.cols as usize;
    let n = b.cols as usize;
    let mut result = AlignedVec::with_capacity(m * n);
    for _ in 0..(m * n) {
        result.push(0.0);
    }

    // ikj loop order for better cache behavior
    for i in 0..m {
        let a_row_base = i * k;
        let out_row_base = i * n;
        for kk in 0..k {
            let a_ik = a.data[a_row_base + kk];
            let b_row_base = kk * n;
            if n >= SIMD_THRESHOLD {
                let chunks = n / 4;
                let sa = f64x4::splat(a_ik);
                for j in 0..chunks {
                    let offset = j * 4;
                    let vb = f64x4::from(unsafe {
                        *(b.data.as_ptr().add(b_row_base + offset) as *const [f64; 4])
                    });
                    let vc = f64x4::from(unsafe {
                        *(result.as_ptr().add(out_row_base + offset) as *const [f64; 4])
                    });
                    let vr = vc + sa * vb;
                    let arr: [f64; 4] = vr.into();
                    for (idx, v) in arr.iter().enumerate() {
                        result[out_row_base + offset + idx] = *v;
                    }
                }
                for j in (chunks * 4)..n {
                    result[out_row_base + j] += a_ik * b.data[b_row_base + j];
                }
            } else {
                for j in 0..n {
                    result[out_row_base + j] += a_ik * b.data[b_row_base + j];
                }
            }
        }
    }

    Ok(MatrixData::from_flat(result, a.rows as u32, b.cols as u32))
}

/// Matrix-vector multiplication: y = A * v.
/// A is (m x n), v has length n, result has length m.
pub fn matrix_matvec(a: &MatrixData, v: &[f64]) -> Result<AlignedVec<f64>, String> {
    let n = a.cols as usize;
    if n != v.len() {
        return Err(format!(
            "Matrix-vector dimension mismatch: {}x{} * vec({})",
            a.rows,
            a.cols,
            v.len()
        ));
    }
    let m = a.rows as usize;
    let mut result = AlignedVec::with_capacity(m);

    for i in 0..m {
        let row_base = i * n;
        let mut acc = 0.0;
        if n >= SIMD_THRESHOLD {
            let chunks = n / 4;
            let mut vacc = f64x4::splat(0.0);
            for j in 0..chunks {
                let offset = j * 4;
                let va = f64x4::from(unsafe {
                    *(a.data.as_ptr().add(row_base + offset) as *const [f64; 4])
                });
                let vv = f64x4::from(unsafe { *(v.as_ptr().add(offset) as *const [f64; 4]) });
                vacc = vacc + va * vv;
            }
            let arr: [f64; 4] = vacc.into();
            acc = arr[0] + arr[1] + arr[2] + arr[3];
            for j in (chunks * 4)..n {
                acc += a.data[row_base + j] * v[j];
            }
        } else {
            for j in 0..n {
                acc += a.data[row_base + j] * v[j];
            }
        }
        result.push(acc);
    }

    Ok(result)
}

/// Matrix transpose: B = A^T.
pub fn matrix_transpose(m: &MatrixData) -> MatrixData {
    let rows = m.rows as usize;
    let cols = m.cols as usize;
    let mut result = AlignedVec::with_capacity(rows * cols);
    for _ in 0..(rows * cols) {
        result.push(0.0);
    }

    for i in 0..rows {
        for j in 0..cols {
            result[j * rows + i] = m.data[i * cols + j];
        }
    }

    MatrixData::from_flat(result, m.cols, m.rows)
}

/// Matrix inverse via Gauss-Jordan elimination.
/// Only works for square matrices.
pub fn matrix_inverse(m: &MatrixData) -> Result<MatrixData, String> {
    if m.rows != m.cols {
        return Err(format!(
            "Cannot invert non-square matrix: {}x{}",
            m.rows, m.cols
        ));
    }
    let n = m.rows as usize;
    if n == 0 {
        return Ok(MatrixData::new(0, 0));
    }

    // Build augmented matrix [A | I]
    let mut aug = vec![0.0f64; n * 2 * n];
    for i in 0..n {
        for j in 0..n {
            aug[i * 2 * n + j] = m.data[i * n + j];
        }
        aug[i * 2 * n + n + i] = 1.0;
    }

    // Forward elimination with partial pivoting
    for col in 0..n {
        // Find pivot
        let mut max_val = aug[col * 2 * n + col].abs();
        let mut max_row = col;
        for row in (col + 1)..n {
            let val = aug[row * 2 * n + col].abs();
            if val > max_val {
                max_val = val;
                max_row = row;
            }
        }

        if max_val < 1e-14 {
            return Err("Matrix is singular and cannot be inverted".to_string());
        }

        // Swap rows
        if max_row != col {
            for j in 0..(2 * n) {
                aug.swap(col * 2 * n + j, max_row * 2 * n + j);
            }
        }

        // Scale pivot row
        let pivot = aug[col * 2 * n + col];
        for j in 0..(2 * n) {
            aug[col * 2 * n + j] /= pivot;
        }

        // Eliminate column
        for row in 0..n {
            if row != col {
                let factor = aug[row * 2 * n + col];
                for j in 0..(2 * n) {
                    aug[row * 2 * n + j] -= factor * aug[col * 2 * n + j];
                }
            }
        }
    }

    // Extract inverse from right half
    let mut result = AlignedVec::with_capacity(n * n);
    for i in 0..n {
        for j in 0..n {
            result.push(aug[i * 2 * n + n + j]);
        }
    }

    Ok(MatrixData::from_flat(result, m.rows, m.cols))
}

/// Matrix determinant via LU decomposition (partial pivoting).
pub fn matrix_determinant(m: &MatrixData) -> Result<f64, String> {
    if m.rows != m.cols {
        return Err(format!(
            "Cannot compute determinant of non-square matrix: {}x{}",
            m.rows, m.cols
        ));
    }
    let n = m.rows as usize;
    if n == 0 {
        return Ok(1.0);
    }
    if n == 1 {
        return Ok(m.data[0]);
    }
    if n == 2 {
        return Ok(m.data[0] * m.data[3] - m.data[1] * m.data[2]);
    }

    // Work on a copy
    let mut a: Vec<f64> = m.data.iter().copied().collect();
    let mut det = 1.0f64;

    for col in 0..n {
        // Partial pivoting
        let mut max_val = a[col * n + col].abs();
        let mut max_row = col;
        for row in (col + 1)..n {
            let val = a[row * n + col].abs();
            if val > max_val {
                max_val = val;
                max_row = row;
            }
        }

        if max_val < 1e-14 {
            return Ok(0.0);
        }

        if max_row != col {
            for j in 0..n {
                a.swap(col * n + j, max_row * n + j);
            }
            det = -det;
        }

        det *= a[col * n + col];

        let pivot = a[col * n + col];
        for row in (col + 1)..n {
            let factor = a[row * n + col] / pivot;
            for j in (col + 1)..n {
                a[row * n + j] -= factor * a[col * n + j];
            }
        }
    }

    Ok(det)
}

/// Matrix trace: sum of diagonal elements.
pub fn matrix_trace(m: &MatrixData) -> Result<f64, String> {
    if m.rows != m.cols {
        return Err(format!(
            "Cannot compute trace of non-square matrix: {}x{}",
            m.rows, m.cols
        ));
    }
    let n = m.rows as usize;
    let mut sum = 0.0;
    for i in 0..n {
        sum += m.data[i * n + i];
    }
    Ok(sum)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mat(data: &[f64], rows: u32, cols: u32) -> MatrixData {
        let mut aligned = AlignedVec::with_capacity(data.len());
        for &v in data {
            aligned.push(v);
        }
        MatrixData::from_flat(aligned, rows, cols)
    }

    fn approx_eq(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-10
    }

    fn mat_approx_eq(a: &MatrixData, b: &MatrixData) -> bool {
        a.rows == b.rows
            && a.cols == b.cols
            && a.data
                .iter()
                .zip(b.data.iter())
                .all(|(x, y)| approx_eq(*x, *y))
    }

    #[test]
    fn test_matrix_add_2x2() {
        let a = mat(&[1.0, 2.0, 3.0, 4.0], 2, 2);
        let b = mat(&[5.0, 6.0, 7.0, 8.0], 2, 2);
        let c = matrix_add(&a, &b).unwrap();
        assert_eq!(c.data.as_slice(), &[6.0, 8.0, 10.0, 12.0]);
    }

    #[test]
    fn test_matrix_sub_2x2() {
        let a = mat(&[5.0, 6.0, 7.0, 8.0], 2, 2);
        let b = mat(&[1.0, 2.0, 3.0, 4.0], 2, 2);
        let c = matrix_sub(&a, &b).unwrap();
        assert_eq!(c.data.as_slice(), &[4.0, 4.0, 4.0, 4.0]);
    }

    #[test]
    fn test_matrix_scale() {
        let a = mat(&[1.0, 2.0, 3.0, 4.0], 2, 2);
        let c = matrix_scale(&a, 3.0);
        assert_eq!(c.data.as_slice(), &[3.0, 6.0, 9.0, 12.0]);
    }

    #[test]
    fn test_matrix_element_mul() {
        let a = mat(&[1.0, 2.0, 3.0, 4.0], 2, 2);
        let b = mat(&[5.0, 6.0, 7.0, 8.0], 2, 2);
        let c = matrix_element_mul(&a, &b).unwrap();
        assert_eq!(c.data.as_slice(), &[5.0, 12.0, 21.0, 32.0]);
    }

    #[test]
    fn test_matrix_matmul_2x2() {
        let a = mat(&[1.0, 2.0, 3.0, 4.0], 2, 2);
        let b = mat(&[5.0, 6.0, 7.0, 8.0], 2, 2);
        let c = matrix_matmul(&a, &b).unwrap();
        assert_eq!(c.data.as_slice(), &[19.0, 22.0, 43.0, 50.0]);
    }

    #[test]
    fn test_matrix_matmul_3x3() {
        let a = mat(&[1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0], 3, 3);
        let b = mat(&[2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0], 3, 3);
        let c = matrix_matmul(&a, &b).unwrap();
        assert_eq!(c.data.as_slice(), b.data.as_slice());
    }

    #[test]
    fn test_matrix_matmul_2x3_3x2() {
        let a = mat(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], 2, 3);
        let b = mat(&[7.0, 8.0, 9.0, 10.0, 11.0, 12.0], 3, 2);
        let c = matrix_matmul(&a, &b).unwrap();
        assert_eq!(c.rows, 2);
        assert_eq!(c.cols, 2);
        // [1*7+2*9+3*11, 1*8+2*10+3*12] = [58, 64]
        // [4*7+5*9+6*11, 4*8+5*10+6*12] = [139, 154]
        assert_eq!(c.data.as_slice(), &[58.0, 64.0, 139.0, 154.0]);
    }

    #[test]
    fn test_matrix_matvec() {
        let a = mat(&[1.0, 2.0, 3.0, 4.0], 2, 2);
        let v = [5.0, 6.0];
        let result = matrix_matvec(&a, &v).unwrap();
        assert_eq!(result.as_slice(), &[17.0, 39.0]);
    }

    #[test]
    fn test_matrix_transpose() {
        let a = mat(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], 2, 3);
        let t = matrix_transpose(&a);
        assert_eq!(t.rows, 3);
        assert_eq!(t.cols, 2);
        assert_eq!(t.data.as_slice(), &[1.0, 4.0, 2.0, 5.0, 3.0, 6.0]);
    }

    #[test]
    fn test_matrix_inverse_2x2() {
        let a = mat(&[4.0, 7.0, 2.0, 6.0], 2, 2);
        let inv = matrix_inverse(&a).unwrap();
        // Verify A * A^-1 = I
        let identity = matrix_matmul(&a, &inv).unwrap();
        assert!(approx_eq(identity.get(0, 0), 1.0));
        assert!(approx_eq(identity.get(0, 1), 0.0));
        assert!(approx_eq(identity.get(1, 0), 0.0));
        assert!(approx_eq(identity.get(1, 1), 1.0));
    }

    #[test]
    fn test_matrix_inverse_3x3() {
        let a = mat(&[1.0, 2.0, 3.0, 0.0, 1.0, 4.0, 5.0, 6.0, 0.0], 3, 3);
        let inv = matrix_inverse(&a).unwrap();
        let identity = matrix_matmul(&a, &inv).unwrap();
        for i in 0..3u32 {
            for j in 0..3u32 {
                let expected = if i == j { 1.0 } else { 0.0 };
                assert!(
                    approx_eq(identity.get(i, j), expected),
                    "identity[{},{}] = {} (expected {})",
                    i,
                    j,
                    identity.get(i, j),
                    expected
                );
            }
        }
    }

    #[test]
    fn test_matrix_inverse_singular() {
        let a = mat(&[1.0, 2.0, 2.0, 4.0], 2, 2);
        assert!(matrix_inverse(&a).is_err());
    }

    #[test]
    fn test_matrix_determinant_2x2() {
        let a = mat(&[3.0, 8.0, 4.0, 6.0], 2, 2);
        let det = matrix_determinant(&a).unwrap();
        assert!(approx_eq(det, -14.0));
    }

    #[test]
    fn test_matrix_determinant_3x3() {
        let a = mat(&[6.0, 1.0, 1.0, 4.0, -2.0, 5.0, 2.0, 8.0, 7.0], 3, 3);
        let det = matrix_determinant(&a).unwrap();
        assert!(approx_eq(det, -306.0));
    }

    #[test]
    fn test_matrix_determinant_singular() {
        let a = mat(&[1.0, 2.0, 2.0, 4.0], 2, 2);
        let det = matrix_determinant(&a).unwrap();
        assert!(approx_eq(det, 0.0));
    }

    #[test]
    fn test_matrix_trace() {
        let a = mat(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0], 3, 3);
        let tr = matrix_trace(&a).unwrap();
        assert!(approx_eq(tr, 15.0));
    }

    #[test]
    fn test_matrix_add_dimension_mismatch() {
        let a = mat(&[1.0, 2.0, 3.0, 4.0], 2, 2);
        let b = mat(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], 2, 3);
        assert!(matrix_add(&a, &b).is_err());
    }

    #[test]
    fn test_matrix_matmul_dimension_mismatch() {
        let a = mat(&[1.0, 2.0, 3.0, 4.0], 2, 2);
        let b = mat(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], 3, 2);
        assert!(matrix_matmul(&a, &b).is_err());
    }

    #[test]
    fn test_matrix_add_large_simd() {
        // Test SIMD path (>= 16 elements)
        let n = 20;
        let data_a: Vec<f64> = (0..n).map(|i| i as f64).collect();
        let data_b: Vec<f64> = (0..n).map(|i| (i * 2) as f64).collect();
        let a = mat(&data_a, 4, 5);
        let b = mat(&data_b, 4, 5);
        let c = matrix_add(&a, &b).unwrap();
        for i in 0..n {
            assert!(approx_eq(c.data[i], data_a[i] + data_b[i]));
        }
    }

    #[test]
    fn test_matrix_matmul_4x4() {
        let a = mat(
            &[
                1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0, 13.0, 14.0, 15.0,
                16.0,
            ],
            4,
            4,
        );
        let identity = mat(
            &[
                1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
            ],
            4,
            4,
        );
        let c = matrix_matmul(&a, &identity).unwrap();
        assert!(mat_approx_eq(&c, &a));
    }
}
