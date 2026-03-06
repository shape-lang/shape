//! Tests for annotation features
//!
//! This module covers annotation grammar rules from shape.pest:
//! - annotation: `@my_ann`
//! - annotation_args: `@timeout(5000)`
//! - annotation_def: `annotation @custom(x) { }`

use super::{FeatureCategory, FeatureTest};

pub const TESTS: &[FeatureTest] = &[
    // === Basic Annotations ===
    FeatureTest {
        name: "annotation_simple",
        covers: &["annotation", "annotation_name", "annotations"],
        code: r#"annotation test_ann() {} @test_ann function test() { return 42; }"#,
        function: "test",
        category: FeatureCategory::Function,
        requires_data: false,
    },
    FeatureTest {
        name: "annotation_with_args",
        covers: &["annotation", "annotation_args", "annotation_name"],
        code: r#"annotation timeout(ms) {} @timeout(5000) function test() { return 42; }"#,
        function: "test",
        category: FeatureCategory::Function,
        requires_data: false,
    },
    FeatureTest {
        name: "annotation_multiple_args",
        covers: &["annotation", "annotation_args"],
        code: r#"annotation config(name, value) {} @config("option1", 100) function test() { return 42; }"#,
        function: "test",
        category: FeatureCategory::Function,
        requires_data: false,
    },
    FeatureTest {
        name: "annotation_multiple",
        covers: &["annotations", "annotation"],
        code: r#"annotation ann_a() {} annotation ann_b() {} @ann_a @ann_b function test() { return 42; }"#,
        function: "test",
        category: FeatureCategory::Function,
        requires_data: false,
    },
    // === Annotation Definitions ===
    FeatureTest {
        name: "annotation_def_simple",
        covers: &["annotation_def", "annotation_def_params"],
        code: r#"
            annotation warmup(amount) {
                return amount;
            }
            function test() { return 42; }
        "#,
        function: "test",
        category: FeatureCategory::Module,
        requires_data: false,
    },
    FeatureTest {
        name: "annotation_def_multiple_params",
        covers: &["annotation_def", "annotation_def_params"],
        code: r#"
            annotation range(min, max) {
                return min + max;
            }
            function test() { return 42; }
        "#,
        function: "test",
        category: FeatureCategory::Module,
        requires_data: false,
    },
    FeatureTest {
        name: "annotation_def_no_params",
        covers: &["annotation_def"],
        code: r#"
            annotation memoize() {
                return null;
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
    fn test_annotation_tests_defined() {
        assert!(!TESTS.is_empty());
        // Verify all tests have unique names
        let names: HashSet<_> = TESTS.iter().map(|t| t.name).collect();
        assert_eq!(names.len(), TESTS.len(), "All test names should be unique");
    }

    #[test]
    fn test_covers_annotation_rules() {
        let all_covered: HashSet<_> = TESTS.iter().flat_map(|t| t.covers.iter()).collect();
        // Check that key annotation rules are covered
        assert!(
            all_covered.contains(&"annotation"),
            "Should cover 'annotation' rule"
        );
        assert!(
            all_covered.contains(&"annotation_args"),
            "Should cover 'annotation_args' rule"
        );
        assert!(
            all_covered.contains(&"annotation_def"),
            "Should cover 'annotation_def' rule"
        );
    }
}
