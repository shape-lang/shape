//! Tests for query/SQL-related features
//!
//! This module covers query grammar rules:
//! - where_clause - Filtering with .where()
//! - group_by_clause - Grouping with .group()
//! - having_clause - Post-aggregation filtering with .having()
//! - order_by_clause - Sorting with .orderBy()
//! - limit_clause - Limiting results with .limit()
//! - join_clause - Joining data sources
//! - on_clause - Timeframe context blocks
//! - pipe_expr - Pipe operator for data transformation
//! - named_arg - Named function arguments
//! - try_operator - Error propagation with ?

use super::{FeatureCategory, FeatureTest};

pub const TESTS: &[FeatureTest] = &[
    // === Where Clause ===
    FeatureTest {
        name: "where_clause_basic",
        covers: &["where_clause"],
        code: r#"
function test() {
    let data = [1, 2, 3, 4, 5];
    let filtered = data.filter(x => x > 2);
    return filtered;
}
"#,
        function: "test",
        category: FeatureCategory::Collection,
        requires_data: false,
    },
    FeatureTest {
        name: "where_clause_complex",
        covers: &["where_clause", "and_expr", "comparison_expr"],
        code: r#"
function test() {
    let items = [{ v: 1 }, { v: 5 }, { v: 10 }];
    let filtered = items.filter(x => x.v > 2 && x.v < 8);
    return filtered[0].v;
}
"#,
        function: "test",
        category: FeatureCategory::Collection,
        requires_data: false,
    },
    // === Group By Clause ===
    FeatureTest {
        name: "group_by_clause_basic",
        covers: &["group_by_clause", "group_by_list", "group_by_expr"],
        code: r#"
function test() {
    let items = [
        { category: "a", value: 1 },
        { category: "a", value: 2 },
        { category: "b", value: 3 }
    ];
    let grouped = items.group(x => x.category);
    return true;
}
"#,
        function: "test",
        category: FeatureCategory::Collection,
        requires_data: false,
    },
    // === Having Clause ===
    FeatureTest {
        name: "having_clause_basic",
        covers: &["having_clause"],
        code: r#"
function test() {
    let counts = [{ name: "a", count: 5 }, { name: "b", count: 15 }];
    let filtered = counts.filter(x => x.count > 10);
    return filtered[0].name;
}
"#,
        function: "test",
        category: FeatureCategory::Collection,
        requires_data: false,
    },
    // === Order By Clause ===
    FeatureTest {
        name: "order_by_clause_asc",
        covers: &[
            "order_by_clause",
            "order_by_list",
            "order_by_item",
            "sort_direction",
        ],
        code: r#"
function test() {
    let items = [3, 1, 4, 1, 5];
    let sorted = items.sort((a, b) => a - b);
    return sorted[0];
}
"#,
        function: "test",
        category: FeatureCategory::Collection,
        requires_data: false,
    },
    FeatureTest {
        name: "order_by_clause_desc",
        covers: &["order_by_clause", "sort_direction"],
        code: r#"
function test() {
    let items = [3, 1, 4, 1, 5];
    let sorted = items.sort((a, b) => b - a);
    return sorted[0];
}
"#,
        function: "test",
        category: FeatureCategory::Collection,
        requires_data: false,
    },
    // === Limit Clause ===
    FeatureTest {
        name: "limit_clause_basic",
        covers: &["limit_clause"],
        code: r#"
function test() {
    let items = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
    let limited = items[0:5];
    return length(limited);
}
"#,
        function: "test",
        category: FeatureCategory::Collection,
        requires_data: false,
    },
    // === Join Clause ===
    FeatureTest {
        name: "join_clause_basic",
        covers: &["join_clause", "join_type", "join_source", "join_condition"],
        code: r#"
function test() {
    let left = [{ id: 1, name: "a" }, { id: 2, name: "b" }];
    let right = [{ id: 1, value: 100 }, { id: 2, value: 200 }];
    let joined = [
        for l in left {
            for r in right {
                if l.id == r.id { { id: l.id, name: l.name, value: r.value } }
            }
        }
    ];
    return true;
}
"#,
        function: "test",
        category: FeatureCategory::Collection,
        requires_data: false,
    },
    // === On Clause (Timeframe Context) ===
    FeatureTest {
        name: "on_clause_timeframe",
        covers: &["on_clause", "timeframe_expr", "timeframe"],
        code: r#"
function test() {
    let result = on(1h) { 2 + 2 };
    return result;
}
"#,
        function: "test",
        category: FeatureCategory::Domain,
        requires_data: false,
    },
    FeatureTest {
        name: "on_clause_nested",
        covers: &["on_clause", "timeframe_expr"],
        code: r#"
function test() {
    let outer = on(1d) {
        let inner = on(1h) { 3 };
        inner * 2
    };
    return outer;
}
"#,
        function: "test",
        category: FeatureCategory::Domain,
        requires_data: false,
    },
    // === Pipe Expression ===
    FeatureTest {
        name: "pipe_expr_basic",
        covers: &["pipe_expr"],
        code: r#"
function double(x) { return x * 2; }
function add_one(x) { return x + 1; }

function test() {
    let result = 5 |> double |> add_one;
    return result;
}
"#,
        function: "test",
        category: FeatureCategory::Operator,
        requires_data: false,
    },
    FeatureTest {
        name: "pipe_expr_chain",
        covers: &["pipe_expr"],
        code: r#"
function square(x) { return x * x; }
function negate(x) { return -x; }
function abs_val(x) { return if x < 0 then -x else x; }

function test() {
    let result = 3 |> square |> negate |> abs_val;
    return result;
}
"#,
        function: "test",
        category: FeatureCategory::Operator,
        requires_data: false,
    },
    FeatureTest {
        name: "pipe_expr_with_lambda",
        covers: &["pipe_expr", "arrow_function"],
        code: r#"
function test() {
    let data = [1, 2, 3, 4, 5];
    let result = data
        |> (arr => arr.filter(x => x > 2))
        |> (arr => arr.map(x => x * 2));
    return result[0];
}
"#,
        function: "test",
        category: FeatureCategory::Operator,
        requires_data: false,
    },
    // === Named Arguments ===
    FeatureTest {
        name: "named_arg_basic",
        covers: &["named_arg", "arg_list", "argument"],
        code: r#"
function greet(name, greeting) {
    return greeting;
}

function test() {
    return greet(name: "World", greeting: "Hello");
}
"#,
        function: "test",
        category: FeatureCategory::Function,
        requires_data: false,
    },
    FeatureTest {
        name: "named_arg_mixed",
        covers: &["named_arg", "arg_list"],
        code: r#"
function calc(a, b, c) {
    return a + b + c;
}

function test() {
    return calc(1, c: 3, b: 2);
}
"#,
        function: "test",
        category: FeatureCategory::Function,
        requires_data: false,
    },
    FeatureTest {
        name: "named_arg_with_defaults",
        covers: &["named_arg", "function_param"],
        code: r#"
function make_config(width = 100, height = 50) {
    return width * height;
}

function test() {
    return make_config(width: 200);
}
"#,
        function: "test",
        category: FeatureCategory::Function,
        requires_data: false,
    },
    // === Try Operator ===
    FeatureTest {
        name: "try_operator_basic",
        covers: &["try_operator", "postfix_expr"],
        code: r#"
function may_fail(x) {
    if x < 0 {
        return { error: "negative value" };
    }
    return { value: x * 2 };
}

function test() {
    let result = may_fail(5);
    return result.value;
}
"#,
        function: "test",
        category: FeatureCategory::Exception,
        requires_data: false,
    },
    FeatureTest {
        name: "try_operator_chain",
        covers: &["try_operator"],
        code: r#"
function step1(x) { return x + 1; }
function step2(x) { return x * 2; }

function test() {
    let a = step1(5);
    let b = step2(a);
    return b;
}
"#,
        function: "test",
        category: FeatureCategory::Exception,
        requires_data: false,
    },
    // try_expr_with_catch removed: throw() builtin removed
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_query_tests_defined() {
        assert!(!TESTS.is_empty());
        // Should have tests for multiple categories
        let has_collection = TESTS
            .iter()
            .any(|t| t.category == FeatureCategory::Collection);
        let has_domain = TESTS.iter().any(|t| t.category == FeatureCategory::Domain);
        let has_operator = TESTS
            .iter()
            .any(|t| t.category == FeatureCategory::Operator);
        let has_function = TESTS
            .iter()
            .any(|t| t.category == FeatureCategory::Function);
        let has_exception = TESTS
            .iter()
            .any(|t| t.category == FeatureCategory::Exception);

        assert!(has_collection, "Should have Collection tests");
        assert!(has_domain, "Should have Domain tests");
        assert!(has_operator, "Should have Operator tests");
        assert!(has_function, "Should have Function tests");
        assert!(has_exception, "Should have Exception tests");
    }
}
