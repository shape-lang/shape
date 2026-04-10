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
    "reduce" => MethodHandler::Legacy(crate::executor::objects::array_aggregation::handle_reduce),
    "fold" => MethodHandler::Legacy(crate::executor::objects::array_aggregation::handle_reduce),
    "forEach" => MethodHandler::Native(crate::executor::objects::array_query::handle_for_each_v2),
    "find" => MethodHandler::Native(crate::executor::objects::array_query::handle_find_v2),
    "findIndex" => MethodHandler::Native(crate::executor::objects::array_query::handle_find_index_v2),
    "some" => MethodHandler::Native(crate::executor::objects::array_query::handle_some_v2),
    "every" => MethodHandler::Native(crate::executor::objects::array_query::handle_every_v2),
    "sort" => MethodHandler::Native(crate::executor::objects::array_transform::handle_sort_v2),
    "groupBy" => MethodHandler::Native(crate::executor::objects::array_transform::handle_group_by_v2),
    "flatMap" => MethodHandler::Native(crate::executor::objects::array_transform::handle_flat_map_v2),

    // Basic operations — still Legacy (waiting for array-basic agent)
    "len" => MethodHandler::Legacy(crate::executor::objects::array_basic::handle_len),
    "length" => MethodHandler::Legacy(crate::executor::objects::array_basic::handle_length),
    "first" => MethodHandler::Legacy(crate::executor::objects::array_basic::handle_first),
    "last" => MethodHandler::Legacy(crate::executor::objects::array_basic::handle_last),
    "reverse" => MethodHandler::Legacy(crate::executor::objects::array_basic::handle_reverse),
    "push" => MethodHandler::Legacy(crate::executor::objects::array_basic::handle_push),
    "pop" => MethodHandler::Legacy(crate::executor::objects::array_basic::handle_pop),
    "zip" => MethodHandler::Legacy(crate::executor::objects::array_basic::handle_zip),
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
    "unique" => MethodHandler::Legacy(crate::executor::objects::array_sets::handle_unique),
    "distinct" => MethodHandler::Legacy(crate::executor::objects::array_sets::handle_distinct),
    "distinctBy" => MethodHandler::Legacy(crate::executor::objects::array_sets::handle_distinct_by),

    // Aggregation — still Legacy (waiting for array-basic agent)
    "sum" => MethodHandler::Legacy(crate::executor::objects::array_aggregation::handle_sum),
    "avg" => MethodHandler::Legacy(crate::executor::objects::array_aggregation::handle_avg),
    "min" => MethodHandler::Legacy(crate::executor::objects::array_aggregation::handle_min),
    "max" => MethodHandler::Legacy(crate::executor::objects::array_aggregation::handle_max),
    "count" => MethodHandler::Legacy(crate::executor::objects::array_aggregation::handle_count),

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

    // Join operations — still Legacy (waiting for array-basic agent)
    "innerJoin" => MethodHandler::Legacy(crate::executor::objects::array_joins::handle_inner_join),
    "leftJoin" => MethodHandler::Legacy(crate::executor::objects::array_joins::handle_left_join),
    "crossJoin" => MethodHandler::Legacy(crate::executor::objects::array_joins::handle_cross_join),

    // Set operations — still Legacy (waiting for array-basic agent)
    "union" => MethodHandler::Legacy(crate::executor::objects::array_sets::handle_union),
    "intersect" => MethodHandler::Legacy(crate::executor::objects::array_sets::handle_intersect),
    "except" => MethodHandler::Legacy(crate::executor::objects::array_sets::handle_except),

    // Clone — still Legacy
    "clone" => MethodHandler::Legacy(crate::executor::objects::array_basic::handle_clone),

    // Iterator — still Legacy
    "iter" => MethodHandler::Legacy(crate::executor::objects::iterator_methods::handle_array_iter),
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
    "origin" => MethodHandler::Legacy(crate::executor::objects::datatable_methods::handle_origin),
    "len" => MethodHandler::Legacy(crate::executor::objects::datatable_methods::handle_len),
    "length" => MethodHandler::Legacy(crate::executor::objects::datatable_methods::handle_len),
    "columns" => MethodHandler::Legacy(crate::executor::objects::datatable_methods::handle_columns),
    "column" => MethodHandler::Legacy(crate::executor::objects::datatable_methods::handle_column),
    "slice" => MethodHandler::Legacy(crate::executor::objects::datatable_methods::handle_slice),
    "head" => MethodHandler::Legacy(crate::executor::objects::datatable_methods::handle_head),
    "tail" => MethodHandler::Legacy(crate::executor::objects::datatable_methods::handle_tail),
    "first" => MethodHandler::Legacy(crate::executor::objects::datatable_methods::handle_first),
    "last" => MethodHandler::Legacy(crate::executor::objects::datatable_methods::handle_last),
    "select" => MethodHandler::Legacy(crate::executor::objects::datatable_methods::handle_select),
    "toMat" => MethodHandler::Legacy(crate::executor::objects::datatable_methods::handle_to_mat),
    "to_mat" => MethodHandler::Legacy(crate::executor::objects::datatable_methods::handle_to_mat),

    // Row/column iteration
    "rows" => MethodHandler::Legacy(crate::executor::objects::datatable_methods::handle_rows),
    "columnsRef" => MethodHandler::Legacy(crate::executor::objects::datatable_methods::handle_columns_ref),

    // Compute
    "sum" => MethodHandler::Legacy(crate::executor::objects::datatable_methods::handle_sum),
    "mean" => MethodHandler::Legacy(crate::executor::objects::datatable_methods::handle_mean),
    "min" => MethodHandler::Legacy(crate::executor::objects::datatable_methods::handle_min),
    "max" => MethodHandler::Legacy(crate::executor::objects::datatable_methods::handle_max),
    "sort" => MethodHandler::Legacy(crate::executor::objects::datatable_methods::handle_sort),

    // Query (Phase 4)
    "filter" => MethodHandler::Legacy(crate::executor::objects::datatable_methods::handle_filter),
    "orderBy" => MethodHandler::Legacy(crate::executor::objects::datatable_methods::handle_order_by),
    "group_by" => MethodHandler::Legacy(crate::executor::objects::datatable_methods::handle_group_by),
    "groupBy" => MethodHandler::Legacy(crate::executor::objects::datatable_methods::handle_group_by),
    "aggregate" => MethodHandler::Legacy(crate::executor::objects::datatable_methods::handle_aggregate),
    "count" => MethodHandler::Legacy(crate::executor::objects::datatable_methods::handle_count),
    "describe" => MethodHandler::Legacy(crate::executor::objects::datatable_methods::handle_describe),
    "forEach" => MethodHandler::Legacy(crate::executor::objects::datatable_methods::handle_for_each),
    "map" => MethodHandler::Legacy(crate::executor::objects::datatable_methods::handle_map),
    "index_by" => MethodHandler::Legacy(crate::executor::objects::datatable_methods::handle_index_by),
    "indexBy" => MethodHandler::Legacy(crate::executor::objects::datatable_methods::handle_index_by),

    // Queryable interface (consistent with DbTable)
    "limit" => MethodHandler::Legacy(crate::executor::objects::datatable_methods::handle_limit),
    "execute" => MethodHandler::Legacy(crate::executor::objects::datatable_methods::handle_execute),

    // Joins
    "innerJoin" => MethodHandler::Legacy(crate::executor::objects::datatable_methods::handle_inner_join),
    "leftJoin" => MethodHandler::Legacy(crate::executor::objects::datatable_methods::handle_left_join),

    // Simulation
    "simulate" => MethodHandler::Legacy(crate::executor::objects::datatable_methods::handle_simulate),

    // SIMD-backed methods
    "correlation" => MethodHandler::Legacy(crate::executor::objects::datatable_methods::handle_correlation),
    "covariance" => MethodHandler::Legacy(crate::executor::objects::datatable_methods::handle_covariance),
    "rolling_sum" => MethodHandler::Legacy(crate::executor::objects::datatable_methods::handle_rolling_sum),
    "rollingSum" => MethodHandler::Legacy(crate::executor::objects::datatable_methods::handle_rolling_sum),
    "rolling_mean" => MethodHandler::Legacy(crate::executor::objects::datatable_methods::handle_rolling_mean),
    "rollingMean" => MethodHandler::Legacy(crate::executor::objects::datatable_methods::handle_rolling_mean),
    "rolling_std" => MethodHandler::Legacy(crate::executor::objects::datatable_methods::handle_rolling_std),
    "rollingStd" => MethodHandler::Legacy(crate::executor::objects::datatable_methods::handle_rolling_std),
    "diff" => MethodHandler::Legacy(crate::executor::objects::datatable_methods::handle_diff),
    "pct_change" => MethodHandler::Legacy(crate::executor::objects::datatable_methods::handle_pct_change),
    "pctChange" => MethodHandler::Legacy(crate::executor::objects::datatable_methods::handle_pct_change),
    "forward_fill" => MethodHandler::Legacy(crate::executor::objects::datatable_methods::handle_forward_fill),
    "forwardFill" => MethodHandler::Legacy(crate::executor::objects::datatable_methods::handle_forward_fill),
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
    // Closure-based — stay Legacy
    "map" => MethodHandler::Legacy(crate::executor::objects::hashmap_methods::handle_map),
    "filter" => MethodHandler::Legacy(crate::executor::objects::hashmap_methods::handle_filter),
    "forEach" => MethodHandler::Legacy(crate::executor::objects::hashmap_methods::handle_for_each),
    "reduce" => MethodHandler::Legacy(crate::executor::objects::hashmap_methods::handle_reduce),
    "groupBy" => MethodHandler::Legacy(crate::executor::objects::hashmap_methods::handle_group_by),

    // Iterator
    "iter" => MethodHandler::Legacy(crate::executor::objects::iterator_methods::handle_hashmap_iter),
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
    // Closure-based — stay Legacy
    "forEach" => MethodHandler::Legacy(crate::executor::objects::set_methods::handle_for_each),
    "map" => MethodHandler::Legacy(crate::executor::objects::set_methods::handle_map),
    "filter" => MethodHandler::Legacy(crate::executor::objects::set_methods::handle_filter),
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
    // Component access
    "year" => MethodHandler::Legacy(crate::executor::objects::datetime_methods::handle_year),
    "month" => MethodHandler::Legacy(crate::executor::objects::datetime_methods::handle_month),
    "day" => MethodHandler::Legacy(crate::executor::objects::datetime_methods::handle_day),
    "hour" => MethodHandler::Legacy(crate::executor::objects::datetime_methods::handle_hour),
    "minute" => MethodHandler::Legacy(crate::executor::objects::datetime_methods::handle_minute),
    "second" => MethodHandler::Legacy(crate::executor::objects::datetime_methods::handle_second),
    "millisecond" => MethodHandler::Legacy(crate::executor::objects::datetime_methods::handle_millisecond),
    "microsecond" => MethodHandler::Legacy(crate::executor::objects::datetime_methods::handle_microsecond),

    // Day info
    "day_of_week" => MethodHandler::Legacy(crate::executor::objects::datetime_methods::handle_day_of_week),
    "day_of_year" => MethodHandler::Legacy(crate::executor::objects::datetime_methods::handle_day_of_year),
    "week_of_year" => MethodHandler::Legacy(crate::executor::objects::datetime_methods::handle_week_of_year),
    "is_weekday" => MethodHandler::Legacy(crate::executor::objects::datetime_methods::handle_is_weekday),
    "is_weekend" => MethodHandler::Legacy(crate::executor::objects::datetime_methods::handle_is_weekend),

    // Formatting
    "format" => MethodHandler::Legacy(crate::executor::objects::datetime_methods::handle_format),
    "iso8601" => MethodHandler::Legacy(crate::executor::objects::datetime_methods::handle_iso8601),
    "rfc2822" => MethodHandler::Legacy(crate::executor::objects::datetime_methods::handle_rfc2822),
    "unix_timestamp" => MethodHandler::Legacy(crate::executor::objects::datetime_methods::handle_unix_timestamp),
    "to_unix_millis" => MethodHandler::Legacy(crate::executor::objects::datetime_methods::handle_to_unix_millis),

    // Timezone
    "to_utc" => MethodHandler::Legacy(crate::executor::objects::datetime_methods::handle_to_utc),
    "to_timezone" => MethodHandler::Legacy(crate::executor::objects::datetime_methods::handle_to_timezone),
    "to_local" => MethodHandler::Legacy(crate::executor::objects::datetime_methods::handle_to_local),
    "timezone" => MethodHandler::Legacy(crate::executor::objects::datetime_methods::handle_timezone),
    "offset" => MethodHandler::Legacy(crate::executor::objects::datetime_methods::handle_offset),

    // Operator-trait arithmetic (add/sub for temporal binary ops)
    "add" => MethodHandler::Legacy(crate::executor::objects::datetime_methods::handle_add),
    "sub" => MethodHandler::Legacy(crate::executor::objects::datetime_methods::handle_sub),

    // Arithmetic
    "add_days" => MethodHandler::Legacy(crate::executor::objects::datetime_methods::handle_add_days),
    "add_hours" => MethodHandler::Legacy(crate::executor::objects::datetime_methods::handle_add_hours),
    "add_minutes" => MethodHandler::Legacy(crate::executor::objects::datetime_methods::handle_add_minutes),
    "add_seconds" => MethodHandler::Legacy(crate::executor::objects::datetime_methods::handle_add_seconds),
    "add_months" => MethodHandler::Legacy(crate::executor::objects::datetime_methods::handle_add_months),

    // Comparison
    "is_before" => MethodHandler::Legacy(crate::executor::objects::datetime_methods::handle_is_before),
    "is_after" => MethodHandler::Legacy(crate::executor::objects::datetime_methods::handle_is_after),
    "is_same_day" => MethodHandler::Legacy(crate::executor::objects::datetime_methods::handle_is_same_day),

    // Diff
    "diff" => MethodHandler::Legacy(crate::executor::objects::datetime_methods::handle_diff),
};

/// PHF registry for TimeSpan (Duration) methods.
///
/// **Operator-trait:** add, sub
pub static TIMESPAN_METHODS: phf::Map<&'static str, MethodHandler> = phf_map! {
    "add" => MethodHandler::Legacy(crate::executor::objects::datetime_methods::handle_timespan_add),
    "sub" => MethodHandler::Legacy(crate::executor::objects::datetime_methods::handle_timespan_sub),
};

/// PHF registry for Instant methods (6 methods)
///
/// **Timing:** elapsed, elapsed_ms, elapsed_us, elapsed_ns, duration_since
/// **Formatting:** to_string
pub static INSTANT_METHODS: phf::Map<&'static str, MethodHandler> = phf_map! {
    "elapsed" => MethodHandler::Legacy(crate::executor::objects::instant_methods::handle_elapsed),
    "elapsed_ms" => MethodHandler::Legacy(crate::executor::objects::instant_methods::handle_elapsed_ms),
    "elapsed_us" => MethodHandler::Legacy(crate::executor::objects::instant_methods::handle_elapsed_us),
    "elapsed_ns" => MethodHandler::Legacy(crate::executor::objects::instant_methods::handle_elapsed_ns),
    "duration_since" => MethodHandler::Legacy(crate::executor::objects::instant_methods::handle_duration_since),
    "to_string" => MethodHandler::Legacy(crate::executor::objects::instant_methods::handle_to_string),
};

/// PHF registry for Iterator methods (15 methods)
///
/// **Lazy transforms:** map, filter, take, skip, flatMap, enumerate, chain
/// **Terminal operations:** collect, toArray, forEach, reduce, count, any, all, find
pub static ITERATOR_METHODS: phf::Map<&'static str, MethodHandler> = phf_map! {
    // Lazy transforms (return new Iterator)
    "map" => MethodHandler::Legacy(crate::executor::objects::iterator_methods::handle_map),
    "filter" => MethodHandler::Legacy(crate::executor::objects::iterator_methods::handle_filter),
    "take" => MethodHandler::Legacy(crate::executor::objects::iterator_methods::handle_take),
    "skip" => MethodHandler::Legacy(crate::executor::objects::iterator_methods::handle_skip),
    "flatMap" => MethodHandler::Legacy(crate::executor::objects::iterator_methods::handle_flat_map),
    "enumerate" => MethodHandler::Legacy(crate::executor::objects::iterator_methods::handle_enumerate),
    "chain" => MethodHandler::Legacy(crate::executor::objects::iterator_methods::handle_chain),

    // Terminal operations (consume the iterator)
    "collect" => MethodHandler::Legacy(crate::executor::objects::iterator_methods::handle_collect),
    "toArray" => MethodHandler::Legacy(crate::executor::objects::iterator_methods::handle_collect),
    "forEach" => MethodHandler::Legacy(crate::executor::objects::iterator_methods::handle_for_each),
    "reduce" => MethodHandler::Legacy(crate::executor::objects::iterator_methods::handle_reduce),
    "count" => MethodHandler::Legacy(crate::executor::objects::iterator_methods::handle_count),
    "any" => MethodHandler::Legacy(crate::executor::objects::iterator_methods::handle_any),
    "all" => MethodHandler::Legacy(crate::executor::objects::iterator_methods::handle_all),
    "find" => MethodHandler::Legacy(crate::executor::objects::iterator_methods::handle_find),
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
    "map" => MethodHandler::Legacy(crate::executor::objects::matrix_methods::handle_map),

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
    "between" => MethodHandler::Legacy(crate::executor::objects::indexed_table_methods::handle_between),
    "resample" => MethodHandler::Legacy(crate::executor::objects::indexed_table_methods::handle_resample),
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
    "normalize" => MethodHandler::Legacy(crate::executor::objects::typed_array_methods::handle_float_normalize),
    "cumsum" => MethodHandler::Legacy(crate::executor::objects::typed_array_methods::handle_float_cumsum),
    "diff" => MethodHandler::Legacy(crate::executor::objects::typed_array_methods::handle_float_diff),
    "abs" => MethodHandler::Legacy(crate::executor::objects::typed_array_methods::handle_float_abs),
    "sqrt" => MethodHandler::Legacy(crate::executor::objects::typed_array_methods::handle_float_sqrt),
    "ln" => MethodHandler::Legacy(crate::executor::objects::typed_array_methods::handle_float_ln),
    "exp" => MethodHandler::Legacy(crate::executor::objects::typed_array_methods::handle_float_exp),
    "map" => MethodHandler::Legacy(crate::executor::objects::typed_array_methods::handle_float_map),
    "filter" => MethodHandler::Legacy(crate::executor::objects::typed_array_methods::handle_float_filter),
    "forEach" => MethodHandler::Legacy(crate::executor::objects::typed_array_methods::handle_float_for_each),
    "toArray" => MethodHandler::Legacy(crate::executor::objects::typed_array_methods::handle_float_to_array),
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
    "abs" => MethodHandler::Legacy(crate::executor::objects::typed_array_methods::handle_int_abs),
    "map" => MethodHandler::Legacy(crate::executor::objects::typed_array_methods::handle_int_map),
    "filter" => MethodHandler::Legacy(crate::executor::objects::typed_array_methods::handle_int_filter),
    "forEach" => MethodHandler::Legacy(crate::executor::objects::typed_array_methods::handle_int_for_each),
    "toArray" => MethodHandler::Legacy(crate::executor::objects::typed_array_methods::handle_int_to_array),
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
    "toArray" => MethodHandler::Legacy(crate::executor::objects::typed_array_methods::handle_bool_to_array),
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
};
