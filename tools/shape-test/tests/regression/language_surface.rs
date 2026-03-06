//! Cross-cutting language regressions that must stay fixed.
//! These tests intentionally cover runtime + LSP expectations.
//!
//! Also includes parser error recovery tests that verify LSP features
//! (semantic tokens, completions, etc.) continue to function when the
//! source code contains syntax errors.

use shape_lsp::inlay_hints::InlayHintConfig;
use shape_test::shape_test::{ShapeTest, pos};

// =========================================================================
// Language Surface Regressions
// =========================================================================

#[test]
fn default_function_parameters_are_optional_at_callsite() {
    let code = r#"
fn add(x: int = 1, y: int = 2) -> int {
  return x + y
}
print(add())
print(add(5))
print(add(5, 6))
"#;

    ShapeTest::new(code)
        .expect_no_semantic_diagnostics()
        .expect_output("3\n7\n11");
}

#[test]
fn comptime_type_info_is_removed_and_not_suggested_by_lsp() {
    let code = r#"
comptime {
  let info = type_info("Point")
}
"#;

    ShapeTest::new(code)
        .expect_semantic_diagnostic_contains("type_info has been removed")
        .at(pos(2, 12))
        .expect_no_completion("type_info");
}

#[test]
fn trait_bound_method_dispatch_resolves_at_runtime() {
    let code = r#"
trait Displayable {
  display(): string
}

type User { name: string }

impl Displayable for User {
  method display() { "user:" + self.name }
}

fn render<T: Displayable>(value: T) -> string {
  return value.display()
}

print(render(User { name: "Ada" }))
"#;

    ShapeTest::new(code)
        .expect_no_semantic_diagnostics()
        .expect_output("user:Ada");
}

#[test]
fn expression_annotation_before_after_hooks_execute() {
    let code = r#"
annotation trace_expr() {
  targets: [expression]
  before(args, ctx) {
    print("before")
    args
  }
  after(args, result, ctx) {
    print("after")
    result
  }
}

let x = @trace_expr() (1 + 2)
print(x)
"#;

    ShapeTest::new(code)
        .expect_no_semantic_diagnostics()
        .expect_output("before\nafter\n3");
}

#[test]
fn top_level_await_executes_in_scripts() {
    let code = r#"
async fn one() {
  1
}

let x = await one()
print(x)
"#;

    ShapeTest::new(code)
        .expect_no_semantic_diagnostics()
        .expect_output("1");
}

#[test]
fn declared_result_return_type_accepts_err_context_without_spurious_generic_mismatch() {
    let code = r#"
fn test() -> Result<int> {
  return Err("some error") !! "yes, something went wrong"
}

test()?
"#;

    ShapeTest::new(code)
        .expect_no_semantic_diagnostics()
        .expect_run_err_contains("yes, something went wrong");
}

#[test]
fn generic_struct_field_access_has_no_lsp_semantic_error() {
    let code = r#"
type MyType<T:int> {
  x: T
}

let a = MyType { x: 1.0 }
let b = MyType { x: 1 }

print(a.x)
print(b.x)
"#;

    ShapeTest::new(code).expect_no_semantic_diagnostics();
}

// =========================================================================
// Error Recovery
// =========================================================================

// -- Semantic tokens on broken code -------------------------------------------

#[test]
fn semantic_tokens_on_broken_function() {
    // Missing closing brace -- should still highlight keywords
    ShapeTest::new("function foo(x) {\n    let y = x +\n")
        .expect_semantic_tokens()
        .expect_semantic_tokens_min(2); // at least "function" and "let"
}

#[test]
fn semantic_tokens_incomplete_enum() {
    // enum without braces -- should highlight "enum" and "Color"
    ShapeTest::new("enum Color")
        .expect_semantic_tokens()
        .expect_semantic_tokens_min(1);
}

#[test]
fn semantic_tokens_missing_semicolon() {
    // Missing semicolons -- tokens should still be produced for valid parts
    ShapeTest::new("let x = 42\nlet y = 10\n")
        .expect_semantic_tokens()
        .expect_semantic_tokens_min(2); // at least two variable tokens
}

#[test]
fn semantic_tokens_unclosed_string() {
    // Unclosed string literal -- preceding tokens should still be highlighted
    ShapeTest::new("let x = \"hello\nlet y = 10\n")
        .expect_semantic_tokens()
        .expect_semantic_tokens_min(1);
}

// -- Completions on broken code -----------------------------------------------

#[test]
fn completions_on_broken_code() {
    // Previous good code, then broken code
    ShapeTest::new("let x = 42;\nfunction foo( {}\nlet y = x")
        .at(pos(2, 9))
        .expect_completions_not_empty();
}

#[test]
fn completions_after_incomplete_expression() {
    // Incomplete expression on previous line
    ShapeTest::new("let x = 42;\nlet z = x +\nlet y = x")
        .at(pos(2, 9))
        .expect_completions_not_empty();
}

// -- Parsing partial programs -------------------------------------------------

#[test]
fn valid_items_parsed_despite_broken_neighbor() {
    // One broken function shouldn't prevent parsing the other
    ShapeTest::new(
        "function good() { return 1; }\nfunction bad( {{{}\nfunction also_good() { return 2; }",
    )
    .expect_semantic_tokens()
    .expect_semantic_tokens_min(3); // at least the function keywords
}

#[test]
fn hover_works_before_syntax_error() {
    // Hover on a variable declared before the error
    ShapeTest::new("let x = 42;\nfunction broken( {}\n")
        .at(pos(0, 4))
        .expect_hover_contains("x");
}

#[test]
fn definition_works_before_syntax_error() {
    // Go-to-definition should work for symbols declared before the error
    ShapeTest::new("let x = 42;\nlet y = x + 1;\nfunction broken( {}\n")
        .at(pos(1, 8))
        .expect_definition();
}

// -- Mixed valid/invalid code -------------------------------------------------

#[test]
fn semantic_tokens_with_trailing_garbage() {
    // Valid code followed by unparseable garbage
    ShapeTest::new("let x = 42;\n@@@\n")
        .expect_semantic_tokens()
        .expect_semantic_tokens_min(1);
}

#[test]
fn parse_error_on_completely_broken_code() {
    // Completely broken code should produce a parse error
    ShapeTest::new("let = ;").expect_parse_err();
}

#[test]
fn inlay_hints_disabled_when_no_recoverable_ast() {
    // Invalid import syntax with no recoverable AST should not emit inlay hints.
    let code = r#"
from std/core/snapshot import { Snapshot }

let x = {x: 1}
x.y = 1
let i = 10D
"#;
    ShapeTest::new(code).expect_no_inlay_hints_with_config(&InlayHintConfig::default());
}
