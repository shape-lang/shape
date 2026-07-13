//! The 9-test frozen-identity matrix (ongoing identity-stability tripwire).
//!
//! S2: constructions go through the single freeze barrier
//! (`SemanticFreeze::freeze` over a populated compiler) — the old
//! `TypeReflectionSnapshot::default()` + field-poking pattern is deleted
//! with the per-site rebuild. Identity semantics under test are unchanged.

use super::*;
use crate::compiler::BytecodeCompiler;
use crate::compiler::comptime_builtins::semantic_freeze::{FreezeOverlay, SemanticFreeze};
use std::sync::Arc;

fn freeze_of(configure: impl FnOnce(&mut BytecodeCompiler)) -> Arc<SemanticFreeze> {
    let mut compiler = BytecodeCompiler::new();
    configure(&mut compiler);
    SemanticFreeze::freeze(&compiler).expect("test compiler state must freeze")
}

fn add_alias(compiler: &mut BytecodeCompiler, alias: &str, target: &str) {
    compiler
        .type_aliases
        .insert(alias.to_string(), target.to_string());
}

fn add_struct(compiler: &mut BytecodeCompiler, name: &str) {
    compiler
        .struct_types
        .insert(name.to_string(), (Vec::new(), shape_ast::ast::Span::DUMMY));
}

#[test]
fn aliases_reuse_stable_underlying_identity() {
    let freeze = freeze_of(|compiler| add_alias(compiler, "UserId", "int"));
    let int_identity = freeze.identity_of("int").expect("int identity");
    assert_eq!(freeze.identity_of("UserId"), Some(int_identity));

    // Adding an unrelated nominal cannot renumber an existing identity.
    let grown = freeze_of(|compiler| {
        add_alias(compiler, "UserId", "int");
        add_struct(compiler, "Unrelated");
    });
    assert_eq!(grown.identity_of("int"), Some(int_identity));
    assert_eq!(grown.identity_of("UserId"), Some(int_identity));
}

#[test]
fn declared_parameter_receives_parameter_identity() {
    let freeze = freeze_of(|_| {});
    let overlay = FreezeOverlay::new(freeze, "map", &["T".to_string()]);

    let identity = overlay.identity_of("T").expect("T identity");
    assert_eq!(
        overlay.category_of(identity),
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
    let mut compiler = BytecodeCompiler::new();
    compiler
        .register_function(definition)
        .expect("generic signature registers");
    compiler.current_function = compiler
        .program
        .functions
        .iter()
        .position(|function| function.name == "identity");
    compiler
        .install_semantic_freeze()
        .expect("registration-complete state freezes");

    let overlay = compiler
        .comptime_freeze_overlay()
        .expect("post-barrier site obtains the handle");
    let identity = overlay.identity_of("T").expect("active T identity");
    assert_eq!(
        overlay.category_of(identity),
        Ok(FrozenTypeCategory::Parameter)
    );
}

#[test]
fn reachable_builtin_categories_match_the_closed_catalog() {
    let freeze = freeze_of(|_| {});

    for (name, expected) in [
        ("int", FrozenTypeCategory::Primitive),
        ("never", FrozenTypeCategory::Never),
        ("any", FrozenTypeCategory::Erased),
        ("Array", FrozenTypeCategory::Nominal),
    ] {
        let identity = freeze.identity_of(name).expect("known type identity");
        assert_eq!(freeze.category_of(identity), Ok(expected));
    }
}

#[test]
fn primitive_synonyms_share_identity_but_distinct_types_do_not() {
    let freeze = freeze_of(|_| {});

    assert_eq!(freeze.identity_of("int"), freeze.identity_of("i64"));
    assert_eq!(freeze.identity_of("number"), freeze.identity_of("f64"));
    assert_eq!(freeze.identity_of("unit"), freeze.identity_of("void"));
    assert_ne!(freeze.identity_of("int"), freeze.identity_of("bool"));
}

#[test]
fn alias_chains_reuse_the_terminal_identity() {
    let freeze = freeze_of(|compiler| {
        add_alias(compiler, "First", "Second");
        add_alias(compiler, "Second", "int");
    });

    let int_identity = freeze.identity_of("int");
    assert_eq!(freeze.identity_of("First"), int_identity);
    assert_eq!(freeze.identity_of("Second"), int_identity);
}

#[test]
fn nominal_identity_is_order_independent_stable_and_distinct() {
    let left = freeze_of(|compiler| {
        add_struct(compiler, "Alpha");
        add_struct(compiler, "Beta");
    });
    let right = freeze_of(|compiler| {
        add_struct(compiler, "Beta");
        add_struct(compiler, "Alpha");
    });

    assert_eq!(left.identity_of("Alpha"), right.identity_of("Alpha"));
    assert_eq!(left.identity_of("Beta"), right.identity_of("Beta"));
    assert_ne!(left.identity_of("Alpha"), left.identity_of("Beta"));

    let alpha = left.identity_of("Alpha");
    let grown = freeze_of(|compiler| {
        add_struct(compiler, "Alpha");
        add_struct(compiler, "Beta");
        add_struct(compiler, "Unrelated");
    });
    assert_eq!(grown.identity_of("Alpha"), alpha);
}

#[test]
fn parameter_identity_is_scoped_by_owning_function() {
    let parameter_identity = |owner: &str| {
        let overlay = FreezeOverlay::new(freeze_of(|_| {}), owner, &["T".to_string()]);
        overlay.identity_of("T").expect("T identity")
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
// declared type params to the freeze overlay when the active (mono) def
// carries `type_params = None`, and the Parameter identity is scoped to the
// BASE function name, never the mono key (declaration stability across
// instantiations). Post-A1 merge this goes through the single freeze barrier
// + `comptime_freeze_overlay` (the per-site snapshot builder is deleted).
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
    compiler
        .install_semantic_freeze()
        .expect("registration-complete state freezes");

    // Without the overlay, the mono def exposes no type params.
    let bare = compiler
        .comptime_freeze_overlay()
        .expect("post-barrier site obtains the handle");
    assert_eq!(bare.identity_of("T"), None);

    // With the overlay, T resolves to a Parameter identity owned by the BASE name.
    compiler.specialization_type_param_overlay =
        Some(("describe".to_string(), vec!["T".to_string()]));
    let overlay = compiler
        .comptime_freeze_overlay()
        .expect("post-barrier site obtains the handle");
    let identity = overlay.identity_of("T").expect("overlay T identity");
    assert_eq!(
        overlay.category_of(identity),
        Ok(FrozenTypeCategory::Parameter)
    );

    // Identity contract: the canonical descriptor is scoped to the BASE fn
    // name, never the mono key (same `parameter:{owner}:{name}` grammar the
    // freeze overlay interns).
    assert_eq!(
        identity,
        FrozenTypeIdentity::from_canonical_descriptor("parameter:describe:T"),
        "overlay Parameter identity must be scoped to the BASE fn name"
    );
    assert_ne!(
        identity,
        FrozenTypeIdentity::from_canonical_descriptor("parameter:describe::i64:T"),
        "overlay Parameter identity must NOT be scoped to the mono key"
    );
}

// ADR-009 A3 (S3): on the specialization path, the overlay-derived Parameter
// identity is STABLE across instantiations of one base generic function
// (identity::i64 and identity::string agree — owner is the BASE name, so two
// mono compiles intern the same "parameter:{base}:{name}" descriptor) and
// DISTINCT across owning functions. Never renumber existing identities — the
// SHA-256 descriptor scheme is fixed; any red here means owner scoping in
// cache.rs/semantic_freeze.rs state threading is wrong.
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
        compiler
            .install_semantic_freeze()
            .expect("registration-complete state freezes");
        compiler.specialization_type_param_overlay =
            Some((base_name.to_string(), vec!["T".to_string()]));
        let overlay = compiler
            .comptime_freeze_overlay()
            .expect("post-barrier site obtains the handle");
        overlay.identity_of("T").expect("overlay T identity")
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
    let freeze = freeze_of(|_| {});

    assert_eq!(
        freeze.category_of(FrozenTypeIdentity::INVALID),
        Err("type_ref received an unknown semantic type identity".to_string())
    );
}

/// ADR-009 §4.1 "one kind vocabulary" (ticket A1, slice S5): confinement
/// sentinel for the legacy `type_info` vocabulary.
///
/// `TypeKindLabel` / `classify_legacy_type_info` / `build_type_info_heap_value`
/// and the `__ComptimeTypeInfo` schema survive ONLY on the legacy `type_info`
/// intrinsic path until ticket E5 deletes them. This sentinel (file-read, same
/// pattern as `executor/tests/no_dynamic.rs`) pins that confinement so the
/// vocabulary cannot silently re-spread into the semantic-freeze module, the
/// shared runtime reflection catalog, or a crate-wide re-export before E5.
#[test]
fn legacy_type_info_vocabulary_is_confined_to_the_legacy_intrinsic_path() {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let read = |relative: &str| {
        std::fs::read_to_string(manifest.join(relative))
            .unwrap_or_else(|error| panic!("sentinel could not read {relative}: {error}"))
    };

    const LEGACY_VOCABULARY: [&str; 4] = [
        "TypeKindLabel",
        "classify_legacy_type_info",
        "build_type_info_heap_value",
        "__ComptimeTypeInfo",
    ];

    // 1. The semantic-freeze module and the shared runtime catalog must not
    //    mention the legacy vocabulary at all (not even in comments — the new
    //    surface describes the legacy path by ticket, never by symbol).
    for relative in [
        "src/compiler/comptime_builtins/semantic_freeze.rs",
        "../shape-runtime/src/comptime_reflection.rs",
    ] {
        let source = read(relative);
        for symbol in LEGACY_VOCABULARY {
            assert!(
                !source.contains(symbol),
                "legacy type_info vocabulary `{symbol}` leaked into {relative} \
                 (E5-deletes territory; confined to the legacy type_info path)"
            );
        }
    }

    // 2. No crate-wide re-export: in `comptime_builtins.rs`,
    //    `build_type_info_heap_value` may appear only path-qualified at the
    //    single legacy `type_info` intrinsic call site (or in comments),
    //    never inside a `use` list.
    let parent = read("src/compiler/comptime_builtins.rs");
    for (index, _) in parent.match_indices("build_type_info_heap_value") {
        let qualified = parent[..index].ends_with("type_reflection::");
        let line_start = parent[..index].rfind('\n').map_or(0, |p| p + 1);
        let comment = parent[line_start..index].trim_start().starts_with("//");
        assert!(
            qualified || comment,
            "`build_type_info_heap_value` must stay confined: every non-comment \
             use in comptime_builtins.rs must be path-qualified \
             `type_reflection::build_type_info_heap_value` at the legacy \
             type_info intrinsic (found an unqualified occurrence, e.g. a \
             crate-wide `use` re-export)"
        );
    }

    // 3. Visibility confinement: the legacy builder is `pub(super)` (reachable
    //    from the parent intrinsic-registration module only), never
    //    `pub(crate)`.
    let reflection = read("src/compiler/comptime_builtins/type_reflection.rs");
    assert!(
        reflection.contains("pub(super) fn build_type_info_heap_value"),
        "build_type_info_heap_value must be pub(super) (legacy-path confinement)"
    );

    // 4. E5-deletes markers: each confined symbol's definition site carries
    //    the `E5-deletes` confinement marker for grep visibility.
    assert!(
        reflection.contains("E5-deletes"),
        "type_reflection.rs must carry the E5-deletes confinement marker"
    );
    let schemas = read("../shape-runtime/src/type_schema/builtin_schemas.rs");
    assert!(
        schemas.contains("E5-deletes"),
        "builtin_schemas.rs __ComptimeTypeInfo registration must carry the \
         E5-deletes confinement marker"
    );
}

// ── ADR-009 (ticket B2, slice S2): trait/impl identities enter the SAME
// canonical SHA-256 descriptor scheme, as a DISTINCT identity kind. ──

/// Dec 49: canonical trait and impl identities are reproducible descriptor
/// hashes (`trait:{name}` / `impl:{trait}:{type}:{impl_name_or_default}`),
/// never counter-allocated, and each descriptor space is disjoint.
#[test]
fn trait_and_impl_descriptors_enter_the_canonical_identity_scheme() {
    let trait_identity = FrozenTypeIdentity::for_trait("Greetable");
    assert_eq!(
        trait_identity,
        FrozenTypeIdentity::from_canonical_descriptor("trait:Greetable")
    );
    // A trait is not a value type: same name, disjoint descriptor space.
    assert_ne!(
        trait_identity,
        FrozenTypeIdentity::from_canonical_descriptor("nominal:Greetable")
    );

    let default_impl = FrozenTypeIdentity::for_impl("Greetable", "User", None);
    assert_eq!(
        default_impl,
        FrozenTypeIdentity::from_canonical_descriptor("impl:Greetable:User:__default__")
    );
    let named_impl = FrozenTypeIdentity::for_impl("Greetable", "User", Some("Loud"));
    assert_eq!(
        named_impl,
        FrozenTypeIdentity::from_canonical_descriptor("impl:Greetable:User:Loud")
    );
    assert_ne!(default_impl, named_impl);
    assert_ne!(default_impl, trait_identity);
    assert_ne!(named_impl, trait_identity);
}

/// Dec 50 rule 5 structural pin: traits use `TraitRef` — there is NO
/// `FrozenTypeCategory::Trait` variant, and none may be added.
#[test]
fn frozen_type_category_has_no_trait_variant() {
    assert!(
        FrozenTypeCategory::ALL
            .iter()
            .all(|category| category.variant_name() != "Trait"),
        "Dec 50 rule 5: traits are not FrozenType categories"
    );
}
