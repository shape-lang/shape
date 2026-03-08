use super::*;
use crate::module_cache::ModuleCache;
use crate::type_inference::extract_wrapper_inner;
use crate::util::offset_to_line_col;

#[test]
fn test_get_word_at_position() {
    let text = "let myVar = 5;";

    // Position on "myVar"
    let word = get_word_at_position(
        text,
        Position {
            line: 0,
            character: 5,
        },
    );
    assert_eq!(word, Some("myVar".to_string()));

    // Position on "let"
    let word = get_word_at_position(
        text,
        Position {
            line: 0,
            character: 1,
        },
    );
    assert_eq!(word, Some("let".to_string()));
}

#[test]
fn test_keyword_hover() {
    let hover = get_keyword_hover("let");
    assert!(hover.is_some());

    let hover = hover.unwrap();
    if let HoverContents::Markup(markup) = hover.contents {
        assert!(markup.value.contains("let"));
        assert!(markup.value.contains("variable"));
    }
}

#[test]
fn test_builtin_function_hover() {
    let hover = get_builtin_function_hover("abs");
    assert!(hover.is_some(), "abs function should have hover info");

    let hover = hover.unwrap();
    if let HoverContents::Markup(markup) = hover.contents {
        assert!(markup.value.contains("abs"), "Should contain function name");
        // Description may come from stdlib or legacy metadata
        assert!(
            markup.value.contains("Signature") || markup.value.contains("**Function**"),
            "Should contain function info"
        );
    }
}

#[test]
fn test_type_hover() {
    let hover = get_type_hover("Table");
    assert!(hover.is_some());

    let hover = hover.unwrap();
    if let HoverContents::Markup(markup) = hover.contents {
        assert!(markup.value.contains("Table"));
        assert!(markup.value.to_lowercase().contains("table"));
    }
}

#[test]
fn test_type_hover_accepts_lowercase_builtin_names() {
    assert!(get_type_hover("number").is_some());
    assert!(get_type_hover("int").is_some());
    assert!(get_type_hover("string").is_some());
    assert!(get_type_hover("bool").is_some());
}

#[test]
fn test_frontmatter_shape_type_hover_still_works() {
    let code = r#"---
[[extensions]]
name = "python"
path = "./extensions/libshape_ext_python.so"
---
fn python percentile(values: Array<number>, pct: number) -> number {
  return pct
}
"#;

    let hover = get_hover(
        code,
        Position {
            line: 5,
            character: 37,
        },
        None,
        None,
        None,
    );
    assert!(
        hover.is_some(),
        "expected hover for builtin type in frontmatter script"
    );
    if let Some(h) = hover
        && let HoverContents::Markup(markup) = h.contents
    {
        assert!(
            markup.value.to_lowercase().contains("type"),
            "expected type hover content, got: {}",
            markup.value
        );
        assert!(
            markup.value.to_lowercase().contains("number"),
            "expected number type hover content, got: {}",
            markup.value
        );
    }
}

#[test]
fn test_user_symbol_hover() {
    let code = r#"let myVar = 42;
const MY_CONST = 3.14;

function myFunc(x, y) {
    return x + y;
}
"#;

    // Test variable hover
    let hover = get_user_symbol_hover(code, "myVar");
    assert!(hover.is_some());

    // Test constant hover
    let hover = get_user_symbol_hover(code, "MY_CONST");
    assert!(hover.is_some());

    // Test function hover
    let hover = get_user_symbol_hover(code, "myFunc");
    assert!(hover.is_some());
}

#[test]
fn test_get_hover_integration() {
    let code = "let x = abs(-5);";

    // Hover over "abs" should show function info (character 8-10)
    let hover = get_hover(
        code,
        Position {
            line: 0,
            character: 9,
        },
        None,
        None,
        None,
    );
    assert!(hover.is_some(), "Should get hover for 'abs' function");

    // Hover over "let" should show keyword info
    let hover = get_hover(
        code,
        Position {
            line: 0,
            character: 1,
        },
        None,
        None,
        None,
    );
    assert!(hover.is_some(), "Should get hover for 'let' keyword");
}

#[test]
fn test_infer_variable_type_from_literal() {
    let code = r#"let x = 42;
let s = "hello";
let b = true;"#;

    let program = parse_program(code).unwrap();

    // Integer literal (42 without decimal point is int)
    let x_type = infer_variable_type(&program, "x");
    assert_eq!(x_type, Some("int".to_string()));

    // String literal
    let s_type = infer_variable_type(&program, "s");
    assert_eq!(s_type, Some("string".to_string()));

    // Boolean literal
    let b_type = infer_variable_type(&program, "b");
    assert_eq!(b_type, Some("bool".to_string()));
}

#[test]
fn test_infer_variable_type_from_explicit_annotation() {
    let code = r#"let result: Result<string> = fetch("url")"#;

    let program = parse_program(code).unwrap();
    let data_type = infer_variable_type(&program, "result");

    // Explicit type annotation should be picked up
    assert!(data_type.is_some(), "Should infer type from annotation");
    let type_str = data_type.unwrap();
    assert!(
        type_str.contains("Result"),
        "Should contain Result, got: {}",
        type_str
    );
}

#[test]
fn test_infer_variable_type_from_string_literal() {
    let code = r#"let name = "hello""#;

    let program = parse_program(code).unwrap();
    let name_type = infer_variable_type(&program, "name");

    assert!(name_type.is_some(), "Should infer type from string literal");
    let type_str = name_type.unwrap();
    assert!(type_str == "string", "Should be string, got: {}", type_str);
}

#[test]
fn test_extract_wrapper_inner() {
    // Result<T>
    assert_eq!(
        extract_wrapper_inner("Result<Instrument>"),
        Some("Instrument".to_string())
    );

    // Result<T, E>
    assert_eq!(
        extract_wrapper_inner("Result<Instrument, Error>"),
        Some("Instrument".to_string())
    );

    // Option<T>
    assert_eq!(
        extract_wrapper_inner("Option<number>"),
        Some("number".to_string())
    );

    // T?
    assert_eq!(extract_wrapper_inner("number?"), Some("number".to_string()));

    // Plain type
    assert_eq!(
        extract_wrapper_inner("Instrument"),
        Some("Instrument".to_string())
    );
}

#[test]
fn test_hover_on_named_decomposition_binding_with_server_context() {
    let code = r#"let a = { x: 1}
let b = { z: 3}
//print(a.y) //compiler error: no y (even though a has y in the shape via optimistic hoisting, see next line)
a.y = 2
print(a.y) //works!
let c = a+b //resulting type is {x: int, y: int, z: int}
//destructuring works, e.g.
let (d:{x}, e: {y, z})  = c
//destructuring to named structs works also but need the as keyword:
type TypeA {x: int, y: int}
type TypeB {z: int}
let (f:TypeA, g: TypeB) = c as (TypeA+TypeB)
print(f, g)
"#;

    let tmp = tempfile::tempdir().expect("tempdir");
    let file_path = tmp.path().join("test.shape");
    std::fs::write(&file_path, code).expect("write shape source");
    let module_cache = ModuleCache::new();
    let cached_program = parse_program(code).expect("program should parse");

    let hover = get_hover(
        code,
        Position {
            line: 12,
            character: 6,
        },
        Some(&module_cache),
        Some(file_path.as_path()),
        Some(&cached_program),
    );

    assert!(
        hover.is_some(),
        "Expected hover for decomposition binding `f` usage in print(f, g)"
    );
    if let Some(h) = hover
        && let HoverContents::Markup(markup) = h.contents
    {
        assert!(
            markup.value.contains("Variable"),
            "Expected variable hover content, got: {}",
            markup.value
        );
        assert!(
            markup.value.contains("`f`"),
            "Expected hover for symbol `f`, got: {}",
            markup.value
        );
    }
}

#[test]
fn test_user_symbol_hover_with_inferred_type() {
    let code = r#"let x = 42;"#;

    let hover = get_user_symbol_hover(code, "x");
    assert!(hover.is_some(), "Should get hover for variable");

    if let Some(h) = hover {
        if let HoverContents::Markup(markup) = h.contents {
            // 42 is an integer literal, so type is "int"
            assert!(
                markup.value.contains("int"),
                "Should show inferred type 'int', got: {}",
                markup.value
            );
        }
    }
}

#[test]
fn test_union_type_hover() {
    let code = r#"
            let result = match 1 {
                1 => true,
                2 => "hello"
            }
        "#;

    let hover = get_user_symbol_hover(code, "result");

    // Should infer union type
    if let Some(h) = hover {
        if let HoverContents::Markup(markup) = h.contents {
            // Should show union type (either explicit or formatted)
            let has_union_info = markup.value.contains("bool") && markup.value.contains("string");
            assert!(
                has_union_info,
                "Should show union type info, got: {}",
                markup.value
            );
        }
    }
}

#[test]
fn test_builtin_hover_has_description() {
    // Hover for core builtins should include signature and description
    for name in &["abs", "sqrt", "print", "max", "min", "len"] {
        let hover = get_builtin_function_hover(name);
        assert!(hover.is_some(), "'{}' should have hover info", name);

        if let Some(h) = hover {
            if let HoverContents::Markup(markup) = h.contents {
                assert!(
                    markup.value.contains(name),
                    "'{}' hover should contain function name, got: {}",
                    name,
                    markup.value
                );
            }
        }
    }
}

#[test]
fn test_keyword_hover_has_examples() {
    // Keywords with enhanced docs should show examples
    let hover = get_keyword_hover("type");
    assert!(hover.is_some());
    if let HoverContents::Markup(markup) = hover.unwrap().contents {
        assert!(
            markup.value.contains("Point"),
            "'type' hover should include example"
        );
    }
}

#[test]
fn test_extract_doc_comment_line() {
    let source = "/// This is a doc comment\nfunction foo() { return 1 }";
    let program = shape_ast::parse_program(source).expect("program should parse");
    assert_eq!(
        program
            .docs
            .comment_for_path("foo")
            .map(|doc| doc.summary.as_str()),
        Some("This is a doc comment")
    );
}

#[test]
fn test_extract_doc_comment_multiline() {
    let source = "/// Line one\n/// Line two\nfunction foo() { return 1 }";
    let program = shape_ast::parse_program(source).expect("program should parse");
    assert_eq!(
        program.docs.comment_for_path("foo").map(|doc| doc.body.as_str()),
        Some("Line one\nLine two")
    );
}

#[test]
fn test_extract_doc_comment_block() {
    let source = "/** Block doc comment */\nfunction foo() { return 1 }";
    let program = shape_ast::parse_program(source).expect("program should parse");
    assert!(program.docs.comment_for_path("foo").is_none());
}

#[test]
fn test_extract_doc_comment_none() {
    let source = "// Regular comment\nfunction foo() { return 1 }";
    let program = shape_ast::parse_program(source).expect("program should parse");
    assert!(program.docs.comment_for_path("foo").is_none());
}

#[test]
fn test_doc_comment_hover() {
    let code = "/// Calculates the sum\nfunction mySum(a, b) { return a + b }";
    let hover = get_user_symbol_hover(code, "mySum");
    assert!(hover.is_some(), "Should get hover for documented function");
    if let Some(h) = hover {
        if let HoverContents::Markup(markup) = h.contents {
            assert!(
                markup.value.contains("Calculates the sum"),
                "Hover should include doc comment, got: {}",
                markup.value
            );
        }
    }
}

#[test]
fn test_struct_field_hover_with_type_def() {
    // With explicit type definition, hover uses declared field types
    let code =
        "type MyType { i: int, name: string }\nlet b = MyType { i: 10, name: \"hello\" }\nb.i\n";
    let hover = get_hover(
        code,
        Position {
            line: 2,
            character: 2,
        },
        None,
        None,
        None,
    );
    assert!(hover.is_some(), "Should get hover for struct field 'i'");
    if let Some(h) = hover {
        if let HoverContents::Markup(markup) = h.contents {
            assert!(
                markup.value.contains("Property"),
                "Should be a property hover, got: {}",
                markup.value
            );
            assert!(
                markup.value.contains("int"),
                "Should show field type 'int', got: {}",
                markup.value
            );
            assert!(
                markup.value.contains("MyType"),
                "Should reference the struct type, got: {}",
                markup.value
            );
        }
    }
}

#[test]
fn test_struct_field_hover_generic_instantiation() {
    let code = "type MyType<T:int> { x: T }\nlet a = MyType { x: 1.0 }\na.x\n";
    let hover = get_hover(
        code,
        Position {
            line: 2,
            character: 2,
        },
        None,
        None,
        None,
    );
    assert!(
        hover.is_some(),
        "Should get hover for generic struct field access"
    );
    if let Some(h) = hover
        && let HoverContents::Markup(markup) = h.contents
    {
        assert!(
            markup.value.contains("number"),
            "Expected instantiated generic field type 'number', got: {}",
            markup.value
        );
        assert!(
            markup.value.contains("MyType<number>"),
            "Expected concrete object type in hover, got: {}",
            markup.value
        );
    }
}

#[test]
fn test_struct_field_hover_generic_default_argument() {
    let code = "type MyType<T:int> { x: T }\nlet b = MyType { x: 1 }\nb.x\n";
    let hover = get_hover(
        code,
        Position {
            line: 2,
            character: 2,
        },
        None,
        None,
        None,
    );
    assert!(
        hover.is_some(),
        "Should get hover for generic struct field with default argument"
    );
    if let Some(h) = hover
        && let HoverContents::Markup(markup) = h.contents
    {
        assert!(
            markup.value.contains("int"),
            "Expected field type 'int', got: {}",
            markup.value
        );
    }
}

#[test]
fn test_struct_field_hover_without_type_def() {
    // Regression: when there's NO type definition, hover should infer field
    // types from the struct literal's value expressions.
    let code = "let b = MyType { i: 10.2D }\nb.i\n";
    let hover = get_hover(
        code,
        Position {
            line: 1,
            character: 2,
        },
        None,
        None,
        None,
    );
    assert!(
        hover.is_some(),
        "Should get hover for struct field 'i' even without type def"
    );
    if let Some(h) = hover {
        if let HoverContents::Markup(markup) = h.contents {
            assert!(
                markup.value.contains("Property"),
                "Should be a property hover, got: {}",
                markup.value
            );
            assert!(
                markup.value.contains("decimal"),
                "Should infer field type 'decimal' from literal, got: {}",
                markup.value
            );
        }
    }
}

#[test]
fn test_struct_field_hover_string() {
    // Without type definition, infer field type from literal
    let code = "let b = MyType { i: 10, name: \"hello\" }\nb.name\n";
    let hover = get_hover(
        code,
        Position {
            line: 1,
            character: 3,
        },
        None,
        None,
        None,
    );
    assert!(hover.is_some(), "Should get hover for struct field 'name'");
    if let Some(h) = hover {
        if let HoverContents::Markup(markup) = h.contents {
            assert!(
                markup.value.contains("Property"),
                "Should be a property hover, got: {}",
                markup.value
            );
            assert!(
                markup.value.contains("string"),
                "Should show field type 'string', got: {}",
                markup.value
            );
        }
    }
}

#[test]
fn test_struct_field_hover_decimal() {
    // Without type definition, decimal field inferred from literal
    let code = "let p = Price { amount: 10D }\np.amount\n";
    let hover = get_hover(
        code,
        Position {
            line: 1,
            character: 3,
        },
        None,
        None,
        None,
    );
    assert!(
        hover.is_some(),
        "Should get hover for struct field 'amount'"
    );
    if let Some(h) = hover {
        if let HoverContents::Markup(markup) = h.contents {
            assert!(
                markup.value.contains("decimal"),
                "Should show field type 'decimal', got: {}",
                markup.value
            );
            assert!(
                markup.value.contains("Price"),
                "Should reference the struct type, got: {}",
                markup.value
            );
        }
    }
}

#[test]
fn test_hover_struct_type_name_in_declaration() {
    let code = "type User { name: String }\n";
    let hover = get_hover(
        code,
        Position {
            line: 0,
            character: 6,
        },
        None,
        None,
        None,
    );
    assert!(hover.is_some(), "Should get hover for type name 'User'");
    if let Some(h) = hover {
        if let HoverContents::Markup(markup) = h.contents {
            assert!(
                markup.value.contains("**Type**: `User`"),
                "Type hover should show User, got: {}",
                markup.value
            );
        }
    }
}

#[test]
fn test_hover_impl_method_infers_return_type_without_local_trait() {
    let code = "type User { name: String }\nimpl Display for User as JsonDisplay {\n  method display() { f$\"\"\"{ \"name\": \"${self.name}\" }\"\"\" }\n}\n";
    let hover = get_hover(
        code,
        Position {
            line: 2,
            character: 10,
        },
        None,
        None,
        None,
    );
    assert!(hover.is_some(), "Should get hover for impl method");
    if let Some(h) = hover {
        if let HoverContents::Markup(markup) = h.contents {
            assert!(
                markup.value.contains("display(): string"),
                "Impl method hover should infer string return type, got: {}",
                markup.value
            );
        }
    }
}

#[test]
fn test_hover_self_inside_dollar_interpolation() {
    let code = "type User { name: String }\nimpl Display for User as JsonDisplay {\n  method display() { f$\"\"\"{ \"name\": \"${self.name}\" }\"\"\" }\n}\n";
    let self_offset = code
        .find("self.name")
        .expect("expected self in interpolation");
    let (line, character) = offset_to_line_col(code, self_offset + 1);
    let hover = get_hover(code, Position { line, character }, None, None, None);
    assert!(
        hover.is_some(),
        "Expected hover for self inside interpolation"
    );
    if let Some(h) = hover {
        if let HoverContents::Markup(markup) = h.contents {
            assert!(
                markup.value.contains("**Variable**: `self`"),
                "Unexpected hover content: {}",
                markup.value
            );
            assert!(
                markup.value.contains("`User`"),
                "Expected receiver type in hover content: {}",
                markup.value
            );
        }
    }
}

#[test]
fn test_hover_self_property_inside_dollar_interpolation() {
    let code = "type User { name: String }\nimpl Display for User as JsonDisplay {\n  method display() { f$\"\"\"{ \"name\": \"${self.name}\" }\"\"\" }\n}\n";
    let name_offset = code
        .find("self.name")
        .expect("expected self.name in interpolation")
        + "self.".len();
    let (line, character) = offset_to_line_col(code, name_offset + 1);
    let hover = get_hover(code, Position { line, character }, None, None, None);
    assert!(
        hover.is_some(),
        "Expected hover for self property inside interpolation"
    );
    if let Some(h) = hover {
        if let HoverContents::Markup(markup) = h.contents {
            assert!(
                markup.value.contains("**Property**: `name`"),
                "Unexpected hover content: {}",
                markup.value
            );
            assert!(
                markup.value.contains("**Receiver:** `User`"),
                "Expected receiver context in hover content: {}",
                markup.value
            );
        }
    }
}

#[test]
fn test_hover_impl_trait_resolves_members_from_module_context() {
    let code = "type User { name: String }\nimpl Display for User as JsonDisplay {\n  method display() { self.name }\n}\n";
    let cache = ModuleCache::new();
    let current_file = std::env::current_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .join("__shape_lsp_trait_hover_test__.shape");
    let hover = get_hover(
        code,
        Position {
            line: 1,
            character: 6,
        },
        Some(&cache),
        Some(current_file.as_path()),
        None,
    );
    assert!(
        hover.is_some(),
        "Should get hover for trait name in impl header"
    );
    if let Some(h) = hover {
        if let HoverContents::Markup(markup) = h.contents {
            assert!(
                markup.value.contains("display(): string"),
                "Trait hover should include trait member signatures, got: {}",
                markup.value
            );
        }
    }
}

#[test]
fn test_hover_local_annotation_usage() {
    let code = "/// Trace function execution.\nannotation trace() {\n  metadata() { return { ok: true } }\n}\n\n@trace\nfn run() { 1 }\n";
    let hover = get_hover(
        code,
        Position {
            line: 5,
            character: 2,
        },
        None,
        None,
        None,
    );
    assert!(
        hover.is_some(),
        "Should get hover for local annotation usage"
    );
    if let Some(h) = hover {
        if let HoverContents::Markup(markup) = h.contents {
            assert!(
                markup.value.contains("**Annotation**: `@trace`"),
                "Annotation hover should show local annotation metadata, got: {}",
                markup.value
            );
            assert!(
                markup.value.contains("Trace function execution."),
                "Annotation hover should render doc comments, got: {}",
                markup.value
            );
            assert!(
                !markup.value.contains("Handlers:"),
                "Annotation hover must not synthesize handler descriptions, got: {}",
                markup.value
            );
        }
    }
}

#[test]
fn test_hover_locally_defined_annotation_usage_with_module_cache() {
    let code = "/// Mark a function for auditing.\nannotation my_ann() {}\n@my_ann\nfn run() { 1 }\n";
    let cache = ModuleCache::new();
    let current_file = std::env::current_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .join("__shape_lsp_annotation_hover_test__.shape");
    let hover = get_hover(
        code,
        Position {
            line: 1,
            character: 2,
        },
        Some(&cache),
        Some(current_file.as_path()),
        None,
    );
    assert!(
        hover.is_some(),
        "Should resolve locally defined annotation hover via module cache"
    );
    if let Some(h) = hover {
        if let HoverContents::Markup(markup) = h.contents {
            assert!(
                markup.value.contains("**Annotation**: `@my_ann`"),
                "Locally defined annotation hover should show annotation metadata, got: {}",
                markup.value
            );
            assert!(
                markup.value.contains("Mark a function for auditing."),
                "Locally defined annotation hover should render doc comments, got: {}",
                markup.value
            );
        }
    }
}

#[test]
fn test_hover_bounded_type_param() {
    let code = "trait Comparable {\n    compare(other): number\n}\nfn foo<T: Comparable>(x: T) {\n    x\n}\n";
    // Hover on "T" in the function declaration
    let hover = get_hover(
        code,
        Position {
            line: 3,
            character: 7,
        },
        None,
        None,
        None,
    );
    assert!(
        hover.is_some(),
        "Should get hover for bounded type param 'T'"
    );
    if let Some(h) = hover {
        if let HoverContents::Markup(markup) = h.contents {
            assert!(
                markup.value.contains("Type Parameter"),
                "Should indicate type parameter, got: {}",
                markup.value
            );
            assert!(
                markup.value.contains("Comparable"),
                "Should mention trait bound 'Comparable', got: {}",
                markup.value
            );
        }
    }
}

#[test]
fn test_hover_default_trait_method() {
    let code = "trait Queryable {\n    filter(pred): any;\n    method execute() {\n        return self\n    }\n}\nimpl Queryable for MyTable {\n    method filter(pred) { self }\n    method execute() { self }\n}\n";
    // Hover on "execute" in the impl block
    let hover = get_hover(
        code,
        Position {
            line: 8,
            character: 11,
        },
        None,
        None,
        None,
    );
    assert!(
        hover.is_some(),
        "Should get hover for default method 'execute'"
    );
    if let Some(h) = hover {
        if let HoverContents::Markup(markup) = h.contents {
            assert!(
                markup.value.contains("default"),
                "Should indicate default method, got: {}",
                markup.value
            );
        }
    }
}

#[test]
fn test_hover_await_keyword() {
    let hover = get_keyword_hover("await");
    assert!(hover.is_some(), "Should get hover for 'await' keyword");
    if let Some(h) = hover {
        if let HoverContents::Markup(markup) = h.contents {
            assert!(
                markup.value.contains("await"),
                "Hover should mention await, got: {}",
                markup.value
            );
        }
    }
}

#[test]
fn test_hover_join_keyword() {
    let hover = get_keyword_hover("join");
    assert!(hover.is_some(), "Should get hover for 'join' keyword");
    if let Some(h) = hover {
        if let HoverContents::Markup(markup) = h.contents {
            assert!(
                markup.value.contains("join"),
                "Hover should mention join, got: {}",
                markup.value
            );
            assert!(
                markup.value.contains("concurrent"),
                "Hover should describe concurrency, got: {}",
                markup.value
            );
        }
    }
}

#[test]
fn test_hover_join_strategies() {
    for strategy in &["race", "any", "settle"] {
        let hover = get_keyword_hover(strategy);
        assert!(
            hover.is_some(),
            "Should get hover for '{}' keyword",
            strategy
        );
        if let Some(h) = hover {
            if let HoverContents::Markup(markup) = h.contents {
                assert!(
                    markup.value.contains("Join strategy") || markup.value.contains("join"),
                    "'{}' hover should mention join strategy, got: {}",
                    strategy,
                    markup.value
                );
            }
        }
    }
}

#[test]
fn test_comptime_builtin_hover_type_info_removed() {
    let hover = get_comptime_builtin_hover("type_info");
    assert!(hover.is_none(), "type_info hover should not exist");
}

#[test]
fn test_comptime_builtin_hover_build_config() {
    let hover = get_comptime_builtin_hover("build_config");
    assert!(hover.is_some(), "Should get hover for build_config");
    if let Some(h) = hover {
        if let HoverContents::Markup(markup) = h.contents {
            assert!(markup.value.contains("build_config"));
            assert!(markup.value.contains("build-time configuration"));
        }
    }
}

#[test]
fn test_comptime_builtin_hover_implements() {
    let hover = get_comptime_builtin_hover("implements");
    assert!(hover.is_some(), "Should get hover for implements");
    if let Some(h) = hover {
        if let HoverContents::Markup(markup) = h.contents {
            assert!(markup.value.contains("implements"));
            assert!(markup.value.contains("trait"));
        }
    }
}

#[test]
fn test_comptime_builtin_hover_unknown() {
    let hover = get_comptime_builtin_hover("unknown_fn");
    assert!(hover.is_none(), "Should not get hover for unknown function");
}

#[test]
fn test_hover_async_let() {
    let code = "async fn foo() {\n  async let x = fetch(\"url\")\n}";
    let hover = get_hover(
        code,
        Position {
            line: 1,
            character: 4,
        }, // on "async"
        None,
        None,
        None,
    );
    assert!(
        hover.is_some(),
        "Should get hover for 'async' in async let context"
    );
    if let Some(h) = hover {
        if let HoverContents::Markup(markup) = h.contents {
            assert!(
                markup.value.contains("Async Let"),
                "Should show Async Let hover info, got: {}",
                markup.value
            );
            assert!(
                markup.value.contains("future handle"),
                "Should mention future handle, got: {}",
                markup.value
            );
        }
    }
}

#[test]
fn test_hover_async_scope() {
    let code = "async fn foo() {\n  async scope {\n    42\n  }\n}";
    let hover = get_hover(
        code,
        Position {
            line: 1,
            character: 4,
        }, // on "async"
        None,
        None,
        None,
    );
    assert!(
        hover.is_some(),
        "Should get hover for 'async' in async scope context"
    );
    if let Some(h) = hover {
        if let HoverContents::Markup(markup) = h.contents {
            assert!(
                markup.value.contains("Async Scope"),
                "Should show Async Scope hover info, got: {}",
                markup.value
            );
            assert!(
                markup.value.contains("structured concurrency"),
                "Should mention structured concurrency, got: {}",
                markup.value
            );
        }
    }
}

#[test]
fn test_hover_scope_keyword_in_async_scope() {
    let code = "async fn foo() {\n  async scope {\n    42\n  }\n}";
    let hover = get_hover(
        code,
        Position {
            line: 1,
            character: 10,
        }, // on "scope"
        None,
        None,
        None,
    );
    assert!(hover.is_some(), "Should get hover for 'scope' keyword");
    if let Some(h) = hover {
        if let HoverContents::Markup(markup) = h.contents {
            assert!(
                markup.value.contains("structured concurrency"),
                "Scope hover should describe structured concurrency, got: {}",
                markup.value
            );
        }
    }
}

#[test]
fn test_hover_variable_with_engine_inferred_type() {
    // Engine-based inference should resolve match expression types
    let code = "let result = match 2 {\n  0 => true,\n  _ => false,\n}";
    let hover = get_user_symbol_hover(code, "result");
    assert!(
        hover.is_some(),
        "Should get hover for match-assigned variable"
    );
    if let Some(h) = hover {
        if let HoverContents::Markup(markup) = h.contents {
            assert!(
                markup.value.contains("bool"),
                "Should show engine-inferred type 'bool' for match, got: {}",
                markup.value
            );
        }
    }
}

#[test]
fn test_hover_function_with_inferred_return_type() {
    let code = "fn add(a: int, b: int) {\n  return a + b\n}";
    let hover = get_user_symbol_hover(code, "add");
    assert!(hover.is_some(), "Should get hover for function");
    if let Some(h) = hover {
        if let HoverContents::Markup(markup) = h.contents {
            assert!(
                markup.value.contains("Signature"),
                "Should show function signature, got: {}",
                markup.value
            );
            assert!(
                markup.value.contains("fn add"),
                "Should contain function signature, got: {}",
                markup.value
            );
        }
    }
}

#[test]
fn test_hover_function_shows_inferred_return() {
    // When return type is NOT annotated, engine infers it
    let code = "fn double(x: int) {\n  return x * 2\n}";
    let hover = get_user_symbol_hover(code, "double");
    assert!(hover.is_some(), "Should get hover for function");
    if let Some(h) = hover {
        if let HoverContents::Markup(markup) = h.contents {
            assert!(
                markup.value.contains("Signature"),
                "Should show enhanced signature, got: {}",
                markup.value
            );
            // The engine should produce a signature with `x: int` and inferred return
            assert!(
                markup.value.contains("x: int"),
                "Should show param type from annotation, got: {}",
                markup.value
            );
        }
    }
}

#[test]
fn test_hover_function_signature_shows_inferred_reference_modes() {
    let code = r#"
fn mutate(a) {
  a = a + "!"
  return a
}
let s = "x"
mutate(s)
"#;
    let hover = get_user_symbol_hover(code, "mutate");
    assert!(hover.is_some(), "Should get hover for function");
    if let Some(h) = hover
        && let HoverContents::Markup(markup) = h.contents
    {
        assert!(
            markup.value.contains("a: &mut string"),
            "Expected inferred reference mode in signature, got: {}",
            markup.value
        );
    }
}

#[test]
fn test_hover_typed_match_pattern_overrides_shadowed_name() {
    let code = "let c = \"outer\"\nfn afunc(c) {\n  let result = match c {\n    c: int => c + 1\n    _ => 1\n  }\n  return result\n}\n";

    // Hover on typed pattern binding `c` in `c: int`
    let pattern_hover = get_hover(
        code,
        Position {
            line: 3,
            character: 4,
        },
        None,
        None,
        None,
    )
    .expect("expected hover on typed pattern binding");
    if let HoverContents::Markup(markup) = pattern_hover.contents {
        assert!(
            markup.value.contains("int"),
            "typed match pattern should show int, got: {}",
            markup.value
        );
        assert!(
            !markup.value.contains("string"),
            "typed match pattern should not resolve to outer string binding: {}",
            markup.value
        );
    }

    // Hover on `c` reference in arm body (`=> c + 1`) should resolve to typed binding.
    let body_hover = get_hover(
        code,
        Position {
            line: 3,
            character: 14,
        },
        None,
        None,
        None,
    )
    .expect("expected hover on typed pattern reference");
    if let HoverContents::Markup(markup) = body_hover.contents {
        assert!(
            markup.value.contains("int"),
            "typed arm reference should show int, got: {}",
            markup.value
        );
        assert!(
            !markup.value.contains("string"),
            "typed arm reference should not resolve to outer string binding: {}",
            markup.value
        );
    }
}

#[test]
fn test_hover_interpolation_fixed_spec_keyword() {
    let code = r#"let s = f"price={p:fixed(2)}""#;
    let hover = get_hover(
        code,
        Position {
            line: 0,
            character: code.find("fixed").unwrap() as u32 + 1,
        },
        None,
        None,
        None,
    );
    assert!(hover.is_some(), "expected hover on fixed spec keyword");
    if let Some(h) = hover {
        if let HoverContents::Markup(markup) = h.contents {
            assert!(
                markup.value.contains("fixed(precision)"),
                "unexpected hover content: {}",
                markup.value
            );
        }
    }
}

#[test]
fn test_hover_interpolation_table_spec_key() {
    let code = r#"let s = f"{rows:table(align=right)}""#;
    let hover = get_hover(
        code,
        Position {
            line: 0,
            character: code.find("align").unwrap() as u32 + 1,
        },
        None,
        None,
        None,
    );
    assert!(hover.is_some(), "expected hover on table spec key");
    if let Some(h) = hover {
        if let HoverContents::Markup(markup) = h.contents {
            assert!(
                markup.value.contains("Table Format Key"),
                "unexpected hover content: {}",
                markup.value
            );
        }
    }
}

#[test]
fn test_hover_content_api_namespace() {
    // Hovering over `Content` should show Content API hover
    let code = "let x = Content.text(\"hi\")";
    let hover = get_hover(
        code,
        Position {
            line: 0,
            character: 10, // in the middle of "Content"
        },
        None,
        None,
        None,
    );
    assert!(hover.is_some(), "expected hover on Content namespace");
    if let Some(h) = hover {
        if let HoverContents::Markup(markup) = h.contents {
            assert!(
                markup.value.contains("Content API"),
                "expected Content API docs, got: {}",
                markup.value
            );
        }
    }
}

#[test]
fn test_hover_color_namespace() {
    let code = "let c = Color.red";
    let hover = get_hover(
        code,
        Position {
            line: 0,
            character: 9, // in "Color"
        },
        None,
        None,
        None,
    );
    assert!(hover.is_some(), "expected hover on Color namespace");
    if let Some(h) = hover {
        if let HoverContents::Markup(markup) = h.contents {
            assert!(
                markup.value.contains("Color Enum"),
                "expected Color Enum docs, got: {}",
                markup.value
            );
        }
    }
}

#[test]
fn test_hover_content_member_access() {
    // Use property access (no call parens) so it's parsed as PropertyAccess
    let code = "let x = Color.red";
    let hover = get_hover(
        code,
        Position {
            line: 0,
            character: 15, // in "red"
        },
        None,
        None,
        None,
    );
    assert!(hover.is_some(), "expected hover on Color.red");
    if let Some(h) = hover {
        if let HoverContents::Markup(markup) = h.contents {
            assert!(
                markup.value.contains("Color.red"),
                "expected Color.red docs, got: {}",
                markup.value
            );
        }
    }
}

#[test]
fn test_hover_border_member_access() {
    let code = "let b = Border.rounded";
    let hover = get_hover(
        code,
        Position {
            line: 0,
            character: 16, // in "rounded"
        },
        None,
        None,
        None,
    );
    assert!(hover.is_some(), "expected hover on Border.rounded");
    if let Some(h) = hover {
        if let HoverContents::Markup(markup) = h.contents {
            assert!(
                markup.value.contains("Border.rounded"),
                "expected Border.rounded docs, got: {}",
                markup.value
            );
        }
    }
}

// --- DateTime / io / time namespace hover tests ---

#[test]
fn test_hover_datetime_namespace() {
    let hover = get_namespace_api_hover("DateTime");
    assert!(hover.is_some(), "expected hover for DateTime namespace");
    if let Some(h) = hover {
        if let HoverContents::Markup(markup) = h.contents {
            assert!(
                markup.value.contains("DateTime API"),
                "expected DateTime API docs"
            );
            assert!(
                markup.value.contains("DateTime.now()"),
                "expected DateTime.now mention"
            );
            assert!(
                markup.value.contains("DateTime.parse"),
                "expected DateTime.parse mention"
            );
        }
    }
}

#[test]
fn test_hover_io_namespace() {
    let hover = get_namespace_api_hover("io");
    assert!(hover.is_some(), "expected hover for io namespace");
    if let Some(h) = hover {
        if let HoverContents::Markup(markup) = h.contents {
            assert!(
                markup.value.contains("io Module"),
                "expected io Module docs"
            );
            assert!(markup.value.contains("io.open"), "expected io.open mention");
            assert!(
                markup.value.contains("tcp_connect"),
                "expected tcp_connect mention"
            );
            assert!(
                markup.value.contains("io.spawn"),
                "expected io.spawn mention"
            );
        }
    }
}

#[test]
fn test_hover_time_namespace() {
    let hover = get_namespace_api_hover("time");
    assert!(hover.is_some(), "expected hover for time namespace");
    if let Some(h) = hover {
        if let HoverContents::Markup(markup) = h.contents {
            assert!(
                markup.value.contains("time Module"),
                "expected time Module docs"
            );
            assert!(
                markup.value.contains("time.now()"),
                "expected time.now mention"
            );
            assert!(
                markup.value.contains("time.sleep"),
                "expected time.sleep mention"
            );
        }
    }
}

#[test]
fn test_hover_datetime_member() {
    let hover = get_namespace_member_hover("DateTime", "now");
    assert!(hover.is_some(), "expected hover for DateTime.now");
    if let Some(h) = hover {
        if let HoverContents::Markup(markup) = h.contents {
            assert!(
                markup.value.contains("DateTime.now"),
                "expected DateTime.now docs"
            );
        }
    }

    let hover = get_namespace_member_hover("DateTime", "parse");
    assert!(hover.is_some(), "expected hover for DateTime.parse");
}

#[test]
fn test_hover_io_member() {
    let hover = get_namespace_member_hover("io", "open");
    assert!(hover.is_some(), "expected hover for io.open");
    if let Some(h) = hover {
        if let HoverContents::Markup(markup) = h.contents {
            assert!(markup.value.contains("io.open"), "expected io.open docs");
            assert!(
                markup.value.contains("IoHandle"),
                "expected IoHandle return type"
            );
        }
    }

    let hover = get_namespace_member_hover("io", "spawn");
    assert!(hover.is_some(), "expected hover for io.spawn");

    let hover = get_namespace_member_hover("io", "tcp_connect");
    assert!(hover.is_some(), "expected hover for io.tcp_connect");
}

#[test]
fn test_hover_time_member() {
    let hover = get_namespace_member_hover("time", "sleep");
    assert!(hover.is_some(), "expected hover for time.sleep");
    if let Some(h) = hover {
        if let HoverContents::Markup(markup) = h.contents {
            assert!(
                markup.value.contains("time.sleep"),
                "expected time.sleep docs"
            );
            assert!(markup.value.contains("Async"), "expected Async mention");
        }
    }

    let hover = get_namespace_member_hover("time", "millis");
    assert!(hover.is_some(), "expected hover for time.millis");
}

#[test]
fn test_hover_namespace_unknown() {
    assert!(get_namespace_api_hover("Foo").is_none());
    assert!(get_namespace_member_hover("io", "nonexistent").is_none());
    assert!(get_namespace_member_hover("DateTime", "nonexistent").is_none());
}
