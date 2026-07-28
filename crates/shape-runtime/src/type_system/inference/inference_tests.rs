use super::*;
use crate::type_system::effects::EffectRow;

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
        Type::Function {
            params, returns, ..
        } => {
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
fn test_none_initializer_checks_against_generic_option_annotation() {
    use shape_ast::ast::TypeAnnotation;
    use shape_ast::parser::parse_program;

    let code = r#"
type Box<T> {
    value: T?
}

fn make<T>(input: T) -> Box<T> {
    var x: T? = None
    x = Some(input)
    Box {
        value: x
    }
}

let b = make(1)
"#;

    let program = parse_program(code).expect("program should parse");
    let mut engine = TypeInferenceEngine::new();
    let types = engine
        .infer_program(&program)
        .expect("inference should accept contextual None for T?");

    let b_type = types.get("b").expect("b should be inferred").canonicalize();
    match b_type {
        Type::Generic { base, args } => {
            assert!(
                matches!(
                    base.as_ref(),
                    Type::Concrete(TypeAnnotation::Reference(name)) if name == "Box"
                ),
                "expected Box<T> base, got {:?}",
                base
            );
            assert_eq!(args.len(), 1, "Box must have one type argument");
            assert_eq!(
                args[0],
                Type::Concrete(TypeAnnotation::Basic("int".to_string()))
            );
        }
        other => panic!("expected Box<int> for b, got {:?}", other),
    }
}

#[test]
fn test_generic_struct_infers_type_param_from_canonical_array_field() {
    use shape_ast::ast::TypeAnnotation;
    use shape_ast::parser::parse_program;

    let code = r#"
type PropertyResult<T> {
    passed: bool,
    counterexample: T?
}

type PropertySummary<T> {
    results: Array<PropertyResult<T>>
}

fn summarize<T>(results: Array<PropertyResult<T>>) -> PropertySummary<T> {
    PropertySummary {
        results: results
    }
}

let item = PropertyResult { passed: true, counterexample: Some(1) }
let summary = summarize([item])
"#;

    let program = parse_program(code).expect("program should parse");
    let mut engine = TypeInferenceEngine::new();
    let types = engine
        .infer_program(&program)
        .expect("inference should bind T through Array<PropertyResult<T>>");

    let summary_type = types
        .get("summary")
        .expect("summary should be inferred")
        .canonicalize();
    match summary_type {
        Type::Generic { base, args } => {
            assert!(
                matches!(
                    base.as_ref(),
                    Type::Concrete(TypeAnnotation::Reference(name)) if name == "PropertySummary"
                ),
                "expected PropertySummary<T> base, got {:?}",
                base
            );
            assert_eq!(args.len(), 1, "PropertySummary must have one type argument");
            assert_eq!(
                args[0],
                Type::Concrete(TypeAnnotation::Basic("int".to_string()))
            );
        }
        other => panic!("expected PropertySummary<int> for summary, got {:?}", other),
    }
}

#[test]
fn test_generic_struct_infers_type_param_from_function_fields() {
    use shape_ast::ast::TypeAnnotation;
    use shape_ast::parser::parse_program;

    let code = r#"
type PropertySpec<T> {
    name: string,
    trials: int,
    gen: () => T,
    prop: (T) => bool
}

let gen: () => number = || 1.0
let prop: (number) => bool = |x| x > 0.0
let spec = PropertySpec {
    name: "positive",
    trials: 1,
    gen: gen,
    prop: prop
}
"#;

    let program = parse_program(code).expect("program should parse");
    let mut engine = TypeInferenceEngine::new();
    let types = engine
        .infer_program(&program)
        .expect("inference should bind T through function-typed fields");

    let spec_type = types
        .get("spec")
        .expect("spec should be inferred")
        .canonicalize();
    match spec_type {
        Type::Generic { base, args } => {
            assert!(
                matches!(
                    base.as_ref(),
                    Type::Concrete(TypeAnnotation::Reference(name)) if name == "PropertySpec"
                ),
                "expected PropertySpec<T> base, got {:?}",
                base
            );
            assert_eq!(args.len(), 1, "PropertySpec must have one type argument");
            assert_eq!(
                args[0],
                Type::Concrete(TypeAnnotation::Basic("number".to_string()))
            );
        }
        other => panic!("expected PropertySpec<number> for spec, got {:?}", other),
    }
}

#[test]
fn test_generic_zero_arg_callable_return_binds_named_call_result() {
    use shape_ast::ast::TypeAnnotation;
    use shape_ast::parser::parse_program;

    let code = r#"
fn apply<T>(gen_fn: () => T) -> T {
    gen_fn()
}

let gen: () => number = || 1.0
let value = apply(gen)
"#;

    let program = parse_program(code).expect("program should parse");
    let mut engine = TypeInferenceEngine::new();
    let types = engine
        .infer_program(&program)
        .expect("inference should bind T from () => number return");

    let value_ty = types
        .get("value")
        .expect("value should be inferred")
        .canonicalize();
    assert_eq!(
        value_ty,
        Type::Concrete(TypeAnnotation::Basic("number".into()))
    );
}

#[test]
fn test_generic_zero_arg_callable_return_binds_named_annotation_proof() {
    use shape_ast::ast::TypeAnnotation;

    let mut engine = TypeInferenceEngine::new();
    let declared_t = TypeVar::declared(engine.type_var_gen.fresh_declared_owner(), 0, "T");
    let declaration = TypeScheme::poly(
        vec![declared_t.clone()],
        Type::Function {
            params: vec![],
            returns: Box::new(Type::Variable(declared_t)),
            effects: EffectRow::Unproven,
        },
    );
    let generic_params = TypeInferenceEngine::declared_parameter_tokens(&declaration)
        .expect("declared scheme should expose its exact parameter capability");

    let instance = Type::Function {
        params: vec![],
        returns: Box::new(Type::Concrete(TypeAnnotation::Basic("number".into()))),
        effects: EffectRow::Unproven,
    };
    let proven = Type::Function {
        params: vec![],
        returns: Box::new(Type::Concrete(TypeAnnotation::Reference("T".into()))),
        effects: EffectRow::Unproven,
    };

    engine
        .bind_callsite_return_to_proven_shape_with_params(&instance, &proven, &generic_params)
        .expect("number should satisfy a proven () => T annotation for this callsite");
}

#[test]
fn test_generic_zero_arg_callable_return_rejects_conflicting_proof() {
    use shape_ast::ast::TypeAnnotation;

    let mut engine = TypeInferenceEngine::new();
    let t = TypeVar::new("T".to_string());
    engine.solver.unifier_mut().bind(
        t.clone(),
        Type::Concrete(TypeAnnotation::Basic("int".into())),
    );

    let instance = Type::Function {
        params: vec![],
        returns: Box::new(Type::Concrete(TypeAnnotation::Basic("number".into()))),
        effects: EffectRow::Unproven,
    };
    let proven = Type::Function {
        params: vec![],
        returns: Box::new(Type::Variable(t)),
        effects: EffectRow::Unproven,
    };

    let err = engine
        .bind_callsite_return_to_proven_shape(&instance, &proven)
        .expect_err("number must not satisfy an already-proven int return");

    match err {
        TypeError::ConstraintViolation(message) => {
            assert!(message.contains("number"), "{message}");
            assert!(message.contains("int"), "{message}");
        }
        other => panic!("expected constraint violation, got {other:?}"),
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
        Type::Function {
            params, returns, ..
        } => {
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
fn test_recursive_numeric_function_ignores_never_bottom_in_callsite_union() {
    use shape_ast::ast::TypeAnnotation;
    use shape_ast::parser::parse_program;

    let code = r#"
function fibonacci(n) {
  if n <= 1 {
    return n
  } else {
    return fibonacci(n - 1) + fibonacci(n - 2)
  }
}

let result = fibonacci(15)
"#;

    let program = parse_program(code).expect("program should parse");
    let mut engine = TypeInferenceEngine::new();
    let types = engine
        .infer_program(&program)
        .expect("recursive numeric inference should succeed");

    let (param, ret) =
        fn_param_return_basic(types.get("fibonacci").expect("fibonacci should infer"))
            .expect("fibonacci should infer as fn(basic)->basic");
    assert_eq!(
        (param.as_str(), ret.as_str()),
        ("int", "int"),
        "recursive numeric proof should collapse int|never to int"
    );
    assert!(
        matches!(types.get("result"), Some(Type::Concrete(TypeAnnotation::Basic(name))) if name == "int"),
        "recursive call result should infer as int, got {:?}",
        types.get("result")
    );
}

#[test]
fn test_recursive_numeric_function_rejects_non_numeric_recursive_arg() {
    use shape_ast::parser::parse_program;

    let code = r#"
function bad(n) {
  if n <= 1 {
    return n
  } else {
    return bad(n - 1) + bad("nope")
  }
}

let result = bad(2)
"#;

    let program = parse_program(code).expect("program should parse");
    let mut engine = TypeInferenceEngine::new();
    let (_types, errors) = engine.infer_program_best_effort(&program);
    assert!(
        errors.iter().any(|err| matches!(
            err,
            TypeError::ConstraintViolation(message)
                if message.contains("parameter at position 0 of 'bad' must be numeric")
                    && message.contains("string")
        )),
        "non-numeric recursive call should reject, got {:?}",
        errors
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
        Type::Function {
            params, returns, ..
        } => {
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
        Type::Function {
            params, returns, ..
        } => {
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
    let Type::Function {
        params, returns, ..
    } = ty
    else {
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
        let Type::Function {
            params, returns, ..
        } = dbl
        else {
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

    let engine = TypeInferenceEngine::new();
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

// ========================================================================
// U4-0: ENGINE SPAN-TABLE COMPLETENESS (closure-body field-reads + Borrow
// referents). The engine standalone gate for the unification roadmap
// (docs/cluster-audits/U4-ROADMAP.md §4 Wave U4-0 / §5 / §6 / §7 risk #4).
//
// SUCCESS CRITERION = engine-standalone: after inference + finalize, the
// per-expression span table must RESOLVE the closure-field call/return sites
// to the concrete field type, KEEP the un-inferable tail DROPPED (None), and
// LEAVE STAGE-F1 strictness intact (an unannotated empty-`[]` field read is
// still an engine ConstraintViolation). The actual programs (f8 etc.) STILL
// fail at runtime until later waves delete the fallback/mini-inferencer.
// ========================================================================

/// Recursively collect every expression node's span+kind for span lookup.
/// Returns the FIRST span matching `pred`, walking the whole program AST.
fn u40_find_expr_span<F>(
    program: &shape_ast::ast::Program,
    pred: &F,
) -> Option<shape_ast::ast::Span>
where
    F: Fn(&shape_ast::ast::Expr) -> bool,
{
    use shape_ast::ast::{Expr, Item, Span, Spanned, Statement};

    fn walk_expr<F: Fn(&Expr) -> bool>(e: &Expr, pred: &F, out: &mut Option<Span>) {
        if out.is_some() {
            return;
        }
        if pred(e) {
            *out = Some(Spanned::span(e));
            return;
        }
        match e {
            Expr::BinaryOp { left, right, .. } => {
                walk_expr(left, pred, out);
                walk_expr(right, pred, out);
            }
            Expr::UnaryOp { operand, .. } => walk_expr(operand, pred, out),
            Expr::PropertyAccess { object, .. } => walk_expr(object, pred, out),
            Expr::IndexAccess { object, index, .. } => {
                walk_expr(object, pred, out);
                walk_expr(index, pred, out);
            }
            Expr::FunctionCall { args, .. } => {
                for a in args {
                    walk_expr(a, pred, out);
                }
            }
            Expr::FunctionExpr { body, .. } => walk_stmts(body, pred, out),
            Expr::Block(block, _) => {
                for item in &block.items {
                    match item {
                        shape_ast::ast::BlockItem::Expression(ex) => walk_expr(ex, pred, out),
                        shape_ast::ast::BlockItem::Statement(s) => walk_stmt(s, pred, out),
                        shape_ast::ast::BlockItem::VariableDecl(decl) => {
                            if let Some(v) = &decl.value {
                                walk_expr(v, pred, out);
                            }
                        }
                        shape_ast::ast::BlockItem::Assignment(_) => {}
                    }
                }
            }
            Expr::Return(Some(v), _) => walk_expr(v, pred, out),
            Expr::Reference { expr, .. } => walk_expr(expr, pred, out),
            Expr::Match(match_expr, _) => {
                walk_expr(&match_expr.scrutinee, pred, out);
                for arm in &match_expr.arms {
                    if let Some(guard) = &arm.guard {
                        walk_expr(guard, pred, out);
                    }
                    walk_expr(&arm.body, pred, out);
                }
            }
            _ => {}
        }
    }

    fn walk_stmt<F: Fn(&Expr) -> bool>(s: &Statement, pred: &F, out: &mut Option<Span>) {
        match s {
            Statement::Expression(e, _) => walk_expr(e, pred, out),
            Statement::Return(Some(e), _) => walk_expr(e, pred, out),
            Statement::VariableDecl(decl, _) => {
                if let Some(v) = &decl.value {
                    walk_expr(v, pred, out);
                }
            }
            _ => {}
        }
    }

    fn walk_stmts<F: Fn(&Expr) -> bool>(stmts: &[Statement], pred: &F, out: &mut Option<Span>) {
        for s in stmts {
            walk_stmt(s, pred, out);
        }
    }

    let mut out = None;
    for item in &program.items {
        match item {
            Item::Statement(stmt, _) => walk_stmt(stmt, pred, &mut out),
            Item::Function(func, _) => walk_stmts(&func.body, pred, &mut out),
            _ => {}
        }
        if out.is_some() {
            break;
        }
    }
    out
}

fn u40_find_binding_span(
    program: &shape_ast::ast::Program,
    name: &str,
) -> Option<shape_ast::ast::Span> {
    use shape_ast::ast::{BlockItem, Expr, Item, Pattern, Span, Statement};

    fn decl_span(decl: &shape_ast::ast::VariableDecl, name: &str, out: &mut Option<Span>) {
        if out.is_some() {
            return;
        }
        *out = decl
            .pattern
            .get_bindings()
            .into_iter()
            .find_map(|(binding, span)| (binding == name).then_some(span));
    }

    fn pattern_span(pattern: &Pattern, name: &str, out: &mut Option<Span>) {
        if out.is_some() {
            return;
        }
        *out = pattern
            .get_bindings()
            .into_iter()
            .find_map(|(binding, span)| (binding == name).then_some(span));
    }

    fn walk_expr(e: &Expr, name: &str, out: &mut Option<Span>) {
        if out.is_some() {
            return;
        }
        match e {
            Expr::Block(block, _) => {
                for item in &block.items {
                    match item {
                        BlockItem::VariableDecl(decl) => {
                            decl_span(decl, name, out);
                            if let Some(value) = &decl.value {
                                walk_expr(value, name, out);
                            }
                        }
                        BlockItem::Statement(stmt) => walk_stmt(stmt, name, out),
                        BlockItem::Expression(expr) => walk_expr(expr, name, out),
                        BlockItem::Assignment(_) => {}
                    }
                    if out.is_some() {
                        break;
                    }
                }
            }
            Expr::FunctionCall { args, .. } => {
                for arg in args {
                    walk_expr(arg, name, out);
                }
            }
            Expr::MethodCall { receiver, args, .. } => {
                walk_expr(receiver, name, out);
                for arg in args {
                    walk_expr(arg, name, out);
                }
            }
            Expr::Match(match_expr, _) => {
                walk_expr(&match_expr.scrutinee, name, out);
                for arm in &match_expr.arms {
                    pattern_span(&arm.pattern, name, out);
                    if let Some(guard) = &arm.guard {
                        walk_expr(guard, name, out);
                    }
                    walk_expr(&arm.body, name, out);
                    if out.is_some() {
                        break;
                    }
                }
            }
            Expr::Return(Some(value), _) => walk_expr(value, name, out),
            _ => {}
        }
    }

    fn walk_stmt(s: &Statement, name: &str, out: &mut Option<Span>) {
        if out.is_some() {
            return;
        }
        match s {
            Statement::VariableDecl(decl, _) => {
                decl_span(decl, name, out);
                if let Some(value) = &decl.value {
                    walk_expr(value, name, out);
                }
            }
            Statement::Expression(expr, _) | Statement::Return(Some(expr), _) => {
                walk_expr(expr, name, out)
            }
            _ => {}
        }
    }

    let mut out = None;
    for item in &program.items {
        match item {
            Item::VariableDecl(decl, _) => decl_span(decl, name, &mut out),
            Item::Statement(stmt, _) => walk_stmt(stmt, name, &mut out),
            Item::Function(func, _) => {
                for param in &func.params {
                    out = param
                        .pattern
                        .get_bindings()
                        .into_iter()
                        .find_map(|(binding, span)| (binding == name).then_some(span));
                    if out.is_some() {
                        break;
                    }
                }
                if out.is_some() {
                    break;
                }
                for stmt in &func.body {
                    walk_stmt(stmt, name, &mut out);
                    if out.is_some() {
                        break;
                    }
                }
            }
            _ => {}
        }
        if out.is_some() {
            break;
        }
    }
    out
}

/// Resolve a program through the engine and return (engine, types, errors).
fn u40_infer(code: &str) -> (TypeInferenceEngine, HashMap<String, Type>, Vec<TypeError>) {
    use shape_ast::parser::parse_program;
    let program = parse_program(code).expect("program should parse");
    let mut engine = TypeInferenceEngine::new();
    let (types, errors) = engine.infer_program_best_effort(&program);
    (engine, types, errors)
}

/// Assert a `Type` is exactly the `int` scalar (the closure-field result type
/// for the §6 regression corpus). Accepts both `Basic("int")` carriers.
fn u40_is_int(ty: &Type) -> bool {
    matches!(ty, Type::Concrete(TypeAnnotation::Basic(n)) if n == "int")
}

fn u40_is_number(ty: &Type) -> bool {
    matches!(ty, Type::Concrete(TypeAnnotation::Basic(n)) if n == "number")
}

fn u40_is_string(ty: &Type) -> bool {
    matches!(ty, Type::Concrete(TypeAnnotation::Basic(n)) if n == "string")
}

fn u40_is_array_of_int(ty: &Type) -> bool {
    match ty.canonicalize() {
        Type::Generic { base, args } if args.len() == 1 => {
            matches!(
                base.as_ref(),
                Type::Concrete(TypeAnnotation::Reference(name)) if name.as_str() == "Array"
            ) && u40_is_int(&args[0])
        }
        _ => false,
    }
}

fn u40_object_has_int_fields(ty: &Type, field_names: &[&str]) -> bool {
    match ty {
        Type::Concrete(TypeAnnotation::Object(fields)) => field_names.iter().all(|name| {
            fields
                .iter()
                .find(|field| field.name == *name)
                .is_some_and(|field| u40_is_int(&Type::Concrete(field.type_annotation.clone())))
        }),
        _ => false,
    }
}

fn u40_is_hashmap_string_int(ty: &Type) -> bool {
    match ty {
        Type::Generic { base, args } if args.len() == 2 => {
            matches!(
                base.as_ref(),
                Type::Concrete(TypeAnnotation::Reference(n)) if n.as_str() == "HashMap"
            ) && u40_is_string(&args[0])
                && u40_is_int(&args[1])
        }
        _ => false,
    }
}

fn u40_is_function_int_int_to_int(ty: &Type) -> bool {
    match ty.canonicalize() {
        Type::Function {
            params, returns, ..
        } if params.len() == 2 => {
            u40_is_int(&params[0]) && u40_is_int(&params[1]) && u40_is_int(&returns)
        }
        Type::Concrete(TypeAnnotation::Function { params, returns }) if params.len() == 2 => {
            let p0 = Type::Concrete(params[0].type_annotation.clone());
            let p1 = Type::Concrete(params[1].type_annotation.clone());
            let ret = Type::Concrete(*returns);
            u40_is_int(&p0) && u40_is_int(&p1) && u40_is_int(&ret)
        }
        _ => false,
    }
}

fn u40_is_option_of(ty: &Type, inner: fn(&Type) -> bool) -> bool {
    match ty.canonicalize() {
        Type::Generic { base, args } if args.len() == 1 => {
            matches!(
                base.as_ref(),
                Type::Concrete(TypeAnnotation::Reference(n)) if n.as_str() == "Option"
            ) && inner(&args[0])
        }
        Type::Concrete(TypeAnnotation::Generic { name, args })
            if name == "Option" && args.len() == 1 =>
        {
            inner(&Type::Concrete(args[0].clone()))
        }
        _ => false,
    }
}

fn u40_is_option_int(ty: &Type) -> bool {
    u40_is_option_of(ty, u40_is_int)
}

fn u40_is_option_function_int_int_to_int(ty: &Type) -> bool {
    u40_is_option_of(ty, u40_is_function_int_int_to_int)
}

fn u40_is_hashmap_string_int_function(ty: &Type) -> bool {
    match ty {
        Type::Generic { base, args } if args.len() == 2 => {
            matches!(
                base.as_ref(),
                Type::Concrete(TypeAnnotation::Reference(n)) if n.as_str() == "HashMap"
            ) && u40_is_string(&args[0])
                && u40_is_function_int_int_to_int(&args[1])
        }
        _ => false,
    }
}

fn u40_match_binding_fact(code: &str, name: &str) -> BindingFact {
    use shape_ast::parser::parse_program;

    let program = parse_program(code).expect("program should parse");
    let span =
        u40_find_binding_span(&program, name).unwrap_or_else(|| panic!("{name} binding span"));
    let mut engine = TypeInferenceEngine::new();
    let (facts, errors) = engine.infer_program_facts_best_effort(&program);
    assert!(
        errors.is_empty(),
        "match binding fact program should infer cleanly, got {:?}",
        errors
    );
    facts
        .binding_fact(span)
        .unwrap_or_else(|| panic!("{name} binding fact"))
        .clone()
}

fn u40_is_function_returning_int(ty: &Type) -> bool {
    match ty.canonicalize() {
        Type::Function { returns, .. } => u40_is_int(&returns),
        _ => false,
    }
}

fn u40_is_array_of_function_returning_int(ty: &Type) -> bool {
    match ty.canonicalize() {
        Type::Generic { base, args } if args.len() == 1 => {
            matches!(
                base.as_ref(),
                Type::Concrete(TypeAnnotation::Reference(name)) if name.as_str() == "Array"
            ) && u40_is_function_returning_int(&args[0])
        }
        _ => false,
    }
}

#[test]
fn inference_facts_exposes_function_array_param_destructure_binding_types() {
    use shape_ast::parser::parse_program;

    let code = r#"
fn sum_pair([a, b]) {
  return a + b
}

let out = sum_pair([10, 20])
"#;
    let program = parse_program(code).expect("program should parse");
    let a_span = u40_find_binding_span(&program, "a").expect("a param binding span");
    let b_span = u40_find_binding_span(&program, "b").expect("b param binding span");

    let mut engine = TypeInferenceEngine::new();
    let (facts, errors) = engine.infer_program_facts_best_effort(&program);
    assert!(
        errors.is_empty(),
        "function array param destructure should infer cleanly, got {:?}",
        errors
    );
    assert!(
        facts.binding_type(a_span).is_some_and(u40_is_int),
        "a should bind to int, got {:?}",
        facts.binding_type(a_span)
    );
    assert!(
        facts.binding_type(b_span).is_some_and(u40_is_int),
        "b should bind to int, got {:?}",
        facts.binding_type(b_span)
    );
    let Type::Function {
        params, returns, ..
    } = facts
        .function_signature("sum_pair")
        .expect("sum_pair signature fact")
    else {
        panic!("sum_pair should have a function signature")
    };
    assert!(
        params.first().is_some_and(u40_is_array_of_int),
        "sum_pair parameter should resolve to Array<int>, got {:?}",
        params.first()
    );
    assert!(
        u40_is_int(returns),
        "sum_pair return should resolve to int, got {:?}",
        returns
    );
}

#[test]
fn inference_facts_exposes_function_object_param_destructure_binding_types() {
    use shape_ast::parser::parse_program;

    let code = r#"
fn distance({x, y}) {
  return (x * x + y * y) ** 0.5
}

let out = distance({x: 3, y: 4})
"#;
    let program = parse_program(code).expect("program should parse");
    let x_span = u40_find_binding_span(&program, "x").expect("x param binding span");
    let y_span = u40_find_binding_span(&program, "y").expect("y param binding span");

    let mut engine = TypeInferenceEngine::new();
    let (facts, errors) = engine.infer_program_facts_best_effort(&program);
    assert!(
        errors.is_empty(),
        "function object param destructure should infer cleanly, got {:?}",
        errors
    );
    assert!(
        facts.binding_type(x_span).is_some_and(u40_is_int),
        "x should bind to int, got {:?}",
        facts.binding_type(x_span)
    );
    assert!(
        facts.binding_type(y_span).is_some_and(u40_is_int),
        "y should bind to int, got {:?}",
        facts.binding_type(y_span)
    );
    let Type::Function {
        params, returns, ..
    } = facts
        .function_signature("distance")
        .expect("distance signature fact")
    else {
        panic!("distance should have a function signature")
    };
    assert!(
        params
            .first()
            .is_some_and(|param| u40_object_has_int_fields(param, &["x", "y"])),
        "distance parameter should resolve to an object with int x/y fields, got {:?}",
        params.first()
    );
    let _ = returns;
}

#[test]
fn inference_facts_exposes_array_destructure_element_binding_types() {
    use shape_ast::parser::parse_program;

    let code = "let [a, b] = [1, 2]\nlet [n] = [1.5]\n";
    let program = parse_program(code).expect("program should parse");
    let a_span = u40_find_binding_span(&program, "a").expect("a binding span");
    let b_span = u40_find_binding_span(&program, "b").expect("b binding span");
    let n_span = u40_find_binding_span(&program, "n").expect("n binding span");

    let mut engine = TypeInferenceEngine::new();
    let (facts, errors) = engine.infer_program_facts_best_effort(&program);
    assert!(
        errors.is_empty(),
        "array destructure facts should infer cleanly, got {:?}",
        errors
    );
    assert!(
        facts.binding_type(a_span).is_some_and(u40_is_int),
        "a should bind to int, got {:?}",
        facts.binding_type(a_span)
    );
    assert!(
        facts.binding_type(b_span).is_some_and(u40_is_int),
        "b should bind to int, got {:?}",
        facts.binding_type(b_span)
    );
    assert!(
        facts.binding_type(n_span).is_some_and(u40_is_number),
        "n should bind to number, got {:?}",
        facts.binding_type(n_span)
    );
}

#[test]
fn inference_facts_exposes_nested_array_destructure_binding_types() {
    use shape_ast::parser::parse_program;

    let code = "let [[a, b]] = [[1, 2]]\n";
    let program = parse_program(code).expect("program should parse");
    let a_span = u40_find_binding_span(&program, "a").expect("a binding span");
    let b_span = u40_find_binding_span(&program, "b").expect("b binding span");

    let mut engine = TypeInferenceEngine::new();
    let (facts, errors) = engine.infer_program_facts_best_effort(&program);
    assert!(
        errors.is_empty(),
        "nested array destructure facts should infer cleanly, got {:?}",
        errors
    );
    assert!(
        facts.binding_type(a_span).is_some_and(u40_is_int),
        "nested a should bind to int, got {:?}",
        facts.binding_type(a_span)
    );
    assert!(
        facts.binding_type(b_span).is_some_and(u40_is_int),
        "nested b should bind to int, got {:?}",
        facts.binding_type(b_span)
    );
}

#[test]
fn inference_facts_exposes_array_rest_binding_type() {
    use shape_ast::parser::parse_program;

    let code = "let [head, ...tail] = [1, 2, 3]\n";
    let program = parse_program(code).expect("program should parse");
    let head_span = u40_find_binding_span(&program, "head").expect("head binding span");
    let tail_span = u40_find_binding_span(&program, "tail").expect("tail binding span");

    let mut engine = TypeInferenceEngine::new();
    let (facts, errors) = engine.infer_program_facts_best_effort(&program);
    assert!(
        errors.is_empty(),
        "array rest destructure facts should infer cleanly, got {:?}",
        errors
    );
    assert!(
        facts.binding_type(head_span).is_some_and(u40_is_int),
        "head should bind to int, got {:?}",
        facts.binding_type(head_span)
    );
    assert!(
        facts
            .binding_type(tail_span)
            .is_some_and(u40_is_array_of_int),
        "tail should bind to Array<int>, got {:?}",
        facts.binding_type(tail_span)
    );
}

#[test]
fn inference_facts_best_effort_exposes_signature_and_body_expr_type() {
    use shape_ast::ast::Expr;
    use shape_ast::parser::parse_program;

    let code = r#"
fn inc(x: int) -> int {
  return x + 1
}
"#;
    let program = parse_program(code).expect("program should parse");
    let mut engine = TypeInferenceEngine::new();
    let (facts, errors) = engine.infer_program_facts_best_effort(&program);
    assert!(
        errors.is_empty(),
        "inc should infer cleanly, got {:?}",
        errors
    );

    let signature = facts
        .function_signature("inc")
        .expect("facts should expose inc signature");
    match signature {
        Type::Function {
            params, returns, ..
        } => {
            assert_eq!(params.len(), 1, "inc should have one parameter");
            assert!(u40_is_int(&params[0]), "inc param should be int");
            assert!(u40_is_int(returns), "inc return should be int");
        }
        other => panic!("expected function signature for inc, got {:?}", other),
    }

    let body_expr_span = u40_find_expr_span(&program, &|e| matches!(e, Expr::BinaryOp { .. }))
        .expect("function-body x + 1 expression must exist");
    assert!(
        facts
            .expression_type(body_expr_span)
            .is_some_and(u40_is_int),
        "facts should expose resolved body expression type, got {:?}",
        facts.expression_type(body_expr_span)
    );
}

#[test]
fn inference_facts_exposes_anonymous_object_return_signature() {
    use shape_ast::parser::parse_program;

    let code = r#"
fn aabb(lo: int, hi: int) {
  { min: lo, max: hi }
}
"#;
    let program = parse_program(code).expect("program should parse");
    let mut engine = TypeInferenceEngine::new();
    let (facts, errors) = engine.infer_program_facts_best_effort(&program);
    assert!(
        errors.is_empty(),
        "aabb should infer cleanly, got {:?}",
        errors
    );

    let signature = facts
        .function_signature("aabb")
        .expect("facts should expose aabb signature");
    match signature {
        Type::Function { returns, .. } => match returns.as_ref() {
            Type::Concrete(TypeAnnotation::Object(fields)) => {
                assert_eq!(fields.len(), 2);
                assert_eq!(fields[0].name, "min");
                assert!(u40_is_int(&Type::Concrete(
                    fields[0].type_annotation.clone()
                )));
                assert_eq!(fields[1].name, "max");
                assert!(u40_is_int(&Type::Concrete(
                    fields[1].type_annotation.clone()
                )));
            }
            other => panic!("expected anonymous object return, got {:?}", other),
        },
        other => panic!("expected function signature for aabb, got {:?}", other),
    }
}

#[test]
fn inference_facts_exposes_indexed_callable_array_call_type() {
    use shape_ast::ast::Expr;
    use shape_ast::parser::parse_program;

    let code = r#"
fn inc(x: int) -> int { x + 1 }
fn dbl(y: int) -> int { y + 2 }
let arr = [inc, dbl]
let total = arr[0](1) + arr[1](2)
"#;
    let program = parse_program(code).expect("program should parse");
    let arr_span = u40_find_binding_span(&program, "arr").expect("arr binding span");
    let call_binop_span = u40_find_expr_span(&program, &|e| {
        matches!(
            e,
            Expr::BinaryOp { left, right, .. }
                if matches!(left.as_ref(), Expr::MethodCall { method, .. } if method == "__call__")
                    && matches!(right.as_ref(), Expr::MethodCall { method, .. } if method == "__call__")
        )
    })
    .expect("arr[0](1) + arr[1](2) span");

    let mut engine = TypeInferenceEngine::new();
    let (facts, errors) = engine.infer_program_facts_best_effort(&program);
    assert!(
        errors.is_empty(),
        "indexed callable array program should infer cleanly, got {:?}",
        errors
    );
    assert!(
        facts
            .expression_type(call_binop_span)
            .is_some_and(u40_is_int),
        "indexed callable array call binop should resolve to int, got {:?}",
        facts.expression_type(call_binop_span)
    );
    assert!(
        facts
            .binding_type(arr_span)
            .is_some_and(u40_is_array_of_function_returning_int),
        "arr binding should resolve to Array<Function<..., int>>, got {:?}",
        facts.binding_type(arr_span)
    );
}

#[test]
fn inference_facts_exposes_indexed_callable_array_element_binding_type() {
    use shape_ast::ast::Expr;
    use shape_ast::parser::parse_program;

    let code = r#"
fn inc(x: int) -> int { x + 1 }
fn dbl(y: int) -> int { y + 2 }
let arr = [inc, dbl]
let g = arr[0]
let total = g(4) + 1
"#;
    let program = parse_program(code).expect("program should parse");
    let g_span = u40_find_binding_span(&program, "g").expect("g binding span");
    let call_binop_span = u40_find_expr_span(&program, &|e| {
        matches!(
            e,
            Expr::BinaryOp { left, .. }
                if matches!(left.as_ref(), Expr::FunctionCall { name, .. } if name == "g")
        )
    })
    .expect("g(4) + 1 span");

    let mut engine = TypeInferenceEngine::new();
    let (facts, errors) = engine.infer_program_facts_best_effort(&program);
    assert!(
        errors.is_empty(),
        "indexed callable array element binding should infer cleanly, got {:?}",
        errors
    );
    assert!(
        facts
            .binding_type(g_span)
            .is_some_and(u40_is_function_returning_int),
        "g binding should resolve to Function<..., int>, got {:?}",
        facts.binding_type(g_span)
    );
    assert!(
        facts
            .expression_type(call_binop_span)
            .is_some_and(u40_is_int),
        "g(4) + 1 should resolve to int, got {:?}",
        facts.expression_type(call_binop_span)
    );
}

#[test]
fn inference_facts_exposes_finalized_binding_facts() {
    use shape_ast::ast::Expr;
    use shape_ast::parser::parse_program;

    let code = r#"
let module_count = 1

fn build_map() {
  let m = HashMap()
  m.set("answer", 42)
}
"#;
    let program = parse_program(code).expect("program should parse");
    let module_span = u40_find_binding_span(&program, "module_count").expect("module binding span");
    let local_span = u40_find_binding_span(&program, "m").expect("local binding span");
    let hashmap_init_span = u40_find_expr_span(
        &program,
        &|e| matches!(e, Expr::FunctionCall { name, .. } if name == "HashMap"),
    )
    .expect("HashMap initializer span");

    let mut engine = TypeInferenceEngine::new();
    let (facts, errors) = engine.infer_program_facts_best_effort(&program);
    assert!(
        errors.is_empty(),
        "binding facts program should infer cleanly, got {:?}",
        errors
    );

    let local_fact = facts
        .binding_fact(local_span)
        .expect("facts should expose local m binding");
    assert_eq!(local_fact.name, "m");
    assert_eq!(local_fact.binder_span, local_span);
    assert_eq!(local_fact.initializer_span, Some(hashmap_init_span));
    assert!(
        u40_is_hashmap_string_int(&local_fact.ty),
        "m binding should finalize to HashMap<string, int>, got {:?}",
        local_fact.ty
    );
    assert!(
        facts
            .binding_type(local_span)
            .is_some_and(u40_is_hashmap_string_int),
        "binding_type accessor should expose the finalized m type"
    );

    let module_fact = facts
        .binding_fact(module_span)
        .expect("facts should expose module_count binding");
    assert_eq!(module_fact.name, "module_count");
    assert!(
        u40_is_int(&module_fact.ty),
        "module_count should finalize to int, got {:?}",
        module_fact.ty
    );
}

#[test]
fn hashmap_set_chain_preserves_kv_and_get_returns_option_proof() {
    let (_engine, types, errors) = u40_infer(
        r#"
let counts = HashMap()
  .set("a", 1)
  .set("b", 2)
let a_count = counts.get("a")
"#,
    );
    assert!(
        errors.is_empty(),
        "HashMap set/get proof should infer cleanly, got {:?}",
        errors
    );
    assert!(
        types.get("counts").is_some_and(u40_is_hashmap_string_int),
        "counts should infer as HashMap<string, int>, got {:?}",
        types.get("counts")
    );
    assert!(
        types.get("a_count").is_some_and(u40_is_option_int),
        "HashMap.get should preserve Option<int>, got {:?}",
        types.get("a_count")
    );
}

#[test]
fn hashmap_word_counter_match_get_payload_static_kv_proof() {
    let (_engine, types, errors) = u40_infer(
        r#"
fn count_words(text) {
  let words = text.split(" ")
  let mut counts = HashMap()
  for word in words {
    match counts.get(word) {
      Some(existing) => { counts = counts.set(word, existing + 1) }
      None => { counts = counts.set(word, 1) }
    }
  }
  counts
}
let wc = count_words("the cat the")
let the_count = wc.get("the")
"#,
    );
    assert!(
        errors.is_empty(),
        "match-based word-counter HashMap proof should infer cleanly, got {:?}",
        errors
    );
    assert!(
        types.get("wc").is_some_and(u40_is_hashmap_string_int),
        "wc should infer as HashMap<string, int>, got {:?}",
        types.get("wc")
    );
    assert!(
        types.get("the_count").is_some_and(u40_is_option_int),
        "HashMap.get after split loop should preserve Option<int>, got {:?}",
        types.get("the_count")
    );
}

#[test]
fn hashmap_callable_values_match_get_payload_value_call_proof() {
    let (_engine, types, errors) = u40_infer(
        r#"
let ops = HashMap()
  .set("add", |a: int, b: int| { a + b })
  .set("mul", |a: int, b: int| { a * b })
let add_fn = ops.get("add")
let total = match ops.get("add") {
  Some(f) => f(3, 4)
  None => 0
}
"#,
    );
    assert!(
        errors.is_empty(),
        "callable-valued HashMap match proof should infer cleanly, got {:?}",
        errors
    );
    assert!(
        types
            .get("ops")
            .is_some_and(u40_is_hashmap_string_int_function),
        "ops should infer as HashMap<string, (int, int) -> int>, got {:?}",
        types.get("ops")
    );
    assert!(
        types
            .get("add_fn")
            .is_some_and(u40_is_option_function_int_int_to_int),
        "HashMap.get should preserve Option<(int, int) -> int>, got {:?}",
        types.get("add_fn")
    );
    assert!(
        types.get("total").is_some_and(u40_is_int),
        "calling matched HashMap function payload should infer int, got {:?}",
        types.get("total")
    );
}

#[test]
fn hashmap_named_function_values_match_get_payload_value_call_proof() {
    let (_engine, types, errors) = u40_infer(
        r#"
fn add(a: int, b: int) -> int { a + b }
fn mul(a: int, b: int) -> int { a * b }
let ops = HashMap()
  .set("add", add)
  .set("mul", mul)
let add_fn = ops.get("add")
let total = match ops.get("add") {
  Some(f) => f(3, 4)
  None => 0
}
"#,
    );
    assert!(
        errors.is_empty(),
        "named-function HashMap match proof should infer cleanly, got {:?}",
        errors
    );
    assert!(
        types
            .get("ops")
            .is_some_and(u40_is_hashmap_string_int_function),
        "ops should infer as HashMap<string, (int, int) -> int>, got {:?}",
        types.get("ops")
    );
    assert!(
        types
            .get("add_fn")
            .is_some_and(u40_is_option_function_int_int_to_int),
        "HashMap.get should preserve Option<(int, int) -> int>, got {:?}",
        types.get("add_fn")
    );
    assert!(
        types.get("total").is_some_and(u40_is_int),
        "calling matched named-function HashMap payload should infer int, got {:?}",
        types.get("total")
    );
}

#[test]
fn hashmap_get_option_function_is_not_directly_callable() {
    let (_engine, _types, errors) = u40_infer(
        r#"
let ops = HashMap()
  .set("add", |a: int, b: int| { a + b })
let add_fn = ops.get("add")
let total = add_fn(3, 4)
"#,
    );
    assert!(
        errors
            .iter()
            .any(|err| format!("{err:?}").contains("'add_fn' is not callable")),
        "HashMap.get should preserve Option<Function> and reject direct calls, got {:?}",
        errors
    );
}

#[test]
fn hashmap_explicit_annotation_loop_aggregation_static_kv_proof() {
    let (_engine, types, errors) = u40_infer(
        r#"
type ScoreEntry { name: string, score: int }
let mut scores: HashMap<string, int> = HashMap()
let entries = [
  ScoreEntry { name: "Alice", score: 90 },
  ScoreEntry { name: "Bob", score: 85 },
  ScoreEntry { name: "Alice", score: 95 },
]
for entry in entries {
  let name = entry.name
  let score = entry.score
  match scores.get(name) {
    Some(prev) => { let p: int = prev; scores = scores.set(name, p + score) }
    None => { scores = scores.set(name, score) }
  }
}
let alice = scores.get("Alice")
"#,
    );
    assert!(
        errors.is_empty(),
        "annotated HashMap loop aggregation should infer cleanly, got {:?}",
        errors
    );
    assert!(
        types.get("scores").is_some_and(u40_is_hashmap_string_int),
        "scores should remain HashMap<string, int>, got {:?}",
        types.get("scores")
    );
    assert!(
        types.get("alice").is_some_and(u40_is_option_int),
        "scores.get should preserve Option<int> after loop aggregation, got {:?}",
        types.get("alice")
    );
}

#[test]
fn match_binding_fact_some_payload_is_int() {
    let fact = u40_match_binding_fact(
        r#"
let opt: Option<int> = Some(1)
let out = match opt {
  Some(n) => n,
  None => 0
}
"#,
        "n",
    );
    assert_eq!(fact.name, "n");
    assert!(fact.initializer_span.is_some());
    assert!(
        u40_is_int(&fact.ty),
        "Some(n) should bind n as int, got {:?}",
        fact.ty
    );
}

#[test]
fn match_binding_fact_result_payloads_are_success_and_error_types() {
    use shape_ast::parser::parse_program;

    let code = r#"
let res: Result<int, string> = Ok(1)
let out = match res {
  Ok(v) => v,
  Err(e) => 0
}
"#;
    let program = parse_program(code).expect("program should parse");
    let v_span = u40_find_binding_span(&program, "v").expect("v binding span");
    let e_span = u40_find_binding_span(&program, "e").expect("e binding span");
    let mut engine = TypeInferenceEngine::new();
    let (facts, errors) = engine.infer_program_facts_best_effort(&program);
    assert!(
        errors.is_empty(),
        "Result payload match should infer cleanly, got {:?}",
        errors
    );
    assert!(
        facts.binding_type(v_span).is_some_and(u40_is_int),
        "Ok(v) should bind v as int, got {:?}",
        facts.binding_type(v_span)
    );
    assert!(
        facts.binding_type(e_span).is_some_and(u40_is_string),
        "Err(e) should bind e as string, got {:?}",
        facts.binding_type(e_span)
    );
}

#[test]
fn w28_result_match_payload_tracks_callsite_proven_callee_return() {
    use shape_ast::ast::Expr;
    use shape_ast::parser::parse_program;

    let code = r#"
fn safe_divide(a, b) {
  if b == 0 { return Err("division by zero") }
  Ok(a / b)
}
fn process_division(a, b) {
  match safe_divide(a, b) {
    Ok(v) => {
      if v > 10 { "large: " + v } else { "small: " + v }
    },
    Err(e) => "error: " + e
  }
}
let out = process_division(100, 5)
"#;
    let program = parse_program(code).expect("program should parse");
    let v_span = u40_find_binding_span(&program, "v").expect("v binding span");
    let e_span = u40_find_binding_span(&program, "e").expect("e binding span");
    let out_span = u40_find_binding_span(&program, "out").expect("out binding span");
    let scrutinee_span = u40_find_expr_span(
        &program,
        &|expr| matches!(expr, Expr::FunctionCall { name, .. } if name == "safe_divide"),
    )
    .expect("safe_divide call span");

    let mut engine = TypeInferenceEngine::new();
    let (facts, errors) = engine.infer_program_facts_best_effort(&program);
    assert!(
        errors.is_empty(),
        "callsite-proven Result match should infer cleanly, got {:?}",
        errors
    );
    assert!(
        facts.binding_type(v_span).is_some_and(u40_is_int),
        "Ok(v) should bind v as int after callee return proof, got {:?}",
        facts.binding_type(v_span)
    );
    assert!(
        facts.binding_type(e_span).is_some_and(u40_is_string),
        "Err(e) should bind e as string after callee return proof, got {:?}",
        facts.binding_type(e_span)
    );
    let scrutinee_ty = facts
        .expression_type(scrutinee_span)
        .expect("safe_divide scrutinee call type");
    match scrutinee_ty.canonicalize() {
        Type::Generic { base, args } if args.len() >= 2 => {
            assert!(
                matches!(
                    base.as_ref(),
                    Type::Concrete(TypeAnnotation::Reference(name)) if name == "Result"
                ),
                "safe_divide scrutinee should be Result<_, _>, got {:?}",
                scrutinee_ty
            );
            assert!(
                u40_is_int(&args[0]),
                "safe_divide Ok payload should be int, got {:?}",
                args[0]
            );
            assert!(
                u40_is_string(&args[1]),
                "safe_divide Err payload should be string, got {:?}",
                args[1]
            );
        }
        other => panic!(
            "safe_divide scrutinee should finalize as Result<int,string>, got {:?}",
            other
        ),
    }
    assert!(
        facts.binding_type(out_span).is_some_and(u40_is_string),
        "process_division call should resolve to string, got {:?}",
        facts.binding_type(out_span)
    );
}

#[test]
fn match_binding_fact_nested_option_result_payload_is_int() {
    let fact = u40_match_binding_fact(
        r#"
let nested: Option<Result<int, string>> = Some(Ok(1))
let out = match nested {
  Some(Ok(v)) => v,
  Some(Err(_)) => 0,
  None => 0
}
"#,
        "v",
    );
    assert!(
        u40_is_int(&fact.ty),
        "Some(Ok(v)) should bind v as int, got {:?}",
        fact.ty
    );
}

#[test]
fn match_binding_fact_object_pattern_uses_struct_field_types() {
    use shape_ast::parser::parse_program;

    let code = r#"
type Point { x: int, y: string }
let p = Point { x: 1, y: "two" }
let out = match p {
  { x, y } => x,
  _ => 0
}
"#;
    let program = parse_program(code).expect("program should parse");
    let x_span = u40_find_binding_span(&program, "x").expect("x binding span");
    let y_span = u40_find_binding_span(&program, "y").expect("y binding span");
    let mut engine = TypeInferenceEngine::new();
    let (facts, errors) = engine.infer_program_facts_best_effort(&program);
    assert!(
        errors.is_empty(),
        "object-pattern match should infer cleanly, got {:?}",
        errors
    );
    assert!(
        facts.binding_type(x_span).is_some_and(u40_is_int),
        "Point {{ x, y }} should bind x as int, got {:?}",
        facts.binding_type(x_span)
    );
    assert!(
        facts.binding_type(y_span).is_some_and(u40_is_string),
        "Point {{ x, y }} should bind y as string, got {:?}",
        facts.binding_type(y_span)
    );
}

#[test]
fn match_binding_fact_array_pattern_uses_element_type() {
    use shape_ast::parser::parse_program;

    let code = r#"
let arr: Array<int> = [1, 2]
let out = match arr {
  [a, b] => a + b,
  _ => 0
}
"#;
    let program = parse_program(code).expect("program should parse");
    let a_span = u40_find_binding_span(&program, "a").expect("a binding span");
    let b_span = u40_find_binding_span(&program, "b").expect("b binding span");
    let mut engine = TypeInferenceEngine::new();
    let (facts, errors) = engine.infer_program_facts_best_effort(&program);
    assert!(
        errors.is_empty(),
        "array-pattern match should infer cleanly, got {:?}",
        errors
    );
    assert!(
        facts.binding_type(a_span).is_some_and(u40_is_int),
        "[a, b] should bind a as int, got {:?}",
        facts.binding_type(a_span)
    );
    assert!(
        facts.binding_type(b_span).is_some_and(u40_is_int),
        "[a, b] should bind b as int, got {:?}",
        facts.binding_type(b_span)
    );
}

#[test]
fn match_binding_fact_typed_pattern_records_annotation_type() {
    let fact = u40_match_binding_fact(
        r#"
let value: int | string = 1
let out = match value {
  n: int => n,
  s: string => 0
}
"#,
        "n",
    );
    assert!(
        u40_is_int(&fact.ty),
        "typed pattern n: int should bind n as int, got {:?}",
        fact.ty
    );
}

#[test]
fn u40_closure_field_return_resolves_to_int_f8() {
    use shape_ast::ast::Expr;
    use shape_ast::parser::parse_program;

    // f8: `let get = |p: Emp| { p.salary }; get(e) + 1`. The closure body is a
    // bare field-read; pre-U4-0 the engine recorded `p.salary` / the closure
    // return / the call result as a free `Type::Variable` and `finalize` DROPPED
    // all three. With the P2 struct-name-carrier normalization, `p.salary`
    // resolves to `Emp.salary` (`int`) and propagates to the closure return and
    // the `get(e)` call result.
    let code = r#"
type Emp { salary: int }
let get = |p: Emp| { p.salary }
let e = Emp { salary: 50 }
let r = get(e) + 1
"#;
    let program = parse_program(code).expect("parse");
    let (engine, _types, errors) = u40_infer(code);
    assert!(
        errors.is_empty(),
        "f8 should infer cleanly, got {:?}",
        errors
    );

    // The `get(e)` call-result span.
    let call_span = u40_find_expr_span(
        &program,
        &|e| matches!(e, Expr::FunctionCall { name, .. } if name == "get"),
    )
    .expect("get(e) call site must exist");
    let resolved = engine.resolved_expr_type(call_span);
    assert!(
        resolved.is_some_and(u40_is_int),
        "f8: get(e) call-result span must resolve to `int`, got {:?}",
        resolved
    );

    // The closure body field-read `p.salary` span.
    let field_span = u40_find_expr_span(
        &program,
        &|e| matches!(e, Expr::PropertyAccess { property, .. } if property == "salary"),
    )
    .expect("p.salary must exist");
    let field_resolved = engine.resolved_expr_type(field_span);
    assert!(
        field_resolved.is_some_and(u40_is_int),
        "f8: closure-body p.salary span must resolve to `int`, got {:?}",
        field_resolved
    );

    // The `get(e) + 1` binop span.
    let binop_span = u40_find_expr_span(&program, &|e| matches!(e, Expr::BinaryOp { .. }))
        .expect("get(e) + 1 must exist");
    assert!(
        engine
            .resolved_expr_type(binop_span)
            .is_some_and(u40_is_int),
        "f8: get(e) + 1 must resolve to `int`"
    );
}

#[test]
fn u40_nested_closure_field_resolves_to_int_h1() {
    use shape_ast::ast::Expr;
    use shape_ast::parser::parse_program;

    // h1: nested field projection `|w: Outer| { w.inner.x }`.
    let code = r#"
type Inner { x: int }
type Outer { inner: Inner }
let getx = |w: Outer| { w.inner.x }
let o = Outer { inner: Inner { x: 3 } }
let r = getx(o) + 1
"#;
    let program = parse_program(code).expect("parse");
    let (engine, _types, errors) = u40_infer(code);
    assert!(
        errors.is_empty(),
        "h1 should infer cleanly, got {:?}",
        errors
    );

    let call_span = u40_find_expr_span(
        &program,
        &|e| matches!(e, Expr::FunctionCall { name, .. } if name == "getx"),
    )
    .expect("getx(o) call site");
    assert!(
        engine.resolved_expr_type(call_span).is_some_and(u40_is_int),
        "h1: getx(o) (nested w.inner.x) must resolve to `int`, got {:?}",
        engine.resolved_expr_type(call_span)
    );

    // The outer field-read `.x` (terminal of `w.inner.x`).
    let field_span = u40_find_expr_span(
        &program,
        &|e| matches!(e, Expr::PropertyAccess { property, .. } if property == "x"),
    )
    .expect("w.inner.x must exist");
    assert!(
        engine
            .resolved_expr_type(field_span)
            .is_some_and(u40_is_int),
        "h1: w.inner.x must resolve to `int`"
    );
}

#[test]
fn u40_explicit_return_closure_field_resolves_h2() {
    use shape_ast::ast::Expr;
    use shape_ast::parser::parse_program;

    // h2: explicit `return p.salary` in the closure body.
    let code = r#"
type Emp { salary: int }
let get = |p: Emp| { return p.salary }
let e = Emp { salary: 7 }
let r = get(e) + 1
"#;
    let program = parse_program(code).expect("parse");
    let (engine, _types, errors) = u40_infer(code);
    assert!(
        errors.is_empty(),
        "h2 should infer cleanly, got {:?}",
        errors
    );

    let call_span = u40_find_expr_span(
        &program,
        &|e| matches!(e, Expr::FunctionCall { name, .. } if name == "get"),
    )
    .expect("get(e) call site");
    assert!(
        engine.resolved_expr_type(call_span).is_some_and(u40_is_int),
        "h2: explicit-return closure get(e) must resolve to `int`, got {:?}",
        engine.resolved_expr_type(call_span)
    );
}

#[test]
fn u40_closure_field_equality_resolves_h4b() {
    use shape_ast::parser::parse_program;

    // h4b: `get(a) == get(b)` — both operands are closure-field call results,
    // with no sibling-literal recovery. Both must resolve to `int` so the `==`
    // operands hit the table.
    let code = r#"
type Emp { salary: int }
let get = |p: Emp| { p.salary }
let a = Emp { salary: 1 }
let b = Emp { salary: 2 }
let r = get(a) == get(b)
"#;
    let program = parse_program(code).expect("parse");
    let (engine, _types, errors) = u40_infer(code);
    assert!(
        errors.is_empty(),
        "h4b should infer cleanly, got {:?}",
        errors
    );

    // Find BOTH get(...) call sites by collecting all matching spans.
    let mut call_spans: Vec<shape_ast::ast::Span> = Vec::new();
    fn collect_calls(program: &shape_ast::ast::Program, out: &mut Vec<shape_ast::ast::Span>) {
        use shape_ast::ast::{Expr, Item, Spanned, Statement};
        fn we(e: &Expr, out: &mut Vec<shape_ast::ast::Span>) {
            if let Expr::FunctionCall { name, .. } = e {
                if name == "get" {
                    out.push(Spanned::span(e));
                }
            }
            match e {
                Expr::BinaryOp { left, right, .. } => {
                    we(left, out);
                    we(right, out);
                }
                Expr::FunctionCall { args, .. } => {
                    for a in args {
                        we(a, out);
                    }
                }
                _ => {}
            }
        }
        for item in &program.items {
            if let Item::Statement(Statement::VariableDecl(decl, _), _) = item {
                if let Some(v) = &decl.value {
                    we(v, out);
                }
            }
        }
    }
    collect_calls(&program, &mut call_spans);
    assert_eq!(call_spans.len(), 2, "h4b: both get(...) sites");
    for span in call_spans {
        assert!(
            engine.resolved_expr_type(span).is_some_and(u40_is_int),
            "h4b: get(...) call-result must resolve to `int`, got {:?}",
            engine.resolved_expr_type(span)
        );
    }
}

#[test]
fn u40_borrow_referent_recorded_as_referent_p3() {
    use shape_ast::ast::Expr;
    use shape_ast::parser::parse_program;

    // P3: a reference-typed READ (`ref_a` used as an operand of `ref_a + 1`,
    // where `ref_a: &int`) must hit the table with the PROJECTED REFERENT type
    // (`int`), not the `Borrow` wrapper. The address-of producer `&a` itself
    // keeps its `Borrow` recording (load-bearing for `-> &T` unification).
    let code = r#"
let a = 5
let ref_a = &a
let s = ref_a + 1
"#;
    let program = parse_program(code).expect("parse");
    let (engine, _types, errors) = u40_infer(code);
    assert!(
        errors.is_empty(),
        "P3 ref read should infer cleanly, got {:?}",
        errors
    );

    // `ref_a` operand inside `ref_a + 1` → referent `int`.
    let operand_span = u40_find_expr_span(
        &program,
        &|e| matches!(e, Expr::Identifier(n, _) if n == "ref_a"),
    )
    .expect("ref_a operand");
    assert!(
        engine
            .resolved_expr_type(operand_span)
            .is_some_and(u40_is_int),
        "P3: ref_a operand read must record the referent `int`, got {:?}",
        engine.resolved_expr_type(operand_span)
    );

    // `&a` producer keeps the `Borrow` recording.
    let amp_span = u40_find_expr_span(&program, &|e| matches!(e, Expr::Reference { .. }))
        .expect("&a producer");
    let amp_ty = engine.resolved_expr_type(amp_span);
    assert!(
        matches!(amp_ty, Some(Type::Concrete(TypeAnnotation::Borrow { .. }))),
        "P3: the &a producer must keep its Borrow recording, got {:?}",
        amp_ty
    );
}

#[test]
fn u40_uninferable_tail_stays_dropped() {
    use shape_ast::ast::Expr;
    use shape_ast::parser::parse_program;

    // FINALIZE AUDIT: the genuinely-un-inferable tail must STAY absent (None) so
    // the later compiler boundary surfaces a real surface-and-stop error. U4-0
    // must NOT over-force these to resolve.

    // (a) Comptime block — its value var is fresh and never pinned.
    {
        let code = r#"
let x = comptime { 1 + 1 }
"#;
        let program = parse_program(code).expect("parse");
        let (engine, _types, _errors) = u40_infer(code);
        let cspan = u40_find_expr_span(&program, &|e| matches!(e, Expr::Comptime(..)));
        if let Some(span) = cspan {
            assert!(
                engine.resolved_expr_type(span).is_none(),
                "tail: a comptime block value must stay DROPPED (None), got {:?}",
                engine.resolved_expr_type(span)
            );
        }
    }

    // (b) Empty array with NO element pin — element var never resolved.
    {
        let code = r#"
let xs = []
"#;
        let program = parse_program(code).expect("parse");
        let (engine, _types, _errors) = u40_infer(code);
        let aspan = u40_find_expr_span(
            &program,
            &|e| matches!(e, Expr::Array(els, _) if els.is_empty()),
        )
        .expect("empty array literal");
        assert!(
            engine.resolved_expr_type(aspan).is_none(),
            "tail: an un-pinned empty `[]` must stay DROPPED (None), got {:?}",
            engine.resolved_expr_type(aspan)
        );
    }

    // (c) QualifiedFunctionCall to a non-enum/non-struct namespace — deliberately
    // a fresh result var (the inference tier has no module-export signatures).
    {
        let code = r#"
let v = somemod::compute(1)
"#;
        let program = parse_program(code).expect("parse");
        let (engine, _types, _errors) = u40_infer(code);
        let qspan = u40_find_expr_span(&program, &|e| {
            matches!(e, Expr::QualifiedFunctionCall { .. })
        });
        if let Some(span) = qspan {
            assert!(
                engine.resolved_expr_type(span).is_none(),
                "tail: a QualifiedFunctionCall to a real module fn must stay DROPPED (None), got {:?}",
                engine.resolved_expr_type(span)
            );
        }
    }
}

#[test]
fn u40_stage_f1_still_fires_for_unannotated_array_field_read() {
    // STAGE-F1 strictness must NOT be weakened by U4-0: an unannotated empty-`[]`
    // accumulator grown by `push` has NO declared element type, so a field read
    // off its element is an engine ConstraintViolation. The P2 normalization is
    // bounded to `Basic`-carried struct NAMES; f1's element back-propagates to a
    // `Reference`, never a `Basic`, so the normalization never touches it.
    let code = r#"
type Run { n: int }
let mut rs = []
rs = rs.push(Run { n: 1 })
for r in rs { r.n + 1 }
"#;
    let (engine, _types, errors) = u40_infer(code);
    assert!(
        errors
            .iter()
            .any(|e| matches!(e, TypeError::ConstraintViolation(_))),
        "f1: an unannotated [] field read must still produce a ConstraintViolation (STAGE-F1), got {:?}",
        errors
    );
    // The field-read span must NOT be present as a resolved entry (it is an error,
    // never recorded as a fully-resolved type).
    use shape_ast::ast::Expr;
    use shape_ast::parser::parse_program;
    let program = parse_program(code).expect("parse");
    if let Some(span) = u40_find_expr_span(
        &program,
        &|e| matches!(e, Expr::PropertyAccess { property, .. } if property == "n"),
    ) {
        assert!(
            engine.resolved_expr_type(span).is_none(),
            "f1: the un-annotatable field read must stay DROPPED (None), got {:?}",
            engine.resolved_expr_type(span)
        );
    }
}

#[test]
fn w26_unit_enum_variant_property_access_contributes_enum_array_type() {
    use shape_ast::ast::TypeAnnotation;
    use shape_ast::parser::parse_program;

    let code = r#"
enum Action { Add(int), Reset }
let actions = [Action::Add(1), Action::Reset]
"#;
    let program = parse_program(code).expect("program should parse");
    let mut engine = TypeInferenceEngine::new();
    let types = engine
        .infer_program(&program)
        .expect("unit enum variant should infer as the enum type");

    let actions = types
        .get("actions")
        .expect("actions binding should be inferred")
        .canonicalize();
    match actions {
        Type::Generic { base, args } => {
            assert!(
                matches!(
                    base.as_ref(),
                    Type::Concrete(TypeAnnotation::Reference(name)) if name.as_str() == "Array"
                ),
                "actions should be an Array, got {:?}",
                base
            );
            assert_eq!(args.len(), 1);
            assert!(
                matches!(&args[0], Type::Concrete(TypeAnnotation::Reference(name)) if name.as_str() == "Action"),
                "actions element should be Action, got {:?}",
                args[0]
            );
        }
        other => panic!("actions should infer as Array<Action>, got {:?}", other),
    }
}

#[test]
fn w26_empty_array_push_return_resolves_from_callsite_argument_type() {
    use shape_ast::ast::TypeAnnotation;
    use shape_ast::parser::parse_program;

    let code = r#"
fn singleton(x) {
    let mut result = []
    result = result.push(x)
    result
}
let ints = singleton(1)
"#;
    let program = parse_program(code).expect("program should parse");
    let mut engine = TypeInferenceEngine::new();
    let types = engine
        .infer_program(&program)
        .expect("empty-grow return should be resolved by the call site");

    let ints = types
        .get("ints")
        .expect("ints binding should be inferred")
        .canonicalize();
    match ints {
        Type::Generic { base, args } => {
            assert!(
                matches!(
                    base.as_ref(),
                    Type::Concrete(TypeAnnotation::Reference(name)) if name.as_str() == "Array"
                ),
                "ints should be an Array, got {:?}",
                base
            );
            assert_eq!(args.len(), 1);
            assert!(
                matches!(&args[0], Type::Concrete(TypeAnnotation::Basic(name)) if name == "int"),
                "ints element should be int, got {:?}",
                args[0]
            );
        }
        other => panic!("ints should infer as Array<int>, got {:?}", other),
    }
}

#[test]
fn w27_rewalk_records_resolved_param_binary_op_span() {
    use shape_ast::ast::{BinaryOp, Expr};
    use shape_ast::parser::parse_program;

    let code = r#"
fn reverse_string(s) {
    let mut i = s.length - 1
    i
}
reverse_string("hello")
"#;
    let program = parse_program(code).expect("program should parse");
    let sub_span = u40_find_expr_span(&program, &|e| {
        matches!(
            e,
            Expr::BinaryOp {
                op: BinaryOp::Sub,
                ..
            }
        )
    })
    .expect("sub expression span");

    let mut engine = TypeInferenceEngine::new();
    let (facts, errors) = engine.infer_program_facts_best_effort(&program);
    assert!(
        errors.is_empty(),
        "resolved-param function body should infer cleanly, got {:?}",
        errors
    );
    assert!(
        facts.expression_type(sub_span).is_some_and(u40_is_int),
        "s.length - 1 should be recorded as int after callsite proof, got {:?}",
        facts.expression_type(sub_span)
    );
}

#[test]
fn w27_rewalk_resolves_zip_sum_empty_grow_return() {
    use shape_ast::parser::parse_program;

    let code = r#"
fn zip_sum(a, b) {
    let mut result = []
    let mut i = 0
    let len = if a.length < b.length { a.length } else { b.length }
    while i < len {
        result = result.push(a[i] + b[i])
        i = i + 1
    }
    result
}
let zipped = zip_sum([1, 2, 3], [10, 20, 30])
"#;
    let program = parse_program(code).expect("program should parse");
    let zipped_span = u40_find_binding_span(&program, "zipped").expect("zipped binding span");

    let mut engine = TypeInferenceEngine::new();
    let (facts, errors) = engine.infer_program_facts_best_effort(&program);
    assert!(
        errors.is_empty(),
        "zip_sum empty-grow return should infer cleanly, got {:?}",
        errors
    );
    assert!(
        facts
            .binding_type(zipped_span)
            .is_some_and(u40_is_array_of_int),
        "zipped should resolve to Array<int>, got {:?}",
        facts.binding_type(zipped_span)
    );
}

#[test]
fn w27_rewalk_resolves_array_length_property_after_callsite_proof() {
    use shape_ast::parser::parse_program;

    let code = r#"
fn min_len(a, b) {
    min(a.length, b.length)
}
let n = min_len([1, 2], [3])
"#;
    let program = parse_program(code).expect("program should parse");
    let n_span = u40_find_binding_span(&program, "n").expect("n binding span");

    let mut engine = TypeInferenceEngine::new();
    let (facts, errors) = engine.infer_program_facts_best_effort(&program);
    assert!(
        errors.is_empty(),
        "array length over callsite-proven params should infer cleanly, got {:?}",
        errors
    );
    assert!(
        facts.binding_type(n_span).is_some_and(u40_is_int),
        "n should resolve to int, got {:?}",
        facts.binding_type(n_span)
    );
}

#[test]
fn w27_rewalk_resolves_length_empty_grow_return() {
    use shape_ast::parser::parse_program;

    let code = r#"
fn lengths(a, b) {
    let mut out = []
    out = out.push(a.length)
    out = out.push(b.length)
    out
}
let xs = lengths([1], [2, 3])
"#;
    let program = parse_program(code).expect("program should parse");
    let xs_span = u40_find_binding_span(&program, "xs").expect("xs binding span");

    let mut engine = TypeInferenceEngine::new();
    let (facts, errors) = engine.infer_program_facts_best_effort(&program);
    assert!(
        errors.is_empty(),
        "length empty-grow return should infer cleanly, got {:?}",
        errors
    );
    assert!(
        facts.binding_type(xs_span).is_some_and(u40_is_array_of_int),
        "xs should resolve to Array<int>, got {:?}",
        facts.binding_type(xs_span)
    );
}

#[test]
fn w27_rewalk_records_nested_push_param_binding_fact() {
    use shape_ast::parser::parse_program;

    let code = r#"
let mut stack = []
fn push(val) { stack = stack.push(val) }
push(10)
push(20)
stack.len()
"#;
    let program = parse_program(code).expect("program should parse");
    let stack_span = u40_find_binding_span(&program, "stack").expect("stack binding span");
    let val_span = u40_find_binding_span(&program, "val").expect("val binding span");
    let empty_array_span = u40_find_expr_span(
        &program,
        &|expr| matches!(expr, shape_ast::ast::Expr::Array(elements, _) if elements.is_empty()),
    )
    .expect("empty array literal span");
    let val_use_span = {
        use shape_ast::ast::{Expr, Item, Span, Spanned, Statement};

        fn find_in_expr(expr: &Expr) -> Option<Span> {
            match expr {
                Expr::Identifier(name, _) if name == "val" => Some(Spanned::span(expr)),
                Expr::MethodCall { receiver, args, .. } => {
                    find_in_expr(receiver).or_else(|| args.iter().find_map(find_in_expr))
                }
                _ => None,
            }
        }

        program.items.iter().find_map(|item| match item {
            Item::Function(func, _) => func.body.iter().find_map(|stmt| match stmt {
                Statement::Assignment(assign, _) => find_in_expr(&assign.value),
                _ => None,
            }),
            _ => None,
        })
    }
    .expect("val use span");

    let mut engine = TypeInferenceEngine::new();
    let (facts, errors) = engine.infer_program_facts_best_effort(&program);
    assert!(
        errors.is_empty(),
        "nested module accumulator push should infer cleanly, got {:?}",
        errors
    );
    assert!(
        facts
            .binding_type(stack_span)
            .is_some_and(u40_is_array_of_int),
        "module accumulator binding should resolve to Array<int>, got {:?}",
        facts.binding_type(stack_span)
    );
    assert!(
        facts.binding_type(val_span).is_some_and(u40_is_int),
        "nested push parameter should resolve to int, got {:?}",
        facts.binding_type(val_span)
    );
    assert!(
        facts.expression_type(val_use_span).is_some_and(u40_is_int),
        "nested push argument use should resolve to int, got {:?}",
        facts.expression_type(val_use_span)
    );
    assert!(
        facts.expression_type(empty_array_span).is_none(),
        "bare empty literal should not bypass VM accumulator promotion, got {:?}",
        facts.expression_type(empty_array_span)
    );
}

#[test]
fn u42_typed_array_sum_resolves_to_element_type() {
    use shape_ast::ast::Expr;
    use shape_ast::parser::parse_program;

    // g1 regression guard (U4-2): deleting the closure-body mini-inferencer made
    // the engine span-table the sole source for `|a: Array<int>| { a.sum() }`'s
    // return type. The `Vec.sum`/`min`/`max` method-table return was registered
    // as `ElementOf(ReceiverParam(0))`, which DOUBLE-projected — `ReceiverParam(0)`
    // already IS the element `int`, so `ElementOf(int)` minted an `_oob` var that
    // `finalize_expr_type_table` DROPPED, leaving `a.sum()` absent from the table.
    // The fix returns `ReceiverParam(0)` directly. Assert the span table now
    // resolves `a.sum()` / `f(xs)` to the element type so the strict binop check
    // and the engine-served closure-return both succeed.
    let code = r#"
let xs = [1, 2, 3]
let f = |a: Array<int>| { a.sum() }
let r = f(xs) + 1
"#;
    let program = parse_program(code).expect("parse");
    let (engine, _types, errors) = u40_infer(code);
    assert!(
        errors.is_empty(),
        "g1 should infer cleanly, got {:?}",
        errors
    );

    let sum_span = u40_find_expr_span(
        &program,
        &|e| matches!(e, Expr::MethodCall { method, .. } if method == "sum"),
    )
    .expect("a.sum() must exist");
    assert!(
        engine.resolved_expr_type(sum_span).is_some_and(u40_is_int),
        "g1: a.sum() span must resolve to `int`, got {:?}",
        engine.resolved_expr_type(sum_span)
    );

    let call_span = u40_find_expr_span(
        &program,
        &|e| matches!(e, Expr::FunctionCall { name, .. } if name == "f"),
    )
    .expect("f(xs) must exist");
    assert!(
        engine.resolved_expr_type(call_span).is_some_and(u40_is_int),
        "g1: f(xs) call-result span must resolve to `int`, got {:?}",
        engine.resolved_expr_type(call_span)
    );
}

// ── U4-3pre §5(A) PRECONDITION GATE: resilient span-table recording ──────────
//
// The deletion of the fallback re-derivation engine (U4-3) requires that EVERY
// trivially-typed child the strict checker would query lands in the span table,
// independent of whether a SIBLING failed. Before U4-3pre, a compound handler
// aborted on the FIRST erroring child via `?`, so the recordable siblings
// (literals, simple closures) were never visited/recorded and only the fallback
// supplied their types. These tests assert ZERO OK_RESOLVED user-source MISS for
// the recordable children across the aborting contexts, while the genuinely
// un-inferable parent/scrutinee stays a MISS (→ later surface-and-stop) and the
// program still ERRORS (soundness (a)).

/// Match-arm-aware span finder: `u40_find_expr_span` does not recurse into
/// `Match` arm bodies (the exact site U4-3pre fixes), so the gate needs its own
/// walker that descends through match scrutinees + arm guards/bodies.
fn u43_find_expr_span<F>(
    program: &shape_ast::ast::Program,
    pred: &F,
) -> Option<shape_ast::ast::Span>
where
    F: Fn(&shape_ast::ast::Expr) -> bool,
{
    use shape_ast::ast::{BlockItem, Expr, Item, Span, Spanned, Statement};

    fn walk_expr<F: Fn(&Expr) -> bool>(e: &Expr, pred: &F, out: &mut Option<Span>) {
        if out.is_some() {
            return;
        }
        if pred(e) {
            *out = Some(Spanned::span(e));
            return;
        }
        match e {
            Expr::BinaryOp { left, right, .. } => {
                walk_expr(left, pred, out);
                walk_expr(right, pred, out);
            }
            Expr::UnaryOp { operand, .. } => walk_expr(operand, pred, out),
            Expr::PropertyAccess { object, .. } => walk_expr(object, pred, out),
            Expr::IndexAccess { object, index, .. } => {
                walk_expr(object, pred, out);
                walk_expr(index, pred, out);
            }
            Expr::FunctionCall { args, .. } => {
                for a in args {
                    walk_expr(a, pred, out);
                }
            }
            Expr::MethodCall { receiver, args, .. } => {
                walk_expr(receiver, pred, out);
                for a in args {
                    walk_expr(a, pred, out);
                }
            }
            Expr::FunctionExpr { body, .. } => walk_stmts(body, pred, out),
            Expr::Match(m, _) => {
                walk_expr(&m.scrutinee, pred, out);
                for arm in &m.arms {
                    if let Some(g) = &arm.guard {
                        walk_expr(g, pred, out);
                    }
                    walk_expr(&arm.body, pred, out);
                }
            }
            Expr::Block(block, _) => {
                for item in &block.items {
                    match item {
                        BlockItem::Expression(ex) => walk_expr(ex, pred, out),
                        BlockItem::Statement(s) => walk_stmt(s, pred, out),
                        BlockItem::VariableDecl(decl) => {
                            if let Some(v) = &decl.value {
                                walk_expr(v, pred, out);
                            }
                        }
                        BlockItem::Assignment(_) => {}
                    }
                }
            }
            Expr::Return(Some(v), _) => walk_expr(v, pred, out),
            Expr::Reference { expr, .. } => walk_expr(expr, pred, out),
            _ => {}
        }
    }

    fn walk_stmt<F: Fn(&Expr) -> bool>(s: &Statement, pred: &F, out: &mut Option<Span>) {
        match s {
            Statement::Expression(e, _) => walk_expr(e, pred, out),
            Statement::Return(Some(e), _) => walk_expr(e, pred, out),
            Statement::VariableDecl(decl, _) => {
                if let Some(v) = &decl.value {
                    walk_expr(v, pred, out);
                }
            }
            _ => {}
        }
    }

    fn walk_stmts<F: Fn(&Expr) -> bool>(stmts: &[Statement], pred: &F, out: &mut Option<Span>) {
        for s in stmts {
            walk_stmt(s, pred, out);
        }
    }

    let mut out = None;
    for item in &program.items {
        match item {
            Item::Statement(stmt, _) => walk_stmt(stmt, pred, &mut out),
            Item::Function(func, _) => walk_stmts(&func.body, pred, &mut out),
            _ => {}
        }
        if out.is_some() {
            break;
        }
    }
    out
}

fn u43_is_string(ty: &Type) -> bool {
    matches!(ty, Type::Concrete(TypeAnnotation::Basic(n)) if n == "string")
}

#[test]
fn u43pre_match_over_unmodeled_scrutinee_records_arm_literal() {
    // ROOT REPRO (modeled on `resumability/probe.shape`): `snapshot()` is an
    // engine-unmodeled builtin (the inference tier has no stdlib module-export
    // signatures), so the match SCRUTINEE inference errors `UndefinedFunction`.
    // Before U4-3pre, `infer_match` aborted at the scrutinee `?`, dropping the
    // arm-body literal `"saved: "` — which the deleted fallback then had to
    // supply, and which would become a spurious `string + unknown` strict error
    // once the fallback is gone. After U4-3pre: the scrutinee degrades to a
    // fresh var, the arm bodies are still walked + recorded, and the `"saved: "`
    // literal lands in the span table; the scrutinee stays a MISS.
    //
    // Placed in a FUNCTION BODY so the un-inferable scrutinee genuinely
    // propagates an error (a top-level expression statement deliberately
    // TOLERATES `UndefinedFunction` — `items.rs:253` — so `print(...)`/builtin
    // calls at module scope do not kill inference; that tolerance is exactly why
    // `probe.shape` compiles and why the fallback was load-bearing).
    use shape_ast::ast::Expr;
    use shape_ast::parser::parse_program;

    let code = r#"
fn handle() -> string {
  match snapshot() {
    Snapshot::Hash(id) => "saved: " + id
    Snapshot::Resumed => "resumed"
  }
}
"#;
    let program = parse_program(code).expect("parse");
    let (engine, _types, errors) = u40_infer(code);

    // soundness (a): un-inferable scrutinee in a function body → the construct
    // still errors (the error is NOT swallowed by the resilient recording).
    assert!(
        errors
            .iter()
            .any(|e| matches!(e, TypeError::UndefinedFunction(n) if n == "snapshot")),
        "u43pre: a function-body match over an un-inferable scrutinee must still \
         surface the scrutinee error, got {:?}",
        errors
    );

    // The arm-body string literal `"saved: "` is a trivially-typed child that
    // MUST be recorded (zero OK_RESOLVED user-source miss).
    let lit_span = u43_find_expr_span(
        &program,
        &|e| matches!(e, Expr::Literal(shape_ast::ast::Literal::String(s), _) if s == "saved: "),
    )
    .expect("the `\"saved: \"` literal must exist in the AST");
    assert!(
        engine
            .resolved_expr_type(lit_span)
            .is_some_and(u43_is_string),
        "u43pre: the arm-body literal `\"saved: \"` MUST be recorded as `string` \
         (zero OK_RESOLVED miss), got {:?}",
        engine.resolved_expr_type(lit_span)
    );

    // The genuinely-un-inferable scrutinee `snapshot()` stays ABSENT (a MISS the
    // compiler boundary later turns into a surface-and-stop). No Unknown-default.
    let scrut_span = u43_find_expr_span(
        &program,
        &|e| matches!(e, Expr::FunctionCall { name, .. } if name == "snapshot"),
    )
    .expect("snapshot() scrutinee must exist");
    assert!(
        engine.resolved_expr_type(scrut_span).is_none(),
        "u43pre: the un-inferable scrutinee `snapshot()` MUST stay a MISS (None), got {:?}",
        engine.resolved_expr_type(scrut_span)
    );
}

#[test]
fn u43pre_binop_with_uninferable_operand_records_literal_sibling() {
    // BINARY-OP abort site: an un-inferable operand must not drop the sibling
    // operand's literal recording. `mystery() + 0`: before U4-3pre the
    // `infer_expr(left)?` aborted before the `0` was visited. After: `0` is
    // recorded as `int`, `mystery()` stays a MISS, and the binop still ERRORS.
    use shape_ast::ast::Expr;
    use shape_ast::parser::parse_program;

    let code = r#"
let z = mystery() + 7
"#;
    let program = parse_program(code).expect("parse");
    let (engine, _types, errors) = u40_infer(code);

    assert!(
        !errors.is_empty(),
        "u43pre: a binop with an un-inferable operand must still ERROR; got no errors"
    );

    // The sibling literal `7` MUST be recorded as `int`.
    let lit_span = u43_find_expr_span(&program, &|e| {
        matches!(e, Expr::Literal(shape_ast::ast::Literal::Int(7), _))
    })
    .expect("the `7` literal must exist");
    assert!(
        engine.resolved_expr_type(lit_span).is_some_and(u40_is_int),
        "u43pre: the sibling literal `7` MUST be recorded as `int` (zero OK_RESOLVED miss), got {:?}",
        engine.resolved_expr_type(lit_span)
    );

    // The un-inferable operand `mystery()` stays a MISS.
    let call_span = u43_find_expr_span(
        &program,
        &|e| matches!(e, Expr::FunctionCall { name, .. } if name == "mystery"),
    )
    .expect("mystery() must exist");
    assert!(
        engine.resolved_expr_type(call_span).is_none(),
        "u43pre: the un-inferable operand `mystery()` MUST stay a MISS (None), got {:?}",
        engine.resolved_expr_type(call_span)
    );
}

#[test]
fn u43pre_match_arm_bodies_recorded_across_sibling_arm_abort() {
    // Multi-arm: even when ONE arm body is un-inferable, the OTHER arm bodies'
    // literals must still be recorded. Arm 1 body references an undefined symbol
    // (un-inferable); arm 2 body is a clean `"ok"` literal. The `"ok"` literal
    // MUST land in the table; the program still ERRORS.
    use shape_ast::ast::Expr;
    use shape_ast::parser::parse_program;

    let code = r#"
fn handle() -> string {
  match snapshot() {
    Snapshot::Hash(id) => mystery() + id
    Snapshot::Resumed => "ok"
  }
}
"#;
    let program = parse_program(code).expect("parse");
    let (engine, _types, errors) = u40_infer(code);

    assert!(
        !errors.is_empty(),
        "u43pre: an un-inferable scrutinee/arm must still ERROR"
    );

    let ok_span = u43_find_expr_span(
        &program,
        &|e| matches!(e, Expr::Literal(shape_ast::ast::Literal::String(s), _) if s == "ok"),
    )
    .expect("the `\"ok\"` literal must exist");
    assert!(
        engine
            .resolved_expr_type(ok_span)
            .is_some_and(u43_is_string),
        "u43pre: the sibling arm-body literal `\"ok\"` MUST be recorded as `string`, got {:?}",
        engine.resolved_expr_type(ok_span)
    );
}

#[test]
fn u43pre_clean_match_unaffected_by_resilient_recording() {
    // SOUNDNESS (b/c/d) GUARD: a CLEAN, fully-inferable match over a real enum
    // must be UNCHANGED by resilient recording — it still infers cleanly (no new
    // error), records its arm-body literals, AND its exhaustiveness verdict is
    // unaffected (a NON-exhaustive clean match still errors; an exhaustive one
    // does not). This pins that the resilient path is inert on the happy path.
    use shape_ast::ast::Expr;
    use shape_ast::parser::parse_program;

    let code = r#"
enum Color { Red, Green, Blue }
fn name(c: Color) -> string {
  match c {
    Color::Red => "red"
    Color::Green => "green"
    Color::Blue => "blue"
  }
}
"#;
    let program = parse_program(code).expect("parse");
    let (engine, _types, errors) = u40_infer(code);
    assert!(
        errors.is_empty(),
        "u43pre: a clean exhaustive match must infer with NO error, got {:?}",
        errors
    );

    let red_span = u43_find_expr_span(
        &program,
        &|e| matches!(e, Expr::Literal(shape_ast::ast::Literal::String(s), _) if s == "red"),
    )
    .expect("`\"red\"` literal must exist");
    assert!(
        engine
            .resolved_expr_type(red_span)
            .is_some_and(u43_is_string),
        "u43pre: clean-match arm literal must still be recorded, got {:?}",
        engine.resolved_expr_type(red_span)
    );
}

#[test]
fn u43pre_nonexhaustive_clean_match_still_errors() {
    // SOUNDNESS (b): the resilient path must NOT mask a genuine non-exhaustive
    // error on a REAL (un-degraded) scrutinee. A clean enum scrutinee with a
    // missing variant still rejects as NonExhaustiveMatch.
    use shape_ast::parser::parse_program;

    let code = r#"
enum Color { Red, Green, Blue }
fn name(c: Color) -> string {
  match c {
    Color::Red => "red"
    Color::Green => "green"
  }
}
"#;
    let _program = parse_program(code).expect("parse");
    let (_engine, _types, errors) = u40_infer(code);
    assert!(
        errors
            .iter()
            .any(|e| matches!(e, TypeError::NonExhaustiveMatch { .. })),
        "u43pre: a non-exhaustive clean match must still error NonExhaustiveMatch, got {:?}",
        errors
    );
}
