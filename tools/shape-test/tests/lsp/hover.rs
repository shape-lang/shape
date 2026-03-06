//! LSP hover tests: hover information display, type inference in hover,
//! module hover, trait/impl hover, comptime hover, and type display hover.

use shape_test::shape_test::{ShapeTest, pos};

// == Core hover (from lsp_analysis) ==========================================

#[test]
fn hover_on_user_defined_function_shows_signature() {
    let code =
        "/// Calculates the sum\nfunction mySum(a, b) { return a + b; }\nlet x = mySum(1, 2);\n";
    ShapeTest::new(code)
        .at(pos(1, 9))
        .expect_hover_contains("mySum");
}

#[test]
fn hover_on_type_keyword_shows_info() {
    ShapeTest::new("type MyType { i: int, name: string }")
        .at(pos(0, 0))
        .expect_hover_contains("type");
}

#[test]
fn hover_on_property_access_shows_property_type() {
    let code =
        "type MyType { i: int, name: string }\nlet b = MyType { i: 10, name: \"hello\" }\nb.i\n";
    ShapeTest::new(code)
        .at(pos(2, 2))
        .expect_hover_contains("Property");
}

#[test]
fn hover_on_keyword_shows_keyword_docs() {
    ShapeTest::new("let x = 42;")
        .at(pos(0, 1))
        .expect_hover_contains("let");
}

#[test]
fn hover_on_empty_space_returns_none() {
    ShapeTest::new("let x = 42;\n\n")
        .at(pos(1, 0))
        .expect_no_hover();
}

#[test]
fn hover_on_doc_comment_function() {
    let code = "/// Calculates the sum\nfunction mySum(a, b) { return a + b; }\n";
    ShapeTest::new(code)
        .at(pos(1, 12))
        .expect_hover_contains("Calculates the sum");
}

#[test]
fn hover_on_annotated_function_shows_annotations_generically() {
    let code = "@strategy\nfn my_strat(data) {\n  return data\n}\n";
    ShapeTest::new(code)
        .at(pos(1, 5))
        .expect_hover_contains("my_strat");
}

#[test]
fn hover_on_module_name_shows_description() {
    let code = "mod csv { fn load(path: string) { path } }\ncsv\n";
    ShapeTest::new(code)
        .at(pos(1, 1))
        .expect_hover_contains("csv");
}

#[test]
fn hover_on_module_function_shows_signature() {
    let code = "mod csv { fn load(path: string) { path } }\nlet df = csv.load(\"/tmp/test.csv\")\n";
    ShapeTest::new(code)
        .at(pos(1, 14))
        .expect_hover_contains("load");
}

// == Comptime hover (from lsp_comptime) ======================================

#[test]
fn hover_comptime_field_in_struct_definition() {
    let code = "type Currency {\n    comptime symbol: string = \"$\",\n    amount: number\n}\n";
    ShapeTest::new(code)
        .at(pos(1, 14))
        .expect_hover_contains("Comptime Field");
}

#[test]
fn hover_comptime_field_in_type_alias_override() {
    let code = "type Currency {\n    comptime symbol: string = \"$\",\n    amount: number\n}\ntype EUR = Currency { symbol: \"EUR\" }\n";
    ShapeTest::new(code)
        .at(pos(4, 24))
        .expect_hover_contains("Comptime Field");
}

#[test]
fn hover_comptime_block_shows_builtin_info() {
    let code = "comptime {\n    let has = implements(\"Foo\", \"Display\")\n}\n";
    ShapeTest::new(code)
        .at(pos(0, 4))
        .expect_hover_contains("Compile-Time Block");
}

// == Type inference in hover (from lsp_type_display) =========================

#[test]
fn hover_shows_int_type_for_integer() {
    ShapeTest::new("let x = 42;")
        .at(pos(0, 4))
        .expect_hover_contains("int");
}

#[test]
fn hover_shows_number_type_for_float() {
    ShapeTest::new("let x = 3.14;")
        .at(pos(0, 4))
        .expect_hover_contains("number");
}

#[test]
fn hover_shows_string_type() {
    ShapeTest::new("let name = \"hello\";")
        .at(pos(0, 4))
        .expect_hover_contains("string");
}

#[test]
fn hover_shows_bool_type() {
    ShapeTest::new("let flag = true;")
        .at(pos(0, 4))
        .expect_hover_contains("bool");
}

#[test]
fn hover_shows_function_info() {
    ShapeTest::new("function add(a, b) { return a + b; }")
        .at(pos(0, 9))
        .expect_hover_contains("add");
}

#[test]
fn hover_shows_variable_keyword_info() {
    ShapeTest::new("let x = 42;")
        .at(pos(0, 4))
        .expect_hover_contains("Variable");
}

#[test]
fn hover_on_const_shows_constant_info() {
    ShapeTest::new("const PI = 3.14;")
        .at(pos(0, 6))
        .expect_hover_contains("PI");
}

#[test]
fn hover_on_expression_result() {
    ShapeTest::new("let x = 1 + 2;")
        .at(pos(0, 4))
        .expect_hover_contains("Variable");
}

#[test]
fn hover_masks_hoisted_fields_before_assignment() {
    let code = "let a = { x: 1 }\na.y = 2\n";
    ShapeTest::new(code)
        .at(pos(0, 4))
        .expect_hover_contains("x: int")
        .expect_hover_contains("/*, y: int */")
        .expect_hover_not_contains("x: int, y: int");
}

#[test]
fn hover_shows_full_shape_after_assignment_site() {
    let code = "let a = { x: 1 }\na.y = 2\n";
    ShapeTest::new(code)
        .at(pos(1, 0))
        .expect_hover_contains("x: int")
        .expect_hover_contains("y: int")
        .expect_hover_not_contains("/*, y: int */");
}

#[test]
fn hover_on_hoisted_property_access_shows_field_type() {
    let code = "let a = { x: 1 }\na.y = 2\nprint(a.y)\n";
    ShapeTest::new(code)
        .at(pos(2, 8))
        .expect_hover_contains("Property")
        .expect_hover_contains("int");
}

// == Hover: union params, match scrutinee (from lsp_type_display) ============

#[test]
fn hover_shows_union_for_unannotated_param_from_mixed_callsites() {
    let code = "fn foo(a) {\n  return a\n}\nlet i = foo(1)\nlet s = foo(\"hi\")\n";
    ShapeTest::new(code)
        .at(pos(0, 4))
        .expect_hover_contains("a: int | &string")
        .expect_hover_contains("-> int | string");
}

#[test]
fn hover_signature_preserves_callsite_union_under_numeric_conflict() {
    let code = r#"
fn afunc(c) {
  print("func called with " + c)
  c = c + 1
  return c
  return "hi"
}
let x = { x: 1, y: 2 }
print(afunc(x))
print(afunc(1))
"#;

    ShapeTest::new(code)
        .expect_semantic_diagnostic_contains("Could not solve type constraints")
        .expect_semantic_diagnostic_at_line_contains(3, "Could not solve type constraints")
        .at(pos(1, 4))
        .expect_hover_contains("{ x: int, y: int } | int")
        .expect_hover_contains("string")
        .expect_hover_not_contains("c: number")
        .expect_hover_not_contains("-> number");
}

#[test]
fn hover_mixed_callsite_return_union_does_not_degrade_to_any() {
    let code = r#"
fn afunc(c) {
  print("func called with " + c)
  return c
  return "hi"
}

let x = { x: 1, y: 2 }
print(afunc(x))
print(afunc(1))
"#;

    ShapeTest::new(code)
        .at(pos(1, 4))
        .expect_hover_contains("c:")
        .expect_hover_contains("x: int")
        .expect_hover_contains("int")
        .expect_hover_contains("string")
        .expect_hover_not_contains("any");
}

#[test]
fn hover_typed_match_pattern_shadows_outer_binding() {
    let code = r#"
let c = "outer"
fn afunc(c) {
  let result = match c {
    c: int => c + 1
    _ => 1
  }
  return result
}
"#;

    ShapeTest::new(code)
        .at(pos(4, 4))
        .expect_hover_contains("Variable")
        .expect_hover_contains("int")
        .expect_hover_not_contains("string")
        .at(pos(4, 14))
        .expect_hover_contains("Variable")
        .expect_hover_contains("int")
        .expect_hover_not_contains("string");
}

#[test]
fn hover_match_scrutinee_prefers_function_param_scope_over_outer_symbol() {
    let code = r#"
let c = "outer"
fn afunc(c: int) {
  let result = match c {
    c: int => c + 1
    _ => 1
  }
  return result
}
"#;

    ShapeTest::new(code)
        .at(pos(3, 21))
        .expect_hover_contains("Variable")
        .expect_hover_contains("int")
        .expect_hover_not_contains("string");
}

#[test]
fn hover_match_scrutinee_shows_annotated_union_param_type() {
    let code = r#"
let c = "outer"
fn afunc(c: { x: int, y: int } | int) {
  let result = match c {
    c: int => c + 1
    _ => 1
  }
  return result
}
"#;

    ShapeTest::new(code)
        .at(pos(3, 21))
        .expect_hover_contains("{ x: int, y: int } | int")
        .expect_hover_not_contains("Type:** `string`");
}

// == Trait/impl hover (from lsp_presentation) ================================

#[test]
fn definition_from_trait_name() {
    let code = "trait Queryable {\n    filter(pred): any\n}\nimpl Queryable for MyTable {\n    method filter(pred) { self }\n}\n";
    ShapeTest::new(code).at(pos(0, 6)).expect_definition();
}

#[test]
fn definition_from_impl_trait_name() {
    let code = "trait Queryable {\n    filter(pred): any\n}\nimpl Queryable for MyTable {\n    method filter(pred) { self }\n}\n";
    ShapeTest::new(code).at(pos(3, 5)).expect_definition();
}

#[test]
fn definition_from_impl_method_name() {
    let code = "trait Queryable {\n    filter(pred): any\n}\nimpl Queryable for MyTable {\n    method filter(pred) { self }\n}\n";
    ShapeTest::new(code).at(pos(4, 11)).expect_definition();
}

#[test]
fn hover_on_impl_method_shows_trait_signature() {
    let code = "trait Queryable {\n    runQuery(q): any\n}\nimpl Queryable for MyTable {\n    method runQuery(q) { self }\n}\n";
    ShapeTest::new(code)
        .at(pos(4, 11))
        .expect_hover_contains("Trait Method");
}

#[test]
fn hover_on_impl_trait_name_without_local_trait_definition() {
    let code = "type User { name: String }\nimpl Display for User {\n    method display() { self.name }\n}\n";
    ShapeTest::new(code)
        .at(pos(1, 6))
        .expect_hover_contains("Trait")
        .expect_hover_contains("Display")
        .expect_hover_contains("User");
}

#[test]
fn hover_on_impl_method_without_local_trait_definition() {
    let code = "type User { name: String }\nimpl Display for User {\n    method display() { self.name }\n}\n";
    ShapeTest::new(code)
        .at(pos(2, 13))
        .expect_hover_contains("Method")
        .expect_hover_contains("display")
        .expect_hover_contains("Display");
}

#[test]
fn hover_on_self_in_impl_method_shows_receiver_type() {
    let code = "type User { name: String }\nimpl Display for User {\n    method display() { self.name }\n}\n";
    ShapeTest::new(code)
        .at(pos(2, 24))
        .expect_hover_contains("Variable")
        .expect_hover_contains("self")
        .expect_hover_contains("Type:** `User`");
}

#[test]
fn hover_on_self_property_in_impl_method_shows_struct_field_type() {
    let code = "type User { name: String }\nimpl Display for User {\n    method display() { self.name }\n}\n";
    ShapeTest::new(code)
        .at(pos(2, 29))
        .expect_hover_contains("Property")
        .expect_hover_contains("String");
}

#[test]
fn hover_on_bounded_type_param_shows_traits() {
    let code = "trait Comparable {\n    compare(other): number\n}\nfn foo<T: Comparable>(x: T) {\n    x\n}\n";
    ShapeTest::new(code)
        .at(pos(3, 7))
        .expect_hover_contains("Type Parameter");
}

#[test]
fn hover_on_bounded_type_param_shows_bound_names() {
    let code = "trait Comparable {\n    compare(other): number\n}\nfn foo<T: Comparable>(x: T) {\n    x\n}\n";
    ShapeTest::new(code)
        .at(pos(3, 7))
        .expect_hover_contains("Comparable");
}

#[test]
fn hover_default_method_shows_default_indicator() {
    let code = "trait Queryable {\n    filter(pred): any;\n    method execute() {\n        return self\n    }\n}\nimpl Queryable for MyTable {\n    method filter(pred) { self }\n    method execute() { self }\n}\n";
    ShapeTest::new(code)
        .at(pos(8, 11))
        .expect_hover_contains("default");
}

#[test]
fn hover_resolves_f_and_g_after_named_intersection_destructure() {
    let code = r#"
let a = { x: 1}
let b = { z: 3}
a.y = 2
let c = a+b
type TypeA {x: int, y: int}
type TypeB {z: int}
let (f:TypeA, g: TypeB) = c as (TypeA+TypeB)
print(f, g)
"#;
    ShapeTest::new(code)
        .at(pos(8, 6))
        .expect_hover_exists()
        .expect_hover_contains("Variable")
        .at(pos(8, 9))
        .expect_hover_exists()
        .expect_hover_contains("Variable");
}

// == Hover deep (from programs_lsp_completeness) =============================

#[test]
fn test_lsp_hover_int_literal() {
    ShapeTest::new("let x = 42")
        .at(pos(0, 4))
        .expect_hover_contains("int");
}

#[test]
fn test_lsp_hover_float_literal() {
    ShapeTest::new("let x = 3.14")
        .at(pos(0, 4))
        .expect_hover_contains("number");
}

#[test]
fn test_lsp_hover_string_literal() {
    ShapeTest::new("let s = \"hello\"")
        .at(pos(0, 4))
        .expect_hover_contains("string");
}

#[test]
fn test_lsp_hover_bool_true() {
    ShapeTest::new("let b = true")
        .at(pos(0, 4))
        .expect_hover_contains("bool");
}

#[test]
fn test_lsp_hover_bool_false() {
    ShapeTest::new("let b = false")
        .at(pos(0, 4))
        .expect_hover_contains("bool");
}

#[test]
fn test_lsp_hover_array_int() {
    ShapeTest::new("let a = [1, 2, 3]")
        .at(pos(0, 4))
        .expect_hover_contains("int");
}

#[test]
fn test_lsp_hover_arithmetic_result_int() {
    ShapeTest::new("let x = 1 + 2")
        .at(pos(0, 4))
        .expect_hover_contains("int");
}

#[test]
fn test_lsp_hover_comparison_result_bool() {
    ShapeTest::new("let b = 1 < 2")
        .at(pos(0, 4))
        .expect_hover_contains("bool");
}

#[test]
fn test_lsp_hover_equality_result_bool() {
    ShapeTest::new("let b = 1 == 2")
        .at(pos(0, 4))
        .expect_hover_contains("bool");
}

#[test]
fn test_lsp_hover_logical_and_bool() {
    ShapeTest::new("let b = true and false")
        .at(pos(0, 4))
        .expect_hover_contains("bool");
}

#[test]
fn test_lsp_hover_negation_result() {
    ShapeTest::new("let b = !true")
        .at(pos(0, 4))
        .expect_hover_contains("bool");
}

#[test]
fn test_lsp_hover_function_signature() {
    let code = "fn add(a: int, b: int) -> int {\n  return a + b\n}\n";
    ShapeTest::new(code)
        .at(pos(0, 3))
        .expect_hover_contains("add");
}

#[test]
fn test_lsp_hover_function_shows_params() {
    let code = "/// Adds two numbers\nfn add(a: int, b: int) -> int {\n  return a + b\n}\n";
    ShapeTest::new(code)
        .at(pos(1, 4))
        .expect_hover_contains("add")
        .expect_hover_contains("Adds two numbers");
}

#[test]
fn test_lsp_hover_object_literal() {
    ShapeTest::new("let o = { x: 1, y: 2 }")
        .at(pos(0, 4))
        .expect_hover_contains("x: int");
}

#[test]
fn test_lsp_hover_const_variable() {
    ShapeTest::new("const PI = 3.14159")
        .at(pos(0, 6))
        .expect_hover_contains("PI");
}

#[test]
fn test_lsp_hover_variable_assignment_chain() {
    let code = "let a = 42\nlet b = a\n";
    ShapeTest::new(code)
        .at(pos(1, 4))
        .expect_hover_contains("Variable");
}

#[test]
fn test_lsp_hover_struct_literal_type() {
    let code = "type Point { x: int, y: int }\nlet p = Point { x: 1, y: 2 }\n";
    ShapeTest::new(code)
        .at(pos(1, 4))
        .expect_hover_contains("Point");
}

#[test]
fn test_lsp_hover_function_return_inferred() {
    let code = "fn double(x: int) {\n  return x * 2\n}\n";
    ShapeTest::new(code)
        .at(pos(0, 3))
        .expect_hover_contains("-> int");
}

#[test]
fn test_lsp_hover_decimal_literal() {
    ShapeTest::new("let d = 10D")
        .at(pos(0, 4))
        .expect_hover_contains("Variable");
}

#[test]
fn test_lsp_hover_on_let_keyword() {
    ShapeTest::new("let x = 42")
        .at(pos(0, 0))
        .expect_hover_contains("let");
}

#[test]
fn test_lsp_hover_on_fn_keyword() {
    ShapeTest::new("fn test() { 1 }")
        .at(pos(0, 0))
        .expect_hover_contains("fn");
}

#[test]
fn test_lsp_hover_enum_variant() {
    let code = "enum Color {\n  Red,\n  Green,\n  Blue\n}\n";
    ShapeTest::new(code)
        .at(pos(0, 5))
        .expect_hover_contains("Color");
}

#[test]
fn test_lsp_hover_match_result_type() {
    let code = "let m = match 1 {\n  0 => \"zero\"\n  _ => \"other\"\n}\n";
    ShapeTest::new(code)
        .at(pos(0, 4))
        .expect_hover_contains("string");
}

#[test]
fn test_lsp_hover_mixed_match_union_type() {
    let code = "let m = match 1 {\n  0 => true\n  _ => \"other\"\n}\n";
    ShapeTest::new(code)
        .at(pos(0, 4))
        .expect_hover_contains("bool");
}

#[test]
fn test_lsp_hover_function_multi_return_union() {
    let code = "fn dual(x: int) {\n  return 1\n  return \"hello\"\n}\n";
    ShapeTest::new(code)
        .at(pos(0, 3))
        .expect_hover_contains("int")
        .expect_hover_contains("string");
}

#[test]
fn test_lsp_hover_type_definition_shows_fields() {
    let code = "type Record { id: int, value: string }\n";
    ShapeTest::new(code)
        .at(pos(0, 5))
        .expect_hover_contains("Record");
}

#[test]
fn test_lsp_hover_trait_definition() {
    let code = "trait Printable {\n  display(): string\n}\n";
    ShapeTest::new(code)
        .at(pos(0, 6))
        .expect_hover_contains("Printable");
}

#[test]
fn test_lsp_hover_impl_method_shows_trait() {
    let code = "trait Renderable {\n  render(): string\n}\nimpl Renderable for Item {\n  method render() { \"item\" }\n}\n";
    ShapeTest::new(code)
        .at(pos(4, 9))
        .expect_hover_contains("Trait Method");
}

#[test]
fn test_lsp_hover_self_in_impl_shows_receiver() {
    let code = "type Box { val: int }\nimpl Display for Box {\n  method show() { self.val }\n}\n";
    ShapeTest::new(code)
        .at(pos(2, 19))
        .expect_hover_contains("self")
        .expect_hover_contains("Box");
}

#[test]
fn test_lsp_hover_property_access_shows_field() {
    let code = "type Point { x: int, y: int }\nlet p = Point { x: 10, y: 20 }\np.x\n";
    ShapeTest::new(code)
        .at(pos(2, 2))
        .expect_hover_contains("Property");
}

#[test]
fn test_lsp_hover_module_name() {
    let code = "mod utils { fn clamp(x: int) { x } }\nutils\n";
    ShapeTest::new(code)
        .at(pos(1, 2))
        .expect_hover_contains("utils");
}

#[test]
fn test_lsp_hover_module_function() {
    let code = "mod csv { fn load(path: string) { path } }\nlet df = csv.load(\"test\")\n";
    ShapeTest::new(code)
        .at(pos(1, 14))
        .expect_hover_contains("load");
}

#[test]
fn test_lsp_hover_doc_comment_on_function() {
    let code = "/// Computes the absolute value\nfn my_abs(x: int) -> int {\n  if x < 0 { return -x }\n  return x\n}\n";
    ShapeTest::new(code)
        .at(pos(1, 4))
        .expect_hover_contains("Computes the absolute value");
}

#[test]
fn test_lsp_hover_empty_line_no_hover() {
    ShapeTest::new("let x = 1\n\nlet y = 2\n")
        .at(pos(1, 0))
        .expect_no_hover();
}

#[test]
fn test_lsp_hover_on_const_keyword() {
    ShapeTest::new("const MAX = 100")
        .at(pos(0, 0))
        .expect_hover_contains("const");
}

#[test]
fn test_lsp_hover_bounded_type_param() {
    let code = "trait Addable {\n  add(other): any\n}\nfn sum<T: Addable>(a: T, b: T) { a }\n";
    ShapeTest::new(code)
        .at(pos(3, 7))
        .expect_hover_contains("Type Parameter");
}

#[test]
fn test_lsp_hover_object_with_hoisted_field_masked() {
    let code = "let a = { x: 1 }\na.y = 2\n";
    ShapeTest::new(code)
        .at(pos(0, 4))
        .expect_hover_contains("x: int")
        .expect_hover_contains("/*, y: int */");
}

#[test]
fn test_lsp_hover_object_after_assignment_shows_full_shape() {
    let code = "let a = { x: 1 }\na.y = 2\n";
    ShapeTest::new(code)
        .at(pos(1, 0))
        .expect_hover_contains("x: int")
        .expect_hover_contains("y: int");
}

#[test]
fn test_lsp_hover_unannotated_param_union() {
    let code = "fn identity(x) {\n  return x\n}\nlet a = identity(1)\nlet b = identity(\"hi\")\n";
    ShapeTest::new(code)
        .at(pos(0, 3))
        .expect_hover_contains("int")
        .expect_hover_contains("string");
}

#[test]
fn test_lsp_hover_match_scrutinee_param_type() {
    let code = "fn check(c: int) {\n  let result = match c {\n    0 => \"zero\"\n    _ => \"other\"\n  }\n  return result\n}\n";
    ShapeTest::new(code)
        .at(pos(1, 21))
        .expect_hover_contains("int");
}
