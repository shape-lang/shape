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
//! Post-Wave-γ G-method-fn-v2-abi (ADR-006 §2.7.9 / Q11): every handler
//! in this module — joins, core, aggregation, query, indexing,
//! simulation, rolling — uses the kinded ABI `args: &[KindedSlot]` →
//! `Result<KindedSlot, VMError>`. The pre-§2.7.9 kind-blind ABI
//! (`&mut [u64]` → `Result<u64, VMError>`) is gone repo-wide. Body
//! migrations off SURFACE are Wave-γ-followup territory per the
//! M-datatable Wave-β `joins.rs` precedent at close commit `eb78699`.
//!
//! Organized into submodules:
//! - `common`: Shared helpers — empty in the post-bulldozer state
//! - `core`: Basic accessors (origin, len, columns, slice, head, tail, toMat, etc.)
//! - `aggregation`: Compute methods (sum, mean, min, max, sort, count, describe, aggregate)
//! - `query`: Query methods (filter, orderBy, group_by, forEach, map)
//! - `joins`: Join methods (innerJoin, leftJoin) — kinded ABI (§2.7.9 / Q11 precedent)
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

// Core methods — kinded ABI surfaces (stub bodies)
pub(crate) use self::core::{
    handle_column, handle_columns, handle_columns_ref, handle_execute, handle_first, handle_head,
    handle_last, handle_len, handle_limit, handle_origin, handle_rows, handle_select, handle_slice,
    handle_tail, handle_to_mat,
};

// Aggregation methods — kinded ABI surfaces (stub bodies)
pub(crate) use self::aggregation::{
    handle_aggregate, handle_count, handle_describe, handle_max, handle_mean, handle_min,
    handle_sort, handle_sum,
};

// Query methods — kinded ABI surfaces (stub bodies)
pub(crate) use self::query::{
    handle_filter, handle_for_each, handle_group_by, handle_map, handle_order_by,
};

// Join methods — kinded ABI (`&[KindedSlot]` → `Result<KindedSlot, _>`).
// The flip from `&mut [u64]` was first landed in M-datatable Wave-β
// (close commit `eb78699`) as the cross-cluster cascade closure
// surfaced from D-window-join Wave-α; window_join.rs's
// `handle_join_execute` deferred surface depends on this signature.
// Wave-γ G-method-fn-v2-abi (ADR-006 §2.7.9 / Q11) generalizes the
// same shape across the entire PHF registry.
pub(crate) use self::joins::{handle_inner_join, handle_left_join};

// Indexing methods — kinded ABI surface (stub body)
pub(crate) use self::indexing::handle_index_by;

// Simulation methods — kinded ABI surface (stub body)
pub(crate) use self::simulation::handle_simulate;

// Rolling/SIMD methods — kinded ABI surfaces (stub bodies)
pub(crate) use self::rolling::{
    handle_correlation, handle_covariance, handle_diff, handle_forward_fill, handle_pct_change,
    handle_rolling_mean, handle_rolling_std, handle_rolling_sum,
};
