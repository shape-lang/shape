//! Shared helpers for DataTable method handlers.
//!
//! ADR-006 §2.7.6 / §2.7.7 — Wave-β M-datatable cluster.
//!
//! All helpers in the prior incarnation of this module were keyed on the
//! deleted `ValueWord` carrier (`extract_dt_nb`, `wrap_result_table_nb`,
//! `extract_array_value_nb`, `typed_object_entries_nb_vm`,
//! `build_datatable_from_objects_nb`, `extract_col_nb`,
//! `extract_indexed_table_nb`, `collect_closure_numbers_nb`,
//! `cmp_nb_values`, `apply_comparison_nb`, `array_values_equal`,
//! `append_f64_column`, `typed_object_to_hashmap_nb_vm`) — every
//! signature took `&ValueWord` / returned `ValueWord` and called deleted
//! constructors (`ValueWord::from_datatable`, `from_typed_table`,
//! `from_indexed_table`, `from_row_view`, `from_heap_value`, ...).
//!
//! Per playbook §7.4 (REVISED) the correct refusal shape for an
//! end-to-end ValueWord-shaped helper module is to delete the helpers
//! and surface their callers via `NotImplemented(SURFACE)` placeholders
//! rather than papering over with forbidden patterns. The kinded
//! re-implementation pulls the receiver via `slot.as_heap_value()` +
//! `HeapValue::DataTable(arc)` / `HeapValue::TableView(tv)` match
//! (ADR-005 §1 single-discriminator, Q8 — no per-heap-variant accessors
//! on the carrier), pushes results as `Arc::into_raw + push_kinded(bits,
//! NativeKind::Ptr(HeapKind::DataTable))` (playbook §3 per-HeapKind
//! table). The body migration is tracked as a phase-2c follow-up to the
//! D-window-join + M-datatable closure once the cascading callers
//! (`indexed_table_methods.rs`, `simulation.rs` correlated-context body,
//! `aggregation.rs` group_by body) are scheduled.

// SURFACE: this module intentionally exports nothing in the
// post-bulldozer state. The helper functions it once provided keyed on
// the deleted `ValueWord` carrier; their kinded successors live alongside
// each handler module's `NotImplemented(SURFACE)` placeholder until the
// phase-2c body migration lands.
