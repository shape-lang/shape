use shape_test::shape_test::ShapeTest;

mod declared_capture;
mod gate_totality;
mod slice4;

fn expect_vm_and_jit_number(source: &str, expected: f64) {
    ShapeTest::new(source).expect_number(expected);
    ShapeTest::new(source).with_jit().expect_number(expected);
}

#[test]
fn generated_function_rejects_implicit_closure_capture() {
    ShapeTest::new(
        r#"
annotation generate_worker() {
  targets: [type]
  comptime post(target, ctx) {
    extend ("fn generated_worker() -> int { let value = 41; let worker = || value + 1; worker() }")
  }
}

@generate_worker()
type Job { id: int }

print(generated_worker())
"#,
    )
    .expect_run_err_contains(
        "generated closure implicitly captures 'value'; generated captures must be explicit",
    );
}

#[test]
fn generated_function_rejects_parameter_capture() {
    ShapeTest::new(
        r#"
annotation generate_worker() {
  targets: [type]
  comptime post(target, ctx) {
    extend ("fn generated_worker(base: int) -> int { let worker = || base + 1; worker() }")
  }
}

@generate_worker()
type Job { id: int }

print(generated_worker(41))
"#,
    )
    .expect_run_err_contains(
        "generated closure implicitly captures 'base'; generated captures must be explicit",
    );
}

#[test]
fn generated_capture_diagnostic_is_deterministically_sorted() {
    let source = r#"
annotation generate_worker() {
  targets: [type]
  comptime post(target, ctx) {
    extend ("fn generated_worker() -> int { let z = 40; let a = 2; let worker = || z + a; worker() }")
  }
}

@generate_worker()
type Job { id: int }

print(generated_worker())
"#;
    let expected =
        "generated closure implicitly captures 'a', 'z'; generated captures must be explicit";
    ShapeTest::new(source).expect_run_err_contains(expected);
    ShapeTest::new(source)
        .with_jit()
        .expect_run_err_contains(expected);
}

#[test]
fn generated_method_rejects_implicit_self_capture() {
    ShapeTest::new(
        r#"
annotation add_reader() {
  targets: [type]
  comptime post(target, ctx) {
    extend (f"extend {target.name} \{ method read() -> int \{ let worker = || self.id; worker() \} \}")
  }
}

@add_reader()
type Job { id: int }

let job = Job { id: 42 }
print(job.read())
"#,
    )
    .expect_run_err_contains(
        "generated closure implicitly captures 'self'; generated captures must be explicit",
    );
}

#[test]
fn generated_closure_parameters_are_not_captures() {
    let source = r#"
annotation generate_increment() {
  targets: [type]
  comptime post(target, ctx) {
    extend ("fn generated_increment() -> int { let increment = |value| value + 1; increment(41) }")
  }
}

@generate_increment()
type Job { id: int }

generated_increment()
"#;
    expect_vm_and_jit_number(source, 42.0);
}

#[test]
fn generated_method_allows_capture_free_closure_in_both_tiers() {
    let source = r#"
annotation add_answer() {
  targets: [type]
  comptime post(target, ctx) {
    extend (f"extend {target.name} \{ method answer() -> int \{ let worker = || 42; worker() \} \}")
  }
}

@add_answer()
type Job { id: int }

let job = Job { id: 1 }
job.answer()
"#;
    expect_vm_and_jit_number(source, 42.0);
}

#[test]
fn ordinary_source_closures_keep_implicit_capture_in_both_tiers() {
    let source = r#"
fn answer() -> int {
  let value = 41
  let worker = || value + 1
  worker()
}

answer()
"#;
    expect_vm_and_jit_number(source, 42.0);
}
