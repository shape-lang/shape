use shape_ast::ast::Span;
use shape_ast::error::ShapeError;
use shape_value::v2::ConcreteType;

use super::{compiler_with_named_slots, semantic_message, semantics};
use crate::compiler::reference_flow::{BindingKey, ReferenceClass};
use crate::compiler::BytecodeCompiler;
use crate::type_tracking::{
    BindingOwnershipClass, BindingSemantics, BindingStorageClass,
};

#[test]
fn successful_transaction_restores_exact_flow_and_local_semantics() {
    let mut compiler = compiler_with_named_slots();
    compiler.set_reference_flow_class(
        BindingKey::Local(3),
        ReferenceClass::SharedReference {
            referent: Some(ConcreteType::I64),
        },
    );
    let expected_flow = compiler.reference_flow_snapshot();
    let expected_semantics = *compiler
        .type_tracker
        .get_local_binding_semantics(3)
        .expect("outer semantics exist");

    let result = compiler.with_callable_reference_flow_transaction(
        "ok_fn",
        Span::DUMMY,
        |compiler| {
            assert!(!compiler.reference_value_locals.contains(&3));
            assert_eq!(
                compiler
                    .type_tracker
                    .get_local_binding_semantics(3)
                    .map(|semantics| semantics.storage_class),
                Some(BindingStorageClass::Direct),
            );
            compiler.type_tracker.clear_locals();
            compiler.type_tracker.set_local_binding_semantics(
                3,
                semantics(BindingStorageClass::Direct),
            );
            compiler.set_reference_flow_class(
                BindingKey::Local(3),
                ReferenceClass::ExclusiveReference {
                    referent: Some(ConcreteType::String),
                },
            );
            Ok(17)
        },
    );

    assert_eq!(result.expect("local-only transaction succeeds"), 17);
    assert_eq!(compiler.reference_flow_snapshot(), expected_flow);
    assert_eq!(
        compiler.type_tracker.get_local_binding_semantics(3),
        Some(&expected_semantics),
    );
}

#[test]
fn failed_transaction_preserves_original_error_and_restores_all_state() {
    let mut compiler = compiler_with_named_slots();
    let expected_flow = compiler.reference_flow_snapshot();
    let expected_semantics = *compiler
        .type_tracker
        .get_local_binding_semantics(3)
        .expect("outer semantics exist");

    let error = compiler
        .with_callable_reference_flow_transaction(
            "bad_fn",
            Span::DUMMY,
            |compiler| {
                compiler.type_tracker.clear_locals();
                compiler.type_tracker.set_local_binding_semantics(
                    3,
                    semantics(BindingStorageClass::Direct),
                );
                compiler.set_reference_flow_class(
                    BindingKey::Local(3),
                    ReferenceClass::ExclusiveReference {
                        referent: Some(ConcreteType::String),
                    },
                );
                compiler.set_reference_flow_class(
                    BindingKey::ModuleBinding(7),
                    ReferenceClass::SharedReference {
                        referent: Some(ConcreteType::I64),
                    },
                );
                Err::<(), _>(ShapeError::SemanticError {
                    message: "original compile error".to_string(),
                    location: None,
                })
            },
        )
        .expect_err("the original compile error must survive");

    assert_eq!(semantic_message(error), "original compile error");
    assert_eq!(compiler.reference_flow_snapshot(), expected_flow);
    assert_eq!(
        compiler.type_tracker.get_local_binding_semantics(3),
        Some(&expected_semantics),
    );
}

#[test]
fn successful_module_reference_transition_is_c0912_and_uses_callable_span() {
    let mut compiler = compiler_with_named_slots();
    compiler.set_source_with_file("first\ncallable\n", "transaction.shape");
    let expected = compiler.reference_flow_snapshot();

    let error = compiler
        .with_callable_reference_flow_transaction(
            "changes_module",
            Span { start: 6, end: 14 },
            |compiler| {
                compiler.set_reference_flow_class(
                    BindingKey::ModuleBinding(7),
                    ReferenceClass::SharedReference {
                        referent: Some(ConcreteType::I64),
                    },
                );
                Ok(())
            },
        )
        .expect_err("module representation effects require a summary");

    match error {
        ShapeError::SemanticError { message, location } => {
            assert!(message.starts_with(
                "[C0912] exact reference-flow conflict at callable 'changes_module' for \
                 ModuleBinding(7) (name 'module_ref')"
            ));
            let location = location.expect("function name span is the fallback anchor");
            assert_eq!(location.file.as_deref(), Some("transaction.shape"));
            assert_eq!((location.line, location.column), (2, 1));
        }
        other => panic!("expected semantic C0912, got {other:?}"),
    }
    assert_eq!(compiler.reference_flow_snapshot(), expected);
}

#[test]
fn unchanged_module_and_value_storage_planning_both_pass() {
    let mut compiler = compiler_with_named_slots();
    let expected = compiler.reference_flow_snapshot();

    compiler
        .with_callable_reference_flow_transaction(
            "unchanged",
            Span::DUMMY,
            |_compiler| Ok(()),
        )
        .expect("an unchanged module projection passes");
    compiler
        .with_callable_reference_flow_transaction(
            "planning_only",
            Span::DUMMY,
            |compiler| {
                compiler
                    .type_tracker
                    .set_binding_storage_class(7, BindingStorageClass::SharedCow);
                Ok(())
            },
        )
        .expect("Value Direct to SharedCow is planning, not representation");

    assert_eq!(compiler.reference_flow_snapshot(), expected);
}

#[test]
fn referent_mode_and_reference_storage_changes_each_reject() {
    let mut compiler = compiler_with_named_slots();
    compiler.set_reference_flow_class(
        BindingKey::ModuleBinding(7),
        ReferenceClass::SharedReference {
            referent: Some(ConcreteType::I64),
        },
    );
    let expected = compiler.reference_flow_snapshot();

    let value = compiler
        .with_callable_reference_flow_transaction(
            "reference_to_value",
            Span::DUMMY,
            |compiler| {
                compiler.set_reference_flow_class(
                    BindingKey::ModuleBinding(7),
                    ReferenceClass::Value,
                );
                Ok(())
            },
        )
        .expect_err("reference to Value must reject");
    assert!(semantic_message(value).contains("Value [storage=Direct]"));

    let referent = compiler
        .with_callable_reference_flow_transaction(
            "referent_change",
            Span::DUMMY,
            |compiler| {
                compiler.set_reference_flow_referent(
                    BindingKey::ModuleBinding(7),
                    Some(ConcreteType::String),
                );
                Ok(())
            },
        )
        .expect_err("referent change must reject");
    assert!(semantic_message(referent).contains("SharedReference<I64>"));

    let mode = compiler
        .with_callable_reference_flow_transaction(
            "mode_change",
            Span::DUMMY,
            |compiler| {
                compiler.set_reference_flow_class(
                    BindingKey::ModuleBinding(7),
                    ReferenceClass::ExclusiveReference {
                        referent: Some(ConcreteType::I64),
                    },
                );
                Ok(())
            },
        )
        .expect_err("shared to exclusive must reject");
    assert!(semantic_message(mode).contains("ExclusiveReference<I64>"));

    let storage = compiler
        .with_callable_reference_flow_transaction(
            "storage_change",
            Span::DUMMY,
            |compiler| {
                compiler
                    .type_tracker
                    .set_binding_storage_class(7, BindingStorageClass::Direct);
                Ok(())
            },
        )
        .expect_err("Reference storage inconsistency must reject");
    assert!(semantic_message(storage).contains("storage=Direct"));

    assert_eq!(compiler.reference_flow_snapshot(), expected);
}

#[test]
fn same_slot_inner_semantics_cannot_poison_outer_scope_order() {
    let mut compiler = BytecodeCompiler::new();
    let outer = BindingSemantics::deferred(BindingOwnershipClass::OwnedImmutable);
    let inner = BindingSemantics::deferred(BindingOwnershipClass::OwnedMutable);
    compiler
        .locals
        .last_mut()
        .expect("initial local scope")
        .insert("outer_zero".to_string(), 0);
    compiler
        .type_tracker
        .set_local_binding_semantics(0, outer);
    compiler.type_tracker.push_scope();

    compiler
        .with_callable_reference_flow_transaction(
            "same_slot",
            Span::DUMMY,
            |compiler| {
                compiler.type_tracker.clear_locals();
                compiler.type_tracker.push_scope();
                compiler
                    .type_tracker
                    .set_local_binding_semantics(0, inner);
                compiler.set_reference_flow_class(
                    BindingKey::Local(0),
                    ReferenceClass::SharedReference { referent: None },
                );
                Ok(())
            },
        )
        .expect("local slot reuse is isolated");

    assert_eq!(compiler.type_tracker.get_local_binding_semantics(0), Some(&outer));
    compiler.type_tracker.pop_scope();
    assert_eq!(compiler.type_tracker.get_local_binding_semantics(0), Some(&outer));
}
