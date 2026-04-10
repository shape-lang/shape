//! PHF-based method registry for Array and DataTable methods
//!
//! This module provides compile-time perfect hash function (PHF) maps for fast O(1) method
//! lookup. The PHF approach is:
//! - Zero runtime overhead: hash computed at compile time
//! - Faster than large match statements (O(1) vs O(n))
//! - Self-documenting: all methods visible in one place
//! - Easy to maintain: add method = one line in phf_map + implementation

use crate::executor::VirtualMachine;
use phf::phf_map;
use shape_runtime::context::ExecutionContext;
use shape_value::{VMError, ValueWord};

/// Method handler: operates on raw u64 stack slots — no Vec, no ValueWord on the hot path.
///
/// - `vm`: Mutable VM instance
/// - `args`: `args[0]` = receiver, `args[1..]` = arguments, all raw u64 bits.
///   **Mutable** so handlers can update pointers after in-place mutation
///   (e.g. `as_heap_mut` → `Arc::make_mut` may reallocate).
///   The dispatcher owns these bits and drops them after the handler returns.
/// - `ctx`: Optional execution context
/// - Returns: `Result<u64, VMError>` — raw result bits pushed onto the stack.
pub type MethodFnV2 = fn(
    &mut VirtualMachine,
    args: &mut [u64],
    ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError>;

/// Method handler stored in PHF maps.
pub type MethodHandler = MethodFnV2;

/// PHF registry for Array methods (47 methods total)
///
/// **Categories:**
/// - Higher-order functions: map, filter, reduce, forEach, find, findIndex, some, every, sort, groupBy, flatMap
/// - Basic operations: len, length, first, last, reverse, slice, concat, take, drop, skip
/// - Search methods: indexOf, includes
/// - Transform methods: join, flatten, unique, distinct, distinctBy
/// - Aggregation methods: sum, avg, min, max, count
/// - SQL-like query: where, select, orderBy, thenBy, takeWhile, skipWhile, single, any, all
/// - Join operations: innerJoin, leftJoin, crossJoin
/// - Set operations: union, intersect, except
pub static ARRAY_METHODS: phf::Map<&'static str, MethodHandler> = phf_map! {
    // Higher-order — Native (closure-based, handler manages VM callbacks)
    "map" => crate::executor::objects::array_transform::handle_map_v2,
    "filter" => crate::executor::objects::array_transform::handle_filter_v2,
    "reduce" => crate::executor::objects::array_aggregation::handle_reduce_v2,
    "fold" => crate::executor::objects::array_aggregation::handle_reduce_v2,
    "forEach" => crate::executor::objects::array_query::handle_for_each_v2,
    "find" => crate::executor::objects::array_query::handle_find_v2,
    "findIndex" => crate::executor::objects::array_query::handle_find_index_v2,
    "some" => crate::executor::objects::array_query::handle_some_v2,
    "every" => crate::executor::objects::array_query::handle_every_v2,
    "sort" => crate::executor::objects::array_transform::handle_sort_v2,
    "groupBy" => crate::executor::objects::array_transform::handle_group_by_v2,
    "flatMap" => crate::executor::objects::array_transform::handle_flat_map_v2,

    // Basic operations — Native
    "len" => crate::executor::objects::array_basic::handle_len_v2,
    "length" => crate::executor::objects::array_basic::handle_len_v2,
    "first" => crate::executor::objects::array_basic::handle_first_v2,
    "last" => crate::executor::objects::array_basic::handle_last_v2,
    "reverse" => crate::executor::objects::array_basic::handle_reverse_v2,
    "push" => crate::executor::objects::array_basic::handle_push_v2,
    "pop" => crate::executor::objects::array_basic::handle_pop_v2,
    "zip" => crate::executor::objects::array_basic::handle_zip_v2,
    "slice" => crate::executor::objects::array_transform::handle_slice_v2,
    "concat" => crate::executor::objects::array_transform::handle_concat_v2,
    "take" => crate::executor::objects::array_transform::handle_take_v2,
    "drop" => crate::executor::objects::array_transform::handle_drop_v2,
    "skip" => crate::executor::objects::array_transform::handle_skip_v2,

    // Search — Native
    "indexOf" => crate::executor::objects::array_query::handle_index_of_v2,
    "includes" => crate::executor::objects::array_query::handle_includes_v2,

    // Transform — Native
    "join" => crate::executor::objects::array_sort::handle_join_str_v2,
    "flatten" => crate::executor::objects::array_transform::handle_flatten_v2,
    "unique" => crate::executor::objects::array_sets::handle_unique_v2,
    "distinct" => crate::executor::objects::array_sets::handle_distinct_v2,
    "distinctBy" => crate::executor::objects::array_sets::handle_distinct_by_v2,

    // Aggregation — Native
    "sum" => crate::executor::objects::array_aggregation::handle_sum_v2,
    "avg" => crate::executor::objects::array_aggregation::handle_avg_v2,
    "min" => crate::executor::objects::array_aggregation::handle_min_v2,
    "max" => crate::executor::objects::array_aggregation::handle_max_v2,
    "count" => crate::executor::objects::array_aggregation::handle_count_v2,

    // SQL-like query — Native
    "where" => crate::executor::objects::array_query::handle_where_v2,
    "select" => crate::executor::objects::array_query::handle_select_v2,
    "orderBy" => crate::executor::objects::array_sort::handle_order_by_v2,
    "thenBy" => crate::executor::objects::array_sort::handle_then_by_v2,
    "takeWhile" => crate::executor::objects::array_query::handle_take_while_v2,
    "skipWhile" => crate::executor::objects::array_query::handle_skip_while_v2,
    "single" => crate::executor::objects::array_query::handle_single_v2,
    "any" => crate::executor::objects::array_query::handle_any_v2,
    "all" => crate::executor::objects::array_query::handle_all_v2,

    // Join operations — Native
    "innerJoin" => crate::executor::objects::array_joins::handle_inner_join_v2,
    "leftJoin" => crate::executor::objects::array_joins::handle_left_join_v2,
    "crossJoin" => crate::executor::objects::array_joins::handle_cross_join_v2,

    // Set operations — Native
    "union" => crate::executor::objects::array_sets::handle_union_v2,
    "intersect" => crate::executor::objects::array_sets::handle_intersect_v2,
    "except" => crate::executor::objects::array_sets::handle_except_v2,

    // Clone — Native
    "clone" => crate::executor::objects::array_basic::handle_clone_v2,

    // Iterator — still Legacy (waiting for iterator agent)
    "iter" => crate::executor::objects::iterator_methods::handle_array_iter,
};

/// PHF registry for DataTable methods (24 methods)
///
/// **Core operations:**
/// - len, columns, column, slice, head, tail, first, last, select
///
/// **Compute operations:**
/// - sum, mean, min, max, sort
///
/// **Query operations (Phase 4):**
/// - filter, orderBy, group_by, groupBy, aggregate, count, describe, forEach
pub static DATATABLE_METHODS: phf::Map<&'static str, MethodHandler> = phf_map! {
    // Core
    "origin" => crate::executor::objects::datatable_methods::handle_origin,
    "len" => crate::executor::objects::datatable_methods::handle_len,
    "length" => crate::executor::objects::datatable_methods::handle_len,
    "columns" => crate::executor::objects::datatable_methods::handle_columns,
    "column" => crate::executor::objects::datatable_methods::handle_column,
    "slice" => crate::executor::objects::datatable_methods::handle_slice,
    "head" => crate::executor::objects::datatable_methods::handle_head,
    "tail" => crate::executor::objects::datatable_methods::handle_tail,
    "first" => crate::executor::objects::datatable_methods::handle_first,
    "last" => crate::executor::objects::datatable_methods::handle_last,
    "select" => crate::executor::objects::datatable_methods::handle_select,
    "toMat" => crate::executor::objects::datatable_methods::handle_to_mat,
    "to_mat" => crate::executor::objects::datatable_methods::handle_to_mat,

    // Row/column iteration
    "rows" => crate::executor::objects::datatable_methods::handle_rows,
    "columnsRef" => crate::executor::objects::datatable_methods::handle_columns_ref,

    // Compute
    "sum" => crate::executor::objects::datatable_methods::handle_sum,
    "mean" => crate::executor::objects::datatable_methods::handle_mean,
    "min" => crate::executor::objects::datatable_methods::handle_min,
    "max" => crate::executor::objects::datatable_methods::handle_max,
    "sort" => crate::executor::objects::datatable_methods::handle_sort,

    // Query (Phase 4)
    "filter" => crate::executor::objects::datatable_methods::handle_filter,
    "orderBy" => crate::executor::objects::datatable_methods::handle_order_by,
    "group_by" => crate::executor::objects::datatable_methods::handle_group_by,
    "groupBy" => crate::executor::objects::datatable_methods::handle_group_by,
    "aggregate" => crate::executor::objects::datatable_methods::handle_aggregate,
    "count" => crate::executor::objects::datatable_methods::handle_count,
    "describe" => crate::executor::objects::datatable_methods::handle_describe,
    "forEach" => crate::executor::objects::datatable_methods::handle_for_each,
    "map" => crate::executor::objects::datatable_methods::handle_map,
    "index_by" => crate::executor::objects::datatable_methods::handle_index_by,
    "indexBy" => crate::executor::objects::datatable_methods::handle_index_by,

    // Queryable interface (consistent with DbTable)
    "limit" => crate::executor::objects::datatable_methods::handle_limit,
    "execute" => crate::executor::objects::datatable_methods::handle_execute,

    // Joins
    "innerJoin" => crate::executor::objects::datatable_methods::handle_inner_join,
    "leftJoin" => crate::executor::objects::datatable_methods::handle_left_join,

    // Simulation
    "simulate" => crate::executor::objects::datatable_methods::handle_simulate,

    // SIMD-backed methods
    "correlation" => crate::executor::objects::datatable_methods::handle_correlation,
    "covariance" => crate::executor::objects::datatable_methods::handle_covariance,
    "rolling_sum" => crate::executor::objects::datatable_methods::handle_rolling_sum,
    "rollingSum" => crate::executor::objects::datatable_methods::handle_rolling_sum,
    "rolling_mean" => crate::executor::objects::datatable_methods::handle_rolling_mean,
    "rollingMean" => crate::executor::objects::datatable_methods::handle_rolling_mean,
    "rolling_std" => crate::executor::objects::datatable_methods::handle_rolling_std,
    "rollingStd" => crate::executor::objects::datatable_methods::handle_rolling_std,
    "diff" => crate::executor::objects::datatable_methods::handle_diff,
    "pct_change" => crate::executor::objects::datatable_methods::handle_pct_change,
    "pctChange" => crate::executor::objects::datatable_methods::handle_pct_change,
    "forward_fill" => crate::executor::objects::datatable_methods::handle_forward_fill,
    "forwardFill" => crate::executor::objects::datatable_methods::handle_forward_fill,
};

/// PHF registry for Column methods (10 methods)
///
/// **Aggregation:** len, sum, mean, min, max, std
/// **Access:** first, last, toArray
/// **Transform:** abs
pub static COLUMN_METHODS: phf::Map<&'static str, MethodHandler> = phf_map! {
    "len" => crate::executor::objects::column_methods::v2_len,
    "length" => crate::executor::objects::column_methods::v2_len,
    "sum" => crate::executor::objects::column_methods::v2_sum,
    "mean" => crate::executor::objects::column_methods::v2_mean,
    "min" => crate::executor::objects::column_methods::v2_min,
    "max" => crate::executor::objects::column_methods::v2_max,
    "std" => crate::executor::objects::column_methods::v2_std,
    "first" => crate::executor::objects::column_methods::v2_first,
    "last" => crate::executor::objects::column_methods::v2_last,
    "toArray" => crate::executor::objects::column_methods::v2_to_array,
    "abs" => crate::executor::objects::column_methods::v2_abs,
};

/// PHF registry for HashMap methods (18 methods)
///
/// **Core:** get, set, has, delete, keys, values, entries, len, length, isEmpty
/// **Higher-order:** map, filter, forEach, reduce, groupBy
/// **Convenience:** merge, getOrDefault, toArray
pub static HASHMAP_METHODS: phf::Map<&'static str, MethodHandler> = phf_map! {
    // Non-closure methods — MethodFnV2
    "get" => crate::executor::objects::hashmap_methods::v2_get,
    "set" => crate::executor::objects::hashmap_methods::v2_set,
    "has" => crate::executor::objects::hashmap_methods::v2_has,
    "delete" => crate::executor::objects::hashmap_methods::v2_delete,
    "keys" => crate::executor::objects::hashmap_methods::v2_keys,
    "values" => crate::executor::objects::hashmap_methods::v2_values,
    "entries" => crate::executor::objects::hashmap_methods::v2_entries,
    "len" => crate::executor::objects::hashmap_methods::v2_len,
    "length" => crate::executor::objects::hashmap_methods::v2_len,
    "isEmpty" => crate::executor::objects::hashmap_methods::v2_is_empty,
    "merge" => crate::executor::objects::hashmap_methods::v2_merge,
    "getOrDefault" => crate::executor::objects::hashmap_methods::v2_get_or_default,
    "toArray" => crate::executor::objects::hashmap_methods::v2_to_array,
    // Closure-based — v2 native
    "map" => crate::executor::objects::hashmap_methods::v2_map,
    "filter" => crate::executor::objects::hashmap_methods::v2_filter,
    "forEach" => crate::executor::objects::hashmap_methods::v2_for_each,
    "reduce" => crate::executor::objects::hashmap_methods::v2_reduce,
    "groupBy" => crate::executor::objects::hashmap_methods::v2_group_by,

    // Iterator
    "iter" => crate::executor::objects::hashmap_methods::v2_iter,
};

/// PHF registry for Set methods (14 methods)
///
/// **Core:** add, has, delete, size, len, length, isEmpty, toArray
/// **Higher-order:** forEach, map, filter
/// **Set operations:** union, intersection, difference
pub static SET_METHODS: phf::Map<&'static str, MethodHandler> = phf_map! {
    "add" => crate::executor::objects::set_methods::v2_add,
    "delete" => crate::executor::objects::set_methods::v2_delete,
    // Read-only — MethodFnV2
    "has" => crate::executor::objects::set_methods::v2_has,
    "size" => crate::executor::objects::set_methods::v2_size,
    "len" => crate::executor::objects::set_methods::v2_size,
    "length" => crate::executor::objects::set_methods::v2_size,
    "isEmpty" => crate::executor::objects::set_methods::v2_is_empty,
    "toArray" => crate::executor::objects::set_methods::v2_to_array,
    "union" => crate::executor::objects::set_methods::v2_union,
    "intersection" => crate::executor::objects::set_methods::v2_intersection,
    "difference" => crate::executor::objects::set_methods::v2_difference,
    // Closure-based — v2 native
    "forEach" => crate::executor::objects::set_methods::v2_for_each,
    "map" => crate::executor::objects::set_methods::v2_map,
    "filter" => crate::executor::objects::set_methods::v2_filter,
};

/// PHF registry for Deque methods
///
/// **Mutation:** pushBack, pushFront, popBack, popFront
/// **Access:** peekBack, peekFront, get
/// **Info:** size, len, length, isEmpty
/// **Conversion:** toArray
pub static DEQUE_METHODS: phf::Map<&'static str, MethodHandler> = phf_map! {
    "pushBack" => crate::executor::objects::deque_methods::v2_push_back,
    "pushFront" => crate::executor::objects::deque_methods::v2_push_front,
    "popBack" => crate::executor::objects::deque_methods::v2_pop_back,
    "popFront" => crate::executor::objects::deque_methods::v2_pop_front,
    // Read-only — MethodFnV2
    "peekBack" => crate::executor::objects::deque_methods::v2_peek_back,
    "peekFront" => crate::executor::objects::deque_methods::v2_peek_front,
    "size" => crate::executor::objects::deque_methods::v2_size,
    "len" => crate::executor::objects::deque_methods::v2_size,
    "length" => crate::executor::objects::deque_methods::v2_size,
    "isEmpty" => crate::executor::objects::deque_methods::v2_is_empty,
    "toArray" => crate::executor::objects::deque_methods::v2_to_array,
    "get" => crate::executor::objects::deque_methods::v2_get,
};

/// PHF registry for PriorityQueue methods
///
/// **Mutation:** push, pop
/// **Access:** peek
/// **Info:** size, len, length, isEmpty
/// **Conversion:** toArray, toSortedArray
pub static PRIORITY_QUEUE_METHODS: phf::Map<&'static str, MethodHandler> = phf_map! {
    "push" => crate::executor::objects::priority_queue_methods::v2_push,
    "pop" => crate::executor::objects::priority_queue_methods::v2_pop,
    // Read-only — MethodFnV2
    "peek" => crate::executor::objects::priority_queue_methods::v2_peek,
    "size" => crate::executor::objects::priority_queue_methods::v2_size,
    "len" => crate::executor::objects::priority_queue_methods::v2_size,
    "length" => crate::executor::objects::priority_queue_methods::v2_size,
    "isEmpty" => crate::executor::objects::priority_queue_methods::v2_is_empty,
    "toArray" => crate::executor::objects::priority_queue_methods::v2_to_array,
    "toSortedArray" => crate::executor::objects::priority_queue_methods::v2_to_sorted_array,
};

/// PHF registry for DateTime methods (30 methods)
///
/// **Categories:**
/// - Component access: year, month, day, hour, minute, second, millisecond, microsecond
/// - Day info: day_of_week, day_of_year, week_of_year, is_weekday, is_weekend
/// - Formatting: format, iso8601, rfc2822, unix_timestamp
/// - Timezone: to_utc, to_timezone, to_local, timezone, offset
/// - Arithmetic: add_days, add_hours, add_minutes, add_seconds, add_months
/// - Comparison: is_before, is_after, is_same_day
pub static DATETIME_METHODS: phf::Map<&'static str, MethodHandler> = phf_map! {
    // Component access — Native
    "year" => crate::executor::objects::datetime_methods::v2_year,
    "month" => crate::executor::objects::datetime_methods::v2_month,
    "day" => crate::executor::objects::datetime_methods::v2_day,
    "hour" => crate::executor::objects::datetime_methods::v2_hour,
    "minute" => crate::executor::objects::datetime_methods::v2_minute,
    "second" => crate::executor::objects::datetime_methods::v2_second,
    "millisecond" => crate::executor::objects::datetime_methods::v2_millisecond,
    "microsecond" => crate::executor::objects::datetime_methods::v2_microsecond,

    // Day info — Native
    "day_of_week" => crate::executor::objects::datetime_methods::v2_day_of_week,
    "day_of_year" => crate::executor::objects::datetime_methods::v2_day_of_year,
    "week_of_year" => crate::executor::objects::datetime_methods::v2_week_of_year,
    "is_weekday" => crate::executor::objects::datetime_methods::v2_is_weekday,
    "is_weekend" => crate::executor::objects::datetime_methods::v2_is_weekend,

    // Formatting — Native
    "format" => crate::executor::objects::datetime_methods::v2_format,
    "iso8601" => crate::executor::objects::datetime_methods::v2_iso8601,
    "rfc2822" => crate::executor::objects::datetime_methods::v2_rfc2822,
    "unix_timestamp" => crate::executor::objects::datetime_methods::v2_unix_timestamp,
    "to_unix_millis" => crate::executor::objects::datetime_methods::v2_to_unix_millis,

    // Timezone — Native
    "to_utc" => crate::executor::objects::datetime_methods::v2_to_utc,
    "to_timezone" => crate::executor::objects::datetime_methods::v2_to_timezone,
    "to_local" => crate::executor::objects::datetime_methods::v2_to_local,
    "timezone" => crate::executor::objects::datetime_methods::v2_timezone,
    "offset" => crate::executor::objects::datetime_methods::v2_offset,

    // Operator-trait arithmetic — Native
    "add" => crate::executor::objects::datetime_methods::v2_add,
    "sub" => crate::executor::objects::datetime_methods::v2_sub,

    // Arithmetic — Native
    "add_days" => crate::executor::objects::datetime_methods::v2_add_days,
    "add_hours" => crate::executor::objects::datetime_methods::v2_add_hours,
    "add_minutes" => crate::executor::objects::datetime_methods::v2_add_minutes,
    "add_seconds" => crate::executor::objects::datetime_methods::v2_add_seconds,
    "add_months" => crate::executor::objects::datetime_methods::v2_add_months,

    // Comparison — Native
    "is_before" => crate::executor::objects::datetime_methods::v2_is_before,
    "is_after" => crate::executor::objects::datetime_methods::v2_is_after,
    "is_same_day" => crate::executor::objects::datetime_methods::v2_is_same_day,

    // Diff — Native
    "diff" => crate::executor::objects::datetime_methods::v2_diff,
};

/// PHF registry for TimeSpan (Duration) methods.
///
/// **Operator-trait:** add, sub
pub static TIMESPAN_METHODS: phf::Map<&'static str, MethodHandler> = phf_map! {
    "add" => crate::executor::objects::datetime_methods::v2_timespan_add,
    "sub" => crate::executor::objects::datetime_methods::v2_timespan_sub,
};

/// PHF registry for Instant methods (6 methods)
///
/// **Timing:** elapsed, elapsed_ms, elapsed_us, elapsed_ns, duration_since
/// **Formatting:** to_string
pub static INSTANT_METHODS: phf::Map<&'static str, MethodHandler> = phf_map! {
    "elapsed" => crate::executor::objects::instant_methods::v2_elapsed,
    "elapsed_ms" => crate::executor::objects::instant_methods::v2_elapsed_ms,
    "elapsed_us" => crate::executor::objects::instant_methods::v2_elapsed_us,
    "elapsed_ns" => crate::executor::objects::instant_methods::v2_elapsed_ns,
    "duration_since" => crate::executor::objects::instant_methods::v2_duration_since,
    "to_string" => crate::executor::objects::instant_methods::v2_to_string,
};

/// PHF registry for Iterator methods (15 methods)
///
/// **Lazy transforms:** map, filter, take, skip, flatMap, enumerate, chain
/// **Terminal operations:** collect, toArray, forEach, reduce, count, any, all, find
pub static ITERATOR_METHODS: phf::Map<&'static str, MethodHandler> = phf_map! {
    // Lazy transforms (return new Iterator)
    "map" => crate::executor::objects::iterator_methods::handle_map,
    "filter" => crate::executor::objects::iterator_methods::handle_filter,
    "take" => crate::executor::objects::iterator_methods::handle_take,
    "skip" => crate::executor::objects::iterator_methods::handle_skip,
    "flatMap" => crate::executor::objects::iterator_methods::handle_flat_map,
    "enumerate" => crate::executor::objects::iterator_methods::handle_enumerate,
    "chain" => crate::executor::objects::iterator_methods::handle_chain,

    // Terminal operations (consume the iterator)
    "collect" => crate::executor::objects::iterator_methods::handle_collect,
    "toArray" => crate::executor::objects::iterator_methods::handle_collect,
    "forEach" => crate::executor::objects::iterator_methods::handle_for_each,
    "reduce" => crate::executor::objects::iterator_methods::handle_reduce,
    "count" => crate::executor::objects::iterator_methods::handle_count,
    "any" => crate::executor::objects::iterator_methods::handle_any,
    "all" => crate::executor::objects::iterator_methods::handle_all,
    "find" => crate::executor::objects::iterator_methods::handle_find,
};

/// PHF registry for Matrix methods (18 methods)
///
/// **Linear algebra:** transpose, inverse, det, determinant, trace
/// **Shape/access:** shape, reshape, row, col, diag, flatten
/// **Higher-order:** map
/// **Aggregation:** sum, min, max, mean, rowSum, colSum
pub static MATRIX_METHODS: phf::Map<&'static str, MethodHandler> = phf_map! {
    // Linear algebra — MethodFnV2
    "transpose" => crate::executor::objects::matrix_methods::v2_transpose,
    "inverse" => crate::executor::objects::matrix_methods::v2_inverse,
    "det" => crate::executor::objects::matrix_methods::v2_determinant,
    "determinant" => crate::executor::objects::matrix_methods::v2_determinant,
    "trace" => crate::executor::objects::matrix_methods::v2_trace,

    // Shape and access — MethodFnV2
    "shape" => crate::executor::objects::matrix_methods::v2_shape,
    "reshape" => crate::executor::objects::matrix_methods::v2_reshape,
    "row" => crate::executor::objects::matrix_methods::v2_row,
    "col" => crate::executor::objects::matrix_methods::v2_col,
    "diag" => crate::executor::objects::matrix_methods::v2_diag,
    "flatten" => crate::executor::objects::matrix_methods::v2_flatten,

    // Higher-order — stays Legacy (closure-based)
    "map" => crate::executor::objects::matrix_methods::handle_map,

    // Aggregation — MethodFnV2
    "sum" => crate::executor::objects::matrix_methods::v2_sum,
    "min" => crate::executor::objects::matrix_methods::v2_min,
    "max" => crate::executor::objects::matrix_methods::v2_max,
    "mean" => crate::executor::objects::matrix_methods::v2_mean,
    "rowSum" => crate::executor::objects::matrix_methods::v2_row_sum,
    "colSum" => crate::executor::objects::matrix_methods::v2_col_sum,
};

/// PHF registry for IndexedTable-specific methods (2 methods)
///
/// These methods require an IndexedTable (table with designated index column).
/// Inherited DataTable methods are dispatched via DATATABLE_METHODS fallback.
///
/// **Query:** between, resample
pub static INDEXED_TABLE_METHODS: phf::Map<&'static str, MethodHandler> = phf_map! {
    "between" => crate::executor::objects::indexed_table_methods::handle_between,
    "resample" => crate::executor::objects::indexed_table_methods::handle_resample,
};

/// PHF registry for Vec<number> (FloatArray) methods
///
/// **Aggregations:** sum, avg, mean, min, max, std, variance
/// **Numeric:** dot, norm, normalize, cumsum, diff, abs, sqrt, ln, exp
/// **Standard:** len, length, map, filter, forEach, toArray
pub static FLOAT_ARRAY_METHODS: phf::Map<&'static str, MethodHandler> = phf_map! {
    // Aggregations — MethodFnV2 (v2 typed array + v1 fallback)
    "sum" => crate::executor::objects::typed_array_methods::v2_float_sum,
    "avg" => crate::executor::objects::typed_array_methods::v2_float_avg,
    "mean" => crate::executor::objects::typed_array_methods::v2_float_avg,
    "min" => crate::executor::objects::typed_array_methods::v2_float_min,
    "max" => crate::executor::objects::typed_array_methods::v2_float_max,
    "std" => crate::executor::objects::typed_array_methods::v2_float_std,
    "variance" => crate::executor::objects::typed_array_methods::v2_float_variance,
    "dot" => crate::executor::objects::typed_array_methods::v2_float_dot,
    "norm" => crate::executor::objects::typed_array_methods::v2_float_norm,
    "len" => crate::executor::objects::typed_array_methods::v2_len,
    "length" => crate::executor::objects::typed_array_methods::v2_len,
    // Transforms — still Legacy (require VM callback invocation or produce arrays)
    "normalize" => crate::executor::objects::typed_array_methods::handle_float_normalize,
    "cumsum" => crate::executor::objects::typed_array_methods::handle_float_cumsum,
    "diff" => crate::executor::objects::typed_array_methods::handle_float_diff,
    "abs" => crate::executor::objects::typed_array_methods::handle_float_abs,
    "sqrt" => crate::executor::objects::typed_array_methods::handle_float_sqrt,
    "ln" => crate::executor::objects::typed_array_methods::handle_float_ln,
    "exp" => crate::executor::objects::typed_array_methods::handle_float_exp,
    "map" => crate::executor::objects::typed_array_methods::handle_float_map,
    "filter" => crate::executor::objects::typed_array_methods::handle_float_filter,
    "forEach" => crate::executor::objects::typed_array_methods::handle_float_for_each,
    "toArray" => crate::executor::objects::typed_array_methods::handle_float_to_array,
};

/// PHF registry for Vec<int> (IntArray) methods
///
/// **Aggregations:** sum, avg, mean, min, max
/// **Numeric:** abs
/// **Standard:** len, length, map, filter, forEach, toArray
pub static INT_ARRAY_METHODS: phf::Map<&'static str, MethodHandler> = phf_map! {
    // Aggregations — MethodFnV2 (v2 typed array + v1 fallback)
    "sum" => crate::executor::objects::typed_array_methods::v2_int_sum,
    "avg" => crate::executor::objects::typed_array_methods::v2_int_avg,
    "mean" => crate::executor::objects::typed_array_methods::v2_int_avg,
    "min" => crate::executor::objects::typed_array_methods::v2_int_min,
    "max" => crate::executor::objects::typed_array_methods::v2_int_max,
    "len" => crate::executor::objects::typed_array_methods::v2_len,
    "length" => crate::executor::objects::typed_array_methods::v2_len,
    // Transforms — still Legacy
    "abs" => crate::executor::objects::typed_array_methods::handle_int_abs,
    "map" => crate::executor::objects::typed_array_methods::handle_int_map,
    "filter" => crate::executor::objects::typed_array_methods::handle_int_filter,
    "forEach" => crate::executor::objects::typed_array_methods::handle_int_for_each,
    "toArray" => crate::executor::objects::typed_array_methods::handle_int_to_array,
};

/// PHF registry for Vec<bool> (BoolArray) methods
///
/// **Standard:** len, length, toArray
/// **Query:** any, all, count
pub static BOOL_ARRAY_METHODS: phf::Map<&'static str, MethodHandler> = phf_map! {
    // MethodFnV2 (v2 typed array + v1 fallback)
    "len" => crate::executor::objects::typed_array_methods::v2_len,
    "length" => crate::executor::objects::typed_array_methods::v2_len,
    "count" => crate::executor::objects::typed_array_methods::v2_bool_count,
    "any" => crate::executor::objects::typed_array_methods::v2_bool_any,
    "all" => crate::executor::objects::typed_array_methods::v2_bool_all,
    // Still Legacy
    "toArray" => crate::executor::objects::typed_array_methods::handle_bool_to_array,
};

// ═══════════════════════════════════════════════════════════════════════════
// Concurrency primitives — compiler-builtin interior mutability types
// ═══════════════════════════════════════════════════════════════════════════

/// Mutex<T> methods: lock, try_lock, set
pub static MUTEX_METHODS: phf::Map<&'static str, MethodHandler> = phf_map! {
    "lock" => crate::executor::objects::concurrency_methods::v2_mutex_lock,
    "try_lock" => crate::executor::objects::concurrency_methods::v2_mutex_try_lock,
    "set" => crate::executor::objects::concurrency_methods::v2_mutex_set,
};

/// Atomic<T> methods: load, store, fetch_add, fetch_sub, compare_exchange
pub static ATOMIC_METHODS: phf::Map<&'static str, MethodHandler> = phf_map! {
    "load" => crate::executor::objects::concurrency_methods::v2_atomic_load,
    "store" => crate::executor::objects::concurrency_methods::v2_atomic_store,
    "fetch_add" => crate::executor::objects::concurrency_methods::v2_atomic_fetch_add,
    "fetch_sub" => crate::executor::objects::concurrency_methods::v2_atomic_fetch_sub,
    "compare_exchange" => crate::executor::objects::concurrency_methods::v2_atomic_compare_exchange,
};

/// Lazy<T> methods: get, is_initialized
pub static LAZY_METHODS: phf::Map<&'static str, MethodHandler> = phf_map! {
    "get" => crate::executor::objects::concurrency_methods::v2_lazy_get,
    "is_initialized" => crate::executor::objects::concurrency_methods::v2_lazy_is_initialized,
};

/// Channel methods: send, recv, try_recv, close, is_closed, is_sender
pub static CHANNEL_METHODS: phf::Map<&'static str, MethodHandler> = phf_map! {
    "send" => crate::executor::objects::channel_methods::v2_channel_send,
    "recv" => crate::executor::objects::channel_methods::v2_channel_recv,
    "try_recv" => crate::executor::objects::channel_methods::v2_channel_try_recv,
    "close" => crate::executor::objects::channel_methods::v2_channel_close,
    "is_closed" => crate::executor::objects::channel_methods::v2_channel_is_closed,
    "is_sender" => crate::executor::objects::channel_methods::v2_channel_is_sender,
};

/// PHF registry for Number/Int methods (simple numeric operations).
///
/// **Rounding:** floor, ceil, round
/// **Arithmetic:** abs, sign
/// **Conversion:** toInt, to_int, toNumber, to_number
/// **Predicates:** isNaN, is_nan, isFinite, is_finite
///
/// Methods NOT in this map (they need Vec<ValueWord> for multi-arg or string
/// return): toFixed, to_fixed, toString, to_string, clamp — handled by the
/// inline `handle_number_method` fallback.
pub static NUMBER_METHODS: phf::Map<&'static str, MethodHandler> = phf_map! {
    "floor" => crate::executor::objects::number_methods::number_floor_v2,
    "ceil" => crate::executor::objects::number_methods::number_ceil_v2,
    "round" => crate::executor::objects::number_methods::number_round_v2,
    "abs" => crate::executor::objects::number_methods::number_abs_v2,
    "sign" => crate::executor::objects::number_methods::number_sign_v2,
    "toInt" => crate::executor::objects::number_methods::number_to_int_v2,
    "to_int" => crate::executor::objects::number_methods::number_to_int_v2,
    "toNumber" => crate::executor::objects::number_methods::number_to_number_v2,
    "to_number" => crate::executor::objects::number_methods::number_to_number_v2,
    "isNaN" => crate::executor::objects::number_methods::number_is_nan_v2,
    "is_nan" => crate::executor::objects::number_methods::number_is_nan_v2,
    "isFinite" => crate::executor::objects::number_methods::number_is_finite_v2,
    "is_finite" => crate::executor::objects::number_methods::number_is_finite_v2,
    "toFixed" => crate::executor::objects::number_methods::number_to_fixed_v2,
    "to_fixed" => crate::executor::objects::number_methods::number_to_fixed_v2,
    "toString" => crate::executor::objects::number_methods::number_to_string_v2,
    "to_string" => crate::executor::objects::number_methods::number_to_string_v2,
    "clamp" => crate::executor::objects::number_methods::number_clamp_v2,
};

/// PHF registry for String methods (v2 native handlers)
///
/// **Categories:**
/// - Info: len, length
/// - Case: toUpperCase, to_upper_case, toLowerCase, to_lower_case
/// - Whitespace: trim, trimStart, trim_start, trimEnd, trim_end
/// - Search: contains, indexOf, index_of, startsWith, starts_with, endsWith, ends_with
/// - Transform: reverse, repeat, charAt, char_at, substring, replace, split, join
/// - Padding: padStart, pad_start, padEnd, pad_end
/// - Predicates: isDigit, is_digit, isAlpha, is_alpha, isAscii, is_ascii
/// - Conversion: toString, to_string, toInt, to_int, toNumber, to_number, toFloat, to_float
/// - Unicode: codePointAt, code_point_at, graphemeLen, grapheme_len
///
/// **Not included (fall through to legacy):** iter, graphemes, normalize
pub static STRING_METHODS: phf::Map<&'static str, MethodHandler> = phf_map! {
    // Info
    "len" => crate::executor::objects::string_methods::v2_string_len,
    "length" => crate::executor::objects::string_methods::v2_string_len,

    // Case
    "toUpperCase" => crate::executor::objects::string_methods::v2_string_to_upper,
    "to_upper_case" => crate::executor::objects::string_methods::v2_string_to_upper,
    "toLowerCase" => crate::executor::objects::string_methods::v2_string_to_lower,
    "to_lower_case" => crate::executor::objects::string_methods::v2_string_to_lower,

    // Whitespace
    "trim" => crate::executor::objects::string_methods::v2_string_trim,
    "trimStart" => crate::executor::objects::string_methods::v2_string_trim_start,
    "trim_start" => crate::executor::objects::string_methods::v2_string_trim_start,
    "trimEnd" => crate::executor::objects::string_methods::v2_string_trim_end,
    "trim_end" => crate::executor::objects::string_methods::v2_string_trim_end,

    // Conversion / identity
    "toString" => crate::executor::objects::string_methods::v2_string_to_string,
    "to_string" => crate::executor::objects::string_methods::v2_string_to_string,

    // Search
    "startsWith" => crate::executor::objects::string_methods::v2_string_starts_with,
    "starts_with" => crate::executor::objects::string_methods::v2_string_starts_with,
    "endsWith" => crate::executor::objects::string_methods::v2_string_ends_with,
    "ends_with" => crate::executor::objects::string_methods::v2_string_ends_with,
    "contains" => crate::executor::objects::string_methods::v2_string_contains,
    "indexOf" => crate::executor::objects::string_methods::v2_string_index_of,
    "index_of" => crate::executor::objects::string_methods::v2_string_index_of,

    // Transform
    "repeat" => crate::executor::objects::string_methods::v2_string_repeat,
    "charAt" => crate::executor::objects::string_methods::v2_string_char_at,
    "char_at" => crate::executor::objects::string_methods::v2_string_char_at,
    "reverse" => crate::executor::objects::string_methods::v2_string_reverse,
    "split" => crate::executor::objects::string_methods::v2_string_split,
    "replace" => crate::executor::objects::string_methods::v2_string_replace,
    "substring" => crate::executor::objects::string_methods::v2_string_substring,
    "join" => crate::executor::objects::string_methods::v2_string_join,

    // Padding
    "padStart" => crate::executor::objects::string_methods::v2_string_pad_start,
    "pad_start" => crate::executor::objects::string_methods::v2_string_pad_start,
    "padEnd" => crate::executor::objects::string_methods::v2_string_pad_end,
    "pad_end" => crate::executor::objects::string_methods::v2_string_pad_end,

    // Predicates
    "isDigit" => crate::executor::objects::string_methods::v2_string_is_digit,
    "is_digit" => crate::executor::objects::string_methods::v2_string_is_digit,
    "isAlpha" => crate::executor::objects::string_methods::v2_string_is_alpha,
    "is_alpha" => crate::executor::objects::string_methods::v2_string_is_alpha,
    "isAscii" => crate::executor::objects::string_methods::v2_string_is_ascii,
    "is_ascii" => crate::executor::objects::string_methods::v2_string_is_ascii,

    // Numeric conversion
    "toInt" => crate::executor::objects::string_methods::v2_string_to_int,
    "to_int" => crate::executor::objects::string_methods::v2_string_to_int,
    "toNumber" => crate::executor::objects::string_methods::v2_string_to_number,
    "to_number" => crate::executor::objects::string_methods::v2_string_to_number,
    "toFloat" => crate::executor::objects::string_methods::v2_string_to_number,
    "to_float" => crate::executor::objects::string_methods::v2_string_to_number,

    // Unicode
    "codePointAt" => crate::executor::objects::string_methods::v2_string_code_point_at,
    "code_point_at" => crate::executor::objects::string_methods::v2_string_code_point_at,
    "graphemeLen" => crate::executor::objects::string_methods::v2_string_grapheme_len,
    "grapheme_len" => crate::executor::objects::string_methods::v2_string_grapheme_len,
    "graphemes" => crate::executor::objects::string_methods::v2_string_graphemes,
    "normalize" => crate::executor::objects::string_methods::v2_string_normalize,
    "iter" => crate::executor::objects::string_methods::v2_string_iter,
};

/// PHF registry for Bool methods
pub static BOOL_METHODS: phf::Map<&'static str, MethodHandler> = phf_map! {
    "toString" => crate::executor::objects::number_methods::bool_to_string_v2,
    "to_string" => crate::executor::objects::number_methods::bool_to_string_v2,
};

/// PHF registry for Char methods (11 methods)
///
/// **Predicates:** is_alphabetic, is_numeric, is_alphanumeric, is_whitespace, is_uppercase, is_lowercase, is_ascii
/// **Transform:** to_uppercase, to_lowercase
/// **Conversion:** to_string, toString
pub static CHAR_METHODS: phf::Map<&'static str, MethodHandler> = phf_map! {
    "is_alphabetic" => crate::executor::objects::number_methods::char_is_alphabetic_v2,
    "isAlphabetic" => crate::executor::objects::number_methods::char_is_alphabetic_v2,
    "is_numeric" => crate::executor::objects::number_methods::char_is_numeric_v2,
    "isNumeric" => crate::executor::objects::number_methods::char_is_numeric_v2,
    "is_alphanumeric" => crate::executor::objects::number_methods::char_is_alphanumeric_v2,
    "isAlphanumeric" => crate::executor::objects::number_methods::char_is_alphanumeric_v2,
    "is_whitespace" => crate::executor::objects::number_methods::char_is_whitespace_v2,
    "isWhitespace" => crate::executor::objects::number_methods::char_is_whitespace_v2,
    "is_uppercase" => crate::executor::objects::number_methods::char_is_uppercase_v2,
    "isUppercase" => crate::executor::objects::number_methods::char_is_uppercase_v2,
    "is_lowercase" => crate::executor::objects::number_methods::char_is_lowercase_v2,
    "isLowercase" => crate::executor::objects::number_methods::char_is_lowercase_v2,
    "is_ascii" => crate::executor::objects::number_methods::char_is_ascii_v2,
    "isAscii" => crate::executor::objects::number_methods::char_is_ascii_v2,
    "to_uppercase" => crate::executor::objects::number_methods::char_to_uppercase_v2,
    "toUppercase" => crate::executor::objects::number_methods::char_to_uppercase_v2,
    "to_lowercase" => crate::executor::objects::number_methods::char_to_lowercase_v2,
    "toLowercase" => crate::executor::objects::number_methods::char_to_lowercase_v2,
    "to_string" => crate::executor::objects::number_methods::char_to_string_v2,
    "toString" => crate::executor::objects::number_methods::char_to_string_v2,
};

/// PHF registry for Content methods
///
/// **Style:** bold, italic, underline, dim, fg, bg
/// **Table/Chart:** border, max_rows, maxRows, series, title, x_label, xLabel, y_label, yLabel
/// **Conversion:** toString
pub static CONTENT_METHODS: phf::Map<&'static str, MethodHandler> = phf_map! {
    "bold" => crate::executor::objects::content_methods::v2_content_bold,
    "italic" => crate::executor::objects::content_methods::v2_content_italic,
    "underline" => crate::executor::objects::content_methods::v2_content_underline,
    "dim" => crate::executor::objects::content_methods::v2_content_dim,
    "fg" => crate::executor::objects::content_methods::v2_content_fg,
    "bg" => crate::executor::objects::content_methods::v2_content_bg,
    "toString" => crate::executor::objects::content_methods::v2_content_to_string,
    "border" => crate::executor::objects::content_methods::v2_content_border,
    "max_rows" => crate::executor::objects::content_methods::v2_content_max_rows,
    "maxRows" => crate::executor::objects::content_methods::v2_content_max_rows_camel,
    "series" => crate::executor::objects::content_methods::v2_content_series,
    "title" => crate::executor::objects::content_methods::v2_content_title,
    "x_label" => crate::executor::objects::content_methods::v2_content_x_label,
    "xLabel" => crate::executor::objects::content_methods::v2_content_x_label_camel,
    "y_label" => crate::executor::objects::content_methods::v2_content_y_label,
    "yLabel" => crate::executor::objects::content_methods::v2_content_y_label_camel,
};

/// PHF registry for Range methods
pub static RANGE_METHODS: phf::Map<&'static str, MethodHandler> = phf_map! {
    "iter" => crate::executor::objects::iterator_methods::v2_range_iter,
};
