use shape_ast::error::ShapeError;
use shape_value::v2::ConcreteType;

use super::{BindingKey, ReferenceClass, ReferenceFlowPredecessor};
use crate::compiler::BytecodeCompiler;
use crate::type_tracking::{BindingOwnershipClass, BindingSemantics, BindingStorageClass};

fn semantics(storage_class: BindingStorageClass) -> BindingSemantics {
    let mut semantics = BindingSemantics::deferred(BindingOwnershipClass::OwnedMutable);
    semantics.storage_class = storage_class;
    semantics
}

fn compiler_with_named_slots() -> BytecodeCompiler {
    let mut compiler = BytecodeCompiler::new();
    compiler
        .locals
        .last_mut()
        .expect("compiler starts with a local scope")
        .insert("local_ref".to_string(), 3);
    compiler
        .module_bindings
        .insert("module_ref".to_string(), 7);
    compiler.type_tracker.set_local_binding_semantics(
        3,
        semantics(BindingStorageClass::Direct),
    );
    compiler.type_tracker.set_binding_semantics(
        7,
        semantics(BindingStorageClass::Direct),
    );
    compiler
}

fn semantic_message(error: ShapeError) -> String {
    match error {
        ShapeError::SemanticError { message, .. } => message,
        other => panic!("expected semantic error, got {other:?}"),
    }
}

#[test]
fn snapshot_restore_roundtrips_classes_referents_and_storage() {
    let mut compiler = compiler_with_named_slots();
    compiler.set_reference_flow_class(
        BindingKey::Local(3),
        ReferenceClass::SharedReference {
            referent: Some(ConcreteType::I64),
        },
    );
    compiler.set_reference_flow_class(
        BindingKey::ModuleBinding(7),
        ReferenceClass::ExclusiveReference {
            referent: Some(ConcreteType::String),
        },
    );
    let expected = compiler.reference_flow_snapshot();

    compiler.set_reference_flow_class(BindingKey::Local(3), ReferenceClass::Value);
    compiler.set_reference_flow_class(BindingKey::ModuleBinding(7), ReferenceClass::Value);
    assert!(!compiler
        .reference_value_local_referent_concrete_type
        .contains_key(&3));
    assert!(!compiler
        .reference_value_module_binding_referent_concrete_type
        .contains_key(&7));
    compiler.restore_reference_flow_snapshot(&expected);

    assert_eq!(compiler.reference_flow_snapshot(), expected);
    assert!(compiler.reference_value_locals.contains(&3));
    assert!(!compiler.exclusive_reference_value_locals.contains(&3));
    assert!(compiler.reference_value_module_bindings.contains(&7));
    assert!(compiler
        .exclusive_reference_value_module_bindings
        .contains(&7));
    assert_eq!(
        compiler
            .reference_value_module_binding_referent_concrete_type
            .get(&7),
        Some(&ConcreteType::String),
    );
}

#[test]
fn restore_preserves_non_reference_module_storage_class() {
    let mut compiler = compiler_with_named_slots();
    let direct = compiler.reference_flow_snapshot();

    compiler.set_reference_flow_class(
        BindingKey::ModuleBinding(7),
        ReferenceClass::SharedReference { referent: None },
    );
    assert_eq!(
        compiler
            .type_tracker
            .get_binding_semantics(7)
            .map(|semantics| semantics.storage_class),
        Some(BindingStorageClass::Reference),
    );

    compiler.restore_reference_flow_snapshot(&direct);
    assert_eq!(
        compiler
            .type_tracker
            .get_binding_semantics(7)
            .map(|semantics| semantics.storage_class),
        Some(BindingStorageClass::Direct),
    );
}

#[test]
fn homogeneous_reachable_predecessors_join_exactly() {
    let mut compiler = compiler_with_named_slots();
    compiler.set_reference_flow_class(
        BindingKey::ModuleBinding(7),
        ReferenceClass::SharedReference {
            referent: Some(ConcreteType::I64),
        },
    );
    let state = compiler.reference_flow_snapshot();

    let joined = compiler
        .join_reference_flow_predecessors(
            "if merge",
            [
                ReferenceFlowPredecessor::reachable("then", state.clone()),
                ReferenceFlowPredecessor::reachable("else", state.clone()),
            ],
        )
        .expect("homogeneous flow joins")
        .expect("a reachable predecessor produces state");

    assert_eq!(joined, state);
}

#[test]
fn value_reference_join_is_named_and_deterministic() {
    let mut compiler = compiler_with_named_slots();
    let value = compiler.reference_flow_snapshot();
    compiler.set_reference_flow_class(
        BindingKey::ModuleBinding(7),
        ReferenceClass::SharedReference { referent: None },
    );
    let reference = compiler.reference_flow_snapshot();

    let error = compiler
        .join_reference_flow_predecessors(
            "if merge",
            [
                ReferenceFlowPredecessor::reachable("z-value", value),
                ReferenceFlowPredecessor::reachable("a-reference", reference),
            ],
        )
        .expect_err("heterogeneous representation must be rejected");
    let message = semantic_message(error);

    assert!(message.starts_with(
        "heterogeneous reference flow at if merge for binding 'module_ref'"
    ));
    assert!(message.contains("predecessor 'a-reference' is SharedReference<?>"));
    assert!(message.contains("predecessor 'z-value' is Value"));
}

#[test]
fn shared_exclusive_join_is_a_conflict() {
    let mut compiler = compiler_with_named_slots();
    compiler.set_reference_flow_class(
        BindingKey::ModuleBinding(7),
        ReferenceClass::SharedReference { referent: None },
    );
    let shared = compiler.reference_flow_snapshot();
    compiler.set_reference_flow_class(
        BindingKey::ModuleBinding(7),
        ReferenceClass::ExclusiveReference { referent: None },
    );
    let exclusive = compiler.reference_flow_snapshot();

    let error = compiler
        .join_reference_flow_predecessors(
            "branch merge",
            [
                ReferenceFlowPredecessor::reachable("shared", shared),
                ReferenceFlowPredecessor::reachable("exclusive", exclusive),
            ],
        )
        .expect_err("borrow modes are distinct representations");

    assert!(semantic_message(error).starts_with(
        "conflicting reference flow at branch merge for binding 'module_ref'"
    ));
}

#[test]
fn referent_evidence_conflict_is_rejected() {
    let mut compiler = compiler_with_named_slots();
    compiler.set_reference_flow_class(
        BindingKey::ModuleBinding(7),
        ReferenceClass::SharedReference {
            referent: Some(ConcreteType::I64),
        },
    );
    let ints = compiler.reference_flow_snapshot();
    compiler.set_reference_flow_class(
        BindingKey::ModuleBinding(7),
        ReferenceClass::SharedReference {
            referent: Some(ConcreteType::String),
        },
    );
    let strings = compiler.reference_flow_snapshot();

    let error = compiler
        .join_reference_flow_predecessors(
            "match merge",
            [
                ReferenceFlowPredecessor::reachable("int arm", ints),
                ReferenceFlowPredecessor::reachable("string arm", strings),
            ],
        )
        .expect_err("referent evidence must agree");
    let message = semantic_message(error);

    assert!(message.contains("binding 'module_ref'"));
    assert!(message.contains("SharedReference<I64>"));
    assert!(message.contains("SharedReference<String>"));
}

#[test]
fn unreachable_predecessors_do_not_participate() {
    let mut compiler = compiler_with_named_slots();
    let value = compiler.reference_flow_snapshot();
    compiler.set_reference_flow_class(
        BindingKey::ModuleBinding(7),
        ReferenceClass::SharedReference { referent: None },
    );
    let reference = compiler.reference_flow_snapshot();

    let joined = compiler
        .join_reference_flow_predecessors(
            "terminal merge",
            [
                ReferenceFlowPredecessor::reachable("fallthrough", value.clone()),
                ReferenceFlowPredecessor::unreachable("return", reference.clone()),
            ],
        )
        .expect("unreachable conflict is ignored")
        .expect("fallthrough remains reachable");
    assert_eq!(joined, value);

    let none = compiler
        .join_reference_flow_predecessors(
            "terminal merge",
            [ReferenceFlowPredecessor::unreachable("return", reference)],
        )
        .expect("all-unreachable join is valid");
    assert!(none.is_none());
}

#[test]
fn restore_repairs_exclusive_subset_invariant() {
    let mut compiler = compiler_with_named_slots();
    compiler.exclusive_reference_value_locals.insert(3);
    let state = compiler.reference_flow_snapshot();

    compiler.exclusive_reference_value_locals.clear();
    compiler.restore_reference_flow_snapshot(&state);

    assert!(compiler.reference_value_locals.contains(&3));
    assert!(compiler.exclusive_reference_value_locals.contains(&3));
    assert!(compiler
        .exclusive_reference_value_locals
        .is_subset(&compiler.reference_value_locals));
}

#[test]
fn pop_scope_evicts_exact_local_reference_state() {
    let mut compiler = compiler_with_named_slots();
    compiler.push_scope();
    let scoped = compiler
        .declare_local("scoped_ref")
        .expect("local slot is available");
    compiler.type_tracker.set_local_binding_semantics(
        scoped,
        semantics(BindingStorageClass::Direct),
    );
    compiler.set_reference_flow_class(
        BindingKey::Local(scoped),
        ReferenceClass::ExclusiveReference {
            referent: Some(ConcreteType::I64),
        },
    );

    compiler.pop_scope();

    assert!(!compiler.reference_value_locals.contains(&scoped));
    assert!(!compiler.exclusive_reference_value_locals.contains(&scoped));
    assert!(!compiler
        .reference_value_local_referent_concrete_type
        .contains_key(&scoped));
}
