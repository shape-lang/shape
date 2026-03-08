//! Destructuring pattern tests.
//!
//! Tests cover:
//! - Empty enum variants (no data)
//! - Result/Option matching
//! - Array patterns
//! - Function calls in guards
//! - Multiple function calls in guards
//! - Enum matching with wildcard fallthrough
//! - Match used in let binding

use shape_test::shape_test::ShapeTest;

// ============================================================================
// 15. Empty enum variants (no data)
// ============================================================================

#[test]
fn pm_15_empty_enum_variant_match() {
    let code = r#"
enum Color { Red, Green, Blue }

function name(c: Color) -> string {
    match c {
        Color::Red => "red",
        Color::Green => "green",
        Color::Blue => "blue"
    }
}
name(Color::Red)
"#;
    ShapeTest::new(code).expect_run_ok().expect_string("red");
}

#[test]
fn pm_15_empty_enum_variant_match_green() {
    let code = r#"
enum Color { Red, Green, Blue }

function name(c: Color) -> string {
    match c {
        Color::Red => "red",
        Color::Green => "green",
        Color::Blue => "blue"
    }
}
name(Color::Green)
"#;
    ShapeTest::new(code).expect_run_ok().expect_string("green");
}

#[test]
fn pm_15_empty_enum_variant_match_blue() {
    let code = r#"
enum Color { Red, Green, Blue }

function name(c: Color) -> string {
    match c {
        Color::Red => "red",
        Color::Green => "green",
        Color::Blue => "blue"
    }
}
name(Color::Blue)
"#;
    ShapeTest::new(code).expect_run_ok().expect_string("blue");
}

// ============================================================================
// 16. Match on Result types (Ok/Err)
// ============================================================================

#[test]
fn pm_16_result_ok_match() {
    let code = r#"
function handle(r) {
    return match r {
        Ok(val) => val,
        Err(e) => -1
    }
}
handle(Ok(42))
"#;
    ShapeTest::new(code).expect_run_ok().expect_number(42.0);
}

#[test]
fn pm_16_result_err_match() {
    let code = r#"
function handle(r) {
    return match r {
        Ok(val) => val,
        Err(e) => -1
    }
}
handle(Err("fail"))
"#;
    ShapeTest::new(code).expect_run_ok().expect_number(-1.0);
}

// ============================================================================
// 17. Match on Option types (Some/None)
// ============================================================================

// BUG FINDING: Option::None in match pattern fails to parse
// Error: "expected something else, found `}`"
// Root cause: `None` is a none_literal in the grammar, and Option::None
// in pattern_qualified_constructor expects ident :: ident, but "None" as
// a keyword may conflict with none_literal parsing precedence.
// Also, Option::Some(val) fails similarly because the parser may not
// handle Option::Some as a qualified constructor correctly in this context.

#[test]
fn pm_17_option_some_match_parse_check() {
    // Option::Some(val) pattern now parses successfully.
    let code = r#"
function unwrap_or(opt, default_val) {
    return match opt {
        Option::Some(val) => val,
        Option::None => default_val
    }
}
"#;
    ShapeTest::new(code).expect_parse_ok();
}

// Regression: Unqualified Some(val) pattern now correctly unwraps the value.
// Previously failed with "UnwrapOption can only be applied to Option (Some/None), got int".
#[test]
fn pm_17_option_some_unqualified_match() {
    let code = r#"
function unwrap_or(opt, default_val) {
    return match opt {
        Some(val) => val,
        _ => default_val
    }
}
unwrap_or(Some(10), 0)
"#;
    ShapeTest::new(code).expect_run_ok().expect_number(10.0);
}

#[test]
fn pm_17_option_none_wildcard_match() {
    // None falls through to wildcard, which works fine
    let code = r#"
function unwrap_or(opt, default_val) {
    return match opt {
        Some(val) => val,
        _ => default_val
    }
}
unwrap_or(None, 0)
"#;
    ShapeTest::new(code).expect_run_ok().expect_number(0.0);
}

// Workaround: Use guard-based null checking instead of Option pattern
#[test]
fn pm_17_workaround_null_check() {
    let code = r#"
function unwrap_or(opt, default_val) {
    return match opt {
        x where x != None => x,
        _ => default_val
    }
}
unwrap_or(42, 0)
"#;
    ShapeTest::new(code).expect_run_ok().expect_number(42.0);
}

#[test]
fn pm_17_workaround_null_check_none() {
    let code = r#"
function unwrap_or(opt, default_val) {
    return match opt {
        x where x != None => x,
        _ => default_val
    }
}
unwrap_or(None, 0)
"#;
    ShapeTest::new(code).expect_run_ok().expect_number(0.0);
}

// ============================================================================
// 18. Array patterns
// ============================================================================

// Regression: Array patterns in match now parse correctly.
// Previously failed with "expected something else, found `}`".
#[test]
fn pm_18_array_pattern_parses() {
    let code = r#"
function head(arr) {
    return match arr {
        [first, second] where first > second => "descending",
        [first, second] where first < second => "ascending",
        [first, second] => "equal",
        _ => "not a pair"
    }
}
"#;
    ShapeTest::new(code).expect_parse_ok();
}

// Workaround: Use identifier patterns with guard-based checks
#[test]
fn pm_18_workaround_array_element_access() {
    let code = r#"
function classify_pair(arr) {
    return match arr {
        a where a[0] > a[1] => "descending",
        a where a[0] < a[1] => "ascending",
        _ => "equal"
    }
}
classify_pair([10, 5])
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_string("descending");
}

#[test]
fn pm_18_workaround_array_ascending() {
    let code = r#"
function classify_pair(arr) {
    return match arr {
        a where a[0] > a[1] => "descending",
        a where a[0] < a[1] => "ascending",
        _ => "equal"
    }
}
classify_pair([3, 10])
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_string("ascending");
}

// ============================================================================
// 19. Match with function calls in guards
// ============================================================================

#[test]
fn pm_19_function_call_in_guard() {
    let code = r#"
function is_even(n) {
    return n % 2 == 0
}

function classify(x) {
    return match x {
        n where is_even(n) => "even",
        _ => "odd"
    }
}
classify(8)
"#;
    ShapeTest::new(code).expect_run_ok().expect_string("even");
}

#[test]
fn pm_19_function_call_in_guard_odd() {
    let code = r#"
function is_even(n) {
    return n % 2 == 0
}

function classify(x) {
    return match x {
        n where is_even(n) => "even",
        _ => "odd"
    }
}
classify(7)
"#;
    ShapeTest::new(code).expect_run_ok().expect_string("odd");
}

// ============================================================================
// 20. Match with multiple function calls in guard
// ============================================================================

#[test]
fn pm_20_multiple_function_calls_in_guard() {
    let code = r#"
function is_positive(n) { return n > 0 }
function is_small(n) { return n < 100 }

function classify(x) {
    return match x {
        n where is_positive(n) and is_small(n) => "small positive",
        n where is_positive(n) => "large positive",
        _ => "non-positive"
    }
}
classify(50)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_string("small positive");
}

#[test]
fn pm_20_multiple_function_calls_large() {
    let code = r#"
function is_positive(n) { return n > 0 }
function is_small(n) { return n < 100 }

function classify(x) {
    return match x {
        n where is_positive(n) and is_small(n) => "small positive",
        n where is_positive(n) => "large positive",
        _ => "non-positive"
    }
}
classify(500)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_string("large positive");
}

// ============================================================================
// 21. Enum matching with wildcard fallthrough
// ============================================================================

#[test]
fn pm_21_enum_match_with_wildcard() {
    let code = r#"
enum Color { Red, Green, Blue }

function is_red(c: Color) -> string {
    match c {
        Color::Red => "yes",
        _ => "no"
    }
}
is_red(Color::Blue)
"#;
    ShapeTest::new(code).expect_run_ok().expect_string("no");
}

// ============================================================================
// 22. Match used in let binding
// ============================================================================

#[test]
fn pm_22_match_in_let_binding() {
    let code = r#"
let x = 42
let label = match x {
    n where n > 0 => "positive",
    _ => "non-positive"
}
label
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_string("positive");
}
