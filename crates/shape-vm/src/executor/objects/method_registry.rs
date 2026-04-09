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
    // Higher-order functions
    "map" => MethodHandler::Legacy(crate::executor::objects::array_transform::handle_map),
    "filter" => MethodHandler::Legacy(crate::executor::objects::array_transform::handle_filter),
    "reduce" => MethodHandler::Legacy(crate::executor::objects::array_aggregation::handle_reduce),
    "fold" => MethodHandler::Legacy(crate::executor::objects::array_aggregation::handle_reduce),
    "forEach" => MethodHandler::Legacy(crate::executor::objects::array_query::handle_for_each),
    "find" => MethodHandler::Legacy(crate::executor::objects::array_query::handle_find),
    "findIndex" => MethodHandler::Legacy(crate::executor::objects::array_query::handle_find_index),
    "some" => MethodHandler::Legacy(crate::executor::objects::array_query::handle_some),
    "every" => MethodHandler::Legacy(crate::executor::objects::array_query::handle_every),
    "sort" => MethodHandler::Legacy(crate::executor::objects::array_transform::handle_sort),
    "groupBy" => MethodHandler::Legacy(crate::executor::objects::array_transform::handle_group_by),
    "flatMap" => MethodHandler::Legacy(crate::executor::objects::array_transform::handle_flat_map),

    // Basic operations
    "len" => MethodHandler::Legacy(crate::executor::objects::array_basic::handle_len),
    "length" => MethodHandler::Legacy(crate::executor::objects::array_basic::handle_length),
    "first" => MethodHandler::Legacy(crate::executor::objects::array_basic::handle_first),
    "last" => MethodHandler::Legacy(crate::executor::objects::array_basic::handle_last),
    "reverse" => MethodHandler::Legacy(crate::executor::objects::array_basic::handle_reverse),
    "push" => MethodHandler::Legacy(crate::executor::objects::array_basic::handle_push),
    "pop" => MethodHandler::Legacy(crate::executor::objects::array_basic::handle_pop),
    "zip" => MethodHandler::Legacy(crate::executor::objects::array_basic::handle_zip),
    "slice" => MethodHandler::Legacy(crate::executor::objects::array_transform::handle_slice),
    "concat" => MethodHandler::Legacy(crate::executor::objects::array_transform::handle_concat),
    "take" => MethodHandler::Legacy(crate::executor::objects::array_transform::handle_take),
    "drop" => MethodHandler::Legacy(crate::executor::objects::array_transform::handle_drop),
    "skip" => MethodHandler::Legacy(crate::executor::objects::array_transform::handle_skip),

    // Search methods
    "indexOf" => MethodHandler::Legacy(crate::executor::objects::array_query::handle_index_of),
    "includes" => MethodHandler::Legacy(crate::executor::objects::array_query::handle_includes),

    // Transform methods
    "join" => MethodHandler::Legacy(crate::executor::objects::array_sort::handle_join_str),
    "flatten" => MethodHandler::Legacy(crate::executor::objects::array_transform::handle_flatten),
    "unique" => MethodHandler::Legacy(crate::executor::objects::array_sets::handle_unique),
    "distinct" => MethodHandler::Legacy(crate::executor::objects::array_sets::handle_distinct),
    "distinctBy" => MethodHandler::Legacy(crate::executor::objects::array_sets::handle_distinct_by),

    // Aggregation methods
    "sum" => MethodHandler::Legacy(crate::executor::objects::array_aggregation::handle_sum),
    "avg" => MethodHandler::Legacy(crate::executor::objects::array_aggregation::handle_avg),
    "min" => MethodHandler::Legacy(crate::executor::objects::array_aggregation::handle_min),
    "max" => MethodHandler::Legacy(crate::executor::objects::array_aggregation::handle_max),
    "count" => MethodHandler::Legacy(crate::executor::objects::array_aggregation::handle_count),

    // SQL-like query methods (aliases and additional)
    "where" => MethodHandler::Legacy(crate::executor::objects::array_query::handle_where),
    "select" => MethodHandler::Legacy(crate::executor::objects::array_query::handle_select),
    "orderBy" => MethodHandler::Legacy(crate::executor::objects::array_sort::handle_order_by),
    "thenBy" => MethodHandler::Legacy(crate::executor::objects::array_sort::handle_then_by),
    "takeWhile" => MethodHandler::Legacy(crate::executor::objects::array_query::handle_take_while),
    "skipWhile" => MethodHandler::Legacy(crate::executor::objects::array_query::handle_skip_while),
    "single" => MethodHandler::Legacy(crate::executor::objects::array_query::handle_single),
    "any" => MethodHandler::Legacy(crate::executor::objects::array_query::handle_any),
    "all" => MethodHandler::Legacy(crate::executor::objects::array_query::handle_all),

    // Join operations
    "innerJoin" => MethodHandler::Legacy(crate::executor::objects::array_joins::handle_inner_join),
    "leftJoin" => MethodHandler::Legacy(crate::executor::objects::array_joins::handle_left_join),
    "crossJoin" => MethodHandler::Legacy(crate::executor::objects::array_joins::handle_cross_join),

    // Set operations
    "union" => MethodHandler::Legacy(crate::executor::objects::array_sets::handle_union),
    "intersect" => MethodHandler::Legacy(crate::executor::objects::array_sets::handle_intersect),
    "except" => MethodHandler::Legacy(crate::executor::objects::array_sets::handle_except),

    // Clone
    "clone" => MethodHandler::Legacy(crate::executor::objects::array_basic::handle_clone),

    // Iterator
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
    "len" => MethodHandler::Legacy(crate::executor::objects::column_methods::handle_len),
    "length" => MethodHandler::Legacy(crate::executor::objects::column_methods::handle_len),
    "sum" => MethodHandler::Legacy(crate::executor::objects::column_methods::handle_sum),
    "mean" => MethodHandler::Legacy(crate::executor::objects::column_methods::handle_mean),
    "min" => MethodHandler::Legacy(crate::executor::objects::column_methods::handle_min),
    "max" => MethodHandler::Legacy(crate::executor::objects::column_methods::handle_max),
    "std" => MethodHandler::Legacy(crate::executor::objects::column_methods::handle_std),
    "first" => MethodHandler::Legacy(crate::executor::objects::column_methods::handle_first),
    "last" => MethodHandler::Legacy(crate::executor::objects::column_methods::handle_last),
    "toArray" => MethodHandler::Legacy(crate::executor::objects::column_methods::handle_to_array),
    "abs" => MethodHandler::Legacy(crate::executor::objects::column_methods::handle_abs),
};

/// PHF registry for HashMap methods (18 methods)
///
/// **Core:** get, set, has, delete, keys, values, entries, len, length, isEmpty
/// **Higher-order:** map, filter, forEach, reduce, groupBy
/// **Convenience:** merge, getOrDefault, toArray
pub static HASHMAP_METHODS: phf::Map<&'static str, MethodHandler> = phf_map! {
    "get" => MethodHandler::Legacy(crate::executor::objects::hashmap_methods::handle_get),
    "set" => MethodHandler::Legacy(crate::executor::objects::hashmap_methods::handle_set),
    "has" => MethodHandler::Legacy(crate::executor::objects::hashmap_methods::handle_has),
    "delete" => MethodHandler::Legacy(crate::executor::objects::hashmap_methods::handle_delete),
    "keys" => MethodHandler::Legacy(crate::executor::objects::hashmap_methods::handle_keys),
    "values" => MethodHandler::Legacy(crate::executor::objects::hashmap_methods::handle_values),
    "entries" => MethodHandler::Legacy(crate::executor::objects::hashmap_methods::handle_entries),
    "len" => MethodHandler::Legacy(crate::executor::objects::hashmap_methods::handle_len),
    "length" => MethodHandler::Legacy(crate::executor::objects::hashmap_methods::handle_len),
    "isEmpty" => MethodHandler::Legacy(crate::executor::objects::hashmap_methods::handle_is_empty),
    "map" => MethodHandler::Legacy(crate::executor::objects::hashmap_methods::handle_map),
    "filter" => MethodHandler::Legacy(crate::executor::objects::hashmap_methods::handle_filter),
    "forEach" => MethodHandler::Legacy(crate::executor::objects::hashmap_methods::handle_for_each),
    "merge" => MethodHandler::Legacy(crate::executor::objects::hashmap_methods::handle_merge),
    "getOrDefault" => MethodHandler::Legacy(crate::executor::objects::hashmap_methods::handle_get_or_default),
    "reduce" => MethodHandler::Legacy(crate::executor::objects::hashmap_methods::handle_reduce),
    "toArray" => MethodHandler::Legacy(crate::executor::objects::hashmap_methods::handle_to_array),
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
    "add" => MethodHandler::Legacy(crate::executor::objects::set_methods::handle_add),
    "has" => MethodHandler::Legacy(crate::executor::objects::set_methods::handle_has),
    "delete" => MethodHandler::Legacy(crate::executor::objects::set_methods::handle_delete),
    "size" => MethodHandler::Legacy(crate::executor::objects::set_methods::handle_size),
    "len" => MethodHandler::Legacy(crate::executor::objects::set_methods::handle_size),
    "length" => MethodHandler::Legacy(crate::executor::objects::set_methods::handle_size),
    "isEmpty" => MethodHandler::Legacy(crate::executor::objects::set_methods::handle_is_empty),
    "toArray" => MethodHandler::Legacy(crate::executor::objects::set_methods::handle_to_array),
    "forEach" => MethodHandler::Legacy(crate::executor::objects::set_methods::handle_for_each),
    "map" => MethodHandler::Legacy(crate::executor::objects::set_methods::handle_map),
    "filter" => MethodHandler::Legacy(crate::executor::objects::set_methods::handle_filter),
    "union" => MethodHandler::Legacy(crate::executor::objects::set_methods::handle_union),
    "intersection" => MethodHandler::Legacy(crate::executor::objects::set_methods::handle_intersection),
    "difference" => MethodHandler::Legacy(crate::executor::objects::set_methods::handle_difference),
};

/// PHF registry for Deque methods
///
/// **Mutation:** pushBack, pushFront, popBack, popFront
/// **Access:** peekBack, peekFront, get
/// **Info:** size, len, length, isEmpty
/// **Conversion:** toArray
pub static DEQUE_METHODS: phf::Map<&'static str, MethodHandler> = phf_map! {
    "pushBack" => MethodHandler::Legacy(crate::executor::objects::deque_methods::handle_push_back),
    "pushFront" => MethodHandler::Legacy(crate::executor::objects::deque_methods::handle_push_front),
    "popBack" => MethodHandler::Legacy(crate::executor::objects::deque_methods::handle_pop_back),
    "popFront" => MethodHandler::Legacy(crate::executor::objects::deque_methods::handle_pop_front),
    "peekBack" => MethodHandler::Legacy(crate::executor::objects::deque_methods::handle_peek_back),
    "peekFront" => MethodHandler::Legacy(crate::executor::objects::deque_methods::handle_peek_front),
    "size" => MethodHandler::Legacy(crate::executor::objects::deque_methods::handle_size),
    "len" => MethodHandler::Legacy(crate::executor::objects::deque_methods::handle_size),
    "length" => MethodHandler::Legacy(crate::executor::objects::deque_methods::handle_size),
    "isEmpty" => MethodHandler::Legacy(crate::executor::objects::deque_methods::handle_is_empty),
    "toArray" => MethodHandler::Legacy(crate::executor::objects::deque_methods::handle_to_array),
    "get" => MethodHandler::Legacy(crate::executor::objects::deque_methods::handle_get),
};

/// PHF registry for PriorityQueue methods
///
/// **Mutation:** push, pop
/// **Access:** peek
/// **Info:** size, len, length, isEmpty
/// **Conversion:** toArray, toSortedArray
pub static PRIORITY_QUEUE_METHODS: phf::Map<&'static str, MethodHandler> = phf_map! {
    "push" => MethodHandler::Legacy(crate::executor::objects::priority_queue_methods::handle_push),
    "pop" => MethodHandler::Legacy(crate::executor::objects::priority_queue_methods::handle_pop),
    "peek" => MethodHandler::Legacy(crate::executor::objects::priority_queue_methods::handle_peek),
    "size" => MethodHandler::Legacy(crate::executor::objects::priority_queue_methods::handle_size),
    "len" => MethodHandler::Legacy(crate::executor::objects::priority_queue_methods::handle_size),
    "length" => MethodHandler::Legacy(crate::executor::objects::priority_queue_methods::handle_size),
    "isEmpty" => MethodHandler::Legacy(crate::executor::objects::priority_queue_methods::handle_is_empty),
    "toArray" => MethodHandler::Legacy(crate::executor::objects::priority_queue_methods::handle_to_array),
    "toSortedArray" => MethodHandler::Legacy(crate::executor::objects::priority_queue_methods::handle_to_sorted_array),
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
    // Linear algebra
    "transpose" => MethodHandler::Legacy(crate::executor::objects::matrix_methods::handle_transpose),
    "inverse" => MethodHandler::Legacy(crate::executor::objects::matrix_methods::handle_inverse),
    "det" => MethodHandler::Legacy(crate::executor::objects::matrix_methods::handle_determinant),
    "determinant" => MethodHandler::Legacy(crate::executor::objects::matrix_methods::handle_determinant),
    "trace" => MethodHandler::Legacy(crate::executor::objects::matrix_methods::handle_trace),

    // Shape and access
    "shape" => MethodHandler::Legacy(crate::executor::objects::matrix_methods::handle_shape),
    "reshape" => MethodHandler::Legacy(crate::executor::objects::matrix_methods::handle_reshape),
    "row" => MethodHandler::Legacy(crate::executor::objects::matrix_methods::handle_row),
    "col" => MethodHandler::Legacy(crate::executor::objects::matrix_methods::handle_col),
    "diag" => MethodHandler::Legacy(crate::executor::objects::matrix_methods::handle_diag),
    "flatten" => MethodHandler::Legacy(crate::executor::objects::matrix_methods::handle_flatten),

    // Higher-order
    "map" => MethodHandler::Legacy(crate::executor::objects::matrix_methods::handle_map),

    // Aggregation
    "sum" => MethodHandler::Legacy(crate::executor::objects::matrix_methods::handle_sum),
    "min" => MethodHandler::Legacy(crate::executor::objects::matrix_methods::handle_min),
    "max" => MethodHandler::Legacy(crate::executor::objects::matrix_methods::handle_max),
    "mean" => MethodHandler::Legacy(crate::executor::objects::matrix_methods::handle_mean),
    "rowSum" => MethodHandler::Legacy(crate::executor::objects::matrix_methods::handle_row_sum),
    "colSum" => MethodHandler::Legacy(crate::executor::objects::matrix_methods::handle_col_sum),
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
    "sum" => MethodHandler::Legacy(crate::executor::objects::typed_array_methods::handle_float_sum),
    "avg" => MethodHandler::Legacy(crate::executor::objects::typed_array_methods::handle_float_avg),
    "mean" => MethodHandler::Legacy(crate::executor::objects::typed_array_methods::handle_float_avg),
    "min" => MethodHandler::Legacy(crate::executor::objects::typed_array_methods::handle_float_min),
    "max" => MethodHandler::Legacy(crate::executor::objects::typed_array_methods::handle_float_max),
    "std" => MethodHandler::Legacy(crate::executor::objects::typed_array_methods::handle_float_std),
    "variance" => MethodHandler::Legacy(crate::executor::objects::typed_array_methods::handle_float_variance),
    "dot" => MethodHandler::Legacy(crate::executor::objects::typed_array_methods::handle_float_dot),
    "norm" => MethodHandler::Legacy(crate::executor::objects::typed_array_methods::handle_float_norm),
    "normalize" => MethodHandler::Legacy(crate::executor::objects::typed_array_methods::handle_float_normalize),
    "cumsum" => MethodHandler::Legacy(crate::executor::objects::typed_array_methods::handle_float_cumsum),
    "diff" => MethodHandler::Legacy(crate::executor::objects::typed_array_methods::handle_float_diff),
    "abs" => MethodHandler::Legacy(crate::executor::objects::typed_array_methods::handle_float_abs),
    "sqrt" => MethodHandler::Legacy(crate::executor::objects::typed_array_methods::handle_float_sqrt),
    "ln" => MethodHandler::Legacy(crate::executor::objects::typed_array_methods::handle_float_ln),
    "exp" => MethodHandler::Legacy(crate::executor::objects::typed_array_methods::handle_float_exp),
    "len" => MethodHandler::Legacy(crate::executor::objects::typed_array_methods::handle_float_len),
    "length" => MethodHandler::Legacy(crate::executor::objects::typed_array_methods::handle_float_len),
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
    "sum" => MethodHandler::Legacy(crate::executor::objects::typed_array_methods::handle_int_sum),
    "avg" => MethodHandler::Legacy(crate::executor::objects::typed_array_methods::handle_int_avg),
    "mean" => MethodHandler::Legacy(crate::executor::objects::typed_array_methods::handle_int_avg),
    "min" => MethodHandler::Legacy(crate::executor::objects::typed_array_methods::handle_int_min),
    "max" => MethodHandler::Legacy(crate::executor::objects::typed_array_methods::handle_int_max),
    "abs" => MethodHandler::Legacy(crate::executor::objects::typed_array_methods::handle_int_abs),
    "len" => MethodHandler::Legacy(crate::executor::objects::typed_array_methods::handle_int_len),
    "length" => MethodHandler::Legacy(crate::executor::objects::typed_array_methods::handle_int_len),
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
    "len" => MethodHandler::Legacy(crate::executor::objects::typed_array_methods::handle_bool_len),
    "length" => MethodHandler::Legacy(crate::executor::objects::typed_array_methods::handle_bool_len),
    "toArray" => MethodHandler::Legacy(crate::executor::objects::typed_array_methods::handle_bool_to_array),
    "count" => MethodHandler::Legacy(crate::executor::objects::typed_array_methods::handle_bool_count_true),
    "any" => MethodHandler::Legacy(crate::executor::objects::typed_array_methods::handle_bool_any),
    "all" => MethodHandler::Legacy(crate::executor::objects::typed_array_methods::handle_bool_all),
};

// ═══════════════════════════════════════════════════════════════════════════
// Concurrency primitives — compiler-builtin interior mutability types
// ═══════════════════════════════════════════════════════════════════════════

/// Mutex<T> methods: lock, try_lock, set
pub static MUTEX_METHODS: phf::Map<&'static str, MethodHandler> = phf_map! {
    "lock" => MethodHandler::Legacy(crate::executor::objects::concurrency_methods::handle_mutex_lock),
    "try_lock" => MethodHandler::Legacy(crate::executor::objects::concurrency_methods::handle_mutex_try_lock),
    "set" => MethodHandler::Legacy(crate::executor::objects::concurrency_methods::handle_mutex_set),
};

/// Atomic<T> methods: load, store, fetch_add, fetch_sub, compare_exchange
pub static ATOMIC_METHODS: phf::Map<&'static str, MethodHandler> = phf_map! {
    "load" => MethodHandler::Legacy(crate::executor::objects::concurrency_methods::handle_atomic_load),
    "store" => MethodHandler::Legacy(crate::executor::objects::concurrency_methods::handle_atomic_store),
    "fetch_add" => MethodHandler::Legacy(crate::executor::objects::concurrency_methods::handle_atomic_fetch_add),
    "fetch_sub" => MethodHandler::Legacy(crate::executor::objects::concurrency_methods::handle_atomic_fetch_sub),
    "compare_exchange" => MethodHandler::Legacy(crate::executor::objects::concurrency_methods::handle_atomic_compare_exchange),
};

/// Lazy<T> methods: get, is_initialized
pub static LAZY_METHODS: phf::Map<&'static str, MethodHandler> = phf_map! {
    "get" => MethodHandler::Legacy(crate::executor::objects::concurrency_methods::handle_lazy_get),
    "is_initialized" => MethodHandler::Legacy(crate::executor::objects::concurrency_methods::handle_lazy_is_initialized),
};

/// Channel methods: send, recv, try_recv, close, is_closed, is_sender
pub static CHANNEL_METHODS: phf::Map<&'static str, MethodHandler> = phf_map! {
    "send" => MethodHandler::Legacy(crate::executor::objects::channel_methods::handle_channel_send),
    "recv" => MethodHandler::Legacy(crate::executor::objects::channel_methods::handle_channel_recv),
    "try_recv" => MethodHandler::Legacy(crate::executor::objects::channel_methods::handle_channel_try_recv),
    "close" => MethodHandler::Legacy(crate::executor::objects::channel_methods::handle_channel_close),
    "is_closed" => MethodHandler::Legacy(crate::executor::objects::channel_methods::handle_channel_is_closed),
    "is_sender" => MethodHandler::Legacy(crate::executor::objects::channel_methods::handle_channel_is_sender),
};
