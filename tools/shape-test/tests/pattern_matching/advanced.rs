//! Advanced pattern matching tests.
//!
//! Tests cover:
//! - Match returning consistent types
//! - Exhaustiveness checking
//! - Print output from match
//! - Top-level match
//! - Enum with mixed payload/no-payload variants
//! - Guard evaluation order
//! - Numeric edge cases
//! - Identifier patterns
//! - Multiple enums
//! - Enum match returning numbers
//! - Parse-only syntax tests
//! - Parenthesized guard expressions

use shape_test::shape_test::ShapeTest;

// ============================================================================
// 23. Match returning consistent type from all arms
// ============================================================================

#[test]
fn pm_23_match_returning_number_from_all_arms() {
    let code = r#"
function pick(n) {
    return match n {
        x where x > 0 => 1,
        x where x < 0 => -1,
        _ => 0
    }
}
pick(42)
"#;
    ShapeTest::new(code).expect_run_ok().expect_number(1.0);
}

// ============================================================================
// 24. Exhaustiveness checking
// ============================================================================

// BUG FINDING: Non-exhaustive enum match does NOT fail at compile time
// The exhaustiveness checker is present in the compiler (check_match_exhaustiveness),
// but as documented in the code: "If type inference fails (e.g., undefined variable),
// skip exhaustiveness checking." The type inference engine needs full program context
// to track variable types through function parameters. When the scrutinee type cannot
// be determined by type inference, exhaustiveness checking is silently skipped.
// This means non-exhaustive matches only fail at RUNTIME (when no arm matches),
// not at compile time as the documentation implies.

#[test]
fn pm_24_non_exhaustive_enum_match_runs_when_matched() {
    // Even with missing Blue variant, Red still matches the first arm
    let code = r#"
enum Color { Red, Green, Blue }

function name(c: Color) -> string {
    match c {
        Color::Red => "red",
        Color::Green => "green"
    }
}
name(Color::Red)
"#;
    // This currently succeeds because Red matches, but Blue would fail at runtime
    ShapeTest::new(code).expect_run_ok().expect_string("red");
}

#[test]
fn pm_24_non_exhaustive_enum_match_fails_at_runtime() {
    // When the unmatched variant is actually used, runtime error occurs
    let code = r#"
enum Color { Red, Green, Blue }

function name(c: Color) -> string {
    match c {
        Color::Red => "red",
        Color::Green => "green"
    }
}
name(Color::Blue)
"#;
    ShapeTest::new(code).expect_run_err_contains("No match arm matched the value");
}

// ============================================================================
// 25. Print output from match
// ============================================================================

#[test]
fn pm_25_match_output_via_print() {
    let code = r#"
function sign(n) {
    return match n {
        x where x < 0 => "negative",
        x where x == 0 => "zero",
        _ => "positive"
    }
}
print(sign(-5))
print(sign(0))
print(sign(10))
"#;
    ShapeTest::new(code).expect_output("negative\nzero\npositive");
}

// ============================================================================
// 26. Match at top level (not inside function)
// ============================================================================

#[test]
fn pm_26_top_level_match() {
    let code = r#"
let x = 42
match x {
    n where n > 0 => "positive",
    _ => "non-positive"
}
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_string("positive");
}

// ============================================================================
// 27. Enum with mixed payload/no-payload variants
// ============================================================================

#[test]
fn pm_27_enum_mixed_variants() {
    let code = r#"
enum Shape {
    Circle(int),
    Square(int),
    Unknown
}

function area_label(s: Shape) -> string {
    match s {
        Shape::Circle(r) => "circle",
        Shape::Square(side) => "square",
        Shape::Unknown => "unknown"
    }
}
area_label(Shape::Unknown)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_string("unknown");
}

#[test]
fn pm_27_enum_mixed_circle() {
    let code = r#"
enum Shape {
    Circle(int),
    Square(int),
    Unknown
}

function area_label(s: Shape) -> string {
    match s {
        Shape::Circle(r) => "circle",
        Shape::Square(side) => "square",
        Shape::Unknown => "unknown"
    }
}
area_label(Shape::Circle(5))
"#;
    ShapeTest::new(code).expect_run_ok().expect_string("circle");
}

// ============================================================================
// 28. Guard evaluation order: guards evaluated top to bottom
// ============================================================================

#[test]
fn pm_28_guard_eval_order_medium() {
    let code = r#"
function classify(x) {
    return match x {
        n where n > 100 => "huge",
        n where n > 50 => "large",
        n where n > 25 => "medium",
        _ => "small"
    }
}
classify(30)
"#;
    ShapeTest::new(code).expect_run_ok().expect_string("medium");
}

// ============================================================================
// 29. Match with numeric edge cases
// ============================================================================

#[test]
fn pm_29_match_zero_literal() {
    let code = r#"
match 0 {
    0 => "zero",
    _ => "not zero"
}
"#;
    ShapeTest::new(code).expect_run_ok().expect_string("zero");
}

#[test]
fn pm_29_match_negative_guard() {
    let code = r#"
function check(n) {
    return match n {
        x where x == -1 => "negative one",
        _ => "other"
    }
}
check(-1)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_string("negative one");
}

// ============================================================================
// 30. Identifier pattern (bare name, no guard) acts as catch-all
// ============================================================================

#[test]
fn pm_30_identifier_pattern_catches_all() {
    let code = r#"
function echo(n) {
    return match n {
        x => x
    }
}
echo(42)
"#;
    ShapeTest::new(code).expect_run_ok().expect_number(42.0);
}

#[test]
fn pm_30_identifier_pattern_with_other_arms() {
    let code = r#"
function process(n) {
    return match n {
        x where x > 100 => "big",
        x => "default"
    }
}
process(5)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_string("default");
}

// ============================================================================
// 31. Multiple enums in same program
// ============================================================================

#[test]
fn pm_31_multiple_enums() {
    let code = r#"
enum Color { Red, Green, Blue }
enum Size { Small, Medium, Large }

function describe_color(c: Color) -> string {
    match c {
        Color::Red => "red",
        Color::Green => "green",
        Color::Blue => "blue"
    }
}

function describe_size(s: Size) -> string {
    match s {
        Size::Small => "small",
        Size::Medium => "medium",
        Size::Large => "large"
    }
}

describe_color(Color::Green)
"#;
    ShapeTest::new(code).expect_run_ok().expect_string("green");
}

// ============================================================================
// 32. Enum match returning number
// ============================================================================

#[test]
fn pm_32_enum_match_returns_number() {
    let code = r#"
enum Priority { Low, Medium, High }

function urgency(p: Priority) -> int {
    match p {
        Priority::Low => 1,
        Priority::Medium => 5,
        Priority::High => 10
    }
}
urgency(Priority::High)
"#;
    ShapeTest::new(code).expect_run_ok().expect_number(10.0);
}

// ============================================================================
// 33. Parse-only tests: verify syntax is accepted
// ============================================================================

#[test]
fn pm_33_parse_match_with_fn_keyword() {
    let code = r#"
fn sign(n: int) -> string {
    match n {
        x: int where x < 0 => "negative"
        x: int where x == 0 => "zero"
        _ => "positive"
    }
}
"#;
    ShapeTest::new(code).expect_parse_ok();
}

#[test]
fn pm_33_parse_match_no_commas() {
    let code = r#"
match 42 {
    x where x > 0 => "positive"
    _ => "non-positive"
}
"#;
    ShapeTest::new(code).expect_parse_ok();
}

#[test]
fn pm_33_parse_match_with_commas() {
    let code = r#"
match 42 {
    x where x > 0 => "positive",
    _ => "non-positive",
}
"#;
    ShapeTest::new(code).expect_parse_ok();
}

// ============================================================================
// 34. Guard with parenthesized expressions
// ============================================================================

#[test]
fn pm_34_guard_with_parens() {
    let code = r#"
function check(x) {
    return match x {
        n where (n % 2 == 0) and (n > 10) => "even and large",
        n where n % 2 == 0 => "even",
        _ => "odd"
    }
}
check(20)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_string("even and large");
}

#[test]
fn pm_34_guard_with_parens_just_even() {
    let code = r#"
function check(x) {
    return match x {
        n where (n % 2 == 0) and (n > 10) => "even and large",
        n where n % 2 == 0 => "even",
        _ => "odd"
    }
}
check(4)
"#;
    ShapeTest::new(code).expect_run_ok().expect_string("even");
}

#[test]
fn pm_34_guard_with_parens_odd() {
    let code = r#"
function check(x) {
    return match x {
        n where (n % 2 == 0) and (n > 10) => "even and large",
        n where n % 2 == 0 => "even",
        _ => "odd"
    }
}
check(7)
"#;
    ShapeTest::new(code).expect_run_ok().expect_string("odd");
}
