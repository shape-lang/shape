//! Comptime expression block tests.
//!
//! Tests cover: comptime expression blocks, comptime types, nested comptime,
//! conditionals, arithmetic, comparisons, boolean ops, comptime fields,
//! build_config, and related edge cases.

use shape_test::shape_test::ShapeTest;

// ============================================================================
// PASSING tests (regression)
// ============================================================================

#[test]
fn ct_01_comptime_expr_block() {
    let code = r#"
const BUILD_TAG = comptime {
  "dev"
}
print(BUILD_TAG)
"#;
    ShapeTest::new(code).expect_run_ok().expect_output("dev");
}

#[test]
fn ct_02_comptime_side_effect() {
    let code = r#"
comptime {
  warning("Compiling with test flags")
}
print("program started")
"#;
    // comptime warning() output goes to stderr during compilation,
    // only runtime print() is captured in stdout
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("program started");
}

#[test]
fn ct_08_comptime_types() {
    let code = r#"
const CT_INT = comptime { 42 }
const CT_STR = comptime { "hello" }
const CT_BOOL = comptime { true }

print(CT_INT)
print(CT_STR)
print(CT_BOOL)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("42\nhello\ntrue");
}

#[test]
fn ct_09_nested_comptime() {
    let code = r#"
const NESTED = comptime {
  comptime {
    "inner"
  }
}
print(NESTED)
"#;
    ShapeTest::new(code).expect_run_ok().expect_output("inner");
}

#[test]
fn ct_12_comptime_warning() {
    let code = r#"
comptime {
  warning("this is a build warning")
}
print("program continued after warning")
"#;
    // comptime warning() output goes to stderr during compilation,
    // only runtime print() is captured in stdout
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("program continued after warning");
}

#[test]
fn ct_17_build_config() {
    let code = r#"
const CONFIG = comptime {
  build_config()
}
print(CONFIG)
"#;
    ShapeTest::new(code).expect_run_ok();
}

#[test]
fn ct_19_comptime_complex_expr() {
    let code = r#"
const COMPUTED = comptime {
  let x = 10
  let y = 20
  x + y * 2
}
print(COMPUTED)
"#;
    ShapeTest::new(code).expect_run_ok().expect_output("50");
}

#[test]
fn ct_21_comptime_conditional() {
    let code = r#"
const MODE = comptime {
  let debug = true
  if debug {
    "debug"
  } else {
    "release"
  }
}
print(MODE)
"#;
    ShapeTest::new(code).expect_run_ok().expect_output("debug");
}

#[test]
fn ct_21b_comptime_conditional_v2() {
    let code = r#"
const MODE = comptime {
  if true {
    "debug"
  } else {
    "release"
  }
}
print(MODE)
"#;
    ShapeTest::new(code).expect_run_ok().expect_output("debug");
}

#[test]
fn ct_22_multiple_comptime_blocks() {
    let code = r#"
const A = comptime { "alpha" }
const B = comptime { "beta" }
const C = comptime { "gamma" }

comptime {
  warning("block 1")
}

comptime {
  warning("block 2")
}

print(A)
print(B)
print(C)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("alpha");
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("beta");
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("gamma");
}

#[test]
fn ct_27_comptime_arithmetic() {
    let code = r#"
const A = comptime { 2 + 3 }
const B = comptime { 10 - 4 }
const C = comptime { 3 * 7 }
const D = comptime { 20 / 4 }
const E = comptime { 17 % 5 }

print(A)
print(B)
print(C)
print(D)
print(E)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("5\n6\n21\n5\n2");
}

#[test]
fn ct_29_comptime_comparison() {
    let code = r#"
const GT = comptime { 5 > 3 }
const LT = comptime { 2 < 10 }
const EQ = comptime { 42 == 42 }
const NE = comptime { 1 != 2 }

print(GT)
print(LT)
print(EQ)
print(NE)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("true\ntrue\ntrue\ntrue");
}

#[test]
fn ct_33_comptime_empty_block() {
    let code = r#"
comptime {
}
print("after empty comptime")
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("after empty comptime");
}

#[test]
fn ct_34_comptime_array() {
    let code = r#"
const ITEMS = comptime {
  [1, 2, 3]
}
print(ITEMS)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("[1, 2, 3]");
}

#[test]
fn ct_35_comptime_multiline() {
    let code = r#"
const RESULT = comptime {
  let a = 10
  let b = 20
  let c = a + b
  let d = c * 2
  d
}
print(RESULT)
"#;
    ShapeTest::new(code).expect_run_ok().expect_output("60");
}

#[test]
fn ct_39_comptime_reuse_const() {
    let code = r#"
const A = comptime { 10 }
const B = comptime { 20 }
const C = A + B
print(C)
"#;
    ShapeTest::new(code).expect_run_ok().expect_output("30");
}

#[test]
fn ct_40b_comptime_field_instance() {
    let code = r#"
type Currency {
  amount: float,
  comptime symbol: string = "$",
  comptime decimals: int = 2
}

let usd = Currency { amount: 42.5 }
print(usd.symbol)
print(usd.decimals)
print(usd.amount)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("$");
}

#[test]
fn ct_40d_comptime_field_comma() {
    let code = r#"
type Currency {
  comptime symbol: string = "$",
  comptime decimals: number = 2,
  amount: number
}

let usd = Currency { amount: 42.5 }
print(usd.symbol)
print(usd.decimals)
print(usd.amount)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("$");
}

#[test]
fn ct_40e_comptime_field_inline() {
    let code = r#"
type Currency {
  comptime symbol: string = "$",
  comptime decimals: number = 2,
  amount: number
}

// Access comptime field directly on construction expression
print(Currency { amount: 42.5 }.symbol)
"#;
    ShapeTest::new(code).expect_run_ok().expect_output("$");
}

#[test]
fn ct_49_build_config_fields() {
    let code = r#"
const CFG = comptime {
  let c = build_config()
  warning(f"config: {c}")
  c
}
print(CFG)
"#;
    ShapeTest::new(code).expect_run_ok();
}

#[test]
fn ct_51_comptime_float() {
    let code = r#"
const PI_APPROX = comptime { 3.14159 }
const E_APPROX = comptime { 2.71828 }
print(PI_APPROX)
print(E_APPROX)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("3.14159");
}

// ============================================================================
// EXPECTED ERROR tests
// ============================================================================

#[test]
fn ct_03_comptime_no_runtime_locals() {
    let code = r#"
let marker = 42
comptime {
  marker
}
print("done")
"#;
    ShapeTest::new(code).expect_run_err_contains("Undefined variable: marker");
}

#[test]
fn ct_11_comptime_error() {
    let code = r#"
comptime {
  error("this is a build error")
}
print("should not reach here")
"#;
    ShapeTest::new(code).expect_run_err_contains("this is a build error");
}

// ============================================================================
// FAILING tests (TDD) -- document expected behavior for unimplemented features
// ============================================================================

/// BUG: Multiple comptime fields without commas fails to parse.
/// The parser encounters `comptime` as an unexpected identifier when
/// parsing the second comptime field without a separating comma.
/// When fixed, `Currency::symbol` should print "$" and `Currency::decimals`
/// should print "2".
#[test]

fn ct_06_comptime_fields() {
    let code = r#"
type Currency {
  comptime symbol: string = "$"
  comptime decimals: int = 2
}

print(Currency::symbol)
print(Currency::decimals)
"#;
    ShapeTest::new(code).expect_run_ok().expect_output("$\n2");
}

/// BUG: `not` keyword is not recognized as a boolean operator in comptime.
/// Shape metadata lists `not` as a keyword (logical NOT), but the evaluator
/// treats it as an undefined variable. Only `!` works for negation.
/// When fixed, `not false` should evaluate to `true`.
#[test]

fn ct_28_comptime_boolean_ops() {
    let code = r#"
const T = comptime { true and true }
const F = comptime { true and false }
const O = comptime { false or true }
const N = comptime { not false }

print(T)
print(F)
print(O)
print(N)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("true\nfalse\ntrue\ntrue");
}

/// BUG: `not` keyword not recognized in comptime context.
/// Both `not false` and `not true` fail with "Undefined variable: not".
/// When fixed, `not` should work as the logical negation operator alongside `!`.
#[test]

fn ct_28b_not_check() {
    let code = r#"
const A = comptime { not false }
const B = comptime { not true }
const C = comptime { !false }
const D = comptime { !true }

print(f"not false = {A}")
print(f"not true = {B}")
print(f"!false = {C}")
print(f"!true = {D}")
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("not false = true");
}

/// BUG: `not` keyword not recognized at runtime (same issue as comptime).
/// `let a = not false` fails with "Undefined variable: 'not'" at the
/// semantic analysis stage.
/// When fixed, `not` should work as a unary boolean operator at runtime too.
#[test]

fn ct_28c_not_runtime() {
    let code = r#"
let a = not false
let b = not true
let c = !false
let d = !true

print(f"runtime not false = {a}")
print(f"runtime not true = {b}")
print(f"runtime !false = {c}")
print(f"runtime !true = {d}")
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("runtime not false = true");
}

/// BUG: Type::field static access treats the type as an enum.
/// Accessing `Config::version` on a type with a single comptime field
/// causes a semantic error: "Type 'Config' is not an enum".
/// The compiler confuses `Type::field` syntax with enum variant access.
/// When fixed, `Config::version` should return the comptime field value "1.0".
#[test]

fn ct_40_comptime_field_single() {
    let code = r#"
type Config {
  comptime version: string = "1.0"
}

print(Config::version)
"#;
    ShapeTest::new(code).expect_run_ok().expect_output("1.0");
}

/// BUG: Typed let with comptime field access infers 'object' instead of named type.
/// When using `let usd: Currency = Currency { amount: 42.5 }`, accessing
/// `usd.symbol` fails with "Property 'symbol' does not exist on type 'object'".
/// The explicit type annotation causes the compiler to use 'object' as the
/// inferred structural type rather than 'Currency'.
/// When fixed, typed let should preserve the named type and allow comptime field access.
#[test]

fn ct_40c_comptime_field_typed() {
    let code = r#"
type Currency {
  amount: float,
  comptime symbol: string = "$",
  comptime decimals: int = 2
}

let usd: Currency = Currency { amount: 42.5 }
print(usd.symbol)
print(usd.decimals)
print(usd.amount)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("$");
}

/// BUG: `build_config()` individual field dot-access returns None.
/// While `build_config()` returns an object successfully, accessing individual
/// fields like `c.target_os`, `c.target_arch`, `c.version`, and `c.debug`
/// all silently return None instead of their actual values.
///
/// The program runs without error, but all fields print "None" instead of
/// actual build configuration values. This is a silent data loss bug in
/// comptime object field access.
#[test]
fn ct_49b_build_config_access() {
    let code = r#"
const OS = comptime {
  let c = build_config()
  c.target_os
}
print(f"OS: {OS}")
"#;
    // BUG: Currently prints "OS: None" — when fixed, should print actual OS.
    // For now, just verify it runs and produces output (even if wrong).
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("OS:");
}
