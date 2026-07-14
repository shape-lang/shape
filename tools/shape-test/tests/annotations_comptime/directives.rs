//! Focused public-surface tests for comptime directives.
//!
//! These tests intentionally exercise the annotation-facing directive syntax,
//! not the lower-level compiler unit-test helpers.

use shape_test::shape_test::ShapeTest;

#[test]
fn set_param_value_supplies_default_for_omitted_arg() {
    ShapeTest::new(
        r#"
annotation default_y(val) {
  targets: [function]
  comptime post(target, ctx) {
    set param y = val
  }
}

@default_y(10)
fn add(x: int, y: int) -> int {
  x + y
}

print(add(5))
"#,
    )
    .expect_run_ok()
    .expect_output("15");
}

#[test]
fn set_param_value_explicit_arg_still_overrides_generated_default() {
    ShapeTest::new(
        r#"
annotation default_y(val) {
  targets: [function]
  comptime post(target, ctx) {
    set param y = val
  }
}

@default_y(10)
fn add(x: int, y: int) -> int {
  x + y
}

print(add(5, 3))
print(add(5))
"#,
    )
    .expect_run_ok()
    .expect_output("8\n15");
}

#[test]
fn set_param_value_unknown_param_is_compile_error() {
    ShapeTest::new(
        r#"
annotation default_missing() {
  targets: [function]
  comptime post(target, ctx) {
    set param missing = 1
  }
}

@default_missing()
fn add(x: int) -> int {
  x
}

print(add(5))
"#,
    )
    .expect_run_err_contains("unknown parameter 'missing'");
}

#[test]
fn set_param_value_string_default_is_supported() {
    ShapeTest::new(
        r#"
annotation default_suffix(val) {
  targets: [function]
  comptime post(target, ctx) {
    set param suffix = val
  }
}

@default_suffix("!")
fn shout(name: string, suffix: string) -> string {
  name + suffix
}

print(shout("shape"))
"#,
    )
    .expect_run_ok()
    .expect_output("shape!");
}

#[test]
fn set_param_value_bool_default_is_supported() {
    ShapeTest::new(
        r#"
annotation default_flag(val) {
  targets: [function]
  comptime post(target, ctx) {
    set param flag = val
  }
}

@default_flag(true)
fn label(flag: bool) -> string {
  if flag { "enabled" } else { "disabled" }
}

print(label())
"#,
    )
    .expect_run_ok()
    .expect_output("enabled");
}

#[test]
fn set_param_value_number_default_is_supported() {
    ShapeTest::new(
        r#"
annotation default_limit(val) {
  targets: [function]
  comptime post(target, ctx) {
    set param limit = val
  }
}

@default_limit(2.5)
fn over_limit(value: number, limit: number) -> bool {
  value > limit
}

print(over_limit(3.0))
"#,
    )
    .expect_run_ok()
    .expect_output("true");
}

#[test]
fn replace_module_from_source_string_replaces_items() {
    ShapeTest::new(
        r#"
annotation synth_module() {
  targets: [module]
  comptime post(target, ctx) {
    replace module ("fn answer() -> int { 42 }")
  }
}

@synth_module()
mod demo {
  fn answer() -> int { 0 }
}

print(demo::answer())
"#,
    )
    .expect_run_ok()
    .expect_output("42");
}

#[test]
fn replace_module_generated_source_type_errors_are_reported() {
    ShapeTest::new(
        r#"
annotation bad_module() {
  targets: [module]
  comptime post(target, ctx) {
    replace module ("fn answer() -> int { \"not an int\" }")
  }
}

@bad_module()
mod demo {
  fn answer() -> int { 0 }
}

print(demo::answer())
"#,
    )
    .expect_run_err_contains_any(&[
        "not compatible",
        "do not unify",
        "type mismatch",
        "return type",
    ]);
}

#[test]
fn replace_module_rejected_on_function_target() {
    ShapeTest::new(
        r#"
annotation bad_replace() {
  targets: [function]
  comptime post(target, ctx) {
    replace module ("fn answer() -> int { 42 }")
  }
}

@bad_replace()
fn answer() -> int {
  0
}

print(answer())
"#,
    )
    .expect_run_err_contains(
        "`replace module` directives are only valid when compiling module targets",
    );
}

#[test]
fn extend_item_fragment_generates_free_function_without_source_string() {
    ShapeTest::new(
        r#"
annotation typed_label() {
  targets: [type]
  comptime post(target, ctx) {
    extend (item_fn(f"{target.name}_label", type_info("string").type_ref, "typed fragment"))
  }
}

@typed_label()
type Widget { id: int }

print(Widget_label())
"#,
    )
    .expect_run_ok()
    .expect_output("typed fragment");
}

#[test]
fn replace_module_payload_is_still_source_text() {
    ShapeTest::new(
        r#"
annotation malformed_module() {
  targets: [module]
  comptime post(target, ctx) {
    replace module ("fn broken( {")
  }
}

@malformed_module()
mod demo {
  fn answer() -> int { 0 }
}

print(demo::answer())
"#,
    )
    .expect_run_err_contains("invalid replacement module payload");
}
