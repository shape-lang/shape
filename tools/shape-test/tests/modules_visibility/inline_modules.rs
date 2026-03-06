//! Inline module tests — parsing and runtime execution.
//!
//! NOTE: BUG-4 (semantic analyzer not registering inline module names) is now
//! fixed. Single-level module member access (e.g., `M.f()`) works correctly.
//! Deeply nested module access (e.g., `A.B.C.deep()`) still has runtime
//! limitations with TypedObject resolution.

use shape_test::shape_test::ShapeTest;

// =============================================================================
// INLINE MODULES — Parsing (~15 tests)
// =============================================================================

#[test]
fn test_mod_simple_parses() {
    ShapeTest::new(
        r#"
        mod M {
            fn f() { 1 }
        }
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn test_mod_nested_two_levels_parses() {
    ShapeTest::new(
        r#"
        mod A {
            mod B {
                fn f() { 2 }
            }
        }
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn test_mod_triple_nested_parses() {
    ShapeTest::new(
        r#"
        mod A {
            mod B {
                mod C {
                    fn deep() { 42 }
                }
            }
        }
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn test_mod_with_const_parses() {
    ShapeTest::new(
        r#"
        mod M {
            const PI = 3
        }
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn test_mod_with_multiple_functions_parses() {
    ShapeTest::new(
        r#"
        mod M {
            fn add(a, b) { a + b }
            fn sub(a, b) { a - b }
            fn mul(a, b) { a * b }
        }
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn test_mod_with_enum_parses() {
    ShapeTest::new(
        r#"
        mod types {
            enum Color { Red, Green, Blue }
        }
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn test_mod_with_struct_parses() {
    ShapeTest::new(
        r#"
        mod models {
            type Point { x: number, y: number }
        }
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn test_mod_empty_parses() {
    ShapeTest::new("mod empty { }").expect_parse_ok();
}

#[test]
fn test_mod_with_let_parses() {
    ShapeTest::new(
        r#"
        mod state {
            let counter = 0
        }
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn test_mod_with_closure_parses() {
    ShapeTest::new(
        r#"
        mod util {
            fn apply(f, x) { f(x) }
        }
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn test_mod_with_pub_items_parses() {
    ShapeTest::new(
        r#"
        mod api {
            pub fn endpoint() { "ok" }
            fn internal_helper() { "secret" }
        }
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn test_mod_multiple_top_level_parses() {
    ShapeTest::new(
        r#"
        mod a { fn fa() { 1 } }
        mod b { fn fb() { 2 } }
        mod c { fn fc() { 3 } }
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn test_mod_with_trait_parses() {
    ShapeTest::new(
        r#"
        mod traits {
            trait Display { show(self): string }
        }
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn test_mod_mixed_item_types_parses() {
    ShapeTest::new(
        r#"
        mod kitchen_sink {
            const VERSION = 1
            type Config { debug: bool }
            enum Level { Low, High }
            fn process() { 1 }
            pub fn public_api() { 2 }
        }
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn test_mod_with_import_inside_parses() {
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

// =============================================================================
// INLINE MODULES — Execution via ShapeEngine (~10 tests)
// BUG-4 fixed: single-level module member access now works.
// Nested module access (A.B.f()) still has runtime limitations.
// =============================================================================

#[test]
fn test_mod_simple_function_call_runtime() {
    // BUG-4 fixed: semantic analyzer now registers module names in scope
    ShapeTest::new(
        r#"
        mod M {
            fn f() { 1 }
        }
        M.f()
    "#,
    )
    .expect_number(1.0);
}

#[test]
fn test_mod_nested_access_runtime() {
    // Module 'A' is now registered, but nested module member access (A.B.f())
    // hits a runtime limitation: B is resolved as a TypedObject without method 'f'
    ShapeTest::new(
        r#"
        mod A {
            mod B {
                fn f() { 2 }
            }
        }
        A.B.f()
    "#,
    )
    .expect_run_err_contains("Unknown method");
}

#[test]
fn test_mod_const_access_runtime() {
    // BUG-4 fixed: semantic analyzer now registers module names in scope
    ShapeTest::new(
        r#"
        mod M {
            const PI = 3
        }
        M.PI
    "#,
    )
    .expect_number(3.0);
}

#[test]
fn test_mod_multiple_functions_runtime() {
    // BUG-4 fixed: semantic analyzer now registers module names in scope
    ShapeTest::new(
        r#"
        mod math {
            fn add(a, b) { a + b }
            fn sub(a, b) { a - b }
        }
        math.add(1, 2)
    "#,
    )
    .expect_number(3.0);
}

#[test]
fn test_mod_function_not_global_runtime() {
    // Module-scoped function should NOT be accessible globally
    // (This is actually correct behavior, not a bug)
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
fn test_mod_triple_nested_access_runtime() {
    // Module 'A' is now registered, but deeply nested module member access
    // (A.B.C.deep()) hits a runtime limitation with nested TypedObject resolution
    ShapeTest::new(
        r#"
        mod A {
            mod B {
                mod C {
                    fn deep() { 99 }
                }
            }
        }
        A.B.C.deep()
    "#,
    )
    .expect_run_err_contains("Unknown method");
}

#[test]
fn test_mod_empty_followed_by_expression() {
    // Empty module should not interfere with subsequent code
    ShapeTest::new(
        r#"
        mod empty { }
        42
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn test_mod_with_function_then_independent_expr() {
    // Module definition should not block independent code after it
    ShapeTest::new(
        r#"
        mod M {
            fn f() { 1 }
        }
        10 + 20
    "#,
    )
    .expect_number(30.0);
}

#[test]
fn test_mod_multiple_then_independent_expr() {
    // Multiple modules then independent expression
    ShapeTest::new(
        r#"
        mod a { fn x() { 1 } }
        mod b { fn y() { 2 } }
        100
    "#,
    )
    .expect_number(100.0);
}

#[test]
fn test_mod_const_does_not_leak() {
    // BUG: const inside a module should not be visible globally
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
