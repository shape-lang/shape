//! Advanced feature parsing tests
//!
//! This module contains tests for:
//! - Pattern matching
//! - Decomposition patterns
//! - Fuzzy comparisons
//! - Annotation definitions
//! - Complex integration tests

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

fn handler_param_names(handler: &crate::ast::AnnotationHandler) -> Vec<&str> {
    handler.params.iter().map(|p| p.name.as_str()).collect()
}

// =========================================================================
// Annotation Lifecycle Handler Tests
// =========================================================================

#[test]
fn test_annotation_def_with_on_define() {
    // Annotation with on_define lifecycle handler
    let content = r#"
        annotation pattern() {
            on_define(fn, ctx) {
                ctx.registry("patterns").set(fn.name, fn)
            }
        }
    "#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "Annotation with on_define should parse: {:?}",
        result.err()
    );

    let items = result.unwrap();
    assert_eq!(items.len(), 1);

    if let crate::ast::Item::AnnotationDef(ann_def, _) = &items[0] {
        assert_eq!(ann_def.name, "pattern");
        assert_eq!(ann_def.handlers.len(), 1);
        assert_eq!(
            ann_def.handlers[0].handler_type,
            crate::ast::AnnotationHandlerType::OnDefine
        );
        assert_eq!(handler_param_names(&ann_def.handlers[0]), vec!["fn", "ctx"]);
    } else {
        panic!("Expected AnnotationDef");
    }
}

#[test]
fn test_legacy_at_annotation_definition_is_rejected() {
    let result = ShapeParser::parse(
        Rule::annotation_def,
        "@annotation old_style() { metadata() { { legacy: true } } }",
    );
    assert!(
        result.is_err(),
        "Legacy @annotation syntax must be rejected"
    );
}

#[test]
fn test_typeof_expression_is_rejected() {
    let content = r#"
        function test() {
            return typeof(1)
        }
    "#;
    let result = parse_program_helper(content);
    assert!(result.is_err(), "typeof must be removed from grammar");
}

#[test]
fn test_annotation_def_with_metadata() {
    // Annotation with metadata handler
    let content = r#"
        annotation indicator() {
            metadata() { { cacheable: true, pure: true } }
        }
    "#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "Annotation with metadata should parse: {:?}",
        result.err()
    );

    let items = result.unwrap();
    if let crate::ast::Item::AnnotationDef(ann_def, _) = &items[0] {
        assert_eq!(ann_def.handlers.len(), 1);
        assert_eq!(
            ann_def.handlers[0].handler_type,
            crate::ast::AnnotationHandlerType::Metadata
        );
        assert!(ann_def.handlers[0].params.is_empty());
    } else {
        panic!("Expected AnnotationDef");
    }
}

#[test]
fn test_annotation_def_with_before_after() {
    // Annotation with before and after handlers for caching
    let content = r#"
        annotation cached() {
            before(fn, args, ctx) {
                let key = hash(fn.name, args);
                ctx.cache.get(key)
            }
            after(fn, args, result, ctx) {
                let key = hash(fn.name, args);
                ctx.cache.set(key, result);
                result
            }
        }
    "#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "Annotation with before/after should parse: {:?}",
        result.err()
    );

    let items = result.unwrap();
    if let crate::ast::Item::AnnotationDef(ann_def, _) = &items[0] {
        assert_eq!(ann_def.name, "cached");
        assert_eq!(ann_def.handlers.len(), 2);
        assert_eq!(
            ann_def.handlers[0].handler_type,
            crate::ast::AnnotationHandlerType::Before
        );
        assert_eq!(
            ann_def.handlers[1].handler_type,
            crate::ast::AnnotationHandlerType::After
        );
    } else {
        panic!("Expected AnnotationDef");
    }
}

#[test]
fn test_annotation_def_with_params() {
    // Annotation with parameters (like @warmup(period))
    let content = r#"
        annotation warmup(period) {
            before(fn, args, ctx) {
                ctx.data.extend_back(period)
            }
            after(fn, args, result, ctx) {
                ctx.data.restore_range();
                result
            }
            metadata() { { warmup_period: period } }
        }
    "#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "Annotation with params should parse: {:?}",
        result.err()
    );

    let items = result.unwrap();
    if let crate::ast::Item::AnnotationDef(ann_def, _) = &items[0] {
        assert_eq!(ann_def.name, "warmup");
        assert_eq!(ann_def.params.len(), 1);
        assert_eq!(ann_def.params[0].simple_name(), Some("period"));
        assert_eq!(ann_def.handlers.len(), 3);
    } else {
        panic!("Expected AnnotationDef");
    }
}

#[test]
fn test_annotation_def_with_return_in_metadata() {
    let content = r#"
        annotation cached(ttl) {
            before(fn, args, ctx) {
                let key = hash(fn.name, args);
                ctx.cache.get(key)
            }

            after(fn, args, result, ctx) {
                let key = hash(fn.name, args);
                ctx.cache.set(key, result);
                result
            }

            metadata() {
                return {
                    cacheable: true,
                    ttl: ttl
                }
            }
        }
    "#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "Annotation with return in metadata should parse: {:?}",
        result.err()
    );
}

// =========================================================================
// Export Functions with Annotations
// =========================================================================

#[test]
fn test_parse_export_function_with_annotation() {
    // Export with @warmup annotation
    let content = "pub @warmup(period) fn foo(series, period) { return series; }";
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "Export with annotation should parse: {:?}",
        result.err()
    );
}

// =========================================================================
// Block Expression Tests
// =========================================================================

#[test]
fn test_block_expr_with_return() {
    let content = r#"
        let x = {
            let y = 10;
            return y * 2
        };
    "#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "Block with return should parse: {:?}",
        result.err()
    );
}

// =========================================================================
// Decomposition Pattern Tests
// =========================================================================

#[test]
fn test_decomposition_pattern_simple() {
    // Decomposition pattern extracts component types from an intersection
    let content = r#"
        let (a: TypeA, b: TypeB) = merged_value;
    "#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "Decomposition pattern should parse: {:?}",
        result.err()
    );

    let items = result.unwrap();
    assert!(!items.is_empty(), "Expected at least one item");
    if let crate::ast::Item::Statement(crate::ast::Statement::VariableDecl(decl, _), _) = &items[0]
    {
        match &decl.pattern {
            crate::ast::DestructurePattern::Decomposition(bindings) => {
                assert_eq!(bindings.len(), 2);
                assert_eq!(bindings[0].name, "a");
                assert_eq!(bindings[1].name, "b");
            }
            other => panic!("Expected Decomposition pattern, got {:?}", other),
        }
    } else {
        panic!("Expected VariableDecl, got {:?}", items[0]);
    }
}

#[test]
fn test_decomposition_pattern_three_bindings() {
    let content = r#"
        let (x: TypeX, y: TypeY, z: TypeZ) = abc;
    "#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "Decomposition with 3 bindings should parse: {:?}",
        result.err()
    );

    let items = result.unwrap();
    if let crate::ast::Item::Statement(crate::ast::Statement::VariableDecl(decl, _), _) = &items[0]
    {
        match &decl.pattern {
            crate::ast::DestructurePattern::Decomposition(bindings) => {
                assert_eq!(bindings.len(), 3);
            }
            other => panic!("Expected Decomposition pattern, got {:?}", other),
        }
    } else {
        panic!("Expected VariableDecl, got {:?}", items[0]);
    }
}

#[test]
fn test_decomposition_pattern_with_generic_types() {
    let content = r#"
        let (reader: Reader<string>, writer: Writer<number>) = io;
    "#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "Decomposition with generic types should parse: {:?}",
        result.err()
    );
}

#[test]
fn test_decomposition_pattern_shorthand_field_set() {
    // Shorthand syntax: field names only, no types
    let content = r#"let (d: {x}, e: {y, z}) = c;"#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "Decomposition with shorthand field set should parse: {:?}",
        result.err()
    );

    let items = result.unwrap();
    if let crate::ast::Item::Statement(crate::ast::Statement::VariableDecl(decl, _), _) = &items[0]
    {
        match &decl.pattern {
            crate::ast::DestructurePattern::Decomposition(bindings) => {
                assert_eq!(bindings.len(), 2);
                assert_eq!(bindings[0].name, "d");
                assert_eq!(bindings[1].name, "e");
                // First binding should have Object type with field "x"
                match &bindings[0].type_annotation {
                    crate::ast::TypeAnnotation::Object(fields) => {
                        assert_eq!(fields.len(), 1);
                        assert_eq!(fields[0].name, "x");
                    }
                    other => panic!("Expected Object type annotation, got {:?}", other),
                }
                // Second binding should have Object type with fields "y", "z"
                match &bindings[1].type_annotation {
                    crate::ast::TypeAnnotation::Object(fields) => {
                        assert_eq!(fields.len(), 2);
                        assert_eq!(fields[0].name, "y");
                        assert_eq!(fields[1].name, "z");
                    }
                    other => panic!("Expected Object type annotation, got {:?}", other),
                }
            }
            other => panic!("Expected Decomposition pattern, got {:?}", other),
        }
    } else {
        panic!("Expected VariableDecl, got {:?}", items[0]);
    }
}

#[test]
fn test_decomposition_pattern_full_object_types() {
    // Full object type syntax with field types
    let content = r#"let (f: {x: int}, g: {y: int, z: int}) = c;"#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "Decomposition with full object types should parse: {:?}",
        result.err()
    );

    let items = result.unwrap();
    if let crate::ast::Item::Statement(crate::ast::Statement::VariableDecl(decl, _), _) = &items[0]
    {
        match &decl.pattern {
            crate::ast::DestructurePattern::Decomposition(bindings) => {
                assert_eq!(bindings.len(), 2);
                assert_eq!(bindings[0].name, "f");
                assert_eq!(bindings[1].name, "g");
                match &bindings[0].type_annotation {
                    crate::ast::TypeAnnotation::Object(fields) => {
                        assert_eq!(fields.len(), 1);
                        assert_eq!(fields[0].name, "x");
                    }
                    other => panic!("Expected Object type annotation, got {:?}", other),
                }
                match &bindings[1].type_annotation {
                    crate::ast::TypeAnnotation::Object(fields) => {
                        assert_eq!(fields.len(), 2);
                        assert_eq!(fields[0].name, "y");
                        assert_eq!(fields[1].name, "z");
                    }
                    other => panic!("Expected Object type annotation, got {:?}", other),
                }
            }
            other => panic!("Expected Decomposition pattern, got {:?}", other),
        }
    } else {
        panic!("Expected VariableDecl, got {:?}", items[0]);
    }
}

// =========================================================================
// Fuzzy Comparison Tests
// =========================================================================

#[test]
fn test_fuzzy_equal_basic() {
    let result = parse_program_helper("let x = 1 ~= 2;");
    assert!(
        result.is_ok(),
        "Basic fuzzy equal should parse: {:?}",
        result.err()
    );

    let items = result.unwrap();
    if let crate::ast::Item::Statement(crate::ast::Statement::VariableDecl(decl, _), _) = &items[0]
    {
        if let Some(crate::ast::Expr::FuzzyComparison { op, tolerance, .. }) = &decl.value {
            assert_eq!(*op, crate::ast::operators::FuzzyOp::Equal);
            // Default tolerance is 2%
            assert!(
                matches!(tolerance, crate::ast::operators::FuzzyTolerance::Percentage(p) if (*p - 0.02).abs() < 0.001)
            );
        } else {
            panic!("Expected FuzzyComparison, got {:?}", decl.value);
        }
    } else {
        panic!("Expected VariableDecl");
    }
}

#[test]
fn test_fuzzy_greater_basic() {
    let result = parse_program_helper("let x = a ~> b;");
    assert!(
        result.is_ok(),
        "Basic fuzzy greater should parse: {:?}",
        result.err()
    );

    let items = result.unwrap();
    if let crate::ast::Item::Statement(crate::ast::Statement::VariableDecl(decl, _), _) = &items[0]
    {
        if let Some(crate::ast::Expr::FuzzyComparison { op, .. }) = &decl.value {
            assert_eq!(*op, crate::ast::operators::FuzzyOp::Greater);
        } else {
            panic!("Expected FuzzyComparison, got {:?}", decl.value);
        }
    }
}

#[test]
fn test_fuzzy_less_basic() {
    let result = parse_program_helper("let x = a ~< b;");
    assert!(
        result.is_ok(),
        "Basic fuzzy less should parse: {:?}",
        result.err()
    );

    let items = result.unwrap();
    if let crate::ast::Item::Statement(crate::ast::Statement::VariableDecl(decl, _), _) = &items[0]
    {
        if let Some(crate::ast::Expr::FuzzyComparison { op, .. }) = &decl.value {
            assert_eq!(*op, crate::ast::operators::FuzzyOp::Less);
        } else {
            panic!("Expected FuzzyComparison, got {:?}", decl.value);
        }
    }
}

#[test]
fn test_fuzzy_with_absolute_tolerance() {
    let result = parse_program_helper("let x = a ~= b within 0.05;");
    assert!(
        result.is_ok(),
        "Fuzzy with absolute tolerance should parse: {:?}",
        result.err()
    );

    let items = result.unwrap();
    if let crate::ast::Item::Statement(crate::ast::Statement::VariableDecl(decl, _), _) = &items[0]
    {
        if let Some(crate::ast::Expr::FuzzyComparison { tolerance, .. }) = &decl.value {
            assert!(
                matches!(tolerance, crate::ast::operators::FuzzyTolerance::Absolute(v) if (*v - 0.05).abs() < 0.001)
            );
        } else {
            panic!("Expected FuzzyComparison, got {:?}", decl.value);
        }
    }
}

#[test]
fn test_fuzzy_with_percentage_tolerance() {
    let result = parse_program_helper("let x = a ~= b within 5%;");
    assert!(
        result.is_ok(),
        "Fuzzy with percentage tolerance should parse: {:?}",
        result.err()
    );

    let items = result.unwrap();
    if let crate::ast::Item::Statement(crate::ast::Statement::VariableDecl(decl, _), _) = &items[0]
    {
        if let Some(crate::ast::Expr::FuzzyComparison { tolerance, .. }) = &decl.value {
            // 5% should be stored as 0.05
            assert!(
                matches!(tolerance, crate::ast::operators::FuzzyTolerance::Percentage(v) if (*v - 0.05).abs() < 0.001)
            );
        } else {
            panic!("Expected FuzzyComparison, got {:?}", decl.value);
        }
    }
}

#[test]
fn test_fuzzy_with_integer_tolerance() {
    let result = parse_program_helper("let x = a ~= b within 10;");
    assert!(
        result.is_ok(),
        "Fuzzy with integer tolerance should parse: {:?}",
        result.err()
    );

    let items = result.unwrap();
    if let crate::ast::Item::Statement(crate::ast::Statement::VariableDecl(decl, _), _) = &items[0]
    {
        if let Some(crate::ast::Expr::FuzzyComparison { tolerance, .. }) = &decl.value {
            assert!(
                matches!(tolerance, crate::ast::operators::FuzzyTolerance::Absolute(v) if (*v - 10.0).abs() < 0.001)
            );
        } else {
            panic!("Expected FuzzyComparison, got {:?}", decl.value);
        }
    }
}

#[test]
fn test_fuzzy_in_function() {
    let result = parse_program_helper(
        r#"
        function is_close(a, b) {
            return a ~= b within 0.01;
        }
    "#,
    );
    assert!(
        result.is_ok(),
        "Fuzzy in function should parse: {:?}",
        result.err()
    );
}

#[test]
fn test_fuzzy_chained_with_and() {
    let result = parse_program_helper("let x = a ~= b within 0.1 and c ~> d;");
    assert!(
        result.is_ok(),
        "Fuzzy chained with and should parse: {:?}",
        result.err()
    );
}

#[test]
fn test_enum_with_typed_function_param() {
    let result = parse_program_helper(
        r#"
        enum Status { Active, Inactive, Pending }

        function check(s: Status) {
            return match s {
                Status::Active => "yes"
            };
        }
    "#,
    );
    assert!(
        result.is_ok(),
        "Enum with typed function param should parse: {:?}",
        result.err()
    );
}

// =========================================================================
// Complex Integration Tests
// =========================================================================

#[test]
fn test_parse_trend_adx_pattern() {
    // Simplified version of adx from trend.shape
    let content = r#"
pub fn adx(high, low, close, period = 14) {
    let adx_val = 42;
    let plus_di = 50;
    let minus_di = 30;

    {
        adx: adx_val,
        plus_di: plus_di,
        minus_di: minus_di
    }
}
"#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "ADX pattern should parse: {:?}",
        result.err()
    );
}

#[test]
fn test_parse_trend_file_minimal() {
    // Minimal reproduction of trend.shape structure
    let content = r#"
from std::finance::indicators::moving_averages use { ema }
from std::finance::indicators::volatility use { atr }
from std::core::utils::rolling use { linear_recurrence, rolling_mean }
from std::core::utils::vector use { select }

// Wilder's Smoothing (Running Moving Average)
function rma(series, period) {
    let alpha = 1.0 / period;
    42
}

pub @warmup(period * 3) fn adx(high, low, close, period = 14) {
    let adx_val = 42;
    {
        adx: adx_val
    }
}
"#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "Trend file minimal should parse: {:?}",
        result.err()
    );
}

#[test]
fn test_parse_trend_file_full() {
    // Read the actual trend.shape file
    let content = include_str!("../../../../shape-core/stdlib/finance/indicators/trend.shape");
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "Full trend.shape should parse: {:?}",
        result.err()
    );
}

// =========================================================================
// Async/Await Tests (Phase 2)
// =========================================================================

#[test]
fn test_async_function_def() {
    let content = r#"async function foo() { return 1 }"#;
    let items = parse_program_helper(content).expect("async function should parse");
    assert_eq!(items.len(), 1);
    match &items[0] {
        crate::ast::Item::Function(func_def, _) => {
            assert_eq!(func_def.name, "foo");
            assert!(func_def.is_async, "function should be async");
            assert!(!func_def.is_comptime, "function should NOT be comptime");
        }
        other => panic!("expected Function, got {:?}", other),
    }
}

#[test]
fn test_async_fn_def() {
    let content = r#"async fn foo() { return 1 }"#;
    let items = parse_program_helper(content).expect("async fn should parse");
    assert_eq!(items.len(), 1);
    match &items[0] {
        crate::ast::Item::Function(func_def, _) => {
            assert_eq!(func_def.name, "foo");
            assert!(func_def.is_async, "function should be async");
        }
        other => panic!("expected Function, got {:?}", other),
    }
}

#[test]
fn test_sync_function_def() {
    let content = r#"function bar() { return 2 }"#;
    let items = parse_program_helper(content).expect("sync function should parse");
    assert_eq!(items.len(), 1);
    match &items[0] {
        crate::ast::Item::Function(func_def, _) => {
            assert_eq!(func_def.name, "bar");
            assert!(!func_def.is_async, "function should NOT be async");
        }
        other => panic!("expected Function, got {:?}", other),
    }
}

#[test]
fn test_sync_fn_def() {
    let content = r#"fn bar() { return 2 }"#;
    let items = parse_program_helper(content).expect("sync fn should parse");
    assert_eq!(items.len(), 1);
    match &items[0] {
        crate::ast::Item::Function(func_def, _) => {
            assert_eq!(func_def.name, "bar");
            assert!(!func_def.is_async, "function should NOT be async");
            assert!(!func_def.is_comptime, "function should NOT be comptime");
        }
        other => panic!("expected Function, got {:?}", other),
    }
}

#[test]
fn test_comptime_fn_def() {
    let content = r#"comptime fn helper() { return 2 }"#;
    let items = parse_program_helper(content).expect("comptime fn should parse");
    assert_eq!(items.len(), 1);
    match &items[0] {
        crate::ast::Item::Function(func_def, _) => {
            assert_eq!(func_def.name, "helper");
            assert!(!func_def.is_async, "function should NOT be async");
            assert!(func_def.is_comptime, "function should be comptime");
        }
        other => panic!("expected Function, got {:?}", other),
    }
}

#[test]
fn test_await_expr_parses() {
    let content = r#"function foo() { let x = await bar(); return x }"#;
    let items = parse_program_helper(content).expect("await expr should parse");
    assert_eq!(items.len(), 1);
    match &items[0] {
        crate::ast::Item::Function(func_def, _) => {
            assert_eq!(func_def.name, "foo");
            // The body should contain a let statement with an await expression
            assert!(!func_def.body.is_empty());
        }
        other => panic!("expected Function, got {:?}", other),
    }
}

#[test]
fn test_async_function_with_await() {
    let content = r#"async function fetch_data() { let result = await get_data(); return result }"#;
    let items = parse_program_helper(content).expect("async function with await should parse");
    assert_eq!(items.len(), 1);
    match &items[0] {
        crate::ast::Item::Function(func_def, _) => {
            assert_eq!(func_def.name, "fetch_data");
            assert!(func_def.is_async);
        }
        other => panic!("expected Function, got {:?}", other),
    }
}

// =========================================================================
// Block Comment Tests (Phase 3.1)
// =========================================================================

#[test]
fn test_block_comment_simple() {
    let content = r#"/* simple block comment */ let x = 1"#;
    let items = parse_program_helper(content).expect("block comment should be ignored");
    assert_eq!(items.len(), 1);
}

#[test]
fn test_block_comment_nested() {
    let content = r#"/* outer /* inner */ still outer */ let x = 1"#;
    let items = parse_program_helper(content).expect("nested block comment should work");
    assert_eq!(items.len(), 1);
}

#[test]
fn test_block_comment_multiline() {
    let content = r#"
/*
  This is a multiline
  block comment
*/
let x = 1
"#;
    let items = parse_program_helper(content).expect("multiline block comment should work");
    assert_eq!(items.len(), 1);
}

#[test]
fn test_block_comment_between_items() {
    let content = r#"
let x = 1
/* between items */
let y = 2
"#;
    let items = parse_program_helper(content).expect("block comment between items should work");
    assert_eq!(items.len(), 2);
}

#[test]
fn test_block_comment_inline() {
    let content = r#"let x = /* inline */ 42"#;
    let items = parse_program_helper(content).expect("inline block comment should work");
    assert_eq!(items.len(), 1);
}

#[test]
fn test_doc_comment_line() {
    let content = r#"
/// This is a doc comment
function foo() { return 1 }
"#;
    let items = parse_program_helper(content).expect("doc comment should be ignored by parser");
    assert_eq!(items.len(), 1);
}

#[test]
fn test_doc_comment_block() {
    let content = r#"
/** This is a block doc comment */
function foo() { return 1 }
"#;
    let items =
        parse_program_helper(content).expect("block doc comment should be ignored by parser");
    assert_eq!(items.len(), 1);
}

#[test]
fn test_mixed_comments() {
    let content = r#"
// line comment
/* block comment */
/// doc comment
/** block doc comment */
let x = 1
"#;
    let items = parse_program_helper(content).expect("mixed comments should all work");
    assert_eq!(items.len(), 1);
}

// ===== Data Source and Query Declaration Tests =====

#[test]
fn test_datasource_declaration() {
    let content = r#"datasource MarketData: DataSource<CandleRow> = provider("market_data")"#;
    let items = parse_program_helper(content).expect("datasource decl should parse");
    assert_eq!(items.len(), 1);
    match &items[0] {
        Item::DataSource(ds, _) => {
            assert_eq!(ds.name, "MarketData");
        }
        other => panic!("expected DataSource, got {:?}", other),
    }
}

#[test]
fn test_query_declaration_with_sql() {
    let content = r#"query UserById: Query<UserRow, Params> = sql(DB, "SELECT id, name FROM users WHERE id = $1")"#;
    let items = parse_program_helper(content).expect("query decl should parse");
    assert_eq!(items.len(), 1);
    match &items[0] {
        Item::QueryDecl(q, _) => {
            assert_eq!(q.name, "UserById");
            assert_eq!(q.source_name, "DB");
            assert!(q.sql.contains("SELECT"));
        }
        other => panic!("expected QueryDecl, got {:?}", other),
    }
}

#[test]
fn test_datasource_with_semicolon() {
    let content = r#"datasource DB: DataSource<UserRow> = provider("postgres");"#;
    let items = parse_program_helper(content).expect("datasource with semicolon should parse");
    assert_eq!(items.len(), 1);
    match &items[0] {
        Item::DataSource(ds, _) => {
            assert_eq!(ds.name, "DB");
        }
        other => panic!("expected DataSource, got {:?}", other),
    }
}

// =========================================================================
// Extend Block Parser Tests
// =========================================================================

#[test]
fn test_extend_basic() {
    let content = r#"
        extend Number {
            method double() {
                return self * 2
            }
        }
    "#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "Basic extend block should parse: {:?}",
        result.err()
    );
    let items = result.unwrap();
    assert_eq!(items.len(), 1);
    match &items[0] {
        Item::Extend(ext, _) => {
            assert_eq!(ext.methods.len(), 1);
            assert_eq!(ext.methods[0].name, "double");
        }
        other => panic!("expected Extend, got {:?}", other),
    }
}

#[test]
fn test_extend_with_params() {
    let content = r#"
        extend Number {
            method add(n: number) {
                return self + n
            }
        }
    "#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "Extend with params should parse: {:?}",
        result.err()
    );
    let items = result.unwrap();
    match &items[0] {
        Item::Extend(ext, _) => {
            assert_eq!(ext.methods[0].params.len(), 1);
            assert_eq!(ext.methods[0].params[0].simple_name(), Some("n"));
        }
        other => panic!("expected Extend, got {:?}", other),
    }
}

#[test]
fn test_extend_multiple_methods() {
    let content = r#"
        extend Number {
            method double() {
                return self * 2
            }
            method triple() {
                return self * 3
            }
        }
    "#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "Extend with multiple methods should parse: {:?}",
        result.err()
    );
    let items = result.unwrap();
    match &items[0] {
        Item::Extend(ext, _) => {
            assert_eq!(ext.methods.len(), 2);
            assert_eq!(ext.methods[0].name, "double");
            assert_eq!(ext.methods[1].name, "triple");
        }
        other => panic!("expected Extend, got {:?}", other),
    }
}

#[test]
fn test_extend_generic_type() {
    let content = r#"
        extend Vec<number> {
            method sum() {
                return self.reduce(|a, b| a + b, 0)
            }
        }
    "#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "Extend on generic type should parse: {:?}",
        result.err()
    );
}

// =========================================================================
// Trait Definition Parser Tests
// =========================================================================

#[test]
fn test_trait_basic() {
    let content = r#"
        trait Queryable {
            filter(predicate: (T) => bool): Self,
            execute(): Result<Table>
        }
    "#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "Basic trait should parse: {:?}",
        result.err()
    );
    let items = result.unwrap();
    assert_eq!(items.len(), 1);
    match &items[0] {
        Item::Trait(def, _) => {
            assert_eq!(def.name, "Queryable");
            assert!(def.type_params.is_none());
            assert_eq!(def.members.len(), 2);
        }
        other => panic!("expected Trait, got {:?}", other),
    }
}

#[test]
fn test_trait_with_type_params() {
    let content = r#"
        trait Queryable<T> {
            filter(predicate: (T) => bool): Self,
            execute(): Result<Table<T>>
        }
    "#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "Trait with type params should parse: {:?}",
        result.err()
    );
    let items = result.unwrap();
    match &items[0] {
        Item::Trait(def, _) => {
            assert_eq!(def.name, "Queryable");
            assert_eq!(def.type_params.as_ref().unwrap().len(), 1);
        }
        other => panic!("expected Trait, got {:?}", other),
    }
}

#[test]
fn test_trait_with_extends_is_rejected() {
    let content = r#"
        trait AdvancedQueryable<T> extends Queryable<T> {
            groupBy(column: string): Self
        }
    "#;
    let result = parse_program_helper(content);
    match result {
        Err(_) => {}
        Ok(items) => assert!(
            items.is_empty(),
            "trait extends should not produce AST items, got: {:?}",
            items
        ),
    }
}

// =========================================================================
// Impl Block Parser Tests
// =========================================================================

#[test]
fn test_impl_basic() {
    let content = r#"
        impl Queryable for Table {
            method filter(predicate) {
                return self
            }
            method execute() {
                return self
            }
        }
    "#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "Basic impl block should parse: {:?}",
        result.err()
    );
    let items = result.unwrap();
    assert_eq!(items.len(), 1);
    match &items[0] {
        Item::Impl(impl_block, _) => {
            assert_eq!(
                impl_block.trait_name,
                crate::ast::TypeName::Simple("Queryable".to_string())
            );
            assert_eq!(
                impl_block.target_type,
                crate::ast::TypeName::Simple("Table".to_string())
            );
            assert_eq!(impl_block.methods.len(), 2);
            assert_eq!(impl_block.methods[0].name, "filter");
            assert_eq!(impl_block.methods[1].name, "execute");
        }
        other => panic!("expected Impl, got {:?}", other),
    }
}

#[test]
fn test_impl_generic_types() {
    let content = r#"
        impl Queryable<T> for Table<T> {
            method filter(predicate) {
                return self
            }
        }
    "#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "Impl with generic types should parse: {:?}",
        result.err()
    );
    let items = result.unwrap();
    match &items[0] {
        Item::Impl(impl_block, _) => {
            match &impl_block.trait_name {
                crate::ast::TypeName::Generic { name, type_args } => {
                    assert_eq!(name, "Queryable");
                    assert_eq!(type_args.len(), 1);
                }
                other => panic!("expected Generic trait name, got {:?}", other),
            }
            match &impl_block.target_type {
                crate::ast::TypeName::Generic { name, type_args } => {
                    assert_eq!(name, "Table");
                    assert_eq!(type_args.len(), 1);
                }
                other => panic!("expected Generic target type, got {:?}", other),
            }
        }
        other => panic!("expected Impl, got {:?}", other),
    }
}

#[test]
fn test_impl_with_method_params() {
    let content = r#"
        impl Sortable for Vec {
            method sort(comparator: (a, b) => number) {
                return self
            }
        }
    "#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "Impl with method params should parse: {:?}",
        result.err()
    );
    let items = result.unwrap();
    match &items[0] {
        Item::Impl(impl_block, _) => {
            assert_eq!(impl_block.methods.len(), 1);
            assert_eq!(impl_block.methods[0].name, "sort");
            assert_eq!(impl_block.methods[0].params.len(), 1);
        }
        other => panic!("expected Impl, got {:?}", other),
    }
}

// =========================================================================
// Sprint 7: Structured Concurrency Parser Tests
// =========================================================================

#[test]
fn test_async_let_parses() {
    let content = r#"
        async function test() {
            async let x = 1 + 2
            await x
        }
    "#;
    let items = parse_program_helper(content).expect("async let should parse");
    assert_eq!(items.len(), 1);
    match &items[0] {
        Item::Function(func_def, _) => {
            assert!(func_def.is_async, "function should be async");
            assert_eq!(func_def.name, "test");
        }
        other => panic!("expected Function, got {:?}", other),
    }
}

#[test]
fn test_async_scope_parses() {
    let content = r#"
        async function test() {
            async scope {
                let x = 42
                x
            }
        }
    "#;
    let items = parse_program_helper(content).expect("async scope should parse");
    assert_eq!(items.len(), 1);
    match &items[0] {
        Item::Function(func_def, _) => {
            assert!(func_def.is_async);
        }
        other => panic!("expected Function, got {:?}", other),
    }
}

#[test]
fn test_for_await_parses() {
    let content = r#"
        async function consume() {
            let items = [1, 2, 3]
            for await item in items {
                print(item)
            }
        }
    "#;
    let items = parse_program_helper(content).expect("for await should parse");
    assert_eq!(items.len(), 1);
    match &items[0] {
        Item::Function(func_def, _) => {
            assert!(func_def.is_async);
        }
        other => panic!("expected Function, got {:?}", other),
    }
}

#[test]
fn test_for_await_expr_parses() {
    let content = r#"
        async function test() {
            let result = for await x in [1, 2, 3] { x * 2 }
            result
        }
    "#;
    let items = parse_program_helper(content).expect("for await expr should parse");
    assert_eq!(items.len(), 1);
}

#[test]
fn test_nested_async_scope_parses() {
    let content = r#"
        async function test() {
            async scope {
                async scope {
                    42
                }
            }
        }
    "#;
    let items = parse_program_helper(content).expect("nested async scope should parse");
    assert_eq!(items.len(), 1);
}

#[test]
fn test_legacy_annotation_comptime_handler_is_rejected() {
    let content = r#"
        annotation derive_debug() {
            comptime(target) {
                let name = target.name
            }
        }
    "#;
    let result = parse_program_helper(content);
    assert!(
        result.is_err(),
        "Legacy `comptime(target)` handler syntax must be rejected"
    );
}

#[test]
fn test_legacy_comptime_before_after_phases_are_rejected() {
    let old_before = r#"
        annotation schema() {
            comptime before(target, ctx) {
                target.name
            }
        }
    "#;
    assert!(
        parse_program_helper(old_before).is_err(),
        "Legacy comptime before(...) phase syntax must be rejected"
    );

    let old_after = r#"
        annotation schema() {
            comptime after(target, ctx) {
                target.name
            }
        }
    "#;
    assert!(
        parse_program_helper(old_after).is_err(),
        "Legacy comptime after(...) phase syntax must be rejected"
    );
}

#[test]
fn test_annotation_keyword_and_variadic_handler_params() {
    let content = r#"
        annotation schema() {
            comptime post(target, ctx, ...config) {
                target.name
            }
        }
    "#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "annotation keyword + variadic params should parse: {:?}",
        result.err()
    );

    let items = result.unwrap();
    if let crate::ast::Item::AnnotationDef(ann_def, _) = &items[0] {
        assert_eq!(ann_def.name, "schema");
        assert_eq!(ann_def.handlers.len(), 1);
        let handler = &ann_def.handlers[0];
        assert_eq!(
            handler.handler_type,
            crate::ast::AnnotationHandlerType::ComptimePost
        );
        assert_eq!(
            handler_param_names(handler),
            vec!["target", "ctx", "config"]
        );
        assert!(!handler.params[0].is_variadic);
        assert!(!handler.params[1].is_variadic);
        assert!(handler.params[2].is_variadic);
    } else {
        panic!("Expected AnnotationDef");
    }
}

#[test]
fn test_annotation_def_with_comptime_pre_post_handlers() {
    let content = r#"
        annotation schema() {
            comptime pre(target, ctx) {
                target.name
            }
            comptime post(target, ctx) {
                target.return_type
            }
        }
    "#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "Annotation with comptime pre/post handlers should parse: {:?}",
        result.err()
    );

    let items = result.unwrap();
    if let crate::ast::Item::AnnotationDef(ann_def, _) = &items[0] {
        assert_eq!(ann_def.name, "schema");
        assert_eq!(ann_def.handlers.len(), 2);
        assert_eq!(
            ann_def.handlers[0].handler_type,
            crate::ast::AnnotationHandlerType::ComptimePre
        );
        assert_eq!(
            ann_def.handlers[1].handler_type,
            crate::ast::AnnotationHandlerType::ComptimePost
        );
    } else {
        panic!("Expected AnnotationDef");
    }
}

#[test]
fn test_function_param_const_flag_parses() {
    let content = r#"
        fn connect(const conn_str: string) {
            conn_str
        }
    "#;

    let items = parse_program_helper(content).expect("function with const param should parse");
    let func = match &items[0] {
        crate::ast::Item::Function(func, _) => func,
        other => panic!("expected function item, got {:?}", other),
    };
    assert_eq!(func.params.len(), 1);
    assert!(
        func.params[0].is_const,
        "parameter should be parsed as const"
    );
}

#[test]
fn test_annotation_def_with_explicit_targets_and_handler() {
    let content = r#"
        annotation only_types() {
            targets: [type, expression]
            comptime post(target, ctx) {
                target.kind
            }
        }
    "#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "Annotation with explicit targets should parse: {:?}",
        result.err()
    );

    let items = result.unwrap();
    if let crate::ast::Item::AnnotationDef(ann_def, _) = &items[0] {
        assert_eq!(ann_def.name, "only_types");
        let targets = ann_def
            .allowed_targets
            .clone()
            .expect("targets should parse");
        assert_eq!(
            targets,
            vec![
                crate::ast::AnnotationTargetKind::Type,
                crate::ast::AnnotationTargetKind::Expression
            ]
        );
        assert_eq!(ann_def.handlers.len(), 1);
        assert_eq!(
            ann_def.handlers[0].handler_type,
            crate::ast::AnnotationHandlerType::ComptimePost
        );
    } else {
        panic!("Expected AnnotationDef");
    }
}

#[test]
fn test_annotation_comptime_directives_parse_in_block() {
    let content = r#"
        annotation transform() {
            targets: [expression]
            comptime post(target, ctx) {
                remove target
            }
        }
    "#;
    let result = parse_program_helper(content).expect("parse should succeed");
    let ann = match &result[0] {
        crate::ast::Item::AnnotationDef(ann_def, _) => ann_def,
        other => panic!("expected AnnotationDef, got {:?}", other),
    };
    let handler = &ann.handlers[0];
    let body_items = match &handler.body {
        crate::ast::Expr::Block(block, _) => &block.items,
        other => panic!("expected block body, got {:?}", other),
    };
    assert!(
        body_items.iter().any(|item| matches!(
            item,
            crate::ast::BlockItem::Statement(crate::ast::Statement::RemoveTarget(_))
        )),
        "expected remove target statement in comptime handler body"
    );
}

#[test]
fn test_annotation_typed_comptime_directives_parse() {
    let content = r#"
        annotation schema() {
            targets: [function]
            comptime post(target, ctx) {
                set param uri: string
                set return DbConnection
                replace body {
                    return runtime_connect(uri)
                }
            }
        }
    "#;

    let result = parse_program_helper(content).expect("parse should succeed");
    let ann = match &result[0] {
        crate::ast::Item::AnnotationDef(ann_def, _) => ann_def,
        other => panic!("expected AnnotationDef, got {:?}", other),
    };
    let handler = &ann.handlers[0];
    let body_items = match &handler.body {
        crate::ast::Expr::Block(block, _) => &block.items,
        other => panic!("expected block body, got {:?}", other),
    };
    assert!(body_items.iter().any(|item| matches!(
        item,
        crate::ast::BlockItem::Statement(crate::ast::Statement::SetParamType { .. })
    )));
    assert!(body_items.iter().any(|item| matches!(
        item,
        crate::ast::BlockItem::Statement(crate::ast::Statement::SetReturnType { .. })
    )));
    assert!(body_items.iter().any(|item| matches!(
        item,
        crate::ast::BlockItem::Statement(crate::ast::Statement::ReplaceBody { .. })
    )));
}

#[test]
fn test_annotation_replace_body_expr_directive_parse() {
    let content = r#"
        annotation schema() {
            targets: [function]
            comptime post(target, ctx) {
                replace body (gen_body(target))
            }
        }
    "#;

    let result = parse_program_helper(content).expect("parse should succeed");
    let ann = match &result[0] {
        crate::ast::Item::AnnotationDef(ann_def, _) => ann_def,
        other => panic!("expected AnnotationDef, got {:?}", other),
    };
    let handler = &ann.handlers[0];
    let body_items = match &handler.body {
        crate::ast::Expr::Block(block, _) => &block.items,
        other => panic!("expected block body, got {:?}", other),
    };
    assert!(body_items.iter().any(|item| matches!(
        item,
        crate::ast::BlockItem::Statement(crate::ast::Statement::ReplaceBodyExpr { .. })
    )));
}

#[test]
fn test_annotation_replace_module_expr_directive_parse() {
    let content = r#"
        annotation schema() {
            targets: [module]
            comptime post(target, ctx) {
                replace module (gen_module(target))
            }
        }
    "#;

    let result = parse_program_helper(content).expect("parse should succeed");
    let ann = match &result[0] {
        crate::ast::Item::AnnotationDef(ann_def, _) => ann_def,
        other => panic!("expected AnnotationDef, got {:?}", other),
    };
    assert!(
        ann.allowed_targets
            .as_ref()
            .is_some_and(|targets| targets.contains(&crate::ast::AnnotationTargetKind::Module)),
        "annotation should allow module targets"
    );

    let handler = &ann.handlers[0];
    let body_items = match &handler.body {
        crate::ast::Expr::Block(block, _) => &block.items,
        other => panic!("expected block body, got {:?}", other),
    };
    assert!(body_items.iter().any(|item| matches!(
        item,
        crate::ast::BlockItem::Statement(crate::ast::Statement::ReplaceModuleExpr { .. })
    )));
}
