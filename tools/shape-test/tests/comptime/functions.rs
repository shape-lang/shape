//! Comptime function tests.
//!
//! Tests cover: comptime fn definitions, comptime fn chaining,
//! string operations, recursive comptime fns, multiple/no params,
//! and trait-evidence checks (`find_impl`).

use shape_test::shape_test::ShapeTest;

// ============================================================================
// PASSING tests (regression)
// ============================================================================

#[test]
fn ct_04_comptime_fn_helpers() {
    let code = r#"
comptime fn make_greeting(name: string) {
  f"Hello {name}"
}

let GREETING: string = comptime {
  make_greeting("Shape")
}
print(GREETING)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("Hello Shape");
}

#[test]
fn ct_10_comptime_fn_chain() {
    let code = r#"
comptime fn upper(s: string) {
  f"{s}"
}

comptime fn make_loud(name: string) {
  upper(f"HELLO {name}")
}

let LOUD: string = comptime {
  make_loud("WORLD")
}
print(LOUD)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("HELLO WORLD");
}

#[test]
fn ct_10b_comptime_fn_chain_fix() {
    let code = r#"
comptime fn upper(s: string) {
  f"{s}"
}

let LOUD: string = comptime {
  upper(f"HELLO WORLD")
}
print(LOUD)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("HELLO WORLD");
}

#[test]
fn ct_24_comptime_string_ops() {
    let code = r#"
comptime fn tag(prefix: string, suffix: string) {
  f"{prefix}_{suffix}"
}

let TAG: string = comptime {
  tag("v1", "release")
}
print(TAG)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("v1_release");
}

#[test]
fn ct_26_comptime_fn_to_fn() {
    let code = r#"
comptime fn prefix(s: string) {
  f"[PREFIX] {s}"
}

comptime fn greet(name: string) {
  f"Hello {name}"
}

let MSG: string = comptime {
  let g = greet("World")
  prefix(g)
}
print(MSG)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("[PREFIX] Hello World");
}

#[test]
fn ct_37_comptime_fn_multiple_params() {
    let code = r#"
comptime fn format_pair(key: string, val: int) {
  f"{key} = {val}"
}

let PAIR: string = comptime {
  format_pair("count", 42)
}
print(PAIR)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("count = 42");
}

#[test]
fn ct_38_comptime_fn_no_params() {
    let code = r#"
comptime fn version() {
  "1.0.0"
}

let VER: string = comptime {
  version()
}
print(VER)
"#;
    ShapeTest::new(code).expect_run_ok().expect_output("1.0.0");
}

#[test]
fn ct_48_comptime_fn_recursive() {
    let code = r#"
comptime fn factorial(n: int) {
  if n <= 1 {
    1
  } else {
    n * factorial(n - 1)
  }
}

let FACT5: int = comptime {
  factorial(5)
}
print(FACT5)
"#;
    ShapeTest::new(code).expect_run_ok().expect_output("120");
}

// ============================================================================
// EXPECTED ERROR tests
// ============================================================================

#[test]
fn ct_20_comptime_fn_no_call() {
    let code = r#"
comptime fn secret() {
  "compile-time-only"
}

let x = secret()
print(x)
"#;
    ShapeTest::new(code).expect_run_err_contains("comptime");
}

#[test]
fn ct_52_comptime_fn_in_comptime_fn() {
    let code = r#"
fn normal_add(a: int, b: int) -> int {
  a + b
}

let SUM: int = comptime {
  normal_add(10, 20)
}
print(SUM)
"#;
    ShapeTest::new(code).expect_run_err_contains("Undefined function: 'normal_add'");
}

// ============================================================================
// FAILING tests (TDD) -- document expected behavior for unimplemented features
// ============================================================================

/// Comptime trait-evidence check via the typed `find_impl(type_ref(T),
/// trait_ref(Tr))` successor (the legacy `implements(...)` builtin was deleted
/// in ADR-009 E5 #21): an implemented `(type, trait)` pair resolves to
/// `Some(_)` at compile time.
#[test]
fn ct_18_find_impl_check() {
    let code = r#"
type Dog {
  name: string
}

trait Speak {
  method speak() -> string;
}

impl Speak for Dog {
  method speak() -> string {
    f"{self.name} says woof"
  }
}

let DOG_SPEAKS: bool = comptime {
  match find_impl(type_ref(Dog), trait_ref(Speak)) {
    Some(_) => true
    None => false
  }
}
print(DOG_SPEAKS)
"#;
    ShapeTest::new(code).expect_run_ok().expect_output("true");
}
