//! Typed method identifiers for zero-cost dispatch.
//!
//! `MethodId(u16)` replaces string-based method lookup at runtime.
//! The compiler resolves method names to `MethodId` at compile time,
//! and the VM dispatches on a `u16` match instead of a PHF string lookup.
//!
//! Methods not known at compile time use `MethodId::DYNAMIC`, which
//! triggers a fallback to string-based lookup.

use serde::{Deserialize, Serialize};
use crate::value_word::ValueWordExt;

/// A typed method identifier. Wraps a `u16` discriminant that the compiler
/// resolves from method name strings at compile time.
///
/// Known methods get a fixed ID (0..N). Unknown/dynamic methods use
/// `MethodId::DYNAMIC` (0xFFFF), which causes the VM to fall back to
/// string-based PHF lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(transparent)]
pub struct MethodId(pub u16);

impl MethodId {
    /// Sentinel value for methods not known at compile time.
    /// The VM falls back to string-based dispatch for this ID.
    pub const DYNAMIC: MethodId = MethodId(0xFFFF);

    /// Returns true if this is a dynamically-dispatched method (not resolved at compile time).
    #[inline]
    pub const fn is_dynamic(self) -> bool {
        self.0 == 0xFFFF
    }

    /// Resolve a method name to a MethodId at compile time.
    /// Returns `MethodId::DYNAMIC` for unknown method names.
    pub fn from_name(name: &str) -> MethodId {
        match name {
            // === Universal intrinsics (0-4) ===
            "type" => Self::TYPE,
            "to_string" | "toString" => Self::TO_STRING,

            // === Array methods — higher-order (10-20) ===
            "map" => Self::MAP,
            "filter" => Self::FILTER,
            "reduce" => Self::REDUCE,
            "forEach" => Self::FOR_EACH,
            "find" => Self::FIND,
            "findIndex" => Self::FIND_INDEX,
            "some" => Self::SOME,
            "every" => Self::EVERY,
            "sort" => Self::SORT,
            "groupBy" | "group_by" => Self::GROUP_BY,
            "flatMap" => Self::FLAT_MAP,

            // === Array methods — basic (30-39) ===
            "len" => Self::LEN,
            "length" => Self::LENGTH,
            "first" => Self::FIRST,
            "last" => Self::LAST,
            "reverse" => Self::REVERSE,
            "slice" => Self::SLICE,
            "concat" => Self::CONCAT,
            "take" => Self::TAKE,
            "drop" => Self::DROP,
            "skip" => Self::SKIP,

            // === Array methods — search (40-41) ===
            "indexOf" => Self::INDEX_OF,
            "includes" => Self::INCLUDES,

            // === Array methods — transform (50-56) ===
            "join" => Self::JOIN,
            "flatten" => Self::FLATTEN,
            "unique" => Self::UNIQUE,
            "distinct" => Self::DISTINCT,
            "distinctBy" => Self::DISTINCT_BY,

            // === Array methods — aggregation (60-64) ===
            "sum" => Self::SUM,
            "avg" => Self::AVG,
            "min" => Self::MIN,
            "max" => Self::MAX,
            "count" => Self::COUNT,

            // === Array methods — SQL-like query (70-79) ===
            "where" => Self::WHERE,
            "select" => Self::SELECT,
            "orderBy" => Self::ORDER_BY,
            "thenBy" => Self::THEN_BY,
            "takeWhile" => Self::TAKE_WHILE,
            "skipWhile" => Self::SKIP_WHILE,
            "single" => Self::SINGLE,
            "any" => Self::ANY,
            "all" => Self::ALL,

            // === Array methods — joins (80-82) ===
            "innerJoin" => Self::INNER_JOIN,
            "leftJoin" => Self::LEFT_JOIN,
            "crossJoin" => Self::CROSS_JOIN,

            // === Array methods — sets (85-87) ===
            "union" => Self::UNION,
            "intersect" => Self::INTERSECT,
            "except" => Self::EXCEPT,

            // === DataTable-specific methods (100-129) ===
            "origin" => Self::ORIGIN,
            "columns" => Self::COLUMNS,
            "column" => Self::COLUMN,
            "head" => Self::HEAD,
            "tail" => Self::TAIL,
            "mean" => Self::MEAN,
            "describe" => Self::DESCRIBE,
            "aggregate" => Self::AGGREGATE,
            "index_by" | "indexBy" => Self::INDEX_BY,
            "limit" => Self::LIMIT,
            "execute" => Self::EXECUTE,
            "simulate" => Self::SIMULATE,
            "correlation" => Self::CORRELATION,
            "covariance" => Self::COVARIANCE,
            "rolling_sum" | "rollingSum" => Self::ROLLING_SUM,
            "rolling_mean" | "rollingMean" => Self::ROLLING_MEAN,
            "rolling_std" | "rollingStd" => Self::ROLLING_STD,
            "diff" => Self::DIFF,
            "pct_change" | "pctChange" => Self::PCT_CHANGE,
            "forward_fill" | "forwardFill" => Self::FORWARD_FILL,

            // === Column methods (130-133) ===
            "std" => Self::STD,
            "toArray" => Self::TO_ARRAY,
            "abs" => Self::ABS,

            // === IndexedTable methods (140-141) ===
            "between" => Self::BETWEEN,
            "resample" => Self::RESAMPLE,

            // === Number methods (150-159) ===
            "toFixed" | "to_fixed" => Self::TO_FIXED,
            "toInt" | "to_int" => Self::TO_INT,
            "toNumber" | "to_number" => Self::TO_NUMBER,
            "floor" => Self::FLOOR,
            "ceil" => Self::CEIL,
            "round" => Self::ROUND,

            // === String methods (160-174) ===
            "toUpperCase" | "to_upper_case" => Self::TO_UPPER_CASE,
            "toLowerCase" | "to_lower_case" => Self::TO_LOWER_CASE,
            "trim" => Self::TRIM,
            "contains" => Self::CONTAINS,
            "startsWith" => Self::STARTS_WITH,
            "endsWith" => Self::ENDS_WITH,
            "split" => Self::SPLIT,
            "replace" => Self::REPLACE,
            "substring" => Self::SUBSTRING,

            // === Option/Result methods (180-189) ===
            "unwrap" => Self::UNWRAP,
            "unwrapOr" => Self::UNWRAP_OR,
            "isSome" => Self::IS_SOME,
            "isNone" => Self::IS_NONE,
            "isOk" => Self::IS_OK,
            "isErr" => Self::IS_ERR,
            "mapErr" => Self::MAP_ERR,

            // === Array mutation methods (190-192) ===
            "push" => Self::PUSH,
            "pop" => Self::POP,
            "isEmpty" => Self::IS_EMPTY,

            _ => Self::DYNAMIC,
        }
    }

    /// Get the canonical method name for a known MethodId.
    /// Returns `None` for `DYNAMIC` IDs.
    pub fn name(self) -> Option<&'static str> {
        match self {
            Self::TYPE => Some("type"),
            Self::TO_STRING => Some("toString"),
            Self::MAP => Some("map"),
            Self::FILTER => Some("filter"),
            Self::REDUCE => Some("reduce"),
            Self::FOR_EACH => Some("forEach"),
            Self::FIND => Some("find"),
            Self::FIND_INDEX => Some("findIndex"),
            Self::SOME => Some("some"),
            Self::EVERY => Some("every"),
            Self::SORT => Some("sort"),
            Self::GROUP_BY => Some("groupBy"),
            Self::FLAT_MAP => Some("flatMap"),
            Self::LEN => Some("len"),
            Self::LENGTH => Some("length"),
            Self::FIRST => Some("first"),
            Self::LAST => Some("last"),
            Self::REVERSE => Some("reverse"),
            Self::SLICE => Some("slice"),
            Self::CONCAT => Some("concat"),
            Self::TAKE => Some("take"),
            Self::DROP => Some("drop"),
            Self::SKIP => Some("skip"),
            Self::INDEX_OF => Some("indexOf"),
            Self::INCLUDES => Some("includes"),
            Self::JOIN => Some("join"),
            Self::FLATTEN => Some("flatten"),
            Self::UNIQUE => Some("unique"),
            Self::DISTINCT => Some("distinct"),
            Self::DISTINCT_BY => Some("distinctBy"),
            Self::SUM => Some("sum"),
            Self::AVG => Some("avg"),
            Self::MIN => Some("min"),
            Self::MAX => Some("max"),
            Self::COUNT => Some("count"),
            Self::WHERE => Some("where"),
            Self::SELECT => Some("select"),
            Self::ORDER_BY => Some("orderBy"),
            Self::THEN_BY => Some("thenBy"),
            Self::TAKE_WHILE => Some("takeWhile"),
            Self::SKIP_WHILE => Some("skipWhile"),
            Self::SINGLE => Some("single"),
            Self::ANY => Some("any"),
            Self::ALL => Some("all"),
            Self::INNER_JOIN => Some("innerJoin"),
            Self::LEFT_JOIN => Some("leftJoin"),
            Self::CROSS_JOIN => Some("crossJoin"),
            Self::UNION => Some("union"),
            Self::INTERSECT => Some("intersect"),
            Self::EXCEPT => Some("except"),
            Self::ORIGIN => Some("origin"),
            Self::COLUMNS => Some("columns"),
            Self::COLUMN => Some("column"),
            Self::HEAD => Some("head"),
            Self::TAIL => Some("tail"),
            Self::MEAN => Some("mean"),
            Self::DESCRIBE => Some("describe"),
            Self::AGGREGATE => Some("aggregate"),
            Self::INDEX_BY => Some("indexBy"),
            Self::LIMIT => Some("limit"),
            Self::EXECUTE => Some("execute"),
            Self::SIMULATE => Some("simulate"),
            Self::CORRELATION => Some("correlation"),
            Self::COVARIANCE => Some("covariance"),
            Self::ROLLING_SUM => Some("rollingSum"),
            Self::ROLLING_MEAN => Some("rollingMean"),
            Self::ROLLING_STD => Some("rollingStd"),
            Self::DIFF => Some("diff"),
            Self::PCT_CHANGE => Some("pctChange"),
            Self::FORWARD_FILL => Some("forwardFill"),
            Self::STD => Some("std"),
            Self::TO_ARRAY => Some("toArray"),
            Self::ABS => Some("abs"),
            Self::BETWEEN => Some("between"),
            Self::RESAMPLE => Some("resample"),
            Self::TO_FIXED => Some("toFixed"),
            Self::TO_INT => Some("toInt"),
            Self::TO_NUMBER => Some("toNumber"),
            Self::FLOOR => Some("floor"),
            Self::CEIL => Some("ceil"),
            Self::ROUND => Some("round"),
            Self::TO_UPPER_CASE => Some("toUpperCase"),
            Self::TO_LOWER_CASE => Some("toLowerCase"),
            Self::TRIM => Some("trim"),
            Self::CONTAINS => Some("contains"),
            Self::STARTS_WITH => Some("startsWith"),
            Self::ENDS_WITH => Some("endsWith"),
            Self::SPLIT => Some("split"),
            Self::REPLACE => Some("replace"),
            Self::SUBSTRING => Some("substring"),
            Self::UNWRAP => Some("unwrap"),
            Self::UNWRAP_OR => Some("unwrapOr"),
            Self::IS_SOME => Some("isSome"),
            Self::IS_NONE => Some("isNone"),
            Self::IS_OK => Some("isOk"),
            Self::IS_ERR => Some("isErr"),
            Self::MAP_ERR => Some("mapErr"),
            Self::PUSH => Some("push"),
            Self::POP => Some("pop"),
            Self::IS_EMPTY => Some("isEmpty"),
            _ => None,
        }
    }

    // === Universal intrinsics ===
    pub const TYPE: MethodId = MethodId(0);
    pub const TO_STRING: MethodId = MethodId(1);

    // === Array methods — higher-order ===
    pub const MAP: MethodId = MethodId(10);
    pub const FILTER: MethodId = MethodId(11);
    pub const REDUCE: MethodId = MethodId(12);
    pub const FOR_EACH: MethodId = MethodId(13);
    pub const FIND: MethodId = MethodId(14);
    pub const FIND_INDEX: MethodId = MethodId(15);
    pub const SOME: MethodId = MethodId(16);
    pub const EVERY: MethodId = MethodId(17);
    pub const SORT: MethodId = MethodId(18);
    pub const GROUP_BY: MethodId = MethodId(19);
    pub const FLAT_MAP: MethodId = MethodId(20);

    // === Array methods — basic ===
    pub const LEN: MethodId = MethodId(30);
    pub const LENGTH: MethodId = MethodId(31);
    pub const FIRST: MethodId = MethodId(32);
    pub const LAST: MethodId = MethodId(33);
    pub const REVERSE: MethodId = MethodId(34);
    pub const SLICE: MethodId = MethodId(35);
    pub const CONCAT: MethodId = MethodId(36);
    pub const TAKE: MethodId = MethodId(37);
    pub const DROP: MethodId = MethodId(38);
    pub const SKIP: MethodId = MethodId(39);

    // === Array methods — search ===
    pub const INDEX_OF: MethodId = MethodId(40);
    pub const INCLUDES: MethodId = MethodId(41);

    // === Array methods — transform ===
    pub const JOIN: MethodId = MethodId(50);
    pub const FLATTEN: MethodId = MethodId(51);
    pub const UNIQUE: MethodId = MethodId(52);
    pub const DISTINCT: MethodId = MethodId(53);
    pub const DISTINCT_BY: MethodId = MethodId(54);

    // === Array methods — aggregation ===
    pub const SUM: MethodId = MethodId(60);
    pub const AVG: MethodId = MethodId(61);
    pub const MIN: MethodId = MethodId(62);
    pub const MAX: MethodId = MethodId(63);
    pub const COUNT: MethodId = MethodId(64);

    // === Array methods — SQL-like query ===
    pub const WHERE: MethodId = MethodId(70);
    pub const SELECT: MethodId = MethodId(71);
    pub const ORDER_BY: MethodId = MethodId(72);
    pub const THEN_BY: MethodId = MethodId(73);
    pub const TAKE_WHILE: MethodId = MethodId(74);
    pub const SKIP_WHILE: MethodId = MethodId(75);
    pub const SINGLE: MethodId = MethodId(76);
    pub const ANY: MethodId = MethodId(77);
    pub const ALL: MethodId = MethodId(78);

    // === Array methods — joins ===
    pub const INNER_JOIN: MethodId = MethodId(80);
    pub const LEFT_JOIN: MethodId = MethodId(81);
    pub const CROSS_JOIN: MethodId = MethodId(82);

    // === Array methods — sets ===
    pub const UNION: MethodId = MethodId(85);
    pub const INTERSECT: MethodId = MethodId(86);
    pub const EXCEPT: MethodId = MethodId(87);

    // === DataTable-specific methods ===
    pub const ORIGIN: MethodId = MethodId(100);
    pub const COLUMNS: MethodId = MethodId(101);
    pub const COLUMN: MethodId = MethodId(102);
    pub const HEAD: MethodId = MethodId(103);
    pub const TAIL: MethodId = MethodId(104);
    pub const MEAN: MethodId = MethodId(105);
    pub const DESCRIBE: MethodId = MethodId(106);
    pub const AGGREGATE: MethodId = MethodId(107);
    pub const INDEX_BY: MethodId = MethodId(108);
    pub const LIMIT: MethodId = MethodId(109);
    pub const EXECUTE: MethodId = MethodId(110);
    pub const SIMULATE: MethodId = MethodId(111);
    pub const CORRELATION: MethodId = MethodId(112);
    pub const COVARIANCE: MethodId = MethodId(113);
    pub const ROLLING_SUM: MethodId = MethodId(114);
    pub const ROLLING_MEAN: MethodId = MethodId(115);
    pub const ROLLING_STD: MethodId = MethodId(116);
    pub const DIFF: MethodId = MethodId(117);
    pub const PCT_CHANGE: MethodId = MethodId(118);
    pub const FORWARD_FILL: MethodId = MethodId(119);

    // === Column methods ===
    pub const STD: MethodId = MethodId(130);
    pub const TO_ARRAY: MethodId = MethodId(131);
    pub const ABS: MethodId = MethodId(132);

    // === IndexedTable methods ===
    pub const BETWEEN: MethodId = MethodId(140);
    pub const RESAMPLE: MethodId = MethodId(141);

    // === Number methods ===
    pub const TO_FIXED: MethodId = MethodId(150);
    pub const TO_INT: MethodId = MethodId(151);
    pub const TO_NUMBER: MethodId = MethodId(152);
    pub const FLOOR: MethodId = MethodId(153);
    pub const CEIL: MethodId = MethodId(154);
    pub const ROUND: MethodId = MethodId(155);

    // === String methods ===
    pub const TO_UPPER_CASE: MethodId = MethodId(160);
    pub const TO_LOWER_CASE: MethodId = MethodId(161);
    pub const TRIM: MethodId = MethodId(162);
    pub const CONTAINS: MethodId = MethodId(163);
    pub const STARTS_WITH: MethodId = MethodId(164);
    pub const ENDS_WITH: MethodId = MethodId(165);
    pub const SPLIT: MethodId = MethodId(166);
    pub const REPLACE: MethodId = MethodId(167);
    pub const SUBSTRING: MethodId = MethodId(168);

    // === Option/Result methods ===
    pub const UNWRAP: MethodId = MethodId(180);
    pub const UNWRAP_OR: MethodId = MethodId(181);
    pub const IS_SOME: MethodId = MethodId(182);
    pub const IS_NONE: MethodId = MethodId(183);
    pub const IS_OK: MethodId = MethodId(184);
    pub const IS_ERR: MethodId = MethodId(185);
    pub const MAP_ERR: MethodId = MethodId(186);

    // === Array mutation methods ===
    pub const PUSH: MethodId = MethodId(190);
    pub const POP: MethodId = MethodId(191);
    pub const IS_EMPTY: MethodId = MethodId(192);
}

impl std::fmt::Display for MethodId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(name) = self.name() {
            write!(f, "{}(#{})", name, self.0)
        } else if self.is_dynamic() {
            write!(f, "<dynamic>")
        } else {
            write!(f, "<unknown #{}>", self.0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_known_methods_roundtrip() {
        let methods = [
            "map",
            "filter",
            "reduce",
            "len",
            "sum",
            "avg",
            "min",
            "max",
            "sort",
            "first",
            "last",
            "push",
            "pop",
            "join",
            "split",
            "trim",
            "replace",
            "contains",
            "toUpperCase",
            "toLowerCase",
            "type",
            "toString",
            "toFixed",
            "floor",
            "ceil",
            "round",
            "abs",
        ];
        for name in methods {
            let id = MethodId::from_name(name);
            assert!(!id.is_dynamic(), "expected known ID for '{}'", name);
            assert!(id.name().is_some(), "expected name for '{}'", name);
        }
    }

    #[test]
    fn test_unknown_method_is_dynamic() {
        let id = MethodId::from_name("nonexistent_method");
        assert!(id.is_dynamic());
        assert_eq!(id, MethodId::DYNAMIC);
        assert!(id.name().is_none());
    }

    #[test]
    fn test_aliases_resolve_to_same_id() {
        assert_eq!(
            MethodId::from_name("to_string"),
            MethodId::from_name("toString")
        );
        assert_eq!(
            MethodId::from_name("group_by"),
            MethodId::from_name("groupBy")
        );
        assert_eq!(
            MethodId::from_name("index_by"),
            MethodId::from_name("indexBy")
        );
        assert_eq!(
            MethodId::from_name("rollingSum"),
            MethodId::from_name("rolling_sum")
        );
        assert_eq!(
            MethodId::from_name("pctChange"),
            MethodId::from_name("pct_change")
        );
        assert_eq!(
            MethodId::from_name("forwardFill"),
            MethodId::from_name("forward_fill")
        );
        assert_eq!(
            MethodId::from_name("toFixed"),
            MethodId::from_name("to_fixed")
        );
        assert_eq!(
            MethodId::from_name("toUpperCase"),
            MethodId::from_name("to_upper_case")
        );
    }

    #[test]
    fn test_display() {
        assert_eq!(format!("{}", MethodId::MAP), "map(#10)");
        assert_eq!(format!("{}", MethodId::DYNAMIC), "<dynamic>");
        assert_eq!(format!("{}", MethodId(9999)), "<unknown #9999>");
    }
}
