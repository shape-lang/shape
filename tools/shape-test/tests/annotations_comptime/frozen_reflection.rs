//! ADR-009 A1 slice S3: annotation comptime handlers consume the real
//! per-compilation-unit semantic freeze — including during the speculative
//! pre-pass, which runs AFTER the (moved) registration-complete freeze
//! barrier. A handler body calling `type_ref`/`type_category` on a USER type
//! resolves against the frozen reflection surface under both VM and JIT;
//! the generated function it emits is visible to every user body (not only
//! at top level), because the pre-pass no longer defers reflection-using
//! handlers to pass 2.
//!
//! JIT proofs are VALUE-based (`expect_vm_and_jit_number`, the same
//! convention as `generated_capture.rs` / `flagship_wf3d.rs`): the
//! JIT-native `print` path does not route through the test harness's
//! `CaptureAdapter` output sink (pre-existing, S3-independent — a bare
//! `print(7)` under `with_jit()` + `expect_output` fails identically),
//! so output-based assertions run VM-side.

use shape_test::shape_test::ShapeTest;

fn expect_vm_and_jit_number(source: &str, expected: f64) {
    ShapeTest::new(source).expect_number(expected);
    ShapeTest::new(source).with_jit().expect_number(expected);
}

/// The load-bearing S3 case: a type-targeting `comptime post` handler whose
/// body USES frozen reflection and generates a free function that a user
/// `fn` body calls. This requires the speculative pre-pass itself to run
/// with the real freeze handle: if reflection were rejected (or fed an
/// empty snapshot) at pre-pass time, the generated function's signature
/// would never be registered before analysis and `show()` would fail with
/// "Undefined function".
#[test]
fn annotation_handler_reflection_reaches_generated_fn_called_from_fn_body() {
    let source = r#"
annotation reflect_category() {
  targets: [type]
  comptime post(target, ctx) {
    let flag = match type_category(type_ref(User)) {
      FrozenTypeCategory::Nominal => 1
      _ => 0
    }
    extend (f"fn user_category_flag() -> int \{ {flag} \}")
  }
}

@reflect_category()
type User { id: int }

fn show() -> int { user_category_flag() }
show()
"#;
    expect_vm_and_jit_number(source, 1.0);
}

/// Same reflection-using handler, consumed at top level, with the printed
/// output asserted on the VM path (JIT print does not reach the
/// `CaptureAdapter` output sink — see module doc).
#[test]
fn annotation_handler_reflection_reaches_generated_fn_called_top_level() {
    let source = r#"
annotation reflect_category() {
  targets: [type]
  comptime post(target, ctx) {
    let flag = match type_category(type_ref(User)) {
      FrozenTypeCategory::Nominal => 1
      _ => 0
    }
    extend (f"fn user_category_flag() -> int \{ {flag} \}")
  }
}

@reflect_category()
type User { id: int }

print(user_category_flag())
user_category_flag()
"#;
    ShapeTest::new(source).expect_output("1");
    expect_vm_and_jit_number(source, 1.0);
}

/// Function-targeting handler (the signature-directive pre-pass path):
/// reflection on a user type inside the handler body resolves against the
/// real freeze in every phase — the handler asserts the frozen category and
/// hard-fails compilation via `error()` if reflection lied.
#[test]
fn function_target_annotation_handler_asserts_frozen_category() {
    let source = r#"
annotation assert_user_is_nominal() {
  targets: [function]
  comptime post(target, ctx) {
    match type_category(type_ref(User)) {
      FrozenTypeCategory::Nominal => 1
      _ => error("frozen reflection returned a non-nominal category for User")
    }
  }
}

type User { id: int }

@assert_user_is_nominal()
fn compute() -> int { 7 }

compute()
"#;
    expect_vm_and_jit_number(source, 7.0);
}

/// Negative counterpart of the function-target case: the handler's frozen
/// category assertion FAILING must surface the handler's `error()` as a
/// compile error — proving the handler really consulted the freeze (a
/// suppressed/empty reflection surface could never take this branch).
#[test]
fn function_target_annotation_handler_error_branch_fires_on_wrong_category() {
    let source = r#"
annotation assert_int_is_nominal() {
  targets: [function]
  comptime post(target, ctx) {
    match type_category(type_ref(int)) {
      FrozenTypeCategory::Nominal => 1
      _ => error("int is not nominal, as frozen reflection correctly reports")
    }
  }
}

@assert_int_is_nominal()
fn compute() -> int { 7 }

compute()
"#;
    ShapeTest::new(source)
        .expect_run_err_contains("int is not nominal, as frozen reflection correctly reports");
    ShapeTest::new(source)
        .with_jit()
        .expect_run_err_contains("int is not nominal, as frozen reflection correctly reports");
}
