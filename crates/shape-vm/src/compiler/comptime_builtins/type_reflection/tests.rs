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

/// ADR-009 A2 (S5): trait names are a named semantic-freeze input — this is
/// the exact store (`known_traits`) the production predeclare pass writes.
fn add_trait(compiler: &mut BytecodeCompiler, name: &str) {
    compiler.known_traits.insert(name.to_string());
}

/// ADR-009 A2 (S5): a struct with declared generic arity, registered exactly
/// as `predeclare_struct_schema` does (`struct_types` row +
/// `struct_generic_info.type_params`).
fn add_generic_struct(compiler: &mut BytecodeCompiler, name: &str, params: &[&str]) {
    add_struct(compiler, name);
    compiler.struct_generic_info.insert(
        name.to_string(),
        crate::compiler::StructGenericInfo {
            type_params: params
                .iter()
                .map(|param| shape_ast::ast::TypeParam::Type {
                    name: (*param).to_string(),
                    span: shape_ast::ast::Span::DUMMY,
                    doc_comment: None,
                    default_type: None,
                    trait_bounds: Vec::new(),
                })
                .collect(),
            runtime_field_types: std::collections::HashMap::new(),
        },
    );
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

// ============================================================================
// ADR-009 A2 (slice S1): composite descriptor/identity canonicalizer matrix.
//
// One canonicalizer (`canonicalize_type_annotation`) maps a resolved
// `TypeAnnotation` to `(canonical descriptor, FrozenTypeCategory,
// FrozenTypeIdentity)`. Leaves resolve ONLY through the freeze/overlay query
// API (alias, synonym and parameter normalization inherited, never
// re-implemented); composites hash a deterministic descriptor grammar that is
// declaration-order independent. These tests are the identity-stability
// tripwire for the composite grammar (the B4/B7 ABI substrate).
// ============================================================================

use shape_ast::ast::{FunctionParam, ObjectTypeField, TypePath};
use shape_runtime::type_system::{TypeVar, tyvar_to_annotation};

fn basic(name: &str) -> TypeAnnotation {
    TypeAnnotation::Basic(name.to_string())
}

fn applied(head: &str, args: Vec<TypeAnnotation>) -> TypeAnnotation {
    TypeAnnotation::Generic {
        name: TypePath::simple(head),
        args,
    }
}

fn record_field(name: &str, optional: bool, annotation: TypeAnnotation) -> ObjectTypeField {
    ObjectTypeField {
        name: name.to_string(),
        optional,
        type_annotation: annotation,
        annotations: Vec::new(),
    }
}

fn callable(params: Vec<TypeAnnotation>, returns: TypeAnnotation) -> TypeAnnotation {
    TypeAnnotation::Function {
        params: params
            .into_iter()
            .map(|annotation| FunctionParam {
                name: None,
                optional: false,
                type_annotation: annotation,
            })
            .collect(),
        returns: Box::new(returns),
    }
}

fn module_overlay(configure: impl FnOnce(&mut BytecodeCompiler)) -> FreezeOverlay {
    FreezeOverlay::new(freeze_of(configure), "<module>", &[])
}

fn canon(overlay: &FreezeOverlay, annotation: &TypeAnnotation) -> CanonicalType {
    canonicalize_type_annotation(annotation, overlay)
        .unwrap_or_else(|error| panic!("annotation must canonicalize: {error}"))
}

fn leaf_hex(overlay: &FreezeOverlay, name: &str) -> String {
    identity_hex(
        overlay
            .identity_of(name)
            .unwrap_or_else(|| panic!("{name} must be frozen")),
    )
}

#[test]
fn tuple_descriptor_embeds_member_identity_hex_in_order() {
    let overlay = module_overlay(|_| {});

    let pair = canon(&overlay, &TypeAnnotation::Tuple(vec![basic("int"), basic("string")]));
    assert_eq!(pair.category, FrozenTypeCategory::Tuple);
    assert_eq!(
        pair.descriptor,
        format!(
            "tuple:[{},{}]",
            leaf_hex(&overlay, "int"),
            leaf_hex(&overlay, "string")
        )
    );
    assert_eq!(
        pair.identity,
        FrozenTypeIdentity::from_canonical_descriptor(&pair.descriptor)
    );

    // Member order is significant for tuples.
    let flipped = canon(
        &overlay,
        &TypeAnnotation::Tuple(vec![basic("string"), basic("int")]),
    );
    assert_ne!(pair.identity, flipped.identity);

    // Primitive-synonym normalization is inherited from the freeze: the
    // spelling `[i64, str]` reaches the identical identity.
    let synonyms = canon(&overlay, &TypeAnnotation::Tuple(vec![basic("i64"), basic("str")]));
    assert_eq!(pair.identity, synonyms.identity);
}

#[test]
fn record_identity_is_field_name_sorted_and_optionality_significant() {
    let overlay = module_overlay(|_| {});

    let xy = canon(
        &overlay,
        &TypeAnnotation::Object(vec![
            record_field("x", false, basic("int")),
            record_field("y", false, basic("number")),
        ]),
    );
    assert_eq!(xy.category, FrozenTypeCategory::Record);

    // Declaration-order independence: source field reorder is identity-neutral.
    let yx = canon(
        &overlay,
        &TypeAnnotation::Object(vec![
            record_field("y", false, basic("number")),
            record_field("x", false, basic("int")),
        ]),
    );
    assert_eq!(xy.identity, yx.identity);
    assert_eq!(xy.descriptor, yx.descriptor);
    assert_eq!(
        xy.descriptor,
        format!(
            "record:{{x:{},y:{}}}",
            leaf_hex(&overlay, "int"),
            leaf_hex(&overlay, "number")
        )
    );

    // Optionality is descriptor-significant: {x?: int} != {x: int}.
    let required = canon(
        &overlay,
        &TypeAnnotation::Object(vec![record_field("x", false, basic("int"))]),
    );
    let optional = canon(
        &overlay,
        &TypeAnnotation::Object(vec![record_field("x", true, basic("int"))]),
    );
    assert_ne!(required.identity, optional.identity);
    assert!(optional.descriptor.contains("x?:"), "{}", optional.descriptor);
}

#[test]
fn record_duplicate_field_names_are_rejected() {
    let overlay = module_overlay(|_| {});

    let error = canonicalize_type_annotation(
        &TypeAnnotation::Object(vec![
            record_field("x", false, basic("int")),
            record_field("x", false, basic("string")),
        ]),
        &overlay,
    )
    .expect_err("duplicate record fields must not canonicalize");
    assert!(error.contains("duplicate field"), "{error}");
    assert!(error.contains('x'), "must name the colliding field: {error}");
}

#[test]
fn callable_descriptor_is_positional_and_return_significant() {
    let overlay = module_overlay(|_| {});

    let int_string_to_bool = canon(
        &overlay,
        &callable(vec![basic("int"), basic("string")], basic("bool")),
    );
    assert_eq!(int_string_to_bool.category, FrozenTypeCategory::Callable);
    assert_eq!(
        int_string_to_bool.descriptor,
        format!(
            "callable:({},{})->{}",
            leaf_hex(&overlay, "int"),
            leaf_hex(&overlay, "string"),
            leaf_hex(&overlay, "bool")
        )
    );
    assert_eq!(
        int_string_to_bool.identity,
        FrozenTypeIdentity::from_canonical_descriptor(&int_string_to_bool.descriptor)
    );

    // Parameter order is significant.
    let string_int_to_bool = canon(
        &overlay,
        &callable(vec![basic("string"), basic("int")], basic("bool")),
    );
    assert_ne!(int_string_to_bool.identity, string_int_to_bool.identity);

    // Return type is significant.
    let int_string_to_int = canon(
        &overlay,
        &callable(vec![basic("int"), basic("string")], basic("int")),
    );
    assert_ne!(int_string_to_bool.identity, int_string_to_int.identity);
}

#[test]
fn reference_mutability_is_descriptor_significant() {
    let overlay = module_overlay(|_| {});

    let shared = canon(
        &overlay,
        &TypeAnnotation::Borrow {
            mutable: false,
            inner: Box::new(basic("int")),
        },
    );
    let exclusive = canon(
        &overlay,
        &TypeAnnotation::Borrow {
            mutable: true,
            inner: Box::new(basic("int")),
        },
    );
    assert_eq!(shared.category, FrozenTypeCategory::Reference);
    assert_eq!(exclusive.category, FrozenTypeCategory::Reference);
    assert_eq!(
        shared.descriptor,
        format!("reference:&{}", leaf_hex(&overlay, "int"))
    );
    assert_eq!(
        exclusive.descriptor,
        format!("reference:&mut {}", leaf_hex(&overlay, "int"))
    );
    assert_ne!(shared.identity, exclusive.identity);
}

#[test]
fn union_members_are_deduped_and_byte_sorted_and_singleton_collapses() {
    let overlay = module_overlay(|_| {});

    let int_or_string = canon(
        &overlay,
        &TypeAnnotation::Union(vec![basic("int"), basic("string")]),
    );
    assert_eq!(int_or_string.category, FrozenTypeCategory::Union);

    // Source member order is identity-neutral (members byte-sorted by their
    // identity-hex embedding).
    let string_or_int = canon(
        &overlay,
        &TypeAnnotation::Union(vec![basic("string"), basic("int")]),
    );
    assert_eq!(int_or_string.identity, string_or_int.identity);
    let mut hexes = vec![leaf_hex(&overlay, "int"), leaf_hex(&overlay, "string")];
    hexes.sort();
    assert_eq!(
        int_or_string.descriptor,
        format!("union:({})", hexes.join("|"))
    );

    // Duplicate members dedup: int | string | int == int | string.
    let with_duplicate = canon(
        &overlay,
        &TypeAnnotation::Union(vec![basic("int"), basic("string"), basic("int")]),
    );
    assert_eq!(int_or_string.identity, with_duplicate.identity);

    // A union whose members all coalesce IS its single member: int | i64 == int.
    let singleton = canon(
        &overlay,
        &TypeAnnotation::Union(vec![basic("int"), basic("i64")]),
    );
    assert_eq!(singleton.category, FrozenTypeCategory::Primitive);
    assert_eq!(
        Some(singleton.identity),
        overlay.identity_of("int"),
        "singleton union must collapse to the member identity"
    );
}

#[test]
fn erased_any_and_dyn_bound_sets_are_order_independent() {
    // S5: dyn bounds resolve against the frozen trait-name set, so the
    // fixture declares the traits it erases (matching real programs).
    let overlay = module_overlay(|compiler| {
        add_trait(compiler, "Show");
        add_trait(compiler, "Eq");
    });

    // `any` is the erased leaf the freeze already interns.
    let any = canon(&overlay, &basic("any"));
    assert_eq!(any.category, FrozenTypeCategory::Erased);
    assert_eq!(Some(any.identity), overlay.identity_of("any"));

    // dyn bound sets are sorted + deduped: dyn A + B == dyn B + A.
    let a_b = canon(
        &overlay,
        &TypeAnnotation::Dyn(vec![TypePath::simple("Show"), TypePath::simple("Eq")]),
    );
    let b_a = canon(
        &overlay,
        &TypeAnnotation::Dyn(vec![TypePath::simple("Eq"), TypePath::simple("Show")]),
    );
    assert_eq!(a_b.category, FrozenTypeCategory::Erased);
    assert_eq!(a_b.identity, b_a.identity);
    assert_eq!(a_b.descriptor, "erased:dyn Eq+Show");

    // Distinct bound sets stay distinct, and dyn is distinct from bare `any`.
    let show_only = canon(&overlay, &TypeAnnotation::Dyn(vec![TypePath::simple("Show")]));
    assert_ne!(a_b.identity, show_only.identity);
    assert_ne!(any.identity, show_only.identity);
}

#[test]
fn applied_generic_is_nominal_with_args_distinct_from_bare_head() {
    let overlay = module_overlay(|compiler| add_struct(compiler, "User"));

    let option_int = canon(&overlay, &applied("Option", vec![basic("int")]));
    assert_eq!(option_int.category, FrozenTypeCategory::Nominal);
    assert_eq!(
        option_int.descriptor,
        format!(
            "applied:{}<{}>",
            leaf_hex(&overlay, "Option"),
            leaf_hex(&overlay, "int")
        )
    );
    // Applied form is distinct from the bare nominal head and from other
    // instantiations.
    assert_ne!(Some(option_int.identity), overlay.identity_of("Option"));
    let option_string = canon(&overlay, &applied("Option", vec![basic("string")]));
    assert_ne!(option_int.identity, option_string.identity);

    // User nominals apply the same way.
    let array_user = canon(&overlay, &applied("Array", vec![basic("User")]));
    assert_eq!(array_user.category, FrozenTypeCategory::Nominal);
    assert_ne!(Some(array_user.identity), overlay.identity_of("Array"));
    assert_ne!(array_user.identity, option_int.identity);

    // Nested applications compose: Option<Array<User>>.
    let nested = canon(
        &overlay,
        &applied("Option", vec![applied("Array", vec![basic("User")])]),
    );
    assert_eq!(
        nested.descriptor,
        format!(
            "applied:{}<{}>",
            leaf_hex(&overlay, "Option"),
            identity_hex(array_user.identity)
        )
    );
}

#[test]
fn array_sugar_and_applied_array_share_identity() {
    let overlay = module_overlay(|_| {});

    let sugar = canon(&overlay, &TypeAnnotation::Array(Box::new(basic("int"))));
    let explicit = canon(&overlay, &applied("Array", vec![basic("int")]));
    assert_eq!(sugar.identity, explicit.identity);
    assert_eq!(sugar.descriptor, explicit.descriptor);
    assert_eq!(sugar.category, FrozenTypeCategory::Nominal);
}

#[test]
fn parameter_leaf_embeds_owner_scoped_identity_hex() {
    let freeze = freeze_of(|_| {});
    let map_overlay = FreezeOverlay::new(Arc::clone(&freeze), "map", &["T".to_string()]);

    let option_t = canon(&map_overlay, &applied("Option", vec![basic("T")]));
    assert_eq!(option_t.category, FrozenTypeCategory::Nominal);
    let parameter_hex = identity_hex(FrozenTypeIdentity::from_canonical_descriptor(
        "parameter:map:T",
    ));
    assert!(
        option_t.descriptor.contains(&parameter_hex),
        "applied descriptor must embed the parameter:{{owner}}:{{name}} identity hex: {}",
        option_t.descriptor
    );

    // Stable across two overlays with the same owner.
    let map_again = FreezeOverlay::new(Arc::clone(&freeze), "map", &["T".to_string()]);
    assert_eq!(
        option_t.identity,
        canon(&map_again, &applied("Option", vec![basic("T")])).identity
    );

    // Distinct across owning functions.
    let filter_overlay = FreezeOverlay::new(freeze, "filter", &["T".to_string()]);
    assert_ne!(
        option_t.identity,
        canon(&filter_overlay, &applied("Option", vec![basic("T")])).identity
    );
}

#[test]
fn composite_alias_targets_normalize_through_applied_forms() {
    // `type UserId = int` (simple) + `type Ids = Array<UserId>` (composite
    // target) + `type Pair = [int, string]` (composite target), registered
    // exactly as the compiler's `Item::TypeAlias` arm does: the string table
    // holds a simple-name projection (debug string for composites) and the
    // type-inference environment holds the full annotation.
    let ids_target = applied("Array", vec![basic("UserId")]);
    let pair_target = TypeAnnotation::Tuple(vec![basic("int"), basic("string")]);
    let freeze = freeze_of(|compiler| {
        add_alias(compiler, "UserId", "int");
        compiler
            .type_aliases
            .insert("Ids".to_string(), format!("{ids_target:?}"));
        compiler
            .type_inference
            .env
            .define_type_alias("Ids", &ids_target, None);
        compiler
            .type_aliases
            .insert("Pair".to_string(), format!("{pair_target:?}"));
        compiler
            .type_inference
            .env
            .define_type_alias("Pair", &pair_target, None);
    });
    let overlay = FreezeOverlay::new(Arc::clone(&freeze), "<module>", &[]);

    // Alias normalization holds THROUGH the applied form:
    // identity(Ids) == identity(Array<int>).
    let array_int = canon(&overlay, &applied("Array", vec![basic("int")]));
    assert_eq!(freeze.identity_of("Ids"), Some(array_int.identity));
    assert_eq!(
        freeze.category_of(array_int.identity),
        Ok(FrozenTypeCategory::Nominal)
    );

    // Composite tuple targets intern with their structural category.
    let pair = canon(&overlay, &pair_target);
    assert_eq!(freeze.identity_of("Pair"), Some(pair.identity));
    assert_eq!(
        freeze.category_of(pair.identity),
        Ok(FrozenTypeCategory::Tuple)
    );

    // And the canonicalizer resolves the alias leaf itself: Array<UserId>
    // spelled directly reaches the Array<int> identity.
    let array_user_id = canon(&overlay, &ids_target);
    assert_eq!(array_user_id.identity, array_int.identity);
}

#[test]
fn unresolved_leaf_names_reject_at_any_depth() {
    let overlay = module_overlay(|_| {});

    for annotation in [
        basic("Bogus"),
        TypeAnnotation::Tuple(vec![basic("int"), basic("Bogus")]),
        applied("Option", vec![basic("Bogus")]),
        callable(vec![basic("Bogus")], basic("int")),
        TypeAnnotation::Object(vec![record_field("x", false, basic("Bogus"))]),
        TypeAnnotation::Borrow {
            mutable: true,
            inner: Box::new(basic("Bogus")),
        },
        TypeAnnotation::Union(vec![basic("int"), basic("Bogus")]),
    ] {
        let error = canonicalize_type_annotation(&annotation, &overlay)
            .expect_err("unresolved leaf must reject");
        assert!(
            error.contains("unknown semantic type identity"),
            "rejection must stay in the freeze-boundary family: {error}"
        );
        assert!(
            error.contains("Bogus"),
            "rejection must name the unresolved leaf: {error}"
        );
    }
}

#[test]
fn inference_holes_reject_with_freeze_boundary_diagnostic() {
    let overlay = module_overlay(|_| {});
    let hole = tyvar_to_annotation(&TypeVar("T3".to_string()));

    for annotation in [
        hole.clone(),
        TypeAnnotation::Tuple(vec![basic("int"), hole.clone()]),
        applied("Option", vec![hole.clone()]),
    ] {
        let error = canonicalize_type_annotation(&annotation, &overlay)
            .expect_err("inference hole must reject");
        assert!(
            error.contains("cannot be frozen"),
            "named Dec 52 class missing: {error}"
        );
        assert!(
            error.contains("unresolved inference variable"),
            "named Dec 52 cause missing: {error}"
        );
    }
}

#[test]
fn applying_arguments_to_a_non_nominal_head_is_rejected() {
    let freeze = freeze_of(|_| {});
    let overlay = FreezeOverlay::new(freeze, "map", &["T".to_string()]);

    // Primitive head.
    let error = canonicalize_type_annotation(&applied("int", vec![basic("string")]), &overlay)
        .expect_err("primitive head must not accept type arguments");
    assert!(error.contains("int"), "{error}");
    assert!(error.contains("apply type arguments"), "{error}");

    // Parameter head.
    let error = canonicalize_type_annotation(&applied("T", vec![basic("int")]), &overlay)
        .expect_err("parameter head must not accept type arguments");
    assert!(error.contains('T'), "{error}");
    assert!(error.contains("apply type arguments"), "{error}");
}

#[test]
fn object_intersection_normalizes_to_the_directly_spelled_record() {
    let overlay = module_overlay(|_| {});

    let intersection = canon(
        &overlay,
        &TypeAnnotation::Intersection(vec![
            TypeAnnotation::Object(vec![record_field("a", false, basic("int"))]),
            TypeAnnotation::Object(vec![record_field("b", false, basic("string"))]),
        ]),
    );
    let direct = canon(
        &overlay,
        &TypeAnnotation::Object(vec![
            record_field("a", false, basic("int")),
            record_field("b", false, basic("string")),
        ]),
    );
    assert_eq!(intersection.category, FrozenTypeCategory::Record);
    assert_eq!(intersection.identity, direct.identity);

    // Non-object intersections do not canonicalize here (named rejection).
    let error = canonicalize_type_annotation(
        &TypeAnnotation::Intersection(vec![basic("int"), basic("string")]),
        &overlay,
    )
    .expect_err("non-object intersection must reject");
    assert!(error.contains("intersection"), "{error}");

    // Field collisions across intersection members are named rejections.
    let error = canonicalize_type_annotation(
        &TypeAnnotation::Intersection(vec![
            TypeAnnotation::Object(vec![record_field("a", false, basic("int"))]),
            TypeAnnotation::Object(vec![record_field("a", false, basic("string"))]),
        ]),
        &overlay,
    )
    .expect_err("colliding intersection fields must reject");
    assert!(error.contains("duplicate field"), "{error}");
}

// ============================================================================
// ADR-009 A2 (slice S5): rejection matrix over composite forms — arity/kind
// checks live HERE, in the single canonicalizer (one derivation), and every
// rejection is a named diagnostic.
// ============================================================================

/// S5 R2 (dyn case): `dyn` bounds resolve against the frozen trait-name set
/// (named freeze input) — a known trait erases; an unknown bound is a named
/// rejection in the unknown-identity family naming the bound.
#[test]
fn dyn_bounds_resolve_against_frozen_trait_names() {
    let overlay = module_overlay(|compiler| add_trait(compiler, "Show"));

    let show = canon(&overlay, &TypeAnnotation::Dyn(vec![TypePath::simple("Show")]));
    assert_eq!(show.category, FrozenTypeCategory::Erased);
    assert_eq!(show.descriptor, "erased:dyn Show");

    let error = canonicalize_type_annotation(
        &TypeAnnotation::Dyn(vec![TypePath::simple("NoSuchTrait")]),
        &overlay,
    )
    .expect_err("unknown trait bound must reject");
    assert!(
        error.contains("unknown semantic type identity"),
        "rejection must stay in the freeze-boundary family: {error}"
    );
    assert!(
        error.contains("NoSuchTrait"),
        "rejection must name the unresolved bound: {error}"
    );

    // A struct name is not a trait: erasing over it is the same named
    // rejection (traits and nominals are distinct freeze inputs).
    let overlay = module_overlay(|compiler| add_struct(compiler, "User"));
    let error = canonicalize_type_annotation(
        &TypeAnnotation::Dyn(vec![TypePath::simple("User")]),
        &overlay,
    )
    .expect_err("nominal used as a trait bound must reject");
    assert!(error.contains("User"), "{error}");
}

/// S5 R8: a trait intersection (`Show + Eq` in type position) erases to the
/// SAME bound-set descriptor/identity as the `dyn Show + Eq` spelling
/// (Dec 50/94 rule 3); mixed object/trait intersections are named rejections.
#[test]
fn trait_intersection_erases_to_the_dyn_bound_set_identity() {
    let overlay = module_overlay(|compiler| {
        add_trait(compiler, "Show");
        add_trait(compiler, "Eq");
    });

    let intersection = canon(
        &overlay,
        &TypeAnnotation::Intersection(vec![basic("Show"), basic("Eq")]),
    );
    let dyn_form = canon(
        &overlay,
        &TypeAnnotation::Dyn(vec![TypePath::simple("Eq"), TypePath::simple("Show")]),
    );
    assert_eq!(intersection.category, FrozenTypeCategory::Erased);
    assert_eq!(intersection.descriptor, dyn_form.descriptor);
    assert_eq!(intersection.identity, dyn_form.identity);

    // Mixed object/trait intersection: named rejection (neither all-object
    // nor all-trait).
    let error = canonicalize_type_annotation(
        &TypeAnnotation::Intersection(vec![
            TypeAnnotation::Object(vec![record_field("a", false, basic("int"))]),
            basic("Show"),
        ]),
        &overlay,
    )
    .expect_err("mixed intersection must reject");
    assert!(error.contains("intersection"), "{error}");
}

/// S5 R5: applied-generic arity is enforced from the freeze — a builtin
/// arity table plus user-struct `struct_generic_info.type_params`. The
/// arity fact is identity-keyed, so alias heads inherit it transparently.
#[test]
fn applied_arity_mismatches_reject_with_a_named_diagnostic() {
    let overlay = module_overlay(|compiler| {
        add_generic_struct(compiler, "Box", &["T"]);
        add_generic_struct(compiler, "Plain", &[]);
        add_alias(compiler, "Opt", "Option");
    });

    for (annotation, head, expected_text) in [
        (
            applied("Option", vec![basic("int"), basic("string")]),
            "Option",
            "expects 1 type argument(s), but 2 were provided",
        ),
        (
            applied("HashMap", vec![basic("int")]),
            "HashMap",
            "expects 2 type argument(s), but 1 were provided",
        ),
        (
            applied("Box", vec![basic("int"), basic("string")]),
            "Box",
            "expects 1 type argument(s), but 2 were provided",
        ),
        (
            applied("Plain", vec![basic("int")]),
            "Plain",
            "expects 0 type argument(s), but 1 were provided",
        ),
        // Alias heads inherit the target's arity (identity-keyed fact).
        (
            applied("Opt", vec![basic("int"), basic("string")]),
            "Opt",
            "expects 1 type argument(s), but 2 were provided",
        ),
    ] {
        let error = canonicalize_type_annotation(&annotation, &overlay)
            .expect_err("arity mismatch must reject");
        assert!(
            error.contains(head),
            "arity rejection must name the head '{head}': {error}"
        );
        assert!(
            error.contains(expected_text),
            "arity rejection must state declared vs provided counts: {error}"
        );
    }

    // Correct arities still canonicalize — and the alias head reaches the
    // SAME applied identity as its target head (Dec 53 transparency).
    let hash_map = canon(
        &overlay,
        &applied("HashMap", vec![basic("string"), basic("int")]),
    );
    assert_eq!(hash_map.category, FrozenTypeCategory::Nominal);
    let boxed = canon(&overlay, &applied("Box", vec![basic("int")]));
    assert_eq!(boxed.category, FrozenTypeCategory::Nominal);
    assert_eq!(
        canon(&overlay, &applied("Opt", vec![basic("int")])).identity,
        canon(&overlay, &applied("Option", vec![basic("int")])).identity
    );
}

#[test]
fn composite_identities_round_trip_their_canonical_descriptors() {
    let overlay = module_overlay(|compiler| {
        add_struct(compiler, "User");
        add_trait(compiler, "Show");
    });

    let forms = [
        TypeAnnotation::Tuple(vec![basic("int"), basic("string")]),
        TypeAnnotation::Object(vec![record_field("x", false, basic("int"))]),
        callable(vec![basic("int")], basic("bool")),
        TypeAnnotation::Borrow {
            mutable: false,
            inner: Box::new(basic("int")),
        },
        TypeAnnotation::Union(vec![basic("int"), basic("string")]),
        TypeAnnotation::Dyn(vec![TypePath::simple("Show")]),
        applied("Option", vec![basic("int")]),
    ];

    let mut identities = Vec::new();
    for form in &forms {
        let canonical = canon(&overlay, form);
        // The composite identity is exactly the hash of its canonical
        // descriptor (the B4/B7 ABI substrate contract).
        assert_eq!(
            canonical.identity,
            FrozenTypeIdentity::from_canonical_descriptor(&canonical.descriptor),
            "identity must round-trip descriptor: {}",
            canonical.descriptor
        );
        identities.push(canonical.identity);
    }
    // Cross-form distinctness: no two composite forms collide.
    for (i, left) in identities.iter().enumerate() {
        for right in &identities[i + 1..] {
            assert_ne!(left, right, "composite forms must not collide");
        }
    }

    // Declaration-order independence: an unrelated declaration in the freeze
    // does not move any composite identity.
    let grown = module_overlay(|compiler| {
        add_struct(compiler, "User");
        add_trait(compiler, "Show");
        add_struct(compiler, "Unrelated");
        add_alias(compiler, "Widened", "int");
    });
    for (form, identity) in forms.iter().zip(&identities) {
        assert_eq!(&canon(&grown, form).identity, identity);
    }

    // Structural leaf spellings normalize through the freeze's synonyms.
    let void_leaf = canon(&overlay, &TypeAnnotation::Void);
    assert_eq!(Some(void_leaf.identity), overlay.identity_of("unit"));
    let never_leaf = canon(&overlay, &TypeAnnotation::Never);
    assert_eq!(never_leaf.category, FrozenTypeCategory::Never);

    // A composite alias target carrying an inference hole rejects the WHOLE
    // freeze at the barrier (Dec 52), before any comptime site could run.
    let mut compiler = BytecodeCompiler::new();
    let holed = TypeAnnotation::Tuple(vec![
        basic("int"),
        tyvar_to_annotation(&TypeVar("T9".to_string())),
    ]);
    compiler
        .type_aliases
        .insert("Holed".to_string(), format!("{holed:?}"));
    compiler
        .type_inference
        .env
        .define_type_alias("Holed", &holed, None);
    let error =
        SemanticFreeze::freeze(&compiler).expect_err("holed composite alias must reject freeze");
    assert!(error.diagnostic().contains("unresolved inference variable"));
    assert!(error.diagnostic().contains("Holed"));
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
