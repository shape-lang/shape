//! Format string (f-string) tests.
//!
//! Covers: f"...", f$"...", f#"..." interpolation.

use shape_test::shape_test::ShapeTest;

fn expect_vm_and_jit_string(source: &str, expected: &str) {
    ShapeTest::new(source).expect_string(expected);
    ShapeTest::new(source).with_jit().expect_string(expected);
}

#[test]
fn fstring_basic_variable() {
    ShapeTest::new(
        r#"
        let name = "world"
        f"hello {name}"
    "#,
    )
    .expect_string("hello world");
}

#[test]
fn fstring_integer_interpolation() {
    ShapeTest::new(
        r#"
        let x = 42
        f"value is {x}"
    "#,
    )
    .expect_string("value is 42");
}

#[test]
fn fstring_expression_interpolation() {
    ShapeTest::new(
        r#"
        let a = 3
        let b = 4
        f"sum is {a + b}"
    "#,
    )
    .expect_string("sum is 7");
}

#[test]
fn fstring_multiple_interpolations() {
    ShapeTest::new(
        r#"
        let first = "John"
        let last = "Doe"
        f"{first} {last}"
    "#,
    )
    .expect_string("John Doe");
}

#[test]
fn fstring_with_string_method() {
    ShapeTest::new(
        r#"
        let name = "world"
        f"HELLO {name.toUpperCase()}"
    "#,
    )
    .expect_string("HELLO WORLD");
}

#[test]
fn fstring_empty_interpolation_at_edges() {
    ShapeTest::new(
        r#"
        let x = "edge"
        f"{x} test"
    "#,
    )
    .expect_string("edge test");
}

// f$ uses ${} for interpolation — bare {} is literal text
#[test]
fn fstring_dollar_literal_braces() {
    ShapeTest::new(
        r#"
        let x = 10
        f$"value: {x}"
    "#,
    )
    .expect_string("value: {x}");
}

#[test]
fn fstring_dollar_interpolation() {
    ShapeTest::new(
        r#"
        let x = 10
        f$"value: ${x}"
    "#,
    )
    .expect_string("value: 10");
}

// f# uses #{} for interpolation — bare {} is literal text
#[test]
fn fstring_hash_literal_braces() {
    ShapeTest::new(
        r#"
        let x = 5
        f#"raw {x}"
    "#,
    )
    .expect_string("raw {x}");
}

#[test]
fn fstring_hash_interpolation() {
    ShapeTest::new(
        r##"
        let x = 5
        f#"raw #{x}"
    "##,
    )
    .expect_string("raw 5");
}

// =========================================================================
// Regression: nested strings inside {}-interpolation blocks
// Bug: f"text: {fn("arg")}" caused the grammar to terminate the f-string at
// the inner `"`, producing a bad parse error with wrong line attribution.
// =========================================================================

#[test]
fn fstring_nested_string_literal_in_interpolation() {
    // A string literal inside {} should be parsed as part of the expression,
    // not terminate the outer f-string.
    ShapeTest::new(
        r#"
        f"value: {"nested"}"
    "#,
    )
    .expect_parse_ok()
    .expect_string("value: nested");
}

#[test]
fn fstring_nested_string_in_function_call() {
    // The exact pattern from the error-handling playground example.
    ShapeTest::new(
        r#"
        let result = Err("oops")
        f"Err: {result}"
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn fstring_nested_string_as_call_arg() {
    // The nested-arg interpolation parses fine, but the program annotates the
    // parameter/return as `str`, which is not Shape's string type (`string`).
    // The strict checker rejects: the f-string body is `string`, which does not
    // unify with the declared `str`.
    ShapeTest::new(
        r#"
        fn greet(name: str) -> str { f"Hello, {name}!" }
        f"msg: {greet("world")}"
    "#,
    )
    .expect_run_err_contains("string is not compatible with str");
}

#[test]
fn fstring_nested_err_with_string_arg() {
    // Regression: this exact snippet produced a bad error and wrong line number.
    ShapeTest::new(
        r#"
        print(f"Err: {Err("oops")}")
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn cstring_nested_string_in_interpolation() {
    // c-strings share the same grammar fix — nested quotes should work there too.
    ShapeTest::new(
        r#"
        c"label: {"value"}"
    "#,
    )
    .expect_parse_ok();
}

// =====================================================================
// f-string infallible-cast interpolation (VM==JIT divergence fix)
// =====================================================================
//
// `f"{true as int}"` must render `1` (not `true`): the bool→int
// infallible cast restamps the interpolated value's kind so the
// formatter renders the int. Under --mode jit the f-string-interp path
// previously DROPPED the cast (the MIR lowering of `Expr::TypeAssertion`
// passed the operand bits through unchanged, leaving the SOURCE Bool
// kind), rendering `true` — a VM!=JIT divergence. Fixed by routing the
// primitive `as`-cast through `Rvalue::PrimitiveCast`, which the JIT MIR
// preflight rejects so the program deopts to the interpreter (where
// `OpCode::ConvertToInt` restamps the kind). VM output below is the
// shared canonical result for both modes.

#[test]
fn fstring_bool_true_as_int_renders_one() {
    ShapeTest::new(r#"print(f"{true as int}")"#)
        .expect_run_ok()
        .expect_output("1");
}

#[test]
fn fstring_bool_false_as_int_renders_zero() {
    ShapeTest::new(r#"print(f"{false as int}")"#)
        .expect_run_ok()
        .expect_output("0");
}

#[test]
fn bool_as_int_print_and_fstring_agree() {
    // The bare `print(true as int)` and the f-string-interp form must
    // produce the identical `1` (and `0` for false).
    let code = r#"print(true as int)
print(f"{true as int}")
print(false as int)
print(f"{false as int}")"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("1\n1\n0\n0");
}

#[test]
fn fstring_int_as_number_cast() {
    ShapeTest::new(r#"print(f"{5 as number}")"#)
        .expect_run_ok()
        .expect_output("5.0");
}

// =====================================================================
// Typed MIR FormatValue producer — ordinary source VM/JIT parity
// =====================================================================

#[test]
fn pure_expression_fstrings_materialize_canonical_strings() {
    let cases = [
        (
            r#"fn tag(value: int) -> string { f"{value}" }
tag(7)"#,
            "7",
        ),
        (
            r#"fn tag(value: bool) -> string { f"{value}" }
tag(true)"#,
            "true",
        ),
        (
            r#"fn tag(value: number) -> string { f"{value}" }
tag(1.0)"#,
            "1.0",
        ),
        (
            r#"fn tag(value: string) -> string { f"{value}" }
tag("shape")"#,
            "shape",
        ),
    ];

    for (source, expected) in cases {
        expect_vm_and_jit_string(source, expected);
    }
}

#[test]
fn adjacent_and_literal_expression_parts_match_in_vm_and_jit() {
    expect_vm_and_jit_string(
        r#"
        let left = 7
        let right = true
        f"{left}{right}"
        "#,
        "7true",
    );
    expect_vm_and_jit_string(
        r#"
        let value = 7
        f"value={value}"
        "#,
        "value=7",
    );
}

#[test]
fn fixed_spec_matches_in_vm_and_native_jit() {
    expect_vm_and_jit_string(
        r#"
        let value = 1.5
        f"{value:fixed(2)}"
        "#,
        "1.50",
    );
}

#[test]
fn table_spec_remains_an_explicit_vm_and_jit_rejection() {
    let source = r#"
        let value = 1
        f"{value:table()}"
    "#;
    ShapeTest::new(source).expect_run_err_contains("FORMAT_SPEC_TABLE rendering deferred");
    ShapeTest::new(source)
        .with_jit()
        .expect_run_err_contains("FORMAT_SPEC_TABLE rendering deferred");
}
