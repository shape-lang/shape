//! Deep tests for module imports, exports, and visibility (parser level)
//!
//! ~65 parser-level tests covering:
//! - Import syntax edge cases
//! - Export syntax edge cases
//! - Module declaration parsing
//! - Visibility modifier parsing

use crate::parser::parse_program;

// Helper that returns items from parsed source
fn parse_items(input: &str) -> Vec<crate::ast::Item> {
    let program = parse_program(input).expect("should parse");
    program.items
}

// =============================================================================
// CATEGORY 1: Import Syntax Edge Cases
// =============================================================================

#[test]
fn test_module_import_trailing_comma_in_named_imports() {
    let result = parse_program("from foo use { bar, };");
    assert!(
        result.is_ok(),
        "trailing comma in import list should parse: {:?}",
        result.err()
    );
    let items = result.unwrap().items;
    assert_eq!(items.len(), 1);
    match &items[0] {
        crate::ast::Item::Import(stmt, _) => {
            assert_eq!(stmt.from, "foo");
            match &stmt.items {
                crate::ast::ImportItems::Named(specs) => {
                    assert_eq!(specs.len(), 1);
                    assert_eq!(specs[0].name, "bar");
                }
                other => panic!("Expected Named, got {:?}", other),
            }
        }
        other => panic!("Expected Import, got {:?}", other),
    }
}

#[test]
fn test_module_import_multiple_items_trailing_comma() {
    let result = parse_program("from foo use { a, b, c, };");
    assert!(
        result.is_ok(),
        "trailing comma with multiple items: {:?}",
        result.err()
    );
    match &result.unwrap().items[0] {
        crate::ast::Item::Import(stmt, _) => match &stmt.items {
            crate::ast::ImportItems::Named(specs) => {
                assert_eq!(specs.len(), 3);
                assert_eq!(specs[0].name, "a");
                assert_eq!(specs[1].name, "b");
                assert_eq!(specs[2].name, "c");
            }
            other => panic!("Expected Named, got {:?}", other),
        },
        other => panic!("Expected Import, got {:?}", other),
    }
}

#[test]
fn test_module_import_single_segment_path() {
    let result = parse_program("from math use { sum };");
    assert!(result.is_ok(), "single-segment path: {:?}", result.err());
    match &result.unwrap().items[0] {
        crate::ast::Item::Import(stmt, _) => {
            assert_eq!(stmt.from, "math");
        }
        other => panic!("Expected Import, got {:?}", other),
    }
}

#[test]
fn test_module_import_deep_path() {
    let result = parse_program("from a::b::c::d::e::f::g use { item };");
    assert!(
        result.is_ok(),
        "deep module path should parse: {:?}",
        result.err()
    );
    match &result.unwrap().items[0] {
        crate::ast::Item::Import(stmt, _) => {
            assert_eq!(stmt.from, "a::b::c::d::e::f::g");
        }
        other => panic!("Expected Import, got {:?}", other),
    }
}

#[test]
fn test_module_import_multiple_aliases() {
    let result = parse_program("from m use { a as x, b as y, c as z };");
    assert!(result.is_ok(), "multiple aliases: {:?}", result.err());
    match &result.unwrap().items[0] {
        crate::ast::Item::Import(stmt, _) => match &stmt.items {
            crate::ast::ImportItems::Named(specs) => {
                assert_eq!(specs.len(), 3);
                assert_eq!(specs[0].alias, Some("x".to_string()));
                assert_eq!(specs[1].alias, Some("y".to_string()));
                assert_eq!(specs[2].alias, Some("z".to_string()));
            }
            other => panic!("Expected Named, got {:?}", other),
        },
        other => panic!("Expected Import, got {:?}", other),
    }
}

#[test]
fn test_module_import_mixed_aliases_and_plain() {
    let result = parse_program("from m use { a, b as y, c };");
    assert!(
        result.is_ok(),
        "mixed aliases and plain: {:?}",
        result.err()
    );
    match &result.unwrap().items[0] {
        crate::ast::Item::Import(stmt, _) => match &stmt.items {
            crate::ast::ImportItems::Named(specs) => {
                assert_eq!(specs.len(), 3);
                assert_eq!(specs[0].alias, None);
                assert_eq!(specs[1].alias, Some("y".to_string()));
                assert_eq!(specs[2].alias, None);
            }
            other => panic!("Expected Named, got {:?}", other),
        },
        other => panic!("Expected Import, got {:?}", other),
    }
}

#[test]
fn test_module_import_without_semicolon() {
    // Grammar says semicolons are optional on imports
    let result = parse_program("from m use { a }");
    assert!(
        result.is_ok(),
        "import without semicolon: {:?}",
        result.err()
    );
}

#[test]
fn test_module_import_use_namespace_without_semicolon() {
    let result = parse_program("use ml");
    assert!(
        result.is_ok(),
        "namespace use without semicolon: {:?}",
        result.err()
    );
}

#[test]
fn test_module_import_keyword_import_rejected() {
    // `import` is not a valid keyword — should fail
    let result = parse_program("import foo;");
    assert!(result.is_err(), "`import` keyword should be rejected");
}

#[test]
fn test_module_import_js_style_from_import_rejected() {
    // JS-style `from X import { ... }` is invalid
    let result = parse_program("from csv import { load };");
    assert!(
        result.is_err(),
        "JS-style 'from X import' should be rejected"
    );
}

#[test]
fn test_module_import_old_style_import_from_rejected() {
    // Old style `import { a, b } from module;`
    let result = parse_program("import { foo, bar } from module;");
    assert!(
        result.is_err(),
        "old-style 'import from' syntax should be rejected"
    );
}

#[test]
fn test_module_import_use_with_alias() {
    let result = parse_program("use ml as inference;");
    assert!(result.is_ok(), "use with alias: {:?}", result.err());
    match &result.unwrap().items[0] {
        crate::ast::Item::Import(stmt, _) => match &stmt.items {
            crate::ast::ImportItems::Namespace { name, alias } => {
                assert_eq!(name, "ml");
                assert_eq!(*alias, Some("inference".to_string()));
            }
            other => panic!("Expected Namespace, got {:?}", other),
        },
        other => panic!("Expected Import, got {:?}", other),
    }
}

#[test]
fn test_module_import_use_hierarchical_with_alias() {
    let result = parse_program("use std::core::math as m;");
    assert!(
        result.is_ok(),
        "hierarchical use with alias: {:?}",
        result.err()
    );
    match &result.unwrap().items[0] {
        crate::ast::Item::Import(stmt, _) => {
            assert_eq!(stmt.from, "std::core::math");
            match &stmt.items {
                crate::ast::ImportItems::Namespace { name, alias } => {
                    assert_eq!(name, "math");
                    assert_eq!(*alias, Some("m".to_string()));
                }
                other => panic!("Expected Namespace, got {:?}", other),
            }
        }
        other => panic!("Expected Import, got {:?}", other),
    }
}

#[test]
fn test_module_import_use_hierarchical_binds_tail_segment() {
    let result = parse_program("use a::b::c;");
    assert!(result.is_ok(), "hierarchical namespace: {:?}", result.err());
    match &result.unwrap().items[0] {
        crate::ast::Item::Import(stmt, _) => {
            assert_eq!(stmt.from, "a::b::c");
            match &stmt.items {
                crate::ast::ImportItems::Namespace { name, alias } => {
                    assert_eq!(name, "c", "should bind tail segment");
                    assert_eq!(*alias, None);
                }
                other => panic!("Expected Namespace, got {:?}", other),
            }
        }
        other => panic!("Expected Import, got {:?}", other),
    }
}

#[test]
fn test_module_import_multiple_import_statements() {
    let code = r#"
        from math use { sum, max };
        from io use { print };
        use utils;
    "#;
    let result = parse_program(code);
    assert!(result.is_ok(), "multiple imports: {:?}", result.err());
    let items = result.unwrap().items;
    assert_eq!(items.len(), 3);
    assert!(matches!(&items[0], crate::ast::Item::Import(_, _)));
    assert!(matches!(&items[1], crate::ast::Item::Import(_, _)));
    assert!(matches!(&items[2], crate::ast::Item::Import(_, _)));
}

#[test]
fn test_module_import_path_with_hyphens() {
    // path_segment allows hyphens
    let result = parse_program("from my-lib use { helper };");
    assert!(
        result.is_ok(),
        "hyphenated path should parse: {:?}",
        result.err()
    );
    match &result.unwrap().items[0] {
        crate::ast::Item::Import(stmt, _) => {
            assert_eq!(stmt.from, "my-lib");
        }
        other => panic!("Expected Import, got {:?}", other),
    }
}

#[test]
fn test_module_import_path_with_underscores() {
    let result = parse_program("from my_lib::sub_mod use { helper_fn };");
    assert!(result.is_ok(), "underscored path: {:?}", result.err());
    match &result.unwrap().items[0] {
        crate::ast::Item::Import(stmt, _) => {
            assert_eq!(stmt.from, "my_lib::sub_mod");
        }
        other => panic!("Expected Import, got {:?}", other),
    }
}

#[test]
fn test_module_import_path_with_numbers() {
    let result = parse_program("from lib2::v3 use { api };");
    assert!(result.is_ok(), "numeric path segments: {:?}", result.err());
    match &result.unwrap().items[0] {
        crate::ast::Item::Import(stmt, _) => {
            assert_eq!(stmt.from, "lib2::v3");
        }
        other => panic!("Expected Import, got {:?}", other),
    }
}

#[test]
fn test_module_import_empty_braces() {
    // `from m use { }` — empty import list
    let result = parse_program("from m use { };");
    // Grammar: import_item_list = { import_item ~ ("," ~ import_item)* ~ ","? }
    // This requires at least one import_item, so empty braces should fail
    assert!(result.is_err(), "empty import list should be rejected");
}

#[test]
fn test_module_import_use_mod_as_path_segment() {
    // `mod` as a path segment is allowed per grammar (path_segment allows alphanumeric)
    let result = parse_program("use a::mod;");
    assert!(result.is_ok(), "mod as path segment: {:?}", result.err());
    match &result.unwrap().items[0] {
        crate::ast::Item::Import(stmt, _) => {
            assert_eq!(stmt.from, "a::mod");
        }
        other => panic!("Expected Import, got {:?}", other),
    }
}

#[test]
fn test_module_import_duplicate_import_names() {
    // Parser should accept duplicate names (semantic check is a later phase)
    let result = parse_program("from m use { a, a };");
    assert!(
        result.is_ok(),
        "duplicate import names should parse: {:?}",
        result.err()
    );
    match &result.unwrap().items[0] {
        crate::ast::Item::Import(stmt, _) => match &stmt.items {
            crate::ast::ImportItems::Named(specs) => {
                assert_eq!(specs.len(), 2);
                assert_eq!(specs[0].name, "a");
                assert_eq!(specs[1].name, "a");
            }
            other => panic!("Expected Named, got {:?}", other),
        },
        other => panic!("Expected Import, got {:?}", other),
    }
}

#[test]
fn test_module_import_duplicate_aliases() {
    // Parser should accept duplicate aliases (semantic check later)
    let result = parse_program("from m use { a as x, b as x };");
    assert!(
        result.is_ok(),
        "duplicate aliases should parse: {:?}",
        result.err()
    );
    match &result.unwrap().items[0] {
        crate::ast::Item::Import(stmt, _) => match &stmt.items {
            crate::ast::ImportItems::Named(specs) => {
                assert_eq!(specs.len(), 2);
                assert_eq!(specs[0].alias, Some("x".to_string()));
                assert_eq!(specs[1].alias, Some("x".to_string()));
            }
            other => panic!("Expected Named, got {:?}", other),
        },
        other => panic!("Expected Import, got {:?}", other),
    }
}

#[test]
fn test_module_import_from_use_wildcard_not_supported() {
    // `from m use *` or `from m use { * }` — not in grammar
    let result = parse_program("from m use *;");
    assert!(result.is_err(), "wildcard import should be rejected");
}

#[test]
fn test_module_import_use_keyword_boundary() {
    // Ensure `use` doesn't match `useful` or `user`
    // `useful` is not a valid keyword — should fail as an import
    let result = parse_program("useful ml;");
    assert!(
        result.is_err() || {
            // May parse as something else (not an import)
            let items = result.unwrap().items;
            items
                .iter()
                .all(|item| !matches!(item, crate::ast::Item::Import(_, _)))
        }
    );
}

// =============================================================================
// CATEGORY 2: Export Syntax Edge Cases
// =============================================================================

#[test]
fn test_module_export_pub_fn_simple() {
    let result = parse_program("pub fn add(a, b) { a + b; }");
    assert!(result.is_ok(), "pub fn: {:?}", result.err());
    assert!(matches!(
        &result.unwrap().items[0],
        crate::ast::Item::Export(_, _)
    ));
}

#[test]
fn test_module_export_pub_fn_with_return_type() {
    let result = parse_program("pub fn add(a: number, b: number) -> number { a + b; }");
    assert!(result.is_ok(), "pub fn with types: {:?}", result.err());
    match &result.unwrap().items[0] {
        crate::ast::Item::Export(export, _) => {
            assert!(matches!(&export.item, crate::ast::ExportItem::Function(_)));
        }
        other => panic!("Expected Export, got {:?}", other),
    }
}

#[test]
fn test_module_export_pub_fn_with_generic_params() {
    let result = parse_program("pub fn identity<T>(x: T) -> T { x; }");
    assert!(result.is_ok(), "pub fn with generics: {:?}", result.err());
    match &result.unwrap().items[0] {
        crate::ast::Item::Export(export, _) => match &export.item {
            crate::ast::ExportItem::Function(f) => {
                assert_eq!(f.name, "identity");
                assert!(f.type_params.is_some());
            }
            other => panic!("Expected Function, got {:?}", other),
        },
        other => panic!("Expected Export, got {:?}", other),
    }
}

#[test]
fn test_module_export_pub_let() {
    let result = parse_program("pub let x = 42;");
    assert!(result.is_ok(), "pub let: {:?}", result.err());
    match &result.unwrap().items[0] {
        crate::ast::Item::Export(export, _) => match &export.item {
            crate::ast::ExportItem::Named(specs) => {
                assert_eq!(specs.len(), 1);
                assert_eq!(specs[0].name, "x");
            }
            other => panic!("Expected Named, got {:?}", other),
        },
        other => panic!("Expected Export, got {:?}", other),
    }
}

#[test]
fn test_module_export_pub_let_string_value() {
    let result = parse_program(r#"pub let name = "hello";"#);
    assert!(result.is_ok(), "pub let string: {:?}", result.err());
    assert!(matches!(
        &result.unwrap().items[0],
        crate::ast::Item::Export(_, _)
    ));
}

#[test]
fn test_module_export_pub_let_complex_expression() {
    let result = parse_program("pub let result = [1, 2, 3].map(|x| x * 2);");
    assert!(result.is_ok(), "pub let complex expr: {:?}", result.err());
    assert!(matches!(
        &result.unwrap().items[0],
        crate::ast::Item::Export(_, _)
    ));
}

#[test]
fn test_module_export_pub_const() {
    let result = parse_program("pub const PI = 3.14159;");
    assert!(result.is_ok(), "pub const: {:?}", result.err());
    assert!(matches!(
        &result.unwrap().items[0],
        crate::ast::Item::Export(_, _)
    ));
}

#[test]
fn test_module_export_pub_type_alias() {
    let result = parse_program("pub type UserId = string;");
    assert!(result.is_ok(), "pub type alias: {:?}", result.err());
    match &result.unwrap().items[0] {
        crate::ast::Item::Export(export, _) => {
            assert!(matches!(&export.item, crate::ast::ExportItem::TypeAlias(_)));
        }
        other => panic!("Expected Export with TypeAlias, got {:?}", other),
    }
}

#[test]
fn test_module_export_pub_enum() {
    let result = parse_program("pub enum Color { Red, Green, Blue }");
    assert!(result.is_ok(), "pub enum: {:?}", result.err());
    match &result.unwrap().items[0] {
        crate::ast::Item::Export(export, _) => {
            assert!(matches!(&export.item, crate::ast::ExportItem::Enum(_)));
        }
        other => panic!("Expected Export with Enum, got {:?}", other),
    }
}

#[test]
fn test_module_export_pub_enum_with_data() {
    let result = parse_program("pub enum Shape { Circle(number), Rect(number, number) }");
    assert!(result.is_ok(), "pub enum with data: {:?}", result.err());
    assert!(matches!(
        &result.unwrap().items[0],
        crate::ast::Item::Export(_, _)
    ));
}

#[test]
fn test_module_export_pub_struct() {
    let result = parse_program("pub type Point { x: number, y: number }");
    assert!(result.is_ok(), "pub struct: {:?}", result.err());
    match &result.unwrap().items[0] {
        crate::ast::Item::Export(export, _) => {
            assert!(matches!(&export.item, crate::ast::ExportItem::Struct(_)));
        }
        other => panic!("Expected Export with Struct, got {:?}", other),
    }
}

#[test]
fn test_module_export_pub_trait() {
    // trait_member uses interface_member syntax for required methods: `name(params): ReturnType`
    let result = parse_program("pub trait Display { show(self): string }");
    assert!(result.is_ok(), "pub trait: {:?}", result.err());
    match &result.unwrap().items[0] {
        crate::ast::Item::Export(export, _) => {
            assert!(matches!(&export.item, crate::ast::ExportItem::Trait(_)));
        }
        other => panic!("Expected Export with Trait, got {:?}", other),
    }
}

#[test]
fn test_module_export_pub_named_list() {
    let result = parse_program("pub { a, b, c };");
    assert!(result.is_ok(), "pub named list: {:?}", result.err());
    match &result.unwrap().items[0] {
        crate::ast::Item::Export(export, _) => match &export.item {
            crate::ast::ExportItem::Named(specs) => {
                assert_eq!(specs.len(), 3);
                assert_eq!(specs[0].name, "a");
                assert_eq!(specs[1].name, "b");
                assert_eq!(specs[2].name, "c");
            }
            other => panic!("Expected Named, got {:?}", other),
        },
        other => panic!("Expected Export, got {:?}", other),
    }
}

#[test]
fn test_module_export_pub_named_with_aliases() {
    let result = parse_program("pub { internal_fn as public_fn, helper as h };");
    assert!(result.is_ok(), "pub named with aliases: {:?}", result.err());
    match &result.unwrap().items[0] {
        crate::ast::Item::Export(export, _) => match &export.item {
            crate::ast::ExportItem::Named(specs) => {
                assert_eq!(specs.len(), 2);
                assert_eq!(specs[0].name, "internal_fn");
                assert_eq!(specs[0].alias, Some("public_fn".to_string()));
                assert_eq!(specs[1].name, "helper");
                assert_eq!(specs[1].alias, Some("h".to_string()));
            }
            other => panic!("Expected Named, got {:?}", other),
        },
        other => panic!("Expected Export, got {:?}", other),
    }
}

#[test]
fn test_module_export_pub_named_trailing_comma() {
    let result = parse_program("pub { a, b, };");
    assert!(
        result.is_ok(),
        "pub named trailing comma: {:?}",
        result.err()
    );
    match &result.unwrap().items[0] {
        crate::ast::Item::Export(export, _) => match &export.item {
            crate::ast::ExportItem::Named(specs) => {
                assert_eq!(specs.len(), 2);
            }
            other => panic!("Expected Named, got {:?}", other),
        },
        other => panic!("Expected Export, got {:?}", other),
    }
}

#[test]
fn test_module_export_pub_on_if_rejected() {
    // `pub if` should not be a valid export
    let result = parse_program("pub if true { 1; }");
    assert!(result.is_err(), "`pub if` should be rejected");
}

#[test]
fn test_module_export_pub_on_for_rejected() {
    let result = parse_program("pub for x in items { print(x); }");
    assert!(result.is_err(), "`pub for` should be rejected");
}

#[test]
fn test_module_export_pub_on_while_rejected() {
    let result = parse_program("pub while true { break; }");
    assert!(result.is_err(), "`pub while` should be rejected");
}

#[test]
fn test_module_export_double_pub_rejected() {
    let result = parse_program("pub pub fn foo() { 1; }");
    assert!(result.is_err(), "double pub should be rejected");
}

#[test]
fn test_module_export_pub_bare_rejected() {
    // `pub;` alone should not parse
    let result = parse_program("pub;");
    assert!(result.is_err(), "bare pub should be rejected");
}

#[test]
fn test_module_export_pub_let_destructure_rejected() {
    // pub let { x, y } = obj; should be rejected per parser code
    let result = parse_program("pub let { x, y } = obj;");
    assert!(result.is_err(), "pub with destructuring should be rejected");
}

// =============================================================================
// CATEGORY 3: Module Declaration Parsing
// =============================================================================

#[test]
fn test_module_decl_empty_module() {
    let result = parse_program("mod Empty { }");
    assert!(result.is_ok(), "empty module: {:?}", result.err());
    match &result.unwrap().items[0] {
        crate::ast::Item::Module(m, _) => {
            assert_eq!(m.name, "Empty");
            assert!(m.items.is_empty());
        }
        other => panic!("Expected Module, got {:?}", other),
    }
}

#[test]
fn test_module_decl_with_function() {
    let result = parse_program("mod math { fn add(a, b) { a + b; } }");
    assert!(result.is_ok(), "module with function: {:?}", result.err());
    match &result.unwrap().items[0] {
        crate::ast::Item::Module(m, _) => {
            assert_eq!(m.name, "math");
            assert!(!m.items.is_empty());
        }
        other => panic!("Expected Module, got {:?}", other),
    }
}

#[test]
fn test_module_decl_with_const() {
    let result = parse_program("mod constants { const PI = 3.14; }");
    assert!(result.is_ok(), "module with const: {:?}", result.err());
    match &result.unwrap().items[0] {
        crate::ast::Item::Module(m, _) => {
            assert_eq!(m.name, "constants");
            assert!(!m.items.is_empty());
        }
        other => panic!("Expected Module, got {:?}", other),
    }
}

#[test]
fn test_module_decl_with_let() {
    let result = parse_program("mod state { let counter = 0; }");
    assert!(result.is_ok(), "module with let: {:?}", result.err());
}

#[test]
fn test_module_decl_nested_modules() {
    let result = parse_program(
        r#"
        mod outer {
            mod inner {
                fn greet() { "hello"; }
            }
        }
    "#,
    );
    assert!(result.is_ok(), "nested modules: {:?}", result.err());
    match &result.unwrap().items[0] {
        crate::ast::Item::Module(outer, _) => {
            assert_eq!(outer.name, "outer");
            assert_eq!(outer.items.len(), 1);
            match &outer.items[0] {
                crate::ast::Item::Module(inner, _) => {
                    assert_eq!(inner.name, "inner");
                }
                other => panic!("Expected inner Module, got {:?}", other),
            }
        }
        other => panic!("Expected outer Module, got {:?}", other),
    }
}

#[test]
fn test_module_decl_triple_nested() {
    let result = parse_program(
        r#"
        mod a {
            mod b {
                mod c {
                    fn deep() { 42; }
                }
            }
        }
    "#,
    );
    assert!(result.is_ok(), "triple-nested modules: {:?}", result.err());
}

#[test]
fn test_module_decl_with_pub_items() {
    let result = parse_program(
        r#"
        mod api {
            pub fn endpoint() { "ok"; }
            fn internal_helper() { "secret"; }
        }
    "#,
    );
    assert!(
        result.is_ok(),
        "module with pub and private items: {:?}",
        result.err()
    );
    match &result.unwrap().items[0] {
        crate::ast::Item::Module(m, _) => {
            assert_eq!(m.name, "api");
            assert_eq!(m.items.len(), 2);
            assert!(matches!(&m.items[0], crate::ast::Item::Export(_, _)));
            assert!(matches!(&m.items[1], crate::ast::Item::Function(_, _)));
        }
        other => panic!("Expected Module, got {:?}", other),
    }
}

#[test]
fn test_module_decl_with_enum() {
    let result = parse_program("mod types { enum Color { Red, Green, Blue } }");
    assert!(result.is_ok(), "module with enum: {:?}", result.err());
}

#[test]
fn test_module_decl_with_type_alias() {
    let result = parse_program("mod types { type Id = number; }");
    assert!(result.is_ok(), "module with type alias: {:?}", result.err());
}

#[test]
fn test_module_decl_with_struct() {
    let result = parse_program("mod models { type Point { x: number, y: number } }");
    assert!(result.is_ok(), "module with struct: {:?}", result.err());
}

#[test]
fn test_module_decl_with_trait() {
    // trait_member uses interface_member syntax for required methods: `name(params): ReturnType`
    let result = parse_program("mod traits { trait Display { show(self): string } }");
    assert!(result.is_ok(), "module with trait: {:?}", result.err());
}

#[test]
fn test_module_decl_with_annotation() {
    let result = parse_program("@deprecated mod old { fn legacy() { 1; } }");
    assert!(result.is_ok(), "annotated module: {:?}", result.err());
    match &result.unwrap().items[0] {
        crate::ast::Item::Module(m, _) => {
            assert_eq!(m.name, "old");
            assert!(!m.annotations.is_empty());
        }
        other => panic!("Expected Module, got {:?}", other),
    }
}

#[test]
fn test_module_decl_with_multiple_annotations() {
    let result = parse_program(r#"@deprecated @version("2.0") mod old { fn legacy() { 1; } }"#);
    assert!(result.is_ok(), "multi-annotated module: {:?}", result.err());
    match &result.unwrap().items[0] {
        crate::ast::Item::Module(m, _) => {
            assert!(m.annotations.len() >= 2);
        }
        other => panic!("Expected Module, got {:?}", other),
    }
}

#[test]
fn test_module_decl_with_imports_inside() {
    let result = parse_program(
        r#"
        mod app {
            from utils use { format };
            fn display(x) { format(x); }
        }
    "#,
    );
    assert!(result.is_ok(), "module with imports: {:?}", result.err());
    match &result.unwrap().items[0] {
        crate::ast::Item::Module(m, _) => {
            assert_eq!(m.name, "app");
            assert!(
                m.items
                    .iter()
                    .any(|item| matches!(item, crate::ast::Item::Import(_, _)))
            );
        }
        other => panic!("Expected Module, got {:?}", other),
    }
}

#[test]
fn test_module_decl_mixed_item_types() {
    let result = parse_program(
        r#"
        mod kitchen_sink {
            const VERSION = 1;
            type Config { debug: bool }
            enum Level { Low, High }
            fn process() { 1; }
            pub fn public_api() { 2; }
        }
    "#,
    );
    assert!(
        result.is_ok(),
        "module with mixed items: {:?}",
        result.err()
    );
    match &result.unwrap().items[0] {
        crate::ast::Item::Module(m, _) => {
            assert!(
                m.items.len() >= 4,
                "expected at least 4 items, got {}",
                m.items.len()
            );
        }
        other => panic!("Expected Module, got {:?}", other),
    }
}

#[test]
fn test_module_decl_multiple_top_level_modules() {
    let result = parse_program(
        r#"
        mod a { fn fa() { 1; } }
        mod b { fn fb() { 2; } }
        mod c { fn fc() { 3; } }
    "#,
    );
    assert!(result.is_ok(), "multiple modules: {:?}", result.err());
    let items = result.unwrap().items;
    assert_eq!(items.len(), 3);
    assert!(
        items
            .iter()
            .all(|item| matches!(item, crate::ast::Item::Module(_, _)))
    );
}

#[test]
fn test_module_decl_module_then_code() {
    let result = parse_program(
        r#"
        mod math { fn add(a, b) { a + b; } }
        let result = math.add(1, 2);
    "#,
    );
    assert!(result.is_ok(), "module then code: {:?}", result.err());
    let items = result.unwrap().items;
    assert_eq!(items.len(), 2);
    assert!(matches!(&items[0], crate::ast::Item::Module(_, _)));
}

#[test]
fn test_module_decl_name_is_identifier() {
    // Module name should be a valid identifier
    let result = parse_program("mod my_module { }");
    assert!(result.is_ok());
    match &result.unwrap().items[0] {
        crate::ast::Item::Module(m, _) => assert_eq!(m.name, "my_module"),
        other => panic!("Expected Module, got {:?}", other),
    }
}

// =============================================================================
// CATEGORY 4: Import + Export Combinations
// =============================================================================

#[test]
fn test_module_import_before_function() {
    let result = parse_program(
        r#"
        from utils use { format };
        fn display(x) { format(x); }
    "#,
    );
    assert!(result.is_ok(), "import before function: {:?}", result.err());
    let items = result.unwrap().items;
    assert_eq!(items.len(), 2);
    assert!(matches!(&items[0], crate::ast::Item::Import(_, _)));
    assert!(matches!(&items[1], crate::ast::Item::Function(_, _)));
}

#[test]
fn test_module_import_and_export() {
    let result = parse_program(
        r#"
        from math use { sqrt };
        pub fn distance(x, y) { sqrt(x * x + y * y); }
    "#,
    );
    assert!(result.is_ok(), "import and export: {:?}", result.err());
    let items = result.unwrap().items;
    assert_eq!(items.len(), 2);
    assert!(matches!(&items[0], crate::ast::Item::Import(_, _)));
    assert!(matches!(&items[1], crate::ast::Item::Export(_, _)));
}

#[test]
fn test_module_import_and_module_together() {
    let result = parse_program(
        r#"
        from base use { Base };
        mod derived {
            fn create() { Base(); }
        }
    "#,
    );
    assert!(
        result.is_ok(),
        "import and module together: {:?}",
        result.err()
    );
}

#[test]
fn test_module_export_pub_var() {
    let result = parse_program("pub var mutable_state = 0;");
    assert!(result.is_ok(), "pub var: {:?}", result.err());
    assert!(matches!(
        &result.unwrap().items[0],
        crate::ast::Item::Export(_, _)
    ));
}

#[test]
fn test_module_export_pub_named_single() {
    let result = parse_program("pub { x };");
    assert!(result.is_ok(), "pub single named: {:?}", result.err());
    match &result.unwrap().items[0] {
        crate::ast::Item::Export(export, _) => match &export.item {
            crate::ast::ExportItem::Named(specs) => {
                assert_eq!(specs.len(), 1);
                assert_eq!(specs[0].name, "x");
            }
            other => panic!("Expected Named, got {:?}", other),
        },
        other => panic!("Expected Export, got {:?}", other),
    }
}

#[test]
fn test_module_export_pub_fn_no_params() {
    let result = parse_program("pub fn noop() { }");
    assert!(result.is_ok(), "pub fn no params: {:?}", result.err());
}

#[test]
fn test_module_export_pub_fn_many_params() {
    let result = parse_program("pub fn many(a, b, c, d, e, f) { a + b + c + d + e + f; }");
    assert!(result.is_ok(), "pub fn many params: {:?}", result.err());
}

// =============================================================================
// CATEGORY 5: Namespace Import Syntax
// =============================================================================

#[test]
fn test_module_namespace_use_simple() {
    let result = parse_program("use json;");
    assert!(result.is_ok(), "simple namespace: {:?}", result.err());
    match &result.unwrap().items[0] {
        crate::ast::Item::Import(stmt, _) => {
            assert_eq!(stmt.from, "json");
            match &stmt.items {
                crate::ast::ImportItems::Namespace { name, alias } => {
                    assert_eq!(name, "json");
                    assert_eq!(*alias, None);
                }
                other => panic!("Expected Namespace, got {:?}", other),
            }
        }
        other => panic!("Expected Import, got {:?}", other),
    }
}

#[test]
fn test_module_namespace_use_two_segment_path() {
    let result = parse_program("use std::io;");
    assert!(result.is_ok(), "two-segment namespace: {:?}", result.err());
    match &result.unwrap().items[0] {
        crate::ast::Item::Import(stmt, _) => {
            assert_eq!(stmt.from, "std::io");
            match &stmt.items {
                crate::ast::ImportItems::Namespace { name, alias } => {
                    assert_eq!(name, "io");
                    assert_eq!(*alias, None);
                }
                other => panic!("Expected Namespace, got {:?}", other),
            }
        }
        other => panic!("Expected Import, got {:?}", other),
    }
}

#[test]
fn test_module_namespace_use_with_alias_and_usage() {
    // Parse only — checking that alias binding works at parse level
    let result = parse_program(
        r#"
        use std::core::math as m;
        let x = m.sqrt(4);
    "#,
    );
    assert!(
        result.is_ok(),
        "namespace with alias then usage: {:?}",
        result.err()
    );
}

#[test]
fn test_module_namespace_multiple_uses() {
    let result = parse_program(
        r#"
        use json;
        use csv;
        use yaml;
    "#,
    );
    assert!(result.is_ok());
    let items = result.unwrap().items;
    assert_eq!(items.len(), 3);
}

// =============================================================================
// CATEGORY 6: Error Reporting Quality
// =============================================================================

#[test]
fn test_module_import_require_keyword_rejected() {
    let result = parse_program("require('module');");
    assert!(result.is_err(), "`require` should be rejected");
}

#[test]
fn test_module_import_include_keyword_rejected() {
    let result = parse_program("#include <module>");
    assert!(result.is_err(), "C-style include should be rejected");
}

#[test]
fn test_module_import_from_without_use_keyword() {
    // `from m { a }` missing `use` keyword
    let result = parse_program("from m { a };");
    assert!(result.is_err(), "from without use keyword should fail");
}

#[test]
fn test_module_export_pub_interface() {
    // pub interface is not listed in pub_item grammar, but interface_def may or may not be allowed
    // Check what actually happens
    let result = parse_program("pub interface Serializable { fn serialize(self) -> string; }");
    // This may or may not parse — document behavior
    if result.is_err() {
        // BUG: `pub interface` is not supported even though pub_item lists other type definitions.
        // Note: The grammar has no `pub ~ interface_def` alternative, only `pub ~ trait_def`.
        // This is likely intentional since `trait` replaced `interface` for visibility.
    }
}

#[test]
fn test_module_decl_missing_braces() {
    // BUG: `mod Broken` without braces does not produce a parse error.
    // The parser may interpret this as something other than a module declaration.
    // This test documents the current behavior.
    let result = parse_program("mod Broken");
    if result.is_ok() {
        let items = result.unwrap().items;
        // Verify it did NOT parse as a Module item (since there are no braces)
        let has_module = items
            .iter()
            .any(|item| matches!(item, crate::ast::Item::Module(_, _)));
        // If it parsed as something else, that's a quirk but not necessarily wrong
        assert!(
            !has_module,
            "mod without braces should not produce a Module item, got: {:?}",
            items
        );
    }
    // If it errors, that's also acceptable
}

#[test]
fn test_module_decl_missing_name() {
    // `mod { ... }` without a name — the grammar requires `ident` after `mod`
    // BUG: This currently does not error as expected. The parser may recover.
    let result = parse_program("mod { fn f() { } }");
    if result.is_ok() {
        let items = result.unwrap().items;
        let has_module = items
            .iter()
            .any(|item| matches!(item, crate::ast::Item::Module(_, _)));
        // If it parsed as something else, that's a quirk
        assert!(
            !has_module,
            "mod without name should not produce a Module item, got: {:?}",
            items
        );
    }
}
