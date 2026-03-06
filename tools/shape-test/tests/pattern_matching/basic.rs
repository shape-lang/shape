//! Basic pattern matching tests.
//!
//! Tests cover:
//! - Basic match with guards
//! - Typed patterns with guards
//! - Type-based matching (union types)
//! - Enum matching
//! - Match as expression returning values
//! - Wildcard patterns
//! - Boolean matching
//! - Literal patterns

use shape_test::shape_test::ShapeTest;

// ============================================================================
// 1. Basic match with guards (book example)
// ============================================================================

#[test]
fn pm_01_basic_match_with_guards_negative() {
    let code = r#"
function sign(n) {
    return match n {
        x where x < 0 => "negative",
        x where x == 0 => "zero",
        _ => "positive"
    }
}
sign(-5)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_string("negative");
}

#[test]
fn pm_01_basic_match_with_guards_zero() {
    let code = r#"
function sign(n) {
    return match n {
        x where x < 0 => "negative",
        x where x == 0 => "zero",
        _ => "positive"
    }
}
sign(0)
"#;
    ShapeTest::new(code).expect_run_ok().expect_string("zero");
}

#[test]
fn pm_01_basic_match_with_guards_positive() {
    let code = r#"
function sign(n) {
    return match n {
        x where x < 0 => "negative",
        x where x == 0 => "zero",
        _ => "positive"
    }
}
sign(10)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_string("positive");
}

// ============================================================================
// 1b. Typed pattern with guards (book syntax: x: int where ...)
// ============================================================================

#[test]
fn pm_01b_typed_pattern_with_guard() {
    let code = r#"
function sign(n) {
    return match n {
        x: int where x < 0 => "negative",
        x: int where x == 0 => "zero",
        _ => "positive"
    }
}
sign(-5)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_string("negative");
}

// ============================================================================
// 2. Type-based matching (union types)
// ============================================================================

#[test]
fn pm_02_union_type_match_int() {
    let code = r#"
let val = 42 as int | string
match val {
    n: int => "got int",
    s: string => "got string"
}
"#;
    ShapeTest::new(code).expect_string("got int");
}

#[test]
fn pm_02_union_type_match_string() {
    let code = r#"
let val = "hello" as int | string
match val {
    n: int => "got int",
    s: string => "got string"
}
"#;
    ShapeTest::new(code).expect_string("got string");
}

// Workaround: Use guard-based type checking instead of typed patterns
#[test]
fn pm_02_workaround_guard_type_check() {
    let code = r#"
function normalize(value) {
    return match value {
        x where x > 0 or x <= 0 => "got number",
        _ => "got something else"
    }
}
normalize(42)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_string("got number");
}

// ============================================================================
// 3. Enum matching
// ============================================================================

#[test]
fn pm_03_enum_match_ok_variant() {
    let code = r#"
enum Status {
    Ok(int),
    Error(string)
}

function render(status: Status) -> string {
    match status {
        Status::Ok(code) => "ok",
        Status::Error(msg) => "error"
    }
}
render(Status::Ok(200))
"#;
    ShapeTest::new(code).expect_run_ok().expect_string("ok");
}

#[test]
fn pm_03_enum_match_error_variant() {
    let code = r#"
enum Status {
    Ok(int),
    Error(string)
}

function render(status: Status) -> string {
    match status {
        Status::Ok(code) => "ok",
        Status::Error(msg) => "error"
    }
}
render(Status::Error("not found"))
"#;
    ShapeTest::new(code).expect_run_ok().expect_string("error");
}

#[test]
fn pm_03_enum_match_extracts_payload() {
    let code = r#"
enum Status {
    Ok(int),
    Error(string)
}

function get_code(status: Status) -> int {
    match status {
        Status::Ok(code) => code,
        Status::Error(msg) => -1
    }
}
get_code(Status::Ok(200))
"#;
    ShapeTest::new(code).expect_run_ok().expect_number(200.0);
}

#[test]
fn pm_03_enum_match_extracts_string_payload() {
    let code = r#"
enum Status {
    Ok(int),
    Error(string)
}

function get_msg(status: Status) -> string {
    match status {
        Status::Ok(code) => "none",
        Status::Error(msg) => msg
    }
}
get_msg(Status::Error("not found"))
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_string("not found");
}

// ============================================================================
// 4. Match as expression returning values
// ============================================================================

#[test]
fn pm_04_match_expr_returns_int_low() {
    let code = r#"
function bucket(n) {
    return match n {
        x where x < 10 => 0,
        x where x < 100 => 1,
        _ => 2
    }
}
bucket(5)
"#;
    ShapeTest::new(code).expect_run_ok().expect_number(0.0);
}

#[test]
fn pm_04_match_expr_returns_int_mid() {
    let code = r#"
function bucket(n) {
    return match n {
        x where x < 10 => 0,
        x where x < 100 => 1,
        _ => 2
    }
}
bucket(50)
"#;
    ShapeTest::new(code).expect_run_ok().expect_number(1.0);
}

#[test]
fn pm_04_match_expr_returns_int_high() {
    let code = r#"
function bucket(n) {
    return match n {
        x where x < 10 => 0,
        x where x < 100 => 1,
        _ => 2
    }
}
bucket(500)
"#;
    ShapeTest::new(code).expect_run_ok().expect_number(2.0);
}

// ============================================================================
// 5. Wildcard pattern
// ============================================================================

#[test]
fn pm_05_wildcard_catches_unmatched() {
    let code = r#"
function check(n) {
    return match n {
        x where x == 1 => "one",
        x where x == 2 => "two",
        _ => "other"
    }
}
check(99)
"#;
    ShapeTest::new(code).expect_run_ok().expect_string("other");
}

#[test]
fn pm_05_wildcard_catches_negative() {
    let code = r#"
function check(n) {
    return match n {
        x where x == 1 => "one",
        x where x == 2 => "two",
        _ => "other"
    }
}
check(-5)
"#;
    ShapeTest::new(code).expect_run_ok().expect_string("other");
}

// ============================================================================
// 6. Matching on boolean values
// ============================================================================

#[test]
fn pm_06_match_bool_true() {
    let code = r#"
function describe(b) {
    return match b {
        x where x == true => "yes",
        _ => "no"
    }
}
describe(true)
"#;
    ShapeTest::new(code).expect_run_ok().expect_string("yes");
}

#[test]
fn pm_06_match_bool_false() {
    let code = r#"
function describe(b) {
    return match b {
        x where x == true => "yes",
        _ => "no"
    }
}
describe(false)
"#;
    ShapeTest::new(code).expect_run_ok().expect_string("no");
}

// ============================================================================
// 7. Literal patterns
// ============================================================================

#[test]
fn pm_07_literal_int_pattern() {
    let code = r#"
match 42 {
    42 => "found it",
    _ => "nope"
}
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_string("found it");
}

#[test]
fn pm_07_literal_int_pattern_miss() {
    let code = r#"
match 99 {
    42 => "found it",
    _ => "nope"
}
"#;
    ShapeTest::new(code).expect_run_ok().expect_string("nope");
}

#[test]
fn pm_07_literal_string_pattern() {
    let code = r#"
match "hello" {
    "hello" => "greeting",
    "bye" => "farewell",
    _ => "unknown"
}
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_string("greeting");
}

#[test]
fn pm_07_literal_string_pattern_second() {
    let code = r#"
match "bye" {
    "hello" => "greeting",
    "bye" => "farewell",
    _ => "unknown"
}
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_string("farewell");
}

#[test]
fn pm_07_literal_true_pattern() {
    let code = r#"
match true {
    true => "yes",
    false => "no"
}
"#;
    ShapeTest::new(code).expect_run_ok().expect_string("yes");
}

#[test]
fn pm_07_literal_false_pattern() {
    let code = r#"
match false {
    true => "yes",
    false => "no"
}
"#;
    ShapeTest::new(code).expect_run_ok().expect_string("no");
}
