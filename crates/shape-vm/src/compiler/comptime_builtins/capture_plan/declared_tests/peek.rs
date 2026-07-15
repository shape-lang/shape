use super::*;

type ClosureParts = (
    Vec<shape_ast::ast::FunctionParameter>,
    Vec<shape_ast::ast::Statement>,
    Option<shape_ast::ast::CaptureClause>,
    shape_ast::ast::Span,
);

fn closure_parts(source: &str) -> ClosureParts {
    let program = shape_ast::parse_program(source).expect("peek fixture parses");
    let shape_ast::ast::Item::Statement(
        shape_ast::ast::Statement::VariableDecl(decl, _),
        _,
    ) = program.items.into_iter().next().expect("one declaration")
    else {
        panic!("fixture must be a variable declaration");
    };
    let Some(shape_ast::ast::Expr::FunctionExpr {
        params,
        body,
        captures,
        span,
        ..
    }) = decl.value
    else {
        panic!("fixture initializer must be a closure");
    };
    (params, body, captures, span)
}

fn artifact_counts(compiler: &BytecodeCompiler) -> (usize, usize, usize, usize) {
    (
        compiler.closure_registry().len(),
        compiler.closure_capture_packs.len(),
        compiler.closure_type_ids().len(),
        compiler.program.closure_function_layouts.len(),
    )
}

/// A monomorphization peek is non-authoritative. It must run emission's exact
/// generated-only surface gate before interning: an otherwise-valid explicit
/// `share` in ordinary source and an implicit capture in generated code both
/// decline without leaving any registry/layout artifact. Recognized generated
/// provenance plus an explicit declaration remains the valid path.
#[test]
fn peek_applies_canonical_capture_surface_before_minting_artifacts() {
    let mut compiler = compile("var total = 40\ntotal");
    let baseline = artifact_counts(&compiler);
    let (explicit_params, explicit_body, explicit_clause, explicit_span) =
        closure_parts("let worker = |; share total| total");
    let explicit_clause = explicit_clause.as_ref().expect("explicit clause");

    assert!(
        compiler
            .mint_closure_type_id_peek(
                &explicit_params,
                &explicit_body,
                Some(explicit_clause),
                None,
                explicit_span,
            )
            .is_none(),
        "ordinary explicit `share` must decline at C0903 before interning"
    );
    assert_eq!(
        artifact_counts(&compiler),
        baseline,
        "ordinary explicit rejection must not mint any closure artifact"
    );

    let origin = compiler.generated_node_issuer.issue(
        shape_ast::ast::GeneratedExpansionFingerprint::from_components(7, 9),
        shape_ast::ast::GeneratedNodePath::decl_root("method:peek").child("closure:0"),
        0,
        shape_ast::ast::Span::DUMMY,
        "worker".to_string(),
    );
    let (implicit_params, implicit_body, implicit_clause, implicit_span) =
        closure_parts("let worker = || total");
    assert!(implicit_clause.is_none());
    assert!(
        compiler
            .mint_closure_type_id_peek(
                &implicit_params,
                &implicit_body,
                None,
                Some(&origin),
                implicit_span,
            )
            .is_none(),
        "generated implicit capture must decline before interning"
    );
    assert_eq!(
        artifact_counts(&compiler),
        baseline,
        "generated implicit rejection must not mint any closure artifact"
    );

    assert!(
        compiler
            .mint_closure_type_id_peek(
                &explicit_params,
                &explicit_body,
                Some(explicit_clause),
                Some(&origin),
                explicit_span,
            )
            .is_some(),
        "recognized generated explicit capture must supply a specialization key"
    );
    assert_eq!(
        artifact_counts(&compiler),
        (baseline.0 + 1, baseline.1, baseline.2, baseline.3),
        "a valid peek interns only the registry key; emission owns packs/layouts"
    );
}
