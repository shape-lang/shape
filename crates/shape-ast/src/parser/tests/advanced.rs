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
fn test_typeof_is_valid_identifier() {
    // typeof is no longer a reserved keyword — it parses as a regular function call.
    let content = r#"
        function test() {
            return typeof(1)
        }
    "#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "typeof should parse as a regular identifier/function call"
    );
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

// =========================================================================
// ADR-009 C3 #14 (slice 4): typed annotation config params
// =========================================================================

/// Extract the sole AnnotationDef from parsed items.
fn only_annotation_def(items: &[crate::ast::Item]) -> &crate::ast::AnnotationDef {
    match &items[0] {
        crate::ast::Item::AnnotationDef(ann_def, _) => ann_def,
        other => panic!("Expected AnnotationDef, got {:?}", other),
    }
}

#[test]
fn test_annotation_def_with_typed_config_params() {
    // ADR-009 C3 #14 (slice 4): `annotation retry(times: int, label: string)` —
    // each config param's type annotation fills the EXISTING
    // `FunctionParameter.type_annotation` field (no new AST field).
    let content = r#"
        annotation retry(times: int, label: string) {
            comptime post(target, ctx) {
                install(before_hook(body_fn, [capture("times", times)]))
            }
        }
    "#;
    let items = parse_program_helper(content).expect("typed config params parse");
    let ann_def = only_annotation_def(&items);
    assert_eq!(ann_def.name, "retry");
    assert_eq!(ann_def.params.len(), 2);
    assert_eq!(ann_def.params[0].simple_name(), Some("times"));
    assert_eq!(
        ann_def.params[0].type_annotation,
        Some(crate::ast::TypeAnnotation::Basic("int".to_string()))
    );
    assert_eq!(ann_def.params[1].simple_name(), Some("label"));
    assert_eq!(
        ann_def.params[1].type_annotation,
        Some(crate::ast::TypeAnnotation::Basic("string".to_string()))
    );
}

#[test]
fn test_annotation_def_with_composite_typed_config_params() {
    // Composite ConstLift-domain spellings parse at the grammar tier
    // (domain checking is the compiler's declaration-site check, not the
    // parser's).
    let content = r#"
        annotation windowed(sizes: Array<int>, fallback: Option<int>) {
            comptime post(target, ctx) {
                install(before_hook(body_fn, [capture("sizes", sizes)]))
            }
        }
    "#;
    let items = parse_program_helper(content).expect("composite typed config params parse");
    let ann_def = only_annotation_def(&items);
    assert_eq!(ann_def.params.len(), 2);
    // `Array<int>` normalizes to the dedicated Array variant at parse time.
    assert!(matches!(
        ann_def.params[0].type_annotation,
        Some(crate::ast::TypeAnnotation::Array(_))
    ));
    assert!(
        ann_def.params[1].type_annotation.is_some(),
        "Option<int> annotation must be carried"
    );
}

#[test]
fn test_annotation_def_untyped_config_params_stay_unannotated() {
    // Legacy (pre-S6) spelling: untyped params must keep parsing and produce
    // a byte-identical AST shape — `type_annotation: None` on every param.
    let content = r#"
        annotation warmup(period, mode) {
            metadata() { { warmup_period: period } }
        }
    "#;
    let items = parse_program_helper(content).expect("untyped config params parse");
    let ann_def = only_annotation_def(&items);
    assert_eq!(ann_def.params.len(), 2);
    assert_eq!(ann_def.params[0].simple_name(), Some("period"));
    assert_eq!(ann_def.params[0].type_annotation, None);
    assert_eq!(ann_def.params[1].simple_name(), Some("mode"));
    assert_eq!(ann_def.params[1].type_annotation, None);
}

#[test]
fn test_annotation_def_mixed_typed_untyped_params_parse() {
    // A MIX parses at the grammar tier (the annotation is grammatically
    // optional per param); rejecting the mix is the compiler's
    // declaration-site classification rule (R2), which needs the parsed AST
    // to name the first untyped param in its sentence.
    let content = r#"
        annotation partial(times: int, label) {
            comptime post(target, ctx) { noop() }
        }
    "#;
    let items = parse_program_helper(content).expect("mixed params parse at the grammar tier");
    let ann_def = only_annotation_def(&items);
    assert_eq!(ann_def.params.len(), 2);
    assert!(ann_def.params[0].type_annotation.is_some());
    assert!(ann_def.params[1].type_annotation.is_none());
}

#[test]
fn test_annotation_def_typed_param_span_integrity() {
    // The param pattern span must cover exactly the parameter NAME (the
    // identifier), for both typed and untyped params — declaration-site
    // rejections anchor there.
    let content = "annotation retry(times: int, label) { metadata() { { t: times } } }";
    let items = parse_program_helper(content).expect("fixture parses");
    let ann_def = only_annotation_def(&items);
    let times_span = ann_def.params[0].span();
    assert_eq!(
        &content[times_span.start..times_span.end],
        "times",
        "typed param span covers the identifier only"
    );
    let label_span = ann_def.params[1].span();
    assert_eq!(
        &content[label_span.start..label_span.end],
        "label",
        "untyped param span covers the identifier only"
    );
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
    let content =
        include_str!("../../../../shape-runtime/stdlib-src/finance/indicators/trend.shape");
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
    let program = parse_program(content).expect("doc comment should parse");
    assert_eq!(program.items.len(), 1);
    assert_eq!(
        program
            .docs
            .comment_for_path("foo")
            .map(|doc| doc.summary.as_str()),
        Some("This is a doc comment")
    );
}

#[test]
fn test_doc_comment_block() {
    let content = r#"
/** This is a block doc comment */
function foo() { return 1 }
"#;
    let program = parse_program(content).expect("block doc comment should be parsed as comment");
    assert_eq!(program.items.len(), 1);
    assert!(program.docs.comment_for_path("foo").is_none());
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
            method filter(predicate: (T) => bool) -> Self;
            method execute() -> Result<Table>;
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
            method filter(predicate: (T) => bool) -> Self;
            method execute() -> Result<Table<T>>;
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
fn test_trait_with_supertrait_colon_syntax() {
    let content = r#"
        trait AdvancedQueryable<T>: Queryable<T> {
            method groupBy(column: string) -> Self;
        }
    "#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "Trait with supertrait : syntax should parse: {:?}",
        result.err()
    );
    let items = result.unwrap();
    match &items[0] {
        Item::Trait(def, _) => {
            assert_eq!(def.name, "AdvancedQueryable");
            assert_eq!(def.super_traits.len(), 1);
            match &def.super_traits[0] {
                crate::ast::TypeAnnotation::Generic { name, args } => {
                    assert_eq!(name, "Queryable");
                    assert_eq!(args.len(), 1);
                }
                other => panic!("expected Generic supertrait, got {:?}", other),
            }
        }
        other => panic!("expected Trait, got {:?}", other),
    }
}

#[test]
fn test_trait_with_multiple_supertraits() {
    let content = r#"
        trait Foo: Bar + Baz {
            method method() -> int
        }
    "#;
    let result = parse_program_helper(content);
    assert!(
        result.is_ok(),
        "Trait with multiple supertraits should parse: {:?}",
        result.err()
    );
    let items = result.unwrap();
    match &items[0] {
        Item::Trait(def, _) => {
            assert_eq!(def.name, "Foo");
            assert_eq!(def.super_traits.len(), 2);
            assert_eq!(def.super_traits[0].as_simple_name(), Some("Bar"));
            assert_eq!(def.super_traits[1].as_simple_name(), Some("Baz"));
        }
        other => panic!("expected Trait, got {:?}", other),
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
                crate::ast::TypeName::Simple("Queryable".into())
            );
            assert_eq!(
                impl_block.target_type,
                crate::ast::TypeName::Simple("Table".into())
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
        annotation only_types() on type, expression {
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
        annotation transform() on expression {
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
        annotation schema() on function {
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
fn test_annotation_set_param_type_expr_directive_parse() {
    let content = r#"
        annotation schema() on function {
            comptime post(target, ctx) {
                set param uri: (string)
                set param value: (target.params[0].type_ref)
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
        crate::ast::BlockItem::Statement(crate::ast::Statement::SetParamTypeExpr { .. })
    )));
}

#[test]
fn test_annotation_replace_body_expr_directive_parse() {
    let content = r#"
        annotation schema() on function {
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
        annotation schema() on module {
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

// ===== ADR-009 E4-D4 (slice 1, issue #73): header `on`-clause targets =====
//
// The header spelling `annotation NAME(config)? on <kind>, ... { ... }`
// populates `allowed_targets` FROM THE HEADER, and after S1b it is the ONE
// accepted spelling: the legacy body `targets: [...]` field is a tombstone that
// yields a NAMED migration rejection (see
// `test_legacy_body_targets_field_is_rejected_with_named_migration_diagnostic`
// below). These pins exercise the header spelling.

/// Pull the single `AnnotationDef` out of a parsed program.
fn annotation_def_of(items: &[crate::ast::Item]) -> &crate::ast::AnnotationDef {
    match &items[0] {
        crate::ast::Item::AnnotationDef(ann_def, _) => ann_def,
        other => panic!("expected AnnotationDef, got {:?}", other),
    }
}

#[test]
fn test_annotation_header_on_clause_single_kind() {
    let content = r#"
        annotation traced(f: int) on function {
            before(args) { args }
        }
    "#;
    let items = parse_program_helper(content).expect("header on-clause should parse");
    let ann = annotation_def_of(&items);
    assert_eq!(ann.name, "traced");
    assert_eq!(
        ann.allowed_targets,
        Some(vec![crate::ast::AnnotationTargetKind::Function]),
        "single-kind header should populate allowed_targets from the header"
    );
    // config param survives alongside the on-clause
    assert_eq!(ann.params.len(), 1);
}

#[test]
fn test_annotation_header_on_clause_multi_kind() {
    let content = r#"
        annotation only_defs() on function, type {
            comptime post(target, ctx) {
                target.kind
            }
        }
    "#;
    let items = parse_program_helper(content).expect("multi-kind header should parse");
    let ann = annotation_def_of(&items);
    assert_eq!(
        ann.allowed_targets,
        Some(vec![
            crate::ast::AnnotationTargetKind::Function,
            crate::ast::AnnotationTargetKind::Type,
        ]),
        "multi-kind header should populate both kinds in source order"
    );
}

#[test]
fn test_annotation_header_on_clause_all_seven_kinds() {
    // Anti-undercount pin (2026-07-22 issue-#73 correction): every one of the
    // SEVEN AnnotationTargetKind kinds is header-eligible and maps to its own
    // variant. Each variant is asserted individually so a 4-of-7 (or any-of-7)
    // regression fails loudly rather than passing on a length check.
    let content = r#"
        annotation everywhere() on function, type, module, expression, block, await_expr, binding {
            metadata() { 1 }
        }
    "#;
    let items = parse_program_helper(content).expect("all-seven-kind header should parse");
    let ann = annotation_def_of(&items);
    let targets = ann
        .allowed_targets
        .clone()
        .expect("all-seven header should populate allowed_targets");
    use crate::ast::AnnotationTargetKind::*;
    assert_eq!(
        targets,
        vec![
            Function, Type, Module, Expression, Block, AwaitExpr, Binding,
        ],
        "all seven kinds must be header-eligible, each mapping to its own variant"
    );
    // Individual variant pins — guard the exact undercount class the
    // issue-#73 correction called out.
    assert!(targets.contains(&Function));
    assert!(targets.contains(&Type));
    assert!(targets.contains(&Module));
    assert!(targets.contains(&Expression));
    assert!(targets.contains(&Block));
    assert!(targets.contains(&AwaitExpr));
    assert!(targets.contains(&Binding));
}

#[test]
fn test_annotation_absent_on_clause_leaves_targets_none() {
    // DN1: a missing on-clause yields None, so target applicability falls
    // through to the existing handler-kind inference (planner). The header
    // OVERRIDES inference when present and is a no-op when absent — it never
    // hardcodes a default here.
    let content = r#"
        annotation traced() {
            before(args) { args }
        }
    "#;
    let items = parse_program_helper(content).expect("annotation without on-clause should parse");
    let ann = annotation_def_of(&items);
    assert_eq!(
        ann.allowed_targets, None,
        "absent on-clause must leave allowed_targets as None (inference default)"
    );
}

#[test]
fn test_legacy_body_targets_field_is_rejected_with_named_migration_diagnostic() {
    // S1b tombstone (DN2): the removed body `targets: [...]` field must produce
    // a NAMED migration diagnostic pointing at the header `on`-clause — never an
    // opaque pest error and never a valid AST. Asserting the message text (not a
    // bare `is_err`) is load-bearing: a future grammar rework that silently
    // turned this into a *different* parse error would pass a bare `is_err`
    // vacuously. Twins the `@annotation`-rejection assertion pattern above.
    //
    // S1c fixup: the S1b commit shipped this fixture in the HEADER form by
    // copy-paste, so it parsed cleanly and the `expect_err` panicked (the pin
    // was RED, not vacuously green). Restored to the body `targets: [...]` form
    // — the spelling this tombstone actually rejects.
    let content = r#"
        annotation legacy_form() {
            targets: [function]
            comptime post(target, ctx) {
                target.kind
            }
        }
    "#;
    let err = parse_program_helper(content).expect_err(
        "body `targets: [...]` field must be rejected after S1b — one accepted spelling",
    );
    let msg = err.to_string();
    assert!(
        msg.contains("annotation targets moved to the header `on` clause"),
        "rejection must name the migration to the header on-clause, got: {msg}"
    );
    assert!(
        msg.contains("issue #73"),
        "rejection should cite issue #73 for the migration, got: {msg}"
    );
}

#[test]
fn test_legacy_body_targets_positive_twin_header_form_parses() {
    // Positive twin of the tombstone: the SAME target set written in the header
    // form parses cleanly and populates `allowed_targets == Some([Function])`.
    let content = r#"
        annotation legacy_form() on function {
            comptime post(target, ctx) {
                target.kind
            }
        }
    "#;
    let items = parse_program_helper(content).expect("header form must parse");
    let ann = annotation_def_of(&items);
    assert_eq!(
        ann.allowed_targets,
        Some(vec![crate::ast::AnnotationTargetKind::Function]),
        "header on-clause must populate allowed_targets == Some([Function])"
    );
}

// ===== Regression: temporal-nav identifier hijack =====
//
// The deleted `back_nav` / `forward_nav` grammar rules sat ahead of `ident` in the
// `primary` alternation, so a call to a user-defined function named `back` or
// `forward` with a literal numeric argument was silently rewritten into a Duration
// (`back(3)` evaluated to -PT3S instead of calling the function). Silent wrong
// results, no diagnostic. These tests pin the identifiers as ordinary calls.

/// Pull the initializer expression out of the first `let` in a program.
fn first_let_init(items: &[crate::ast::Item]) -> &crate::ast::Expr {
    items
        .iter()
        .find_map(|item| match item {
            crate::ast::Item::Statement(crate::ast::Statement::VariableDecl(decl, _), _) => {
                decl.value.as_ref()
            }
            _ => None,
        })
        .expect("expected a variable declaration")
}

/// `back(3)` must parse as a CALL to the user's function, never as a Duration.
#[test]
fn back_is_an_ordinary_identifier_not_temporal_nav() {
    let items = parse_program_helper("fn back(x: int) -> int { return x * 2 }\nlet r = back(3)\n")
        .expect("parse should succeed");

    match first_let_init(&items) {
        crate::ast::Expr::FunctionCall { name, .. } => {
            assert_eq!(name, "back", "call must target the user fn `back`");
        }
        crate::ast::Expr::Duration(d, _) => {
            panic!("`back(3)` was hijacked into a Duration ({d:?}) — the temporal-nav regression");
        }
        other => panic!("expected a call to `back`, got {other:?}"),
    }
}

/// Same for `forward`, the sibling temporal-nav rule.
#[test]
fn forward_is_an_ordinary_identifier_not_temporal_nav() {
    let items =
        parse_program_helper("fn forward(x: int) -> int { return x + 1 }\nlet r = forward(10)\n")
            .expect("parse should succeed");

    match first_let_init(&items) {
        crate::ast::Expr::FunctionCall { name, .. } => {
            assert_eq!(name, "forward", "call must target the user fn `forward`");
        }
        crate::ast::Expr::Duration(d, _) => {
            panic!(
                "`forward(10)` was hijacked into a Duration ({d:?}) — the temporal-nav regression"
            );
        }
        other => panic!("expected a call to `forward`, got {other:?}"),
    }
}

/// Duration LITERALS are a separate, working feature — deleting temporal-nav must not break them.
#[test]
fn duration_literals_still_parse() {
    let items = parse_program_helper("let a = 5m\nlet b = 1.5d\n").expect("parse should succeed");
    let durations = items
        .iter()
        .filter(|item| {
            matches!(
                item,
                crate::ast::Item::Statement(crate::ast::Statement::VariableDecl(decl, _), _)
                    if matches!(decl.value, Some(crate::ast::Expr::Duration(..)))
            )
        })
        .count();
    assert_eq!(
        durations, 2,
        "both `5m` and `1.5d` must remain Duration literals"
    );
}

// ═══ ADR-009 C3 #14 (slice 4, C3-G12) — the nested-fn annotation CARRIER ═══
// The `let name = fn(...)` desugar of a fn-local nested `fn` formerly
// DROPPED spelled annotations silently (S0 a4/a4c). Both desugar sites
// (statement position, parser/statements.rs; block-item position,
// parser/expressions/control_flow/loops.rs) now thread them onto
// `Expr::FunctionExpr.annotations` so the compiler can fire the loud
// typed-config rejection. Closure literals stay `None`.

#[test]
fn nested_fn_annotations_are_carried_through_the_desugar() {
    let items = parse_program_helper(
        "fn outer() -> int {\n  @retry(3)\n  fn inner(x: int) -> int { return x }\n  return inner(4)\n}\n",
    )
    .expect("parse should succeed");
    let crate::ast::Item::Function(outer, _) = &items[0] else {
        panic!("expected outer fn item");
    };
    let crate::ast::Statement::VariableDecl(decl, _) = &outer.body[0] else {
        panic!("expected the nested fn to desugar to a VariableDecl, got {:?}", outer.body[0]);
    };
    let Some(crate::ast::Expr::FunctionExpr { annotations, .. }) = &decl.value else {
        panic!("expected a FunctionExpr initializer");
    };
    let annotations = annotations.as_deref().expect("annotations must be carried, not dropped");
    assert_eq!(annotations.len(), 1);
    assert_eq!(annotations[0].name, "retry");
    assert_eq!(annotations[0].args.len(), 1);
}

#[test]
fn closure_literal_carries_no_nested_fn_annotations() {
    let items = parse_program_helper("let f = |x| x + 1\n").expect("parse should succeed");
    let crate::ast::Item::Statement(crate::ast::Statement::VariableDecl(decl, _), _) = &items[0]
    else {
        panic!("expected a let statement, got {:?}", items[0]);
    };
    let Some(crate::ast::Expr::FunctionExpr { annotations, .. }) = &decl.value else {
        panic!("expected a FunctionExpr initializer");
    };
    assert!(annotations.is_none(), "closure literals never carry annotations");
}
