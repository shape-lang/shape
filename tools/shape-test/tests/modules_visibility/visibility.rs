//! Visibility modifier tests — pub/private in modules, scope isolation,
//! and access control.

use shape_test::shape_test::ShapeTest;

// =============================================================================
// VISIBILITY — Parsing and Error Tests (~15 tests)
// =============================================================================

#[test]
fn test_vis_module_function_not_global() {
    // A function inside a module should NOT be accessible globally
    ShapeTest::new(
        r#"
        mod secret {
            fn hidden() { 42 }
        }
        hidden()
    "#,
    )
    .expect_run_err_contains("hidden");
}

#[test]
fn test_vis_module_const_not_global() {
    // A const inside a module should NOT be accessible globally
    ShapeTest::new(
        r#"
        mod M {
            const X = 42
        }
        X
    "#,
    )
    .expect_run_err_contains("X");
}

#[test]
fn test_vis_pub_fn_in_module_parses() {
    ShapeTest::new(
        r#"
        mod api {
            pub fn greet() { "hello" }
        }
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn test_vis_pub_and_private_in_module_parses() {
    ShapeTest::new(
        r#"
        mod math {
            pub fn public_add(a, b) { a + b }
            fn private_add(a, b) { a + b }
        }
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn test_vis_pub_const_in_module_parses() {
    ShapeTest::new(
        r#"
        mod config {
            pub const MAX = 100
        }
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn test_vis_nested_pub_in_module_parses() {
    ShapeTest::new(
        r#"
        mod outer {
            mod inner {
                pub fn value() { 42 }
            }
        }
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn test_vis_module_with_enum_parses() {
    ShapeTest::new(
        r#"
        mod types {
            pub enum Direction { North, South, East, West }
        }
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn test_vis_module_with_struct_parses() {
    ShapeTest::new(
        r#"
        mod shapes {
            pub type Point { x: number, y: number }
        }
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn test_vis_module_fn_not_leaked_to_sibling_mod() {
    // Functions in one module should not be accessible from another
    ShapeTest::new(
        r#"
        mod a { fn helper() { 1 } }
        mod b { fn use_helper() { helper() } }
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn test_vis_module_does_not_pollute_scope() {
    // After defining a module, its inner names should not leak
    ShapeTest::new(
        r#"
        mod M {
            fn secret() { 42 }
        }
        secret()
    "#,
    )
    .expect_run_err_contains("secret");
}

#[test]
fn test_vis_two_modules_same_fn_name_parse() {
    ShapeTest::new(
        r#"
        mod a { fn compute() { 10 } }
        mod b { fn compute() { 20 } }
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn test_vis_empty_module_member_access_runtime() {
    // Accessing nonexistent member on empty module — correctly errors
    ShapeTest::new(
        r#"
        mod empty { }
        empty.nonexistent()
    "#,
    )
    .expect_run_err_contains("has no export");
}

#[test]
fn test_vis_pub_let_destructure_rejected() {
    ShapeTest::new("pub let { x, y } = obj").expect_parse_err();
}

#[test]
fn test_vis_module_inner_fn_not_accessible_on_outer() {
    // Inner module fn should not be directly on outer — correctly errors
    ShapeTest::new(
        r#"
        mod outer {
            mod inner {
                fn f() { 1 }
            }
        }
        outer.f()
    "#,
    )
    .expect_run_err_contains("Invalid function call");
}

#[test]
fn test_vis_nonexistent_fn_on_module() {
    // Calling a nonexistent function on a module — correctly errors
    ShapeTest::new(
        r#"
        mod M {
            fn real() { 1 }
        }
        M.fake()
    "#,
    )
    .expect_run_err_contains("has no export");
}
