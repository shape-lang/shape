use super::*;
use shape_ast::ast::{DestructurePattern, FunctionParameter};

fn simple_parameter(name: &str) -> FunctionParameter {
    FunctionParameter {
        pattern: DestructurePattern::Identifier(name.to_string(), Span::DUMMY),
        is_const: false,
        is_reference: false,
        is_mut_reference: false,
        is_out: false,
        type_annotation: None,
        default_value: None,
    }
}

fn parameter_evidence(access: CaptureAccess) -> CaptureParameterEvidence {
    CaptureParameterEvidence {
        access,
        binding_span: None,
        binding_lineage: None,
        semantic_type: CaptureSemanticEvidence::unavailable(
            CaptureSemanticIssueKind::MissingInferenceFact,
            "planner slot-semantics proof has no inference subject",
        ),
    }
}

#[test]
fn inherited_parameter_semantics_are_installed_by_ordinal_and_only_for_param_access() {
    let mut compiler = BytecodeCompiler::new();
    compiler.locals = vec![std::collections::HashMap::from([
        ("named_zero".to_string(), 4),
        ("named_one".to_string(), 0),
        ("named_two".to_string(), 1),
        ("named_three".to_string(), 2),
        ("named_four".to_string(), 3),
    ])];
    let params = [
        simple_parameter("named_zero"),
        simple_parameter("named_one"),
        simple_parameter("named_two"),
        simple_parameter("named_three"),
        simple_parameter("named_four"),
    ];
    let accesses = [
        CaptureAccess::Param,
        CaptureAccess::SharedCell,
        CaptureAccess::Param,
        CaptureAccess::OwnedMutableCell,
        CaptureAccess::MutableCell,
    ];
    let generic_param_semantics = BytecodeCompiler::owned_mutable_binding_semantics();
    for slot in 0..accesses.len() as u16 {
        compiler
            .type_tracker
            .set_local_binding_semantics(slot, generic_param_semantics);
    }

    compiler
        .install_inherited_capture_parameter_evidence(
            "misleading_names",
            &params,
            accesses.len(),
            Some(
                accesses
                    .iter()
                    .copied()
                    .map(parameter_evidence)
                    .collect(),
            ),
        )
        .expect("structural capture parameters install by ordinal");

    let immutable = BytecodeCompiler::owned_immutable_binding_semantics();
    for slot in [0, 2] {
        assert_eq!(
            compiler.type_tracker.get_local_binding_semantics(slot),
            Some(&immutable),
            "Param evidence owns its exact ordinal slot"
        );
        assert!(compiler.immutable_locals.contains(&slot));
    }
    for slot in [1, 3, 4] {
        assert_eq!(
            compiler.type_tracker.get_local_binding_semantics(slot),
            Some(&generic_param_semantics),
            "cell-backed evidence preserves generic parameter semantics"
        );
        assert!(!compiler.immutable_locals.contains(&slot));
        assert!(!compiler.const_locals.contains(&slot));
    }
    assert!(params.iter().all(|param| !param.is_const));
    for (slot, access) in accesses.into_iter().enumerate() {
        assert_eq!(
            compiler
                .inherited_capture_parameter_evidence
                .get(&(slot as u16))
                .map(|evidence| evidence.access),
            Some(access)
        );
        assert_ne!(
            compiler.resolve_local(params[slot].simple_name().expect("simple parameter")),
            Some(slot as u16),
            "misleading names must not select the evidence slot"
        );
    }
}

#[test]
fn inherited_parameter_without_lineage_is_never_reminted_from_immediate_slot() {
    let mut compiler = BytecodeCompiler::new();
    let origin = compiler.generated_node_issuer.issue(
        shape_ast::ast::GeneratedExpansionFingerprint::from_components(3, 5),
        shape_ast::ast::GeneratedNodePath::decl_root("extend:Job")
            .child("method:read")
            .child("closure:1"),
        9,
        Span { start: 4, end: 8 },
        "Job.read".to_string(),
    );
    let facts = CaptureBindingFacts {
        name: "forwarded".to_string(),
        target: Some(CaptureTarget::Local(0)),
        binding_span: None,
        binding_lineage: None,
        binding_file_id: 9,
        semantic_type: CaptureSemanticEvidence::unavailable(
            CaptureSemanticIssueKind::MissingInferenceFact,
            "planner test has no binding inference subject",
        ),
        ownership: Some(BindingOwnershipClass::OwnedImmutable),
        storage: Some(BindingStorageClass::Direct),
        mutated: false,
        boxed: false,
        witness_shared_local: false,
        witness_shared_module_binding: false,
        witness_owned_mutable_local: false,
        inherited_capture_parameter: true,
        inherited_shared_cell: false,
    };
    let plan = PlannedCapture {
        plan: infer_plan(&facts),
        facts,
        declared: None,
        declaration_span: None,
        use_spans: Vec::new(),
    };

    let pack = compiler
        .build_capture_pack(
            1,
            &[plan],
            Some(&origin),
            CallableSemanticEvidence::unavailable(
                CallableSemanticIssueKind::PeekOnly,
                "planner lineage test has no callable inference subject",
            ),
        )
        .expect("well-formed pack");
    assert_eq!(pack.descriptors[0].target, Some(CaptureTarget::Local(0)));
    assert!(
        pack.descriptors[0].binding_lineage.is_none(),
        "an inherited carrier without proof must remain unavailable for query quarantine"
    );
}
