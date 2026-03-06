//! extern "C" fn syntax parsing, type C definitions, pointer types.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// extern "C" fn syntax
// =========================================================================

#[test]
fn extern_c_fn_parses() {
    ShapeTest::new(
        r#"
        extern "C" fn cos(x: number) -> number from "libm.so.6"
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn extern_c_fn_with_as_symbol_parses() {
    ShapeTest::new(
        r#"
        extern "C" fn my_cos(x: number) -> number from "libm.so.6" as "cos"
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn extern_c_fn_multiple_params_parses() {
    ShapeTest::new(
        r#"
        extern "C" fn add(a: int, b: int) -> int from "libcalc.so"
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn extern_c_fn_no_return_parses() {
    ShapeTest::new(
        r#"
        extern "C" fn initialize() -> int from "libinit.so"
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn pub_extern_c_fn_parses() {
    ShapeTest::new(
        r#"
        pub extern "C" fn exported_fn(x: int) -> int from "libexport.so"
    "#,
    )
    .expect_parse_ok();
}

// =========================================================================
// type C definitions
// =========================================================================

// TDD: type C struct syntax may not be in grammar yet
#[test]
fn extern_type_c_struct_parses() {
    ShapeTest::new(
        r#"
        type "C" Point {
            x: number,
            y: number
        }
    "#,
    )
    .expect_parse_ok();
}

// =========================================================================
// Pointer types (cview / cmut)
// =========================================================================

// TDD: cview/cmut pointer types may not be in grammar yet
#[test]
fn cview_pointer_type_parses() {
    ShapeTest::new(
        r#"
        extern "C" fn read_buf(buf: cview<int>, len: int) -> int from "libio.so"
    "#,
    )
    .expect_parse_ok();
}

// TDD: cview/cmut pointer types may not be in grammar yet
#[test]
fn cmut_pointer_type_parses() {
    ShapeTest::new(
        r#"
        extern "C" fn write_buf(buf: cmut<int>, len: int) -> int from "libio.so"
    "#,
    )
    .expect_parse_ok();
}
