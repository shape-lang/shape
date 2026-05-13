//! Method handlers for the Matrix type — kinded `Arc<MatrixData>` carrier
//! reached through `HeapKind::Matrix = 34` (ADR-006 §2.7.22 amendment,
//! Round 18 S3 W12-matrix-floatslice-heapkind-exit, 2026-05-13).
//!
//! ## §2.7.22 amendment — Matrix exits the `TypedArrayData` carrier
//!
//! The pre-amendment Q23 ruling (Matrix lives under `HeapKind::TypedArray`
//! via `TypedArrayData::Matrix(Arc<MatrixData>)`) is superseded by the
//! Round 17 deletion-audit + cluster-0-transition strategic-owner
//! authorization (2026-05-13). The category-error in the Q23 ruling:
//! `TypedArrayData::Matrix` carries a **single Matrix**, not a
//! buffer-of-Matrix; placing it inside the element-typed-array carrier
//! enum was a structural mismatch the prior ruling justified under
//! ADR-005 §1 single-discriminator. With `TypedArrayData::Matrix`
//! deleted (this sub-cluster), Matrix becomes its own top-level
//! HeapKind / HeapValue, in the same dispatch shape as §2.7.20 Channel
//! / §2.7.15 HashSet (typed-Arc) but with the pure-discriminator
//! refcount-dispatch shape of §2.7.9 FilterExpr (slot bits are
//! `Arc::into_raw(Arc<MatrixData>)` directly, `as_heap_value()` is
//! unsound on these bits, retain/release routes through
//! `clone_with_kind` / `drop_with_kind` matching the kind label).
//!
//! ## Receiver-projection contract (post-amendment)
//!
//! Method handlers project the receiver via the canonical
//! reconstruct-clone-restore pattern (`iterator_methods::clone_typed_array_arc`
//! mirror): kind-gate on `Ptr(HeapKind::Matrix)`,
//! `Arc::<MatrixData>::from_raw(slot.raw() as *const MatrixData)`,
//! clone the inner `Arc<MatrixData>` share, then restore the outer
//! share via `Arc::into_raw` so the slot's owned share stays balanced.
//! Output construction goes back through `KindedSlot::from_matrix(arc)`
//! for matrix returns, `KindedSlot::from_typed_array(Arc::new(
//! TypedArrayData::F64(...)))` for vector returns. Scalar returns use
//! `KindedSlot::from_number` / `from_int`.
//!
//! ## Forbidden patterns refused
//!
//! - ValueWord revival in `Vec<ValueWord>` disguise — payload is
//!   typed `AlignedVec<f64>` end-to-end.
//! - Bool-default fallback at receiver-kind mismatch — surface a
//!   typed RuntimeError.
//! - Transitional shims preserving deleted Matrix-shape names —
//!   refused on sight.
//! - "Keep Matrix in TypedArrayData under documented exception" —
//!   superseded by this amendment; deletion is systematic.

use crate::executor::VirtualMachine;
use shape_runtime::context::ExecutionContext;
use shape_runtime::intrinsics::matrix_kernels;
use shape_value::aligned_vec::AlignedVec;
use shape_value::heap_value::{HeapKind, MatrixData, TypedArrayData};
use shape_value::typed_buffer::{AlignedTypedBuffer, TypedBuffer};
use shape_value::{KindedSlot, NativeKind, VMError};
use std::sync::Arc;

// ADR-006 §2.7.22 amendment (Round 18 S3, 2026-05-13): Matrix slot bits
// are `Arc::into_raw(Arc<MatrixData>) as u64` with kind
// `Ptr(HeapKind::Matrix)`. The pre-amendment `Ptr(HeapKind::TypedArray)`
// + `TypedArrayData::Matrix` two-step is retired.

// ── Receiver projection helpers ───────────────────────────────────────────

#[inline]
fn type_error(msg: impl Into<String>) -> VMError {
    VMError::RuntimeError(msg.into())
}

/// Project the receiver `KindedSlot` to the inner `Arc<MatrixData>`.
///
/// ADR-006 §2.7.22 amendment (Round 18 S3, 2026-05-13): the canonical
/// recovery is reconstruct-clone-restore against the typed-Arc payload
/// directly (mirror of §2.7.9 FilterExpr / §2.7.16 IteratorState
/// receiver-recovery). Slot bits are
/// `Arc::into_raw(Arc<MatrixData>) as u64` with kind
/// `Ptr(HeapKind::Matrix)`. The pre-amendment two-step
/// (`Ptr(HeapKind::TypedArray)` → `Arc::<TypedArrayData>::from_raw` →
/// match `TypedArrayData::Matrix(m)`) is retired — Matrix is a
/// first-class HeapKind, not buried inside `TypedArrayData`.
#[inline]
fn as_matrix(slot: &KindedSlot) -> Result<Arc<MatrixData>, VMError> {
    if !matches!(slot.kind, NativeKind::Ptr(HeapKind::Matrix)) {
        return Err(type_error(format!(
            "Matrix method receiver must be a Matrix (got kind {:?})",
            slot.kind
        )));
    }
    let bits = slot.slot.raw();
    if bits == 0 {
        return Err(type_error("Matrix method receiver: slot bits null"));
    }
    // SAFETY: per the construction-side contract on `KindedSlot::from_matrix`
    // / `op_new_matrix`, a `Ptr(HeapKind::Matrix)` slot's bits are
    // `Arc::into_raw(Arc<MatrixData>)` and the slot owns one strong-count
    // share. Reconstruct, clone (bumping the share), then restore the
    // slot's owned share via Arc::into_raw so the carrier stays balanced.
    let arc = unsafe { Arc::<MatrixData>::from_raw(bits as *const MatrixData) };
    let cloned = Arc::clone(&arc);
    let _ = Arc::into_raw(arc);
    Ok(cloned)
}

/// Wrap a `MatrixData` into a `KindedSlot` carrying
/// `Ptr(HeapKind::Matrix)` — mirror of `op_new_matrix`'s post-amendment
/// emit path so dispatch tables and stack-side retain/release see one shape.
#[inline]
fn matrix_slot(m: MatrixData) -> KindedSlot {
    KindedSlot::from_matrix(Arc::new(m))
}

/// Wrap a flat `AlignedVec<f64>` into a `KindedSlot` carrying
/// `Ptr(HeapKind::TypedArray)` over a `TypedArrayData::F64` arm.
#[inline]
fn float_array_slot(data: AlignedVec<f64>) -> KindedSlot {
    let buf = AlignedTypedBuffer::from_aligned(data);
    let arr = Arc::new(TypedArrayData::F64(Arc::new(buf)));
    KindedSlot::from_typed_array(arr)
}

/// Wrap a `Vec<i64>` into a `KindedSlot` carrying
/// `Ptr(HeapKind::TypedArray)` over a `TypedArrayData::I64` arm.
#[inline]
fn int_array_slot(data: Vec<i64>) -> KindedSlot {
    let buf = TypedBuffer::<i64>::from_vec(data);
    let arr = Arc::new(TypedArrayData::I64(Arc::new(buf)));
    KindedSlot::from_typed_array(arr)
}

/// Extract a scalar `i64` from an arg slot — accepts `Int64` and
/// narrows `Float64` only when the value is integer-representable.
#[inline]
fn arg_as_index(slot: &KindedSlot, name: &str) -> Result<i64, VMError> {
    match slot.kind {
        NativeKind::Int64 => Ok(slot.slot.raw() as i64),
        NativeKind::Float64 => {
            let f = f64::from_bits(slot.slot.raw());
            if f.is_finite() && f.trunc() == f {
                Ok(f as i64)
            } else {
                Err(type_error(format!(
                    "Matrix.{}: expected integer index, got non-integral number {}",
                    name, f
                )))
            }
        }
        other => Err(type_error(format!(
            "Matrix.{}: expected integer index, got kind {:?}",
            name, other
        ))),
    }
}

// ── Method bodies ──────────────────────────────────────────────────────────

/// `mat.transpose()` — return a fresh transposed matrix.
pub fn v2_transpose(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.is_empty() {
        return Err(type_error("transpose: missing receiver"));
    }
    let m = as_matrix(&args[0])?;
    let out = matrix_kernels::matrix_transpose(&m);
    Ok(matrix_slot(out))
}

/// `mat.inverse()` — return the matrix inverse, surfacing on singular
/// or non-square inputs (kernel returns `Result<MatrixData, String>`).
pub fn v2_inverse(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.is_empty() {
        return Err(type_error("inverse: missing receiver"));
    }
    let m = as_matrix(&args[0])?;
    let out = matrix_kernels::matrix_inverse(&m).map_err(VMError::RuntimeError)?;
    Ok(matrix_slot(out))
}

/// `mat.det()` / `mat.determinant()` — return the determinant scalar.
pub fn v2_determinant(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.is_empty() {
        return Err(type_error("det: missing receiver"));
    }
    let m = as_matrix(&args[0])?;
    let det = matrix_kernels::matrix_determinant(&m).map_err(VMError::RuntimeError)?;
    Ok(KindedSlot::from_number(det))
}

/// `mat.trace()` — return the trace scalar (sum of diagonal).
pub fn v2_trace(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.is_empty() {
        return Err(type_error("trace: missing receiver"));
    }
    let m = as_matrix(&args[0])?;
    let tr = matrix_kernels::matrix_trace(&m).map_err(VMError::RuntimeError)?;
    Ok(KindedSlot::from_number(tr))
}

/// `mat.shape()` — return `[rows, cols]` as `Array<int>`.
pub fn v2_shape(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.is_empty() {
        return Err(type_error("shape: missing receiver"));
    }
    let m = as_matrix(&args[0])?;
    Ok(int_array_slot(vec![m.rows as i64, m.cols as i64]))
}

/// `mat.reshape(rows, cols)` — re-wrap the same flat data with new
/// dimensions; rows*cols must equal the existing element count.
pub fn v2_reshape(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 3 {
        return Err(type_error(
            "reshape: expected (matrix, rows, cols)".to_string(),
        ));
    }
    let m = as_matrix(&args[0])?;
    let rows = arg_as_index(&args[1], "reshape")?;
    let cols = arg_as_index(&args[2], "reshape")?;
    if rows < 0 || cols < 0 {
        return Err(type_error(format!(
            "reshape: dimensions must be non-negative (got rows={}, cols={})",
            rows, cols
        )));
    }
    let total = (rows as usize) * (cols as usize);
    if total != m.data.len() {
        return Err(type_error(format!(
            "reshape: rows*cols ({}) must equal element count ({})",
            total,
            m.data.len()
        )));
    }
    // Copy the underlying buffer — the new MatrixData owns its storage.
    let mut data = AlignedVec::<f64>::with_capacity(total);
    for v in m.data.as_slice().iter() {
        data.push(*v);
    }
    let out = MatrixData::from_flat(data, rows as u32, cols as u32);
    Ok(matrix_slot(out))
}

/// `mat.row(idx)` — return the i-th row as `Array<number>`.
pub fn v2_row(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 2 {
        return Err(type_error("row: expected (matrix, index)".to_string()));
    }
    let m = as_matrix(&args[0])?;
    let idx = arg_as_index(&args[1], "row")?;
    if idx < 0 || (idx as u32) >= m.rows {
        return Err(type_error(format!(
            "row: index {} out of bounds (rows={})",
            idx, m.rows
        )));
    }
    let cols = m.cols as usize;
    let mut data = AlignedVec::<f64>::with_capacity(cols);
    for v in m.row_slice(idx as u32).iter() {
        data.push(*v);
    }
    Ok(float_array_slot(data))
}

/// `mat.col(idx)` — return the i-th column as `Array<number>`.
pub fn v2_col(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 2 {
        return Err(type_error("col: expected (matrix, index)".to_string()));
    }
    let m = as_matrix(&args[0])?;
    let idx = arg_as_index(&args[1], "col")?;
    if idx < 0 || (idx as u32) >= m.cols {
        return Err(type_error(format!(
            "col: index {} out of bounds (cols={})",
            idx, m.cols
        )));
    }
    let cols = m.cols as usize;
    let rows = m.rows as usize;
    let mut data = AlignedVec::<f64>::with_capacity(rows);
    for r in 0..rows {
        data.push(m.data.as_slice()[r * cols + idx as usize]);
    }
    Ok(float_array_slot(data))
}

/// `mat.diag()` — return the diagonal as `Array<number>`. Length is
/// `min(rows, cols)`.
pub fn v2_diag(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.is_empty() {
        return Err(type_error("diag: missing receiver"));
    }
    let m = as_matrix(&args[0])?;
    let n = (m.rows as usize).min(m.cols as usize);
    let cols = m.cols as usize;
    let mut data = AlignedVec::<f64>::with_capacity(n);
    for i in 0..n {
        data.push(m.data.as_slice()[i * cols + i]);
    }
    Ok(float_array_slot(data))
}

/// `mat.flatten()` — return the row-major flat data as `Array<number>`.
pub fn v2_flatten(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.is_empty() {
        return Err(type_error("flatten: missing receiver"));
    }
    let m = as_matrix(&args[0])?;
    let mut data = AlignedVec::<f64>::with_capacity(m.data.len());
    for v in m.data.as_slice().iter() {
        data.push(*v);
    }
    Ok(float_array_slot(data))
}

/// `mat.sum()` — element-wise sum.
pub fn v2_sum(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.is_empty() {
        return Err(type_error("sum: missing receiver"));
    }
    let m = as_matrix(&args[0])?;
    let s: f64 = m.data.as_slice().iter().sum();
    Ok(KindedSlot::from_number(s))
}

/// `mat.min()` — element-wise min. Empty matrix surfaces a RuntimeError
/// per the Vec<number> precedent.
pub fn v2_min(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.is_empty() {
        return Err(type_error("min: missing receiver"));
    }
    let m = as_matrix(&args[0])?;
    if m.data.is_empty() {
        return Err(type_error("min: empty matrix"));
    }
    let mn = m
        .data
        .as_slice()
        .iter()
        .copied()
        .fold(f64::INFINITY, f64::min);
    Ok(KindedSlot::from_number(mn))
}

/// `mat.max()` — element-wise max.
pub fn v2_max(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.is_empty() {
        return Err(type_error("max: missing receiver"));
    }
    let m = as_matrix(&args[0])?;
    if m.data.is_empty() {
        return Err(type_error("max: empty matrix"));
    }
    let mx = m
        .data
        .as_slice()
        .iter()
        .copied()
        .fold(f64::NEG_INFINITY, f64::max);
    Ok(KindedSlot::from_number(mx))
}

/// `mat.mean()` — arithmetic mean.
pub fn v2_mean(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.is_empty() {
        return Err(type_error("mean: missing receiver"));
    }
    let m = as_matrix(&args[0])?;
    if m.data.is_empty() {
        return Err(type_error("mean: empty matrix"));
    }
    let s: f64 = m.data.as_slice().iter().sum();
    Ok(KindedSlot::from_number(s / (m.data.len() as f64)))
}

/// `mat.rowSum()` — per-row sum, returned as `Array<number>` of length `rows`.
pub fn v2_row_sum(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.is_empty() {
        return Err(type_error("rowSum: missing receiver"));
    }
    let m = as_matrix(&args[0])?;
    let rows = m.rows as usize;
    let cols = m.cols as usize;
    let mut data = AlignedVec::<f64>::with_capacity(rows);
    for r in 0..rows {
        let s: f64 = m.data.as_slice()[r * cols..r * cols + cols].iter().sum();
        data.push(s);
    }
    Ok(float_array_slot(data))
}

/// `mat.colSum()` — per-column sum, returned as `Array<number>` of
/// length `cols`.
pub fn v2_col_sum(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.is_empty() {
        return Err(type_error("colSum: missing receiver"));
    }
    let m = as_matrix(&args[0])?;
    let rows = m.rows as usize;
    let cols = m.cols as usize;
    let mut sums = vec![0.0_f64; cols];
    let slice = m.data.as_slice();
    for r in 0..rows {
        for c in 0..cols {
            sums[c] += slice[r * cols + c];
        }
    }
    let mut data = AlignedVec::<f64>::with_capacity(cols);
    for s in sums.into_iter() {
        data.push(s);
    }
    Ok(float_array_slot(data))
}

/// `mat.map(fn)` — apply a per-element callback. Closure receives each
/// element as `Float64` and is expected to return a numeric kind
/// (`Float64` / `Int64` / `Bool` are accepted; non-numeric returns
/// surface a typed RuntimeError per ADR-006 §2.7.10 / Q11). Output is
/// a fresh `MatrixData` of the same shape.
pub(crate) fn handle_map(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 2 {
        return Err(type_error("map: expected (matrix, closure)".to_string()));
    }
    if args[1].kind != NativeKind::Ptr(HeapKind::Closure) {
        return Err(type_error(format!(
            "map: second argument must be a closure, got kind {:?}",
            args[1].kind
        )));
    }
    let m = as_matrix(&args[0])?;
    let total = m.data.len();
    let closure = &args[1];
    let mut out = AlignedVec::<f64>::with_capacity(total);
    for i in 0..total {
        let elem = KindedSlot::from_number(m.data.as_slice()[i]);
        let result = vm.call_value_immediate_nb(closure, &[elem], ctx.as_deref_mut())?;
        let v = match result.kind {
            NativeKind::Float64 => f64::from_bits(result.slot.raw()),
            NativeKind::Int64 => (result.slot.raw() as i64) as f64,
            NativeKind::Bool => {
                if result.slot.raw() != 0 {
                    1.0
                } else {
                    0.0
                }
            }
            other => {
                return Err(type_error(format!(
                    "map: closure must return a numeric kind, got {:?}",
                    other
                )));
            }
        };
        out.push(v);
    }
    let new_matrix = MatrixData::from_flat(out, m.rows, m.cols);
    Ok(matrix_slot(new_matrix))
}

#[cfg(test)]
mod tests {
    //! Storage-layer tests for the matrix method body path. Per playbook §0
    //! "at least 4 unit tests in shape-value (storage layer)" — these tests
    //! live alongside the body bodies because the W15-matrix close ruling
    //! (§2.7.22 amendment) is "no new HeapKind, no new HeapValue arm, no
    //! new HashSetData-style storage struct" — Matrix already has its
    //! storage in `shape_value::heap_value::MatrixData`. The closest
    //! storage-tier counterpart is exercising MatrixData + matrix_kernels
    //! end-to-end.
    //!
    //! The smoke target `let m = matrix([[1,2],[3,4]]); m.transpose()[0][0]`
    //! decomposes into:
    //!   1. `matrix(...)` ctor — Wave 5e territory, surface-stops here.
    //!   2. `op_new_matrix` flat-buffer construction — already lands an
    //!      `Arc<TypedArrayData>` containing
    //!      `TypedArrayData::Matrix(Arc::new(MatrixData::from_flat(...)))`.
    //!   3. `m.transpose()` — body in this file, exercises `as_matrix`
    //!      projection + `matrix_kernels::matrix_transpose` + `matrix_slot`
    //!      output.
    //!   4. `[0][0]` indexing — TypedArrayData::Matrix already has an
    //!      element-access path through `m.data.as_slice()[0]` (see
    //!      `array_transform.rs:546`).
    //!
    //! These tests pin (2) + the `matrix_kernels` half of (3) + the
    //! shape-side of (4).

    use super::*;
    use shape_value::aligned_vec::AlignedVec;

    fn aligned_from(vals: &[f64]) -> AlignedVec<f64> {
        let mut a = AlignedVec::with_capacity(vals.len());
        for v in vals {
            a.push(*v);
        }
        a
    }

    #[test]
    fn matrix_from_flat_round_trips_dimensions_and_data() {
        // Smoke target's input row-major: [[1,2],[3,4]] -> [1,2,3,4] / 2x2.
        let data = aligned_from(&[1.0, 2.0, 3.0, 4.0]);
        let m = MatrixData::from_flat(data, 2, 2);
        assert_eq!(m.rows, 2);
        assert_eq!(m.cols, 2);
        assert_eq!(m.data.len(), 4);
        assert_eq!(m.get(0, 0), 1.0);
        assert_eq!(m.get(0, 1), 2.0);
        assert_eq!(m.get(1, 0), 3.0);
        assert_eq!(m.get(1, 1), 4.0);
    }

    #[test]
    fn matrix_transpose_reorders_row_major_layout() {
        // [[1,2],[3,4]].transpose() = [[1,3],[2,4]].
        // Smoke target's first half: m.transpose()[0][0] == 1
        //                          (since transpose's [0][0] is m's [0][0]).
        let data = aligned_from(&[1.0, 2.0, 3.0, 4.0]);
        let m = MatrixData::from_flat(data, 2, 2);
        let t = matrix_kernels::matrix_transpose(&m);
        assert_eq!(t.rows, 2);
        assert_eq!(t.cols, 2);
        // Transpose: new[r][c] = old[c][r]
        assert_eq!(t.get(0, 0), 1.0); // smoke target asserts this
        assert_eq!(t.get(0, 1), 3.0);
        assert_eq!(t.get(1, 0), 2.0);
        assert_eq!(t.get(1, 1), 4.0);
    }

    #[test]
    fn matrix_transpose_non_square_swaps_dimensions() {
        // 2x3 -> 3x2
        let data = aligned_from(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        let m = MatrixData::from_flat(data, 2, 3);
        let t = matrix_kernels::matrix_transpose(&m);
        assert_eq!(t.rows, 3);
        assert_eq!(t.cols, 2);
        assert_eq!(t.get(0, 0), 1.0);
        assert_eq!(t.get(0, 1), 4.0);
        assert_eq!(t.get(1, 0), 2.0);
        assert_eq!(t.get(1, 1), 5.0);
        assert_eq!(t.get(2, 0), 3.0);
        assert_eq!(t.get(2, 1), 6.0);
    }

    #[test]
    fn matrix_determinant_2x2_matches_classic_formula() {
        // det([[1,2],[3,4]]) = 1*4 - 2*3 = -2
        let data = aligned_from(&[1.0, 2.0, 3.0, 4.0]);
        let m = MatrixData::from_flat(data, 2, 2);
        let d = matrix_kernels::matrix_determinant(&m).expect("det");
        assert!((d - (-2.0)).abs() < 1e-12);
    }

    #[test]
    fn matrix_trace_sums_diagonal() {
        // trace([[1,2,3],[4,5,6],[7,8,9]]) = 1+5+9 = 15
        let data = aligned_from(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0]);
        let m = MatrixData::from_flat(data, 3, 3);
        let t = matrix_kernels::matrix_trace(&m).expect("trace");
        assert!((t - 15.0).abs() < 1e-12);
    }

    #[test]
    fn matrix_data_clone_shares_arc_payload() {
        // ADR-006 §2.7.22 amendment (Round 18 S3, 2026-05-13): Matrix
        // retain/release goes through the typed-Arc payload directly
        // under `HeapKind::Matrix`. Cloning an `Arc<MatrixData>` is a
        // single strong-count bump on the shared inner buffer.
        let data = aligned_from(&[1.0, 2.0, 3.0, 4.0]);
        let m = Arc::new(MatrixData::from_flat(data, 2, 2));
        let cloned = Arc::clone(&m);
        // Both shares should point to the same underlying MatrixData.
        assert!(Arc::ptr_eq(&m, &cloned));
        // Two live shares: m, cloned
        assert_eq!(Arc::strong_count(&m), 2);
    }
}
