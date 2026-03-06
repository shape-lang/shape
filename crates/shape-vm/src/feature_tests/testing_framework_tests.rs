//! Tests for testing framework features
//!
//! This module covers testing framework grammar rules from shape.pest:
//! - test_def: `test "description" { ... }`
//! - assert_statement: `assert x == 5;`
//! - test_setup: `setup { ... }`
//! - test_teardown: `teardown { ... }`
//! - test_tag: `@slow test "..." { }`
//! - expect_statement: `expect(x).toBe(5);`
//! - should_statement: `x should equal 5;`

use super::{FeatureCategory, FeatureTest};

pub const TESTS: &[FeatureTest] = &[
    // === Test Definition ===
    FeatureTest {
        name: "test_def_basic",
        covers: &["test_def", "test_body", "test_case", "test_statements"],
        code: r#"
            test "basic test suite" {
                test "should return correct value" {
                    let x = 42;
                }
            }
            function test() { return 42; }
        "#,
        function: "test",
        category: FeatureCategory::Module,
        requires_data: false,
    },
    FeatureTest {
        name: "test_def_with_it",
        covers: &["test_def", "test_case"],
        code: r#"
            test "using it keyword" {
                it "should work with it" {
                    let y = 10;
                }
            }
            function test() { return 42; }
        "#,
        function: "test",
        category: FeatureCategory::Module,
        requires_data: false,
    },
    // === Assert Statement ===
    FeatureTest {
        name: "assert_statement_basic",
        covers: &["assert_statement", "test_statement"],
        code: r#"
            test "assert tests" {
                test "basic assert" {
                    assert 1 == 1;
                }
            }
            function test() { return 42; }
        "#,
        function: "test",
        category: FeatureCategory::Module,
        requires_data: false,
    },
    FeatureTest {
        name: "assert_statement_with_message",
        covers: &["assert_statement"],
        code: r#"
            test "assert with message" {
                test "named assert" {
                    assert 1 == 1, "one should equal one";
                }
            }
            function test() { return 42; }
        "#,
        function: "test",
        category: FeatureCategory::Module,
        requires_data: false,
    },
    // === Setup and Teardown ===
    FeatureTest {
        name: "test_setup_basic",
        covers: &["test_setup", "test_body"],
        code: r#"
            test "setup example" {
                setup {
                    let shared = 100;
                }
                test "uses setup" {
                    let x = 1;
                }
            }
            function test() { return 42; }
        "#,
        function: "test",
        category: FeatureCategory::Module,
        requires_data: false,
    },
    FeatureTest {
        name: "test_teardown_basic",
        covers: &["test_teardown", "test_body"],
        code: r#"
            test "teardown example" {
                teardown {
                    let cleanup = true;
                }
                test "with teardown" {
                    let x = 1;
                }
            }
            function test() { return 42; }
        "#,
        function: "test",
        category: FeatureCategory::Module,
        requires_data: false,
    },
    FeatureTest {
        name: "test_setup_teardown_combined",
        covers: &["test_setup", "test_teardown", "test_body"],
        code: r#"
            test "full lifecycle" {
                setup {
                    let data = 42;
                }
                teardown {
                    let cleanup = true;
                }
                test "lifecycle test" {
                    let x = 1;
                }
            }
            function test() { return 42; }
        "#,
        function: "test",
        category: FeatureCategory::Module,
        requires_data: false,
    },
    // === Test Tags ===
    FeatureTest {
        name: "test_tag_single",
        covers: &["test_tag", "test_tags", "test_case"],
        code: r#"
            test "tagged tests" {
                test "slow test" -> [slow] {
                    let x = 1;
                }
            }
            function test() { return 42; }
        "#,
        function: "test",
        category: FeatureCategory::Module,
        requires_data: false,
    },
    FeatureTest {
        name: "test_tag_multiple",
        covers: &["test_tag", "test_tags"],
        code: r#"
            test "multiple tags" {
                test "integration test" -> [slow, integration, network] {
                    let x = 1;
                }
            }
            function test() { return 42; }
        "#,
        function: "test",
        category: FeatureCategory::Module,
        requires_data: false,
    },
    FeatureTest {
        name: "test_tag_string",
        covers: &["test_tag"],
        code: r#"
            test "string tags" {
                test "with string tag" -> ["custom-tag"] {
                    let x = 1;
                }
            }
            function test() { return 42; }
        "#,
        function: "test",
        category: FeatureCategory::Module,
        requires_data: false,
    },
    // === Expect Statement ===
    FeatureTest {
        name: "expect_to_be",
        covers: &["expect_statement", "expectation_matcher"],
        code: r#"
            test "expect toBe" {
                test "value comparison" {
                    expect(42).toBe(42);
                }
            }
            function test() { return 42; }
        "#,
        function: "test",
        category: FeatureCategory::Module,
        requires_data: false,
    },
    FeatureTest {
        name: "expect_to_equal",
        covers: &["expect_statement", "expectation_matcher"],
        code: r#"
            test "expect toEqual" {
                test "equality check" {
                    expect(10).toEqual(10);
                }
            }
            function test() { return 42; }
        "#,
        function: "test",
        category: FeatureCategory::Module,
        requires_data: false,
    },
    FeatureTest {
        name: "expect_to_be_close_to",
        covers: &["expect_statement", "expectation_matcher"],
        code: r#"
            test "expect toBeCloseTo" {
                test "float comparison" {
                    expect(3.14159).toBeCloseTo(3.14, 2);
                }
            }
            function test() { return 42; }
        "#,
        function: "test",
        category: FeatureCategory::Module,
        requires_data: false,
    },
    FeatureTest {
        name: "expect_to_be_greater_than",
        covers: &["expect_statement", "expectation_matcher"],
        code: r#"
            test "expect toBeGreaterThan" {
                test "greater comparison" {
                    expect(10).toBeGreaterThan(5);
                }
            }
            function test() { return 42; }
        "#,
        function: "test",
        category: FeatureCategory::Module,
        requires_data: false,
    },
    FeatureTest {
        name: "expect_to_be_less_than",
        covers: &["expect_statement", "expectation_matcher"],
        code: r#"
            test "expect toBeLessThan" {
                test "less comparison" {
                    expect(3).toBeLessThan(10);
                }
            }
            function test() { return 42; }
        "#,
        function: "test",
        category: FeatureCategory::Module,
        requires_data: false,
    },
    FeatureTest {
        name: "expect_to_contain",
        covers: &["expect_statement", "expectation_matcher"],
        code: r#"
            test "expect toContain" {
                test "contains check" {
                    expect([1, 2, 3]).toContain(2);
                }
            }
            function test() { return 42; }
        "#,
        function: "test",
        category: FeatureCategory::Module,
        requires_data: false,
    },
    FeatureTest {
        name: "expect_to_be_truthy",
        covers: &["expect_statement", "expectation_matcher"],
        code: r#"
            test "expect toBeTruthy" {
                test "truthy check" {
                    expect(true).toBeTruthy();
                }
            }
            function test() { return 42; }
        "#,
        function: "test",
        category: FeatureCategory::Module,
        requires_data: false,
    },
    FeatureTest {
        name: "expect_to_be_falsy",
        covers: &["expect_statement", "expectation_matcher"],
        code: r#"
            test "expect toBeFalsy" {
                test "falsy check" {
                    expect(false).toBeFalsy();
                }
            }
            function test() { return 42; }
        "#,
        function: "test",
        category: FeatureCategory::Module,
        requires_data: false,
    },
    // expect_to_throw removed: throw() builtin removed
    FeatureTest {
        name: "expect_to_match_pattern",
        covers: &[
            "expect_statement",
            "expectation_matcher",
            "test_match_options",
        ],
        code: r#"
            test "expect toMatchPattern" {
                test "pattern match" {
                    expect(data).toMatchPattern(hammer, { fuzzy: 0.02 });
                }
            }
            function test() { return 42; }
        "#,
        function: "test",
        category: FeatureCategory::Domain,
        requires_data: false,
    },
    // === Should Statement ===
    FeatureTest {
        name: "should_be",
        covers: &["should_statement", "should_matcher"],
        code: r#"
            test "should be" {
                test "be matcher" {
                    42 should be 42;
                }
            }
            function test() { return 42; }
        "#,
        function: "test",
        category: FeatureCategory::Module,
        requires_data: false,
    },
    FeatureTest {
        name: "should_equal",
        covers: &["should_statement", "should_matcher"],
        code: r#"
            test "should equal" {
                test "equal matcher" {
                    10 should equal 10;
                }
            }
            function test() { return 42; }
        "#,
        function: "test",
        category: FeatureCategory::Module,
        requires_data: false,
    },
    FeatureTest {
        name: "should_contain",
        covers: &["should_statement", "should_matcher"],
        code: r#"
            test "should contain" {
                test "contain matcher" {
                    [1, 2, 3] should contain 2;
                }
            }
            function test() { return 42; }
        "#,
        function: "test",
        category: FeatureCategory::Module,
        requires_data: false,
    },
    FeatureTest {
        name: "should_match",
        covers: &["should_statement", "should_matcher"],
        code: r#"
            test "should match" {
                test "match pattern" {
                    data should match hammer;
                }
            }
            function test() { return 42; }
        "#,
        function: "test",
        category: FeatureCategory::Domain,
        requires_data: false,
    },
    FeatureTest {
        name: "should_be_close_to",
        covers: &["should_statement", "should_matcher"],
        code: r#"
            test "should be_close_to" {
                test "close to matcher" {
                    3.14159 should be_close_to 3.14 within 0.01;
                }
            }
            function test() { return 42; }
        "#,
        function: "test",
        category: FeatureCategory::Module,
        requires_data: false,
    },
    // === Test Fixtures ===
    FeatureTest {
        name: "test_fixture_with_data",
        covers: &["test_fixture_statement"],
        code: r#"
            test "fixture tests" {
                test "with data fixture" {
                    with_data([1, 2, 3]) {
                        let sum = 6;
                    }
                }
            }
            function test() { return 42; }
        "#,
        function: "test",
        category: FeatureCategory::Module,
        requires_data: false,
    },
    FeatureTest {
        name: "test_fixture_with_mock",
        covers: &["test_fixture_statement"],
        code: r#"
            test "mock tests" {
                test "with mock fixture" {
                    with_mock(fetch, { status: 200 }) {
                        let result = true;
                    }
                }
            }
            function test() { return 42; }
        "#,
        function: "test",
        category: FeatureCategory::Module,
        requires_data: false,
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn test_testing_framework_tests_defined() {
        assert!(!TESTS.is_empty());
        // Verify all tests have unique names
        let names: HashSet<_> = TESTS.iter().map(|t| t.name).collect();
        assert_eq!(names.len(), TESTS.len(), "All test names should be unique");
    }

    #[test]
    fn test_covers_testing_rules() {
        let all_covered: HashSet<_> = TESTS.iter().flat_map(|t| t.covers.iter()).collect();
        // Check that key testing framework rules are covered
        assert!(
            all_covered.contains(&"test_def"),
            "Should cover 'test_def' rule"
        );
        assert!(
            all_covered.contains(&"assert_statement"),
            "Should cover 'assert_statement' rule"
        );
        assert!(
            all_covered.contains(&"test_setup"),
            "Should cover 'test_setup' rule"
        );
        assert!(
            all_covered.contains(&"test_teardown"),
            "Should cover 'test_teardown' rule"
        );
        assert!(
            all_covered.contains(&"test_tag"),
            "Should cover 'test_tag' rule"
        );
        assert!(
            all_covered.contains(&"expect_statement"),
            "Should cover 'expect_statement' rule"
        );
        assert!(
            all_covered.contains(&"should_statement"),
            "Should cover 'should_statement' rule"
        );
    }
}
