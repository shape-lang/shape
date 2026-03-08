//! Tests for annotations applied to non-function, non-type targets.
//!
//! Covers: annotations on expressions, blocks, let bindings, modules.
//! Most of these are TDD since not all target kinds are implemented.

use shape_test::shape_test::ShapeTest;

// TDD: annotation on expression is explicitly rejected (ct_07 in existing tests)
#[test]
fn annotation_on_expression_is_rejected() {
    ShapeTest::new(
        r#"
annotation log_expr(label) {
  before(args, ctx) {
    print(f"[{label}] evaluating")
    args
  }
}

let value = @log_expr("test") (40 + 2)
print(value)
"#,
    )
    .expect_run_err_contains("cannot be applied");
}

// TDD: annotation on let binding parse error "expected something else, found identifier `print`"
#[test]
fn annotation_on_let_binding() {
    ShapeTest::new(
        r#"
annotation validate(label) {
  before(args, ctx) {
    print(f"[{label}] binding created")
    args
  }
}

@validate("x")
let x = 42
print(x)
"#,
    )
    .expect_run_err();
}

// Annotation on block statement is rejected with "cannot be applied to a block"
#[test]
fn annotation_on_block_statement() {
    ShapeTest::new(
        r#"
annotation timed(label) {
  before(args, ctx) {
    print(f"[{label}] block start")
    args
  }
  after(args, result, ctx) {
    print(f"[{label}] block end")
    result
  }
}

@timed("block")
{
  let x = 1 + 2
  print(x)
}
"#,
    )
    .expect_run_err();
}

#[test]
fn targets_declaration_function_on_function_works() {
    ShapeTest::new(
        r#"
annotation fn_target(tag) {
  targets: [function]
  before(args, ctx) {
    print(f"[{tag}] before")
    args
  }
}

@fn_target("ok")
fn hello() { print("hello") }

hello()
"#,
    )
    .expect_run_ok()
    .expect_output("[ok] before\nhello");
}

#[test]
fn targets_declaration_type_on_type_works() {
    ShapeTest::new(
        r#"
annotation type_target() {
  targets: [type]
  comptime post(target, ctx) {
    extend target {
      method id_str() { f"id:{self.id}" }
    }
  }
}

@type_target()
type Entity { id: int }

let e = Entity { id: 7 }
print(e.id_str())
"#,
    )
    .expect_run_ok()
    .expect_output("id:7");
}

// TDD: annotation target mismatch error not yet reported at compile time
#[test]
fn targets_function_applied_to_type_errors() {
    ShapeTest::new(
        r#"
annotation fn_only() {
  targets: [function]
  before(args, ctx) {
    print("should not reach")
    args
  }
}

@fn_only()
type Wrong { x: int }

print("after")
"#,
    )
    .expect_run_err_contains("target");
}

// TDD: annotation target mismatch error not yet reported at compile time
#[test]
fn targets_type_applied_to_function_errors() {
    ShapeTest::new(
        r#"
annotation type_only() {
  targets: [type]
  comptime post(target, ctx) {
    extend target {
      method nope() { 0 }
    }
  }
}

@type_only()
fn wrong() -> int { 1 }

print(wrong())
"#,
    )
    .expect_run_err_contains("target");
}

// TDD: annotation on module-level scope not yet implemented
#[test]
fn annotation_on_module_item() {
    ShapeTest::new(
        r#"
annotation module_meta(label) {
  before(args, ctx) {
    print(f"[{label}] module loaded")
    args
  }
}

@module_meta("mymod")
fn module_init() {
  print("init")
}

module_init()
"#,
    )
    .expect_run_ok()
    .expect_output_contains("[mymod] module loaded")
    .expect_output_contains("init");
}
