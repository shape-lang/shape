use super::*;

#[test]
fn inherited_parameter_without_lineage_is_never_reminted_from_immediate_slot() {
    let mut compiler = BytecodeCompiler::new();
    let origin = compiler.generated_node_issuer.issue(
        (3, 5),
        vec![
            "extend:Job".to_string(),
            "method:read".to_string(),
            "closure:1".to_string(),
        ],
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
