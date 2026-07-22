//! Stdlib comptime showcases (comptime-excellence design §4.9).
//!
//! Exercises the two shipped derive-style / LLM-integration annotation patterns
//! end to end through cross-module import (`from std::serde::derive use { ... }`
//! / `from std::llm::tools use { ... }`), under BOTH the interpreter and the JIT
//! executor, asserting the generated code produces the exact expected output.
//!
//! The programs return the generated string as the program's final value and
//! the tests assert on it via `expect_string` (not `expect_output`). A
//! natively-JIT-compiled `print` emits straight to process stdout
//! (`jit_v2_string_print`), bypassing the runtime `OutputAdapter` the harness
//! installs to capture output — so `.expect_output()` under `--mode jit` would
//! only observe output on a deopt. Asserting the return value observes the
//! generated code's result identically under native JIT and the interpreter
//! (WF-3D generated-free-function JIT-native fix). Print-channel VM == JIT
//! parity for these same programs is gated by the book truth-gate.
//!
//! These make the CLAUDE.md "user-defined LLM integration patterns in
//! stdlib/userland" claim true for the first time: `@json_schema`, `@llm_tool`
//! and `@prompt` are ordinary Shape annotations living in `stdlib-src/serde/`
//! and `stdlib-src/llm/`, built entirely on the public comptime contract
//! (`target.fields` / `target.params` / `target.return_type`, `error()`, and
//! the typed `extend (item_fn(...))` / `extend (extend_method(...))`
//! code-generation surface — ADR-009 E2 #18 slices 4/4.5 moved these off the
//! retired `extend (f"…")` + `string_lit` source-string form).

use shape_test::shape_test::ShapeTest;

const SERDE_PROGRAM: &str = r#"
from std::serde::derive use { @json_schema }

@json_schema()
type User {
    @description("Unique identifier") id: int,
    name: string,
    email: string?,
}

User_json_schema()
"#;

const SERDE_EXPECTED: &str = r#"{"type": "object", "title": "User", "properties": {"id": {"type": "integer", "description": "Unique identifier"}, "name": {"type": "string"}, "email": {"type": "string"}}, "required": ["id", "name"]}"#;

#[test]
fn json_schema_derives_schema_via_stdlib_import_vm() {
    ShapeTest::new(SERDE_PROGRAM)
        .with_stdlib()
        .expect_string(SERDE_EXPECTED);
}

#[test]
fn json_schema_derives_schema_via_stdlib_import_jit() {
    ShapeTest::new(SERDE_PROGRAM)
        .with_stdlib()
        .with_jit()
        .expect_string(SERDE_EXPECTED);
}

// A field whose type has no JSON Schema mapping is a compile error naming the
// field — the deriver never emits a schema it cannot justify (§4.9.1).
#[test]
fn json_schema_unsupported_field_type_is_compile_error() {
    ShapeTest::new(
        r#"
from std::serde::derive use { @json_schema }

type Inner { v: int }

@json_schema()
type Outer { thing: Inner }

print("unreachable")
"#,
    )
    .with_stdlib()
    .expect_run_err_contains("has no JSON Schema mapping");
}

const TO_JSON_PROGRAM: &str = r#"
from std::serde::serialize use { @to_json }

@to_json()
type User { id: int, name: string }

let u = User { id: 1, name: "Ada" }
u.to_json()
"#;

const TO_JSON_EXPECTED: &str = r#"{ "id": 1, "name": "Ada" }"#;

#[test]
fn to_json_serializes_via_stdlib_import_vm() {
    ShapeTest::new(TO_JSON_PROGRAM)
        .with_stdlib()
        .expect_string(TO_JSON_EXPECTED);
}

#[test]
fn to_json_serializes_via_stdlib_import_jit() {
    ShapeTest::new(TO_JSON_PROGRAM)
        .with_stdlib()
        .with_jit()
        .expect_string(TO_JSON_EXPECTED);
}

// ADR-009 E2 #18 slice 4.5 (E2-Q2/B condition 3) — end-to-end injection guard:
// the `extend_method` template channel is structurally incapable of carrying an
// arbitrary handler expression. A field-splice value with non-identifier content
// (`a} + boom() + {b`) is REJECTED at the builtin boundary with the named
// `[C0927]` diagnostic and never assembled into a generated body. This is the
// negative pin for the BOUNDED HOLE GRAMMAR condition, through the real comptime
// path (the builder-tier pin is `comptime_builtins::tests::
// extend_method_rejects_non_identifier_field_splice_c0927`).
#[test]
fn extend_method_rejects_injection_field_splice() {
    ShapeTest::new(
        r#"
annotation inject() on type {
    comptime post(target, ctx) {
        extend (extend_method(target.name, "evil", "string", ["{ ", " }"], ["a} + boom() + {b"]))
    }
}

@inject()
type T { x: int }

print("unreachable")
"#,
    )
    .expect_run_err_contains("C0927");
}

// ADR-009 E2 #18 slice 4.5 (review F2) — DISCLOSED behavior change, pinned so it
// fails LOUDLY if silently "fixed". A 0-field `@to_json` type is REJECTED
// ("requires at least one field"): the typed template's field_splices array would
// have to be empty, and an empty typed-array literal init is not available in
// comptime ([C0001]). The retired string-concat serializer produced `{  }` for a
// 0-field type — this is an accepted behavior delta vs the source route (an
// untested edge case), not a regression to fix silently. `error()` surfaces on
// the stdlib-import path (unlike a swallowed [C0001]), so the rejection is
// observable end-to-end.
#[test]
fn to_json_zero_field_type_is_rejected() {
    ShapeTest::new(
        r#"
from std::serde::serialize use { @to_json }

@to_json()
type Empty {}

print("unreachable")
"#,
    )
    .with_stdlib()
    .expect_run_err_contains("requires at least one field");
}

const LLM_PROGRAM: &str = r#"
from std::llm::tools use { @llm_tool }

/// Get current weather for a city
@llm_tool("Get current weather for a city")
fn get_weather(city: string, units: string) -> string {
    f"\{\"city\": \"{city}\", \"temp_c\": 21\}"
}

get_weather_tool_def()
"#;

const LLM_EXPECTED: &str = r#"{"name": "get_weather", "description": "Get current weather for a city", "parameters": {"type": "object", "properties": {"city": {"type": "string"}, "units": {"type": "string"}}, "required": ["city", "units"]}}"#;

#[test]
fn llm_tool_derives_schema_via_stdlib_import_vm() {
    ShapeTest::new(LLM_PROGRAM)
        .with_stdlib()
        .expect_string(LLM_EXPECTED);
}

#[test]
fn llm_tool_derives_schema_via_stdlib_import_jit() {
    ShapeTest::new(LLM_PROGRAM)
        .with_stdlib()
        .with_jit()
        .expect_string(LLM_EXPECTED);
}

// A parameter whose type has no JSON mapping is a compile error naming the
// parameter and the function (§4.9.2).
#[test]
fn llm_tool_unsupported_param_type_is_compile_error() {
    ShapeTest::new(
        r#"
from std::llm::tools use { @llm_tool }

type Coord { x: int }

@llm_tool("locate a point")
fn locate(where_at: Coord) -> string { "x" }

print("unreachable")
"#,
    )
    .with_stdlib()
    .expect_run_err_contains("has no JSON Schema mapping");
}

// A valid prompt template — every `{placeholder}` names a parameter — compiles
// and runs.
#[test]
fn prompt_valid_template_compiles_and_runs() {
    ShapeTest::new(
        r#"
from std::llm::tools use { @prompt }

@prompt("Summarize the weather in {city} for a {audience} audience")
fn weather_prompt(city: string, audience: string) -> string {
    f"Weather in {city} for {audience}"
}

print("ok")
"#,
    )
    .with_stdlib()
    .expect_output("ok");
}

// A placeholder typo is a compile error naming the placeholder and the function.
#[test]
fn prompt_placeholder_typo_is_compile_error() {
    ShapeTest::new(
        r#"
from std::llm::tools use { @prompt }

@prompt("Summarize the weather in {city} for a {audence} audience")
fn weather_prompt(city: string, audience: string) -> string {
    f"Weather in {city}"
}

print("unreachable")
"#,
    )
    .with_stdlib()
    .expect_run_err_contains("{audence}");
}

/// THE ERGONOMICS CELL for `#74 INTERIM REJECTION` (ADR-009 E4-D5, slice S2).
/// MEASURED at `75eca793` before the fix: this exact program compiled, ran,
/// printed `7`, exited 0 — and generated NOTHING, so `labs_tool_def()` simply
/// did not exist. That is literally the scenario #74's polyglot bullet
/// describes. It now fails LOUD, citing #74. Sits beside the
/// `llm_tool_derives_schema_via_stdlib_import_*` positives, which prove the
/// same annotation still generates its schema on an ordinary Shape fn.
#[test]
fn llm_tool_on_extern_c_fn_is_rejected_citing_74() {
    ShapeTest::new(
        r#"
from std::llm::tools use { @llm_tool }

@llm_tool("absolute value")
extern "C" fn labs(x: int) -> int from "c"

print("unreachable")
"#,
    )
    .with_stdlib()
    .expect_run_err_contains("see issue #74");
}
