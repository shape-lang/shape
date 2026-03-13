//! LSP presentation tests: semantic tokens, inlay hints, document symbols,
//! code lens, and type hint labels.

use shape_lsp::inlay_hints::InlayHintConfig;
use shape_test::shape_test::{ShapeTest, pos};

// == Semantic tokens (from lsp_presentation) =================================

#[test]
fn semantic_tokens_for_function_definition() {
    let code = "function foo(a, b) {\n    return a + b;\n}\n";
    ShapeTest::new(code)
        .expect_semantic_tokens()
        .expect_semantic_tokens_min(2);
}

#[test]
fn semantic_tokens_for_fn_keyword() {
    let code = "fn foo(a, b) {\n    return a + b;\n}\n";
    ShapeTest::new(code)
        .expect_semantic_tokens()
        .expect_semantic_tokens_min(2);
}

#[test]
fn semantic_tokens_for_variables() {
    ShapeTest::new("let x = 42;\nprint(\"hello\");\n").expect_semantic_tokens();
}

#[test]
fn semantic_tokens_fstring_has_multiple_segments() {
    let code = "let x = 42\nlet s = f\"value: {x}\"\n";
    ShapeTest::new(code)
        .expect_semantic_tokens()
        .expect_semantic_tokens_min(4);
}

#[test]
fn semantic_tokens_fstring_with_function_call() {
    let code = "fn foo(a) { return a }\nlet s = f\"result: {foo(1)}\"\n";
    ShapeTest::new(code)
        .expect_semantic_tokens()
        .expect_semantic_tokens_min(5);
}

#[test]
fn semantic_tokens_for_trait_definition() {
    let code = "trait Queryable {\n    filter(pred): any;\n    select(cols): any\n}\n";
    ShapeTest::new(code)
        .expect_semantic_tokens()
        .expect_semantic_tokens_min(2);
}

#[test]
fn semantic_tokens_for_impl_block() {
    let code = "trait Queryable {\n    filter(pred): any\n}\nimpl Queryable for MyTable {\n    method filter(pred) { self }\n}\n";
    ShapeTest::new(code)
        .expect_semantic_tokens()
        .expect_semantic_tokens_min(6);
}

// == Semantic tokens (from lsp_new_features) =================================

#[test]
fn semantic_tokens_for_let_declaration() {
    ShapeTest::new("let x = 1;")
        .expect_semantic_tokens()
        .expect_semantic_tokens_min(2);
}

#[test]
fn semantic_tokens_for_var_declaration() {
    ShapeTest::new("let mut y = 2;")
        .expect_semantic_tokens()
        .expect_semantic_tokens_min(2);
}

#[test]
fn semantic_tokens_distinguish_let_var() {
    ShapeTest::new("let x = 1;\nlet mut y = 2;")
        .expect_semantic_tokens()
        .expect_semantic_tokens_min(4);
}

#[test]
fn semantic_tokens_for_const_declaration() {
    ShapeTest::new("const PI = 3.14;")
        .expect_semantic_tokens()
        .expect_semantic_tokens_min(2);
}

#[test]
fn semantic_tokens_for_if_else() {
    ShapeTest::new("if (true) { 1 } else { 2 }")
        .expect_semantic_tokens()
        .expect_semantic_tokens_min(2);
}

#[test]
fn semantic_tokens_for_while_loop() {
    ShapeTest::new("let mut i = 0;\nwhile (i < 10) { i = i + 1; }")
        .expect_semantic_tokens()
        .expect_semantic_tokens_min(3);
}

#[test]
fn semantic_tokens_for_for_loop() {
    ShapeTest::new("for (let mut i = 0; i < 10; i = i + 1) { print(i); }")
        .expect_semantic_tokens()
        .expect_semantic_tokens_min(2);
}

// == Semantic tokens: comptime (from lsp_comptime) ===========================

#[test]
fn semantic_tokens_comptime_keyword() {
    let code = "type Currency {\n    comptime symbol: string = \"$\",\n    amount: number\n}\n";
    ShapeTest::new(code)
        .expect_semantic_tokens()
        .expect_semantic_tokens_min(2);
}

#[test]
fn semantic_tokens_for_comptime_block() {
    let code = "comptime {\n    let x = 42\n}\n";
    ShapeTest::new(code)
        .expect_semantic_tokens()
        .expect_semantic_tokens_min(1);
}

#[test]
fn semantic_tokens_for_annotation_comptime_directives() {
    let code = r#"
annotation drop_expr() {
    targets: [expression]
    comptime post(target, ctx) {
        remove target
    }
}

annotation add_sum() {
    targets: [type]
    comptime post(target, ctx) {
        extend target {
            method sum() { self.x + self.y }
        }
    }
}
"#;
    ShapeTest::new(code)
        .expect_semantic_tokens()
        .expect_semantic_tokens_min(6);
}

// == Inlay hints (from lsp_presentation) =====================================

#[test]
fn inlay_hint_for_integer_literal() {
    ShapeTest::new("let i = 10\n").expect_inlay_hints_not_empty();
}

#[test]
fn inlay_hint_for_decimal_literal() {
    ShapeTest::new("let d = 10D\n").expect_inlay_hints_not_empty();
}

#[test]
fn inlay_hint_for_float_literal() {
    ShapeTest::new("let f = 10.0\n").expect_inlay_hints_not_empty();
}

#[test]
fn inlay_hints_disabled_by_config_returns_empty() {
    let config = InlayHintConfig {
        show_type_hints: false,
        show_parameter_hints: false,
        show_variable_type_hints: false,
        show_return_type_hints: false,
    };
    ShapeTest::new("let x = 42;\nlet y = 10;\n").expect_no_inlay_hints_with_config(&config);
}

#[test]
fn parameter_hint_property_access_position() {
    ShapeTest::new("let a = { x: 10 }\nabs(a.x)").expect_parameter_hint_at(pos(1, 4));
}

#[test]
fn object_type_hint_with_property_assignment() {
    ShapeTest::new("let a = { x: 10 }\na.y = 2")
        .expect_type_hint_label(": { x: int /*, y: int */ }");
}

#[test]
fn match_union_type_hint_deduplicates() {
    ShapeTest::new("let m = match 2 {\n  0 => true,\n  1 => false,\n  2 => \"test\",\n}")
        .expect_type_hint_label(": bool | string");
}

#[test]
fn match_type_hint_persists_with_print() {
    ShapeTest::new("let m = match 2 {\n  0 => true,\n  1 => false,\n  2 => \"test\",\n}\nprint(m)")
        .expect_type_hint_label(": bool | string");
}

// == Function return type inlay hints (from lsp_presentation) ================

#[test]
fn inlay_hint_function_return_type() {
    let code = "fn add(a: int, b: int) {\n  return a + b\n}\n";
    ShapeTest::new(code).expect_type_hint_label("-> int");
}

#[test]
fn inlay_hint_no_return_type_when_annotated() {
    let code = "fn double(x: int) -> int {\n  return x * 2\n}\n";
    ShapeTest::new(code).expect_no_type_hint_label("-> int");
}

#[test]
fn inlay_hint_function_bool_return() {
    let code = "fn is_positive(x: number) {\n  return x > 0\n}\n";
    ShapeTest::new(code).expect_type_hint_label("-> bool");
}

#[test]
fn inlay_hint_function_string_return() {
    let code = "fn greet() {\n  return \"hello\"\n}\n";
    ShapeTest::new(code).expect_type_hint_label("-> string");
}

// == Inlay hint type labels (from lsp_type_display) ==========================

#[test]
fn inlay_hint_for_int_variable() {
    ShapeTest::new("let x = 42;").expect_type_hint_label(": int");
}

#[test]
fn inlay_hint_for_string_variable() {
    ShapeTest::new("let name = \"hello\";").expect_type_hint_label(": string");
}

#[test]
fn inlay_hint_for_bool_variable() {
    ShapeTest::new("let flag = true;").expect_type_hint_label(": bool");
}

#[test]
fn no_inlay_hint_when_explicit_annotation() {
    ShapeTest::new("let x: number = 42;").expect_no_type_hint_label(": number");
}

#[test]
fn inlay_hint_for_float_variable() {
    ShapeTest::new("let f = 3.14;").expect_type_hint_label(": number");
}

#[test]
fn inlay_hint_for_decimal_variable() {
    ShapeTest::new("let d = 10D;").expect_inlay_hints_not_empty();
}

#[test]
fn inlay_hint_masks_hoisted_fields_before_assignment() {
    let code = "let x = { x: 1 }\nx.y = 1\n";
    ShapeTest::new(code).expect_type_hint_label(": { x: int /*, y: int */ }");
}

#[test]
fn inlay_hints_for_multiple_variables() {
    ShapeTest::new("let x = 42;\nlet name = \"hello\";\nlet flag = true;\n")
        .expect_type_hint_label(": int")
        .expect_type_hint_label(": string")
        .expect_type_hint_label(": bool");
}

// == Function return type hints (from lsp_type_display) ======================

#[test]
fn inlay_hint_function_return_number() {
    let code = "fn add(a: int, b: int) {\n  return a + b\n}\n";
    ShapeTest::new(code).expect_type_hint_label("-> int");
}

#[test]
fn inlay_hint_function_return_string() {
    let code = "fn greet() {\n  return \"hello\"\n}\n";
    ShapeTest::new(code).expect_type_hint_label("-> string");
}

#[test]
fn inlay_hint_function_return_bool() {
    let code = "fn is_positive(x: number) {\n  return x > 0\n}\n";
    ShapeTest::new(code).expect_type_hint_label("-> bool");
}

#[test]
fn inlay_hint_function_return_union_from_multiple_returns() {
    let code = "fn afunc(c) {\n  return 1\n  return \"hi\"\n}\n";
    ShapeTest::new(code)
        .expect_type_hint_label("-> int | string")
        .at(pos(0, 4))
        .expect_hover_contains("-> int | string");
}

#[test]
fn inlay_hint_function_return_result_from_expression_style_ok_err() {
    let code = "fn test() {\n  Ok(1)\n  Err(\"some error\")\n}\n";
    ShapeTest::new(code)
        .expect_type_hint_label("-> Result<int>")
        .at(pos(0, 4))
        .expect_hover_contains("-> Result<int>");
}

#[test]
fn inlay_hint_function_return_result_union_for_mixed_ok_values() {
    let code = "fn test() {\n  Ok(1)\n  Ok(\"str\")\n}\n";
    ShapeTest::new(code)
        .expect_type_hint_label("-> Result<int | string>")
        .at(pos(0, 4))
        .expect_hover_contains("-> Result<int | string>");
}

#[test]
fn inlay_hint_try_operator_unwraps_ok_constructor_inner_type() {
    let code = "fn test() {\n  let r = Ok(1)?\n}\n";
    ShapeTest::new(code).expect_type_hint_label(": int");
}

#[test]
fn inlay_hint_struct_generic_argument_is_inferred_from_literal() {
    let code = "type MyType<T = int> { x: T }\nlet a = MyType { x: 1.0 }\n";
    ShapeTest::new(code)
        .expect_type_hint_label(": MyType<number>")
        .at(pos(1, 4))
        .expect_hover_contains("MyType<number>");
}

#[test]
fn inlay_hint_struct_generic_default_hides_default_argument() {
    let code = "type MyType<T = int> { x: T }\nlet a = MyType { x: 1 }\n";
    ShapeTest::new(code)
        .expect_type_hint_label(": MyType")
        .at(pos(1, 4))
        .expect_hover_contains("MyType")
        .expect_hover_not_contains("MyType<int>");
}

#[test]
fn no_return_hint_when_annotated() {
    let code = "fn double(x: int) -> int {\n  return x * 2\n}\n";
    ShapeTest::new(code).expect_no_type_hint_label("-> int");
}

#[test]
fn inlay_hint_keeps_object_merge_type_with_semantic_error_elsewhere() {
    let code = "let a = { x: 1 }\nlet b = { z: 3 }\nprint(a.y)\na.y = 2\nlet c = a + b\n";
    ShapeTest::new(code).expect_type_hint_label(": { x: int, y: int, z: int }");
}

#[test]
fn inlay_hint_closure_param_is_refined_from_body_constraints() {
    let code = "let x = { x: 1 }\nlet y = | x | 10 * (x.x * 2)\nprint(y(x))\n";
    ShapeTest::new(code).expect_type_hint_label(": ({ x: number }) -> number");
}

#[test]
fn inlay_hint_for_match_expression_local_result() {
    let code = r#"
fn afunc(c) {
  let result = match c {
    c: int => c + 1
    _ => 1
  }
  return result
}
"#;

    ShapeTest::new(code).expect_type_hint_label(": int");
}

// == Document symbols (from lsp_presentation) ================================

#[test]
fn document_symbols_for_mixed_declarations() {
    let code = "let myVar = 5;\n\nfunction myFunc(x, y) {\n    return x + y;\n}\n";
    ShapeTest::new(code).expect_document_symbols();
}

#[test]
fn document_symbols_for_empty_file() {
    ShapeTest::new("").expect_no_document_symbols();
}

// == Document symbols (from lsp_new_features) ================================

#[test]
fn document_symbols_for_function() {
    ShapeTest::new("function foo() { return 1; }").expect_document_symbols();
}

#[test]
fn document_symbols_for_multiple_items() {
    let code = "let x = 1;\nfunction foo() { return 1; }\nfunction bar() { return 2; }\n";
    ShapeTest::new(code).expect_document_symbols();
}

#[test]
fn document_symbols_for_type_and_function() {
    ShapeTest::new(
        "type Point { x: number, y: number }\nfunction origin() { return Point { x: 0, y: 0 }; }",
    )
    .expect_document_symbols();
}

#[test]
fn document_symbols_for_enum() {
    ShapeTest::new("enum Color { Red, Green, Blue }").expect_document_symbols();
}

// == Folding ranges (from lsp_new_features) ==================================

#[test]
fn folding_range_function_parses() {
    ShapeTest::new("function foo() {\n    let x = 1;\n    return x;\n}").expect_parse_ok();
}

#[test]
fn folding_range_nested_blocks_parse() {
    let code = "function foo() {\n    if (true) {\n        let x = 1;\n    }\n    return 0;\n}";
    ShapeTest::new(code).expect_parse_ok();
}

// == Code lens (from lsp_new_features) =======================================

#[test]
fn code_lens_on_single_function() {
    let code = "function myFunc() {\n    return 1;\n}\nlet a = myFunc();\n";
    ShapeTest::new(code)
        .expect_code_lens_not_empty()
        .expect_code_lens_at_line(0);
}

#[test]
fn code_lens_on_multiple_functions() {
    let code = "function foo() { return 1; }\nfunction bar() { return foo(); }\nlet x = bar();\n";
    ShapeTest::new(code).expect_code_lens_not_empty();
}

// == Code lens: traits (from lsp_presentation) ===============================

#[test]
fn code_lens_on_trait_shows_implementations() {
    let code = "trait Queryable {\n    filter(pred): any\n}\nimpl Queryable for MyTable {\n    method filter(pred) { self }\n}\n";
    ShapeTest::new(code).expect_code_lens_not_empty();
}

#[test]
fn code_lens_on_trait_at_correct_line() {
    let code = "trait Queryable {\n    filter(pred): any\n}\nimpl Queryable for MyTable {\n    method filter(pred) { self }\n}\n";
    ShapeTest::new(code).expect_code_lens_at_line(0);
}

#[test]
fn definition_from_format_navigates_to_display_impl() {
    let code = "trait Display {\n    format(self): string\n}\nimpl Display for MyType {\n    method format() {\n        return \"test\"\n    }\n}\nlet x = MyType {}\nx.format()\n";
    ShapeTest::new(code).at(pos(9, 2)).expect_definition();
}

// == Inlay hints deep (from programs_lsp_completeness) =======================

#[test]
fn test_lsp_inlay_int_variable() {
    ShapeTest::new("let x = 42").expect_type_hint_label(": int");
}

#[test]
fn test_lsp_inlay_number_variable() {
    ShapeTest::new("let f = 3.14").expect_type_hint_label(": number");
}

#[test]
fn test_lsp_inlay_string_variable() {
    ShapeTest::new("let s = \"world\"").expect_type_hint_label(": string");
}

#[test]
fn test_lsp_inlay_bool_variable() {
    ShapeTest::new("let b = false").expect_type_hint_label(": bool");
}

#[test]
fn test_lsp_inlay_decimal_variable() {
    ShapeTest::new("let d = 99D").expect_inlay_hints_not_empty();
}

#[test]
fn test_lsp_inlay_function_return_int() {
    let code = "fn square(x: int) {\n  return x * x\n}\n";
    ShapeTest::new(code).expect_type_hint_label("-> int");
}

#[test]
fn test_lsp_inlay_function_return_string() {
    let code = "fn greet(name: string) {\n  return \"Hello, \" + name\n}\n";
    ShapeTest::new(code).expect_type_hint_label("-> string");
}

#[test]
fn test_lsp_inlay_function_return_bool() {
    let code = "fn is_even(n: int) {\n  return n % 2 == 0\n}\n";
    ShapeTest::new(code).expect_type_hint_label("-> bool");
}

#[test]
fn test_lsp_inlay_no_hint_when_type_annotated() {
    ShapeTest::new("let x: int = 42").expect_no_type_hint_label(": int");
}

#[test]
fn test_lsp_inlay_no_return_hint_when_annotated() {
    let code = "fn double(x: int) -> int {\n  return x * 2\n}\n";
    ShapeTest::new(code).expect_no_type_hint_label("-> int");
}

#[test]
fn test_lsp_inlay_object_shape() {
    ShapeTest::new("let o = { name: \"test\" }").expect_type_hint_label(": { name: string }");
}

#[test]
fn test_lsp_inlay_multiple_variables_each_typed() {
    ShapeTest::new("let a = 1\nlet b = \"hi\"\nlet c = true\n")
        .expect_type_hint_label(": int")
        .expect_type_hint_label(": string")
        .expect_type_hint_label(": bool");
}

#[test]
fn test_lsp_inlay_closure_type() {
    let code = "let obj = { x: 1 }\nlet f = |o| 10 * (o.x * 2)\nprint(f(obj))\n";
    ShapeTest::new(code).expect_inlay_hints_not_empty();
}

#[test]
fn test_lsp_inlay_config_all_disabled() {
    let config = InlayHintConfig {
        show_type_hints: false,
        show_parameter_hints: false,
        show_variable_type_hints: false,
        show_return_type_hints: false,
    };
    ShapeTest::new("let x = 42\nfn add(a: int, b: int) { return a + b }\n")
        .expect_no_inlay_hints_with_config(&config);
}

#[test]
fn test_lsp_inlay_config_only_return_disabled() {
    let config = InlayHintConfig {
        show_type_hints: true,
        show_parameter_hints: true,
        show_variable_type_hints: true,
        show_return_type_hints: false,
    };
    let code = "fn add(a: int, b: int) {\n  return a + b\n}\n";
    ShapeTest::new(code).expect_no_inlay_hints_with_config(&config);
}

#[test]
fn test_lsp_inlay_config_only_variable_disabled() {
    let code = "let x = 42\nfn negate(a: int) {\n  return -a\n}\n";
    let config = InlayHintConfig {
        show_type_hints: true,
        show_parameter_hints: true,
        show_variable_type_hints: false,
        show_return_type_hints: false,
    };
    ShapeTest::new(code).expect_no_inlay_hints_with_config(&config);
}

#[test]
fn test_lsp_inlay_variable_inside_function_body() {
    let code = "fn test() {\n  let x = 42\n  return x\n}\n";
    ShapeTest::new(code).expect_type_hint_label(": int");
}

#[test]
fn test_lsp_inlay_for_loop_variable() {
    let code = "let items = [1, 2, 3]\nfor item in items {\n  print(item)\n}\n";
    ShapeTest::new(code).expect_inlay_hints_not_empty();
}

#[test]
fn test_lsp_inlay_union_return_type() {
    let code = "fn maybe(x: int) {\n  return 1\n  return \"nope\"\n}\n";
    ShapeTest::new(code).expect_type_hint_label("-> int | string");
}

#[test]
fn test_lsp_inlay_result_return_type() {
    let code = "fn safe() {\n  Ok(42)\n  Err(\"fail\")\n}\n";
    ShapeTest::new(code).expect_type_hint_label("-> Result<int>");
}

// == Semantic tokens & document symbols deep (from programs_lsp_completeness) =

#[test]
fn test_lsp_semantic_tokens_for_complete_program() {
    let code =
        "let x = 42\nfn add(a: int, b: int) -> int {\n  return a + b\n}\nlet result = add(x, 10)\n";
    ShapeTest::new(code)
        .expect_semantic_tokens()
        .expect_semantic_tokens_min(5);
}

#[test]
fn test_lsp_document_symbols_mixed_declarations() {
    let code =
        "type Config { debug: bool }\nfn init() { Config { debug: false } }\nlet c = init()\n";
    ShapeTest::new(code).expect_document_symbols();
}

#[test]
fn test_lsp_document_symbols_empty_file() {
    ShapeTest::new("").expect_no_document_symbols();
}

#[test]
fn test_lsp_semantic_tokens_enum_definition() {
    let code = "enum Status {\n  Active,\n  Inactive\n}\n";
    ShapeTest::new(code)
        .expect_semantic_tokens()
        .expect_semantic_tokens_min(2);
}

#[test]
fn test_lsp_code_lens_on_trait() {
    let code = "trait Serializable {\n  serialize(): string\n}\nimpl Serializable for Data {\n  method serialize() { \"{}\" }\n}\n";
    ShapeTest::new(code).expect_code_lens_not_empty();
}
