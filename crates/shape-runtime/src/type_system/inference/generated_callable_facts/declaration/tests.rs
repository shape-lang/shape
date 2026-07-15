use super::*;

fn callable_declaration() -> FunctionDef {
    FunctionDef {
        name: "subject".to_string(),
        name_span: shape_ast::ast::Span::DUMMY,
        declaring_module_path: None,
        doc_comment: None,
        type_params: None,
        params: Vec::new(),
        return_type: None,
        body: Vec::new(),
        annotations: Vec::new(),
        where_clause: None,
        is_async: false,
        is_comptime: false,
    }
}

fn callable_scheme(quantified: Vec<TypeVar>, result: TypeVar) -> TypeScheme {
    TypeScheme::poly(
        quantified,
        Type::Function {
            params: Vec::new(),
            returns: Box::new(Type::Variable(result)),
        },
    )
}

#[test]
fn raw_quantifiers_do_not_mint_declared_capabilities() {
    let mut engine = TypeInferenceEngine::new();
    let raw = engine.type_var_gen.fresh_var();
    let scheme = callable_scheme(vec![raw.clone()], raw);

    assert!(declared_quantifiers(&scheme).unwrap().is_empty());
    assert!(
        TypeInferenceEngine::declared_parameter_tokens(&scheme)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn mixed_schemes_expose_only_the_declared_subsequence() {
    let mut engine = TypeInferenceEngine::new();
    let raw = engine.type_var_gen.fresh_var();
    let declared = TypeVar::declared(engine.type_var_gen.fresh_declared_owner(), 0, "T");
    let scheme = callable_scheme(vec![raw, declared.clone()], declared.clone());

    assert_eq!(declared_quantifiers(&scheme).unwrap(), vec![declared.clone()]);
    let tokens = TypeInferenceEngine::declared_parameter_tokens(&scheme).unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(tokens.get("T"), Some(&declared));
}

#[test]
fn raw_quantifiers_may_change_while_republication_preserves_the_binding() {
    let mut engine = TypeInferenceEngine::new();
    let function = callable_declaration();
    let first = engine.type_var_gen.fresh_var();
    let first_scheme = callable_scheme(vec![first.clone()], first);
    let first_type = first_scheme.ty.clone();
    engine
        .predeclare_named_callable_scheme(&function, first_scheme, &first_type)
        .unwrap();
    let token = engine.env.lookup_binding_token("subject").unwrap();
    let declaration = InferenceCallableDeclarationToken::of(&function);
    assert!(
        engine.generated_inference.callable_declared_parameters[&declaration].is_empty()
    );

    let second = engine.type_var_gen.fresh_var();
    let second_scheme = callable_scheme(vec![second.clone()], second.clone());
    let second_type = second_scheme.ty.clone();
    engine
        .republish_named_callable_scheme(&function, second_scheme, &second_type)
        .unwrap();

    assert_eq!(engine.env.lookup_binding_token("subject"), Some(token));
    assert_eq!(engine.env.lookup("subject").unwrap().quantified, vec![second]);
    assert!(
        engine.generated_inference.callable_declared_parameters[&declaration].is_empty()
    );
}

#[test]
fn declared_source_rename_is_rejected_even_when_typed_identity_matches() {
    let mut engine = TypeInferenceEngine::new();
    let function = callable_declaration();
    let owner = engine.type_var_gen.fresh_declared_owner();
    let original = TypeVar::declared(owner, 0, "T");
    let original_scheme = callable_scheme(vec![original.clone()], original);
    let original_type = original_scheme.ty.clone();
    engine
        .predeclare_named_callable_scheme(&function, original_scheme, &original_type)
        .unwrap();

    let renamed = TypeVar::declared(owner, 0, "RenamedT");
    let renamed_scheme = callable_scheme(vec![renamed.clone()], renamed);
    let renamed_type = renamed_scheme.ty.clone();
    let error = engine
        .republish_named_callable_scheme(&function, renamed_scheme, &renamed_type)
        .unwrap_err();
    assert_eq!(
        error,
        TypeError::ConstraintViolation(
            "internal inference error: callable re-publication changed its declared generic tokens"
                .to_string()
        )
    );
}

#[test]
fn malformed_declared_subsequences_remain_rejected() {
    let mut engine = TypeInferenceEngine::new();
    let first_owner = engine.type_var_gen.fresh_declared_owner();
    let second_owner = engine.type_var_gen.fresh_declared_owner();
    let body = engine.type_var_gen.fresh_var();
    let invalid = [
        vec![
            TypeVar::declared(first_owner, 0, "T"),
            TypeVar::declared(second_owner, 1, "U"),
        ],
        vec![TypeVar::declared(first_owner, 1, "T")],
        vec![
            TypeVar::declared(first_owner, 0, "T"),
            TypeVar::declared(first_owner, 1, "T"),
        ],
    ];

    for quantified in invalid {
        let scheme = callable_scheme(quantified, body.clone());
        assert!(declared_quantifiers(&scheme).is_err());
    }
}
