//! Tests for stream grammar rules
//!
//! This module covers:
//! - Stream definitions
//! - Stream state declarations
//! - Stream event handlers (on_tick, on_bar)
//! - Stream lifecycle

use super::{FeatureCategory, FeatureTest};

pub const TESTS: &[FeatureTest] = &[
    // === Stream Definitions ===
    FeatureTest {
        name: "stream_def_basic",
        covers: &["stream_def"],
        code: r#"
            stream Counter {
                state { count: 0 }
                on_tick { state.count = state.count + 1; }
            }
            function test() { return 1; }
        "#,
        function: "test",
        category: FeatureCategory::Domain,
        requires_data: false,
    },
    FeatureTest {
        name: "stream_def_with_params",
        covers: &["stream_def", "function_params"],
        code: r#"
            stream MovingAverage(period) {
                state { sum: 0, count: 0 }
                on_tick {
                    state.sum = state.sum + data[0].close;
                    state.count = state.count + 1;
                }
            }
            function test() { return 1; }
        "#,
        function: "test",
        category: FeatureCategory::Domain,
        requires_data: false,
    },
    // === Stream State ===
    FeatureTest {
        name: "stream_state_simple",
        covers: &["stream_state", "object_literal"],
        code: r#"
            stream TestStream {
                state { value: 0 }
                on_tick { }
            }
            function test() { return 1; }
        "#,
        function: "test",
        category: FeatureCategory::Domain,
        requires_data: false,
    },
    FeatureTest {
        name: "stream_state_multiple_fields",
        covers: &["stream_state", "object_literal", "object_field"],
        code: r#"
            stream TestStream {
                state {
                    count: 0,
                    sum: 0.0,
                    high: -999999,
                    low: 999999,
                    active: false
                }
                on_tick { }
            }
            function test() { return 1; }
        "#,
        function: "test",
        category: FeatureCategory::Domain,
        requires_data: false,
    },
    FeatureTest {
        name: "stream_state_nested",
        covers: &["stream_state", "object_literal"],
        code: r#"
            stream TestStream {
                state {
                    position: { size: 0, entry_price: 0 },
                    stats: { wins: 0, losses: 0 }
                }
                on_tick { }
            }
            function test() { return 1; }
        "#,
        function: "test",
        category: FeatureCategory::Domain,
        requires_data: false,
    },
    // === Stream on_tick Handler ===
    FeatureTest {
        name: "stream_on_tick_basic",
        covers: &["stream_on_tick", "block_expr"],
        code: r#"
            stream TickCounter {
                state { ticks: 0 }
                on_tick {
                    state.ticks = state.ticks + 1;
                }
            }
            function test() { return 1; }
        "#,
        function: "test",
        category: FeatureCategory::Domain,
        requires_data: false,
    },
    FeatureTest {
        name: "stream_on_tick_with_logic",
        covers: &["stream_on_tick", "if_stmt", "block_expr"],
        code: r#"
            stream HighTracker {
                state { high: 0 }
                on_tick {
                    if (data[0].high > state.high) {
                        state.high = data[0].high;
                    }
                }
            }
            function test() { return 1; }
        "#,
        function: "test",
        category: FeatureCategory::Domain,
        requires_data: false,
    },
    FeatureTest {
        name: "stream_on_tick_emit",
        covers: &["stream_on_tick", "function_call"],
        code: r#"
            stream SignalEmitter {
                state { triggered: false }
                on_tick {
                    if (data[0].close > data[1].close && !state.triggered) {
                        emit("buy_signal", { price: data[0].close });
                        state.triggered = true;
                    }
                }
            }
            function test() { return 1; }
        "#,
        function: "test",
        category: FeatureCategory::Domain,
        requires_data: false,
    },
    // === Stream on_bar Handler ===
    FeatureTest {
        name: "stream_on_bar_basic",
        covers: &["stream_on_bar", "block_expr"],
        code: r#"
            stream BarCounter {
                state { bars: 0 }
                on_bar {
                    state.bars = state.bars + 1;
                }
            }
            function test() { return 1; }
        "#,
        function: "test",
        category: FeatureCategory::Domain,
        requires_data: false,
    },
    FeatureTest {
        name: "stream_on_bar_aggregation",
        covers: &["stream_on_bar", "block_expr", "assignment"],
        code: r#"
            stream VolumeAccumulator {
                state { total_volume: 0, bar_count: 0 }
                on_bar {
                    state.total_volume = state.total_volume + bar.volume;
                    state.bar_count = state.bar_count + 1;
                }
            }
            function test() { return 1; }
        "#,
        function: "test",
        category: FeatureCategory::Domain,
        requires_data: false,
    },
    FeatureTest {
        name: "stream_on_bar_with_condition",
        covers: &["stream_on_bar", "if_stmt"],
        code: r#"
            stream GreenBarCounter {
                state { green_bars: 0, red_bars: 0 }
                on_bar {
                    if (bar.close > bar.open) {
                        state.green_bars = state.green_bars + 1;
                    } else {
                        state.red_bars = state.red_bars + 1;
                    }
                }
            }
            function test() { return 1; }
        "#,
        function: "test",
        category: FeatureCategory::Domain,
        requires_data: false,
    },
    // === Combined Stream Patterns ===
    FeatureTest {
        name: "stream_combined_handlers",
        covers: &[
            "stream_def",
            "stream_state",
            "stream_on_tick",
            "stream_on_bar",
        ],
        code: r#"
            stream CompleteStream {
                state {
                    tick_count: 0,
                    bar_count: 0,
                    last_price: 0
                }

                on_tick {
                    state.tick_count = state.tick_count + 1;
                    state.last_price = data[0].close;
                }

                on_bar {
                    state.bar_count = state.bar_count + 1;
                }
            }
            function test() { return 1; }
        "#,
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
    fn test_stream_tests_defined() {
        assert!(!TESTS.is_empty());
        // Should have at least 8 tests
        assert!(
            TESTS.len() >= 8,
            "Expected at least 8 stream tests, got {}",
            TESTS.len()
        );
    }

    #[test]
    fn test_stream_tests_cover_main_rules() {
        let all_rules: BTreeSet<_> = TESTS
            .iter()
            .flat_map(|t| t.covers.iter().copied())
            .collect();

        // Check that key stream rules are covered
        assert!(
            all_rules.contains(&"stream_def"),
            "Missing stream_def coverage"
        );
        assert!(
            all_rules.contains(&"stream_state"),
            "Missing stream_state coverage"
        );
        assert!(
            all_rules.contains(&"stream_on_tick"),
            "Missing stream_on_tick coverage"
        );
        assert!(
            all_rules.contains(&"stream_on_bar"),
            "Missing stream_on_bar coverage"
        );
    }
}
