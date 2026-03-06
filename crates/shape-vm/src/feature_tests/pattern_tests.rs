//! Tests for pattern-related features
//!
//! This module covers pattern grammar rules:
//! - pattern_def - Pattern definitions
//! - pattern_ref - Pattern references using find()
//! - inline_pattern - Inline patterns in find expressions

use super::{FeatureCategory, FeatureTest};

pub const TESTS: &[FeatureTest] = &[
    // === Pattern Definition ===
    FeatureTest {
        name: "pattern_def_basic",
        covers: &["pattern_def", "pattern_body", "condition_list"],
        code: r#"
pattern simple_up {
    data[0].close > data[0].open
}

function test() {
    return true;
}
"#,
        function: "test",
        category: FeatureCategory::Domain,
        requires_data: false,
    },
    FeatureTest {
        name: "pattern_def_with_params",
        covers: &[
            "pattern_def",
            "pattern_params",
            "pattern_param_list",
            "pattern_param",
        ],
        code: r#"
pattern threshold_cross(level) {
    data[0].close > level
}

function test() {
    return true;
}
"#,
        function: "test",
        category: FeatureCategory::Domain,
        requires_data: false,
    },
    FeatureTest {
        name: "pattern_def_with_threshold",
        covers: &["pattern_def", "threshold"],
        code: r#"
pattern fuzzy_up ~0.02 {
    data[0].close > data[0].open
}

function test() {
    return true;
}
"#,
        function: "test",
        category: FeatureCategory::Domain,
        requires_data: false,
    },
    FeatureTest {
        name: "pattern_def_with_statements",
        covers: &["pattern_def", "pattern_statement_list", "pattern_statement"],
        code: r#"
pattern complex_pattern(c) {
    let body = abs(c.close - c.open);
    let upper_wick = c.high - max(c.open, c.close);
    body > upper_wick * 2
}

function test() {
    return true;
}
"#,
        function: "test",
        category: FeatureCategory::Domain,
        requires_data: false,
    },
    FeatureTest {
        name: "pattern_def_with_annotation",
        covers: &["pattern_def", "annotations", "annotation"],
        code: r#"
@export
pattern hammer(c) {
    let body = abs(c.close - c.open);
    let lower_wick = min(c.open, c.close) - c.low;
    lower_wick > body * 2
}

function test() {
    return true;
}
"#,
        function: "test",
        category: FeatureCategory::Domain,
        requires_data: false,
    },
    FeatureTest {
        name: "pattern_def_weighted_conditions",
        covers: &["pattern_def", "condition", "weight"],
        code: r#"
pattern weighted_pattern {
    data[0].close > data[0].open weight 2
    and data[0].volume > 1000 weight 1
}

function test() {
    return true;
}
"#,
        function: "test",
        category: FeatureCategory::Domain,
        requires_data: false,
    },
    // === Pattern Reference ===
    FeatureTest {
        name: "pattern_ref_by_name",
        covers: &["pattern_ref", "pattern_name"],
        code: r#"
function test() {
    let p = pattern::hammer;
    return p;
}
"#,
        function: "test",
        category: FeatureCategory::Domain,
        requires_data: false,
    },
    // === Inline Pattern ===
    FeatureTest {
        name: "inline_pattern_basic",
        covers: &["inline_pattern", "pattern_body"],
        code: r#"
function test() {
    let inline = { data[0].close > data[0].open };
    return true;
}
"#,
        function: "test",
        category: FeatureCategory::Domain,
        requires_data: false,
    },
    FeatureTest {
        name: "inline_pattern_with_lambda",
        covers: &["inline_pattern", "arrow_function"],
        code: r#"
function test() {
    let checker = c => c.close > c.open;
    return checker({ close: 10, open: 5 });
}
"#,
        function: "test",
        category: FeatureCategory::Domain,
        requires_data: false,
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pattern_tests_defined() {
        assert!(!TESTS.is_empty());
        // All tests should be in Domain category
        for test in TESTS {
            assert_eq!(test.category, FeatureCategory::Domain);
        }
    }
}
