//! Tests for type system features
use super::{FeatureCategory, FeatureTest};

pub const TESTS: &[FeatureTest] = &[
    // === Type Alias ===
    FeatureTest {
        name: "type_alias_simple",
        covers: &["type_alias_def", "type_annotation", "object_type"],
        code: r#"
type Point = { x: number; y: number };
function test() {
    let p: Point = { x: 10, y: 20 };
    return p.x + p.y;
}
"#,
        function: "test",
        category: FeatureCategory::TypeSystem,
        requires_data: false,
    },
    FeatureTest {
        name: "type_alias_with_basic_type",
        covers: &["type_alias_def", "basic_type"],
        code: r#"
type ID = number;
function test() {
    let id: ID = 42;
    return id;
}
"#,
        function: "test",
        category: FeatureCategory::TypeSystem,
        requires_data: false,
    },
    // === Interface Definition ===
    FeatureTest {
        name: "interface_basic",
        covers: &["interface_def", "interface_body", "interface_member"],
        code: r#"
interface Shape {
    area: number;
}
function test() {
    let s: Shape = { area: 100 };
    return s.area;
}
"#,
        function: "test",
        category: FeatureCategory::TypeSystem,
        requires_data: false,
    },
    FeatureTest {
        name: "interface_with_method_signature",
        covers: &["interface_def", "interface_member"],
        code: r#"
interface Calculator {
    add(a: number, b: number): number;
}
function test() {
    let c = { add: (a, b) => a + b };
    return c.add(5, 3);
}
"#,
        function: "test",
        category: FeatureCategory::TypeSystem,
        requires_data: false,
    },
    // === Enum Definition ===
    FeatureTest {
        name: "enum_basic",
        covers: &["enum_def", "enum_members", "enum_member"],
        code: r#"
enum Color { Red, Green, Blue }
function test() {
    return 1;
}
"#,
        function: "test",
        category: FeatureCategory::TypeSystem,
        requires_data: false,
    },
    FeatureTest {
        name: "enum_with_values",
        covers: &["enum_def", "enum_member"],
        code: r#"
enum Status {
    Pending = 0,
    Active = 1,
    Completed = 2
}
function test() {
    return 1;
}
"#,
        function: "test",
        category: FeatureCategory::TypeSystem,
        requires_data: false,
    },
    FeatureTest {
        name: "enum_with_string_values",
        covers: &["enum_def", "enum_member"],
        code: r#"
enum Direction {
    Up = "up",
    Down = "down",
    Left = "left",
    Right = "right"
}
function test() {
    return 1;
}
"#,
        function: "test",
        category: FeatureCategory::TypeSystem,
        requires_data: false,
    },
    // === Union Type ===
    FeatureTest {
        name: "union_type_basic",
        covers: &["union_type", "type_alias_def"],
        code: r#"
type StringOrNumber = string | number;
function test() {
    let x: StringOrNumber = 42;
    return x;
}
"#,
        function: "test",
        category: FeatureCategory::TypeSystem,
        requires_data: false,
    },
    FeatureTest {
        name: "union_type_multiple",
        covers: &["union_type"],
        code: r#"
type Result = number | string | boolean;
function test() {
    let r: Result = true;
    return if r then 1 else 0;
}
"#,
        function: "test",
        category: FeatureCategory::TypeSystem,
        requires_data: false,
    },
    // === Optional Type ===
    FeatureTest {
        name: "optional_type_variable",
        covers: &["optional_type", "type_annotation"],
        code: r#"
function test() {
    let x: number? = null;
    return x ?? 42;
}
"#,
        function: "test",
        category: FeatureCategory::TypeSystem,
        requires_data: false,
    },
    FeatureTest {
        name: "optional_type_in_object",
        covers: &["optional_type", "object_type", "object_type_member"],
        code: r#"
type User = { name: string; age?: number };
function test() {
    let u: User = { name: "Alice" };
    return 1;
}
"#,
        function: "test",
        category: FeatureCategory::TypeSystem,
        requires_data: false,
    },
    // === Type Parameters (Generics) ===
    FeatureTest {
        name: "type_params_single",
        covers: &["type_params", "type_param_name", "function_def"],
        code: r#"
function identity<T>(x: T) -> T {
    return x;
}
function test() {
    return identity(42);
}
"#,
        function: "test",
        category: FeatureCategory::TypeSystem,
        requires_data: false,
    },
    FeatureTest {
        name: "type_params_multiple",
        covers: &["type_params", "type_param_name"],
        code: r#"
function pair<T, U>(a: T, b: U) -> [T, U] {
    return [a, b];
}
function test() {
    let p = pair(1, "hello");
    return p[0];
}
"#,
        function: "test",
        category: FeatureCategory::TypeSystem,
        requires_data: false,
    },
    FeatureTest {
        name: "type_params_with_default",
        covers: &["type_params", "type_param_name"],
        code: r#"
function first<T = int>(arr: Vec<T>) {
    return arr[0];
}
function test() {
    return first([1, 2, 3]);
}
"#,
        function: "test",
        category: FeatureCategory::TypeSystem,
        requires_data: false,
    },
    FeatureTest {
        name: "trait_bound_single",
        covers: &["trait_def", "type_param_name"],
        code: r#"
trait NumericLike {
    to_number(): number
}
impl NumericLike for number {
    method to_number() { self }
}
function first<T: NumericLike>(value: T) {
    value.to_number()
}
function test() {
    return first(3.0);
}
"#,
        function: "test",
        category: FeatureCategory::TypeSystem,
        requires_data: false,
    },
    // === Function Type ===
    FeatureTest {
        name: "function_type_basic",
        covers: &["function_type", "type_alias_def", "type_param_list"],
        code: r#"
type Handler = (x: number) => string;
function test() {
    let h: Handler = x => "value";
    return 1;
}
"#,
        function: "test",
        category: FeatureCategory::TypeSystem,
        requires_data: false,
    },
    FeatureTest {
        name: "function_type_multiple_params",
        covers: &["function_type", "type_param_list", "type_param"],
        code: r#"
type BinaryOp = (a: number, b: number) => number;
function test() {
    let add: BinaryOp = (a, b) => a + b;
    return add(3, 4);
}
"#,
        function: "test",
        category: FeatureCategory::TypeSystem,
        requires_data: false,
    },
    FeatureTest {
        name: "function_type_no_params",
        covers: &["function_type"],
        code: r#"
type Supplier = () => number;
function test() {
    let supply: Supplier = () => 42;
    return supply();
}
"#,
        function: "test",
        category: FeatureCategory::TypeSystem,
        requires_data: false,
    },
    // === Generic Type Usage ===
    FeatureTest {
        name: "generic_type_in_alias",
        covers: &["type_alias_def", "type_params", "generic_type"],
        code: r#"
type Container<T> = { value: T };
function test() {
    let c: Container<number> = { value: 42 };
    return c.value;
}
"#,
        function: "test",
        category: FeatureCategory::TypeSystem,
        requires_data: false,
    },
    FeatureTest {
        name: "trait_with_type_params",
        covers: &["trait_def", "type_params"],
        code: r#"
trait Boxed<T> {
    contents(self): T
}
impl Boxed<number> for number {
    method contents() { self }
}
function test() {
    let b = 100.0
    return b.contents()
}
"#,
        function: "test",
        category: FeatureCategory::TypeSystem,
        requires_data: false,
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn test_type_system_tests_defined() {
        assert!(!TESTS.is_empty());
        // All tests should be in TypeSystem category
        for test in TESTS {
            assert_eq!(test.category, FeatureCategory::TypeSystem);
        }
    }

    #[test]
    fn test_covers_type_system_grammar_rules() {
        let covered: BTreeSet<_> = TESTS
            .iter()
            .flat_map(|t| t.covers.iter().copied())
            .collect();
        // Verify key grammar rules are covered
        assert!(covered.contains(&"type_alias_def"));
        assert!(covered.contains(&"trait_def"));
        assert!(covered.contains(&"enum_def"));
        assert!(covered.contains(&"union_type"));
        assert!(covered.contains(&"optional_type"));
        assert!(covered.contains(&"type_params"));
        assert!(covered.contains(&"function_type"));
    }
}
