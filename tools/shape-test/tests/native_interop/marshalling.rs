//! Int/number/string/struct marshalling to C types.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// Numeric marshalling
// =========================================================================

// TDD: ShapeTest does not expose native function calling (requires .so)
#[test]
fn extern_fn_int_param_syntax() {
    // Verify extern fn with int parameter parses correctly
    ShapeTest::new(
        r#"
        extern "C" fn abs_val(x: int) -> int from "libc.so.6" as "abs"
    "#,
    )
    .expect_parse_ok();
}

// TDD: ShapeTest does not expose native function calling (requires .so)
#[test]
fn extern_fn_number_param_syntax() {
    // number maps to C double
    ShapeTest::new(
        r#"
        extern "C" fn sqrt(x: number) -> number from "libm.so.6"
    "#,
    )
    .expect_parse_ok();
}

// TDD: ShapeTest does not expose native function calling (requires .so)
#[test]
fn extern_fn_multiple_numeric_params() {
    ShapeTest::new(
        r#"
        extern "C" fn pow(base: number, exp: number) -> number from "libm.so.6"
    "#,
    )
    .expect_parse_ok();
}

// =========================================================================
// String marshalling
// =========================================================================

// TDD: ShapeTest does not expose native function calling (requires .so)
#[test]
fn extern_fn_string_param_syntax() {
    // string maps to const char* in C
    ShapeTest::new(
        r#"
        extern "C" fn strlen(s: string) -> int from "libc.so.6"
    "#,
    )
    .expect_parse_ok();
}

// TDD: ShapeTest does not expose native function calling (requires .so)
#[test]
fn extern_fn_string_return_syntax() {
    ShapeTest::new(
        r#"
        extern "C" fn getenv(name: string) -> string from "libc.so.6"
    "#,
    )
    .expect_parse_ok();
}

// =========================================================================
// Struct marshalling
// =========================================================================

// TDD: struct marshalling requires type C definitions and .so fixtures
#[test]
fn extern_fn_with_struct_concept() {
    // Extern functions should support struct parameters once type C is available.
    // For now, verify that regular type + extern fn both parse.
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        extern "C" fn distance(x1: number, y1: number, x2: number, y2: number) -> number from "libgeo.so"
    "#,
    )
    .expect_parse_ok();
}

// TDD: struct marshalling for C interop not yet available
#[test]
fn extern_fn_returning_struct_concept() {
    // Once available, extern fns should be able to return C structs.
    ShapeTest::new(
        r#"
        extern "C" fn make_point(x: number, y: number) -> number from "libgeo.so" as "make_point_x"
    "#,
    )
    .expect_parse_ok();
}

// =========================================================================
// ptr ↔ int (ffi-rebuild §4.6.5 / §4.10 S4)
// =========================================================================

/// `int as ptr` then `ptr as int` is a bit-preserving reinterpretation — the
/// value round-trips (pure compile+run, no `.so` needed).
#[test]
fn ptr_int_reinterpret_round_trips() {
    ShapeTest::new("let a: int = 42\nlet p: ptr = a as ptr\nlet back: int = p as int\nprint(back)")
        .expect_output("42");
}

/// A non-negative integer literal losslessly adopts a `ptr` context — this is
/// what makes the book's manual out-param `ptr_write(cell, 0)` compile.
#[test]
fn non_negative_int_literal_adopts_ptr_context() {
    ShapeTest::new("let p: ptr = 0\nprint(\"ok\")").expect_output("ok");
}

/// `int as ptr` / `ptr as int` must NOT be implicit: a bare int in a ptr slot
/// via a *variable* (not a literal) still requires the explicit cast.
#[test]
fn int_variable_does_not_implicitly_become_ptr() {
    ShapeTest::new("let a: int = 5\nlet p: ptr = a\nprint(\"x\")").expect_run_err();
}
