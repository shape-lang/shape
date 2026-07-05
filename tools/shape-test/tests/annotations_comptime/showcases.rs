//! Stdlib comptime showcases (comptime-excellence design §4.9).
//!
//! Exercises the two shipped derive-style / LLM-integration annotation patterns
//! end to end through cross-module import (`from std::serde::derive use { ... }`
//! / `from std::llm::tools use { ... }`), under BOTH the interpreter and the JIT
//! executor, asserting the generated code produces the exact expected output.
//!
//! These make the CLAUDE.md "user-defined LLM integration patterns in
//! stdlib/userland" claim true for the first time: `@json_schema`, `@llm_tool`
//! and `@prompt` are ordinary Shape annotations living in `stdlib-src/serde/`
//! and `stdlib-src/llm/`, built entirely on the public comptime contract
//! (`target.fields` / `target.params` / `target.return_type`, `error()`, and
//! the `extend (...)` + `string_lit` code-generation surface).

use shape_test::shape_test::ShapeTest;

const SERDE_PROGRAM: &str = r#"
from std::serde::derive use { @json_schema }

@json_schema()
type User {
    @description("Unique identifier") id: int,
    name: string,
    email: string?,
}

print(User_json_schema())
"#;

const SERDE_EXPECTED: &str = r#"{"type": "object", "title": "User", "properties": {"id": {"type": "integer", "description": "Unique identifier"}, "name": {"type": "string"}, "email": {"type": "string"}}, "required": ["id", "name"]}"#;

#[test]
fn json_schema_derives_schema_via_stdlib_import_vm() {
    ShapeTest::new(SERDE_PROGRAM)
        .with_stdlib()
        .expect_output(SERDE_EXPECTED);
}

#[test]
fn json_schema_derives_schema_via_stdlib_import_jit() {
    ShapeTest::new(SERDE_PROGRAM)
        .with_stdlib()
        .with_jit()
        .expect_output(SERDE_EXPECTED);
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
print(User_to_json(u))
"#;

const TO_JSON_EXPECTED: &str = r#"{ "id": 1, "name": "Ada" }"#;

#[test]
fn to_json_serializes_via_stdlib_import_vm() {
    ShapeTest::new(TO_JSON_PROGRAM)
        .with_stdlib()
        .expect_output(TO_JSON_EXPECTED);
}

#[test]
fn to_json_serializes_via_stdlib_import_jit() {
    ShapeTest::new(TO_JSON_PROGRAM)
        .with_stdlib()
        .with_jit()
        .expect_output(TO_JSON_EXPECTED);
}

const LLM_PROGRAM: &str = r#"
from std::llm::tools use { @llm_tool }

/// Get current weather for a city
@llm_tool("Get current weather for a city")
fn get_weather(city: string, units: string) -> string {
    f"\{\"city\": \"{city}\", \"temp_c\": 21\}"
}

print(get_weather_tool_def())
"#;

const LLM_EXPECTED: &str = r#"{"name": "get_weather", "description": "Get current weather for a city", "parameters": {"type": "object", "properties": {"city": {"type": "string"}, "units": {"type": "string"}}, "required": ["city", "units"]}}"#;

#[test]
fn llm_tool_derives_schema_via_stdlib_import_vm() {
    ShapeTest::new(LLM_PROGRAM)
        .with_stdlib()
        .expect_output(LLM_EXPECTED);
}

#[test]
fn llm_tool_derives_schema_via_stdlib_import_jit() {
    ShapeTest::new(LLM_PROGRAM)
        .with_stdlib()
        .with_jit()
        .expect_output(LLM_EXPECTED);
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
