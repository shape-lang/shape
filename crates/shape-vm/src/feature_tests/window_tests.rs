//! Tests for window and time-related features
//!
//! This module covers:
//! - Window function calls (row_number, rank, etc.)
//! - OVER clauses with PARTITION BY and ORDER BY
//! - Time windows (last, between)
//! - Timeframe specifications
//! - Datetime arithmetic
//! - Compound durations
//! - Window frame clauses
//! - Session windows

use super::{FeatureCategory, FeatureTest};

pub const TESTS: &[FeatureTest] = &[
    // === Window Functions ===
    FeatureTest {
        name: "window_function_row_number",
        covers: &["window_function_call", "over_clause"],
        code: "function test() { return row_number() OVER (ORDER BY x); }",
        function: "test",
        category: FeatureCategory::Domain,
        requires_data: false,
    },
    FeatureTest {
        name: "window_function_partition_by",
        covers: &["window_function_call", "over_clause", "partition_clause"],
        code: "function test() { return sum(value) OVER (PARTITION BY category ORDER BY date); }",
        function: "test",
        category: FeatureCategory::Domain,
        requires_data: false,
    },
    FeatureTest {
        name: "window_function_rank",
        covers: &["window_function_call", "over_clause"],
        code: "function test() { return rank() OVER (PARTITION BY group ORDER BY score DESC); }",
        function: "test",
        category: FeatureCategory::Domain,
        requires_data: false,
    },
    // === Time Windows ===
    FeatureTest {
        name: "time_window_last_days",
        covers: &["time_window", "last_window"],
        code: r#"function test() { return last(5, "days"); }"#,
        function: "test",
        category: FeatureCategory::Domain,
        requires_data: false,
    },
    FeatureTest {
        name: "time_window_last_keyword",
        covers: &["last_window", "duration"],
        code: "function test() { return last 1 year; }",
        function: "test",
        category: FeatureCategory::Domain,
        requires_data: false,
    },
    FeatureTest {
        name: "time_window_last_bars",
        covers: &["time_window", "last_window"],
        code: r#"function test() { return last(100, "bars"); }"#,
        function: "test",
        category: FeatureCategory::Domain,
        requires_data: false,
    },
    // === Between Windows ===
    FeatureTest {
        name: "between_window_dates",
        covers: &["between_window", "datetime_literal"],
        code: r#"function test() { return between @"2020-01-01" and @"2021-01-01"; }"#,
        function: "test",
        category: FeatureCategory::Domain,
        requires_data: false,
    },
    FeatureTest {
        name: "between_window_time_refs",
        covers: &["between_window", "time_ref"],
        code: "function test() { return between @today and @tomorrow; }",
        function: "test",
        category: FeatureCategory::Domain,
        requires_data: false,
    },
    // === Timeframe Specifications ===
    FeatureTest {
        name: "timeframe_spec_5m",
        covers: &["timeframe_spec", "timeframe", "data_ref"],
        code: "function test() { return data(5m)[0]; }",
        function: "test",
        category: FeatureCategory::Domain,
        requires_data: true,
    },
    FeatureTest {
        name: "timeframe_spec_1h",
        covers: &["timeframe_spec", "timeframe"],
        code: "function test() { return data(1h)[0].close; }",
        function: "test",
        category: FeatureCategory::Domain,
        requires_data: true,
    },
    FeatureTest {
        name: "timeframe_spec_daily",
        covers: &["timeframe_spec", "timeframe"],
        code: "function test() { return data(1d)[-1]; }",
        function: "test",
        category: FeatureCategory::Domain,
        requires_data: true,
    },
    // === Datetime Arithmetic ===
    FeatureTest {
        name: "datetime_arithmetic_add_days",
        covers: &["datetime_arithmetic", "datetime_expr", "duration"],
        code: "function test() { return @today + 5d; }",
        function: "test",
        category: FeatureCategory::Domain,
        requires_data: false,
    },
    FeatureTest {
        name: "datetime_arithmetic_sub_hours",
        covers: &["datetime_arithmetic", "datetime_expr", "duration"],
        code: "function test() { return @now - 2h; }",
        function: "test",
        category: FeatureCategory::Domain,
        requires_data: false,
    },
    FeatureTest {
        name: "datetime_arithmetic_complex",
        covers: &["datetime_arithmetic", "datetime_literal", "duration"],
        code: r#"function test() { return @"2020-06-15" + 30d - 1w; }"#,
        function: "test",
        category: FeatureCategory::Domain,
        requires_data: false,
    },
    // === Compound Durations ===
    FeatureTest {
        name: "compound_duration_hm",
        covers: &["compound_duration", "duration_unit"],
        code: "function test() { return 5h30m; }",
        function: "test",
        category: FeatureCategory::Literal,
        requires_data: false,
    },
    FeatureTest {
        name: "compound_duration_dh",
        covers: &["compound_duration", "duration_unit"],
        code: "function test() { return 1d12h; }",
        function: "test",
        category: FeatureCategory::Literal,
        requires_data: false,
    },
    FeatureTest {
        name: "compound_duration_hms",
        covers: &["compound_duration", "duration_unit"],
        code: "function test() { return 2h30m45s; }",
        function: "test",
        category: FeatureCategory::Literal,
        requires_data: false,
    },
    // === Window Frame Clauses ===
    FeatureTest {
        name: "window_frame_rows_between",
        covers: &["window_frame_clause", "frame_bound"],
        code: "function test() { return sum(x) OVER (ORDER BY y ROWS BETWEEN 1 PRECEDING AND 1 FOLLOWING); }",
        function: "test",
        category: FeatureCategory::Domain,
        requires_data: false,
    },
    FeatureTest {
        name: "window_frame_rows_unbounded",
        covers: &["window_frame_clause", "frame_bound"],
        code: "function test() { return sum(x) OVER (ORDER BY y ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW); }",
        function: "test",
        category: FeatureCategory::Domain,
        requires_data: false,
    },
    FeatureTest {
        name: "window_frame_range",
        covers: &["window_frame_clause", "frame_bound"],
        code: "function test() { return avg(price) OVER (ORDER BY date RANGE BETWEEN 7d PRECEDING AND CURRENT ROW); }",
        function: "test",
        category: FeatureCategory::Domain,
        requires_data: false,
    },
    // === Session Windows ===
    FeatureTest {
        name: "session_window_30m",
        covers: &["session_window", "duration"],
        code: "function test() { return session(30m); }",
        function: "test",
        category: FeatureCategory::Domain,
        requires_data: false,
    },
    FeatureTest {
        name: "session_window_with_gap",
        covers: &["session_window", "duration"],
        code: "function test() { return session(5m, gap: 1m); }",
        function: "test",
        category: FeatureCategory::Domain,
        requires_data: false,
    },
    FeatureTest {
        name: "session_window_1h",
        covers: &["session_window", "duration"],
        code: "function test() { return session(1h); }",
        function: "test",
        category: FeatureCategory::Domain,
        requires_data: false,
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn test_window_tests_defined() {
        assert!(!TESTS.is_empty());
        // Should have at least 10 tests
        assert!(
            TESTS.len() >= 10,
            "Expected at least 10 window tests, got {}",
            TESTS.len()
        );
    }

    #[test]
    fn test_window_tests_cover_main_rules() {
        let all_rules: BTreeSet<_> = TESTS
            .iter()
            .flat_map(|t| t.covers.iter().copied())
            .collect();

        // Check that key window rules are covered
        assert!(
            all_rules.contains(&"window_function_call"),
            "Missing window_function_call coverage"
        );
        assert!(
            all_rules.contains(&"over_clause"),
            "Missing over_clause coverage"
        );
        assert!(
            all_rules.contains(&"time_window") || all_rules.contains(&"last_window"),
            "Missing time window coverage"
        );
        assert!(
            all_rules.contains(&"between_window"),
            "Missing between_window coverage"
        );
        assert!(
            all_rules.contains(&"datetime_arithmetic"),
            "Missing datetime_arithmetic coverage"
        );
        assert!(
            all_rules.contains(&"compound_duration"),
            "Missing compound_duration coverage"
        );
        assert!(
            all_rules.contains(&"window_frame_clause"),
            "Missing window_frame_clause coverage"
        );
        assert!(
            all_rules.contains(&"session_window"),
            "Missing session_window coverage"
        );
    }
}
