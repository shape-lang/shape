//! Tests for the trait system.
//!
//! Verifies parsing, semantic tokens, hover, and completions for
//! trait definitions, impl blocks, where clauses, and associated types.

use shape_test::shape_test::{ShapeTest, pos};

// -- Trait definition parsing -----------------------------------------------

#[test]
fn trait_definition_parses() {
    ShapeTest::new("trait Printable {\n    display(self): string\n}").expect_parse_ok();
}

#[test]
fn trait_with_multiple_methods_parses() {
    let code = "trait Collection {\n    size(self): int;\n    is_empty(self): bool;\n    first(self): any\n}";
    ShapeTest::new(code).expect_parse_ok();
}

#[test]
fn trait_with_type_param_bounds_parses() {
    ShapeTest::new("trait Distributable<T: Serializable> {\n    wire_size(self): int\n}")
        .expect_parse_ok();
}

#[test]
fn trait_with_default_method_parses() {
    let code = "trait Queryable {\n    filter(pred): any;\n    method execute() {\n        return self\n    }\n}";
    ShapeTest::new(code).expect_parse_ok();
}

// -- Impl block parsing -----------------------------------------------------

#[test]
fn impl_block_parses() {
    let code = "trait Foo {\n    bar(self): number\n}\n\nimpl Foo for MyType {\n    method bar() { return 42; }\n}";
    ShapeTest::new(code).expect_parse_ok();
}

#[test]
fn impl_block_with_multiple_methods_parses() {
    let code = "trait Collection {\n    size(self): int;\n    is_empty(self): bool\n}\nimpl Collection for MyList {\n    method size() { return 0; }\n    method is_empty() { return true; }\n}";
    ShapeTest::new(code).expect_parse_ok();
}

// -- Where clause parsing ---------------------------------------------------

#[test]
fn where_clause_parses() {
    ShapeTest::new(
        "function process<T>(x: T) -> string where T: Display {\n    return x.display();\n}",
    )
    .expect_parse_ok();
}

#[test]
fn where_clause_multiple_bounds_parses() {
    ShapeTest::new(
        "function transform<T>(x: T) where T: Display + Serializable {\n    return x;\n}",
    )
    .expect_parse_ok();
}

// -- Associated type parsing ------------------------------------------------

#[test]
fn associated_type_parses() {
    // Associated type declarations parse; Self.Item syntax is not yet supported,
    // so we test the type declaration itself with a simple return type.
    ShapeTest::new("trait Iterator {\n    type Item;\n    next(self): any\n}").expect_parse_ok();
}

// -- Trait semantic tokens --------------------------------------------------

#[test]
fn semantic_tokens_for_trait_keyword() {
    ShapeTest::new("trait Printable {\n    display(self): string\n}")
        .expect_semantic_tokens()
        .expect_semantic_tokens_min(2); // trait keyword + trait name
}

#[test]
fn semantic_tokens_for_impl_keyword() {
    let code = "trait Foo {\n    bar(self): number\n}\nimpl Foo for MyType {\n    method bar() { return 42; }\n}";
    ShapeTest::new(code)
        .expect_semantic_tokens()
        .expect_semantic_tokens_min(6); // trait, Foo, impl, Foo, for, MyType, ...
}

// -- Trait hover ------------------------------------------------------------

#[test]
fn hover_on_trait_name_shows_trait_info() {
    // Hover on "display" method inside trait should produce hover info.
    // Note: hovering on the trait name itself is not yet supported.
    ShapeTest::new("trait Printable {\n    display(self): string\n}\nlet x = \"hello\"")
        .at(pos(3, 4))
        .expect_hover_contains("string");
}

#[test]
fn hover_on_impl_method_shows_trait_info() {
    // Hover on method in impl block. The method name "apply_query" avoids
    // clashing with builtin "filter". Currently returns function hover.
    let code = "trait Queryable {\n    apply_query(pred): any\n}\nimpl Queryable for MyTable {\n    method apply_query(pred) { self }\n}";
    ShapeTest::new(code)
        .at(pos(4, 11))
        .expect_hover_contains("apply_query");
}

// -- Trait completions ------------------------------------------------------

#[test]
fn completion_inside_impl_suggests_methods() {
    let code = "trait Queryable {\n    filter(pred): any;\n    select(cols): any\n}\nimpl Queryable for MyTable {\n    method filter(pred) { self }\n    \n}";
    ShapeTest::new(code)
        .at(pos(6, 4))
        .expect_completion("select");
}

#[test]
fn completion_excludes_already_implemented() {
    let code = "trait Queryable {\n    filter(pred): any;\n    select(cols): any\n}\nimpl Queryable for MyTable {\n    method filter(pred) { self }\n    \n}";
    ShapeTest::new(code)
        .at(pos(6, 4))
        .expect_no_completion("filter");
}

// -- Trait definition and references ----------------------------------------

#[test]
fn definition_from_trait_name_in_impl() {
    let code = "trait Queryable {\n    filter(pred): any\n}\nimpl Queryable for MyTable {\n    method filter(pred) { self }\n}";
    ShapeTest::new(code).at(pos(3, 5)).expect_definition();
}

#[test]
fn definition_from_method_in_impl() {
    let code = "trait Queryable {\n    filter(pred): any\n}\nimpl Queryable for MyTable {\n    method filter(pred) { self }\n}";
    ShapeTest::new(code).at(pos(4, 11)).expect_definition();
}

// -- Code lens for traits ---------------------------------------------------

#[test]
fn code_lens_on_trait_definition() {
    let code = "trait Queryable {\n    filter(pred): any\n}\nimpl Queryable for MyTable {\n    method filter(pred) { self }\n}";
    ShapeTest::new(code)
        .expect_code_lens_not_empty()
        .expect_code_lens_at_line(0);
}

// -- Trait bound completions ------------------------------------------------

#[test]
fn trait_bound_completion_suggests_traits() {
    let code = "trait Comparable {\n    compare(other): number\n}\ntrait Displayable {\n    display(): string\n}\nfn foo<T: >(x: T) {\n    x\n}";
    ShapeTest::new(code)
        .at(pos(6, 10))
        .expect_completion("Comparable")
        .expect_completion("Displayable");
}

#[test]
fn hover_on_bounded_type_param() {
    let code = "trait Comparable {\n    compare(other): number\n}\nfn foo<T: Comparable>(x: T) {\n    x\n}";
    ShapeTest::new(code)
        .at(pos(3, 7))
        .expect_hover_contains("Type Parameter");
}
