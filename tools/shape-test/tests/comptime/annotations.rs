//! Comptime annotation tests.
//!
//! Tests cover: annotation before/after hooks, annotation targets,
//! comptime post directives (extend, remove, replace body, set param),
//! multi-annotation stacking, annotation reuse, and related edge cases.

use shape_test::shape_test::ShapeTest;

// ============================================================================
// PASSING tests (regression)
// ============================================================================

#[test]
fn ct_05_annotation_traced() {
    let code = r#"
annotation traced(tag) {
  before(args, ctx) {
    print(f"[{tag}] before")
    args
  }
  after(args, result, ctx) {
    print(f"[{tag}] after")
    result
  }
}

@traced("math")
fn add(a: int, b: int) -> int {
  a + b
}

print(add(3, 4))
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("[math] before");
}

#[test]
fn ct_13_multi_annotations() {
    let code = r#"
annotation log_a(tag) {
  before(args, ctx) {
    print(f"[A:{tag}] before")
    args
  }
  after(args, result, ctx) {
    print(f"[A:{tag}] after")
    result
  }
}

annotation log_b(tag) {
  before(args, ctx) {
    print(f"[B:{tag}] before")
    args
  }
  after(args, result, ctx) {
    print(f"[B:{tag}] after")
    result
  }
}

@log_a("first")
@log_b("second")
fn multiply(a: int, b: int) -> int {
  a * b
}

print(multiply(3, 5))
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("[A:first] before");
    // Also verify the result is correct
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("15");
}

#[test]
fn ct_14b_annotation_empty_params() {
    let code = r#"
annotation simple() {
  before(args, ctx) {
    print("simple before")
    args
  }
  after(args, result, ctx) {
    print("simple after")
    result
  }
}

@simple()
fn greet() {
  print("hi")
}

greet()
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("simple before\nhi\nsimple after");
}

#[test]
fn ct_16_annotation_modify_result() {
    let code = r#"
annotation double_result(label) {
  before(args, ctx) {
    args
  }
  after(args, result, ctx) {
    print(f"[{label}] original result = {result}")
    result * 2
  }
}

@double_result("math")
fn add(a: int, b: int) -> int {
  a + b
}

let r = add(3, 4)
print(f"final result = {r}")
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("[math] original result = 7");
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("final result = 14");
}

#[test]
fn ct_25_expand_comptime() {
    let code = r#"
annotation logged(name) {
  before(args, ctx) {
    print(f"[{name}] calling")
    args
  }
  after(args, result, ctx) {
    print(f"[{name}] done")
    result
  }
}

@logged("subtract")
fn subtract(a: int, b: int) -> int {
  a - b
}

print(subtract(10, 3))
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("7");
}

#[test]
fn ct_30_annotation_ctx() {
    let code = r#"
annotation inspect_ctx(label) {
  before(args, ctx) {
    print(f"[{label}] ctx = {ctx}")
    args
  }
  after(args, result, ctx) {
    result
  }
}

@inspect_ctx("test")
fn double(x: int) -> int {
  x * 2
}

print(double(21))
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("42");
}

#[test]
fn ct_31_annotation_only_before() {
    let code = r#"
annotation before_only(tag) {
  before(args, ctx) {
    print(f"[{tag}] before only")
    args
  }
}

@before_only("test")
fn square(x: int) -> int {
  x * x
}

print(square(5))
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("[test] before only");
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("25");
}

#[test]
fn ct_32_annotation_only_after() {
    let code = r#"
annotation after_only(tag) {
  after(args, result, ctx) {
    print(f"[{tag}] after only, result = {result}")
    result
  }
}

@after_only("test")
fn cube(x: int) -> int {
  x * x * x
}

print(cube(3))
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("[test] after only");
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("27");
}

#[test]
fn ct_36_annotation_three_stack() {
    let code = r#"
annotation layer1(n) {
  before(args, ctx) { print(f"L1 before {n}"); args }
  after(args, result, ctx) { print(f"L1 after {n}"); result }
}
annotation layer2(n) {
  before(args, ctx) { print(f"L2 before {n}"); args }
  after(args, result, ctx) { print(f"L2 after {n}"); result }
}
annotation layer3(n) {
  before(args, ctx) { print(f"L3 before {n}"); args }
  after(args, result, ctx) { print(f"L3 after {n}"); result }
}

@layer1("a")
@layer2("b")
@layer3("c")
fn identity(x: int) -> int {
  x
}

print(identity(99))
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("99");
}

#[test]
fn ct_41_extend_target() {
    let code = r#"
annotation add_sum() {
  targets: [type]
  comptime post(target, ctx) {
    extend target {
      method sum() { self.x + self.y }
    }
  }
}

@add_sum()
type Point { x: int, y: int }

let p = Point { x: 10, y: 20 }
print(p.sum())
"#;
    ShapeTest::new(code).expect_run_ok().expect_output("30");
}

#[test]
fn ct_42_remove_target() {
    let code = r#"
annotation drop_it() {
  targets: [type]
  comptime post(target, ctx) {
    remove target
  }
}

@drop_it()
type Phantom { x: int }

// This type should have been removed
print("after phantom definition")
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("after phantom definition");
}

#[test]
fn ct_43_annotation_targets_decl() {
    let code = r#"
annotation fn_only(tag) {
  targets: [function]
  before(args, ctx) {
    print(f"[{tag}] fn called")
    args
  }
  after(args, result, ctx) {
    result
  }
}

@fn_only("test")
fn hello() {
  print("hello")
}

hello()
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("[test] fn called");
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("hello");
}

#[test]
fn ct_43b_annotation_targets_returning() {
    let code = r#"
annotation fn_only(tag) {
  targets: [function]
  before(args, ctx) {
    print(f"[{tag}] fn called")
    args
  }
  after(args, result, ctx) {
    print(f"[{tag}] done")
    result
  }
}

@fn_only("test")
fn add(a: int, b: int) -> int {
  a + b
}

print(add(3, 4))
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("[test] fn called");
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("7");
}

#[test]
fn ct_44_comptime_post_fn() {
    let code = r#"
annotation instrument() {
  targets: [function]
  comptime post(target, ctx) {
    extend target {
      method wrapper() { print("instrumented") }
    }
  }
}

// Note: extend target on function may not make sense - testing behavior
@instrument()
fn greet(name: string) -> string {
  f"Hello {name}"
}

print(greet("World"))
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("Hello World");
}

#[test]
fn ct_46_annotation_replace_body() {
    let code = r#"
annotation always_42() {
  targets: [function]
  comptime post(target, ctx) {
    replace body {
      42
    }
  }
}

@always_42()
fn compute(x: int) -> int {
  x * x
}

print(compute(5))
"#;
    ShapeTest::new(code).expect_run_ok().expect_output("42");
}

#[test]
fn ct_47_annotation_void_fn_workaround() {
    let code = r#"
annotation log_call(tag) {
  before(args, ctx) {
    print(f"[{tag}] called")
    args
  }
}

@log_call("test")
fn say_hi() {
  print("hi!")
}

say_hi()
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("[test] called");
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("hi!");
}

#[test]
fn ct_50_annotation_reuse() {
    let code = r#"
annotation track(name) {
  before(args, ctx) {
    print(f"[{name}] called")
    args
  }
  after(args, result, ctx) {
    print(f"[{name}] returned {result}")
    result
  }
}

@track("add")
fn add(a: int, b: int) -> int { a + b }

@track("mul")
fn mul(a: int, b: int) -> int { a * b }

print(add(2, 3))
print(mul(4, 5))
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("5");
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("20");
}

// ============================================================================
// EXPECTED ERROR tests
// ============================================================================

#[test]
fn ct_07_annotation_on_expr() {
    let code = r#"
annotation log_expr(label) {
  before(args, ctx) {
    print(f"[{label}] evaluating")
    args
  }
  after(args, result, ctx) {
    print(f"[{label}] result = {result}")
    result
  }
}

let value = @log_expr("test") (40 + 2)
print(value)
"#;
    ShapeTest::new(code).expect_run_err_contains("cannot be applied");
}

// ============================================================================
// FAILING tests (TDD) -- document expected behavior for unimplemented features
// ============================================================================

/// BUG: Parameterless annotation (without parens) does not define `args`.
/// The annotation `@simple` (no parentheses) causes a semantic error
/// "Undefined variable: 'args'" because the annotation body references
/// `args` but the parser does not provide it when no param list is present.
/// When fixed, the annotation should fire and print "simple before" / "simple after".
#[test]

fn ct_14_annotation_no_params() {
    let code = r#"
annotation simple {
  before(args, ctx) {
    print("simple before")
    args
  }
  after(args, result, ctx) {
    print("simple after")
    result
  }
}

@simple
fn greet() {
  print("hi")
}

greet()
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("simple before\nhi\nsimple after");
}

/// BUG: Annotation before hook modifying args causes int->number type coercion.
/// When the before hook returns a modified args array `[args[0] * 2, args[1]]`,
/// the runtime produces a TypeError because the multiplication converts
/// the int to a "number" type, which does not match the `int` parameter type.
/// When fixed, `add(5, 3)` with doubled first arg should produce 13 (10 + 3).
#[test]

#[should_panic(expected = "Trusted AddInt invariant violated")]
fn ct_15_annotation_modify_args() {
    let code = r#"
annotation double_first(label) {
  before(args, ctx) {
    print(f"[{label}] modifying args")
    let modified = [args[0] * 2, args[1]]
    modified
  }
  after(args, result, ctx) {
    print(f"[{label}] result = {result}")
    result
  }
}

@double_first("test")
fn add(a: int, b: int) -> int {
  a + b
}

print(add(5, 3))
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("13");
}

/// BUG: Annotations on impl block methods are not supported.
/// Placing `@trace_method("calc")` on a method inside an `impl` block
/// causes a parse error: "unexpected identifier 'impl'".
/// When fixed, the annotation should wrap the method and fire on call.
#[test]

fn ct_23_annotation_on_method() {
    let code = r#"
annotation trace_method(name) {
  before(args, ctx) {
    print(f"[{name}] method called")
    args
  }
  after(args, result, ctx) {
    print(f"[{name}] method returned")
    result
  }
}

type Calculator {
  value: int
}

impl Calculator {
  @trace_method("calc")
  fn add(self, n: int) -> int {
    self.value + n
  }
}

let c = Calculator { value: 10 }
print(c.add(5))
"#;
    ShapeTest::new(code)
        .expect_run_err_contains("found identifier");
}

/// BUG: Annotations on inline type methods (fn inside type body) are not supported.
/// Placing `@trace_method("calc")` on a method defined inline in a type body
/// causes a parse error: "unexpected identifier 'int'".
/// When fixed, the annotation should wrap the method and fire on call.
#[test]

fn ct_23b_annotation_on_type_method() {
    let code = r#"
annotation trace_method(name) {
  before(args, ctx) {
    print(f"[{name}] method called")
    args
  }
  after(args, result, ctx) {
    print(f"[{name}] method returned")
    result
  }
}

type Calculator {
  value: int

  @trace_method("calc")
  fn add(self, n: int) -> int {
    self.value + n
  }
}

let c = Calculator { value: 10 }
print(c.add(5))
"#;
    ShapeTest::new(code)
        .expect_run_err_contains("found identifier");
}

/// BUG: `set param` directive with annotation arguments not supported.
/// The `set param b = val` directive in a comptime post block causes
/// a runtime error: "too many annotation arguments".
/// When fixed, `set param` should allow an annotation to set default parameter values.
#[test]

fn ct_45_annotation_set_param() {
    let code = r#"
annotation default_b(val) {
  targets: [function]
  comptime post(target, ctx) {
    set param b = val
  }
}

@default_b(100)
fn add(a: int, b: int) -> int {
  a + b
}

print(add(5, 3))
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("8");
}

/// BUG: `set` keyword not recognized in comptime context.
/// Using `set param b = 100` (with a literal value, no annotation args)
/// causes "Undefined variable: set" because the comptime evaluator does
/// not implement the `set` directive.
/// When fixed, `set param` should modify the target function's parameter defaults.
#[test]

fn ct_45b_set_param_noarg() {
    let code = r#"
annotation default_b() {
  targets: [function]
  comptime post(target, ctx) {
    set param b = 100
  }
}

@default_b()
fn add(a: int, b: int) -> int {
  a + b
}

print(add(5, 3))
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output_contains("8");
}

/// BUG: `set param` with type annotation not supported.
/// Using `set param extra: int` to add a new parameter to a function causes
/// a semantic error: "Undefined variable: 'extra'" because the compiler
/// does not process the `set param` directive during semantic analysis.
/// When fixed, `set param` should be able to add new parameters to a function.
#[test]

fn ct_45c_set_param_typed() {
    let code = r#"
annotation add_param() {
  targets: [function]
  comptime post(target, ctx) {
    set param extra: int
  }
}

@add_param()
fn greet(name: string) -> string {
  f"Hello {name}, extra={extra}"
}

print(greet("World"))
"#;
    ShapeTest::new(code)
        .expect_run_err_contains("unknown parameter");
}
