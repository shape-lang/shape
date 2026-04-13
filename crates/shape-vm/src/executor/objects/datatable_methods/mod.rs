//! DataTable method handlers for the VM.
//!
//! Implements methods callable on DataTable values from Shape code.
//! All handlers follow the standard MethodFn signature.
//!
//! Organized into submodules:
//! - `common`: Shared helpers (extract_dt, wrap_result_table, etc.)
//! - `core`: Basic accessors (origin, len, columns, slice, head, tail, toMat, etc.)
//! - `aggregation`: Compute methods (sum, mean, min, max, sort, count, describe, aggregate)
//! - `query`: Query methods (filter, orderBy, group_by, forEach, map)
//! - `joins`: Join methods (innerJoin, leftJoin)
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

// Core methods
pub(crate) use self::core::{
    handle_column, handle_columns, handle_columns_ref, handle_execute, handle_first, handle_head,
    handle_last, handle_len, handle_limit, handle_origin, handle_rows, handle_select, handle_slice,
    handle_tail, handle_to_mat,
};

// Aggregation methods
pub(in crate::executor::objects) use self::aggregation::{compute_aggregation, parse_agg_spec_nb};
pub(crate) use self::aggregation::{
    handle_aggregate, handle_count, handle_describe, handle_max, handle_mean, handle_min,
    handle_sort, handle_sum,
};

// Query methods
pub(crate) use self::query::{
    handle_filter, handle_for_each, handle_group_by, handle_map, handle_order_by,
};

// Join methods
pub(crate) use self::joins::{handle_inner_join, handle_left_join};

// Indexing methods
pub(crate) use self::indexing::handle_index_by;

// Simulation methods
pub(crate) use self::simulation::handle_simulate;

// Rolling/SIMD methods
pub(crate) use self::rolling::{
    handle_correlation, handle_covariance, handle_diff, handle_forward_fill, handle_pct_change,
    handle_rolling_mean, handle_rolling_std, handle_rolling_sum,
};
