//! Type system parsing tests
//!
//! This module contains tests for:
//! - Type annotations and aliases
//! - Interface definitions
//! - Enum definitions
//! - Intersection types
//! - Meta/format definitions
//! - Type casting

use super::super::*;
use crate::error::{Result, ShapeError};

/// Helper to parse a full program
fn parse_program_helper(input: &str) -> Result<Vec<crate::ast::Item>> {
    let pairs = ShapeParser::parse(Rule::program, input).map_err(|e| ShapeError::ParseError {
        message: e.to_string(),
        location: None,
    })?;

    let mut items = Vec::new();
    for pair in pairs {
        if pair.as_rule() == Rule::program {
            for inner in pair.into_inner() {
                if let Rule::item = inner.as_rule() {
                    items.push(parse_item(inner)?);
                }
            }
        }
    }
    Ok(items)
}

// =========================================================================
// Type Annotation Tests
// =========================================================================

#[test]
fn test_variable_decl_with_type() {
    let result = parse_program_helper("let x: number = 42;");
    assert!(
        result.is_ok(),
        "Let with type should parse: {:?}",
        result.err()
    );
}

#[test]
fn test_variable_decl_with_vec_type() {
    let items = parse_program_helper("let xs: Vec<number> = [1, 2, 3];")
        .expect("Vec<T> alias should parse");
    match &items[0] {
        crate::ast::Item::Statement(crate::ast::Statement::VariableDecl(decl, _), _) => {
            let ann = decl
                .type_annotation
                .as_ref()
                .expect("expected type annotation");
            match ann {
                crate::ast::TypeAnnotation::Array(inner) => {
                    assert!(
                        matches!(inner.as_ref(), crate::ast::TypeAnnotation::Basic(name) if name == "number"),
                        "Vec<number> should canonicalize to Vec<number>, got {:?}",
                        ann
                    );
                }
                other => panic!("Expected Array type annotation for Vec<T>, got {:?}", other),
            }
        }
        other => panic!("Expected Statement(VariableDecl), got {:?}", other),
    }
}

#[test]
fn test_function_with_return_type() {
    let result =
        parse_program_helper("function add(a: number, b: number) -> number { return a + b; }");
    assert!(
        result.is_ok(),
        "Function with types should parse: {:?}",
        result.err()
    );
}

#[test]
fn test_array_generic_is_alias_for_vec() {
    let result = parse_program_helper("let xs: Array<number> = [1, 2, 3];");
    assert!(
        result.is_ok(),
        "Array<T> should be accepted as alias for Vec<T>"
    );
}

#[test]
fn test_matrix_generic_is_rejected() {
    let result = parse_program_helper("let m: Matrix<number> = [];");
    assert!(result.is_err(), "Matrix<T> should be rejected");
    let msg = format!("{:?}", result.err());
    assert!(
        msg.contains("Matrix<T> has been removed; use Mat<T> instead"),
        "Expected Mat migration error, got {}",
        msg
    );
}

// =========================================================================
// Typed Object Tests
// =========================================================================

#[test]
fn test_typed_object_literals() {
    let result = parse_program_helper("let obj = { x: number = 1, y: string = \"a\" };");
    assert!(
        result.is_ok(),
        "Typed object literals should parse: {:?}",
        result.err()
    );

    let items = result.unwrap();
    if let crate::ast::Item::Statement(crate::ast::Statement::VariableDecl(decl, _), _) = &items[0]
    {
        let value = decl.value.as_ref().expect("Expected value in decl");
        if let crate::ast::Expr::Object(entries, _) = value {
            if let crate::ast::ObjectEntry::Field {
                key,
                type_annotation,
                ..
            } = &entries[0]
            {
                assert_eq!(key, "x");
                assert!(
                    type_annotation.is_some(),
                    "Expected type annotation on field"
                );
            } else {
                panic!("Expected ObjectEntry::Field");
            }
        } else {
            panic!("Expected object literal");
        }
    } else {
        panic!("Expected Statement(VariableDecl)");
    }
}

#[test]
fn test_function_param_typed_defaults_parse() {
    let result = parse_program_helper("fn add(a: int = 1, b: int = 2) -> int { return a + b }");
    assert!(
        result.is_ok(),
        "Failed to parse typed defaults: {:?}",
        result
    );

    let items = result.unwrap();
    match &items[0] {
        crate::ast::Item::Function(func, _) => {
            assert_eq!(func.params.len(), 2);
            assert!(
                func.params[0].default_value.is_some(),
                "first param default missing"
            );
            assert!(
                func.params[1].default_value.is_some(),
                "second param default missing"
            );
        }
        other => panic!("Expected function, got {:?}", other),
    }
}

// =========================================================================
// Removed Language Surface Tests
// =========================================================================

#[test]
fn test_interface_definition_is_accepted() {
    // The grammar supports `interface` as a valid item keyword.
    let content = r#"
interface CandleLike {
    timestamp: timestamp
}
"#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "interface keyword should parse successfully: {:?}",
        result.err()
    );
    let items = result.unwrap();
    assert!(
        !items.is_empty(),
        "interface definition should produce at least one AST item"
    );
}

#[test]
fn test_trait_extends_syntax_is_rejected() {
    let content = r#"
trait Distributable extends Serializable {
    wire_size(self): int
}
"#;
    let result = parse_program_helper(content);
    match result {
        Err(_) => {}
        Ok(items) => assert!(
            items.is_empty(),
            "trait extends syntax should not produce AST items, got: {:?}",
            items
        ),
    }
}

#[test]
fn test_type_param_extends_syntax_is_rejected() {
    let content = r#"
fn foo<T extends Numeric>(x: T) {
    return x
}
"#;
    let result = parse_program_helper(content);
    match result {
        Err(_) => {}
        Ok(items) => assert!(
            items.is_empty(),
            "type parameter extends syntax should be rejected, got: {:?}",
            items
        ),
    }
}

// =========================================================================
// Type Alias Tests
// =========================================================================

#[test]
fn test_type_alias_simple() {
    let content = r#"
        type Percent = Number;
    "#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "Simple type alias should parse: {:?}",
        result.err()
    );

    let items = result.unwrap();
    if let crate::ast::Item::TypeAlias(alias, _) = &items[0] {
        assert_eq!(alias.name, "Percent");
        assert!(alias.meta_param_overrides.is_none());
    } else {
        panic!("Expected TypeAlias, got {:?}", items[0]);
    }
}

#[test]
fn test_type_alias_object_with_commas() {
    let content = r#"
        type Point = { x: number, y: number, label?: string };
    "#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "Type alias with comma-separated object type should parse: {:?}",
        result.err()
    );

    let items = result.unwrap();
    if let crate::ast::Item::TypeAlias(alias, _) = &items[0] {
        match &alias.type_annotation {
            crate::ast::TypeAnnotation::Object(fields) => {
                assert_eq!(fields.len(), 3);
            }
            other => panic!("Expected object type annotation, got {:?}", other),
        }
    } else {
        panic!("Expected TypeAlias");
    }
}

#[test]
fn test_type_alias_with_meta_param_overrides() {
    let content = r#"
        type Percent4 = Percent { decimals: 4 };
    "#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "Type alias with param overrides should parse: {:?}",
        result.err()
    );

    let items = result.unwrap();
    if let crate::ast::Item::TypeAlias(alias, _) = &items[0] {
        assert_eq!(alias.name, "Percent4");
        assert!(
            alias.meta_param_overrides.is_some(),
            "Expected meta_param_overrides"
        );
        let overrides = alias.meta_param_overrides.as_ref().unwrap();
        assert!(
            overrides.contains_key("decimals"),
            "Expected 'decimals' key"
        );

        // Verify the value is 4 (parsed as Int)
        if let crate::ast::Expr::Literal(crate::ast::Literal::Int(n), _) = &overrides["decimals"] {
            assert_eq!(*n, 4);
        } else {
            panic!("Expected Int literal, got: {:?}", overrides["decimals"]);
        }
    } else {
        panic!("Expected TypeAlias, got {:?}", items[0]);
    }
}

#[test]
fn test_type_alias_with_multiple_param_overrides() {
    let content = r#"
        type EUR = Currency { symbol: "€", decimals: 2 };
    "#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "Type alias with multiple overrides should parse: {:?}",
        result.err()
    );

    let items = result.unwrap();
    if let crate::ast::Item::TypeAlias(alias, _) = &items[0] {
        assert_eq!(alias.name, "EUR");
        let overrides = alias.meta_param_overrides.as_ref().unwrap();
        assert_eq!(overrides.len(), 2);
        assert!(overrides.contains_key("symbol"));
        assert!(overrides.contains_key("decimals"));
    } else {
        panic!("Expected TypeAlias");
    }
}

// =========================================================================
// Intersection Type Tests
// =========================================================================

#[test]
fn test_intersection_type_simple() {
    let content = r#"
        type Combined = TypeA + TypeB;
    "#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "Intersection type should parse: {:?}",
        result.err()
    );

    let items = result.unwrap();
    if let crate::ast::Item::TypeAlias(alias, _) = &items[0] {
        assert_eq!(alias.name, "Combined");
        match &alias.type_annotation {
            crate::ast::TypeAnnotation::Intersection(types) => {
                assert_eq!(types.len(), 2);
            }
            other => panic!("Expected Intersection type, got {:?}", other),
        }
    } else {
        panic!("Expected TypeAlias");
    }
}

#[test]
fn test_intersection_type_with_objects() {
    let content = r#"
        type Combined = { x: number } + { y: string };
    "#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "Intersection type with objects should parse: {:?}",
        result.err()
    );

    let items = result.unwrap();
    if let crate::ast::Item::TypeAlias(alias, _) = &items[0] {
        match &alias.type_annotation {
            crate::ast::TypeAnnotation::Intersection(types) => {
                assert_eq!(types.len(), 2);
                assert!(matches!(types[0], crate::ast::TypeAnnotation::Object(_)));
                assert!(matches!(types[1], crate::ast::TypeAnnotation::Object(_)));
            }
            other => panic!("Expected Intersection type, got {:?}", other),
        }
    } else {
        panic!("Expected TypeAlias");
    }
}

#[test]
fn test_intersection_type_multiple() {
    let content = r#"
        type All = A + B + C;
    "#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "Multiple intersection types should parse: {:?}",
        result.err()
    );

    let items = result.unwrap();
    if let crate::ast::Item::TypeAlias(alias, _) = &items[0] {
        match &alias.type_annotation {
            crate::ast::TypeAnnotation::Intersection(types) => {
                assert_eq!(types.len(), 3);
            }
            other => panic!(
                "Expected Intersection type with 3 elements, got {:?}",
                other
            ),
        }
    } else {
        panic!("Expected TypeAlias");
    }
}

#[test]
fn test_intersection_combined_with_union() {
    // Intersection has higher precedence than union
    // A | B + C should parse as A | (B + C)
    let content = r#"
        type Mixed = A | B + C;
    "#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "Union with intersection should parse: {:?}",
        result.err()
    );

    let items = result.unwrap();
    if let crate::ast::Item::TypeAlias(alias, _) = &items[0] {
        match &alias.type_annotation {
            crate::ast::TypeAnnotation::Union(types) => {
                assert_eq!(types.len(), 2);
                // Second element should be an intersection
                assert!(matches!(
                    types[1],
                    crate::ast::TypeAnnotation::Intersection(_)
                ));
            }
            other => panic!("Expected Union type, got {:?}", other),
        }
    } else {
        panic!("Expected TypeAlias");
    }
}

#[test]
fn test_function_param_with_intersection_type() {
    let content = r#"
        function process(data: TypeA + TypeB) -> number {
            return 42
        }
    "#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "Function with intersection param should parse: {:?}",
        result.err()
    );
}

// =========================================================================
// Enum Definition Tests
// =========================================================================

#[test]
fn test_enum_rust_style_variants() {
    let content = r#"
        enum Signal {
            Buy,
            Sell = "sell",
            Hold = 0,
            Limit { price: number, size: number },
            Market(number, number)
        }
    "#;
    let items = parse_program_helper(content).expect("enum should parse");
    match &items[0] {
        crate::ast::Item::Enum(enum_def, _) => {
            assert_eq!(enum_def.name, "Signal");
            assert_eq!(enum_def.members.len(), 5);
            assert!(matches!(
                enum_def.members[0].kind,
                crate::ast::EnumMemberKind::Unit { value: None }
            ));
            assert!(matches!(
                enum_def.members[1].kind,
                crate::ast::EnumMemberKind::Unit {
                    value: Some(crate::ast::EnumValue::String(ref value))
                } if value == "sell"
            ));
            assert!(matches!(
                enum_def.members[2].kind,
                crate::ast::EnumMemberKind::Unit {
                    value: Some(crate::ast::EnumValue::Number(value))
                } if value == 0.0
            ));
            assert!(matches!(
                enum_def.members[3].kind,
                crate::ast::EnumMemberKind::Struct(_)
            ));
            assert!(matches!(
                enum_def.members[4].kind,
                crate::ast::EnumMemberKind::Tuple(_)
            ));
        }
        other => panic!("Expected Enum item, got {:?}", other),
    }
}

#[test]
fn test_enum_constructor_expressions() {
    let content = r#"
        let a = Signal::Buy;
        let b = Signal::Limit { price: 1, size: 2 };
        let c = Signal::Market(1, 2);
    "#;
    let items = parse_program_helper(content).expect("enum constructors should parse");
    for item in items {
        if let crate::ast::Item::Statement(crate::ast::Statement::VariableDecl(decl, _), _) = item {
            let value = decl.value.as_ref().expect("expected value");
            match value {
                crate::ast::Expr::EnumConstructor {
                    enum_name,
                    variant,
                    payload,
                    ..
                } => {
                    assert_eq!(enum_name, "Signal");
                    assert!(variant == "Buy" || variant == "Limit");
                    match (variant.as_str(), payload) {
                        ("Buy", crate::ast::EnumConstructorPayload::Unit) => {}
                        ("Limit", crate::ast::EnumConstructorPayload::Struct(fields)) => {
                            assert_eq!(fields.len(), 2);
                        }
                        _ => panic!("Unexpected payload for variant {}", variant),
                    }
                }
                // The parser can't distinguish tuple enum constructors from
                // qualified function calls without type information, so
                // Signal::Market(1, 2) parses as a QualifiedFunctionCall.
                crate::ast::Expr::QualifiedFunctionCall {
                    namespace,
                    function,
                    args,
                    ..
                } => {
                    assert_eq!(namespace, "Signal");
                    assert_eq!(function, "Market");
                    assert_eq!(args.len(), 2);
                }
                other => panic!("Expected EnumConstructor or QualifiedFunctionCall, got {:?}", other),
            }
        }
    }
}

#[test]
fn test_enum_match_pattern_qualified() {
    let content = r#"
        let x = match signal {
            Signal::Buy => 1,
            Signal::Limit { price: p } => p
        };
    "#;
    let items = parse_program_helper(content).expect("enum match should parse");
    if let crate::ast::Item::Statement(crate::ast::Statement::VariableDecl(decl, _), _) = &items[0]
    {
        let value = decl.value.as_ref().expect("expected value");
        if let crate::ast::Expr::Match(match_expr, _) = value {
            assert_eq!(match_expr.arms.len(), 2);
            if let crate::ast::Pattern::Constructor {
                enum_name, variant, ..
            } = &match_expr.arms[0].pattern
            {
                assert_eq!(enum_name.as_deref(), Some("Signal"));
                assert_eq!(variant, "Buy");
            } else {
                panic!("Expected constructor pattern");
            }
        } else {
            panic!("Expected match expression");
        }
    } else {
        panic!("Expected Statement(VariableDecl)");
    }
}

// =========================================================================
// Struct Type Definition Tests
// =========================================================================

#[test]
fn test_struct_type_def_simple() {
    let content = r#"
        type Point { x: number, y: number }
    "#;
    let items = parse_program_helper(content).expect("struct type should parse");
    match &items[0] {
        crate::ast::Item::StructType(def, _) => {
            assert_eq!(def.name, "Point");
            assert!(def.type_params.is_none());
            assert_eq!(def.fields.len(), 2);
            assert_eq!(def.fields[0].name, "x");
            assert_eq!(def.fields[1].name, "y");
        }
        other => panic!("Expected StructType, got {:?}", other),
    }
}

#[test]
fn test_struct_type_def_with_generics() {
    let content = r#"
        type DataVec<V, K> { index: Vec<K>, data: Vec<V> }
    "#;
    let items = parse_program_helper(content).expect("generic struct type should parse");
    match &items[0] {
        crate::ast::Item::StructType(def, _) => {
            assert_eq!(def.name, "DataVec");
            let params = def.type_params.as_ref().expect("expected type params");
            assert_eq!(params.len(), 2);
            assert_eq!(params[0].name, "V");
            assert_eq!(params[1].name, "K");
            assert_eq!(def.fields.len(), 2);
            assert_eq!(def.fields[0].name, "index");
            assert_eq!(def.fields[1].name, "data");
        }
        other => panic!("Expected StructType, got {:?}", other),
    }
}

#[test]
fn test_struct_type_def_trailing_comma() {
    let content = r#"
        type Color { r: number, g: number, b: number, }
    "#;
    let items = parse_program_helper(content).expect("struct with trailing comma should parse");
    match &items[0] {
        crate::ast::Item::StructType(def, _) => {
            assert_eq!(def.name, "Color");
            assert_eq!(def.fields.len(), 3);
        }
        other => panic!("Expected StructType, got {:?}", other),
    }
}

#[test]
fn test_struct_type_def_single_field() {
    let content = r#"
        type Wrapper { value: number }
    "#;
    let items = parse_program_helper(content).expect("single-field struct should parse");
    match &items[0] {
        crate::ast::Item::StructType(def, _) => {
            assert_eq!(def.name, "Wrapper");
            assert_eq!(def.fields.len(), 1);
            assert_eq!(def.fields[0].name, "value");
        }
        other => panic!("Expected StructType, got {:?}", other),
    }
}

#[test]
fn test_struct_field_annotations() {
    let content = r#"
        type Trade {
            @alias("Close Price")
            close: number,
            @alias("vol.")
            @format("currency")
            volume: number,
            timestamp: number,
        }
    "#;
    let items = parse_program_helper(content).expect("struct with field annotations should parse");
    match &items[0] {
        crate::ast::Item::StructType(def, _) => {
            assert_eq!(def.name, "Trade");
            assert_eq!(def.fields.len(), 3);

            // First field: one annotation
            assert_eq!(def.fields[0].name, "close");
            assert_eq!(def.fields[0].annotations.len(), 1);
            assert_eq!(def.fields[0].annotations[0].name, "alias");
            assert_eq!(def.fields[0].annotations[0].args.len(), 1);

            // Second field: two annotations
            assert_eq!(def.fields[1].name, "volume");
            assert_eq!(def.fields[1].annotations.len(), 2);
            assert_eq!(def.fields[1].annotations[0].name, "alias");
            assert_eq!(def.fields[1].annotations[1].name, "format");

            // Third field: no annotations
            assert_eq!(def.fields[2].name, "timestamp");
            assert!(def.fields[2].annotations.is_empty());
        }
        other => panic!("Expected StructType, got {:?}", other),
    }
}

// =========================================================================
// Struct Literal Tests
// =========================================================================

#[test]
fn test_struct_literal_simple() {
    let content = r#"
        let p = Point { x: 3, y: 4 };
    "#;
    let items = parse_program_helper(content).expect("struct literal should parse");
    if let crate::ast::Item::Statement(crate::ast::Statement::VariableDecl(decl, _), _) = &items[0]
    {
        let value = decl.value.as_ref().expect("expected value");
        if let crate::ast::Expr::StructLiteral {
            type_name, fields, ..
        } = value
        {
            assert_eq!(type_name, "Point");
            assert_eq!(fields.len(), 2);
            assert_eq!(fields[0].0, "x");
            assert_eq!(fields[1].0, "y");
        } else {
            panic!("Expected StructLiteral, got {:?}", value);
        }
    } else {
        panic!("Expected Statement(VariableDecl)");
    }
}

#[test]
fn test_struct_literal_empty() {
    let content = r#"
        let e = Empty {};
    "#;
    let items = parse_program_helper(content).expect("empty struct literal should parse");
    if let crate::ast::Item::Statement(crate::ast::Statement::VariableDecl(decl, _), _) = &items[0]
    {
        let value = decl.value.as_ref().expect("expected value");
        if let crate::ast::Expr::StructLiteral {
            type_name, fields, ..
        } = value
        {
            assert_eq!(type_name, "Empty");
            assert_eq!(fields.len(), 0);
        } else {
            panic!("Expected StructLiteral, got {:?}", value);
        }
    } else {
        panic!("Expected Statement(VariableDecl)");
    }
}

// =========================================================================
// Meta/Format Definition Tests (continued)
// =========================================================================

#[test]
fn test_meta_param_override_in_as_cast() {
    let content = r#"
        let y = 0.15 as Percent { decimals: 4 };
    "#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "As cast with param override should parse: {:?}",
        result.err()
    );

    let items = result.unwrap();
    // Parse wraps variable declarations in Statement
    if let crate::ast::Item::Statement(crate::ast::Statement::VariableDecl(decl, _), _) = &items[0]
    {
        if let Some(ref value) = decl.value {
            if let crate::ast::Expr::TypeAssertion {
                meta_param_overrides,
                ..
            } = value
            {
                assert!(
                    meta_param_overrides.is_some(),
                    "Expected meta_param_overrides"
                );
                let overrides = meta_param_overrides.as_ref().unwrap();
                assert!(
                    overrides.contains_key("decimals"),
                    "Expected 'decimals' key"
                );
            } else {
                panic!("Expected TypeAssertion, got {:?}", value);
            }
        } else {
            panic!("Expected value in decl");
        }
    } else {
        panic!("Expected Statement(VariableDecl), got {:?}", items[0]);
    }
}

// =========================================================================
// Comptime Field Tests
// =========================================================================

#[test]
fn test_struct_comptime_field_basic() {
    let content = r#"
        type Currency {
            comptime symbol: string = "$",
            amount: number
        }
    "#;
    let items = parse_program_helper(content).expect("struct with comptime field should parse");
    match &items[0] {
        crate::ast::Item::StructType(def, _) => {
            assert_eq!(def.name, "Currency");
            assert_eq!(def.fields.len(), 2);

            // First field: comptime
            assert_eq!(def.fields[0].name, "symbol");
            assert!(def.fields[0].is_comptime);
            assert!(def.fields[0].default_value.is_some());
            if let Some(crate::ast::Expr::Literal(crate::ast::Literal::String(s), _)) =
                &def.fields[0].default_value
            {
                assert_eq!(s, "$");
            } else {
                panic!(
                    "Expected string literal default, got {:?}",
                    def.fields[0].default_value
                );
            }

            // Second field: runtime
            assert_eq!(def.fields[1].name, "amount");
            assert!(!def.fields[1].is_comptime);
            assert!(def.fields[1].default_value.is_none());
        }
        other => panic!("Expected StructType, got {:?}", other),
    }
}

#[test]
fn test_struct_comptime_field_with_annotations() {
    let content = r#"
        type Metric {
            @label("Unit")
            comptime unit: string = "meters",
            value: number
        }
    "#;
    let items =
        parse_program_helper(content).expect("comptime field with annotations should parse");
    match &items[0] {
        crate::ast::Item::StructType(def, _) => {
            assert_eq!(def.fields[0].name, "unit");
            assert!(def.fields[0].is_comptime);
            assert_eq!(def.fields[0].annotations.len(), 1);
            assert_eq!(def.fields[0].annotations[0].name, "label");
        }
        other => panic!("Expected StructType, got {:?}", other),
    }
}

#[test]
fn test_struct_multiple_comptime_fields() {
    let content = r#"
        type Currency {
            comptime symbol: string = "$",
            comptime decimals: number = 2,
            amount: number
        }
    "#;
    let items = parse_program_helper(content).expect("multiple comptime fields should parse");
    match &items[0] {
        crate::ast::Item::StructType(def, _) => {
            assert_eq!(def.fields.len(), 3);
            assert!(def.fields[0].is_comptime);
            assert!(def.fields[1].is_comptime);
            assert!(!def.fields[2].is_comptime);
        }
        other => panic!("Expected StructType, got {:?}", other),
    }
}

#[test]
fn test_struct_comptime_field_no_default() {
    // Comptime fields without defaults are allowed — must be provided via type alias override
    let content = r#"
        type Measurement {
            comptime unit: string,
            value: number
        }
    "#;
    let items = parse_program_helper(content).expect("comptime field without default should parse");
    match &items[0] {
        crate::ast::Item::StructType(def, _) => {
            assert!(def.fields[0].is_comptime);
            assert!(def.fields[0].default_value.is_none());
        }
        other => panic!("Expected StructType, got {:?}", other),
    }
}

#[test]
fn test_struct_comptime_field_numeric_default() {
    let content = r#"
        type Percent {
            comptime decimals: number = 2,
            value: number
        }
    "#;
    let items =
        parse_program_helper(content).expect("comptime field with numeric default should parse");
    match &items[0] {
        crate::ast::Item::StructType(def, _) => {
            assert!(def.fields[0].is_comptime);
            if let Some(crate::ast::Expr::Literal(crate::ast::Literal::Int(n), _)) =
                &def.fields[0].default_value
            {
                assert_eq!(*n, 2);
            } else {
                panic!(
                    "Expected int literal default, got {:?}",
                    def.fields[0].default_value
                );
            }
        }
        other => panic!("Expected StructType, got {:?}", other),
    }
}

#[test]
fn test_struct_runtime_field_no_comptime() {
    // Existing structs without comptime should continue to work
    let content = r#"
        type Point { x: number, y: number }
    "#;
    let items = parse_program_helper(content).expect("regular struct should parse");
    match &items[0] {
        crate::ast::Item::StructType(def, _) => {
            assert!(!def.fields[0].is_comptime);
            assert!(!def.fields[1].is_comptime);
            assert!(def.fields[0].default_value.is_none());
            assert!(def.fields[1].default_value.is_none());
        }
        other => panic!("Expected StructType, got {:?}", other),
    }
}

// =========================================================================
// Trait Bound Parsing Tests
// =========================================================================

#[test]
fn test_trait_bound_single() {
    let content = r#"
        fn foo<T: Comparable>(a: T, b: T) -> bool {
            return true
        }
    "#;
    let items = parse_program_helper(content).expect("single trait bound should parse");
    match &items[0] {
        crate::ast::Item::Function(func, _) => {
            let tp = &func.type_params.as_ref().expect("expected type params")[0];
            assert_eq!(tp.name, "T");
            assert_eq!(tp.trait_bounds, vec!["Comparable".to_string()]);
        }
        other => panic!("Expected Function, got {:?}", other),
    }
}

#[test]
fn test_trait_bound_multiple() {
    let content = r#"
        fn bar<T: Serializable + Display>(x: T) {
            return x
        }
    "#;
    let items = parse_program_helper(content).expect("multiple trait bounds should parse");
    match &items[0] {
        crate::ast::Item::Function(func, _) => {
            let tp = &func.type_params.as_ref().expect("expected type params")[0];
            assert_eq!(tp.name, "T");
            assert_eq!(
                tp.trait_bounds,
                vec!["Serializable".to_string(), "Display".to_string()]
            );
        }
        other => panic!("Expected Function, got {:?}", other),
    }
}

#[test]
fn test_trait_bound_none_backward_compatible() {
    let content = r#"
        fn baz<T>(x: T) {
            return x
        }
    "#;
    let items = parse_program_helper(content).expect("no trait bounds should parse");
    match &items[0] {
        crate::ast::Item::Function(func, _) => {
            let tp = &func.type_params.as_ref().expect("expected type params")[0];
            assert_eq!(tp.name, "T");
            assert!(tp.trait_bounds.is_empty(), "Expected empty trait bounds");
        }
        other => panic!("Expected Function, got {:?}", other),
    }
}

#[test]
fn test_type_param_default_type_parses() {
    let content = r#"
        fn foo<T = int>(x: T) {
            return x
        }
    "#;
    let items = parse_program_helper(content).expect("type param default should parse");
    match &items[0] {
        crate::ast::Item::Function(func, _) => {
            let tp = &func.type_params.as_ref().expect("expected type params")[0];
            assert_eq!(tp.name, "T");
            assert!(tp.trait_bounds.is_empty(), "Expected no trait bounds");
            assert_eq!(
                tp.default_type,
                Some(crate::ast::TypeAnnotation::Basic("int".to_string()))
            );
        }
        other => panic!("Expected Function, got {:?}", other),
    }
}

#[test]
fn test_type_param_bounds_with_default_type_parses() {
    let content = r#"
        fn foo<T: Numeric = int>(x: T) {
            return x
        }
    "#;
    let items = parse_program_helper(content).expect("bounded type param default should parse");
    match &items[0] {
        crate::ast::Item::Function(func, _) => {
            let tp = &func.type_params.as_ref().expect("expected type params")[0];
            assert_eq!(tp.name, "T");
            assert_eq!(tp.trait_bounds, vec!["Numeric".to_string()]);
            assert_eq!(
                tp.default_type,
                Some(crate::ast::TypeAnnotation::Basic("int".to_string()))
            );
        }
        other => panic!("Expected Function, got {:?}", other),
    }
}

// ---------------------------------------------------------------
// Associated types in traits
// ---------------------------------------------------------------

#[test]
fn test_trait_with_associated_type() {
    let content = r#"
        trait Iterator {
            type Item;
            next(self): Item
        }
    "#;
    let items = parse_program_helper(content).expect("should parse trait with associated type");
    match &items[0] {
        crate::ast::Item::Trait(trait_def, _) => {
            assert_eq!(trait_def.name, "Iterator");
            assert_eq!(trait_def.members.len(), 2);
            match &trait_def.members[0] {
                crate::ast::TraitMember::AssociatedType { name, bounds, .. } => {
                    assert_eq!(name, "Item");
                    assert!(bounds.is_empty());
                }
                other => panic!("Expected AssociatedType, got {:?}", other),
            }
        }
        other => panic!("Expected Trait, got {:?}", other),
    }
}

#[test]
fn test_trait_with_bounded_associated_type() {
    let content = r#"
        trait Container {
            type Item: Comparable + Display;
            get(self, idx: int): Item
        }
    "#;
    let items = parse_program_helper(content).expect("should parse bounded associated type");
    match &items[0] {
        crate::ast::Item::Trait(trait_def, _) => {
            assert_eq!(trait_def.name, "Container");
            match &trait_def.members[0] {
                crate::ast::TraitMember::AssociatedType { name, bounds, .. } => {
                    assert_eq!(name, "Item");
                    assert_eq!(bounds.len(), 2);
                }
                other => panic!("Expected AssociatedType, got {:?}", other),
            }
        }
        other => panic!("Expected Trait, got {:?}", other),
    }
}

#[test]
fn test_impl_with_associated_type_binding() {
    let content = r#"
        impl Iterator for Range {
            type Item = number;
            method next() {
                return self.current
            }
        }
    "#;
    let items = parse_program_helper(content).expect("should parse impl with associated type");
    match &items[0] {
        crate::ast::Item::Impl(impl_block, _) => {
            assert_eq!(impl_block.associated_type_bindings.len(), 1);
            assert_eq!(impl_block.associated_type_bindings[0].name, "Item");
            assert!(matches!(
                &impl_block.associated_type_bindings[0].concrete_type,
                crate::ast::TypeAnnotation::Basic(s) if s == "number"
            ));
            assert_eq!(impl_block.methods.len(), 1);
            assert_eq!(impl_block.methods[0].name, "next");
        }
        other => panic!("Expected Impl, got {:?}", other),
    }
}

#[test]
fn test_impl_with_multiple_associated_types() {
    let content = r#"
        impl Collection for HashMap {
            type Key = string;
            type Value = number;
            method len() {
                return 0
            }
        }
    "#;
    let items = parse_program_helper(content).expect("should parse multiple associated types");
    match &items[0] {
        crate::ast::Item::Impl(impl_block, _) => {
            assert_eq!(impl_block.associated_type_bindings.len(), 2);
            assert_eq!(impl_block.associated_type_bindings[0].name, "Key");
            assert_eq!(impl_block.associated_type_bindings[1].name, "Value");
        }
        other => panic!("Expected Impl, got {:?}", other),
    }
}

#[test]
fn test_named_impl_block_parses_impl_name() {
    let content = r#"
        impl Display for User as JsonDisplay {
            method display() {
                return ""
            }
        }
    "#;
    let items = parse_program_helper(content).expect("should parse named impl");
    match &items[0] {
        crate::ast::Item::Impl(impl_block, _) => {
            assert_eq!(impl_block.impl_name.as_deref(), Some("JsonDisplay"));
            assert_eq!(impl_block.methods.len(), 1);
            assert_eq!(impl_block.methods[0].name, "display");
        }
        other => panic!("Expected Impl, got {:?}", other),
    }
}

#[test]
fn test_using_impl_postfix_expr_parses() {
    let content = r#"
        let rendered = user using JsonDisplay;
    "#;
    let items = parse_program_helper(content).expect("should parse using-impl expression");
    match &items[0] {
        crate::ast::Item::Statement(crate::ast::Statement::VariableDecl(decl, _), _) => {
            let value = decl.value.as_ref().expect("expected initializer");
            match value {
                crate::ast::Expr::UsingImpl {
                    expr, impl_name, ..
                } => {
                    assert_eq!(impl_name, "JsonDisplay");
                    assert!(
                        matches!(expr.as_ref(), crate::ast::Expr::Identifier(name, _) if name == "user")
                    );
                }
                other => panic!("Expected UsingImpl expression, got {:?}", other),
            }
        }
        other => panic!("Expected Statement(VariableDecl), got {:?}", other),
    }
}

// ---------------------------------------------------------------
// Where clauses
// ---------------------------------------------------------------

#[test]
fn test_function_with_where_clause() {
    let content = r#"
        fn sort<T>(items: Vec<T>) -> Vec<T>
            where T: Comparable
        {
            return items
        }
    "#;
    let items = parse_program_helper(content).expect("should parse where clause");
    match &items[0] {
        crate::ast::Item::Function(func, _) => {
            assert_eq!(func.name, "sort");
            let wc = func.where_clause.as_ref().expect("expected where clause");
            assert_eq!(wc.len(), 1);
            assert_eq!(wc[0].type_name, "T");
            assert_eq!(wc[0].bounds, vec!["Comparable"]);
        }
        other => panic!("Expected Function, got {:?}", other),
    }
}

#[test]
fn test_function_with_multiple_where_predicates() {
    let content = r#"
        fn process<T, U>(a: T, b: U) -> string
            where T: Serializable + Display, U: Comparable
        {
            return ""
        }
    "#;
    let items = parse_program_helper(content).expect("should parse multiple predicates");
    match &items[0] {
        crate::ast::Item::Function(func, _) => {
            let wc = func.where_clause.as_ref().expect("expected where clause");
            assert_eq!(wc.len(), 2);
            assert_eq!(wc[0].type_name, "T");
            assert_eq!(wc[0].bounds, vec!["Serializable", "Display"]);
            assert_eq!(wc[1].type_name, "U");
            assert_eq!(wc[1].bounds, vec!["Comparable"]);
        }
        other => panic!("Expected Function, got {:?}", other),
    }
}

#[test]
fn test_impl_with_where_clause() {
    let content = r#"
        impl Sortable for Array where T: Comparable {
            method sort() {
                return self
            }
        }
    "#;
    let items = parse_program_helper(content).expect("should parse impl with where clause");
    match &items[0] {
        crate::ast::Item::Impl(impl_block, _) => {
            let wc = impl_block
                .where_clause
                .as_ref()
                .expect("expected where clause on impl");
            assert_eq!(wc.len(), 1);
            assert_eq!(wc[0].type_name, "T");
            assert_eq!(wc[0].bounds, vec!["Comparable"]);
        }
        other => panic!("Expected Impl, got {:?}", other),
    }
}

// =========================================================================
// Dyn Trait Object Type Tests
// =========================================================================

#[test]
fn test_dyn_type_single_trait() {
    let content = r#"
        let x: dyn Display = value;
    "#;
    let items = parse_program_helper(content).expect("dyn type should parse");
    match &items[0] {
        crate::ast::Item::Statement(crate::ast::Statement::VariableDecl(decl, _), _) => {
            let ann = decl
                .type_annotation
                .as_ref()
                .expect("expected type annotation");
            match ann {
                crate::ast::TypeAnnotation::Dyn(traits) => {
                    assert_eq!(traits.len(), 1);
                    assert_eq!(traits[0], "Display");
                }
                other => panic!("Expected Dyn type annotation, got {:?}", other),
            }
        }
        other => panic!("Expected Statement(VariableDecl), got {:?}", other),
    }
}

#[test]
fn test_dyn_type_multiple_traits() {
    let content = r#"
        let x: dyn Display + Serializable = value;
    "#;
    let items = parse_program_helper(content).expect("dyn multi-trait should parse");
    match &items[0] {
        crate::ast::Item::Statement(crate::ast::Statement::VariableDecl(decl, _), _) => {
            let ann = decl
                .type_annotation
                .as_ref()
                .expect("expected type annotation");
            match ann {
                crate::ast::TypeAnnotation::Dyn(traits) => {
                    assert_eq!(traits.len(), 2);
                    assert_eq!(traits[0], "Display");
                    assert_eq!(traits[1], "Serializable");
                }
                other => panic!("Expected Dyn type annotation, got {:?}", other),
            }
        }
        other => panic!("Expected Statement(VariableDecl), got {:?}", other),
    }
}

#[test]
fn test_dyn_type_three_traits() {
    let content = r#"
        let x: dyn Display + Serializable + Comparable = value;
    "#;
    let items = parse_program_helper(content).expect("dyn three-trait should parse");
    match &items[0] {
        crate::ast::Item::Statement(crate::ast::Statement::VariableDecl(decl, _), _) => {
            let ann = decl
                .type_annotation
                .as_ref()
                .expect("expected type annotation");
            match ann {
                crate::ast::TypeAnnotation::Dyn(traits) => {
                    assert_eq!(traits.len(), 3);
                    assert_eq!(traits[0], "Display");
                    assert_eq!(traits[1], "Serializable");
                    assert_eq!(traits[2], "Comparable");
                }
                other => panic!("Expected Dyn type annotation, got {:?}", other),
            }
        }
        other => panic!("Expected Statement(VariableDecl), got {:?}", other),
    }
}

#[test]
fn test_dyn_type_in_function_param() {
    let content = r#"
        function render(obj: dyn Display) -> string {
            return ""
        }
    "#;
    let items = parse_program_helper(content).expect("dyn in function param should parse");
    match &items[0] {
        crate::ast::Item::Function(func, _) => {
            assert_eq!(func.name, "render");
            let param_type = func.params[0]
                .type_annotation
                .as_ref()
                .expect("expected type annotation on param");
            match param_type {
                crate::ast::TypeAnnotation::Dyn(traits) => {
                    assert_eq!(traits.len(), 1);
                    assert_eq!(traits[0], "Display");
                }
                other => panic!("Expected Dyn type, got {:?}", other),
            }
        }
        other => panic!("Expected Function, got {:?}", other),
    }
}

#[test]
fn test_dyn_type_in_return_type() {
    let content = r#"
        function make_display() -> dyn Display {
            return 42
        }
    "#;
    let items = parse_program_helper(content).expect("dyn in return type should parse");
    match &items[0] {
        crate::ast::Item::Function(func, _) => {
            let ret_type = func.return_type.as_ref().expect("expected return type");
            match ret_type {
                crate::ast::TypeAnnotation::Dyn(traits) => {
                    assert_eq!(traits.len(), 1);
                    assert_eq!(traits[0], "Display");
                }
                other => panic!("Expected Dyn return type, got {:?}", other),
            }
        }
        other => panic!("Expected Function, got {:?}", other),
    }
}
