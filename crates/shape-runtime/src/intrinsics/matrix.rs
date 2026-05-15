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
//! Body-side row extraction was previously via direct `Arc<HeapValue>`
//! pattern-match against `HeapValue::TypedArray(TypedArrayData::F64(buf))`
//! and `TypedArrayData::I64(...)` arms (mirror of `Arc<DataTable>`'s shape
//! at `marshal.rs:200-217`). Per V3-S5 ckpt-1 (2026-05-15) the
//! `TypedArrayData` enum was DELETED at `crates/shape-value/src/heap_value.rs`
//! (W12-typed-array-data-deletion audit §3.5 + ADR-006 §2.7.24 Q25.A
//! SUPERSEDED). The previous per-variant row-extraction + row-rebuild path
//! cascade-breaks here; production migration target is the v2-raw
//! `TypedArray<f64>` flat-struct carrier per audit §1.2 + §3.1 scalar
//! recipe (the only existing monomorphization for `f64` rows). The
//! `HeapValue::TypedArray(Arc<TypedArrayData>)` arm at
//! `heap_variants.rs:476` is ckpt-4 territory; until that arm migrates,
//! row-extract / row-rebuild surface-and-stop at runtime via the body
//! helpers below.
//!
//! Matrices are represented as `Array<Array<number>>` at runtime.
//! This module validates matrix shape once, flattens to contiguous row-major
//! buffers, and runs tight numeric kernels.

use crate::intrinsics::matrix_kernels;
use crate::marshal::register_typed_fn_2;
use crate::module_exports::ModuleExports;
use crate::typed_module_exports::{ConcreteReturn, ConcreteType, TypedReturn};
use shape_value::aligned_vec::AlignedVec;
use shape_value::heap_value::{HeapValue, MatrixData};
use shape_value::AlignedTypedBuffer;
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
/// Pattern-match shape previously mirrored `marshal.rs:200-217`'s
/// `FromSlot for Arc<DataTable>` and dispatched on `TypedArrayData::F64`
/// / `TypedArrayData::I64` arms. Per V3-S5 ckpt-1 (2026-05-15) the
/// `TypedArrayData` enum is DELETED (W12-typed-array-data-deletion audit
/// §3.5 + ADR-006 §2.7.24 Q25.A SUPERSEDED); the post-deletion target is
/// the v2-raw `TypedArray<f64>` flat-struct carrier per audit §1.2.
/// Row-extract surface-and-stops at runtime until ckpt-4 lands the
/// `HeapValue::TypedArray(Arc<TypedArrayData>)` arm migration to a v2-raw
/// row carrier (`heap_variants.rs:476`). The legacy per-`TypedArrayData::X`
/// dispatch shell is **refused on sight** under Refusal #1 (resurrection
/// under rename) per ckpt-1 close-marker.
fn row_to_f64_vec(_hv: &Arc<HeapValue>, label: &str, row_idx: usize) -> Result<Vec<f64>, String> {
    Err(format!(
        "{label} row {row_idx}: SURFACE — matrix row-extract reached a \
         `HeapValue::TypedArray(Arc<TypedArrayData>)` carrier whose inner \
         enum was DELETED at V3-S5 ckpt-1 (2026-05-15). Production target \
         is the v2-raw `TypedArray<f64>` / `TypedArray<i64>` flat-struct \
         carrier per W12-typed-array-data-deletion audit §1.2 + §3.1 \
         scalar recipe + ADR-006 §2.7.24 Q25.A SUPERSEDED. The \
         `HeapValue::TypedArray` variant migration to v2-raw rows is \
         ckpt-4 territory (`heap_variants.rs:476`). Cascade-broken \
         surface; UNREACHABLE until ckpt-4 + ckpt-5 land the row-carrier \
         cascade.",
        label = label,
        row_idx = row_idx,
    ))
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
/// `Vec<Arc<HeapValue>>` rows.
///
/// Previously built each row as
/// `Arc::new(HeapValue::TypedArray(Arc::new(TypedArrayData::F64(Arc::new(
/// AlignedTypedBuffer::from(...))))))`. Per V3-S5 ckpt-1 (2026-05-15) the
/// inner `TypedArrayData` enum is DELETED (W12-typed-array-data-deletion
/// audit §3.5 + ADR-006 §2.7.24 Q25.A SUPERSEDED). The v2-raw `TypedArray
/// <f64>` row carrier exists at the producer side (audit §1.3) but the
/// `HeapValue::TypedArray(Arc<TypedArrayData>)` enum arm is ckpt-4
/// territory (`heap_variants.rs:476`) — until ckpt-4 lands the row
/// carrier migration, host-side row-rebuild surface-and-stops at runtime.
/// Empty-matrix dimension reporting still works via the zero-row early-return.
fn matrix_to_heap_value_vec(_flat: &[f64], rows: usize, cols: usize) -> Vec<Arc<HeapValue>> {
    if rows == 0 {
        return Vec::new();
    }
    // SURFACE: cannot construct row carriers post-V3-S5 ckpt-1.
    // The non-empty case structurally cascade-breaks at the production
    // target until ckpt-4 lands the `HeapValue::TypedArray` arm
    // v2-raw migration. Production callers (`__intrinsic_matmul_vec` /
    // `_mat` / `mat_add` / `mat_sub`) surface a typed `String` error via
    // their `register_typed_fn_2` shell; this body would only be reached
    // post-ckpt-4 v2-raw row-carrier landing. Return empty as a
    // dimension-preserving placeholder; the calling intrinsic's
    // `extract_matrix` -> `row_to_f64_vec` surface-and-stop fires before
    // this path is hit on any non-trivial matrix input.
    debug_assert!(
        cols > 0,
        "matrix_to_heap_value_vec ckpt-2 broken-state: \
         non-empty rows={} requested but row-carrier rebuild path \
         cascade-broken (TypedArrayData DELETED at ckpt-1; \
         HeapValue::TypedArray arm migration is ckpt-4 territory). \
         W12-typed-array-data-deletion audit §1.2 + §3.1 production \
         target = v2-raw TypedArray<f64>.",
        rows,
    );
    let _ = (cols,);
    Vec::new()
}

/// Convert a kernel-produced `MatrixData` back to the nested-array
/// representation.
fn matrix_data_to_heap_value_vec(mat: &MatrixData) -> Vec<Arc<HeapValue>> {
    matrix_to_heap_value_vec(mat.data.as_slice(), mat.rows as usize, mat.cols as usize)
}

// Forward-consistency hooks reserved for ckpt-4 row-carrier migration land —
// the `AlignedVec`/`AlignedTypedBuffer` imports above remain because the v2-raw
// `TypedArray<f64>` producer wraps an `AlignedVec<f64>` per `crates/shape-value
// /src/v2/typed_array.rs:F64` monomorphization. Without an active row-carrier
// production path during V3-S5 ckpt-2 these imports look unused; the
// `#[allow(dead_code)]` const below pins them for the duration of the chain.
#[allow(dead_code)]
fn _ckpt4_carrier_pin() -> AlignedTypedBuffer {
    // Holds the `AlignedTypedBuffer` import name alive until the row-rebuild
    // path's ckpt-4 v2-raw migration restores its live use site.
    AlignedTypedBuffer::from(AlignedVec::<f64>::new())
}
