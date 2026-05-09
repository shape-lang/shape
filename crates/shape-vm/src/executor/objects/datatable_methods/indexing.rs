//! DataTable indexing methods: index_by.
//!
//! ADR-006 §2.7.6 / §2.7.7 — Wave-β M-datatable cluster.
//!
//! `handle_index_by` is a placeholder (`NotImplemented(SURFACE)`) per
//! playbook §7.4 REVISED. The pre-Wave-6.5 body extracted the column
//! name through deleted `raw_helpers::extract_str(u64)` ValueWord-bits
//! accessor and emitted an IndexedTable via the deleted
//! `ValueWord::from_indexed_table` constructor. The kinded
//! re-implementation pulls the column name as `NativeKind::String` and
//! pushes the result as
//! `NativeKind::Ptr(HeapKind::TableView)` (IndexedTable variant).

use shape_value::{KindedSlot, VMError};
use shape_runtime::context::ExecutionContext;

use crate::executor::VirtualMachine;

/// `dt.index_by(col)` — produce an IndexedTable view keyed by `col`.
pub(crate) fn handle_index_by(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "datatable.index_by — SURFACE: phase-2c body migration. Receiver kind \
         = NativeKind::Ptr(HeapKind::DataTable); body re-shape requires \
         kinded receiver dispatch via slot.as_heap_value() + \
         HeapValue::DataTable match per ADR-005 §1, kinded column-name arg \
         (NativeKind::String, Arc<String> via Arc::from_raw per playbook \
         §3), and result push as `Arc::into_raw(Arc<TableViewData>) + \
         push_kinded(bits, NativeKind::Ptr(HeapKind::TableView))` (the \
         IndexedTable variant of TableViewData)."
            .to_string(),
    ))
}
