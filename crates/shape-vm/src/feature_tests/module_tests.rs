//! Tests for module system features
use super::{FeatureCategory, FeatureTest};

pub const TESTS: &[FeatureTest] = &[
    // === Module Use Statement ===
    FeatureTest {
        name: "import_named",
        covers: &["import_stmt", "import_item_list", "import_item"],
        code: r#"
from mylib::module use { foo }
function test() {
    return 42;
}
"#,
        function: "test",
        category: FeatureCategory::Module,
        requires_data: false,
    },
    FeatureTest {
        name: "import_named_multiple",
        covers: &["import_stmt", "import_item_list", "import_item"],
        code: r#"
from mylib::module use { foo, bar, baz }
function test() {
    return 1;
}
"#,
        function: "test",
        category: FeatureCategory::Module,
        requires_data: false,
    },
    FeatureTest {
        name: "import_with_alias",
        covers: &["import_stmt", "import_item"],
        code: r#"
from mylib::module use { foo as myFoo }
function test() {
    return 1;
}
"#,
        function: "test",
        category: FeatureCategory::Module,
        requires_data: false,
    },
    FeatureTest {
        name: "import_module_alias",
        covers: &["import_stmt"],
        code: r#"
use utils as u
function test() {
    return 1;
}
"#,
        function: "test",
        category: FeatureCategory::Module,
        requires_data: false,
    },
    FeatureTest {
        name: "import_simple",
        covers: &["import_stmt"],
        code: r#"
use utils
function test() {
    return 1;
}
"#,
        function: "test",
        category: FeatureCategory::Module,
        requires_data: false,
    },
    FeatureTest {
        name: "import_from_syntax",
        covers: &[
            "import_stmt",
            "import_item_list",
            "import_item",
            "module_path",
        ],
        code: r#"
from std::core::math use { sum, avg }
function test() {
    return 1;
}
"#,
        function: "test",
        category: FeatureCategory::Module,
        requires_data: false,
    },
    // === Pub Item ===
    FeatureTest {
        name: "pub_function",
        covers: &["pub_item", "function_def"],
        code: r#"
pub fn add(a, b) {
    return a + b;
}
function test() {
    return add(1, 2);
}
"#,
        function: "test",
        category: FeatureCategory::Module,
        requires_data: false,
    },
    FeatureTest {
        name: "pub_variable",
        covers: &["pub_item", "variable_decl"],
        code: r#"
pub let PI = 3.14159;
function test() {
    return PI;
}
"#,
        function: "test",
        category: FeatureCategory::Module,
        requires_data: false,
    },
    FeatureTest {
        name: "pub_enum",
        covers: &["pub_item", "enum_def"],
        code: r#"
pub enum Color {
    Red,
    Green,
    Blue
}
function test() {
    return 1;
}
"#,
        function: "test",
        category: FeatureCategory::Module,
        requires_data: false,
    },
    FeatureTest {
        name: "pub_struct",
        covers: &["pub_item", "struct_type_def"],
        code: r#"
pub type Point {
    x: number,
    y: number
}
function test() {
    return 1;
}
"#,
        function: "test",
        category: FeatureCategory::Module,
        requires_data: false,
    },
    FeatureTest {
        name: "pub_type_alias",
        covers: &["pub_item", "type_alias_def"],
        code: r#"
pub type ID = number;
function test() {
    return 1;
}
"#,
        function: "test",
        category: FeatureCategory::Module,
        requires_data: false,
    },
    FeatureTest {
        name: "pub_interface",
        covers: &["pub_item", "interface_def"],
        code: r#"
pub interface Printable {
    fn toString(): string;
}
function test() {
    return 1;
}
"#,
        function: "test",
        category: FeatureCategory::Module,
        requires_data: false,
    },
    // === Export Spec ===
    FeatureTest {
        name: "pub_spec_basic",
        covers: &["pub_item", "export_spec_list", "export_spec"],
        code: r#"
let foo = 1;
let bar = 2;
pub { foo, bar };
function test() {
    return foo + bar;
}
"#,
        function: "test",
        category: FeatureCategory::Module,
        requires_data: false,
    },
    FeatureTest {
        name: "pub_spec_with_alias",
        covers: &["pub_item", "export_spec_list", "export_spec"],
        code: r#"
let internalName = 42;
pub { internalName as publicName };
function test() {
    return internalName;
}
"#,
        function: "test",
        category: FeatureCategory::Module,
        requires_data: false,
    },
    FeatureTest {
        name: "pub_spec_mixed_aliases",
        covers: &["export_spec_list", "export_spec"],
        code: r#"
let a = 1;
let b = 2;
let c = 3;
pub { a, b as renamed, c };
function test() {
    return a + b + c;
}
"#,
        function: "test",
        category: FeatureCategory::Module,
        requires_data: false,
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn test_module_tests_defined() {
        assert!(!TESTS.is_empty());
        // All tests should be in Module category
        for test in TESTS {
            assert_eq!(test.category, FeatureCategory::Module);
        }
    }

    #[test]
    fn test_covers_module_grammar_rules() {
        let covered: BTreeSet<_> = TESTS
            .iter()
            .flat_map(|t| t.covers.iter().copied())
            .collect();
        // Verify key grammar rules are covered
        assert!(covered.contains(&"import_stmt"));
        assert!(covered.contains(&"pub_item"));
        assert!(covered.contains(&"export_spec"));
    }
}
