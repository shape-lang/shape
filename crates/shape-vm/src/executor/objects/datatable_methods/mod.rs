//! DataTable method handlers for the VM.
//!
//! ADR-006 §2.7.6 / §2.7.7 — Wave-β M-datatable cluster.
//!
//! All handler bodies in this module are placeholders
//! (`NotImplemented(SURFACE)`) per playbook §7.4 REVISED. The
//! pre-Wave-6.5 bodies were keyed on the deleted `ValueWord` carrier,
//! used the deleted `ArgVec`, called the deleted `ValueWord::from_*`
//! constructors, and dispatched through the deleted
//! `call_value_immediate_raw` raw-bits closure-call API. Migrating any
//! handler body without first migrating its callers and helper surface
//! would either reintroduce a forbidden pattern (CLAUDE.md) or paper
//! over the cascade with a Bool-default kind (§2.7.7 #9 forbidden).
//!
//! The join handlers (`handle_inner_join` / `handle_left_join`) use the
//! kinded ABI `args: &[KindedSlot]` → `Result<KindedSlot, VMError>` so
//! the cluster D `D-window-join` `handle_join_execute` deferred surface
//! can land in a follow-up. The other handlers retain the legacy
//! `MethodFnV2` signature (`&mut [u64]` → `Result<u64, VMError>`) at
//! the export boundary so the `method_registry.rs` PHF map keeps
//! compiling — their bodies are surface stubs that never touch the raw
//! u64 args, so no forbidden pattern is reintroduced.
//!
//! Organized into submodules:
//! - `common`: Shared helpers — empty in the post-bulldozer state
//! - `core`: Basic accessors (origin, len, columns, slice, head, tail, toMat, etc.)
//! - `aggregation`: Compute methods (sum, mean, min, max, sort, count, describe, aggregate)
//! - `query`: Query methods (filter, orderBy, group_by, forEach, map)
//! - `joins`: Join methods (innerJoin, leftJoin) — kinded ABI
//! - `indexing`: IndexedTable creation (index_by)
//! - `simulation`: Simulation method (simulate)
//! - `rolling`: SIMD-backed rolling/transform methods (correlation, covariance, rolling_*, diff, pct_change, forward_fill)

mod aggregation;
pub(crate) mod common;
mod core;
mod indexing;
mod joins;
mod query;
mod rolling;
mod simulation;

#[cfg(test)]
mod tests;

// Re-export all public handler functions so external callers (method_registry, indexed_table_methods)
// can continue to use `datatable_methods::handle_*` paths.

// Core methods — legacy MethodFnV2 ABI surfaces (stub bodies)
pub(crate) use self::core::{
    handle_column, handle_columns, handle_columns_ref, handle_execute, handle_first, handle_head,
    handle_last, handle_len, handle_limit, handle_origin, handle_rows, handle_select, handle_slice,
    handle_tail, handle_to_mat,
};

// Aggregation methods — legacy MethodFnV2 ABI surfaces (stub bodies)
pub(crate) use self::aggregation::{
    handle_aggregate, handle_count, handle_describe, handle_max, handle_mean, handle_min,
    handle_sort, handle_sum,
};

// Query methods — legacy MethodFnV2 ABI surfaces (stub bodies)
pub(crate) use self::query::{
    handle_filter, handle_for_each, handle_group_by, handle_map, handle_order_by,
};

// Join methods — KINDED ABI (`&[KindedSlot]` → `Result<KindedSlot, _>`).
// The flip from `&mut [u64]` is the cross-cluster cascade closure
// surfaced from D-window-join Wave-α; window_join.rs's
// `handle_join_execute` deferred surface depends on this signature.
pub(crate) use self::joins::{handle_inner_join, handle_left_join};

// Indexing methods — legacy MethodFnV2 ABI surface (stub body)
pub(crate) use self::indexing::handle_index_by;

// Simulation methods — legacy MethodFnV2 ABI surface (stub body)
pub(crate) use self::simulation::handle_simulate;

// Rolling/SIMD methods — legacy MethodFnV2 ABI surfaces (stub bodies)
pub(crate) use self::rolling::{
    handle_correlation, handle_covariance, handle_diff, handle_forward_fill, handle_pct_change,
    handle_rolling_mean, handle_rolling_std, handle_rolling_sum,
};
