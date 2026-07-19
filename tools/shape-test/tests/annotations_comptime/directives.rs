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
    // E1-D4 (slice 2): the analysis pre-pass now resolves the param spelling
    // against the frozen callable and fails closed with [C0930] BEFORE pass-2's
    // "unknown parameter" message — a still-a-compile-error message change.
    .expect_run_err_contains("[C0930]");
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

// ADR-009 E2 #18 5b Part B (companion A-part-3): `replace_module_from_source_
// string_replaces_items` RETIRED — its subject (a source-string `replace module
// ("…")` that installs and runs) IS the deleted U03 route. The surviving typed
// coverage is `replace_module_typed_fragment_installs_and_runs` below (item_fn ->
// CheckedModule, install+run).

// ADR-009 E2 #18 5b Part B (companion A-part-3): REBASELINED off the deleted
// source-string route onto the typed `item_fn` carrier. `item_fn("answer", "int",
// "not an int")` mints `fn answer() -> int { "not an int" }` — a string body in an
// int-returning function — so the driver's check sequence still reports the
// int/string mismatch. Preserves the "typed-generated bodies ARE type-checked"
// coverage; the U03 source string is gone.
#[test]
fn replace_module_typed_fragment_body_type_errors_are_reported() {
    ShapeTest::new(
        r#"
annotation bad_module() {
  targets: [module]
  comptime post(target, ctx) {
    replace module (item_fn("answer", "int", "not an int"))
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

// ADR-009 E2 #18 5b Part B (companion A-part-3): `replace_module_rejected_on_
// function_target` RETIRED — its source-string arg feeds the deleted U03 route,
// and its subject (a `replace module` on a function target is rejected) is carried
// identically by the typed sibling `replace_module_typed_fragment_rejected_on_
// function_target` below (item_fn carrier, same "only valid when compiling module
// targets" rejection).

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

// ADR-009 E2 #18 5b Part B (companion A-part-3): `replace_module_payload_is_still_
// source_text` RETIRED — its subject (a source-string `replace module ("…")`
// payload reparsed as module source) IS the deleted U03 route. Its deletion-
// boundary SUCCESSOR is `replace_module_source_string_is_rejected_with_named_
// alternative` below (companion B, post-deletion, where C0929 fires).

/// ADR-009 E2 #18 slice 5b Part B (companion B) — the replace-module
/// deletion-boundary successor. Post-U03-deletion (slice 5), a source-string
/// `replace module ("…")` payload is rejected at the builtin boundary with the
/// named `[C0929]` diagnostic pointing at the typed producer alternatives. The
/// replace-module twin of the extend-side d12 successor
/// (`executed_extend_authority::computed_snippet_extend_is_rejected_with_named_alternative`).
#[test]
fn replace_module_source_string_is_rejected_with_named_alternative() {
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
    .expect_run_err_contains("[C0929]")
    .expect_run_err_contains("item_fn");
}

// ADR-009 E2 #18 — the TYPED `replace module` route: a `__CheckedItem` (from
// `item_fn`) reaches the directive WITHOUT a source/JSON string, is built into a
// `CheckedModule` (provenance-stamped, hygienic exports), and installs + runs.
// (Slice 5 deleted the legacy source-string route + its parity test; this is now
// the only `replace module` transport.)

#[test]
fn replace_module_typed_fragment_installs_and_runs() {
    ShapeTest::new(
        r#"
annotation synth_module() {
  targets: [module]
  comptime post(target, ctx) {
    replace module (item_fn("answer", "int", 42))
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
fn replace_module_typed_fragment_rejected_on_function_target() {
    ShapeTest::new(
        r#"
annotation bad_typed_replace() {
  targets: [function]
  comptime post(target, ctx) {
    replace module (item_fn("answer", "int", 42))
  }
}

@bad_typed_replace()
fn answer() -> int { 0 }

print(answer())
"#,
    )
    .expect_run_err_contains("only valid when compiling module targets");
}
