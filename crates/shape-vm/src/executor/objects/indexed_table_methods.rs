//! IndexedTable method handlers for the VM.
//!
//! Phase 1.B-vm Wave-β cluster M-collection-tail: bodies surface
//! `NotImplemented(SURFACE)` per playbook §7 REVISED + §10 D-objects-mod /
//! D-obj-tail precedent (ADR-006 §2.7.6 / §2.7.7).
//!
//! `IndexedTable` is **not** a surviving `HeapKind` variant per ADR-006
//! §2.3 trim (`crates/shape-value/src/heap_variants.rs`); the
//! `HeapValue::IndexedTable { schema_id, table, index_col }` payload was
//! removed alongside the `ValueWord::from_indexed_table` constructor.
//! The kinded equivalent is naturally a `HeapValue::TableView` projection
//! (`Arc<TableViewData>` per ADR-006 §2.3) plus an index-column id —
//! that is a Phase 2c Stage C item once the `TableView` view-shape
//! settles. The `extract_indexed_table_nb` helper imported here lives in
//! the `datatable_methods::common` carrier and is itself awaiting that
//! cascade.
//!
//! The pre-Wave-6 implementation used the deleted
//! `shape_value::{ValueWord, ValueWordExt}` surface, the deleted
//! `ValueWord::from_indexed_table` / `from_raw_bits` / `from_f64` /
//! `from_i64` / `from_string` constructors, the
//! `raw_helpers::extract_number_coerce` helper (deleted in cluster
//! D-raw-helpers — only the FilterExpr extractor remains), and the
//! kindless MethodHandler ABI. Per playbook §4 #1 / #9 a Bool-default
//! kinded shim is forbidden; per §7.4 the correct response is
//! `NotImplemented(SURFACE)`.

use crate::executor::VirtualMachine;
use shape_value::VMError;

#[inline]
fn surface(method: &str) -> VMError {
    VMError::NotImplemented(format!(
        "phase-2c — IndexedTable.{}(): IndexedTable is not a surviving HeapKind \
         variant per ADR-006 §2.3 trim; needs typed-Arc replacement \
         (HeapValue::TableView projection + index-column id). MethodHandler ABI \
         also needs kinded migration (cluster E-builtins-backlog, Wave 5b \
         template).",
        method
    ))
}

/// `indexed.between(start, end)` — filter rows where index is in [start, end] (v2).
pub(crate) fn handle_between(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("between"))
}

/// `indexed.resample(interval, { col: "agg_fn", ... })` — bucket by interval, aggregate (v2).
pub(crate) fn handle_resample(
    _vm: &mut VirtualMachine,
    _args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    Err(surface("resample"))
}
