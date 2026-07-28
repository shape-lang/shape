use super::*;

#[test]
fn first_class_reference_values_preserve_mode_and_optional_referent() {
    let mut compiler = compiler_with_named_slots();
    let cases = [
        (
            BindingKey::Local(3),
            ReferenceClass::SharedReference {
                referent: Some(ConcreteType::I64),
            },
        ),
        (
            BindingKey::Local(3),
            ReferenceClass::ExclusiveReference { referent: None },
        ),
        (
            BindingKey::ModuleBinding(7),
            ReferenceClass::SharedReference { referent: None },
        ),
        (
            BindingKey::ModuleBinding(7),
            ReferenceClass::ExclusiveReference {
                referent: Some(ConcreteType::String),
            },
        ),
    ];

    for (key, class) in cases {
        compiler.set_reference_flow_class(key, class.clone());
        assert_eq!(compiler.current_true_reference_class(key), Some(class));
        compiler.set_reference_flow_class(key, ReferenceClass::Value);
        assert_eq!(compiler.current_true_reference_class(key), None);
    }
}

#[test]
fn explicit_reference_parameters_are_true_references_with_unknown_referents() {
    let mut shared = compiler_with_named_slots();
    shared.ref_locals.insert(3);
    assert_eq!(
        shared.current_true_reference_class(BindingKey::Local(3)),
        Some(ReferenceClass::SharedReference { referent: None })
    );

    let mut exclusive = compiler_with_named_slots();
    exclusive.ref_locals.insert(3);
    exclusive.exclusive_ref_locals.insert(3);
    assert_eq!(
        exclusive.current_true_reference_class(BindingKey::Local(3)),
        Some(ReferenceClass::ExclusiveReference { referent: None })
    );
}

#[test]
fn inferred_shared_and_exclusive_reference_parameters_remain_owned_values() {
    for exclusive in [false, true] {
        let mut compiler = compiler_with_named_slots();
        compiler.ref_locals.insert(3);
        compiler.inferred_ref_locals.insert(3);
        if exclusive {
            compiler.exclusive_ref_locals.insert(3);
        }

        assert_eq!(
            compiler.current_true_reference_class(BindingKey::Local(3)),
            None,
            "inferred exclusive={exclusive} must not become true-reference evidence"
        );
    }
}

#[test]
fn first_class_reference_value_precedes_parameter_abi_markers() {
    let mut compiler = compiler_with_named_slots();
    compiler.ref_locals.insert(3);
    compiler.exclusive_ref_locals.insert(3);
    compiler.set_reference_flow_class(
        BindingKey::Local(3),
        ReferenceClass::SharedReference {
            referent: Some(ConcreteType::I64),
        },
    );

    assert_eq!(
        compiler.current_true_reference_class(BindingKey::Local(3)),
        Some(ReferenceClass::SharedReference {
            referent: Some(ConcreteType::I64),
        })
    );
}

#[test]
fn same_spelled_local_and_module_bindings_do_not_cross_contaminate_slots() {
    let mut compiler = compiler_with_named_slots();
    compiler
        .locals
        .last_mut()
        .expect("compiler has a local scope")
        .insert("shadowed".to_string(), 3);
    compiler
        .module_bindings
        .insert("shadowed".to_string(), 7);

    compiler.set_reference_flow_class(
        BindingKey::Local(3),
        ReferenceClass::SharedReference { referent: None },
    );
    assert!(compiler
        .current_true_reference_class(BindingKey::Local(3))
        .is_some());
    assert_eq!(
        compiler.current_true_reference_class(BindingKey::ModuleBinding(7)),
        None
    );

    compiler.set_reference_flow_class(BindingKey::Local(3), ReferenceClass::Value);
    compiler.set_reference_flow_class(
        BindingKey::ModuleBinding(7),
        ReferenceClass::ExclusiveReference { referent: None },
    );
    assert_eq!(
        compiler.current_true_reference_class(BindingKey::Local(3)),
        None
    );
    assert!(compiler
        .current_true_reference_class(BindingKey::ModuleBinding(7))
        .is_some());
}
