use super::*;

#[test]
fn aliases_reuse_stable_underlying_identity() {
    let mut snapshot = TypeReflectionSnapshot::default();
    snapshot.alias_defs.insert(
        "UserId".to_string(),
        TypeAnnotation::Basic("int".to_string()),
    );
    snapshot.rebuild_frozen_type_index();

    let int_identity = snapshot.frozen_type_id("int").expect("int identity");
    assert_eq!(snapshot.frozen_type_id("UserId"), Some(int_identity));

    snapshot
        .struct_defs
        .insert("Unrelated".to_string(), Vec::new());
    snapshot.rebuild_frozen_type_index();
    assert_eq!(snapshot.frozen_type_id("int"), Some(int_identity));
    assert_eq!(snapshot.frozen_type_id("UserId"), Some(int_identity));
}

#[test]
fn declared_parameter_receives_parameter_identity() {
    let mut snapshot = TypeReflectionSnapshot {
        parameter_owner: Some("map".to_string()),
        ..TypeReflectionSnapshot::default()
    };
    snapshot.known_type_params.insert("T".to_string());
    snapshot.rebuild_frozen_type_index();

    let identity = snapshot.frozen_type_id("T").expect("T identity");
    assert_eq!(
        snapshot.category_for_identity(identity),
        Ok(FrozenTypeCategory::Parameter)
    );
}

#[test]
fn active_function_type_parameters_are_discovered_from_compiler_state() {
    let program = shape_ast::parse_program("fn identity<T>(value: T) -> T { value }")
        .expect("generic function parses");
    let shape_ast::ast::Item::Function(definition, _) = &program.items[0] else {
        panic!("expected function item");
    };
    let mut compiler = crate::compiler::BytecodeCompiler::new();
    compiler
        .register_function(definition)
        .expect("generic signature registers");
    compiler.current_function = compiler
        .program
        .functions
        .iter()
        .position(|function| function.name == "identity");

    let snapshot = build_type_reflection_snapshot(&compiler, &[]);
    let identity = snapshot.frozen_type_id("T").expect("active T identity");
    assert_eq!(
        snapshot.category_for_identity(identity),
        Ok(FrozenTypeCategory::Parameter)
    );
}

#[test]
fn reachable_builtin_categories_match_the_closed_catalog() {
    let mut snapshot = TypeReflectionSnapshot::default();
    snapshot.rebuild_frozen_type_index();

    for (name, expected) in [
        ("int", FrozenTypeCategory::Primitive),
        ("never", FrozenTypeCategory::Never),
        ("any", FrozenTypeCategory::Erased),
        ("Array", FrozenTypeCategory::Nominal),
    ] {
        let identity = snapshot.frozen_type_id(name).expect("known type identity");
        assert_eq!(snapshot.category_for_identity(identity), Ok(expected));
    }
}

#[test]
fn primitive_synonyms_share_identity_but_distinct_types_do_not() {
    let mut snapshot = TypeReflectionSnapshot::default();
    snapshot.rebuild_frozen_type_index();

    assert_eq!(
        snapshot.frozen_type_id("int"),
        snapshot.frozen_type_id("i64")
    );
    assert_eq!(
        snapshot.frozen_type_id("number"),
        snapshot.frozen_type_id("f64")
    );
    assert_eq!(
        snapshot.frozen_type_id("unit"),
        snapshot.frozen_type_id("void")
    );
    assert_ne!(
        snapshot.frozen_type_id("int"),
        snapshot.frozen_type_id("bool")
    );
}

#[test]
fn alias_chains_reuse_the_terminal_identity() {
    let mut snapshot = TypeReflectionSnapshot::default();
    snapshot.alias_defs.insert(
        "First".to_string(),
        TypeAnnotation::Basic("Second".to_string()),
    );
    snapshot.alias_defs.insert(
        "Second".to_string(),
        TypeAnnotation::Basic("int".to_string()),
    );
    snapshot.rebuild_frozen_type_index();

    let int_identity = snapshot.frozen_type_id("int");
    assert_eq!(snapshot.frozen_type_id("First"), int_identity);
    assert_eq!(snapshot.frozen_type_id("Second"), int_identity);
}

#[test]
fn nominal_identity_is_order_independent_stable_and_distinct() {
    let mut left = TypeReflectionSnapshot::default();
    left.struct_defs.insert("Alpha".to_string(), Vec::new());
    left.struct_defs.insert("Beta".to_string(), Vec::new());
    left.rebuild_frozen_type_index();

    let mut right = TypeReflectionSnapshot::default();
    right.struct_defs.insert("Beta".to_string(), Vec::new());
    right.struct_defs.insert("Alpha".to_string(), Vec::new());
    right.rebuild_frozen_type_index();

    assert_eq!(left.frozen_type_id("Alpha"), right.frozen_type_id("Alpha"));
    assert_eq!(left.frozen_type_id("Beta"), right.frozen_type_id("Beta"));
    assert_ne!(left.frozen_type_id("Alpha"), left.frozen_type_id("Beta"));

    let alpha = left.frozen_type_id("Alpha");
    left.struct_defs.insert("Unrelated".to_string(), Vec::new());
    left.rebuild_frozen_type_index();
    assert_eq!(left.frozen_type_id("Alpha"), alpha);
}

#[test]
fn parameter_identity_is_scoped_by_owning_function() {
    let parameter_identity = |owner: &str| {
        let mut snapshot = TypeReflectionSnapshot {
            parameter_owner: Some(owner.to_string()),
            ..TypeReflectionSnapshot::default()
        };
        snapshot.known_type_params.insert("T".to_string());
        snapshot.rebuild_frozen_type_index();
        snapshot.frozen_type_id("T").expect("T identity")
    };

    assert_eq!(parameter_identity("map"), parameter_identity("map"));
    assert_ne!(parameter_identity("map"), parameter_identity("filter"));
}

#[test]
fn unknown_identity_is_rejected_at_the_freeze_boundary() {
    let mut snapshot = TypeReflectionSnapshot::default();
    snapshot.rebuild_frozen_type_index();

    assert_eq!(
        snapshot.category_for_identity(FrozenTypeIdentity::INVALID),
        Err("type_ref received an unknown semantic type identity".to_string())
    );
}
