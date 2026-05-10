//! Method metadata stubs for legacy LSP completion paths.
//!
//! (W15-column, 2026-05-10) — `Column` is not a surviving `HeapKind`
//! variant per ADR-006 §2.7.21 / Q22 (the §2.3 trim removed
//! `HeapValue::ColumnRef`). Its semantics — a typed view into a single
//! column of a `DataTable` — are absorbed by `HeapKind::TableView` +
//! `TableViewData::ColumnRef { schema_id, table, col_id }` (see
//! `crates/shape-value/src/heap_value.rs`).
//!
//! `column_methods()` is preserved as an empty-`Vec` stub so the LSP
//! completion call sites
//! (`tools/shape-lsp/src/completion/types.rs`) keep compiling. When
//! Column-shaped completions are wanted, they belong on TableView's
//! `ColumnRef` projection methods (`datatable_methods/`), not on a
//! standalone `Column` value-type metadata table that no compiler or
//! VM dispatch surface honors.

use super::types::MethodInfo;

/// Method metadata for `Column` — empty by design (W15-column close).
///
/// Preserves the LSP-facing API shape; returns no entries because no
/// surviving runtime type has the name "Column". Callers that wanted
/// "this object resembles a column, suggest column methods" should
/// route through `TableView::ColumnRef` projection method metadata
/// once that exists.
pub fn column_methods() -> Vec<MethodInfo> {
    Vec::new()
}
