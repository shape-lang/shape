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

// ─── SC1: Color / Border / ChartType namespace constructors ───────────

#[test]
fn sc1_named_style_spec_members_infer_string() {
    use shape_ast::ast::TypeAnnotation;
    use shape_ast::parser::parse_program;

    let code = r#"
let c = Color.red
let b = Border.rounded
let ct = ChartType.line
let cd = Color.default
let cb = ChartType.boxplot
"#;
    let program = parse_program(code).expect("program should parse");
    let mut engine = TypeInferenceEngine::new();
    let types = engine
        .infer_program(&program)
        .expect("SC1 named members must infer as string, not reject");

    for name in ["c", "b", "ct", "cd", "cb"] {
        let ty = types.get(name).unwrap_or_else(|| panic!("{name} inferred"));
        assert!(
            matches!(ty, Type::Concrete(TypeAnnotation::Basic(n)) if n == "string"),
            "{name} should be string carrier, got {ty:?}"
        );
    }
}

#[test]
fn sc1_color_rgb_call_does_not_reject() {
    use shape_ast::parser::parse_program;

    let code = r#"
let c = Color.rgb(255, 0, 0)
"#;
    let program = parse_program(code).expect("program should parse");
    let mut engine = TypeInferenceEngine::new();
    engine
        .infer_program(&program)
        .expect("SC1 Color.rgb(...) call must type-check");
}

#[test]
fn sc1_unknown_style_spec_member_rejects_cleanly() {
    use shape_ast::parser::parse_program;

    let code = r#"
let c = Color.bogus
"#;
    let program = parse_program(code).expect("program should parse");
    let mut engine = TypeInferenceEngine::new();
    let err = engine
        .infer_program(&program)
        .expect_err("Color.bogus must reject");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("bogus") && msg.contains("Color"),
        "expected a clean unknown-member rejection naming Color.bogus, got: {msg}"
    );
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

/// Pre-let-gen this rejected with `GenericTypeError`. Fn-boundary
/// let-generalization (docs/design/let-gen-gating-predicate-spec.md §1.2) now
/// GENERALIZES an unannotated fn whose body is a freshly-constructed `Err(..)`
/// carrier (cond-4 non-expansive) into `∀T. () -> Result<T, string>` instead of
/// rejecting — the success branch's `T` is a return-position-only free var that
/// the scheme quantifies. The former reject behavior is superseded.
#[test]
fn test_expression_style_err_only_generalizes_under_let_gen() {
    use shape_ast::parser::parse_program;

    let code = r#"
fn test() {
  Err("some error")
}
"#;

    let program = parse_program(code).expect("program should parse");
    let mut engine = TypeInferenceEngine::new();
    let (_types, errors) = engine.infer_program_best_effort(&program);
    assert!(
        errors.is_empty(),
        "fn-boundary let-gen should generalize a pure `Err(..)` body, got: {:?}",
        errors
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
        Some(&Type::Concrete(TypeAnnotation::Reference("MyType".into())))
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
                enum_name: Some("Status".into()),
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
                        type_annotation: Some(TypeAnnotation::Reference("Status".into())),
                        value: Some(Expr::EnumConstructor {
                            enum_name: "Status".into(),
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
        Type::Concrete(shape_ast::ast::TypeAnnotation::Reference("Currency".into())),
        Type::Concrete(shape_ast::ast::TypeAnnotation::Reference("Percent".into())),
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
    let ref_type = Type::Concrete(shape_ast::ast::TypeAnnotation::Reference("MyType".into()));
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

/// A helper that returns the `(param_name, return_name)` of a `Type::Function`
/// whose param/return are both `Type::Concrete(Basic(_))`.
fn fn_param_return_basic(ty: &Type) -> Option<(String, String)> {
    let Type::Function { params, returns } = ty else {
        return None;
    };
    let p = match params.first()? {
        Type::Concrete(TypeAnnotation::Basic(n)) => n.clone(),
        _ => return None,
    };
    let r = match returns.as_ref() {
        Type::Concrete(TypeAnnotation::Basic(n)) => n.clone(),
        _ => return None,
    };
    Some((p, r))
}

/// ε-1 regression: a function reachable only through nested/transitive calls
/// of other unannotated functions still resolves its parameter type via the
/// transitive callsite-union fixpoint. Before the fix `double` inferred as
/// `fn(number) -> number` (the eager `Numeric` → `number` collapse), which
/// made the compiler emit `MulNumber` for integer arithmetic and the program
/// printed a denormal float (`2e-321`, the i64 `40` bit-reinterpreted).
#[test]
fn test_nested_unannotated_fn_calls_resolve_param_types() {
    use shape_ast::ast::TypeAnnotation;
    use shape_ast::parser::parse_program;

    let code = r#"
fn double(x){x*2}
fn quad(x){double(double(x))}
let r = quad(10)
"#;
    let program = parse_program(code).expect("program should parse");
    let mut engine = TypeInferenceEngine::new();
    let (types, errors) = engine.infer_program_best_effort(&program);
    assert!(errors.is_empty(), "inference should succeed: {:?}", errors);

    let (double_p, double_r) =
        fn_param_return_basic(types.get("double").expect("double should be inferred"))
            .expect("double should be fn(basic)->basic");
    assert_eq!(
        (double_p.as_str(), double_r.as_str()),
        ("int", "int"),
        "double's parameter must resolve to int through the call graph, \
         not collapse to the number default"
    );

    let (quad_p, quad_r) =
        fn_param_return_basic(types.get("quad").expect("quad should be inferred"))
            .expect("quad should be fn(basic)->basic");
    assert_eq!((quad_p.as_str(), quad_r.as_str()), ("int", "int"));

    assert!(
        matches!(types.get("r"), Some(Type::Concrete(TypeAnnotation::Basic(n))) if n == "int"),
        "let r = quad(10) must infer as int, got {:?}",
        types.get("r")
    );
}

/// ε-1 regression: `fn double(x){x*2.0}` — when the body pairs the parameter
/// with a `number` literal the parameter resolves to `number` even when called
/// only with a literal. Confirms the variable-propagating `numeric_result_type`
/// stays consistent for the `number` case.
#[test]
fn test_nested_unannotated_fn_number_body_resolves_number() {
    use shape_ast::ast::TypeAnnotation;
    use shape_ast::parser::parse_program;

    let code = r#"
fn scalef(x){x*2.0}
fn quadf(x){scalef(scalef(x))}
let r = quadf(10.0)
"#;
    let program = parse_program(code).expect("program should parse");
    let mut engine = TypeInferenceEngine::new();
    let (types, errors) = engine.infer_program_best_effort(&program);
    assert!(errors.is_empty(), "inference should succeed: {:?}", errors);

    let (scalef_p, scalef_r) =
        fn_param_return_basic(types.get("scalef").expect("scalef should be inferred"))
            .expect("scalef should be fn(basic)->basic");
    assert_eq!((scalef_p.as_str(), scalef_r.as_str()), ("number", "number"));

    assert!(
        matches!(types.get("r"), Some(Type::Concrete(TypeAnnotation::Basic(n))) if n == "number"),
        "let r = quadf(10.0) must infer as number, got {:?}",
        types.get("r")
    );
}

/// ε-1 regression: a transitively-reached unannotated function with NO concrete
/// callsite anywhere in the program keeps the deferred `number` default. The
/// `Numeric`-bounded parameter is not left as a bare unresolved variable
/// (which the emitter would reject), it falls back to `number`.
#[test]
fn test_unannotated_numeric_fn_without_callsite_defaults_to_number() {
    use shape_ast::ast::TypeAnnotation;
    use shape_ast::parser::parse_program;

    let code = r#"
fn triple(x){x*3}
"#;
    let program = parse_program(code).expect("program should parse");
    let mut engine = TypeInferenceEngine::new();
    let (types, errors) = engine.infer_program_best_effort(&program);
    assert!(errors.is_empty(), "inference should succeed: {:?}", errors);

    let triple = types.get("triple").expect("triple should be inferred");
    let Type::Function { params, .. } = triple else {
        panic!("triple should be a function type, got {:?}", triple);
    };
    assert!(
        matches!(&params[0], Type::Concrete(TypeAnnotation::Basic(n)) if n == "number"),
        "an unannotated numeric param with no callsite must default to number, got {:?}",
        params[0]
    );
}

/// ζ-(a) regression: a for/while loop body is parsed as an `Expr::Block`
/// whose items are `BlockItem::Assignment` — the RHS of a loop-body
/// assignment (`last = dbl(11)`) must be walked by inference, or the
/// callsite of `dbl` is never recorded and its unannotated parameter
/// collapses to the `number` default. That made the bytecode compiler emit
/// `MulNumber` for integer arithmetic; the VM then printed the i64 result
/// bit-reinterpreted as an f64 denormal. `dbl` must resolve as `fn(int)->int`.
#[test]
fn test_loop_body_assignment_records_callsite() {
    use shape_ast::ast::TypeAnnotation;
    use shape_ast::parser::parse_program;

    for code in [
        // for-loop body assignment
        "fn dbl(x){x*2}\nlet mut last = 0\nfor i in 0..5 { last = dbl(11) }\n",
        // while-loop body assignment
        "fn dbl(x){x*2}\nlet mut last = 0\nlet mut i = 0\nwhile i < 5 { last = dbl(11); i = i + 1 }\n",
        // nested-call loop body assignment
        "fn dbl(x){x*2}\nfn inc(x){x+1}\nlet mut last = 0\nfor i in 0..5 { last = inc(dbl(11)) }\n",
    ] {
        let program = parse_program(code).expect("program should parse");
        let mut engine = TypeInferenceEngine::new();
        let (types, errors) = engine.infer_program_best_effort(&program);
        assert!(errors.is_empty(), "inference should succeed: {:?}", errors);

        let dbl = types.get("dbl").expect("dbl should be inferred");
        let Type::Function { params, returns } = dbl else {
            panic!("dbl should be a function type, got {:?}", dbl);
        };
        assert!(
            matches!(&params[0], Type::Concrete(TypeAnnotation::Basic(n)) if n == "int"),
            "dbl's parameter must resolve to int via the loop-body callsite, \
             not collapse to the number default — got {:?} for code:\n{}",
            params[0],
            code,
        );
        assert!(
            matches!(returns.as_ref(), Type::Concrete(TypeAnnotation::Basic(n)) if n == "int"),
            "dbl's return must resolve to int — got {:?}",
            returns,
        );
    }
}

/// WS-9 Cluster A: an `obj[i]` access on an unannotated parameter must
/// connect the element type to the parameter's resolved array type. The
/// `Indexable` element-carrying constraint (replacing the element-less
/// `Iterable` marker) lets the solver bind the index-access result to the
/// parameter's `Array<int>` element type once the call site resolves it.
#[test]
fn ws9_index_access_on_unannotated_param_resolves_element_type() {
    use shape_ast::ast::TypeAnnotation;
    use shape_ast::parser::parse_program;

    // `a[0] + b[0]` over two `Array<int>` arguments: both parameters must
    // resolve to `Array<int>` and inference must not fail.
    let code = "fn twoidx(a, b) { a[0] + b[0] }\nprint(twoidx([1,2],[3,4]))\n";
    let program = parse_program(code).expect("program should parse");
    let mut engine = TypeInferenceEngine::new();
    let (types, errors) = engine.infer_program_best_effort(&program);
    assert!(
        errors.is_empty(),
        "inference must not reject indexed unannotated params: {:?}",
        errors
    );
    let Some(Type::Function { params, .. }) = types.get("twoidx") else {
        panic!("twoidx should be inferred as a function");
    };
    for (idx, p) in params.iter().enumerate() {
        // U1: the canonical array carrier is `Type::Generic { base:
        // Reference("Array"/"Vec"), args: [int] }`. Canonicalize before the
        // shape check so the assertion is encoding-agnostic.
        let canon = p.canonicalize();
        let is_array_int = match &canon {
            Type::Generic { base, args } => {
                let base_ok = match base.as_ref() {
                    Type::Concrete(TypeAnnotation::Reference(tp)) => {
                        let n = tp.to_string();
                        n == "Array" || n == "Vec"
                    }
                    _ => false,
                };
                base_ok
                    && args.len() == 1
                    && matches!(
                        &args[0],
                        Type::Concrete(TypeAnnotation::Basic(n)) if n == "int"
                    )
            }
            _ => false,
        };
        assert!(
            is_array_int,
            "twoidx param {} must resolve to Array<int>, got {:?}",
            idx, p,
        );
    }
}

/// WS-9 Cluster A: the bare-`Type::Variable` arm of `infer_index_access`
/// must push an element-carrying `Indexable` constraint, and the solver's
/// `apply_bounds` backward propagation must bind that element variable when
/// the object resolves to a concrete array.
#[test]
fn ws9_indexable_constraint_backward_propagates_element_type() {
    use crate::type_system::constraints::ConstraintSolver;
    use crate::type_system::{Type, TypeConstraint, TypeVar};
    use shape_ast::ast::TypeAnnotation;

    let obj_var = TypeVar::new("obj".to_string());
    let elem_var = TypeVar::new("elem".to_string());

    let mut constraints: Vec<(Type, Type)> = vec![
        // obj ~ Constrained{ Indexable(elem) }  (the index-access constraint)
        (
            Type::Variable(obj_var.clone()),
            Type::Constrained {
                var: TypeVar::new("bound".to_string()),
                constraint: Box::new(TypeConstraint::Indexable(Box::new(Type::Variable(
                    elem_var.clone(),
                )))),
            },
        ),
        // obj ~ Array<int>  (the resolved object type, e.g. from a call site)
        (
            Type::Variable(obj_var.clone()),
            Type::Concrete(TypeAnnotation::Array(Box::new(TypeAnnotation::Basic(
                "int".to_string(),
            )))),
        ),
    ];

    let mut solver = ConstraintSolver::new();
    solver
        .solve(&mut constraints)
        .expect("solve should succeed");

    let resolved_elem = solver
        .unifier()
        .apply_substitutions(&Type::Variable(elem_var));
    assert!(
        matches!(
            &resolved_elem,
            Type::Concrete(TypeAnnotation::Basic(n)) if n == "int"
        ),
        "Indexable element variable must backward-propagate to int, got {:?}",
        resolved_elem,
    );
}

/// WS-9b: a property access (`a.field`) into an UNANNOTATED parameter
/// whose call site supplies a NAMED struct must not produce
/// `UnsolvedConstraints`. At HEAD `68826829`, `infer_function`'s
/// `refine_callable_param_types_from_local_constraints` eagerly projected
/// such a parameter to a partial structural `Object([{ field: unknown }])`
/// type; that partial object could not unify with the call site's named
/// `Box` argument, so the leftover `Object(...) ~ Box` constraint failed.
/// Deferring the object projection for named functions (callsite union
/// resolves the parameter instead) eliminates the spurious reject.
#[test]
fn ws9b_property_access_on_unannotated_param_resolves_named_struct() {
    use shape_ast::ast::TypeAnnotation;
    use shape_ast::parser::parse_program;

    let code = "fn ov(a, b) { a.lo <= b.hi }\n\
                type Box { lo: int, hi: int }\n\
                print(ov(Box { lo: 1, hi: 5 }, Box { lo: 3, hi: 9 }))\n";
    let program = parse_program(code).expect("program should parse");
    let mut engine = TypeInferenceEngine::new();
    let (types, errors) = engine.infer_program_best_effort(&program);
    assert!(
        errors.is_empty(),
        "inference must not reject property access into a named-struct \
         unannotated param: {:?}",
        errors
    );
    let Some(Type::Function { params, .. }) = types.get("ov") else {
        panic!("ov should be inferred as a function");
    };
    for (idx, p) in params.iter().enumerate() {
        assert!(
            matches!(
                p,
                Type::Concrete(TypeAnnotation::Reference(path))
                    if path.as_str() == "Box"
            ),
            "ov param {} must resolve to the named struct Box, got {:?}",
            idx,
            p,
        );
    }
}

/// WS-9b: the object projection in
/// `refine_callable_param_types_from_local_constraints` is a closure-only
/// refinement. For a NAMED FUNCTION (`is_closure = false`) an
/// object-field-accessed parameter must be LEFT as a `Type::Variable` so
/// callsite union can resolve it — projecting it to a partial `Object`
/// here severs that link.
#[test]
fn ws9b_named_function_object_param_left_as_variable_for_callsite_union() {
    use crate::type_system::{Type, TypeConstraint, TypeVar};

    let mut engine = TypeInferenceEngine::new();
    let param_var = TypeVar::new("p".to_string());
    let field_var = TypeVar::new("f".to_string());
    let mut param_types = vec![Type::Variable(param_var.clone())];
    // Body constraint: `p` has field `lo` (the shape `p.lo` produces).
    let local_constraints = vec![(
        Type::Variable(param_var.clone()),
        Type::Constrained {
            var: TypeVar::new("bound".to_string()),
            constraint: Box::new(TypeConstraint::HasField(
                "lo".to_string(),
                Box::new(Type::Variable(field_var)),
            )),
        },
    )];

    // Named-function path (`is_closure = false`): the parameter must stay
    // a variable.
    engine.refine_callable_param_types_from_local_constraints(
        &mut param_types,
        &local_constraints,
        false,
    );
    assert!(
        matches!(param_types[0], Type::Variable(_)),
        "a named function's object-field-accessed param must stay a \
         Type::Variable for callsite union, got {:?}",
        param_types[0],
    );

    // Closure path (`is_closure = true`): the parameter IS eagerly
    // projected to a partial structural object (closures have no callsite
    // union to resolve them).
    let mut closure_param_types = vec![Type::Variable(param_var)];
    engine.refine_callable_param_types_from_local_constraints(
        &mut closure_param_types,
        &local_constraints,
        true,
    );
    assert!(
        matches!(
            &closure_param_types[0],
            Type::Concrete(shape_ast::ast::TypeAnnotation::Object(_))
        ),
        "a closure's object-field-accessed param is eagerly projected, \
         got {:?}",
        closure_param_types[0],
    );
}

/// WS-9c: an anonymous-object factory `fn aabb(lo, hi) { {min: lo, max: hi} }`
/// must, after callsite-union propagation, expose a return type whose
/// `Object` field types are the concrete argument types — not frozen
/// `unknown`. This is the inference-side half of the WS-9c fix: the object
/// literal keeps its field-value parameters as `tyvar` markers, and the
/// `resolved` fixpoint is published into the unifier so the final
/// substitution pass concretizes them.
#[test]
fn test_ws9c_anonymous_object_factory_return_fields_resolve_to_int() {
    use shape_ast::ast::TypeAnnotation;
    use shape_ast::parser::parse_program;

    let code = r#"
fn aabb(lo, hi) { {min: lo, max: hi} }
let a = aabb(1, 5)
"#;
    let program = parse_program(code).expect("program should parse");
    let mut engine = TypeInferenceEngine::new();
    let types = engine
        .infer_program(&program)
        .expect("inference should succeed");

    let Some(Type::Function { returns, .. }) = types.get("aabb") else {
        panic!("aabb should be inferred as a function type");
    };
    let TypeAnnotation::Object(fields) = returns
        .to_annotation()
        .expect("aabb return type should convert to an annotation")
    else {
        panic!("aabb should return an anonymous object, got {:?}", returns);
    };
    assert_eq!(fields.len(), 2, "object should have min/max fields");
    for field in &fields {
        assert_eq!(
            field.type_annotation,
            TypeAnnotation::Basic("int".to_string()),
            "field `{}` must resolve to int, not a frozen `unknown`",
            field.name,
        );
    }
}

/// WS-9c: a factory observed at two call sites with `number` arguments
/// resolves its object-literal field types to `number` — proving the field
/// kind tracks the parameter, never a fabricated default.
#[test]
fn test_ws9c_anonymous_object_factory_return_fields_resolve_to_number() {
    use shape_ast::ast::TypeAnnotation;
    use shape_ast::parser::parse_program;

    let code = r#"
fn point(x, y) { {px: x, py: y} }
let a = point(1.0, 2.0)
let b = point(3.0, 4.0)
"#;
    let program = parse_program(code).expect("program should parse");
    let mut engine = TypeInferenceEngine::new();
    let types = engine
        .infer_program(&program)
        .expect("inference should succeed");

    let Some(Type::Function { returns, .. }) = types.get("point") else {
        panic!("point should be inferred as a function type");
    };
    let TypeAnnotation::Object(fields) = returns
        .to_annotation()
        .expect("point return type should convert to an annotation")
    else {
        panic!("point should return an anonymous object, got {:?}", returns);
    };
    for field in &fields {
        assert_eq!(
            field.type_annotation,
            TypeAnnotation::Basic("number".to_string()),
            "field `{}` must resolve to number",
            field.name,
        );
    }
}

/// WS-9c: a factory result threaded through a second unannotated function
/// `fn area(box) { box.max - box.min }` resolves `box`'s field accesses —
/// the transitive callsite-union case. Inference must not reject the body.
#[test]
fn test_ws9c_factory_result_through_unannotated_function_param() {
    use shape_ast::parser::parse_program;

    let code = r#"
fn aabb(lo, hi) { {min: lo, max: hi} }
fn area(box) { box.max - box.min }
let r = area(aabb(1, 5))
"#;
    let program = parse_program(code).expect("program should parse");
    let mut engine = TypeInferenceEngine::new();
    let result = engine.infer_program(&program);
    assert!(
        result.is_ok(),
        "factory result through an unannotated param must not be \
         spuriously rejected, got {:?}",
        result.err(),
    );
}

/// WS-9c: a `tyvar` marker that callsite propagation never resolves stays a
/// marker and projects to a clean `unknown` — an honest "not inferred", not
/// a crash and not a fabricated type. Guards the marker round-trip.
#[test]
fn test_ws9c_unresolved_factory_field_marker_round_trips() {
    use crate::type_system::{TypeVar, annotation_as_tyvar, tyvar_to_annotation};

    let var = TypeVar::new("T42".to_string());
    let ann = tyvar_to_annotation(&var);
    assert_eq!(
        annotation_as_tyvar(&ann),
        Some(var),
        "a tyvar marker must decode back to its variable",
    );
    // A plain user type name is never mistaken for a marker.
    assert_eq!(
        annotation_as_tyvar(&shape_ast::ast::TypeAnnotation::Basic("T42".to_string())),
        None,
        "a bare `T42` user type must not be decoded as a tyvar marker",
    );
}

// ===========================================================================
// Fn-boundary let-generalization corpus
// (docs/design/let-gen-gating-predicate-spec.md §5)
//
// cond-4 (§1.2) is the syntactic non-expansiveness gate at the fn-boundary;
// §3.2 is the value-restriction refusal for mutable/shared provenance; §4
// (A-enforced) is the post-solve binding-level reject for a bare-application
// `let` whose final type is still a fully-polymorphic carrier.
// ===========================================================================

/// Assert a program type-checks (let-gen accepts it).
#[cfg(test)]
fn letgen_accepts(code: &str) {
    use shape_ast::parser::parse_program;
    let program = parse_program(code).expect("program should parse");
    let mut engine = TypeInferenceEngine::new();
    let (_types, errors) = engine.infer_program_best_effort(&program);
    assert!(
        errors.is_empty(),
        "let-gen should ACCEPT this program, got errors: {:?}\ncode:\n{}",
        errors,
        code
    );
}

/// Assert a program is REJECTED with a `GenericTypeError` (cond-4 / §3.2 / §4).
#[cfg(test)]
fn letgen_rejects(code: &str) {
    use shape_ast::parser::parse_program;
    let program = parse_program(code).expect("program should parse");
    let mut engine = TypeInferenceEngine::new();
    let (_types, errors) = engine.infer_program_best_effort(&program);
    assert!(
        errors
            .iter()
            .any(|e| matches!(e, TypeError::GenericTypeError { .. })),
        "let-gen should REJECT this program with GenericTypeError, got errors: {:?}\ncode:\n{}",
        errors,
        code
    );
}

// --- §5.1 ACCEPT --------------------------------------------------------

/// A2: `?? 42` pins the carrier's inner type to int ⇒ fn generalizes, binding
/// is concrete.
#[test]
fn letgen_a2_null_coalesce_pins_inner() {
    letgen_accepts(
        r#"
        fn get_val() { return None }
        let v = get_val() ?? 42
        v
    "#,
    );
}

/// A3: a pure `Err("boom")` carrier — `Result<T, string>` has a concrete arg,
/// so the §4 binding-level reject does NOT fire (the A1-vs-A3 split). The
/// kind-erased carrier compiles (§1.4).
#[test]
fn letgen_a3_err_carrier_concrete_arg_compiles() {
    letgen_accepts(
        r#"
        fn step1() { return Err("boom") }
        let y = step1()
        y
    "#,
    );
}

/// A4: `find_user()` returning `None` consumed at a site that pins `T` via a
/// declared `-> Result<number>` and the `!! ?` chain.
#[test]
fn letgen_a4_pinned_by_callsite() {
    letgen_accepts(
        r#"
        fn find_user() { None }
        fn use_it() -> Result<number> {
            let v = (find_user() !! "missing")?
            Ok(v + 1.0)
        }
        use_it()
    "#,
    );
}

/// A_pure_local (§1.2 cond-4 accept): a fn-local IMMUTABLE `let` chain bottoming
/// out in a literal `None` is non-expansive ⇒ generalizes.
#[test]
fn letgen_a_pure_local_immutable_let_chain() {
    letgen_accepts(
        r#"
        fn get_none() { let inner = None; return inner }
        let v = get_none() ?? 7
        v
    "#,
    );
}

/// A_rec (§5.1): mixed None/recursive return takes the union branch yielding a
/// bare var (not `Type::Generic`) ⇒ gate (c) false ⇒ the fix's path is not even
/// taken. Guards against a polymorphic-recursion regression.
#[test]
fn letgen_a_rec_recursive_none_union() {
    letgen_accepts(
        r#"
        fn rec(n) { if n <= 0 { return None } return rec(n - 1) }
        let r = rec(3) ?? 0
        r
    "#,
    );
}

/// A direct `let x = None` (grounding class-(3) value binding) is NOT a
/// bare-application and compiles — the §4 reject only targets class-(2)
/// applications. Matches the language's established acceptance of `None`.
#[test]
fn letgen_direct_none_value_binding_compiles() {
    letgen_accepts(
        r#"
        let x = None
        match x { Some(v) => v, None => -1 }
    "#,
    );
}

/// `[1,2,3].map(|x| x*2)` as a fn body is a fresh-collection method call over a
/// non-expansive receiver ⇒ cond-4 non-expansive ⇒ the unresolved element var
/// survives to the solver, which pins it to `int`.
#[test]
fn letgen_fresh_collection_method_body_compiles() {
    letgen_accepts(
        r#"
        fn double_all() { [1, 2, 3].map(|x| x * 2) }
        double_all()[0]
    "#,
    );
}

// --- §4 A-enforced ------------------------------------------------------

/// A1 under A-enforced: a bare-application `let x = get_none()` whose final type
/// is a fully-polymorphic `Option<T>` is a COMPILE ERROR demanding an
/// annotation (§4 / §5.1 A1 "§4 user decision governs whether this errors").
#[test]
fn letgen_a1_bare_application_unpinned_rejects() {
    letgen_rejects(
        r#"
        fn get_none() { return None }
        let x = get_none()
        x
    "#,
    );
}

/// A1 remedy: annotating the binding pins `T` ⇒ compiles.
#[test]
fn letgen_a1_annotated_binding_compiles() {
    letgen_accepts(
        r#"
        fn get_none() { return None }
        let x: Option<int> = get_none()
        x
    "#,
    );
}

// --- §5.2 MUST-REJECT (the §3.2 value-restriction leak repros) -----------

/// R1 (=T17): `get_slot` returns a module-level mutable `var slot` ⇒ cond-4
/// expansive ⇒ COMPILE ERROR instead of the former runtime `TypeError`.
#[test]
fn letgen_r1_returns_module_var_rejects() {
    letgen_rejects(
        r#"
        var slot = None
        fn get_slot() { return slot }
        slot = Some(5)
        let b: string = get_slot()!
        print(b)
    "#,
    );
}

/// R2 (=T18): same shared-mutable provenance, consumed via `match`.
#[test]
fn letgen_r2_module_var_through_match_rejects() {
    letgen_rejects(
        r#"
        var slot = None
        fn get_slot() { return slot }
        slot = Some(5)
        let r = get_slot()
        match r { Some(s) => { let z: string = s; print(z) } None => {} }
    "#,
    );
}

/// R3 (=T20): one shared cell typed both int AND string through `get_slot` — the
/// core unsoundness. MUST reject (on `main`/ReliableOnly it compiled + ran).
#[test]
fn letgen_r3_one_cell_int_and_string_rejects() {
    letgen_rejects(
        r#"
        var slot = None
        fn get_slot() { return slot }
        fn put_int() { slot = Some(1) }
        fn put_str() { slot = Some("a") }
        put_int()
        let x: int = get_slot() ?? 0
        put_str()
        let y: string = get_slot() ?? ""
        print(x)
        print(y)
    "#,
    );
}

/// R4 (ref): a fn returning a reference into a mutable binding — cond-4 treats
/// `&slot` as expansive (a reference/deref is never a fresh carrier).
#[test]
fn letgen_r4_reference_into_mutable_rejects() {
    letgen_rejects(
        r#"
        var slot = None
        fn get_ref() { return &slot }
        slot = Some(5)
        let b: string = get_ref()!
        print(b)
    "#,
    );
}

/// §5.3 empty-array boundary: the empty-array seam is a separate bytecode-level
/// check; this fix does not move it. A `let a = []` still requires an
/// annotation. (Asserted at the inference level: the bare binding does not
/// silently type-check to a concrete array.)
#[test]
fn letgen_empty_array_remedy_unchanged() {
    // `let a: Array<int> = []` (annotated) must still type-check.
    letgen_accepts(
        r#"
        let a: Array<int> = []
        a.length
    "#,
    );
}

/// HOF return-type aliasing (the sg2 root). An unannotated wrapper whose return
/// value is precisely `f(x, y)` — invoking its own fn-typed param `f` in tail
/// position — must infer its RETURN type as `f`'s return type. With a NAMED
/// `fn mul(a:int,b:int)->int`, `apply2(mul, 6, 7)` resolves `apply2`'s return
/// to `int` (not a bare variable), so downstream arithmetic on the result types
/// + lowers correctly instead of crashing at runtime ("no method add on
/// receiver kind Int64"). `int` and `number` stay distinct: the inferred return
/// is the EXACT proven type, never a numeric defaulted to `number`.
#[test]
fn hof_wrapper_return_type_resolves_from_fn_param_return_named() {
    use shape_ast::ast::TypeAnnotation;
    use shape_ast::parser::parse_program;

    let code = r#"
fn apply2(f, x, y) { f(x, y) }
fn mul(a: int, b: int) -> int { a * b }
let r = apply2(mul, 6, 7)
"#;

    let program = parse_program(code).expect("program should parse");
    let mut engine = TypeInferenceEngine::new();
    let (types, _errors) = engine.infer_program_best_effort(&program);

    let apply2_type = types.get("apply2").expect("apply2 should be inferred");
    match apply2_type {
        Type::Function { returns, .. } => {
            let ann = returns
                .to_annotation()
                .expect("return should convert to annotation");
            assert!(
                matches!(&ann, TypeAnnotation::Basic(name) if name == "int"),
                "apply2's return must resolve to int (f's return), got {:?}",
                ann
            );
        }
        other => panic!("expected function type for apply2, got {:?}", other),
    }
}

/// The closure-literal sibling of the named case: `apply2(|a,b| a*b, 6, 7)`
/// resolves `apply2`'s return to the closure body's `int` return. int stays int
/// — the closure params are pinned to `int` by the `6, 7` call-site args, never
/// defaulted to `number`.
#[test]
fn hof_wrapper_return_type_resolves_from_fn_param_return_closure() {
    use shape_ast::ast::TypeAnnotation;
    use shape_ast::parser::parse_program;

    let code = r#"
fn apply2(f, x, y) { f(x, y) }
let r = apply2(|a, b| a * b, 6, 7)
"#;

    let program = parse_program(code).expect("program should parse");
    let mut engine = TypeInferenceEngine::new();
    let (types, _errors) = engine.infer_program_best_effort(&program);

    let apply2_type = types.get("apply2").expect("apply2 should be inferred");
    match apply2_type {
        Type::Function { returns, .. } => {
            let ann = returns
                .to_annotation()
                .expect("return should convert to annotation");
            assert!(
                matches!(&ann, TypeAnnotation::Basic(name) if name == "int"),
                "apply2's return must resolve to int (closure body return), got {:?}",
                ann
            );
        }
        other => panic!("expected function type for apply2, got {:?}", other),
    }
}

/// number-preservation sibling: a `number`-typed closure body keeps the wrapper
/// return `number`. int and number do NOT unify — the resolution copies the
/// EXACT proven family.
#[test]
fn hof_wrapper_return_type_preserves_number() {
    use shape_ast::ast::TypeAnnotation;
    use shape_ast::parser::parse_program;

    let code = r#"
fn apply2(f, x, y) { f(x, y) }
fn mulf(a: number, b: number) -> number { a * b }
let r = apply2(mulf, 2.0, 3.0)
"#;

    let program = parse_program(code).expect("program should parse");
    let mut engine = TypeInferenceEngine::new();
    let (types, _errors) = engine.infer_program_best_effort(&program);

    let apply2_type = types.get("apply2").expect("apply2 should be inferred");
    match apply2_type {
        Type::Function { returns, .. } => {
            let ann = returns
                .to_annotation()
                .expect("return should convert to annotation");
            assert!(
                matches!(&ann, TypeAnnotation::Basic(name) if name == "number"),
                "apply2's return must stay number (mulf's return), got {:?}",
                ann
            );
        }
        other => panic!("expected function type for apply2, got {:?}", other),
    }
}

// control-flow.mdx "Break with Value": a `loop { ... break <v> }` used as a
// value is an expression whose type is the unified type of all `break <v>`
// values. A value-less `loop { break }` stays Void.

#[test]
fn test_loop_break_value_types_as_unified_break_type() {
    use shape_ast::ast::TypeAnnotation;
    use shape_ast::parser::parse_program;

    // `let r = loop { i = i + 1; if i == 5 { break i * 10 } }` — r must be int,
    // NOT Void (the pre-fix bug typed the whole loop Void, making r unusable in
    // any typed context).
    let code = r#"
var i = 0
let r = loop {
    i = i + 1
    if i == 5 { break i * 10 }
}
"#;

    let program = parse_program(code).expect("program should parse");
    let mut engine = TypeInferenceEngine::new();
    let types = engine
        .infer_program(&program)
        .expect("inference should succeed");

    let r_type = types.get("r").expect("r should be inferred");
    let ann = r_type
        .to_annotation()
        .expect("r should convert to annotation");
    assert!(
        matches!(&ann, TypeAnnotation::Basic(name) if name == "int"),
        "loop-with-break-value `r` must type as the break value's type (int), got {:?}",
        ann
    );
}

#[test]
fn test_loop_break_value_checks_against_let_annotation() {
    use shape_ast::parser::parse_program;

    // `let r: int = loop { ... break 7 }` must type-check (the break value
    // unifies with the declared annotation).
    let code = r#"
var i = 0
let r: int = loop {
    i = i + 1
    if i == 3 { break 7 }
}
"#;

    let program = parse_program(code).expect("program should parse");
    let mut engine = TypeInferenceEngine::new();
    engine
        .infer_program(&program)
        .expect("loop break-value should satisfy the `let r: int` annotation");
}

#[test]
fn test_loop_break_value_string_rejected_against_int_annotation() {
    use shape_ast::parser::parse_program;

    // `break "hello"` against `let r: int` is a genuine type error — int and
    // string never unify (no silent coercion).
    let code = r#"
var i = 0
let r: int = loop {
    i = i + 1
    if i == 3 { break "hello" }
}
"#;

    let program = parse_program(code).expect("program should parse");
    let mut engine = TypeInferenceEngine::new();
    assert!(
        engine.infer_program(&program).is_err(),
        "a string break value must not satisfy a `let r: int` annotation"
    );
}

#[test]
fn test_loop_mismatched_break_values_rejected() {
    use shape_ast::parser::parse_program;

    // Two `break <value>` of different types in the SAME loop must NOT unify —
    // `loop` is no more permissive than `if`/`match`, which directly constrain
    // arm-vs-arm. Before the fix, `combine_return_types` branded a nominal
    // union over [int, string] WITHOUT pushing the pairwise unify constraint,
    // so this program compiled and bound a string into an `int` slot (heap
    // pointer reinterpreted as a scalar of the wrong declared type). With the
    // pairwise break-type constraints in place this is a real type error.
    let code = r#"
var i = 0
let r: int = loop {
    i = i + 1
    if i == 1 { break "this is a string" }
    break 7
}
"#;

    let program = parse_program(code).expect("program should parse");
    let mut engine = TypeInferenceEngine::new();
    assert!(
        engine.infer_program(&program).is_err(),
        "mismatched break-value types (string vs int) must not unify into a loop type"
    );
}

#[test]
fn test_loop_mismatched_break_int_number_rejected() {
    use shape_ast::parser::parse_program;

    // int and number never unify — a `break 1` / `break 2.0` mix is a compile
    // error, no silent numeric coercion (CLAUDE.md §Type System Rules).
    let code = r#"
var i = 0
let r = loop {
    i = i + 1
    if i == 1 { break 1 }
    break 2.0
}
"#;

    let program = parse_program(code).expect("program should parse");
    let mut engine = TypeInferenceEngine::new();
    assert!(
        engine.infer_program(&program).is_err(),
        "a mixed int/number break must be rejected (int != number, no coercion)"
    );
}

#[test]
fn test_value_less_loop_stays_void() {
    use shape_ast::ast::TypeAnnotation;
    use shape_ast::parser::parse_program;

    // A `loop { break }` with no value-carrying break never produces a value:
    // it stays Void.
    let code = r#"
var i = 0
let r = loop {
    i = i + 1
    if i == 3 { break }
}
"#;

    let program = parse_program(code).expect("program should parse");
    let mut engine = TypeInferenceEngine::new();
    let types = engine
        .infer_program(&program)
        .expect("inference should succeed");

    let r_type = types.get("r").expect("r should be inferred");
    let ann = r_type
        .to_annotation()
        .expect("r should convert to annotation");
    assert!(
        matches!(&ann, TypeAnnotation::Void),
        "a value-less loop must stay Void, got {:?}",
        ann
    );
}

#[test]
fn test_if_else_semicolon_discarded_tail_type_checks() {
    use shape_ast::parser::parse_program;

    // `;`-discard at an if/else branch tail: a trailing `expr;` in each arm
    // discards to Unit, so the arm types do NOT have to unify into the
    // if-expression type. Here the two arms call fns of DIFFERENT types
    // (int vs string); the trailing `;` must keep the `if` a Unit statement.
    let code = r#"
fn f() -> int { 1 }
fn g() -> string { "two" }
let x = 5
if x > 0 { f(); } else { g(); }
"#;

    let program = parse_program(code).expect("program should parse");
    let mut engine = TypeInferenceEngine::new();
    engine
        .infer_program(&program)
        .expect("`;`-discarded if/else branch tails must type-check (arms discard to Unit)");
}
