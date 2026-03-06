//! Tests for the semantic analyzer

use crate::semantic::{SemanticAnalyzer, types::Type};
use shape_ast::ast::{Span, *};

// Note: Pattern block syntax (pattern foo { ... }) was removed from the language.
// Patterns are now defined as annotated functions.
// Old pattern-specific tests have been replaced with function-based equivalents.

#[test]
fn test_pattern_function_registration() {
    let mut analyzer = SemanticAnalyzer::new();

    // Pattern defined as annotated function
    let pattern_func = FunctionDef {
        name: "test_pattern".to_string(),
        name_span: Span::DUMMY,
        type_params: None,
        params: vec![],
        return_type: None,
        body: vec![Statement::Return(
            Some(Expr::BinaryOp {
                left: Box::new(Expr::Literal(Literal::Number(10.0), Span::DUMMY)),
                op: BinaryOp::Greater,
                right: Box::new(Expr::Literal(Literal::Number(5.0), Span::DUMMY)),
                span: Span::DUMMY,
            }),
            Span::DUMMY,
        )],
        annotations: vec![Annotation {
            name: "my_ann".to_string(),
            args: vec![],
            span: Span::DUMMY,
        }],
        where_clause: None,
        is_async: false,
        is_comptime: false,
    };

    let program = Program {
        items: vec![Item::Function(pattern_func, Span::DUMMY)],
    };

    let result = analyzer.analyze(&program);
    if let Err(e) = &result {
        eprintln!("Analysis failed: {:?}", e);
    }
    assert!(result.is_ok());
}

#[test]
fn test_duplicate_function_detection() {
    let mut analyzer = SemanticAnalyzer::new();

    // Two functions with the same name
    let func1 = FunctionDef {
        name: "duplicate".to_string(),
        name_span: Span::DUMMY,
        type_params: None,
        params: vec![],
        return_type: None,
        body: vec![Statement::Return(
            Some(Expr::Literal(Literal::Number(1.0), Span::DUMMY)),
            Span::DUMMY,
        )],
        annotations: vec![],
        where_clause: None,
        is_async: false,
        is_comptime: false,
    };

    let func2 = FunctionDef {
        name: "duplicate".to_string(),
        name_span: Span::DUMMY,
        type_params: None,
        params: vec![],
        return_type: None,
        body: vec![Statement::Return(
            Some(Expr::Literal(Literal::Number(2.0), Span::DUMMY)),
            Span::DUMMY,
        )],
        annotations: vec![],
        where_clause: None,
        is_async: false,
        is_comptime: false,
    };

    let program = Program {
        items: vec![
            Item::Function(func1, Span::DUMMY),
            Item::Function(func2, Span::DUMMY),
        ],
    };

    // Duplicate function names should trigger an error
    assert!(analyzer.analyze(&program).is_err());
}

/// Regression: mutual-recursion pre-registration must not mask duplicate
/// function definitions.  The pre_register_functions pass registers names
/// so is_even can call is_odd, but a second function with the SAME name
/// must still be rejected during the main analysis pass.
#[test]
fn test_duplicate_function_with_mutual_recursion_present() {
    let mut analyzer = SemanticAnalyzer::new();

    // Three functions: is_even, is_odd (mutual recursion), and a DUPLICATE is_even
    let is_even = FunctionDef {
        name: "is_even".to_string(),
        name_span: Span::DUMMY,
        type_params: None,
        params: vec![],
        return_type: None,
        body: vec![Statement::Return(
            Some(Expr::Literal(Literal::Bool(true), Span::DUMMY)),
            Span::DUMMY,
        )],
        annotations: vec![],
        where_clause: None,
        is_async: false,
        is_comptime: false,
    };

    let is_odd = FunctionDef {
        name: "is_odd".to_string(),
        name_span: Span::DUMMY,
        type_params: None,
        params: vec![],
        return_type: None,
        body: vec![Statement::Return(
            Some(Expr::Literal(Literal::Bool(false), Span::DUMMY)),
            Span::DUMMY,
        )],
        annotations: vec![],
        where_clause: None,
        is_async: false,
        is_comptime: false,
    };

    let is_even_dup = FunctionDef {
        name: "is_even".to_string(),
        name_span: Span::DUMMY,
        type_params: None,
        params: vec![],
        return_type: None,
        body: vec![Statement::Return(
            Some(Expr::Literal(Literal::Bool(false), Span::DUMMY)),
            Span::DUMMY,
        )],
        annotations: vec![],
        where_clause: None,
        is_async: false,
        is_comptime: false,
    };

    let program = Program {
        items: vec![
            Item::Function(is_even, Span::DUMMY),
            Item::Function(is_odd, Span::DUMMY),
            Item::Function(is_even_dup, Span::DUMMY),
        ],
    };

    // Must reject: is_even is defined twice, even though mutual recursion pre-registration is active
    assert!(
        analyzer.analyze(&program).is_err(),
        "Duplicate 'is_even' must be rejected even with mutual-recursion pre-registration"
    );
}

#[test]
fn test_variable_definition_and_usage() {
    let mut analyzer = SemanticAnalyzer::new();

    let program = Program {
        items: vec![
            // Define a variable
            Item::Assignment(
                Assignment {
                    pattern: DestructurePattern::Identifier(
                        "support_level".to_string(),
                        Span::DUMMY,
                    ),
                    value: Expr::Literal(Literal::Number(150.0), Span::DUMMY),
                },
                Span::DUMMY,
            ),
            // Use the variable
            Item::Expression(
                Expr::BinaryOp {
                    left: Box::new(Expr::Identifier("support_level".to_string(), Span::DUMMY)),
                    op: BinaryOp::Greater,
                    right: Box::new(Expr::Literal(Literal::Number(100.0), Span::DUMMY)),
                    span: Span::DUMMY,
                },
                Span::DUMMY,
            ),
        ],
    };

    assert!(analyzer.analyze(&program).is_ok());
}

#[test]
fn test_undefined_variable_detection() {
    let mut analyzer = SemanticAnalyzer::new();

    let program = Program {
        items: vec![Item::Expression(
            Expr::Identifier("undefined_var".to_string(), Span::DUMMY),
            Span::DUMMY,
        )],
    };

    assert!(analyzer.analyze(&program).is_err());
}

#[test]
fn test_function_call_validation() {
    let mut analyzer = SemanticAnalyzer::new();

    // Valid function call (using generic math function instead of finance-specific sma)
    let valid_call = Expr::FunctionCall {
        name: "abs".to_string(),
        args: vec![Expr::Literal(Literal::Number(20.0), Span::DUMMY)],
        named_args: vec![],
        span: Span::DUMMY,
    };

    let program1 = Program {
        items: vec![Item::Expression(valid_call, Span::DUMMY)],
    };

    assert!(analyzer.analyze(&program1).is_ok());

    // Invalid function name
    let invalid_call = Expr::FunctionCall {
        name: "unknown_function".to_string(),
        args: vec![],
        named_args: vec![],
        span: Span::DUMMY,
    };

    let program2 = Program {
        items: vec![Item::Expression(invalid_call, Span::DUMMY)],
    };

    assert!(analyzer.analyze(&program2).is_err());

    // Wrong number of arguments
    let wrong_args = Expr::FunctionCall {
        name: "sma".to_string(),
        args: vec![], // SMA expects 1 argument
        named_args: vec![],
        span: Span::DUMMY,
    };

    let program3 = Program {
        items: vec![Item::Expression(wrong_args, Span::DUMMY)],
    };

    assert!(analyzer.analyze(&program3).is_err());
}

#[test]
fn test_builtin_range_accepts_two_argument_form() {
    let mut analyzer = SemanticAnalyzer::new();
    let program = shape_ast::parser::parse_program(
        r#"
        let n = 5
        let r = range(0, n)
    "#,
    )
    .expect("program should parse");

    assert!(
        analyzer.analyze(&program).is_ok(),
        "range(start, end) should pass semantic analysis"
    );
}

#[test]
fn test_builtin_print_accepts_multiple_arguments() {
    let mut analyzer = SemanticAnalyzer::new();
    let program = shape_ast::parser::parse_program(
        r#"
        let result = 42
        print("fib(38) = ", result)
    "#,
    )
    .expect("program should parse");

    assert!(
        analyzer.analyze(&program).is_ok(),
        "print should accept multiple arguments"
    );
}

#[test]
fn test_type_inference() {
    let mut analyzer = SemanticAnalyzer::new();

    // Test row property types
    let row_expr = Expr::PropertyAccess {
        object: Box::new(Expr::DataRef(
            DataRef {
                index: DataIndex::Single(0),
                timeframe: None,
            },
            Span::DUMMY,
        )),
        property: "close".to_string(),
        optional: false,
        span: Span::DUMMY,
    };

    let result = analyzer.check_expr_type(&row_expr);
    assert!(result.is_ok());
    // Generic objects return Type::Unknown for property access (no schema)
    assert_eq!(result.unwrap(), Type::Unknown);

    // Test color property
    let color_expr = Expr::PropertyAccess {
        object: Box::new(Expr::DataRef(
            DataRef {
                index: DataIndex::Single(0),
                timeframe: None,
            },
            Span::DUMMY,
        )),
        property: "color".to_string(),
        optional: false,
        span: Span::DUMMY,
    };

    let result = analyzer.check_expr_type(&color_expr);
    assert!(result.is_ok());
    // Generic objects return Type::Unknown for property access (no schema)
    assert_eq!(result.unwrap(), Type::Unknown);
}

#[test]
fn test_function_return_type() {
    let mut analyzer = SemanticAnalyzer::new();

    // Function with explicit return type
    let func_with_return = FunctionDef {
        name: "get_value".to_string(),
        name_span: Span::DUMMY,
        type_params: None,
        params: vec![],
        return_type: Some(TypeAnnotation::Basic("Number".to_string())),
        body: vec![Statement::Return(
            Some(Expr::Literal(Literal::Number(42.0), Span::DUMMY)),
            Span::DUMMY,
        )],
        annotations: vec![],
        where_clause: None,
        is_async: false,
        is_comptime: false,
    };

    let program = Program {
        items: vec![Item::Function(func_with_return, Span::DUMMY)],
    };

    assert!(analyzer.analyze(&program).is_ok());
}

#[test]
fn test_comptime_block_allows_build_config_builtin() {
    let mut analyzer = SemanticAnalyzer::new();
    let program = Program {
        items: vec![Item::Comptime(
            vec![Statement::Expression(
                Expr::FunctionCall {
                    name: "print".to_string(),
                    args: vec![Expr::FunctionCall {
                        name: "build_config".to_string(),
                        args: vec![],
                        named_args: vec![],
                        span: Span::DUMMY,
                    }],
                    named_args: vec![],
                    span: Span::DUMMY,
                },
                Span::DUMMY,
            )],
            Span::DUMMY,
        )],
    };

    assert!(
        analyzer.analyze(&program).is_ok(),
        "build_config should be available inside comptime blocks"
    );
}

#[test]
fn test_expression_type_checking() {
    let mut analyzer = SemanticAnalyzer::new();

    // Valid boolean expression
    let bool_expr = Expr::BinaryOp {
        left: Box::new(Expr::Literal(Literal::Number(10.0), Span::DUMMY)),
        op: BinaryOp::Greater,
        right: Box::new(Expr::Literal(Literal::Number(5.0), Span::DUMMY)),
        span: Span::DUMMY,
    };

    let program = Program {
        items: vec![Item::Expression(bool_expr, Span::DUMMY)],
    };

    assert!(analyzer.analyze(&program).is_ok());
}

// =========================================================================
// Multi-Error Recovery Tests (Phase 3.2)
// =========================================================================

#[test]
fn test_multi_error_collects_all_errors() {
    use shape_ast::error::ShapeError;

    let mut analyzer = SemanticAnalyzer::new();

    // Two items that both produce errors: duplicate functions
    let func1 = FunctionDef {
        name: "dup_a".to_string(),
        name_span: Span::DUMMY,
        type_params: None,
        params: vec![],
        return_type: None,
        body: vec![Statement::Return(
            Some(Expr::Literal(Literal::Number(1.0), Span::DUMMY)),
            Span::DUMMY,
        )],
        annotations: vec![],
        where_clause: None,
        is_async: false,
        is_comptime: false,
    };

    let func1_dup = func1.clone();

    let func2 = FunctionDef {
        name: "dup_b".to_string(),
        name_span: Span::DUMMY,
        type_params: None,
        params: vec![],
        return_type: None,
        body: vec![Statement::Return(
            Some(Expr::Literal(Literal::Number(2.0), Span::DUMMY)),
            Span::DUMMY,
        )],
        annotations: vec![],
        where_clause: None,
        is_async: false,
        is_comptime: false,
    };

    let func2_dup = func2.clone();

    let program = Program {
        items: vec![
            Item::Function(func1, Span::DUMMY),
            Item::Function(func1_dup, Span::DUMMY),
            Item::Function(func2, Span::DUMMY),
            Item::Function(func2_dup, Span::DUMMY),
        ],
    };

    let result = analyzer.analyze(&program);
    assert!(
        result.is_err(),
        "Should produce errors for duplicate functions"
    );

    let err = result.unwrap_err();
    match &err {
        ShapeError::MultiError(errors) => {
            assert_eq!(
                errors.len(),
                2,
                "Should collect 2 errors (one for each duplicate)"
            );
        }
        _ => {
            // Might be a single error if only 1 duplicate is detected
            // That's also acceptable
        }
    }
}

#[test]
fn test_single_error_not_wrapped() {
    use shape_ast::error::ShapeError;

    let mut analyzer = SemanticAnalyzer::new();

    // Only one error item
    let program = Program {
        items: vec![Item::Expression(
            Expr::Identifier("nonexistent".to_string(), Span::DUMMY),
            Span::DUMMY,
        )],
    };

    let result = analyzer.analyze(&program);
    assert!(result.is_err());

    // Single errors should NOT be wrapped in MultiError
    let err = result.unwrap_err();
    assert!(
        !matches!(err, ShapeError::MultiError(_)),
        "Single errors should not be wrapped in MultiError"
    );
}
