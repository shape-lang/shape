use shape_ast::ast::Span;
use shape_ast::error::ShapeError;
use shape_value::v2::ConcreteType;

use super::{compiler_with_named_slots, semantic_message, semantics};
use crate::compiler::reference_flow::{
    BindingKey, ReferenceClass, ReferenceFlowPredecessor,
};
use crate::compiler::BytecodeCompiler;
use crate::type_tracking::{BindingOwnershipClass, BindingSemantics, BindingStorageClass};

#[test]
fn restore_resets_current_only_reference_storage_in_both_namespaces() {
    let mut compiler = compiler_with_named_slots();
    let saved = compiler.reference_flow_snapshot();

    compiler.type_tracker.set_local_binding_semantics(
        11,
        semantics(BindingStorageClass::Direct),
    );
    compiler.type_tracker.set_binding_semantics(
        12,
        BindingSemantics::deferred(BindingOwnershipClass::Flexible),
    );
    compiler.set_reference_flow_class(
        BindingKey::Local(11),
        ReferenceClass::ExclusiveReference {
            referent: Some(ConcreteType::I64),
        },
    );
    compiler.set_reference_flow_class(
        BindingKey::ModuleBinding(12),
        ReferenceClass::SharedReference {
            referent: Some(ConcreteType::String),
        },
    );

    compiler.restore_reference_flow_snapshot(&saved);

    assert!(!compiler.reference_value_locals.contains(&11));
    assert!(!compiler.exclusive_reference_value_locals.contains(&11));
    assert!(!compiler.reference_value_module_bindings.contains(&12));
    assert!(!compiler
        .reference_value_local_referent_concrete_type
        .contains_key(&11));
    assert!(!compiler
        .reference_value_module_binding_referent_concrete_type
        .contains_key(&12));
    assert_eq!(
        compiler
            .type_tracker
            .get_local_binding_semantics(11)
            .map(|semantics| semantics.storage_class),
        Some(BindingStorageClass::Direct),
    );
    assert_eq!(
        compiler
            .type_tracker
            .get_binding_semantics(12)
            .map(|semantics| semantics.storage_class),
        Some(BindingStorageClass::Deferred),
    );
}

#[test]
fn storage_only_conflict_is_identical_in_both_input_orders() {
    let mut compiler = compiler_with_named_slots();
    let direct = compiler.reference_flow_snapshot();
    compiler
        .type_tracker
        .set_binding_storage_class(7, BindingStorageClass::UniqueHeap);
    let unique = compiler.reference_flow_snapshot();

    let forward = compiler
        .join_reference_flow_predecessors(
            "storage merge",
            [
                ReferenceFlowPredecessor::reachable("z-direct", direct.clone()),
                ReferenceFlowPredecessor::reachable("a-unique", unique.clone()),
            ],
        )
        .expect_err("same-class storage mismatch must reject");
    let reversed = compiler
        .join_reference_flow_predecessors(
            "storage merge",
            [
                ReferenceFlowPredecessor::reachable("a-unique", unique),
                ReferenceFlowPredecessor::reachable("z-direct", direct),
            ],
        )
        .expect_err("input order cannot change the result");

    let forward = semantic_message(forward);
    let reversed = semantic_message(reversed);
    assert_eq!(forward, reversed);
    assert!(forward.starts_with(
        "[C0912] exact reference-flow conflict at storage merge for ModuleBinding(7)"
    ));
    assert!(forward.contains("Value [storage=UniqueHeap]"));
    assert!(forward.contains("Value [storage=Direct]"));
    assert!(forward.ends_with("(storage class differs)"));
}

#[test]
fn same_number_local_and_module_keys_remain_independent() {
    let mut compiler = BytecodeCompiler::new();
    compiler
        .locals
        .last_mut()
        .expect("initial local scope")
        .insert("local_zero".to_string(), 0);
    compiler
        .module_bindings
        .insert("module_zero".to_string(), 0);
    compiler.type_tracker.set_local_binding_semantics(
        0,
        semantics(BindingStorageClass::Direct),
    );
    compiler.type_tracker.set_binding_semantics(
        0,
        semantics(BindingStorageClass::Direct),
    );
    compiler.set_reference_flow_class(
        BindingKey::Local(0),
        ReferenceClass::SharedReference {
            referent: Some(ConcreteType::I64),
        },
    );
    let local_reference = compiler.reference_flow_snapshot();

    compiler.set_reference_flow_class(
        BindingKey::ModuleBinding(0),
        ReferenceClass::ExclusiveReference {
            referent: Some(ConcreteType::String),
        },
    );
    let both_references = compiler.reference_flow_snapshot();

    let error = compiler
        .join_reference_flow_predecessors(
            "namespace merge",
            [
                ReferenceFlowPredecessor::reachable("a-local-only", local_reference),
                ReferenceFlowPredecessor::reachable("z-both", both_references.clone()),
            ],
        )
        .expect_err("only the module namespace differs");
    let message = semantic_message(error);
    assert!(message.contains("ModuleBinding(0) (name 'module_zero')"));
    assert!(!message.contains("Local(0) (name 'local_zero')"));

    compiler.restore_reference_flow_snapshot(&both_references);
    assert!(compiler.reference_value_locals.contains(&0));
    assert!(compiler.reference_value_module_bindings.contains(&0));
    assert!(!compiler.exclusive_reference_value_locals.contains(&0));
    assert!(compiler
        .exclusive_reference_value_module_bindings
        .contains(&0));
}

#[test]
fn c0912_uses_structural_key_name_and_best_binding_location() {
    let mut compiler = compiler_with_named_slots();
    compiler.set_source_with_file("first line\nmodule_ref here\n", "flow.shape");
    compiler
        .module_binding_spans
        .insert(7, Span { start: 11, end: 21 });
    let direct = compiler.reference_flow_snapshot();
    compiler
        .type_tracker
        .set_binding_storage_class(7, BindingStorageClass::SharedCow);
    let shared = compiler.reference_flow_snapshot();

    let error = compiler
        .join_reference_flow_predecessors_at(
            "if merge",
            Some(Span { start: 0, end: 5 }),
            [
                ReferenceFlowPredecessor::reachable("then", direct),
                ReferenceFlowPredecessor::reachable("else", shared),
            ],
        )
        .expect_err("exact storage conflict must reject");

    match error {
        ShapeError::SemanticError { message, location } => {
            assert!(message.starts_with(
                "[C0912] exact reference-flow conflict at if merge for ModuleBinding(7) \
                 (name 'module_ref')"
            ));
            let location = location.expect("binding span is the best anchor");
            assert_eq!(location.file.as_deref(), Some("flow.shape"));
            assert_eq!((location.line, location.column), (2, 1));
        }
        other => panic!("expected semantic C0912, got {other:?}"),
    }
}

#[test]
fn c0912_uses_fallback_span_without_fabricating_a_binding_name() {
    let mut compiler = BytecodeCompiler::new();
    compiler.set_source("first\nmerge site\n");
    compiler.type_tracker.set_local_binding_semantics(
        19,
        semantics(BindingStorageClass::Direct),
    );
    compiler.set_reference_flow_class(
        BindingKey::Local(19),
        ReferenceClass::SharedReference { referent: None },
    );
    let reference = compiler.reference_flow_snapshot();
    compiler.set_reference_flow_class(BindingKey::Local(19), ReferenceClass::Value);
    let value = compiler.reference_flow_snapshot();

    let error = compiler
        .join_reference_flow_predecessors_at(
            "fallback merge",
            Some(Span { start: 6, end: 11 }),
            [
                ReferenceFlowPredecessor::reachable("reference", reference),
                ReferenceFlowPredecessor::reachable("value", value),
            ],
        )
        .expect_err("representation conflict must reject");

    match error {
        ShapeError::SemanticError { message, location } => {
            assert!(message.contains("for Local(19):"));
            assert!(!message.contains("name '"));
            let location = location.expect("merge span is the fallback anchor");
            assert_eq!((location.line, location.column), (2, 1));
        }
        other => panic!("expected semantic C0912, got {other:?}"),
    }
}
