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

/// Type alias for method handler functions.
///
/// All method handlers follow this signature:
/// - `vm`: Mutable VM instance (used by handlers that invoke closures or
///   otherwise touch VM state; pure handlers ignore it)
/// - `args`: Owned vector of arguments (receiver is first element, as ValueWord)
/// - `ctx`: Optional execution context for runtime integration
/// - Returns: `Result<ValueWord, VMError>` — the result value. The dispatcher
///   pushes it to the stack on success.
pub type MethodFn = fn(
    &mut VirtualMachine,
    Vec<ValueWord>,
    Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError>;

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
pub static ARRAY_METHODS: phf::Map<&'static str, MethodFn> = phf_map! {
    // Higher-order functions
    "map" => crate::executor::objects::array_transform::handle_map,
    "filter" => crate::executor::objects::array_transform::handle_filter,
    "reduce" => crate::executor::objects::array_aggregation::handle_reduce,
    "fold" => crate::executor::objects::array_aggregation::handle_reduce,
    "forEach" => crate::executor::objects::array_query::handle_for_each,
    "find" => crate::executor::objects::array_query::handle_find,
    "findIndex" => crate::executor::objects::array_query::handle_find_index,
    "some" => crate::executor::objects::array_query::handle_some,
    "every" => crate::executor::objects::array_query::handle_every,
    "sort" => crate::executor::objects::array_transform::handle_sort,
    "groupBy" => crate::executor::objects::array_transform::handle_group_by,
    "flatMap" => crate::executor::objects::array_transform::handle_flat_map,

    // Basic operations
    "len" => crate::executor::objects::array_basic::handle_len,
    "length" => crate::executor::objects::array_basic::handle_length,
    "first" => crate::executor::objects::array_basic::handle_first,
    "last" => crate::executor::objects::array_basic::handle_last,
    "reverse" => crate::executor::objects::array_basic::handle_reverse,
    "push" => crate::executor::objects::array_basic::handle_push,
    "pop" => crate::executor::objects::array_basic::handle_pop,
    "zip" => crate::executor::objects::array_basic::handle_zip,
    "slice" => crate::executor::objects::array_transform::handle_slice,
    "concat" => crate::executor::objects::array_transform::handle_concat,
    "take" => crate::executor::objects::array_transform::handle_take,
    "drop" => crate::executor::objects::array_transform::handle_drop,
    "skip" => crate::executor::objects::array_transform::handle_skip,

    // Search methods
    "indexOf" => crate::executor::objects::array_query::handle_index_of,
    "includes" => crate::executor::objects::array_query::handle_includes,

    // Transform methods
    "join" => crate::executor::objects::array_sort::handle_join_str,
    "flatten" => crate::executor::objects::array_transform::handle_flatten,
    "unique" => crate::executor::objects::array_sets::handle_unique,
    "distinct" => crate::executor::objects::array_sets::handle_distinct,
    "distinctBy" => crate::executor::objects::array_sets::handle_distinct_by,

    // Aggregation methods
    "sum" => crate::executor::objects::array_aggregation::handle_sum,
    "avg" => crate::executor::objects::array_aggregation::handle_avg,
    "min" => crate::executor::objects::array_aggregation::handle_min,
    "max" => crate::executor::objects::array_aggregation::handle_max,
    "count" => crate::executor::objects::array_aggregation::handle_count,

    // SQL-like query methods (aliases and additional)
    "where" => crate::executor::objects::array_query::handle_where,
    "select" => crate::executor::objects::array_query::handle_select,
    "orderBy" => crate::executor::objects::array_sort::handle_order_by,
    "thenBy" => crate::executor::objects::array_sort::handle_then_by,
    "takeWhile" => crate::executor::objects::array_query::handle_take_while,
    "skipWhile" => crate::executor::objects::array_query::handle_skip_while,
    "single" => crate::executor::objects::array_query::handle_single,
    "any" => crate::executor::objects::array_query::handle_any,
    "all" => crate::executor::objects::array_query::handle_all,

    // Join operations
    "innerJoin" => crate::executor::objects::array_joins::handle_inner_join,
    "leftJoin" => crate::executor::objects::array_joins::handle_left_join,
    "crossJoin" => crate::executor::objects::array_joins::handle_cross_join,

    // Set operations
    "union" => crate::executor::objects::array_sets::handle_union,
    "intersect" => crate::executor::objects::array_sets::handle_intersect,
    "except" => crate::executor::objects::array_sets::handle_except,

    // Clone
    "clone" => crate::executor::objects::array_basic::handle_clone,

    // Iterator
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
pub static DATATABLE_METHODS: phf::Map<&'static str, MethodFn> = phf_map! {
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
pub static COLUMN_METHODS: phf::Map<&'static str, MethodFn> = phf_map! {
    "len" => crate::executor::objects::column_methods::handle_len,
    "length" => crate::executor::objects::column_methods::handle_len,
    "sum" => crate::executor::objects::column_methods::handle_sum,
    "mean" => crate::executor::objects::column_methods::handle_mean,
    "min" => crate::executor::objects::column_methods::handle_min,
    "max" => crate::executor::objects::column_methods::handle_max,
    "std" => crate::executor::objects::column_methods::handle_std,
    "first" => crate::executor::objects::column_methods::handle_first,
    "last" => crate::executor::objects::column_methods::handle_last,
    "toArray" => crate::executor::objects::column_methods::handle_to_array,
    "abs" => crate::executor::objects::column_methods::handle_abs,
};

/// PHF registry for HashMap methods (18 methods)
///
/// **Core:** get, set, has, delete, keys, values, entries, len, length, isEmpty
/// **Higher-order:** map, filter, forEach, reduce, groupBy
/// **Convenience:** merge, getOrDefault, toArray
pub static HASHMAP_METHODS: phf::Map<&'static str, MethodFn> = phf_map! {
    "get" => crate::executor::objects::hashmap_methods::handle_get,
    "set" => crate::executor::objects::hashmap_methods::handle_set,
    "has" => crate::executor::objects::hashmap_methods::handle_has,
    "delete" => crate::executor::objects::hashmap_methods::handle_delete,
    "keys" => crate::executor::objects::hashmap_methods::handle_keys,
    "values" => crate::executor::objects::hashmap_methods::handle_values,
    "entries" => crate::executor::objects::hashmap_methods::handle_entries,
    "len" => crate::executor::objects::hashmap_methods::handle_len,
    "length" => crate::executor::objects::hashmap_methods::handle_len,
    "isEmpty" => crate::executor::objects::hashmap_methods::handle_is_empty,
    "map" => crate::executor::objects::hashmap_methods::handle_map,
    "filter" => crate::executor::objects::hashmap_methods::handle_filter,
    "forEach" => crate::executor::objects::hashmap_methods::handle_for_each,
    "merge" => crate::executor::objects::hashmap_methods::handle_merge,
    "getOrDefault" => crate::executor::objects::hashmap_methods::handle_get_or_default,
    "reduce" => crate::executor::objects::hashmap_methods::handle_reduce,
    "toArray" => crate::executor::objects::hashmap_methods::handle_to_array,
    "groupBy" => crate::executor::objects::hashmap_methods::handle_group_by,

    // Iterator
    "iter" => crate::executor::objects::iterator_methods::handle_hashmap_iter,
};

/// PHF registry for Set methods (14 methods)
///
/// **Core:** add, has, delete, size, len, length, isEmpty, toArray
/// **Higher-order:** forEach, map, filter
/// **Set operations:** union, intersection, difference
pub static SET_METHODS: phf::Map<&'static str, MethodFn> = phf_map! {
    "add" => crate::executor::objects::set_methods::handle_add,
    "has" => crate::executor::objects::set_methods::handle_has,
    "delete" => crate::executor::objects::set_methods::handle_delete,
    "size" => crate::executor::objects::set_methods::handle_size,
    "len" => crate::executor::objects::set_methods::handle_size,
    "length" => crate::executor::objects::set_methods::handle_size,
    "isEmpty" => crate::executor::objects::set_methods::handle_is_empty,
    "toArray" => crate::executor::objects::set_methods::handle_to_array,
    "forEach" => crate::executor::objects::set_methods::handle_for_each,
    "map" => crate::executor::objects::set_methods::handle_map,
    "filter" => crate::executor::objects::set_methods::handle_filter,
    "union" => crate::executor::objects::set_methods::handle_union,
    "intersection" => crate::executor::objects::set_methods::handle_intersection,
    "difference" => crate::executor::objects::set_methods::handle_difference,
};

/// PHF registry for Deque methods
///
/// **Mutation:** pushBack, pushFront, popBack, popFront
/// **Access:** peekBack, peekFront, get
/// **Info:** size, len, length, isEmpty
/// **Conversion:** toArray
pub static DEQUE_METHODS: phf::Map<&'static str, MethodFn> = phf_map! {
    "pushBack" => crate::executor::objects::deque_methods::handle_push_back,
    "pushFront" => crate::executor::objects::deque_methods::handle_push_front,
    "popBack" => crate::executor::objects::deque_methods::handle_pop_back,
    "popFront" => crate::executor::objects::deque_methods::handle_pop_front,
    "peekBack" => crate::executor::objects::deque_methods::handle_peek_back,
    "peekFront" => crate::executor::objects::deque_methods::handle_peek_front,
    "size" => crate::executor::objects::deque_methods::handle_size,
    "len" => crate::executor::objects::deque_methods::handle_size,
    "length" => crate::executor::objects::deque_methods::handle_size,
    "isEmpty" => crate::executor::objects::deque_methods::handle_is_empty,
    "toArray" => crate::executor::objects::deque_methods::handle_to_array,
    "get" => crate::executor::objects::deque_methods::handle_get,
};

/// PHF registry for PriorityQueue methods
///
/// **Mutation:** push, pop
/// **Access:** peek
/// **Info:** size, len, length, isEmpty
/// **Conversion:** toArray, toSortedArray
pub static PRIORITY_QUEUE_METHODS: phf::Map<&'static str, MethodFn> = phf_map! {
    "push" => crate::executor::objects::priority_queue_methods::handle_push,
    "pop" => crate::executor::objects::priority_queue_methods::handle_pop,
    "peek" => crate::executor::objects::priority_queue_methods::handle_peek,
    "size" => crate::executor::objects::priority_queue_methods::handle_size,
    "len" => crate::executor::objects::priority_queue_methods::handle_size,
    "length" => crate::executor::objects::priority_queue_methods::handle_size,
    "isEmpty" => crate::executor::objects::priority_queue_methods::handle_is_empty,
    "toArray" => crate::executor::objects::priority_queue_methods::handle_to_array,
    "toSortedArray" => crate::executor::objects::priority_queue_methods::handle_to_sorted_array,
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
pub static DATETIME_METHODS: phf::Map<&'static str, MethodFn> = phf_map! {
    // Component access
    "year" => crate::executor::objects::datetime_methods::handle_year,
    "month" => crate::executor::objects::datetime_methods::handle_month,
    "day" => crate::executor::objects::datetime_methods::handle_day,
    "hour" => crate::executor::objects::datetime_methods::handle_hour,
    "minute" => crate::executor::objects::datetime_methods::handle_minute,
    "second" => crate::executor::objects::datetime_methods::handle_second,
    "millisecond" => crate::executor::objects::datetime_methods::handle_millisecond,
    "microsecond" => crate::executor::objects::datetime_methods::handle_microsecond,

    // Day info
    "day_of_week" => crate::executor::objects::datetime_methods::handle_day_of_week,
    "day_of_year" => crate::executor::objects::datetime_methods::handle_day_of_year,
    "week_of_year" => crate::executor::objects::datetime_methods::handle_week_of_year,
    "is_weekday" => crate::executor::objects::datetime_methods::handle_is_weekday,
    "is_weekend" => crate::executor::objects::datetime_methods::handle_is_weekend,

    // Formatting
    "format" => crate::executor::objects::datetime_methods::handle_format,
    "iso8601" => crate::executor::objects::datetime_methods::handle_iso8601,
    "rfc2822" => crate::executor::objects::datetime_methods::handle_rfc2822,
    "unix_timestamp" => crate::executor::objects::datetime_methods::handle_unix_timestamp,
    "to_unix_millis" => crate::executor::objects::datetime_methods::handle_to_unix_millis,

    // Timezone
    "to_utc" => crate::executor::objects::datetime_methods::handle_to_utc,
    "to_timezone" => crate::executor::objects::datetime_methods::handle_to_timezone,
    "to_local" => crate::executor::objects::datetime_methods::handle_to_local,
    "timezone" => crate::executor::objects::datetime_methods::handle_timezone,
    "offset" => crate::executor::objects::datetime_methods::handle_offset,

    // Arithmetic
    "add_days" => crate::executor::objects::datetime_methods::handle_add_days,
    "add_hours" => crate::executor::objects::datetime_methods::handle_add_hours,
    "add_minutes" => crate::executor::objects::datetime_methods::handle_add_minutes,
    "add_seconds" => crate::executor::objects::datetime_methods::handle_add_seconds,
    "add_months" => crate::executor::objects::datetime_methods::handle_add_months,

    // Comparison
    "is_before" => crate::executor::objects::datetime_methods::handle_is_before,
    "is_after" => crate::executor::objects::datetime_methods::handle_is_after,
    "is_same_day" => crate::executor::objects::datetime_methods::handle_is_same_day,

    // Diff
    "diff" => crate::executor::objects::datetime_methods::handle_diff,
};

/// PHF registry for Instant methods (6 methods)
///
/// **Timing:** elapsed, elapsed_ms, elapsed_us, elapsed_ns, duration_since
/// **Formatting:** to_string
pub static INSTANT_METHODS: phf::Map<&'static str, MethodFn> = phf_map! {
    "elapsed" => crate::executor::objects::instant_methods::handle_elapsed,
    "elapsed_ms" => crate::executor::objects::instant_methods::handle_elapsed_ms,
    "elapsed_us" => crate::executor::objects::instant_methods::handle_elapsed_us,
    "elapsed_ns" => crate::executor::objects::instant_methods::handle_elapsed_ns,
    "duration_since" => crate::executor::objects::instant_methods::handle_duration_since,
    "to_string" => crate::executor::objects::instant_methods::handle_to_string,
};

/// PHF registry for Iterator methods (15 methods)
///
/// **Lazy transforms:** map, filter, take, skip, flatMap, enumerate, chain
/// **Terminal operations:** collect, toArray, forEach, reduce, count, any, all, find
pub static ITERATOR_METHODS: phf::Map<&'static str, MethodFn> = phf_map! {
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
pub static MATRIX_METHODS: phf::Map<&'static str, MethodFn> = phf_map! {
    // Linear algebra
    "transpose" => crate::executor::objects::matrix_methods::handle_transpose,
    "inverse" => crate::executor::objects::matrix_methods::handle_inverse,
    "det" => crate::executor::objects::matrix_methods::handle_determinant,
    "determinant" => crate::executor::objects::matrix_methods::handle_determinant,
    "trace" => crate::executor::objects::matrix_methods::handle_trace,

    // Shape and access
    "shape" => crate::executor::objects::matrix_methods::handle_shape,
    "reshape" => crate::executor::objects::matrix_methods::handle_reshape,
    "row" => crate::executor::objects::matrix_methods::handle_row,
    "col" => crate::executor::objects::matrix_methods::handle_col,
    "diag" => crate::executor::objects::matrix_methods::handle_diag,
    "flatten" => crate::executor::objects::matrix_methods::handle_flatten,

    // Higher-order
    "map" => crate::executor::objects::matrix_methods::handle_map,

    // Aggregation
    "sum" => crate::executor::objects::matrix_methods::handle_sum,
    "min" => crate::executor::objects::matrix_methods::handle_min,
    "max" => crate::executor::objects::matrix_methods::handle_max,
    "mean" => crate::executor::objects::matrix_methods::handle_mean,
    "rowSum" => crate::executor::objects::matrix_methods::handle_row_sum,
    "colSum" => crate::executor::objects::matrix_methods::handle_col_sum,
};

/// PHF registry for IndexedTable-specific methods (2 methods)
///
/// These methods require an IndexedTable (table with designated index column).
/// Inherited DataTable methods are dispatched via DATATABLE_METHODS fallback.
///
/// **Query:** between, resample
pub static INDEXED_TABLE_METHODS: phf::Map<&'static str, MethodFn> = phf_map! {
    "between" => crate::executor::objects::indexed_table_methods::handle_between,
    "resample" => crate::executor::objects::indexed_table_methods::handle_resample,
};

/// PHF registry for Vec<number> (FloatArray) methods
///
/// **Aggregations:** sum, avg, mean, min, max, std, variance
/// **Numeric:** dot, norm, normalize, cumsum, diff, abs, sqrt, ln, exp
/// **Standard:** len, length, map, filter, forEach, toArray
pub static FLOAT_ARRAY_METHODS: phf::Map<&'static str, MethodFn> = phf_map! {
    "sum" => crate::executor::objects::typed_array_methods::handle_float_sum,
    "avg" => crate::executor::objects::typed_array_methods::handle_float_avg,
    "mean" => crate::executor::objects::typed_array_methods::handle_float_avg,
    "min" => crate::executor::objects::typed_array_methods::handle_float_min,
    "max" => crate::executor::objects::typed_array_methods::handle_float_max,
    "std" => crate::executor::objects::typed_array_methods::handle_float_std,
    "variance" => crate::executor::objects::typed_array_methods::handle_float_variance,
    "dot" => crate::executor::objects::typed_array_methods::handle_float_dot,
    "norm" => crate::executor::objects::typed_array_methods::handle_float_norm,
    "normalize" => crate::executor::objects::typed_array_methods::handle_float_normalize,
    "cumsum" => crate::executor::objects::typed_array_methods::handle_float_cumsum,
    "diff" => crate::executor::objects::typed_array_methods::handle_float_diff,
    "abs" => crate::executor::objects::typed_array_methods::handle_float_abs,
    "sqrt" => crate::executor::objects::typed_array_methods::handle_float_sqrt,
    "ln" => crate::executor::objects::typed_array_methods::handle_float_ln,
    "exp" => crate::executor::objects::typed_array_methods::handle_float_exp,
    "len" => crate::executor::objects::typed_array_methods::handle_float_len,
    "length" => crate::executor::objects::typed_array_methods::handle_float_len,
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
pub static INT_ARRAY_METHODS: phf::Map<&'static str, MethodFn> = phf_map! {
    "sum" => crate::executor::objects::typed_array_methods::handle_int_sum,
    "avg" => crate::executor::objects::typed_array_methods::handle_int_avg,
    "mean" => crate::executor::objects::typed_array_methods::handle_int_avg,
    "min" => crate::executor::objects::typed_array_methods::handle_int_min,
    "max" => crate::executor::objects::typed_array_methods::handle_int_max,
    "abs" => crate::executor::objects::typed_array_methods::handle_int_abs,
    "len" => crate::executor::objects::typed_array_methods::handle_int_len,
    "length" => crate::executor::objects::typed_array_methods::handle_int_len,
    "map" => crate::executor::objects::typed_array_methods::handle_int_map,
    "filter" => crate::executor::objects::typed_array_methods::handle_int_filter,
    "forEach" => crate::executor::objects::typed_array_methods::handle_int_for_each,
    "toArray" => crate::executor::objects::typed_array_methods::handle_int_to_array,
};

/// PHF registry for Vec<bool> (BoolArray) methods
///
/// **Standard:** len, length, toArray
/// **Query:** any, all, count
pub static BOOL_ARRAY_METHODS: phf::Map<&'static str, MethodFn> = phf_map! {
    "len" => crate::executor::objects::typed_array_methods::handle_bool_len,
    "length" => crate::executor::objects::typed_array_methods::handle_bool_len,
    "toArray" => crate::executor::objects::typed_array_methods::handle_bool_to_array,
    "count" => crate::executor::objects::typed_array_methods::handle_bool_count_true,
    "any" => crate::executor::objects::typed_array_methods::handle_bool_any,
    "all" => crate::executor::objects::typed_array_methods::handle_bool_all,
};

// ═══════════════════════════════════════════════════════════════════════════
// Concurrency primitives — compiler-builtin interior mutability types
// ═══════════════════════════════════════════════════════════════════════════

/// Mutex<T> methods: lock, try_lock, set
pub static MUTEX_METHODS: phf::Map<&'static str, MethodFn> = phf_map! {
    "lock" => crate::executor::objects::concurrency_methods::handle_mutex_lock,
    "try_lock" => crate::executor::objects::concurrency_methods::handle_mutex_try_lock,
    "set" => crate::executor::objects::concurrency_methods::handle_mutex_set,
};

/// Atomic<T> methods: load, store, fetch_add, fetch_sub, compare_exchange
pub static ATOMIC_METHODS: phf::Map<&'static str, MethodFn> = phf_map! {
    "load" => crate::executor::objects::concurrency_methods::handle_atomic_load,
    "store" => crate::executor::objects::concurrency_methods::handle_atomic_store,
    "fetch_add" => crate::executor::objects::concurrency_methods::handle_atomic_fetch_add,
    "fetch_sub" => crate::executor::objects::concurrency_methods::handle_atomic_fetch_sub,
    "compare_exchange" => crate::executor::objects::concurrency_methods::handle_atomic_compare_exchange,
};

/// Lazy<T> methods: get, is_initialized
pub static LAZY_METHODS: phf::Map<&'static str, MethodFn> = phf_map! {
    "get" => crate::executor::objects::concurrency_methods::handle_lazy_get,
    "is_initialized" => crate::executor::objects::concurrency_methods::handle_lazy_is_initialized,
};

/// Channel methods: send, recv, try_recv, close, is_closed, is_sender
pub static CHANNEL_METHODS: phf::Map<&'static str, MethodFn> = phf_map! {
    "send" => crate::executor::objects::channel_methods::handle_channel_send,
    "recv" => crate::executor::objects::channel_methods::handle_channel_recv,
    "try_recv" => crate::executor::objects::channel_methods::handle_channel_try_recv,
    "close" => crate::executor::objects::channel_methods::handle_channel_close,
    "is_closed" => crate::executor::objects::channel_methods::handle_channel_is_closed,
    "is_sender" => crate::executor::objects::channel_methods::handle_channel_is_sender,
};
