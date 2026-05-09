//! Method handlers for the Matrix type.
//!
//! Phase 1.B-vm Wave-β cluster M-collection-tail: bodies surface
//! `NotImplemented(SURFACE)` per playbook §7 REVISED + §10 D-objects-mod /
//! D-obj-tail precedent (ADR-006 §2.7.6 / §2.7.7).
//!
//! `Matrix` is **not** a surviving `HeapKind` variant per ADR-006 §2.3
//! trim (`crates/shape-value/src/heap_variants.rs`); the
//! `HeapValue::Matrix(MatrixData)` payload was removed alongside
//! `from_matrix` / `from_float_array` constructors. The data shape itself
//! (an `AlignedVec<f64>` of `rows × cols`) overlaps with
//! `TypedArrayData::Float64` plus a stride descriptor — re-introducing
//! Matrix is a Phase 2c item that picks between (a) a kinded
//! `Arc<MatrixData>` HeapKind variant or (b) `TypedArray<f64>` with a
//! parallel shape descriptor. Either path requires an ADR-006 follow-up.
//!
//! The pre-Wave-6 implementation used the deleted `ValueWord::from_matrix`
//! / `from_float_array` / `from_f64` / `from_i64` / `from_array`
//! constructors, the deleted `HeapValue::Matrix` arm, the deleted
//! `value_word_drop::vw_drop` helper, `vmarray_from_vec`, and the
//! `raw_helpers::{extract_matrix, extract_number_coerce, type_name_from_bits}`
//! helpers (deleted in cluster D-raw-helpers). Per playbook §4 #1 / #9 a
//! Bool-default kinded shim is forbidden; per §7.4 the correct response
//! is `NotImplemented(SURFACE)`.

use crate::executor::VirtualMachine;
use shape_runtime::context::ExecutionContext;
use shape_value::VMError;

#[inline]
fn surface(method: &str) -> VMError {
    VMError::NotImplemented(format!(
        "phase-2c — Matrix.{}(): Matrix is not a surviving HeapKind variant per \
         ADR-006 §2.3 trim; needs typed-Arc replacement (Arc<MatrixData> arm \
         or TypedArray<f64> + shape descriptor). MethodHandler ABI also needs \
         kinded migration (cluster E-builtins-backlog, Wave 5b template).",
        method
    ))
}

pub fn v2_transpose(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("transpose"))
}

pub fn v2_inverse(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("inverse"))
}

pub fn v2_determinant(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("determinant"))
}

pub fn v2_trace(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("trace"))
}

pub fn v2_shape(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("shape"))
}

pub fn v2_reshape(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("reshape"))
}

pub fn v2_row(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("row"))
}

pub fn v2_col(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("col"))
}

pub fn v2_diag(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("diag"))
}

pub fn v2_flatten(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("flatten"))
}

pub fn v2_sum(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("sum"))
}

pub fn v2_min(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("min"))
}

pub fn v2_max(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("max"))
}

pub fn v2_mean(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("mean"))
}

pub fn v2_row_sum(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("rowSum"))
}

pub fn v2_col_sum(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("colSum"))
}

/// `mat.map(fn)` — apply per-element callback (v2).
pub(crate) fn handle_map(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("map"))
}
