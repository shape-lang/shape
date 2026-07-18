//! Tests for annotation-driven code generation at compile time.
//!
//! Covers: annotations generating wrapper functions, generating serialization
//! methods, generating accessor methods, and composing generated code.

use shape_test::shape_test::ShapeTest;

#[test]
fn annotation_generates_display_method() {
    ShapeTest::new(
        r#"
annotation displayable() {
  targets: [type]
  comptime post(target, ctx) {
    extend target {
      method display() { f"({self.x}, {self.y})" }
    }
  }
}

@displayable()
type Point { x: int, y: int }

let p = Point { x: 3, y: 4 }
print(p.display())
"#,
    )
    .expect_run_ok()
    .expect_output("(3, 4)");
}

#[test]
fn annotation_generates_getter_method() {
    ShapeTest::new(
        r#"
annotation with_getter() {
  targets: [type]
  comptime post(target, ctx) {
    extend target {
      method get_value() { self.value }
    }
  }
}

@with_getter()
type Container { value: int }

let c = Container { value: 42 }
print(c.get_value())
"#,
    )
    .expect_run_ok()
    .expect_output("42");
}

#[test]
fn annotation_replace_body_generates_constant_function() {
    // Annotation arguments are accepted even when this handler emits a fixed
    // replacement body.
    ShapeTest::new(
        r#"
annotation stub_return(val) {
  targets: [function]
  comptime post(target, ctx) {
    replace body {
      "stubbed"
    }
  }
}

@stub_return("placeholder")
fn fetch_data() -> string {
  "real data from server"
}

print(fetch_data())
"#,
    )
    .expect_run_ok()
    .expect_output("stubbed");
}

/// ADR-009 E2 #18 (slice 3) — a CLOSURE-BEARING `replace body` edit compiles and
/// runs correctly after slice-3's pre-analysis materialization of the
/// replacement. The replacement carries an explicit `move`-capture closure (the
/// C0911 shape); slice-3 makes the analyzer see it pre-analysis (publishing its
/// structural fact — the LSP flip), and this pin guards that the same
/// materialization leaves RUNTIME behavior intact: the VM produces 42 (the
/// post-edit replacement, not the pre-edit `7`, via the authoritative pass-2
/// swap).
///
/// VM-ONLY here on purpose: the shape-test harness `JITExecutor` leg captures
/// EMPTY stdout for annotation-generated programs — a PRE-EXISTING harness debt
/// (the same family as the `generated_method_runtime` baseline names, e.g.
/// `generated_extend_target_method_behaves_identically_in_vm_and_jit`; root cause
/// untraced, main-side, NOT E2 scope). A `.with_jit()` leg here would red on that
/// hole, not on any real defect. The JIT-side truth for this exact fixture — it
/// installs, runs, and prints 42 under `--mode jit` — is proven where the harness
/// works, by the CLI native proof
/// (`bin/shape-cli/tests/cli/jit_c2_install_native.rs::e2_closure_bearing_replace_body_runs_natively_both_tiers`,
/// zero-fallback) and the supervisor's 4-way CLI differential (2026-07-18).
#[test]
fn closure_bearing_replace_body_edit_runs_in_vm() {
    let program = r#"
annotation edit_answer() {
  targets: [function]
  comptime post(target, ctx) {
    replace body {
      let base = 40
      let worker = |; move base| base + 2
      return worker()
    }
  }
}

@edit_answer()
fn answer() -> int { 7 }

print(answer())
"#;
    ShapeTest::new(program).expect_output("42");
}

#[test]
fn annotation_extends_type_with_equality_check() {
    ShapeTest::new(
        r#"
annotation with_eq() {
  targets: [type]
  comptime post(target, ctx) {
    extend target {
      method eq(other) { self.id == other.id }
    }
  }
}

@with_eq()
type Entity { id: int, name: string }

let a = Entity { id: 1, name: "Alice" }
let b = Entity { id: 1, name: "Bob" }
let c = Entity { id: 2, name: "Alice" }

print(a.eq(b))
print(a.eq(c))
"#,
    )
    .expect_run_ok()
    .expect_output("true\nfalse");
}

#[test]
fn stacked_annotations_both_extend_type() {
    ShapeTest::new(
        r#"
annotation with_sum() {
  targets: [type]
  comptime post(target, ctx) {
    extend target {
      method sum() { self.a + self.b }
    }
  }
}

annotation with_diff() {
  targets: [type]
  comptime post(target, ctx) {
    extend target {
      method diff() { self.a - self.b }
    }
  }
}

@with_sum()
@with_diff()
type Numbers { a: int, b: int }

let n = Numbers { a: 10, b: 3 }
print(n.sum())
print(n.diff())
"#,
    )
    .expect_run_ok()
    .expect_output("13\n7");
}

// TDD: annotation code gen for serialization methods requires string building from fields
#[test]
fn annotation_generates_to_string_method() {
    ShapeTest::new(
        r#"
annotation stringable() {
  targets: [type]
  comptime post(target, ctx) {
    extend target {
      method to_str() { f"{self.name}:{self.value}" }
    }
  }
}

@stringable()
type Config { name: string, value: int }

let c = Config { name: "timeout", value: 30 }
print(c.to_str())
"#,
    )
    .expect_run_ok()
    .expect_output("timeout:30");
}

#[test]
fn annotation_generated_extend_method_runs_under_jit() {
    ShapeTest::new(
        r#"
annotation summary() {
  targets: [type]
  comptime post(target, ctx) {
    extend target {
      method summary() -> string { f"{self.name}:{self.id}" }
    }
  }
}

@summary()
type User { id: int, name: string }

let u = User { id: 7, name: "Ada" }
u.summary()
"#,
    )
    .with_jit()
    .expect_string("Ada:7");
}

#[test]
fn annotation_generates_predicate_method() {
    ShapeTest::new(
        r#"
annotation checkable() {
  targets: [type]
  comptime post(target, ctx) {
    extend target {
      method is_positive() { self.value > 0 }
    }
  }
}

@checkable()
type Measurement { value: int }

let pos = Measurement { value: 5 }
let neg = Measurement { value: -3 }
print(pos.is_positive())
print(neg.is_positive())
"#,
    )
    .expect_run_ok()
    .expect_output("true\nfalse");
}
