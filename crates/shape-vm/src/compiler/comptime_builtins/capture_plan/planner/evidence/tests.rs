use super::*;

use shape_ast::ast::{
    Expr, GeneratedExpansionFingerprint, GeneratedNodeIssuer, GeneratedNodePath, Item, Span,
    Statement,
};
use shape_runtime::type_system::{
    GeneratedCaptureFact, GeneratedCaptureKey, GeneratedNodeKey, TypeInferenceEngine,
};
use std::collections::{HashMap, HashSet};

use crate::bytecode::Function;
use crate::compiler::reference_flow::{BindingKey, ReferenceClass};
use crate::mir::{SlotId, StoragePlan};
use crate::type_tracking::{BindingOwnershipClass, BindingSemantics};

const LOCAL_SLOT: u16 = 3;
const MODULE_SLOT: u16 = 7;

fn bytecode_function(name: &str) -> Function {
    Function {
        name: name.to_string(),
        arity: 0,
        param_names: Vec::new(),
        locals_count: 0,
        entry_point: 0,
        body_length: 0,
        is_closure: false,
        captures_count: 0,
        is_async: false,
        ref_params: Vec::new(),
        ref_mutates: Vec::new(),
        mutable_captures: Vec::new(),
        frame_descriptor: None,
        osr_entry_points: Vec::new(),
        mir_data: None,
    }
}

fn semantics(storage: BindingStorageClass) -> BindingSemantics {
    let mut semantics = BindingSemantics::deferred(BindingOwnershipClass::OwnedImmutable);
    semantics.storage_class = storage;
    semantics
}

fn compiler_with_direct_local() -> BytecodeCompiler {
    let mut compiler = BytecodeCompiler::new();
    compiler
        .locals
        .last_mut()
        .expect("compiler starts with a local scope")
        .insert("captured".to_string(), LOCAL_SLOT);
    compiler
        .type_tracker
        .set_local_binding_semantics(LOCAL_SLOT, semantics(BindingStorageClass::Direct));
    compiler
        .program
        .functions
        .push(bytecode_function("evidence"));
    compiler.current_function = Some(0);
    compiler.mir_storage_plans.insert(
        "evidence".to_string(),
        StoragePlan {
            slot_classes: HashMap::from([(
                SlotId(LOCAL_SLOT.saturating_add(1)),
                BindingStorageClass::Direct,
            )]),
            slot_semantics: HashMap::new(),
            inline_array_sizes: HashMap::new(),
            non_escaping_closure_slots: HashSet::new(),
            reference_escape_promotion_slots: HashSet::new(),
            escape: Default::default(),
        },
    );
    compiler
}

fn capture_facts(compiler: &BytecodeCompiler, name: &str) -> CaptureBindingFacts {
    compiler.capture_binding_facts(
        name,
        false,
        None,
        0,
        Err("ordinary source has no semantic freeze".to_string()),
    )
}

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
                    generated_origin, ..
                }) => generated_origin.as_deref().cloned(),
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

#[test]
fn true_local_reference_precedes_a_direct_mir_storage_plan() {
    let mut compiler = compiler_with_direct_local();
    compiler.set_reference_flow_class(
        BindingKey::Local(LOCAL_SLOT),
        ReferenceClass::SharedReference {
            referent: Some(shape_value::v2::ConcreteType::I64),
        },
    );

    assert_eq!(
        capture_facts(&compiler, "captured").storage,
        Some(BindingStorageClass::Reference)
    );
}

#[test]
fn true_local_reference_precedes_inherited_shared_cell_evidence() {
    let mut compiler = compiler_with_direct_local();
    compiler.inherited_capture_parameter_evidence.insert(
        LOCAL_SLOT,
        CaptureParameterEvidence {
            access: CaptureAccess::SharedCell,
            binding_span: None,
            binding_lineage: None,
            semantic_type: CaptureSemanticEvidence::unavailable(
                CaptureSemanticIssueKind::MissingInferenceFact,
                "precedence fixture has no semantic subject",
            ),
        },
    );
    compiler.set_reference_flow_class(
        BindingKey::Local(LOCAL_SLOT),
        ReferenceClass::ExclusiveReference { referent: None },
    );

    let facts = capture_facts(&compiler, "captured");
    assert_eq!(facts.storage, Some(BindingStorageClass::Reference));
    assert!(facts.inherited_shared_cell);
}

#[test]
fn inherited_shared_cell_precedes_mir_when_no_true_reference_exists() {
    let mut compiler = compiler_with_direct_local();
    compiler.inherited_capture_parameter_evidence.insert(
        LOCAL_SLOT,
        CaptureParameterEvidence {
            access: CaptureAccess::SharedCell,
            binding_span: None,
            binding_lineage: None,
            semantic_type: CaptureSemanticEvidence::unavailable(
                CaptureSemanticIssueKind::MissingInferenceFact,
                "precedence fixture has no semantic subject",
            ),
        },
    );

    assert_eq!(
        capture_facts(&compiler, "captured").storage,
        Some(BindingStorageClass::SharedCow)
    );
}

#[test]
fn inferred_reference_optimization_does_not_trigger_reference_rejection() {
    for exclusive in [false, true] {
        let mut compiler = compiler_with_direct_local();
        compiler.ref_locals.insert(LOCAL_SLOT);
        compiler.inferred_ref_locals.insert(LOCAL_SLOT);
        if exclusive {
            compiler.exclusive_ref_locals.insert(LOCAL_SLOT);
        }

        let facts = capture_facts(&compiler, "captured");
        assert_eq!(facts.storage, Some(BindingStorageClass::Direct));
        assert!(
            lower_declared(CaptureMode::Move, &facts).is_ok(),
            "inferred exclusive={exclusive} is an owned-value optimization"
        );
    }
}

#[test]
fn module_true_reference_precedes_tracker_storage_semantics() {
    let mut compiler = BytecodeCompiler::new();
    compiler
        .module_bindings
        .insert("module_ref".to_string(), MODULE_SLOT);
    compiler
        .type_tracker
        .set_binding_semantics(MODULE_SLOT, semantics(BindingStorageClass::Direct));
    compiler.set_reference_flow_class(
        BindingKey::ModuleBinding(MODULE_SLOT),
        ReferenceClass::SharedReference { referent: None },
    );

    assert_eq!(
        capture_facts(&compiler, "module_ref").storage,
        Some(BindingStorageClass::Reference)
    );
}
