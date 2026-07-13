//! ADR-009 ticket D1 (slice S3) — VM + JIT behavior-unchanged proofs for
//! generated declarations on the extend/materialization path.
//!
//! Provenance stamping and real source anchors (SymbolId / ExpansionIdentity /
//! GeneratedOrigin, no `Span::DUMMY` on generated decls) are compile-time
//! metadata: runtime behavior of `extend target { method }` generated methods
//! and generated free functions must be IDENTICAL in both execution modes,
//! before and after D1. Each test runs the same program through the VM
//! interpreter and through the JIT (frozen_type.rs pattern) and requires the
//! same output — Wave-38F generated-method JIT parity is the regression this
//! file keeps green.

use shape_test::shape_test::ShapeTest;

fn expect_vm_and_jit_output(source: &str, expected: &str) {
    ShapeTest::new(source).expect_output(expected);
    ShapeTest::new(source).with_jit().expect_output(expected);
}

/// Direct handler-AST directive shape: `extend target { method }`.
#[test]
fn generated_extend_target_method_behaves_identically_in_vm_and_jit() {
    expect_vm_and_jit_output(
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
        "(3, 4)",
    );
}

/// Generated method with arithmetic over receiver fields (JIT-favored shape).
#[test]
fn generated_extend_target_arithmetic_method_behaves_identically_in_vm_and_jit() {
    expect_vm_and_jit_output(
        r#"
annotation summable() {
  targets: [type]
  comptime post(target, ctx) {
    extend target {
      method total() -> int { self.a + self.b * 2 }
    }
  }
}

@summable()
type Pair { a: int, b: int }

let pair = Pair { a: 5, b: 10 }
print(pair.total())
"#,
        "25",
    );
}

/// Snippet directive shape: `extend (f"extend {target.name} { … }")` — the
/// generated method is parsed from synthetic snippet text and re-anchored to
/// the application site; its runtime behavior is unchanged.
#[test]
fn generated_snippet_extend_method_behaves_identically_in_vm_and_jit() {
    expect_vm_and_jit_output(
        r#"
annotation gen() {
  targets: [type]
  comptime post(target, ctx) {
    extend (f"extend {target.name} \{ method answer() -> int \{ 42 \} \}")
  }
}

@gen()
type Point { id: int }

let p = Point { id: 1 }
print(p.answer())
"#,
        "42",
    );
}

/// Generated FREE function (the §4.5.1 pre-pass visibility case): resolved
/// from user function bodies, compiled through the full MIR/JIT driver,
/// identical RESULT VALUE in both modes.
///
/// Asserted on the program's result value rather than `print` capture: for
/// the `print(user_fn_call())` shape the JIT path writes through raw stdout
/// instead of the harness CaptureAdapter — a pre-existing harness/output
/// gap that a hand-written control (`fn handwritten_flag…`) reproduces
/// byte-for-byte, so it is NOT a generated-decl behavior difference. The
/// result value is captured identically in both modes.
#[test]
fn generated_free_function_behaves_identically_in_vm_and_jit() {
    let source = r#"
annotation gen2() {
  targets: [type]
  comptime post(target, ctx) {
    extend ("fn generated_flag() -> int { 7 }")
  }
}

@gen2()
type Point { id: int }

fn double_flag() -> int { generated_flag() * 2 }

double_flag()
"#;
    ShapeTest::new(source).expect_number(14.0);
    ShapeTest::new(source).with_jit().expect_number(14.0);
}
