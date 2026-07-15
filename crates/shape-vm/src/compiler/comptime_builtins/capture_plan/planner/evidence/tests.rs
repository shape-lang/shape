use super::*;

use shape_ast::ast::{
    Expr, GeneratedExpansionFingerprint, GeneratedNodeIssuer, GeneratedNodePath, Item, Span,
    Statement,
};
use shape_runtime::type_system::{
    GeneratedCaptureFact, GeneratedCaptureKey, GeneratedNodeKey, TypeInferenceEngine,
};

fn stamped_polymorphic_capture() -> (shape_ast::ast::Program, GeneratedNodeOrigin) {
    let mut program = shape_ast::parse_program(
        r#"
            fn identity<T>(value: T) -> T { value }
            fn run() -> int {
                let closure = || identity(1)
                closure()
            }
            run()
        "#,
    )
    .expect("generated capture fixture parses");
    let issuer = GeneratedNodeIssuer::new();
    let root = issuer.issue(
        GeneratedExpansionFingerprint::from_components(41, 43),
        GeneratedNodePath::decl_root("extend:Fixture").child("method:run"),
        0,
        Span::DUMMY,
        "Fixture.run".to_string(),
    );
    let run = program
        .items
        .iter_mut()
        .find_map(|item| match item {
            Item::Function(function, _) if function.name == "run" => Some(function),
            _ => None,
        })
        .expect("fixture declares run");
    shape_ast::transform::stamp_generated_closures(&mut run.body, &root);
    let origin = run
        .body
        .iter()
        .find_map(|statement| match statement {
            Statement::VariableDecl(declaration, _) => match declaration.value.as_ref() {
                Some(Expr::FunctionExpr {
                    generated_origin: Some(origin),
                    ..
                }) => Some(origin.clone()),
                _ => None,
            },
            _ => None,
        })
        .expect("stamped closure retains its generated origin");
    (program, origin)
}

#[test]
fn issuer_unavailable_capture_fact_preserves_kind_and_detail() {
    let (program, origin) = stamped_polymorphic_capture();
    let mut inference = TypeInferenceEngine::new();
    let (facts, errors) = inference.infer_program_facts_best_effort(&program);
    assert!(errors.is_empty(), "unexpected inference errors: {errors:?}");
    let key = GeneratedCaptureKey::new(GeneratedNodeKey::from_origin(&origin), 0);
    let issuer_detail = match facts
        .generated_capture_fact(&key)
        .expect("inference finalizer publishes the exact capture key")
    {
        GeneratedCaptureFact::Unavailable(issue) => issue.detail().to_string(),
        other => panic!("polymorphic capture must be unavailable: {other:?}"),
    };
    assert_eq!(
        issuer_detail,
        "captured binding 'identity' is polymorphic and has no monomorphic value type"
    );

    let mut compiler = BytecodeCompiler::new();
    compiler.inference_facts = facts;
    let evidence = compiler.generated_capture_semantic_evidence(
        "identity",
        Some(&origin),
        0,
        Err("freeze must not be consulted for unavailable inference".to_string()),
    );
    let CaptureSemanticEvidence::Unavailable(issue) = evidence else {
        panic!("issuer-unavailable fact must remain typed unavailable")
    };
    assert_eq!(issue.kind(), CaptureSemanticIssueKind::InferenceUnavailable);
    assert_eq!(
        issue.detail(),
        format!("capture 'identity' structural inference is unavailable: {issuer_detail}")
    );
}
