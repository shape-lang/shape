//! Tests for annotations applied to non-function, non-type targets.
//!
//! Covers: annotations on expressions, blocks, let bindings, modules.
//! Most of these are TDD since not all target kinds are implemented.
//!
//! C3-S5c pin-rewrite wave 1 rewrote the two green fn-target pins
//! (`targets_declaration_function_on_function_works`,
//! `annotation_on_module_item`); the C3-S6 A-phase wave rewrote the four
//! target-validation rejection pins (`annotation_on_expression_is_rejected`,
//! `annotation_on_let_binding`, `annotation_on_block_statement`,
//! `targets_function_applied_to_type_errors`) onto typed defs. The comptime
//! pre/post pins are surface-class-independent (unaffected). No legacy
//! spellings remain in this file.

use shape_test::shape_test::ShapeTest;

// TDD: annotation on expression is explicitly rejected (ct_07 in existing tests)
// C3-S6 A-phase typed rewrite (observer `before()`; the rejection semantics
// are the pin, not the hook body).
#[test]
fn annotation_on_expression_is_rejected() {
    ShapeTest::new(
        r#"
annotation log_expr(label: string) {
  before() {
    print(f"[{label}] evaluating")
  }
}

let value = @log_expr("test") (40 + 2)
print(value)
"#,
    )
    .expect_run_err_contains("cannot be applied");
}

// TDD: annotation on let binding parse error "expected something else, found identifier `print`"
// C3-S6 A-phase typed rewrite (observer `before()`).
#[test]
fn annotation_on_let_binding() {
    ShapeTest::new(
        r#"
annotation validate(label: string) {
  before() {
    print(f"[{label}] binding created")
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
// C3-S6 A-phase typed rewrite (observer pair).
#[test]
fn annotation_on_block_statement() {
    ShapeTest::new(
        r#"
annotation timed(label: string) {
  before() {
    print(f"[{label}] block start")
  }
  after() {
    print(f"[{label}] block end")
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
annotation fn_target(tag: string) on function {
  before() {
    print(f"[{tag}] before")
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
annotation type_target() on type {
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
// C3-S6 A-phase typed rewrite: the def gains a typed config param (`tag`) so
// it classifies TypedConfig; the pin's semantic — `targets: [function]`
// applied to a type is a NAMED rejection mentioning targets — is unchanged.
#[test]
fn targets_function_applied_to_type_errors() {
    ShapeTest::new(
        r#"
annotation fn_only(tag: string) on function {
  before() {
    print(f"[{tag}] should not reach")
  }
}

@fn_only("t")
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
annotation type_only() on type {
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

// A fn-target pin despite the name (the F1 ledger classification).
#[test]
fn annotation_on_module_item() {
    ShapeTest::new(
        r#"
annotation module_meta(label: string) {
  before() {
    print(f"[{label}] module loaded")
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
