//! LSP completion, context, and signature help tests.

use shape_test::shape_test::{ShapeTest, pos};

// == Completions: struct/object fields (from lsp_analysis) ===================

#[test]
fn completions_after_dot_shows_struct_fields() {
    let code =
        "type MyType { i: int, name: string }\nlet b = MyType { i: 10, name: \"hello\" }\nb.i\n";
    ShapeTest::new(code)
        .at(pos(2, 2))
        .expect_completion("i")
        .expect_completion("name");
}

#[test]
fn completions_at_statement_start_include_keywords() {
    ShapeTest::new("let x = 5;\n")
        .at(pos(1, 0))
        .expect_completion_any_of(&["let", "function", "fn"]);
}

#[test]
fn completions_include_user_symbols() {
    ShapeTest::new("let myVar = 5;\nconst MY_CONST = 10;\n")
        .at(pos(1, 18))
        .expect_completion("myVar");
}

#[test]
fn completions_include_builtin_functions() {
    ShapeTest::new("")
        .at(pos(0, 0))
        .expect_completion("abs")
        .expect_completion("print");
}

#[test]
fn completions_include_annotated_function_as_function() {
    let code =
        "annotation my_ann() {}\n@my_ann\nfunction my_pattern(c) {\n  return true\n}\nmy_p\n";
    ShapeTest::new(code)
        .at(pos(5, 4))
        .expect_completion("my_pattern");
}

#[test]
fn completions_after_module_dot_show_functions() {
    let code = "mod csv { fn load(path: string) { path } }\ncsv.";
    ShapeTest::new(code)
        .at(pos(1, 4))
        .expect_completions_not_empty();
}

// == F-string interpolation completions (from lsp_analysis) ==================

#[test]
fn fstring_interpolation_provides_variable_completions() {
    let code = "let myVar = 42\nlet s = f\"value is {myV\"\n";
    ShapeTest::new(code)
        .at(pos(1, 23))
        .expect_completion("myVar");
}

#[test]
fn fstring_interpolation_context_is_not_string() {
    let code = "let x = 10\nf\"hello {x\"\n";
    ShapeTest::new(code)
        .at(pos(1, 10))
        .expect_completions_not_empty();
}

// == Comptime completions (from lsp_comptime) ================================

#[test]
fn completion_type_alias_override_suggests_comptime_fields() {
    let code = "type Currency { comptime symbol: string = \"$\", comptime decimals: number = 2, amount: number }\ntype EUR = Currency { symbol: \"E\" }";
    ShapeTest::new(code)
        .at(pos(1, 22))
        .expect_completion("symbol");
}

#[test]
fn completion_type_alias_override_excludes_runtime_fields() {
    let code = "type Currency { comptime symbol: string = \"$\", comptime decimals: number = 2, amount: number }\ntype EUR = Currency { symbol: \"E\" }";
    ShapeTest::new(code)
        .at(pos(1, 22))
        .expect_no_completion("amount");
}

#[test]
fn completion_inside_comptime_block_offers_builtins() {
    let code = "comptime {\n    \n}\n";
    ShapeTest::new(code)
        .at(pos(1, 4))
        .expect_completion("build_config");
}

#[test]
fn completion_inside_comptime_block_offers_implements() {
    let code = "comptime {\n    \n}\n";
    ShapeTest::new(code)
        .at(pos(1, 4))
        .expect_completion("implements");
}

#[test]
fn completion_for_generated_method_from_comptime_extend_target() {
    let code = r#"
annotation add_sum() {
    targets: [type]
    comptime post(target, ctx) {
        extend target {
            method sum() { self.x + self.y }
        }
    }
}
@add_sum()
type Point { x: int, y: int }
let p = Point { x: 1, y: 2 }
let _ = p.s
"#;
    ShapeTest::new(code)
        .at(pos(11, 11))
        .expect_completion("sum");
}

// == Trait/impl completions (from lsp_presentation) ==========================

#[test]
fn completion_impl_block_suggests_unimplemented_methods() {
    let code = "trait Queryable {\n    filter(pred): any;\n    select(cols): any\n}\nimpl Queryable for MyTable {\n    method filter(pred) { self }\n    \n}\n";
    ShapeTest::new(code)
        .at(pos(6, 4))
        .expect_completion("select");
}

#[test]
fn completion_impl_block_excludes_already_implemented() {
    let code = "trait Queryable {\n    filter(pred): any;\n    select(cols): any\n}\nimpl Queryable for MyTable {\n    method filter(pred) { self }\n    \n}\n";
    ShapeTest::new(code)
        .at(pos(6, 4))
        .expect_no_completion("filter");
}

#[test]
fn trait_bound_completions_suggest_trait_names() {
    let code = "trait Comparable {\n    compare(other): number\n}\ntrait Displayable {\n    display(): string\n}\nfn foo<T: >(x: T) {\n    x\n}\n";
    ShapeTest::new(code)
        .at(pos(6, 10))
        .expect_completion("Comparable");
}

#[test]
fn trait_bound_completions_include_all_traits() {
    let code = "trait Comparable {\n    compare(other): number\n}\ntrait Displayable {\n    display(): string\n}\nfn foo<T: >(x: T) {\n    x\n}\n";
    ShapeTest::new(code)
        .at(pos(6, 10))
        .expect_completion("Displayable");
}

#[test]
fn completion_impl_block_suggests_default_methods() {
    let code = "trait Queryable {\n    filter(pred): any;\n    method execute() {\n        return self\n    }\n}\nimpl Queryable for MyTable {\n    method filter(pred) { self }\n    \n}\n";
    ShapeTest::new(code)
        .at(pos(8, 4))
        .expect_completion("execute");
}

// == Context detection (from lsp_analysis) ===================================

#[test]
fn context_after_dot_is_property_access() {
    ShapeTest::new("data[0].")
        .at(pos(0, 8))
        .expect_context_property_access();
}

#[test]
fn context_at_statement_start_is_general() {
    ShapeTest::new("let x = 5;\n")
        .at(pos(1, 0))
        .expect_context_general();
}

#[test]
fn context_inside_import_is_import_module() {
    ShapeTest::new("use ")
        .at(pos(0, 4))
        .expect_context_import_module();
}

// == Signature help (from lsp_analysis) ======================================

#[test]
fn signature_help_inside_function_parens() {
    ShapeTest::new("let x = abs(")
        .at(pos(0, 12))
        .expect_signature_help();
}

#[test]
fn signature_help_active_parameter_advances_with_commas() {
    ShapeTest::new("sma(series, ")
        .at(pos(0, 12))
        .expect_active_parameter_min(1);
}

#[test]
fn signature_help_user_defined_function() {
    let code = "function myFunc(x, y) {\n    return x + y;\n}\nlet r = myFunc(\n";
    ShapeTest::new(code)
        .at(pos(3, 15))
        .expect_signature_help_if_available();
}

#[test]
fn signature_help_for_module_function() {
    let code = "mod csv { fn load(path: string) { path } }\ncsv.load(";
    ShapeTest::new(code).at(pos(1, 9)).expect_signature_help();
}

// == Signature help (from lsp_new_features) ==================================

#[test]
fn signature_help_nested_call() {
    ShapeTest::new("function foo(x) { return x; }\nlet y = foo(abs(")
        .at(pos(1, 16))
        .expect_signature_help();
}

#[test]
fn signature_help_after_first_arg() {
    ShapeTest::new("function add(a, b) { return a + b; }\nadd(1, ")
        .at(pos(1, 7))
        .expect_active_parameter_min(1);
}

// == Completions deep (from programs_lsp_completeness) =======================

#[test]
fn test_lsp_completion_struct_fields_after_dot() {
    let code =
        "type User { name: string, age: int }\nlet u = User { name: \"Alice\", age: 30 }\nu.n\n";
    ShapeTest::new(code)
        .at(pos(2, 3))
        .expect_completions_not_empty();
}

#[test]
fn test_lsp_completion_object_fields_after_dot() {
    let code = "let o = { x: 1, y: 2 }\no.x\n";
    ShapeTest::new(code)
        .at(pos(1, 2))
        .expect_completions_not_empty();
}

#[test]
fn test_lsp_completion_keywords_at_start() {
    ShapeTest::new("let x = 5\n")
        .at(pos(1, 0))
        .expect_completion_any_of(&["let", "fn", "function", "const"]);
}

#[test]
fn test_lsp_completion_includes_user_variable() {
    let code = "let myValue = 100\nlet other = myV\n";
    ShapeTest::new(code)
        .at(pos(1, 15))
        .expect_completion("myValue");
}

#[test]
fn test_lsp_completion_includes_user_function() {
    let code = "fn calculate(x: int) -> int { return x * 2 }\ncal\n";
    ShapeTest::new(code)
        .at(pos(1, 3))
        .expect_completion("calculate");
}

#[test]
fn test_lsp_completion_includes_builtin_print() {
    ShapeTest::new("").at(pos(0, 0)).expect_completion("print");
}

#[test]
fn test_lsp_completion_includes_builtin_abs() {
    ShapeTest::new("").at(pos(0, 0)).expect_completion("abs");
}

#[test]
fn test_lsp_completion_module_dot_functions() {
    let code = "mod math { fn sqrt(x: number) { x } }\nmath.\n";
    ShapeTest::new(code)
        .at(pos(1, 5))
        .expect_completions_not_empty();
}

#[test]
fn test_lsp_completion_inside_function_body_shows_locals() {
    let code = "fn test() {\n  let localVar = 42\n  \n}\n";
    ShapeTest::new(code)
        .at(pos(2, 2))
        .expect_completions_not_empty();
}

#[test]
fn test_lsp_completion_after_dot_context_is_property_access() {
    ShapeTest::new("let o = { x: 1 }\no.")
        .at(pos(1, 2))
        .expect_context_property_access();
}

#[test]
fn test_lsp_completion_import_context() {
    ShapeTest::new("use ")
        .at(pos(0, 4))
        .expect_context_import_module();
}

#[test]
fn test_lsp_completion_enum_variant() {
    let code = "enum Direction {\n  North,\n  South\n}\nDir\n";
    ShapeTest::new(code)
        .at(pos(4, 3))
        .expect_completions_not_empty();
}

#[test]
fn test_lsp_completion_fstring_variable() {
    let code = "let name = \"world\"\nlet s = f\"hello {na\"\n";
    ShapeTest::new(code)
        .at(pos(1, 19))
        .expect_completion("name");
}

#[test]
fn test_lsp_completion_trait_method_in_impl() {
    let code = "trait Greet {\n  greet(): string;\n  farewell(): string\n}\nimpl Greet for Person {\n  method greet() { \"hi\" }\n  \n}\n";
    ShapeTest::new(code)
        .at(pos(6, 2))
        .expect_completion("farewell");
}

#[test]
fn test_lsp_completion_excludes_implemented_trait_method() {
    let code = "trait Greet {\n  greet(): string;\n  farewell(): string\n}\nimpl Greet for Person {\n  method greet() { \"hi\" }\n  \n}\n";
    ShapeTest::new(code)
        .at(pos(6, 2))
        .expect_no_completion("greet");
}

#[test]
fn test_lsp_completion_not_empty_for_general_context() {
    ShapeTest::new("let x = 42\n")
        .at(pos(1, 0))
        .expect_completions_not_empty();
}

#[test]
fn test_lsp_completion_annotated_function_appears() {
    let code = "annotation my_ann() {}\n@my_ann\nfn my_func(x: int) -> int { return x }\nmy_\n";
    ShapeTest::new(code)
        .at(pos(3, 3))
        .expect_completion("my_func");
}

#[test]
fn test_lsp_completion_struct_type_name() {
    let code = "type Rectangle { width: int, height: int }\nRect\n";
    ShapeTest::new(code)
        .at(pos(1, 4))
        .expect_completion("Rectangle");
}

#[test]
fn test_lsp_completion_trait_bound_suggests_trait() {
    let code = "trait Sortable {\n  sort(): any\n}\nfn process<T: >(items: T) { items }\n";
    ShapeTest::new(code)
        .at(pos(3, 14))
        .expect_completion("Sortable");
}

// == Signature help deep (from programs_lsp_completeness) ====================

#[test]
fn test_lsp_nav_signature_help_builtin() {
    ShapeTest::new("let x = abs(")
        .at(pos(0, 12))
        .expect_signature_help();
}

#[test]
fn test_lsp_nav_signature_help_user_function() {
    let code = "fn greet(name: string, loud: bool) -> string { return name }\ngreet(\n";
    ShapeTest::new(code)
        .at(pos(1, 6))
        .expect_signature_help_if_available();
}
