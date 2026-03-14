//! Import syntax, export syntax, export execution, and
//! import + export combination tests.

use shape_test::shape_test::ShapeTest;

// =============================================================================
// IMPORT SYNTAX — Parsing (~25 tests)
// =============================================================================

#[test]
fn test_import_named_single_parses() {
    ShapeTest::new("from math use { sum }").expect_parse_ok();
}

#[test]
fn test_import_named_multi_parses() {
    ShapeTest::new("from math use { a, b, c }").expect_parse_ok();
}

#[test]
fn test_import_aliased_parses() {
    ShapeTest::new("from math use { orig as alias }").expect_parse_ok();
}

#[test]
fn test_import_namespace_parses() {
    ShapeTest::new("use std::core::json").expect_parse_ok();
}

#[test]
fn test_import_namespace_aliased_parses() {
    ShapeTest::new("use std::core::math as m").expect_parse_ok();
}

#[test]
fn test_import_hierarchical_path_parses() {
    ShapeTest::new("from a::b::c use { item }").expect_parse_ok();
}

#[test]
fn test_import_trailing_comma_parses() {
    ShapeTest::new("from foo use { bar, }").expect_parse_ok();
}

#[test]
fn test_import_mixed_aliases_parses() {
    ShapeTest::new("from m use { a, b as y, c }").expect_parse_ok();
}

#[test]
fn test_import_multiple_statements_parse() {
    ShapeTest::new(
        r#"
        from math use { sum, max }
        from std::core::io use { print }
        use utils
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn test_import_without_semicolon_parses() {
    ShapeTest::new("from m use { a }").expect_parse_ok();
}

#[test]
fn test_import_namespace_without_semicolon_parses() {
    ShapeTest::new("use ml").expect_parse_ok();
}

#[test]
fn test_import_deep_path_parses() {
    ShapeTest::new("from a::b::c::d::e::f::g use { item }").expect_parse_ok();
}

#[test]
fn test_import_hyphenated_path_parses() {
    ShapeTest::new("from my-lib use { helper }").expect_parse_ok();
}

#[test]
fn test_import_underscored_path_parses() {
    ShapeTest::new("from my_lib::sub_mod use { helper_fn }").expect_parse_ok();
}

#[test]
fn test_import_numeric_path_parses() {
    ShapeTest::new("from lib2::v3 use { api }").expect_parse_ok();
}

#[test]
fn test_import_use_hierarchical_two_segment_parses() {
    ShapeTest::new("use std::io").expect_parse_ok();
}

#[test]
fn test_import_use_hierarchical_three_segment_parses() {
    ShapeTest::new("use a::b::c").expect_parse_ok();
}

#[test]
fn test_import_multiple_uses_parse() {
    ShapeTest::new(
        r#"
        use std::core::json
        use std::core::csv
        use std::core::yaml
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn test_import_js_style_rejected() {
    ShapeTest::new("from std::core::csv import { load }").expect_parse_err();
}

#[test]
fn test_import_keyword_rejected() {
    ShapeTest::new("import foo").expect_parse_err();
}

#[test]
fn test_import_wildcard_rejected() {
    ShapeTest::new("from m use *").expect_parse_err();
}

#[test]
fn test_import_empty_braces_rejected() {
    ShapeTest::new("from m use { }").expect_parse_err();
}

#[test]
fn test_import_require_style_rejected() {
    ShapeTest::new("require('module')").expect_parse_err();
}

#[test]
fn test_import_c_include_rejected() {
    ShapeTest::new("#include <module>").expect_parse_err();
}

#[test]
fn test_import_from_without_use_rejected() {
    ShapeTest::new("from m { a }").expect_parse_err();
}

// =============================================================================
// EXPORT SYNTAX — Parsing (~20 tests)
// =============================================================================

#[test]
fn test_export_pub_fn_parses() {
    ShapeTest::new("pub fn add(a, b) { a + b }").expect_parse_ok();
}

#[test]
fn test_export_pub_fn_with_return_type_parses() {
    ShapeTest::new("pub fn add(a: number, b: number) -> number { a + b }").expect_parse_ok();
}

#[test]
fn test_export_pub_fn_generic_parses() {
    ShapeTest::new("pub fn identity<T>(x: T) -> T { x }").expect_parse_ok();
}

#[test]
fn test_export_pub_let_parses() {
    ShapeTest::new("pub let x = 42").expect_parse_ok();
}

#[test]
fn test_export_pub_let_string_parses() {
    ShapeTest::new(r#"pub let name = "hello""#).expect_parse_ok();
}

#[test]
fn test_export_pub_const_parses() {
    ShapeTest::new("pub const PI = 3").expect_parse_ok();
}

#[test]
fn test_export_pub_type_alias_parses() {
    ShapeTest::new("pub type UserId = string").expect_parse_ok();
}

#[test]
fn test_export_pub_enum_parses() {
    ShapeTest::new("pub enum Color { Red, Green, Blue }").expect_parse_ok();
}

#[test]
fn test_export_pub_enum_with_data_parses() {
    ShapeTest::new("pub enum Shape { Circle(number), Rect(number, number) }").expect_parse_ok();
}

#[test]
fn test_export_pub_struct_parses() {
    ShapeTest::new("pub type Point { x: number, y: number }").expect_parse_ok();
}

#[test]
fn test_export_pub_trait_parses() {
    ShapeTest::new("pub trait Display { show(self): string }").expect_parse_ok();
}

#[test]
fn test_export_pub_named_list_parses() {
    ShapeTest::new(
        r#"
        let a = 1
        let b = 2
        pub { a, b }
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn test_export_pub_named_with_alias_parses() {
    ShapeTest::new(
        r#"
        let internal = 100
        pub { internal as external }
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn test_export_pub_named_trailing_comma_parses() {
    ShapeTest::new("pub { a, b, }").expect_parse_ok();
}

#[test]
fn test_export_pub_fn_no_params_parses() {
    ShapeTest::new("pub fn noop() { }").expect_parse_ok();
}

#[test]
fn test_export_pub_fn_many_params_parses() {
    ShapeTest::new("pub fn many(a, b, c, d, e, f) { a + b + c + d + e + f }").expect_parse_ok();
}

#[test]
fn test_export_pub_var_parses() {
    ShapeTest::new("pub let mut mutable_state = 0").expect_parse_ok();
}

#[test]
fn test_export_double_pub_rejected() {
    ShapeTest::new("pub pub fn foo() { 1 }").expect_parse_err();
}

#[test]
fn test_export_pub_if_rejected() {
    ShapeTest::new("pub if true { 1 }").expect_parse_err();
}

#[test]
fn test_export_pub_for_rejected() {
    ShapeTest::new("pub for x in [1] { print(x) }").expect_parse_err();
}

#[test]
fn test_export_pub_while_rejected() {
    ShapeTest::new("pub while true { break }").expect_parse_err();
}

#[test]
fn test_export_pub_bare_rejected() {
    ShapeTest::new("pub;").expect_parse_err();
}

// =============================================================================
// EXPORT — Execution (~5 tests)
// =============================================================================

#[test]
fn test_export_pub_fn_executes() {
    ShapeTest::new(
        r#"
        pub fn add(a, b) { a + b }
        add(10, 20)
    "#,
    )
    .expect_number(30.0);
}

#[test]
fn test_export_pub_const_executes() {
    // `pub const` now works — the semantic analyzer no longer rejects it.
    ShapeTest::new(
        r#"
        pub const MAX_SIZE = 1024
        MAX_SIZE
    "#,
    )
    .expect_number(1024.0);
}

#[test]
fn test_export_pub_fn_with_logic() {
    ShapeTest::new(
        r#"
        pub fn double(x) { x * 2 }
        double(21)
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn test_export_pub_fn_string_return() {
    ShapeTest::new(
        r#"
        pub fn greet(name) { "hello " + name }
        greet("world")
    "#,
    )
    .expect_string("hello world");
}

#[test]
fn test_export_pub_fn_bool_return() {
    ShapeTest::new(
        r#"
        pub fn is_even(n) { n % 2 == 0 }
        is_even(4)
    "#,
    )
    .expect_bool(true);
}

// =============================================================================
// IMPORT + EXPORT COMBINATIONS (~10 tests)
// =============================================================================

#[test]
fn test_combo_import_before_function_parses() {
    ShapeTest::new(
        r#"
        from utils use { format }
        fn display(x) { format(x) }
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn test_combo_import_and_export_parses() {
    ShapeTest::new(
        r#"
        from math use { sqrt }
        pub fn distance(x, y) { sqrt(x * x + y * y) }
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn test_combo_import_and_module_parses() {
    ShapeTest::new(
        r#"
        from base use { Base }
        mod derived {
            fn create() { Base() }
        }
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn test_combo_module_then_code_parses() {
    ShapeTest::new(
        r#"
        mod math { fn add(a, b) { a + b } }
        let result = math.add(1, 2)
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn test_combo_multiple_imports_and_module_parses() {
    ShapeTest::new(
        r#"
        from std::core::io use { read, write }
        from net use { connect }
        mod server {
            fn start() { "running" }
        }
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn test_combo_use_namespace_with_alias_and_usage_parses() {
    ShapeTest::new(
        r#"
        use std::core::math as m
        let x = m.sqrt(4)
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn test_combo_pub_fn_and_import_parses() {
    ShapeTest::new(
        r#"
        from helpers use { format_number }
        pub fn display_value(x) { format_number(x) }
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn test_combo_module_with_imports_inside_parses() {
    ShapeTest::new(
        r#"
        mod app {
            from utils use { format }
            fn display(x) { format(x) }
        }
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn test_combo_module_with_pub_export_list_parses() {
    ShapeTest::new(
        r#"
        mod api {
            fn internal_a() { 1 }
            fn internal_b() { 2 }
            pub { internal_a, internal_b }
        }
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn test_combo_multiple_modules_with_exports_parses() {
    ShapeTest::new(
        r#"
        mod math {
            pub fn add(a, b) { a + b }
            pub const PI = 3
        }
        mod io {
            pub fn read() { "data" }
        }
    "#,
    )
    .expect_parse_ok();
}
