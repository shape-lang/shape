//! Method handlers for the Column type (a typed view into a `DataTable`
//! column).
//!
//! Phase 1.B-vm Wave-β cluster M-collection-tail: bodies surface
//! `NotImplemented(SURFACE)` per playbook §7 REVISED + §10 D-objects-mod /
//! D-obj-tail precedent (ADR-006 §2.7.6 / §2.7.7).
//!
//! `Column` is **not** a surviving `HeapKind` variant per ADR-006 §2.3
//! trim (`crates/shape-value/src/heap_variants.rs`); the
//! `HeapValue::ColumnRef { schema_id, table, col_id }` payload was
//! removed alongside the `ValueWord::from_column_ref` constructor. The
//! kinded equivalent is naturally a `HeapValue::TableView` projection
//! (`Arc<TableViewData>` per ADR-006 §2.3) plus a column-id selector,
//! which is a Phase 2c Stage C item once the `TableView` view-shape
//! settles.
//!
//! The pre-Wave-6 implementation used the deleted
//! `ValueWord::from_column_ref` / `from_array` / `from_f64` / `from_i64`
//! / `from_bool` / `from_string` / `none` / `from_raw_bits`
//! constructors, the deleted `ValueWord::extract_col_nb` accessor, the
//! deleted `vmarray_from_vec`, the Arrow-array-to-`ValueWord` conversion
//! helpers, plus the kindless MethodHandler ABI. Per playbook §4 #1 / #9
//! a Bool-default kinded shim is forbidden; per §7.4 the correct
//! response is `NotImplemented(SURFACE)`.

use crate::executor::VirtualMachine;
use shape_runtime::context::ExecutionContext;
use shape_value::{KindedSlot, VMError};

#[inline]
fn surface(method: &str) -> VMError {
    VMError::NotImplemented(format!(
        "phase-2c — Column.{}(): Column is not a surviving HeapKind variant per \
         ADR-006 §2.3 trim; needs typed-Arc replacement (HeapValue::TableView \
         projection + column-id selector). MethodHandler ABI also needs kinded \
         migration (cluster E-builtins-backlog, Wave 5b template).",
        method
    ))
}

pub fn v2_len(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("len"))
}

pub fn v2_sum(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("sum"))
}

pub fn v2_mean(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("mean"))
}

pub fn v2_min(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("min"))
}

pub fn v2_max(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("max"))
}

pub fn v2_std(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("std"))
}

pub fn v2_first(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("first"))
}

pub fn v2_last(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("last"))
}

pub fn v2_to_array(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("toArray"))
}

pub fn v2_abs(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(surface("abs"))
}
