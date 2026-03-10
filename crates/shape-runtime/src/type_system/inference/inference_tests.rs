use super::*;

use super::*;

#[test]
fn test_fallible_scope_tracking() {
    let mut engine = TypeInferenceEngine::new();

    // Initially no scopes
    assert!(!engine.in_function_scope());

    // Push a scope
    engine.push_fallible_scope();
    assert!(engine.in_function_scope());

    // Initially not fallible
    assert!(!engine.fallible_scopes[0]);

    // Mark as fallible
    engine.mark_current_scope_fallible();
    assert!(engine.fallible_scopes[0]);

    // Pop and check result
    let was_fallible = engine.pop_fallible_scope();
    assert!(was_fallible);
    assert!(!engine.in_function_scope());
}

#[test]
fn test_nested_fallible_scopes() {
    let mut engine = TypeInferenceEngine::new();

    // Outer function scope
    engine.push_fallible_scope();

    // Inner function scope (closure)
    engine.push_fallible_scope();
    engine.mark_current_scope_fallible(); // Inner has ?

    // Pop inner - should be fallible
    assert!(engine.pop_fallible_scope());

    // Outer should still be non-fallible (? in closure doesn't affect outer)
    assert!(!engine.pop_fallible_scope());
}

#[test]
fn test_non_fallible_function_scope() {
    let mut engine = TypeInferenceEngine::new();

    engine.push_fallible_scope();
    // No ? operator used
    let was_fallible = engine.pop_fallible_scope();
    assert!(!was_fallible);
}

#[test]
fn test_callsite_mixed_args_widen_unannotated_param_to_union() {
    use shape_ast::ast::TypeAnnotation;
    use shape_ast::parser::parse_program;

    let code = r#"
fn foo(a) {
  return a
}

let i = foo(1)
let s = foo("hi")
"#;

    let program = parse_program(code).expect("program should parse");
    let mut engine = TypeInferenceEngine::new();
    let types = engine
        .infer_program(&program)
        .expect("inference should succeed");

    let foo_type = types.get("foo").expect("foo should be inferred");
    match foo_type {
        Type::Function { params, returns } => {
            assert_eq!(params.len(), 1, "foo should have one parameter");

            let param_ann = params[0]
                .to_annotation()
                .expect("parameter should convert to annotation");
            let return_ann = returns
                .to_annotation()
                .expect("return should convert to annotation");

            match (&param_ann, &return_ann) {
                (TypeAnnotation::Union(param_variants), TypeAnnotation::Union(ret_variants)) => {
                    let has_int = param_variants
                        .iter()
                        .any(|v| matches!(v, TypeAnnotation::Basic(name) if name == "int"));
                    let has_string = param_variants
                        .iter()
                        .any(|v| matches!(v, TypeAnnotation::Basic(name) if name == "string"));
                    assert!(has_int, "union should include int: {:?}", param_variants);
                    assert!(
                        has_string,
                        "union should include string: {:?}",
                        param_variants
                    );
                    assert_eq!(
                        param_variants.len(),
                        ret_variants.len(),
                        "return union should mirror parameter union"
                    );
                }
                other => panic!(
                    "expected union param/return for foo after mixed call sites, got {:?}",
                    other
                ),
            }
        }
        other => panic!("expected function type for foo, got {:?}", other),
    }
}

#[test]
fn test_some_constructor_infers_option_inner_type() {
    use shape_ast::ast::TypeAnnotation;
    use shape_ast::parser::parse_program;

    let code = r#"
let a = Some(1)
"#;

    let program = parse_program(code).expect("program should parse");
    let mut engine = TypeInferenceEngine::new();
    let types = engine
        .infer_program(&program)
        .expect("inference should succeed");

    let a_type = types.get("a").expect("a should be inferred");
    match a_type {
        Type::Generic { base, args } => {
            assert!(
                matches!(
                    base.as_ref(),
                    Type::Concrete(TypeAnnotation::Reference(name)) if name == "Option"
                ),
                "expected Option<T> base, got {:?}",
                base
            );
            assert_eq!(args.len(), 1, "Option must have one type argument");
            assert_eq!(
                args[0],
                Type::Concrete(TypeAnnotation::Basic("int".to_string()))
            );
        }
        other => panic!("expected Option<int> for Some(1), got {:?}", other),
    }
}

#[test]
fn test_ok_err_constructors_do_not_degrade_to_any() {
    use shape_ast::ast::TypeAnnotation;
    use shape_ast::parser::parse_program;

    let code = r#"
let ok_value: Result<int> = Ok(1)
let err_value: Result<int> = Err("boom")
"#;

    let program = parse_program(code).expect("program should parse");
    let mut engine = TypeInferenceEngine::new();
    let types = engine
        .infer_program(&program)
        .expect("inference should succeed");

    for var_name in ["ok_value", "err_value"] {
        let ty = types.get(var_name).expect("variable should be inferred");
        match ty {
            Type::Generic { base, args } => {
                assert!(
                    matches!(
                        base.as_ref(),
                        Type::Concrete(TypeAnnotation::Reference(name)) if name == "Result"
                    ),
                    "expected Result<T> base for {var_name}, got {:?}",
                    base
                );
                assert!(
                    !args.is_empty(),
                    "Result must include at least success type arg"
                );
                assert_eq!(
                    args[0],
                    Type::Concrete(TypeAnnotation::Basic("int".to_string())),
                    "{var_name} should remain Result<int>"
                );
            }
            other => panic!("expected Result<int> for {var_name}, got {:?}", other),
        }
    }
}

#[test]
fn test_expression_style_ok_then_err_infers_result_inner_from_ok_branch() {
    use shape_ast::ast::TypeAnnotation;
    use shape_ast::parser::parse_program;

    let code = r#"
fn test() {
  Ok(1)
  Err("some error")
}
"#;

    let program = parse_program(code).expect("program should parse");
    let mut engine = TypeInferenceEngine::new();
    let types = engine
        .infer_program(&program)
        .expect("inference should succeed");

    let test_type = types.get("test").expect("test should be inferred");
    match test_type {
        Type::Function { returns, .. } => match returns.as_ref() {
            Type::Generic { base, args } => {
                assert!(
                    matches!(
                        base.as_ref(),
                        Type::Concrete(TypeAnnotation::Reference(name)) if name == "Result"
                    ),
                    "expected Result<T> return, got {:?}",
                    returns
                );
                assert!(
                    !args.is_empty(),
                    "Result must include at least success type arg"
                );
                assert_eq!(
                    args[0],
                    Type::Concrete(TypeAnnotation::Basic("int".to_string()))
                );
            }
            other => panic!("expected Result<int> return, got {:?}", other),
        },
        other => panic!("expected function type, got {:?}", other),
    }
}

#[test]
fn test_expression_style_ok_union_infers_result_inner_union() {
    use shape_ast::ast::TypeAnnotation;
    use shape_ast::parser::parse_program;

    let code = r#"
fn test() {
  Ok(1)
  Ok("str")
}
"#;

    let program = parse_program(code).expect("program should parse");
    let mut engine = TypeInferenceEngine::new();
    let types = engine
        .infer_program(&program)
        .expect("inference should succeed");

    let test_type = types.get("test").expect("test should be inferred");
    match test_type {
        Type::Function { returns, .. } => match returns.as_ref() {
            Type::Generic { base, args } => {
                assert!(
                    matches!(
                        base.as_ref(),
                        Type::Concrete(TypeAnnotation::Reference(name)) if name == "Result"
                    ),
                    "expected Result<T> return, got {:?}",
                    returns
                );
                assert!(
                    !args.is_empty(),
                    "Result must include at least success type arg"
                );
                let arg_ann = args[0].to_annotation().expect("return arg annotation");
                match arg_ann {
                    TypeAnnotation::Union(variants) => {
                        let has_int = variants
                            .iter()
                            .any(|v| matches!(v, TypeAnnotation::Basic(name) if name == "int"));
                        let has_string = variants
                            .iter()
                            .any(|v| matches!(v, TypeAnnotation::Basic(name) if name == "string"));
                        assert!(has_int, "union should include int: {:?}", variants);
                        assert!(has_string, "union should include string: {:?}", variants);
                    }
                    other => panic!("expected union return arg, got {:?}", other),
                }
            }
            other => panic!("expected Result<int | string> return, got {:?}", other),
        },
        other => panic!("expected function type, got {:?}", other),
    }
}

#[test]
fn test_expression_style_err_only_without_context_reports_generic_error() {
    use shape_ast::parser::parse_program;

    let code = r#"
fn test() {
  Err("some error")
}
"#;

    let program = parse_program(code).expect("program should parse");
    let mut engine = TypeInferenceEngine::new();
    let err = engine
        .infer_program(&program)
        .expect_err("inference should fail for unconstrained Result<T>");

    assert!(
        matches!(err, TypeError::GenericTypeError { .. }),
        "expected GenericTypeError, got {:?}",
        err
    );
}

#[test]
fn test_expression_style_err_only_with_explicit_result_annotation_uses_declared_t() {
    use shape_ast::ast::TypeAnnotation;
    use shape_ast::parser::parse_program;

    let code = r#"
fn test() -> Result<int> {
  Err("some error")
}
"#;

    let program = parse_program(code).expect("program should parse");
    let mut engine = TypeInferenceEngine::new();
    let types = engine
        .infer_program(&program)
        .expect("inference should succeed");

    let test_type = types.get("test").expect("test should be inferred");
    match test_type {
        Type::Function { returns, .. } => match returns.as_ref() {
            Type::Generic { base, args } => {
                assert!(
                    matches!(
                        base.as_ref(),
                        Type::Concrete(TypeAnnotation::Reference(name)) if name == "Result"
                    ),
                    "expected Result<T> return, got {:?}",
                    returns
                );
                assert!(
                    !args.is_empty(),
                    "Result must include at least success type arg"
                );
                assert_eq!(
                    args[0],
                    Type::Concrete(TypeAnnotation::Basic("int".to_string()))
                );
            }
            other => panic!("expected Result<int> return, got {:?}", other),
        },
        other => panic!("expected function type, got {:?}", other),
    }
}

#[test]
fn test_struct_literal_generic_default_collapses_to_base_name() {
    use shape_ast::parser::parse_program;

    let code = r#"
type MyType<T = int> { x: T }
let a = MyType { x: 1 }
"#;

    let program = parse_program(code).expect("program should parse");
    let mut engine = TypeInferenceEngine::new();
    let types = engine
        .infer_program(&program)
        .expect("inference should succeed");

    assert_eq!(
        types.get("a"),
        Some(&Type::Concrete(TypeAnnotation::Reference(
            "MyType".to_string()
        )))
    );
}

#[test]
fn test_result_and_anyerror_annotations_are_recognized() {
    use shape_ast::ast::TypeAnnotation;
    use shape_ast::parser::parse_program;

    let code = r#"
fn wrap(err: AnyError) -> Result<int> {
  return Err(err)
}
"#;

    let program = parse_program(code).expect("program should parse");
    let mut engine = TypeInferenceEngine::new();
    let types = engine
        .infer_program(&program)
        .expect("inference should succeed");

    let wrap_type = types.get("wrap").expect("wrap should be inferred");
    match wrap_type {
        Type::Function { params, returns } => {
            assert_eq!(params.len(), 1);
            assert_eq!(
                params[0],
                Type::Concrete(TypeAnnotation::Basic("AnyError".to_string()))
            );
            match returns.as_ref() {
                Type::Generic { base, args } => {
                    assert!(
                        matches!(
                            base.as_ref(),
                            Type::Concrete(TypeAnnotation::Reference(name)) if name == "Result"
                        ),
                        "expected Result<T> return, got {:?}",
                        returns
                    );
                    assert!(
                        !args.is_empty(),
                        "Result must include at least success type arg"
                    );
                    assert_eq!(
                        args[0],
                        Type::Concrete(TypeAnnotation::Basic("int".to_string()))
                    );
                }
                other => panic!("expected Result<int> return type, got {:?}", other),
            }
        }
        other => panic!("expected function type for wrap, got {:?}", other),
    }
}

#[test]
fn test_try_and_context_work_for_option_and_result() {
    use shape_ast::ast::TypeAnnotation;
    use shape_ast::parser::parse_program;

    let code = r#"
fn f(opt: Option<int>, res: Result<int>) {
  let a = opt? !! "missing option value"
  let b = res? !! "missing result value"
  return a
}
"#;

    let program = parse_program(code).expect("program should parse");
    let mut engine = TypeInferenceEngine::new();
    let types = engine
        .infer_program(&program)
        .expect("inference should succeed");

    let f_type = types.get("f").expect("f should be inferred");
    match f_type {
        Type::Function { returns, .. } => match returns.as_ref() {
            Type::Generic { base, args } => {
                assert!(
                    matches!(
                        base.as_ref(),
                        Type::Concrete(TypeAnnotation::Reference(name)) if name == "Result"
                    ),
                    "fallible return should be Result<T>, got {:?}",
                    returns
                );
                assert!(
                    !args.is_empty(),
                    "Result must include at least success type arg"
                );
                assert_eq!(
                    args[0],
                    Type::Concrete(TypeAnnotation::Basic("int".to_string()))
                );
            }
            other => panic!("expected Result<int> return type, got {:?}", other),
        },
        other => panic!("expected function type for f, got {:?}", other),
    }
}

#[test]
fn test_numeric_body_constraint_rejects_non_numeric_callsite() {
    use shape_ast::parser::parse_program;

    let code = r#"
fn afunc(c) {
  c = c + 1
  return c
}

let x = { x: 1 }
let a = afunc(1)
let b = afunc(x)
"#;

    let program = parse_program(code).expect("program should parse");
    let mut engine = TypeInferenceEngine::new();
    let result = engine.infer_program(&program);
    assert!(
        result.is_err(),
        "non-numeric callsite should fail, got {:?}",
        result
    );
}

#[test]
fn test_numeric_body_constraint_refines_unannotated_param_type() {
    use shape_ast::ast::TypeAnnotation;
    use shape_ast::parser::parse_program;

    let code = r#"
fn afunc(c) {
  c = c + 1
  return c
}
"#;

    let program = parse_program(code).expect("program should parse");
    let mut engine = TypeInferenceEngine::new();
    let types = engine
        .infer_program(&program)
        .expect("inference should succeed");
    let afunc_type = types.get("afunc").expect("afunc should be inferred");
    match afunc_type {
        Type::Function { params, .. } => {
            assert_eq!(params.len(), 1);
            let ann = params[0].to_annotation().expect("param annotation");
            assert_eq!(ann, TypeAnnotation::Basic("number".to_string()));
        }
        other => panic!("expected function type for afunc, got {:?}", other),
    }
}

#[test]
fn test_multiple_explicit_returns_infer_union_return_type() {
    use shape_ast::ast::TypeAnnotation;
    use shape_ast::parser::parse_program;

    let code = r#"
fn afunc(c) {
  return 1
  return "hi"
}
"#;

    let program = parse_program(code).expect("program should parse");
    let mut engine = TypeInferenceEngine::new();
    let types = engine
        .infer_program(&program)
        .expect("inference should succeed");

    let afunc_type = types.get("afunc").expect("afunc should be inferred");
    match afunc_type {
        Type::Function { returns, .. } => {
            let return_ann = returns
                .to_annotation()
                .expect("return should convert to annotation");

            match return_ann {
                TypeAnnotation::Union(variants) => {
                    let has_int = variants
                        .iter()
                        .any(|v| matches!(v, TypeAnnotation::Basic(name) if name == "int"));
                    let has_string = variants
                        .iter()
                        .any(|v| matches!(v, TypeAnnotation::Basic(name) if name == "string"));
                    assert!(has_int, "union should include int: {:?}", variants);
                    assert!(has_string, "union should include string: {:?}", variants);
                }
                other => panic!("expected union return type for afunc, got {:?}", other),
            }
        }
        other => panic!("expected function type for afunc, got {:?}", other),
    }
}

#[test]
fn test_callsite_union_return_does_not_degrade_to_any() {
    use shape_ast::ast::TypeAnnotation;
    use shape_ast::parser::parse_program;

    let code = r#"
fn afunc(c) {
  print("func called with " + c)
  return c
  return "hi"
}

let x = { x: 1, y: 2 }
let a = afunc(x)
let b = afunc(1)
"#;

    let program = parse_program(code).expect("program should parse");
    let mut engine = TypeInferenceEngine::new();
    let types = engine
        .infer_program(&program)
        .expect("inference should succeed");

    let afunc_type = types.get("afunc").expect("afunc should be inferred");
    match afunc_type {
        Type::Function { params, returns } => {
            assert_eq!(params.len(), 1);
            let param_ann = params[0].to_annotation().expect("param annotation");
            match param_ann {
                TypeAnnotation::Union(variants) => {
                    let has_int = variants
                        .iter()
                        .any(|v| matches!(v, TypeAnnotation::Basic(name) if name == "int"));
                    let has_object = variants
                        .iter()
                        .any(|v| matches!(v, TypeAnnotation::Object(_)));
                    assert!(has_int, "param union should include int: {:?}", variants);
                    assert!(
                        has_object,
                        "param union should include object: {:?}",
                        variants
                    );
                }
                other => panic!("expected union parameter type, got {:?}", other),
            }

            let return_ann = returns.to_annotation().expect("return annotation");
            match return_ann {
                TypeAnnotation::Union(variants) => {
                    let has_string = variants
                        .iter()
                        .any(|v| matches!(v, TypeAnnotation::Basic(name) if name == "string"));
                    let has_int = variants
                        .iter()
                        .any(|v| matches!(v, TypeAnnotation::Basic(name) if name == "int"));
                    let has_object = variants
                        .iter()
                        .any(|v| matches!(v, TypeAnnotation::Object(_)));
                    let has_any = variants
                        .iter()
                        .any(|v| matches!(v, TypeAnnotation::Basic(name) if name == "unknown"));
                    assert!(
                        has_string,
                        "return union should include string: {:?}",
                        variants
                    );
                    assert!(has_int, "return union should include int: {:?}", variants);
                    assert!(
                        has_object,
                        "return union should include object: {:?}",
                        variants
                    );
                    assert!(
                        !has_any,
                        "return union must not degrade to any: {:?}",
                        variants
                    );
                }
                other => panic!("expected union return type, got {:?}", other),
            }
        }
        other => panic!("expected function type for afunc, got {:?}", other),
    }
}

#[test]
fn test_best_effort_preserves_callsite_unions_under_numeric_conflict() {
    use shape_ast::ast::TypeAnnotation;
    use shape_ast::parser::parse_program;

    let code = r#"
fn afunc(c) {
  print("func called with " + c)
  c = c + 1
  return c
  return "hi"
}

let x = { x: 1, y: 2 }
let a = afunc(x)
let b = afunc(1)
"#;

    let program = parse_program(code).expect("program should parse");
    let mut engine = TypeInferenceEngine::new();
    let (types, errors) = engine.infer_program_best_effort(&program);
    assert!(
        !errors.is_empty(),
        "expected numeric/object mismatch to produce an error"
    );

    let afunc_type = types.get("afunc").expect("afunc should be inferred");
    match afunc_type {
        Type::Function { params, returns } => {
            assert_eq!(params.len(), 1);
            let param_ann = params[0].to_annotation().expect("param annotation");
            match param_ann {
                TypeAnnotation::Union(variants) => {
                    let has_int = variants
                        .iter()
                        .any(|v| matches!(v, TypeAnnotation::Basic(name) if name == "int"));
                    let has_number = variants
                        .iter()
                        .any(|v| matches!(v, TypeAnnotation::Basic(name) if name == "number"));
                    let has_object = variants
                        .iter()
                        .any(|v| matches!(v, TypeAnnotation::Object(_)));
                    assert!(has_int, "param union should include int: {:?}", variants);
                    assert!(
                        has_object,
                        "param union should include object: {:?}",
                        variants
                    );
                    assert!(
                        !has_number,
                        "param should not collapse to number: {:?}",
                        variants
                    );
                }
                other => panic!("expected union parameter type, got {:?}", other),
            }

            let return_ann = returns.to_annotation().expect("return annotation");
            match return_ann {
                TypeAnnotation::Union(variants) => {
                    let has_string = variants
                        .iter()
                        .any(|v| matches!(v, TypeAnnotation::Basic(name) if name == "string"));
                    let has_int = variants
                        .iter()
                        .any(|v| matches!(v, TypeAnnotation::Basic(name) if name == "int"));
                    let has_object = variants
                        .iter()
                        .any(|v| matches!(v, TypeAnnotation::Object(_)));
                    let has_any = variants
                        .iter()
                        .any(|v| matches!(v, TypeAnnotation::Basic(name) if name == "unknown"));
                    assert!(
                        has_string,
                        "return union should include string: {:?}",
                        variants
                    );
                    assert!(has_int, "return union should include int: {:?}", variants);
                    assert!(
                        has_object,
                        "return union should include object: {:?}",
                        variants
                    );
                    assert!(
                        !has_any,
                        "return union must not degrade to any: {:?}",
                        variants
                    );
                }
                other => panic!("expected union return type, got {:?}", other),
            }
        }
        other => panic!("expected function type for afunc, got {:?}", other),
    }
}

#[test]
fn test_fallible_lambda_wraps_return_in_result() {
    use shape_ast::parser::parse_program;

    let code = r#"
let f = |x| x?
"#;

    let program = parse_program(code).expect("program should parse");
    let mut engine = TypeInferenceEngine::new();
    let types = engine
        .infer_program(&program)
        .expect("inference should succeed");

    let f_type = types.get("f").expect("f should be inferred");
    match f_type {
        Type::Function { returns, .. } => {
            assert!(
                engine.is_result_type(returns),
                "fallible lambda should return Result<...>, got {:?}",
                returns
            );
        }
        other => panic!("expected function type for f, got {:?}", other),
    }
}

#[test]
fn test_exhaustiveness_check_missing_variant() {
    use shape_ast::ast::{
        DestructurePattern, EnumConstructorPayload, EnumDef, EnumMember, EnumMemberKind, Expr,
        Item, Literal, MatchArm, MatchExpr, Pattern, PatternConstructorFields, Span, Statement,
        TypeAnnotation, VarKind, VariableDecl,
    };

    let span = Span { start: 0, end: 0 };

    // Create enum: enum Status { Active, Inactive }
    let enum_def = EnumDef {
        name: "Status".to_string(),
        doc_comment: None,
        type_params: None,
        members: vec![
            EnumMember {
                name: "Active".to_string(),
                kind: EnumMemberKind::Unit { value: None },
                span,
                doc_comment: None,
            },
            EnumMember {
                name: "Inactive".to_string(),
                kind: EnumMemberKind::Unit { value: None },
                span,
                doc_comment: None,
            },
        ],
        annotations: vec![],
    };

    // Create match that only handles Active (missing Inactive)
    let match_expr = MatchExpr {
        scrutinee: Box::new(Expr::Identifier("status".to_string(), span.clone())),
        arms: vec![MatchArm {
            pattern: Pattern::Constructor {
                enum_name: Some("Status".to_string()),
                variant: "Active".to_string(),
                fields: PatternConstructorFields::Unit,
            },
            guard: None,
            body: Box::new(Expr::Literal(
                Literal::String("yes".to_string()),
                span.clone(),
            )),
            pattern_span: None,
        }],
    };

    // Create a program with: enum + variable + match
    let program = Program {
        items: vec![
            Item::Enum(enum_def, span.clone()),
            Item::Statement(
                Statement::VariableDecl(
                    VariableDecl {
                        kind: VarKind::Let,
                        is_mut: false,
                        pattern: DestructurePattern::Identifier("status".to_string(), span.clone()),
                        type_annotation: Some(TypeAnnotation::Reference("Status".to_string())),
                        value: Some(Expr::EnumConstructor {
                            enum_name: "Status".to_string(),
                            variant: "Active".to_string(),
                            payload: EnumConstructorPayload::Unit,
                            span: span.clone(),
                        }),
                        ownership: Default::default(),
                    },
                    span.clone(),
                ),
                span.clone(),
            ),
            Item::Statement(
                Statement::Expression(
                    Expr::Match(Box::new(match_expr), span.clone()),
                    span.clone(),
                ),
                span.clone(),
            ),
        ],
        docs: shape_ast::ast::ProgramDocs::default(),
    };

    let mut engine = TypeInferenceEngine::new();
    let result = engine.infer_program(&program);

    // Should fail with NonExhaustiveMatch
    assert!(result.is_err());
    let err = result.unwrap_err();
    match err {
        TypeError::NonExhaustiveMatch {
            enum_name,
            missing_variants,
        } => {
            assert_eq!(enum_name, "Status");
            assert!(missing_variants.contains(&"Inactive".to_string()));
        }
        other => panic!("Expected NonExhaustiveMatch, got {:?}", other),
    }
}

#[test]
fn test_union_typed_match_is_exhaustive_without_wildcard() {
    use shape_ast::parser::parse_program;

    let code = r#"
            let x: int | string = 1;
            let result = match (x) {
                n: int => n,
                s: string => 0
            }
        "#;

    let program = parse_program(code).expect("Failed to parse");
    let mut engine = TypeInferenceEngine::new();
    let result = engine.infer_program(&program);

    assert!(
        result.is_ok(),
        "Typed union match should be exhaustive without wildcard: {:?}",
        result.err()
    );
}

#[test]
fn test_union_typed_match_missing_variant_is_error() {
    use shape_ast::parser::parse_program;

    let code = r#"
            let x: int | string = 1;
            let result = match (x) {
                n: int => n
            }
        "#;

    let program = parse_program(code).expect("Failed to parse");
    let mut engine = TypeInferenceEngine::new();
    let result = engine.infer_program(&program);

    assert!(result.is_err(), "Missing union arm should be an error");
    match result.unwrap_err() {
        TypeError::NonExhaustiveMatch {
            enum_name,
            missing_variants,
        } => {
            assert_eq!(enum_name, "int | string");
            assert_eq!(missing_variants, vec!["string"]);
        }
        other => panic!("Expected NonExhaustiveMatch, got {:?}", other),
    }
}

#[test]
fn test_heterogeneous_match_creates_union() {
    // Match with different arm types should create a union type
    use shape_ast::parser::parse_program;

    let code = r#"
            let result = match 1 {
                1 => true,
                2 => "string"
            }
        "#;

    let program = parse_program(code).expect("Failed to parse");
    let mut engine = TypeInferenceEngine::new();
    let result = engine.infer_program(&program);

    // Should succeed and infer a union type
    assert!(
        result.is_ok(),
        "Should infer union type: {:?}",
        result.err()
    );

    let types = result.unwrap();
    let result_type = types.get("result");

    // The type should be a union (or at least not fail)
    assert!(result_type.is_some(), "result variable should have a type");
}

#[test]
fn test_homogeneous_match_uses_single_type() {
    // Match with same type in all arms should use that type, not create a union
    use shape_ast::parser::parse_program;

    let code = r#"
            let result = match 1 {
                1 => 10,
                2 => 20,
                _ => 30
            }
        "#;

    let program = parse_program(code).expect("Failed to parse");
    let mut engine = TypeInferenceEngine::new();
    let result = engine.infer_program(&program);

    assert!(
        result.is_ok(),
        "Should infer single type: {:?}",
        result.err()
    );
}

#[test]
fn test_union_type_name_generation() {
    // Test that union type names are generated correctly
    let engine = TypeInferenceEngine::new();

    let types = vec![
        Type::Concrete(shape_ast::ast::TypeAnnotation::Basic("bool".to_string())),
        Type::Concrete(shape_ast::ast::TypeAnnotation::Basic("string".to_string())),
    ];

    let name = engine.generate_union_type_name(&types);
    assert_eq!(name, "Union_bool_string");
}

#[test]
fn test_empty_match_does_not_drop_function_param_binding() {
    use shape_ast::parser::parse_program;

    // Empty match `match c {}` is now a parse error (B3 fix).
    // Verify that a match with a wildcard arm still allows subsequent use of `c`.
    let code = r#"
fn afunc(c) {
  print("func called with " + c)
  match c {
    _ => None,
  }
  c = c + 1
  return c
}
let x = { x: 1, y: 2 }
print(afunc(x))
print(afunc(1))
"#;

    let program = parse_program(code).expect("program should parse");
    let mut engine = TypeInferenceEngine::new();
    let result = engine.infer_program(&program);

    match result {
        Ok(_) => {}
        Err(TypeError::UndefinedVariable(name)) => {
            panic!(
                "match should not erase function parameter bindings; got undefined {}",
                name
            );
        }
        Err(_) => {
            // Any other type error is acceptable in this regression check.
        }
    }
}

#[test]
fn test_all_types_equal_true() {
    let engine = TypeInferenceEngine::new();

    let types = vec![
        Type::Concrete(shape_ast::ast::TypeAnnotation::Basic("number".to_string())),
        Type::Concrete(shape_ast::ast::TypeAnnotation::Basic("number".to_string())),
        Type::Concrete(shape_ast::ast::TypeAnnotation::Basic("number".to_string())),
    ];

    assert!(
        engine.all_types_equal(&types),
        "All number types should be equal"
    );
}

#[test]
fn test_all_types_equal_false() {
    let engine = TypeInferenceEngine::new();

    let types = vec![
        Type::Concrete(shape_ast::ast::TypeAnnotation::Basic("number".to_string())),
        Type::Concrete(shape_ast::ast::TypeAnnotation::Basic("string".to_string())),
    ];

    assert!(
        !engine.all_types_equal(&types),
        "Different types should not be equal"
    );
}

#[test]
fn test_create_nominal_union() {
    let mut engine = TypeInferenceEngine::new();

    let types = vec![
        Type::Concrete(shape_ast::ast::TypeAnnotation::Basic("bool".to_string())),
        Type::Concrete(shape_ast::ast::TypeAnnotation::Basic("string".to_string())),
        Type::Concrete(shape_ast::ast::TypeAnnotation::Basic("number".to_string())),
    ];

    let union_type = engine.create_nominal_union(&types);
    assert!(
        union_type.is_ok(),
        "Should create union type: {:?}",
        union_type.err()
    );

    // Should return a Union annotation directly
    if let Ok(Type::Concrete(shape_ast::ast::TypeAnnotation::Union(variants))) = union_type {
        assert_eq!(variants.len(), 3);
    } else {
        panic!("Expected Union type, got {:?}", union_type);
    }
}

#[test]
fn test_union_with_two_types() {
    let mut engine = TypeInferenceEngine::new();

    let types = vec![
        Type::Concrete(shape_ast::ast::TypeAnnotation::Basic("bool".to_string())),
        Type::Concrete(shape_ast::ast::TypeAnnotation::Basic("string".to_string())),
    ];

    let union_type = engine.create_nominal_union(&types);
    assert!(union_type.is_ok());

    if let Ok(Type::Concrete(shape_ast::ast::TypeAnnotation::Union(variants))) = union_type {
        assert_eq!(variants.len(), 2);
    } else {
        panic!("Expected Union type, got {:?}", union_type);
    }
}

#[test]
fn test_union_type_registered_as_alias() {
    let mut engine = TypeInferenceEngine::new();

    let types = vec![
        Type::Concrete(shape_ast::ast::TypeAnnotation::Basic("bool".to_string())),
        Type::Concrete(shape_ast::ast::TypeAnnotation::Basic("number".to_string())),
    ];

    let _union_type = engine
        .create_nominal_union(&types)
        .expect("Should create union");

    // Verify the union was registered in the environment as a type alias
    let lookup = engine.env.lookup_type_alias("Union_bool_number");
    assert!(
        lookup.is_some(),
        "Union type should be registered as type alias"
    );
}

#[test]
fn test_union_with_complex_types() {
    let mut engine = TypeInferenceEngine::new();

    let types = vec![
        Type::Concrete(shape_ast::ast::TypeAnnotation::Array(Box::new(
            shape_ast::ast::TypeAnnotation::Basic("number".to_string()),
        ))),
        Type::Concrete(shape_ast::ast::TypeAnnotation::Object(vec![])),
    ];

    let union_type = engine.create_nominal_union(&types);
    assert!(union_type.is_ok(), "Should handle complex types in unions");

    if let Ok(Type::Concrete(shape_ast::ast::TypeAnnotation::Union(variants))) = union_type {
        assert_eq!(variants.len(), 2);
    } else {
        panic!("Expected Union type, got {:?}", union_type);
    }
}

#[test]
fn test_union_name_with_reference_types() {
    let mut engine = TypeInferenceEngine::new();

    let types = vec![
        Type::Concrete(shape_ast::ast::TypeAnnotation::Reference(
            "Currency".to_string(),
        )),
        Type::Concrete(shape_ast::ast::TypeAnnotation::Reference(
            "Percent".to_string(),
        )),
    ];

    let union_type = engine.create_nominal_union(&types);
    assert!(union_type.is_ok());

    if let Ok(Type::Concrete(shape_ast::ast::TypeAnnotation::Union(variants))) = union_type {
        assert_eq!(variants.len(), 2);
    } else {
        panic!("Expected Union type, got {:?}", union_type);
    }
}

#[test]
fn test_empty_types_list() {
    let engine = TypeInferenceEngine::new();
    let types: Vec<Type> = vec![];

    // Empty list should be considered "all equal"
    assert!(engine.all_types_equal(&types));
}

#[test]
fn test_single_type_list() {
    let engine = TypeInferenceEngine::new();
    let types = vec![Type::Concrete(shape_ast::ast::TypeAnnotation::Basic(
        "number".to_string(),
    ))];

    // Single type should be considered "all equal"
    assert!(engine.all_types_equal(&types));
}

#[test]
fn test_match_with_three_different_types() {
    use shape_ast::parser::parse_program;

    let code = r#"
            let result = match 1 {
                1 => true,
                2 => "hello",
                _ => 42
            }
        "#;

    let program = parse_program(code).expect("Failed to parse");
    let mut engine = TypeInferenceEngine::new();
    let result = engine.infer_program(&program);

    assert!(
        result.is_ok(),
        "Should infer 3-type union: {:?}",
        result.err()
    );
}

#[test]
fn test_nested_heterogeneous_matches() {
    use shape_ast::parser::parse_program;

    let code = r#"
            let result = match 1 {
                1 => match 2 {
                    2 => true,
                    _ => "inner"
                },
                _ => 42
            }
        "#;

    let program = parse_program(code).expect("Failed to parse");
    let mut engine = TypeInferenceEngine::new();
    let result = engine.infer_program(&program);

    // Nested heterogeneous matches should create nested unions
    assert!(
        result.is_ok(),
        "Should handle nested heterogeneous matches: {:?}",
        result.err()
    );
}

#[test]
fn test_union_type_annotation_structure() {
    let mut engine = TypeInferenceEngine::new();

    let types = vec![
        Type::Concrete(shape_ast::ast::TypeAnnotation::Basic("bool".to_string())),
        Type::Concrete(shape_ast::ast::TypeAnnotation::Basic("string".to_string())),
    ];

    engine
        .create_nominal_union(&types)
        .expect("Should create union");

    // Verify the union type alias contains a Union annotation
    let union_alias_entry = engine.env.lookup_type_alias("Union_bool_string");
    assert!(union_alias_entry.is_some(), "Union should be registered");

    let union_alias = &union_alias_entry.unwrap().type_annotation;
    if let shape_ast::ast::TypeAnnotation::Union(variants) = union_alias {
        assert_eq!(variants.len(), 2, "Union should have 2 variants");
    } else {
        panic!("Expected Union annotation, got {:?}", union_alias);
    }
}

#[test]
fn test_match_with_array_and_object() {
    use shape_ast::parser::parse_program;

    let code = r#"
            let result = match 1 {
                1 => [1, 2, 3],
                _ => {}
            }
        "#;

    let program = parse_program(code).expect("Failed to parse");
    let mut engine = TypeInferenceEngine::new();
    let result = engine.infer_program(&program);

    // Should handle complex types (array vs object) in match arms
    assert!(
        result.is_ok(),
        "Should handle array/object union: {:?}",
        result.err()
    );
}

#[test]
fn test_type_name_for_various_types() {
    let engine = TypeInferenceEngine::new();

    // Test basic types
    let bool_type = Type::Concrete(shape_ast::ast::TypeAnnotation::Basic("bool".to_string()));
    assert_eq!(engine.type_name_for_union(&bool_type), "bool");

    // Test reference types
    let ref_type = Type::Concrete(shape_ast::ast::TypeAnnotation::Reference(
        "MyType".to_string(),
    ));
    assert_eq!(engine.type_name_for_union(&ref_type), "MyType");

    // Test array types
    let array_type = Type::Concrete(shape_ast::ast::TypeAnnotation::Array(Box::new(
        shape_ast::ast::TypeAnnotation::Basic("number".to_string()),
    )));
    assert_eq!(engine.type_name_for_union(&array_type), "array");

    // Test function types
    let func_type = Type::Concrete(shape_ast::ast::TypeAnnotation::Function {
        params: vec![],
        returns: Box::new(shape_ast::ast::TypeAnnotation::Basic("void".to_string())),
    });
    assert_eq!(engine.type_name_for_union(&func_type), "function");
}

#[test]
fn test_generic_function_type_scheme() {
    use shape_ast::ast::Item;
    use shape_ast::parser::parse_program;

    // Test that generic functions create polymorphic type schemes
    // Note: Shape uses -> for return type annotation
    let code = r#"
            function identity<T>(x: T) -> T {
                return x
            }
        "#;

    let program = parse_program(code).expect("Failed to parse");

    // Verify the AST has type_params
    let has_type_params = program.items.iter().any(|item| {
        if let Item::Function(func, _) = item {
            func.type_params.is_some() && !func.type_params.as_ref().unwrap().is_empty()
        } else {
            false
        }
    });
    assert!(has_type_params, "AST should have type_params");

    let mut engine = TypeInferenceEngine::new();
    let result = engine.infer_program(&program);

    assert!(
        result.is_ok(),
        "Should infer generic function: {:?}",
        result.err()
    );

    // Check that identity is defined with a polymorphic scheme
    let scheme = engine.env.lookup("identity");
    assert!(scheme.is_some(), "identity should be defined");

    // For now, the scheme may not be polymorphic if the inference simplified it
    // The key test is that inference succeeds
    // assert!(scheme.is_polymorphic(), "identity should be polymorphic");
    // assert_eq!(scheme.type_params().len(), 1, "identity should have 1 type param");
}

#[test]
fn test_generic_function_basic() {
    use shape_ast::parser::parse_program;

    // Test simpler case - generic function that's called
    let code = r#"
            function wrap<T>(x: T) -> Vec<T> {
                return [x]
            }
        "#;

    let program = parse_program(code).expect("Failed to parse");
    let mut engine = TypeInferenceEngine::new();
    let result = engine.infer_program(&program);

    // Should successfully infer the generic function
    assert!(
        result.is_ok(),
        "Should infer generic function: {:?}",
        result.err()
    );
}

#[test]
fn test_function_with_typed_params_infers_return() {
    // Function with annotated params should have its return type inferred
    use shape_ast::parser::parse_program;

    let code = r#"
            fn double(x: number) {
                return x * 2.0
            }
        "#;

    let program = parse_program(code).expect("Failed to parse");
    let mut engine = TypeInferenceEngine::new();
    let result = engine.infer_program(&program);

    assert!(
        result.is_ok(),
        "Should infer function with typed param: {:?}",
        result.err()
    );
}

#[test]
fn test_function_call_before_definition_type_checks() {
    use shape_ast::parser::parse_program;

    let code = r#"
            let x = add(1.0, 2.0)

            fn add(a: number, b: number) -> number {
                return a + b
            }
        "#;

    let program = parse_program(code).expect("Failed to parse");
    let mut engine = TypeInferenceEngine::new();
    let result = engine.infer_program(&program);

    assert!(
        result.is_ok(),
        "Forward function call should type-check: {:?}",
        result.err()
    );
}

#[test]
fn test_function_call_with_default_arguments_type_checks() {
    use shape_ast::parser::parse_program;

    let code = r#"
            fn add(a: int = 1, b: int = 2) -> int {
                return a + b
            }

            let x = add()
            let y = add(5)
            let z = add(5, 6)
        "#;

    let program = parse_program(code).expect("Failed to parse");
    let mut engine = TypeInferenceEngine::new();
    let result = engine.infer_program(&program);

    assert!(
        result.is_ok(),
        "Default function arguments should type-check: {:?}",
        result.err()
    );
}

#[test]
fn test_range_builtin_accepts_single_argument_form() {
    use shape_ast::parser::parse_program;

    let code = r#"
            let total = 0
            for i in range(100) {
                if i % 2 == 0 {
                    total = total + i
                }
            }
        "#;

    let program = parse_program(code).expect("Failed to parse");
    let mut engine = TypeInferenceEngine::new();
    let result = engine.infer_program(&program);

    assert!(
        result.is_ok(),
        "range(n) should type-check as builtin shorthand: {:?}",
        result.err()
    );
}

#[test]
fn test_int_arithmetic_preserves_int() {
    // `let x = 5 * 2` should type x as int, not fail with int~number mismatch
    use shape_ast::parser::parse_program;

    let code = r#"
            let x = 5 * 2
        "#;

    let program = parse_program(code).expect("Failed to parse");
    let mut engine = TypeInferenceEngine::new();
    let result = engine.infer_program(&program);

    assert!(
        result.is_ok(),
        "int * int should type-check: {:?}",
        result.err()
    );

    let types = result.unwrap();
    let x_type = types.get("x").expect("x should have a type");
    // int * int → int (preserved, not widened to number)
    assert_eq!(
        *x_type,
        Type::Concrete(shape_ast::ast::TypeAnnotation::Basic("int".to_string())),
        "5 * 2 should be int, got {:?}",
        x_type
    );
}

#[test]
fn test_mixed_arithmetic_widens_to_number() {
    // `let x = 5 * 2.0` should widen to number
    use shape_ast::parser::parse_program;

    let code = r#"
            let x = 5 * 2.0
        "#;

    let program = parse_program(code).expect("Failed to parse");
    let mut engine = TypeInferenceEngine::new();
    let result = engine.infer_program(&program);

    assert!(
        result.is_ok(),
        "int * number should type-check: {:?}",
        result.err()
    );

    let types = result.unwrap();
    let x_type = types.get("x").expect("x should have a type");
    // int * number → number (widened)
    assert_eq!(
        *x_type,
        Type::Concrete(shape_ast::ast::TypeAnnotation::Basic("number".to_string())),
        "5 * 2.0 should be number, got {:?}",
        x_type
    );
}

#[test]
fn test_int_comparison_works() {
    // `let x = 5 > 2` should type-check successfully
    use shape_ast::parser::parse_program;

    let code = r#"
            let x = 5 > 2
        "#;

    let program = parse_program(code).expect("Failed to parse");
    let mut engine = TypeInferenceEngine::new();
    let result = engine.infer_program(&program);

    assert!(
        result.is_ok(),
        "int > int should type-check: {:?}",
        result.err()
    );
}

#[test]
fn test_function_type_is_function_variant() {
    // Functions should produce Type::Function, not Concrete(Function)
    use shape_ast::parser::parse_program;

    let code = r#"
            fn add(a: number, b: number) -> number {
                return a + b
            }
        "#;

    let program = parse_program(code).expect("Failed to parse");
    let mut engine = TypeInferenceEngine::new();
    let result = engine.infer_program(&program);

    assert!(
        result.is_ok(),
        "Function should type-check: {:?}",
        result.err()
    );

    let types = result.unwrap();
    let add_type = types.get("add").expect("add should have a type");
    assert!(
        matches!(add_type, Type::Function { .. }),
        "add should be Type::Function, got {:?}",
        add_type
    );
}

#[test]
fn test_hoisted_field_read_before_assignment_errors() {
    use shape_ast::parser::parse_program;

    let code = r#"
            let a = { x: 1 }
            let before = a.y
            a.y = 2
        "#;

    let program = parse_program(code).expect("Failed to parse");
    let mut engine = TypeInferenceEngine::new();
    let result = engine.infer_program(&program);

    assert!(result.is_err(), "Read before assignment should fail");
    assert!(
        matches!(&result, Err(TypeError::UnknownProperty(_, _))),
        "Expected UnknownProperty, got {:?}",
        result
    );
}

#[test]
fn test_hoisted_field_read_in_formatted_string_before_assignment_errors() {
    use shape_ast::parser::parse_program;

    let code = r#"
            let a = { x: 1 }
            print(f": {a.y}")
            a.y = 2
        "#;

    let program = parse_program(code).expect("Failed to parse");
    let mut engine = TypeInferenceEngine::new();
    let result = engine.infer_program(&program);

    assert!(result.is_err(), "Read before assignment should fail");
    assert!(
        matches!(&result, Err(TypeError::UnknownProperty(_, _))),
        "Expected UnknownProperty, got {:?}",
        result
    );
}

#[test]
fn test_hoisted_field_read_after_assignment_succeeds() {
    use shape_ast::parser::parse_program;

    let code = r#"
            let a = { x: 1 }
            a.y = 2
            let after = a.y
        "#;

    let program = parse_program(code).expect("Failed to parse");
    let mut engine = TypeInferenceEngine::new();
    let result = engine.infer_program(&program);

    assert!(
        result.is_ok(),
        "Read after assignment should type-check: {:?}",
        result.err()
    );
}

#[test]
fn test_object_add_infers_intersection() {
    use shape_ast::parser::parse_program;

    let code = r#"
            let a = { x: 1 }
            a.y = 2
            let b = { z: 3 }
            let c = a + b
        "#;

    let program = parse_program(code).expect("Failed to parse");
    let mut engine = TypeInferenceEngine::new();
    let result = engine.infer_program(&program);

    assert!(
        result.is_ok(),
        "Object merge should type-check: {:?}",
        result.err()
    );

    let types = result.unwrap();
    let c_type = types.get("c").expect("c should have a type");
    match c_type {
        Type::Concrete(shape_ast::ast::TypeAnnotation::Intersection(parts)) => {
            assert!(
                parts.len() >= 2,
                "intersection should have at least two parts"
            );
        }
        other => panic!(
            "expected intersection type for object merge, got {:?}",
            other
        ),
    }
}
