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

// ADR-009 A3 (Wave-46 gap #4): compiling (not fabricating) a specialization of
// a generic function whose body reflects on its own declared type parameter
// must succeed — the declared base type params reach the reflection snapshot
// during the specialized-body compile at monomorphization time.
#[test]
fn specialized_generic_body_resolves_own_type_param_through_comptime() {
    // Control: the same comptime reflection at module scope compiles with a
    // bare BytecodeCompiler — proves any failure below is specialization-
    // specific, not test-harness setup.
    let control = r#"
let control_label = comptime {
  match type_category(type_ref(int)) {
    FrozenTypeCategory::Primitive => "primitive"
    _ => "other"
  }
}
"#;
    let control_program = shape_ast::parse_program(control).expect("control parses");
    let control_compiler = crate::compiler::BytecodeCompiler::new();
    control_compiler
        .compile(&control_program)
        .expect("module-scope comptime reflection compiles with a bare compiler");

    let source = r#"
fn describe<T>(value: T) -> string {
  let label = comptime {
    match type_category(type_ref(T)) {
      FrozenTypeCategory::Parameter => "parameter"
      _ => "other"
    }
  }
  label
}

let result = describe(1)
"#;
    let program = shape_ast::parse_program(source).expect("generic program parses");
    let compiler = crate::compiler::BytecodeCompiler::new();
    let bytecode = compiler
        .compile(&program)
        .expect("specialized generic body with type_ref(T) compiles");
    assert!(
        bytecode
            .functions
            .iter()
            .any(|function| function.name.starts_with("describe::")),
        "expected a registered specialization of 'describe', got: {:?}",
        bytecode
            .functions
            .iter()
            .map(|function| function.name.clone())
            .collect::<Vec<_>>()
    );
}

// ADR-009 A3 — the specialization overlay supplies the BASE generic function's
// declared type params to the snapshot when the active (mono) def carries
// `type_params = None`, and the Parameter identity is scoped to the BASE
// function name, never the mono key (declaration stability across
// instantiations).
#[test]
fn specialization_overlay_supplies_base_scoped_parameter_identity() {
    let program = shape_ast::parse_program("fn describe(value: int) -> int { value }")
        .expect("mono-shaped function parses");
    let shape_ast::ast::Item::Function(definition, _) = &program.items[0] else {
        panic!("expected function item");
    };
    // Register under a mono-key name with type_params = None, exactly like a
    // substituted specialization.
    let mut mono_def = definition.clone();
    mono_def.name = "describe::i64".to_string();
    assert!(mono_def.type_params.is_none());
    let mut compiler = crate::compiler::BytecodeCompiler::new();
    compiler
        .register_function(&mono_def)
        .expect("mono def registers");
    compiler.current_function = compiler
        .program
        .functions
        .iter()
        .position(|function| function.name == "describe::i64");

    // Without the overlay, the mono def exposes no type params.
    let bare = build_type_reflection_snapshot(&compiler, &[]);
    assert_eq!(bare.frozen_type_id("T"), None);

    // With the overlay, T resolves to a Parameter identity owned by the BASE name.
    compiler.specialization_type_param_overlay =
        Some(("describe".to_string(), vec!["T".to_string()]));
    let snapshot = build_type_reflection_snapshot(&compiler, &[]);
    let identity = snapshot.frozen_type_id("T").expect("overlay T identity");
    assert_eq!(
        snapshot.category_for_identity(identity),
        Ok(FrozenTypeCategory::Parameter)
    );

    let identity_for_owner = |owner: &str| {
        let mut fabricated = TypeReflectionSnapshot {
            parameter_owner: Some(owner.to_string()),
            ..TypeReflectionSnapshot::default()
        };
        fabricated.known_type_params.insert("T".to_string());
        fabricated.rebuild_frozen_type_index();
        fabricated.frozen_type_id("T").expect("T identity")
    };
    assert_eq!(
        identity,
        identity_for_owner("describe"),
        "overlay Parameter identity must be scoped to the BASE fn name"
    );
    assert_ne!(
        identity,
        identity_for_owner("describe::i64"),
        "overlay Parameter identity must NOT be scoped to the mono key"
    );
}

// ADR-009 A3 (S3): on the specialization path, the overlay-derived Parameter
// identity is STABLE across instantiations of one base generic function
// (identity::i64 and identity::string agree — owner is the BASE name, so two
// mono compiles intern the same "parameter:{base}:{name}" descriptor) and
// DISTINCT across owning functions. Never renumber existing identities — the
// SHA-256 descriptor scheme is fixed; any red here means owner scoping in
// cache.rs/type_reflection.rs state threading is wrong.
#[test]
fn specialization_overlay_identity_is_stable_across_instantiations() {
    let overlay_identity = |mono_name: &str, base_name: &str| {
        let program = shape_ast::parse_program("fn placeholder(value: int) -> int { value }")
            .expect("mono-shaped function parses");
        let shape_ast::ast::Item::Function(definition, _) = &program.items[0] else {
            panic!("expected function item");
        };
        let mut mono_def = definition.clone();
        mono_def.name = mono_name.to_string();
        assert!(mono_def.type_params.is_none());
        let mut compiler = crate::compiler::BytecodeCompiler::new();
        compiler
            .register_function(&mono_def)
            .expect("mono def registers");
        compiler.current_function = compiler
            .program
            .functions
            .iter()
            .position(|function| function.name == mono_name);
        compiler.specialization_type_param_overlay =
            Some((base_name.to_string(), vec!["T".to_string()]));
        let snapshot = build_type_reflection_snapshot(&compiler, &[]);
        snapshot.frozen_type_id("T").expect("overlay T identity")
    };

    // Same base, different instantiations: identical identity.
    assert_eq!(
        overlay_identity("identity::i64", "identity"),
        overlay_identity("identity::string", "identity"),
        "Parameter identity must be stable across instantiations of one base fn"
    );
    // Different owning functions: distinct identities.
    assert_ne!(
        overlay_identity("identity::i64", "identity"),
        overlay_identity("filter::i64", "filter"),
        "Parameter identities must be distinct across owning functions"
    );
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
