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

/// V2 method handler: receives raw u64 stack slots, no Vec allocation.
///
/// - `vm`: Mutable VM instance
/// - `args`: `args[0]` = receiver, `args[1..]` = arguments, all raw u64 bits
/// - `ctx`: Optional execution context for runtime integration
/// - Returns: `Result<u64, VMError>` — the raw result bits. The dispatcher
///   pushes them onto the stack on success.
pub type MethodFnV2 = fn(
    &mut VirtualMachine,
    args: &[u64],
    ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError>;

/// Wrapper enum to support incremental migration from `MethodFn` (Vec-based)
/// to `MethodFnV2` (raw stack-slice). All existing handlers start as `Legacy`;
/// new or migrated handlers use `Native`.
#[derive(Clone, Copy)]
pub enum MethodHandler {
    /// Legacy handler: receives `Vec<ValueWord>`, returns `ValueWord`.
    Legacy(MethodFn),
    /// Native v2 handler: receives `&[u64]` stack slice, returns raw `u64`.
    Native(MethodFnV2),
}

impl MethodHandler {
    /// Extract the legacy handler function pointer, if this is a `Legacy` variant.
    #[inline]
    pub fn as_legacy(&self) -> Option<MethodFn> {
        match self {
            MethodHandler::Legacy(f) => Some(*f),
            MethodHandler::Native(_) => None,
        }
    }
}

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
    "map" => MethodHandler::Native(crate::executor::objects::array_transform::handle_map_v2),
    "filter" => MethodHandler::Native(crate::executor::objects::array_transform::handle_filter_v2),
    "reduce" => MethodHandler::Native(crate::executor::objects::array_aggregation::handle_reduce_v2),
    "fold" => MethodHandler::Native(crate::executor::objects::array_aggregation::handle_reduce_v2),
    "forEach" => MethodHandler::Native(crate::executor::objects::array_query::handle_for_each_v2),
    "find" => MethodHandler::Native(crate::executor::objects::array_query::handle_find_v2),
    "findIndex" => MethodHandler::Native(crate::executor::objects::array_query::handle_find_index_v2),
    "some" => MethodHandler::Native(crate::executor::objects::array_query::handle_some_v2),
    "every" => MethodHandler::Native(crate::executor::objects::array_query::handle_every_v2),
    "sort" => MethodHandler::Native(crate::executor::objects::array_transform::handle_sort_v2),
    "groupBy" => MethodHandler::Native(crate::executor::objects::array_transform::handle_group_by_v2),
    "flatMap" => MethodHandler::Native(crate::executor::objects::array_transform::handle_flat_map_v2),

    // Basic operations — Native
    "len" => MethodHandler::Native(crate::executor::objects::array_basic::handle_len_v2),
    "length" => MethodHandler::Native(crate::executor::objects::array_basic::handle_len_v2),
    "first" => MethodHandler::Native(crate::executor::objects::array_basic::handle_first_v2),
    "last" => MethodHandler::Native(crate::executor::objects::array_basic::handle_last_v2),
    "reverse" => MethodHandler::Native(crate::executor::objects::array_basic::handle_reverse_v2),
    "push" => MethodHandler::Native(crate::executor::objects::array_basic::handle_push_v2),
    "pop" => MethodHandler::Native(crate::executor::objects::array_basic::handle_pop_v2),
    "zip" => MethodHandler::Native(crate::executor::objects::array_basic::handle_zip_v2),
    "slice" => MethodHandler::Native(crate::executor::objects::array_transform::handle_slice_v2),
    "concat" => MethodHandler::Native(crate::executor::objects::array_transform::handle_concat_v2),
    "take" => MethodHandler::Native(crate::executor::objects::array_transform::handle_take_v2),
    "drop" => MethodHandler::Native(crate::executor::objects::array_transform::handle_drop_v2),
    "skip" => MethodHandler::Native(crate::executor::objects::array_transform::handle_skip_v2),

    // Search — Native
    "indexOf" => MethodHandler::Native(crate::executor::objects::array_query::handle_index_of_v2),
    "includes" => MethodHandler::Native(crate::executor::objects::array_query::handle_includes_v2),

    // Transform — Native
    "join" => MethodHandler::Native(crate::executor::objects::array_sort::handle_join_str_v2),
    "flatten" => MethodHandler::Native(crate::executor::objects::array_transform::handle_flatten_v2),
    "unique" => MethodHandler::Native(crate::executor::objects::array_sets::handle_unique_v2),
    "distinct" => MethodHandler::Native(crate::executor::objects::array_sets::handle_distinct_v2),
    "distinctBy" => MethodHandler::Native(crate::executor::objects::array_sets::handle_distinct_by_v2),

    // Aggregation — Native
    "sum" => MethodHandler::Native(crate::executor::objects::array_aggregation::handle_sum_v2),
    "avg" => MethodHandler::Native(crate::executor::objects::array_aggregation::handle_avg_v2),
    "min" => MethodHandler::Native(crate::executor::objects::array_aggregation::handle_min_v2),
    "max" => MethodHandler::Native(crate::executor::objects::array_aggregation::handle_max_v2),
    "count" => MethodHandler::Native(crate::executor::objects::array_aggregation::handle_count_v2),

    // SQL-like query — Native
    "where" => MethodHandler::Native(crate::executor::objects::array_query::handle_where_v2),
    "select" => MethodHandler::Native(crate::executor::objects::array_query::handle_select_v2),
    "orderBy" => MethodHandler::Native(crate::executor::objects::array_sort::handle_order_by_v2),
    "thenBy" => MethodHandler::Native(crate::executor::objects::array_sort::handle_then_by_v2),
    "takeWhile" => MethodHandler::Native(crate::executor::objects::array_query::handle_take_while_v2),
    "skipWhile" => MethodHandler::Native(crate::executor::objects::array_query::handle_skip_while_v2),
    "single" => MethodHandler::Native(crate::executor::objects::array_query::handle_single_v2),
    "any" => MethodHandler::Native(crate::executor::objects::array_query::handle_any_v2),
    "all" => MethodHandler::Native(crate::executor::objects::array_query::handle_all_v2),

    // Join operations — Native
    "innerJoin" => MethodHandler::Native(crate::executor::objects::array_joins::handle_inner_join_v2),
    "leftJoin" => MethodHandler::Native(crate::executor::objects::array_joins::handle_left_join_v2),
    "crossJoin" => MethodHandler::Native(crate::executor::objects::array_joins::handle_cross_join_v2),

    // Set operations — Native
    "union" => MethodHandler::Native(crate::executor::objects::array_sets::handle_union_v2),
    "intersect" => MethodHandler::Native(crate::executor::objects::array_sets::handle_intersect_v2),
    "except" => MethodHandler::Native(crate::executor::objects::array_sets::handle_except_v2),

    // Clone — Native
    "clone" => MethodHandler::Native(crate::executor::objects::array_basic::handle_clone_v2),

    // Iterator — still Legacy (waiting for iterator agent)
    "iter" => MethodHandler::Native(crate::executor::objects::iterator_methods::handle_array_iter),
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
    "origin" => MethodHandler::Native(crate::executor::objects::datatable_methods::handle_origin),
    "len" => MethodHandler::Native(crate::executor::objects::datatable_methods::handle_len),
    "length" => MethodHandler::Native(crate::executor::objects::datatable_methods::handle_len),
    "columns" => MethodHandler::Native(crate::executor::objects::datatable_methods::handle_columns),
    "column" => MethodHandler::Native(crate::executor::objects::datatable_methods::handle_column),
    "slice" => MethodHandler::Native(crate::executor::objects::datatable_methods::handle_slice),
    "head" => MethodHandler::Native(crate::executor::objects::datatable_methods::handle_head),
    "tail" => MethodHandler::Native(crate::executor::objects::datatable_methods::handle_tail),
    "first" => MethodHandler::Native(crate::executor::objects::datatable_methods::handle_first),
    "last" => MethodHandler::Native(crate::executor::objects::datatable_methods::handle_last),
    "select" => MethodHandler::Native(crate::executor::objects::datatable_methods::handle_select),
    "toMat" => MethodHandler::Native(crate::executor::objects::datatable_methods::handle_to_mat),
    "to_mat" => MethodHandler::Native(crate::executor::objects::datatable_methods::handle_to_mat),

    // Row/column iteration
    "rows" => MethodHandler::Native(crate::executor::objects::datatable_methods::handle_rows),
    "columnsRef" => MethodHandler::Native(crate::executor::objects::datatable_methods::handle_columns_ref),

    // Compute
    "sum" => MethodHandler::Native(crate::executor::objects::datatable_methods::handle_sum),
    "mean" => MethodHandler::Native(crate::executor::objects::datatable_methods::handle_mean),
    "min" => MethodHandler::Native(crate::executor::objects::datatable_methods::handle_min),
    "max" => MethodHandler::Native(crate::executor::objects::datatable_methods::handle_max),
    "sort" => MethodHandler::Native(crate::executor::objects::datatable_methods::handle_sort),

    // Query (Phase 4)
    "filter" => MethodHandler::Native(crate::executor::objects::datatable_methods::handle_filter),
    "orderBy" => MethodHandler::Native(crate::executor::objects::datatable_methods::handle_order_by),
    "group_by" => MethodHandler::Native(crate::executor::objects::datatable_methods::handle_group_by),
    "groupBy" => MethodHandler::Native(crate::executor::objects::datatable_methods::handle_group_by),
    "aggregate" => MethodHandler::Native(crate::executor::objects::datatable_methods::handle_aggregate),
    "count" => MethodHandler::Native(crate::executor::objects::datatable_methods::handle_count),
    "describe" => MethodHandler::Native(crate::executor::objects::datatable_methods::handle_describe),
    "forEach" => MethodHandler::Native(crate::executor::objects::datatable_methods::handle_for_each),
    "map" => MethodHandler::Native(crate::executor::objects::datatable_methods::handle_map),
    "index_by" => MethodHandler::Native(crate::executor::objects::datatable_methods::handle_index_by),
    "indexBy" => MethodHandler::Native(crate::executor::objects::datatable_methods::handle_index_by),

    // Queryable interface (consistent with DbTable)
    "limit" => MethodHandler::Native(crate::executor::objects::datatable_methods::handle_limit),
    "execute" => MethodHandler::Native(crate::executor::objects::datatable_methods::handle_execute),

    // Joins
    "innerJoin" => MethodHandler::Native(crate::executor::objects::datatable_methods::handle_inner_join),
    "leftJoin" => MethodHandler::Native(crate::executor::objects::datatable_methods::handle_left_join),

    // Simulation
    "simulate" => MethodHandler::Native(crate::executor::objects::datatable_methods::handle_simulate),

    // SIMD-backed methods
    "correlation" => MethodHandler::Native(crate::executor::objects::datatable_methods::handle_correlation),
    "covariance" => MethodHandler::Native(crate::executor::objects::datatable_methods::handle_covariance),
    "rolling_sum" => MethodHandler::Native(crate::executor::objects::datatable_methods::handle_rolling_sum),
    "rollingSum" => MethodHandler::Native(crate::executor::objects::datatable_methods::handle_rolling_sum),
    "rolling_mean" => MethodHandler::Native(crate::executor::objects::datatable_methods::handle_rolling_mean),
    "rollingMean" => MethodHandler::Native(crate::executor::objects::datatable_methods::handle_rolling_mean),
    "rolling_std" => MethodHandler::Native(crate::executor::objects::datatable_methods::handle_rolling_std),
    "rollingStd" => MethodHandler::Native(crate::executor::objects::datatable_methods::handle_rolling_std),
    "diff" => MethodHandler::Native(crate::executor::objects::datatable_methods::handle_diff),
    "pct_change" => MethodHandler::Native(crate::executor::objects::datatable_methods::handle_pct_change),
    "pctChange" => MethodHandler::Native(crate::executor::objects::datatable_methods::handle_pct_change),
    "forward_fill" => MethodHandler::Native(crate::executor::objects::datatable_methods::handle_forward_fill),
    "forwardFill" => MethodHandler::Native(crate::executor::objects::datatable_methods::handle_forward_fill),
};

/// PHF registry for Column methods (10 methods)
///
/// **Aggregation:** len, sum, mean, min, max, std
/// **Access:** first, last, toArray
/// **Transform:** abs
pub static COLUMN_METHODS: phf::Map<&'static str, MethodHandler> = phf_map! {
    "len" => MethodHandler::Native(crate::executor::objects::column_methods::v2_len),
    "length" => MethodHandler::Native(crate::executor::objects::column_methods::v2_len),
    "sum" => MethodHandler::Native(crate::executor::objects::column_methods::v2_sum),
    "mean" => MethodHandler::Native(crate::executor::objects::column_methods::v2_mean),
    "min" => MethodHandler::Native(crate::executor::objects::column_methods::v2_min),
    "max" => MethodHandler::Native(crate::executor::objects::column_methods::v2_max),
    "std" => MethodHandler::Native(crate::executor::objects::column_methods::v2_std),
    "first" => MethodHandler::Native(crate::executor::objects::column_methods::v2_first),
    "last" => MethodHandler::Native(crate::executor::objects::column_methods::v2_last),
    "toArray" => MethodHandler::Native(crate::executor::objects::column_methods::v2_to_array),
    "abs" => MethodHandler::Native(crate::executor::objects::column_methods::v2_abs),
};

/// PHF registry for HashMap methods (18 methods)
///
/// **Core:** get, set, has, delete, keys, values, entries, len, length, isEmpty
/// **Higher-order:** map, filter, forEach, reduce, groupBy
/// **Convenience:** merge, getOrDefault, toArray
pub static HASHMAP_METHODS: phf::Map<&'static str, MethodHandler> = phf_map! {
    // Non-closure methods — MethodFnV2
    "get" => MethodHandler::Native(crate::executor::objects::hashmap_methods::v2_get),
    "set" => MethodHandler::Native(crate::executor::objects::hashmap_methods::v2_set),
    "has" => MethodHandler::Native(crate::executor::objects::hashmap_methods::v2_has),
    "delete" => MethodHandler::Native(crate::executor::objects::hashmap_methods::v2_delete),
    "keys" => MethodHandler::Native(crate::executor::objects::hashmap_methods::v2_keys),
    "values" => MethodHandler::Native(crate::executor::objects::hashmap_methods::v2_values),
    "entries" => MethodHandler::Native(crate::executor::objects::hashmap_methods::v2_entries),
    "len" => MethodHandler::Native(crate::executor::objects::hashmap_methods::v2_len),
    "length" => MethodHandler::Native(crate::executor::objects::hashmap_methods::v2_len),
    "isEmpty" => MethodHandler::Native(crate::executor::objects::hashmap_methods::v2_is_empty),
    "merge" => MethodHandler::Native(crate::executor::objects::hashmap_methods::v2_merge),
    "getOrDefault" => MethodHandler::Native(crate::executor::objects::hashmap_methods::v2_get_or_default),
    "toArray" => MethodHandler::Native(crate::executor::objects::hashmap_methods::v2_to_array),
    // Closure-based — v2 native
    "map" => MethodHandler::Native(crate::executor::objects::hashmap_methods::v2_map),
    "filter" => MethodHandler::Native(crate::executor::objects::hashmap_methods::v2_filter),
    "forEach" => MethodHandler::Native(crate::executor::objects::hashmap_methods::v2_for_each),
    "reduce" => MethodHandler::Native(crate::executor::objects::hashmap_methods::v2_reduce),
    "groupBy" => MethodHandler::Native(crate::executor::objects::hashmap_methods::v2_group_by),

    // Iterator
    "iter" => MethodHandler::Native(crate::executor::objects::hashmap_methods::v2_iter),
};

/// PHF registry for Set methods (14 methods)
///
/// **Core:** add, has, delete, size, len, length, isEmpty, toArray
/// **Higher-order:** forEach, map, filter
/// **Set operations:** union, intersection, difference
pub static SET_METHODS: phf::Map<&'static str, MethodHandler> = phf_map! {
    "add" => MethodHandler::Native(crate::executor::objects::set_methods::v2_add),
    "delete" => MethodHandler::Native(crate::executor::objects::set_methods::v2_delete),
    // Read-only — MethodFnV2
    "has" => MethodHandler::Native(crate::executor::objects::set_methods::v2_has),
    "size" => MethodHandler::Native(crate::executor::objects::set_methods::v2_size),
    "len" => MethodHandler::Native(crate::executor::objects::set_methods::v2_size),
    "length" => MethodHandler::Native(crate::executor::objects::set_methods::v2_size),
    "isEmpty" => MethodHandler::Native(crate::executor::objects::set_methods::v2_is_empty),
    "toArray" => MethodHandler::Native(crate::executor::objects::set_methods::v2_to_array),
    "union" => MethodHandler::Native(crate::executor::objects::set_methods::v2_union),
    "intersection" => MethodHandler::Native(crate::executor::objects::set_methods::v2_intersection),
    "difference" => MethodHandler::Native(crate::executor::objects::set_methods::v2_difference),
    // Closure-based — v2 native
    "forEach" => MethodHandler::Native(crate::executor::objects::set_methods::v2_for_each),
    "map" => MethodHandler::Native(crate::executor::objects::set_methods::v2_map),
    "filter" => MethodHandler::Native(crate::executor::objects::set_methods::v2_filter),
};

/// PHF registry for Deque methods
///
/// **Mutation:** pushBack, pushFront, popBack, popFront
/// **Access:** peekBack, peekFront, get
/// **Info:** size, len, length, isEmpty
/// **Conversion:** toArray
pub static DEQUE_METHODS: phf::Map<&'static str, MethodHandler> = phf_map! {
    "pushBack" => MethodHandler::Native(crate::executor::objects::deque_methods::v2_push_back),
    "pushFront" => MethodHandler::Native(crate::executor::objects::deque_methods::v2_push_front),
    "popBack" => MethodHandler::Native(crate::executor::objects::deque_methods::v2_pop_back),
    "popFront" => MethodHandler::Native(crate::executor::objects::deque_methods::v2_pop_front),
    // Read-only — MethodFnV2
    "peekBack" => MethodHandler::Native(crate::executor::objects::deque_methods::v2_peek_back),
    "peekFront" => MethodHandler::Native(crate::executor::objects::deque_methods::v2_peek_front),
    "size" => MethodHandler::Native(crate::executor::objects::deque_methods::v2_size),
    "len" => MethodHandler::Native(crate::executor::objects::deque_methods::v2_size),
    "length" => MethodHandler::Native(crate::executor::objects::deque_methods::v2_size),
    "isEmpty" => MethodHandler::Native(crate::executor::objects::deque_methods::v2_is_empty),
    "toArray" => MethodHandler::Native(crate::executor::objects::deque_methods::v2_to_array),
    "get" => MethodHandler::Native(crate::executor::objects::deque_methods::v2_get),
};

/// PHF registry for PriorityQueue methods
///
/// **Mutation:** push, pop
/// **Access:** peek
/// **Info:** size, len, length, isEmpty
/// **Conversion:** toArray, toSortedArray
pub static PRIORITY_QUEUE_METHODS: phf::Map<&'static str, MethodHandler> = phf_map! {
    "push" => MethodHandler::Native(crate::executor::objects::priority_queue_methods::v2_push),
    "pop" => MethodHandler::Native(crate::executor::objects::priority_queue_methods::v2_pop),
    // Read-only — MethodFnV2
    "peek" => MethodHandler::Native(crate::executor::objects::priority_queue_methods::v2_peek),
    "size" => MethodHandler::Native(crate::executor::objects::priority_queue_methods::v2_size),
    "len" => MethodHandler::Native(crate::executor::objects::priority_queue_methods::v2_size),
    "length" => MethodHandler::Native(crate::executor::objects::priority_queue_methods::v2_size),
    "isEmpty" => MethodHandler::Native(crate::executor::objects::priority_queue_methods::v2_is_empty),
    "toArray" => MethodHandler::Native(crate::executor::objects::priority_queue_methods::v2_to_array),
    "toSortedArray" => MethodHandler::Native(crate::executor::objects::priority_queue_methods::v2_to_sorted_array),
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
    "year" => MethodHandler::Native(crate::executor::objects::datetime_methods::v2_year),
    "month" => MethodHandler::Native(crate::executor::objects::datetime_methods::v2_month),
    "day" => MethodHandler::Native(crate::executor::objects::datetime_methods::v2_day),
    "hour" => MethodHandler::Native(crate::executor::objects::datetime_methods::v2_hour),
    "minute" => MethodHandler::Native(crate::executor::objects::datetime_methods::v2_minute),
    "second" => MethodHandler::Native(crate::executor::objects::datetime_methods::v2_second),
    "millisecond" => MethodHandler::Native(crate::executor::objects::datetime_methods::v2_millisecond),
    "microsecond" => MethodHandler::Native(crate::executor::objects::datetime_methods::v2_microsecond),

    // Day info — Native
    "day_of_week" => MethodHandler::Native(crate::executor::objects::datetime_methods::v2_day_of_week),
    "day_of_year" => MethodHandler::Native(crate::executor::objects::datetime_methods::v2_day_of_year),
    "week_of_year" => MethodHandler::Native(crate::executor::objects::datetime_methods::v2_week_of_year),
    "is_weekday" => MethodHandler::Native(crate::executor::objects::datetime_methods::v2_is_weekday),
    "is_weekend" => MethodHandler::Native(crate::executor::objects::datetime_methods::v2_is_weekend),

    // Formatting — Native
    "format" => MethodHandler::Native(crate::executor::objects::datetime_methods::v2_format),
    "iso8601" => MethodHandler::Native(crate::executor::objects::datetime_methods::v2_iso8601),
    "rfc2822" => MethodHandler::Native(crate::executor::objects::datetime_methods::v2_rfc2822),
    "unix_timestamp" => MethodHandler::Native(crate::executor::objects::datetime_methods::v2_unix_timestamp),
    "to_unix_millis" => MethodHandler::Native(crate::executor::objects::datetime_methods::v2_to_unix_millis),

    // Timezone — Native
    "to_utc" => MethodHandler::Native(crate::executor::objects::datetime_methods::v2_to_utc),
    "to_timezone" => MethodHandler::Native(crate::executor::objects::datetime_methods::v2_to_timezone),
    "to_local" => MethodHandler::Native(crate::executor::objects::datetime_methods::v2_to_local),
    "timezone" => MethodHandler::Native(crate::executor::objects::datetime_methods::v2_timezone),
    "offset" => MethodHandler::Native(crate::executor::objects::datetime_methods::v2_offset),

    // Operator-trait arithmetic — Native
    "add" => MethodHandler::Native(crate::executor::objects::datetime_methods::v2_add),
    "sub" => MethodHandler::Native(crate::executor::objects::datetime_methods::v2_sub),

    // Arithmetic — Native
    "add_days" => MethodHandler::Native(crate::executor::objects::datetime_methods::v2_add_days),
    "add_hours" => MethodHandler::Native(crate::executor::objects::datetime_methods::v2_add_hours),
    "add_minutes" => MethodHandler::Native(crate::executor::objects::datetime_methods::v2_add_minutes),
    "add_seconds" => MethodHandler::Native(crate::executor::objects::datetime_methods::v2_add_seconds),
    "add_months" => MethodHandler::Native(crate::executor::objects::datetime_methods::v2_add_months),

    // Comparison — Native
    "is_before" => MethodHandler::Native(crate::executor::objects::datetime_methods::v2_is_before),
    "is_after" => MethodHandler::Native(crate::executor::objects::datetime_methods::v2_is_after),
    "is_same_day" => MethodHandler::Native(crate::executor::objects::datetime_methods::v2_is_same_day),

    // Diff — Native
    "diff" => MethodHandler::Native(crate::executor::objects::datetime_methods::v2_diff),
};

/// PHF registry for TimeSpan (Duration) methods.
///
/// **Operator-trait:** add, sub
pub static TIMESPAN_METHODS: phf::Map<&'static str, MethodHandler> = phf_map! {
    "add" => MethodHandler::Native(crate::executor::objects::datetime_methods::v2_timespan_add),
    "sub" => MethodHandler::Native(crate::executor::objects::datetime_methods::v2_timespan_sub),
};

/// PHF registry for Instant methods (6 methods)
///
/// **Timing:** elapsed, elapsed_ms, elapsed_us, elapsed_ns, duration_since
/// **Formatting:** to_string
pub static INSTANT_METHODS: phf::Map<&'static str, MethodHandler> = phf_map! {
    "elapsed" => MethodHandler::Native(crate::executor::objects::instant_methods::v2_elapsed),
    "elapsed_ms" => MethodHandler::Native(crate::executor::objects::instant_methods::v2_elapsed_ms),
    "elapsed_us" => MethodHandler::Native(crate::executor::objects::instant_methods::v2_elapsed_us),
    "elapsed_ns" => MethodHandler::Native(crate::executor::objects::instant_methods::v2_elapsed_ns),
    "duration_since" => MethodHandler::Native(crate::executor::objects::instant_methods::v2_duration_since),
    "to_string" => MethodHandler::Native(crate::executor::objects::instant_methods::v2_to_string),
};

/// PHF registry for Iterator methods (15 methods)
///
/// **Lazy transforms:** map, filter, take, skip, flatMap, enumerate, chain
/// **Terminal operations:** collect, toArray, forEach, reduce, count, any, all, find
pub static ITERATOR_METHODS: phf::Map<&'static str, MethodHandler> = phf_map! {
    // Lazy transforms (return new Iterator)
    "map" => MethodHandler::Native(crate::executor::objects::iterator_methods::handle_map),
    "filter" => MethodHandler::Native(crate::executor::objects::iterator_methods::handle_filter),
    "take" => MethodHandler::Native(crate::executor::objects::iterator_methods::handle_take),
    "skip" => MethodHandler::Native(crate::executor::objects::iterator_methods::handle_skip),
    "flatMap" => MethodHandler::Native(crate::executor::objects::iterator_methods::handle_flat_map),
    "enumerate" => MethodHandler::Native(crate::executor::objects::iterator_methods::handle_enumerate),
    "chain" => MethodHandler::Native(crate::executor::objects::iterator_methods::handle_chain),

    // Terminal operations (consume the iterator)
    "collect" => MethodHandler::Native(crate::executor::objects::iterator_methods::handle_collect),
    "toArray" => MethodHandler::Native(crate::executor::objects::iterator_methods::handle_collect),
    "forEach" => MethodHandler::Native(crate::executor::objects::iterator_methods::handle_for_each),
    "reduce" => MethodHandler::Native(crate::executor::objects::iterator_methods::handle_reduce),
    "count" => MethodHandler::Native(crate::executor::objects::iterator_methods::handle_count),
    "any" => MethodHandler::Native(crate::executor::objects::iterator_methods::handle_any),
    "all" => MethodHandler::Native(crate::executor::objects::iterator_methods::handle_all),
    "find" => MethodHandler::Native(crate::executor::objects::iterator_methods::handle_find),
};

/// PHF registry for Matrix methods (18 methods)
///
/// **Linear algebra:** transpose, inverse, det, determinant, trace
/// **Shape/access:** shape, reshape, row, col, diag, flatten
/// **Higher-order:** map
/// **Aggregation:** sum, min, max, mean, rowSum, colSum
pub static MATRIX_METHODS: phf::Map<&'static str, MethodHandler> = phf_map! {
    // Linear algebra — MethodFnV2
    "transpose" => MethodHandler::Native(crate::executor::objects::matrix_methods::v2_transpose),
    "inverse" => MethodHandler::Native(crate::executor::objects::matrix_methods::v2_inverse),
    "det" => MethodHandler::Native(crate::executor::objects::matrix_methods::v2_determinant),
    "determinant" => MethodHandler::Native(crate::executor::objects::matrix_methods::v2_determinant),
    "trace" => MethodHandler::Native(crate::executor::objects::matrix_methods::v2_trace),

    // Shape and access — MethodFnV2
    "shape" => MethodHandler::Native(crate::executor::objects::matrix_methods::v2_shape),
    "reshape" => MethodHandler::Native(crate::executor::objects::matrix_methods::v2_reshape),
    "row" => MethodHandler::Native(crate::executor::objects::matrix_methods::v2_row),
    "col" => MethodHandler::Native(crate::executor::objects::matrix_methods::v2_col),
    "diag" => MethodHandler::Native(crate::executor::objects::matrix_methods::v2_diag),
    "flatten" => MethodHandler::Native(crate::executor::objects::matrix_methods::v2_flatten),

    // Higher-order — stays Legacy (closure-based)
    "map" => MethodHandler::Native(crate::executor::objects::matrix_methods::handle_map),

    // Aggregation — MethodFnV2
    "sum" => MethodHandler::Native(crate::executor::objects::matrix_methods::v2_sum),
    "min" => MethodHandler::Native(crate::executor::objects::matrix_methods::v2_min),
    "max" => MethodHandler::Native(crate::executor::objects::matrix_methods::v2_max),
    "mean" => MethodHandler::Native(crate::executor::objects::matrix_methods::v2_mean),
    "rowSum" => MethodHandler::Native(crate::executor::objects::matrix_methods::v2_row_sum),
    "colSum" => MethodHandler::Native(crate::executor::objects::matrix_methods::v2_col_sum),
};

/// PHF registry for IndexedTable-specific methods (2 methods)
///
/// These methods require an IndexedTable (table with designated index column).
/// Inherited DataTable methods are dispatched via DATATABLE_METHODS fallback.
///
/// **Query:** between, resample
pub static INDEXED_TABLE_METHODS: phf::Map<&'static str, MethodHandler> = phf_map! {
    "between" => MethodHandler::Native(crate::executor::objects::indexed_table_methods::handle_between),
    "resample" => MethodHandler::Native(crate::executor::objects::indexed_table_methods::handle_resample),
};

/// PHF registry for Vec<number> (FloatArray) methods
///
/// **Aggregations:** sum, avg, mean, min, max, std, variance
/// **Numeric:** dot, norm, normalize, cumsum, diff, abs, sqrt, ln, exp
/// **Standard:** len, length, map, filter, forEach, toArray
pub static FLOAT_ARRAY_METHODS: phf::Map<&'static str, MethodHandler> = phf_map! {
    // Aggregations — MethodFnV2 (v2 typed array + v1 fallback)
    "sum" => MethodHandler::Native(crate::executor::objects::typed_array_methods::v2_float_sum),
    "avg" => MethodHandler::Native(crate::executor::objects::typed_array_methods::v2_float_avg),
    "mean" => MethodHandler::Native(crate::executor::objects::typed_array_methods::v2_float_avg),
    "min" => MethodHandler::Native(crate::executor::objects::typed_array_methods::v2_float_min),
    "max" => MethodHandler::Native(crate::executor::objects::typed_array_methods::v2_float_max),
    "std" => MethodHandler::Native(crate::executor::objects::typed_array_methods::v2_float_std),
    "variance" => MethodHandler::Native(crate::executor::objects::typed_array_methods::v2_float_variance),
    "dot" => MethodHandler::Native(crate::executor::objects::typed_array_methods::v2_float_dot),
    "norm" => MethodHandler::Native(crate::executor::objects::typed_array_methods::v2_float_norm),
    "len" => MethodHandler::Native(crate::executor::objects::typed_array_methods::v2_len),
    "length" => MethodHandler::Native(crate::executor::objects::typed_array_methods::v2_len),
    // Transforms — still Legacy (require VM callback invocation or produce arrays)
    "normalize" => MethodHandler::Native(crate::executor::objects::typed_array_methods::handle_float_normalize),
    "cumsum" => MethodHandler::Native(crate::executor::objects::typed_array_methods::handle_float_cumsum),
    "diff" => MethodHandler::Native(crate::executor::objects::typed_array_methods::handle_float_diff),
    "abs" => MethodHandler::Native(crate::executor::objects::typed_array_methods::handle_float_abs),
    "sqrt" => MethodHandler::Native(crate::executor::objects::typed_array_methods::handle_float_sqrt),
    "ln" => MethodHandler::Native(crate::executor::objects::typed_array_methods::handle_float_ln),
    "exp" => MethodHandler::Native(crate::executor::objects::typed_array_methods::handle_float_exp),
    "map" => MethodHandler::Native(crate::executor::objects::typed_array_methods::handle_float_map),
    "filter" => MethodHandler::Native(crate::executor::objects::typed_array_methods::handle_float_filter),
    "forEach" => MethodHandler::Native(crate::executor::objects::typed_array_methods::handle_float_for_each),
    "toArray" => MethodHandler::Native(crate::executor::objects::typed_array_methods::handle_float_to_array),
};

/// PHF registry for Vec<int> (IntArray) methods
///
/// **Aggregations:** sum, avg, mean, min, max
/// **Numeric:** abs
/// **Standard:** len, length, map, filter, forEach, toArray
pub static INT_ARRAY_METHODS: phf::Map<&'static str, MethodHandler> = phf_map! {
    // Aggregations — MethodFnV2 (v2 typed array + v1 fallback)
    "sum" => MethodHandler::Native(crate::executor::objects::typed_array_methods::v2_int_sum),
    "avg" => MethodHandler::Native(crate::executor::objects::typed_array_methods::v2_int_avg),
    "mean" => MethodHandler::Native(crate::executor::objects::typed_array_methods::v2_int_avg),
    "min" => MethodHandler::Native(crate::executor::objects::typed_array_methods::v2_int_min),
    "max" => MethodHandler::Native(crate::executor::objects::typed_array_methods::v2_int_max),
    "len" => MethodHandler::Native(crate::executor::objects::typed_array_methods::v2_len),
    "length" => MethodHandler::Native(crate::executor::objects::typed_array_methods::v2_len),
    // Transforms — still Legacy
    "abs" => MethodHandler::Native(crate::executor::objects::typed_array_methods::handle_int_abs),
    "map" => MethodHandler::Native(crate::executor::objects::typed_array_methods::handle_int_map),
    "filter" => MethodHandler::Native(crate::executor::objects::typed_array_methods::handle_int_filter),
    "forEach" => MethodHandler::Native(crate::executor::objects::typed_array_methods::handle_int_for_each),
    "toArray" => MethodHandler::Native(crate::executor::objects::typed_array_methods::handle_int_to_array),
};

/// PHF registry for Vec<bool> (BoolArray) methods
///
/// **Standard:** len, length, toArray
/// **Query:** any, all, count
pub static BOOL_ARRAY_METHODS: phf::Map<&'static str, MethodHandler> = phf_map! {
    // MethodFnV2 (v2 typed array + v1 fallback)
    "len" => MethodHandler::Native(crate::executor::objects::typed_array_methods::v2_len),
    "length" => MethodHandler::Native(crate::executor::objects::typed_array_methods::v2_len),
    "count" => MethodHandler::Native(crate::executor::objects::typed_array_methods::v2_bool_count),
    "any" => MethodHandler::Native(crate::executor::objects::typed_array_methods::v2_bool_any),
    "all" => MethodHandler::Native(crate::executor::objects::typed_array_methods::v2_bool_all),
    // Still Legacy
    "toArray" => MethodHandler::Native(crate::executor::objects::typed_array_methods::handle_bool_to_array),
};

// ═══════════════════════════════════════════════════════════════════════════
// Concurrency primitives — compiler-builtin interior mutability types
// ═══════════════════════════════════════════════════════════════════════════

/// Mutex<T> methods: lock, try_lock, set
pub static MUTEX_METHODS: phf::Map<&'static str, MethodHandler> = phf_map! {
    "lock" => MethodHandler::Native(crate::executor::objects::concurrency_methods::v2_mutex_lock),
    "try_lock" => MethodHandler::Native(crate::executor::objects::concurrency_methods::v2_mutex_try_lock),
    "set" => MethodHandler::Native(crate::executor::objects::concurrency_methods::v2_mutex_set),
};

/// Atomic<T> methods: load, store, fetch_add, fetch_sub, compare_exchange
pub static ATOMIC_METHODS: phf::Map<&'static str, MethodHandler> = phf_map! {
    "load" => MethodHandler::Native(crate::executor::objects::concurrency_methods::v2_atomic_load),
    "store" => MethodHandler::Native(crate::executor::objects::concurrency_methods::v2_atomic_store),
    "fetch_add" => MethodHandler::Native(crate::executor::objects::concurrency_methods::v2_atomic_fetch_add),
    "fetch_sub" => MethodHandler::Native(crate::executor::objects::concurrency_methods::v2_atomic_fetch_sub),
    "compare_exchange" => MethodHandler::Native(crate::executor::objects::concurrency_methods::v2_atomic_compare_exchange),
};

/// Lazy<T> methods: get, is_initialized
pub static LAZY_METHODS: phf::Map<&'static str, MethodHandler> = phf_map! {
    "get" => MethodHandler::Native(crate::executor::objects::concurrency_methods::v2_lazy_get),
    "is_initialized" => MethodHandler::Native(crate::executor::objects::concurrency_methods::v2_lazy_is_initialized),
};

/// Channel methods: send, recv, try_recv, close, is_closed, is_sender
pub static CHANNEL_METHODS: phf::Map<&'static str, MethodHandler> = phf_map! {
    "send" => MethodHandler::Native(crate::executor::objects::channel_methods::v2_channel_send),
    "recv" => MethodHandler::Native(crate::executor::objects::channel_methods::v2_channel_recv),
    "try_recv" => MethodHandler::Native(crate::executor::objects::channel_methods::v2_channel_try_recv),
    "close" => MethodHandler::Native(crate::executor::objects::channel_methods::v2_channel_close),
    "is_closed" => MethodHandler::Native(crate::executor::objects::channel_methods::v2_channel_is_closed),
    "is_sender" => MethodHandler::Native(crate::executor::objects::channel_methods::v2_channel_is_sender),
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
    "floor" => MethodHandler::Native(crate::executor::objects::number_methods::number_floor_v2),
    "ceil" => MethodHandler::Native(crate::executor::objects::number_methods::number_ceil_v2),
    "round" => MethodHandler::Native(crate::executor::objects::number_methods::number_round_v2),
    "abs" => MethodHandler::Native(crate::executor::objects::number_methods::number_abs_v2),
    "sign" => MethodHandler::Native(crate::executor::objects::number_methods::number_sign_v2),
    "toInt" => MethodHandler::Native(crate::executor::objects::number_methods::number_to_int_v2),
    "to_int" => MethodHandler::Native(crate::executor::objects::number_methods::number_to_int_v2),
    "toNumber" => MethodHandler::Native(crate::executor::objects::number_methods::number_to_number_v2),
    "to_number" => MethodHandler::Native(crate::executor::objects::number_methods::number_to_number_v2),
    "isNaN" => MethodHandler::Native(crate::executor::objects::number_methods::number_is_nan_v2),
    "is_nan" => MethodHandler::Native(crate::executor::objects::number_methods::number_is_nan_v2),
    "isFinite" => MethodHandler::Native(crate::executor::objects::number_methods::number_is_finite_v2),
    "is_finite" => MethodHandler::Native(crate::executor::objects::number_methods::number_is_finite_v2),
    "toFixed" => MethodHandler::Native(crate::executor::objects::number_methods::number_to_fixed_v2),
    "to_fixed" => MethodHandler::Native(crate::executor::objects::number_methods::number_to_fixed_v2),
    "toString" => MethodHandler::Native(crate::executor::objects::number_methods::number_to_string_v2),
    "to_string" => MethodHandler::Native(crate::executor::objects::number_methods::number_to_string_v2),
    "clamp" => MethodHandler::Native(crate::executor::objects::number_methods::number_clamp_v2),
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
    "len" => MethodHandler::Native(crate::executor::objects::string_methods::v2_string_len),
    "length" => MethodHandler::Native(crate::executor::objects::string_methods::v2_string_len),

    // Case
    "toUpperCase" => MethodHandler::Native(crate::executor::objects::string_methods::v2_string_to_upper),
    "to_upper_case" => MethodHandler::Native(crate::executor::objects::string_methods::v2_string_to_upper),
    "toLowerCase" => MethodHandler::Native(crate::executor::objects::string_methods::v2_string_to_lower),
    "to_lower_case" => MethodHandler::Native(crate::executor::objects::string_methods::v2_string_to_lower),

    // Whitespace
    "trim" => MethodHandler::Native(crate::executor::objects::string_methods::v2_string_trim),
    "trimStart" => MethodHandler::Native(crate::executor::objects::string_methods::v2_string_trim_start),
    "trim_start" => MethodHandler::Native(crate::executor::objects::string_methods::v2_string_trim_start),
    "trimEnd" => MethodHandler::Native(crate::executor::objects::string_methods::v2_string_trim_end),
    "trim_end" => MethodHandler::Native(crate::executor::objects::string_methods::v2_string_trim_end),

    // Conversion / identity
    "toString" => MethodHandler::Native(crate::executor::objects::string_methods::v2_string_to_string),
    "to_string" => MethodHandler::Native(crate::executor::objects::string_methods::v2_string_to_string),

    // Search
    "startsWith" => MethodHandler::Native(crate::executor::objects::string_methods::v2_string_starts_with),
    "starts_with" => MethodHandler::Native(crate::executor::objects::string_methods::v2_string_starts_with),
    "endsWith" => MethodHandler::Native(crate::executor::objects::string_methods::v2_string_ends_with),
    "ends_with" => MethodHandler::Native(crate::executor::objects::string_methods::v2_string_ends_with),
    "contains" => MethodHandler::Native(crate::executor::objects::string_methods::v2_string_contains),
    "indexOf" => MethodHandler::Native(crate::executor::objects::string_methods::v2_string_index_of),
    "index_of" => MethodHandler::Native(crate::executor::objects::string_methods::v2_string_index_of),

    // Transform
    "repeat" => MethodHandler::Native(crate::executor::objects::string_methods::v2_string_repeat),
    "charAt" => MethodHandler::Native(crate::executor::objects::string_methods::v2_string_char_at),
    "char_at" => MethodHandler::Native(crate::executor::objects::string_methods::v2_string_char_at),
    "reverse" => MethodHandler::Native(crate::executor::objects::string_methods::v2_string_reverse),
    "split" => MethodHandler::Native(crate::executor::objects::string_methods::v2_string_split),
    "replace" => MethodHandler::Native(crate::executor::objects::string_methods::v2_string_replace),
    "substring" => MethodHandler::Native(crate::executor::objects::string_methods::v2_string_substring),
    "join" => MethodHandler::Native(crate::executor::objects::string_methods::v2_string_join),

    // Padding
    "padStart" => MethodHandler::Native(crate::executor::objects::string_methods::v2_string_pad_start),
    "pad_start" => MethodHandler::Native(crate::executor::objects::string_methods::v2_string_pad_start),
    "padEnd" => MethodHandler::Native(crate::executor::objects::string_methods::v2_string_pad_end),
    "pad_end" => MethodHandler::Native(crate::executor::objects::string_methods::v2_string_pad_end),

    // Predicates
    "isDigit" => MethodHandler::Native(crate::executor::objects::string_methods::v2_string_is_digit),
    "is_digit" => MethodHandler::Native(crate::executor::objects::string_methods::v2_string_is_digit),
    "isAlpha" => MethodHandler::Native(crate::executor::objects::string_methods::v2_string_is_alpha),
    "is_alpha" => MethodHandler::Native(crate::executor::objects::string_methods::v2_string_is_alpha),
    "isAscii" => MethodHandler::Native(crate::executor::objects::string_methods::v2_string_is_ascii),
    "is_ascii" => MethodHandler::Native(crate::executor::objects::string_methods::v2_string_is_ascii),

    // Numeric conversion
    "toInt" => MethodHandler::Native(crate::executor::objects::string_methods::v2_string_to_int),
    "to_int" => MethodHandler::Native(crate::executor::objects::string_methods::v2_string_to_int),
    "toNumber" => MethodHandler::Native(crate::executor::objects::string_methods::v2_string_to_number),
    "to_number" => MethodHandler::Native(crate::executor::objects::string_methods::v2_string_to_number),
    "toFloat" => MethodHandler::Native(crate::executor::objects::string_methods::v2_string_to_number),
    "to_float" => MethodHandler::Native(crate::executor::objects::string_methods::v2_string_to_number),

    // Unicode
    "codePointAt" => MethodHandler::Native(crate::executor::objects::string_methods::v2_string_code_point_at),
    "code_point_at" => MethodHandler::Native(crate::executor::objects::string_methods::v2_string_code_point_at),
    "graphemeLen" => MethodHandler::Native(crate::executor::objects::string_methods::v2_string_grapheme_len),
    "grapheme_len" => MethodHandler::Native(crate::executor::objects::string_methods::v2_string_grapheme_len),
    "graphemes" => MethodHandler::Native(crate::executor::objects::string_methods::v2_string_graphemes),
    "normalize" => MethodHandler::Native(crate::executor::objects::string_methods::v2_string_normalize),
    "iter" => MethodHandler::Native(crate::executor::objects::string_methods::v2_string_iter),
};

/// PHF registry for Bool methods
pub static BOOL_METHODS: phf::Map<&'static str, MethodHandler> = phf_map! {
    "toString" => MethodHandler::Native(crate::executor::objects::number_methods::bool_to_string_v2),
    "to_string" => MethodHandler::Native(crate::executor::objects::number_methods::bool_to_string_v2),
};

/// PHF registry for Char methods (11 methods)
///
/// **Predicates:** is_alphabetic, is_numeric, is_alphanumeric, is_whitespace, is_uppercase, is_lowercase, is_ascii
/// **Transform:** to_uppercase, to_lowercase
/// **Conversion:** to_string, toString
pub static CHAR_METHODS: phf::Map<&'static str, MethodHandler> = phf_map! {
    "is_alphabetic" => MethodHandler::Native(crate::executor::objects::number_methods::char_is_alphabetic_v2),
    "isAlphabetic" => MethodHandler::Native(crate::executor::objects::number_methods::char_is_alphabetic_v2),
    "is_numeric" => MethodHandler::Native(crate::executor::objects::number_methods::char_is_numeric_v2),
    "isNumeric" => MethodHandler::Native(crate::executor::objects::number_methods::char_is_numeric_v2),
    "is_alphanumeric" => MethodHandler::Native(crate::executor::objects::number_methods::char_is_alphanumeric_v2),
    "isAlphanumeric" => MethodHandler::Native(crate::executor::objects::number_methods::char_is_alphanumeric_v2),
    "is_whitespace" => MethodHandler::Native(crate::executor::objects::number_methods::char_is_whitespace_v2),
    "isWhitespace" => MethodHandler::Native(crate::executor::objects::number_methods::char_is_whitespace_v2),
    "is_uppercase" => MethodHandler::Native(crate::executor::objects::number_methods::char_is_uppercase_v2),
    "isUppercase" => MethodHandler::Native(crate::executor::objects::number_methods::char_is_uppercase_v2),
    "is_lowercase" => MethodHandler::Native(crate::executor::objects::number_methods::char_is_lowercase_v2),
    "isLowercase" => MethodHandler::Native(crate::executor::objects::number_methods::char_is_lowercase_v2),
    "is_ascii" => MethodHandler::Native(crate::executor::objects::number_methods::char_is_ascii_v2),
    "isAscii" => MethodHandler::Native(crate::executor::objects::number_methods::char_is_ascii_v2),
    "to_uppercase" => MethodHandler::Native(crate::executor::objects::number_methods::char_to_uppercase_v2),
    "toUppercase" => MethodHandler::Native(crate::executor::objects::number_methods::char_to_uppercase_v2),
    "to_lowercase" => MethodHandler::Native(crate::executor::objects::number_methods::char_to_lowercase_v2),
    "toLowercase" => MethodHandler::Native(crate::executor::objects::number_methods::char_to_lowercase_v2),
    "to_string" => MethodHandler::Native(crate::executor::objects::number_methods::char_to_string_v2),
    "toString" => MethodHandler::Native(crate::executor::objects::number_methods::char_to_string_v2),
};

/// PHF registry for Content methods
///
/// **Style:** bold, italic, underline, dim, fg, bg
/// **Table/Chart:** border, max_rows, maxRows, series, title, x_label, xLabel, y_label, yLabel
/// **Conversion:** toString
pub static CONTENT_METHODS: phf::Map<&'static str, MethodHandler> = phf_map! {
    "bold" => MethodHandler::Native(crate::executor::objects::content_methods::v2_content_bold),
    "italic" => MethodHandler::Native(crate::executor::objects::content_methods::v2_content_italic),
    "underline" => MethodHandler::Native(crate::executor::objects::content_methods::v2_content_underline),
    "dim" => MethodHandler::Native(crate::executor::objects::content_methods::v2_content_dim),
    "fg" => MethodHandler::Native(crate::executor::objects::content_methods::v2_content_fg),
    "bg" => MethodHandler::Native(crate::executor::objects::content_methods::v2_content_bg),
    "toString" => MethodHandler::Native(crate::executor::objects::content_methods::v2_content_to_string),
    "border" => MethodHandler::Native(crate::executor::objects::content_methods::v2_content_border),
    "max_rows" => MethodHandler::Native(crate::executor::objects::content_methods::v2_content_max_rows),
    "maxRows" => MethodHandler::Native(crate::executor::objects::content_methods::v2_content_max_rows_camel),
    "series" => MethodHandler::Native(crate::executor::objects::content_methods::v2_content_series),
    "title" => MethodHandler::Native(crate::executor::objects::content_methods::v2_content_title),
    "x_label" => MethodHandler::Native(crate::executor::objects::content_methods::v2_content_x_label),
    "xLabel" => MethodHandler::Native(crate::executor::objects::content_methods::v2_content_x_label_camel),
    "y_label" => MethodHandler::Native(crate::executor::objects::content_methods::v2_content_y_label),
    "yLabel" => MethodHandler::Native(crate::executor::objects::content_methods::v2_content_y_label_camel),
};

/// PHF registry for Range methods
pub static RANGE_METHODS: phf::Map<&'static str, MethodHandler> = phf_map! {
    "iter" => MethodHandler::Native(crate::executor::objects::iterator_methods::v2_range_iter),
};
