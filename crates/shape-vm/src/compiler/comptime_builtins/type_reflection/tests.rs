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

    let pair = canon(
        &overlay,
        &TypeAnnotation::Tuple(vec![basic("int"), basic("string")]),
    );
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
    let synonyms = canon(
        &overlay,
        &TypeAnnotation::Tuple(vec![basic("i64"), basic("str")]),
    );
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
    assert!(
        optional.descriptor.contains("x?:"),
        "{}",
        optional.descriptor
    );
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
    assert!(
        error.contains('x'),
        "must name the colliding field: {error}"
    );
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

/// Review round 1 (A2): union membership is an associative SET — a
/// syntactically nested union (the parenthesized spelling the grammar admits
/// via `non_array_type ::= … | "(" type_annotation ")"`) splices its members
/// into the enclosing union BEFORE dedup/byte-sort, so semantically equal
/// union spellings mint ONE ABI identity and members reached through nesting
/// cannot escape dedup.
#[test]
fn nested_unions_flatten_to_the_flat_set_semantic_identity() {
    let overlay = module_overlay(|_| {});

    let flat = canon(
        &overlay,
        &TypeAnnotation::Union(vec![basic("int"), basic("string"), basic("bool")]),
    );
    // (int | string) | bool — left-nested spelling.
    let left_nested = canon(
        &overlay,
        &TypeAnnotation::Union(vec![
            TypeAnnotation::Union(vec![basic("int"), basic("string")]),
            basic("bool"),
        ]),
    );
    // int | (string | bool) — right-nested spelling.
    let right_nested = canon(
        &overlay,
        &TypeAnnotation::Union(vec![
            basic("int"),
            TypeAnnotation::Union(vec![basic("string"), basic("bool")]),
        ]),
    );
    assert_eq!(
        left_nested.identity, flat.identity,
        "(int | string) | bool must mint the flat union identity"
    );
    assert_eq!(
        right_nested.identity, flat.identity,
        "int | (string | bool) must mint the flat union identity"
    );
    // The descriptor is the FLAT three-member form — leaf hexes only, never
    // an embedded nested-union identity hex.
    let mut hexes = vec![
        leaf_hex(&overlay, "int"),
        leaf_hex(&overlay, "string"),
        leaf_hex(&overlay, "bool"),
    ];
    hexes.sort();
    assert_eq!(flat.descriptor, format!("union:({})", hexes.join("|")));
    assert_eq!(left_nested.descriptor, flat.descriptor);

    // Members reached through nesting cannot escape dedup:
    // int | (int | string) == int | string.
    let int_or_string = canon(
        &overlay,
        &TypeAnnotation::Union(vec![basic("int"), basic("string")]),
    );
    let dup_through_nesting = canon(
        &overlay,
        &TypeAnnotation::Union(vec![
            basic("int"),
            TypeAnnotation::Union(vec![basic("int"), basic("string")]),
        ]),
    );
    assert_eq!(
        dup_through_nesting.identity, int_or_string.identity,
        "a member reached through nesting must not escape dedup"
    );

    // Singleton collapse holds through nesting: (int | i64) | int IS int.
    let collapsed = canon(
        &overlay,
        &TypeAnnotation::Union(vec![
            TypeAnnotation::Union(vec![basic("int"), basic("i64")]),
            basic("int"),
        ]),
    );
    assert_eq!(collapsed.category, FrozenTypeCategory::Primitive);
    assert_eq!(
        Some(collapsed.identity),
        overlay.identity_of("int"),
        "a nested union whose members all coalesce collapses to the member"
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
    let show_only = canon(
        &overlay,
        &TypeAnnotation::Dyn(vec![TypePath::simple("Show")]),
    );
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

    let show = canon(
        &overlay,
        &TypeAnnotation::Dyn(vec![TypePath::simple("Show")]),
    );
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

// ─── ADR-009 B1 S2: freeze payload query + payload descriptor builders ────

mod payload_query {
    use super::payloads::{FrozenPayloadDescriptor, pending_payload_rejection};
    use super::*;
    use crate::compiler::comptime_builtins::semantic_freeze;
    use shape_runtime::comptime_reflection::{
        FLOAT_WIDTH_SCHEMA_NAME, FROZEN_TYPE_ENABLED_PAYLOAD_CATEGORIES, FloatWidth,
        FrozenPrimitive, INTEGER_WIDTH_SCHEMA_NAME, IntegerWidth,
    };
    use shape_runtime::type_schema::builtin_schemas::{
        COMPTIME_FROZEN_ERASED_SCHEMA, COMPTIME_FROZEN_NEVER_SCHEMA,
        COMPTIME_FROZEN_PRIMITIVE_SCHEMA, COMPTIME_FROZEN_TYPE_SCHEMA,
    };
    use shape_value::heap_value::{HeapValue, TypedObjectStorage};
    use shape_value::{KindedSlot, NativeKind};

    /// The full primitive payload matrix: every sealed-sub-algebra family
    /// member AND every synonym resolves to the same exact width/domain
    /// payload through the ONE query API (`payload_of` beside
    /// `identity_of`/`category_of`). `bigint` is the named
    /// `SignedInteger(Arbitrary)` decision.
    #[test]
    fn primitive_payload_matrix_covers_every_family_and_synonym() {
        let freeze = freeze_of(|_| {});
        let matrix: &[(&[&str], FrozenPrimitive)] = &[
            (&["unit", "void", "()"], FrozenPrimitive::Unit),
            (&["bool"], FrozenPrimitive::Bool),
            (&["char"], FrozenPrimitive::Char),
            (
                &["int", "i64"],
                FrozenPrimitive::SignedInteger(IntegerWidth::W64),
            ),
            (&["i8"], FrozenPrimitive::SignedInteger(IntegerWidth::W8)),
            (&["i16"], FrozenPrimitive::SignedInteger(IntegerWidth::W16)),
            (&["i32"], FrozenPrimitive::SignedInteger(IntegerWidth::W32)),
            (&["u8"], FrozenPrimitive::UnsignedInteger(IntegerWidth::W8)),
            (
                &["u16"],
                FrozenPrimitive::UnsignedInteger(IntegerWidth::W16),
            ),
            (
                &["u32"],
                FrozenPrimitive::UnsignedInteger(IntegerWidth::W32),
            ),
            (
                &["u64"],
                FrozenPrimitive::UnsignedInteger(IntegerWidth::W64),
            ),
            (
                &["bigint"],
                FrozenPrimitive::SignedInteger(IntegerWidth::Arbitrary),
            ),
            (
                &["number", "f64", "float"],
                FrozenPrimitive::BinaryFloat(FloatWidth::W64),
            ),
            (&["f32"], FrozenPrimitive::BinaryFloat(FloatWidth::W32)),
            (&["decimal"], FrozenPrimitive::Decimal),
            (&["string", "str"], FrozenPrimitive::String),
            (&["null"], FrozenPrimitive::Null),
            (&["undefined"], FrozenPrimitive::Undefined),
        ];
        for (names, expected) in matrix {
            for name in *names {
                let identity = freeze
                    .identity_of(name)
                    .unwrap_or_else(|| panic!("{name} must be frozen"));
                assert_eq!(
                    freeze.payload_of(identity),
                    Ok(FrozenPayloadDescriptor::Primitive(*expected)),
                    "payload for {name}"
                );
            }
        }
    }

    /// `never` reflects to the `Never` payload; `any` reflects to `Erased`
    /// with the empty bound set (the only reachable erased spelling until
    /// A2 lands trait-bound syntax).
    #[test]
    fn never_and_erased_payloads_are_complete_for_reachable_forms() {
        let freeze = freeze_of(|_| {});

        let never = freeze.identity_of("never").expect("never identity");
        assert_eq!(freeze.payload_of(never), Ok(FrozenPayloadDescriptor::Never));

        let any = freeze.identity_of("any").expect("any identity");
        match freeze.payload_of(any) {
            Ok(FrozenPayloadDescriptor::Erased { bounds }) => {
                assert!(bounds.is_empty(), "any carries the empty bound set");
            }
            other => panic!("any must reflect to Erased, got {other:?}"),
        }
    }

    /// Rejection-matrix row R1: each of the 7 non-enabled categories has ONE
    /// named per-category diagnostic — naming the category, stating the
    /// payload descriptor has not landed, and pointing at `type_category` —
    /// never a partial descriptor. Parameter is asserted end-to-end through
    /// a scoped overlay identity (`parameter:{owner}:{name}`), Nominal
    /// end-to-end through a frozen struct.
    #[test]
    fn non_enabled_categories_reject_with_named_per_category_diagnostics() {
        for category in FrozenTypeCategory::ALL {
            if FROZEN_TYPE_ENABLED_PAYLOAD_CATEGORIES.contains(&category) {
                continue;
            }
            let diagnostic = pending_payload_rejection(category);
            assert_eq!(
                diagnostic,
                format!(
                    "reflect: the {} payload descriptor has not landed \
                     (pending payload ticket); use type_category for the \
                     exhaustive category",
                    category.variant_name()
                )
            );
        }

        // Parameter — through a scoped overlay identity.
        let overlay = FreezeOverlay::new(freeze_of(|_| {}), "map", &["T".to_string()]);
        let t = overlay.identity_of("T").expect("T identity");
        assert_eq!(
            overlay.payload_of(t),
            Err(pending_payload_rejection(FrozenTypeCategory::Parameter))
        );

        // Nominal — through the base freeze.
        let freeze = freeze_of(|compiler| add_struct(compiler, "Alpha"));
        let alpha = freeze.identity_of("Alpha").expect("Alpha identity");
        let error = freeze.payload_of(alpha).expect_err("Nominal must reject");
        assert!(
            error.contains("the Nominal payload descriptor has not landed")
                && error.contains("use type_category"),
            "R1 Nominal diagnostic missing: {error}"
        );
    }

    /// The unknown-identity freeze-boundary rejection is unchanged by the
    /// payload query.
    #[test]
    fn unknown_identity_payload_rejection_is_unchanged() {
        let freeze = freeze_of(|_| {});
        assert_eq!(
            freeze.payload_of(FrozenTypeIdentity::INVALID),
            Err("type_ref received an unknown semantic type identity".to_string())
        );
    }

    // ── A2×B1 seam: payload query over the site-interned composites layer ─

    /// A2×B1 seam: `payload_of` resolves the site-interned composites layer
    /// symmetrically with `category_of` (one query, three layers — spec
    /// §4.1). Every pending composite category answers its NAMED R1
    /// per-category rejection — never the wrong-family unknown-identity
    /// diagnostic (the identity IS known: the same overlay minted it).
    #[test]
    fn site_interned_pending_composites_reject_with_named_per_category_diagnostics() {
        let overlay = module_overlay(|compiler| add_struct(compiler, "User"));
        let forms: Vec<(TypeAnnotation, FrozenTypeCategory)> = vec![
            (
                TypeAnnotation::Tuple(vec![basic("int"), basic("string")]),
                FrozenTypeCategory::Tuple,
            ),
            (
                TypeAnnotation::Object(vec![record_field("x", false, basic("int"))]),
                FrozenTypeCategory::Record,
            ),
            (
                callable(vec![basic("int")], basic("bool")),
                FrozenTypeCategory::Callable,
            ),
            (
                TypeAnnotation::Borrow {
                    mutable: false,
                    inner: Box::new(basic("User")),
                },
                FrozenTypeCategory::Reference,
            ),
            (
                TypeAnnotation::Union(vec![basic("int"), basic("string")]),
                FrozenTypeCategory::Union,
            ),
            (
                applied("Option", vec![basic("int")]),
                FrozenTypeCategory::Nominal,
            ),
        ];
        for (annotation, category) in forms {
            let identity = overlay
                .canonicalize_type(&annotation)
                .expect("composite type expression canonicalizes");
            assert_eq!(overlay.category_of(identity), Ok(category));
            assert_eq!(
                overlay.payload_of(identity),
                Err(pending_payload_rejection(category)),
                "payload_of must answer the composites layer for {category:?}"
            );
        }
    }

    /// A2×B1 seam, Erased disposition: a site-interned `dyn` /
    /// trait-intersection composite classifies as the ENABLED Erased
    /// category, but its bound-set payload elements are the B2
    /// trait-reference descriptors (`FrozenErasedBound` is uninhabited until
    /// then) — the payload query is the NAMED bounded-erased rejection,
    /// never an empty (partial) bound set, never unknown-identity.
    #[test]
    fn site_interned_dyn_erased_composite_rejects_with_the_named_bounded_diagnostic() {
        let overlay = module_overlay(|compiler| {
            add_trait(compiler, "Walk");
            add_trait(compiler, "Swim");
        });
        for annotation in [
            TypeAnnotation::Dyn(vec![TypePath::simple("Walk")]),
            TypeAnnotation::Dyn(vec![TypePath::simple("Walk"), TypePath::simple("Swim")]),
            TypeAnnotation::Intersection(vec![basic("Walk"), basic("Swim")]),
        ] {
            let identity = overlay
                .canonicalize_type(&annotation)
                .expect("erased composite canonicalizes");
            assert_eq!(
                overlay.category_of(identity),
                Ok(FrozenTypeCategory::Erased)
            );
            let error = overlay
                .payload_of(identity)
                .expect_err("bounded erased must reject until B2");
            assert!(
                error.contains("Erased bound-set payload") && error.contains("use type_category"),
                "named bounded-erased diagnostic missing: {error}"
            );
        }
    }

    /// Union coalescing memoizes base LEAF identities (`int | i64` → the
    /// `int` leaf, `any | any` → the `any` leaf): through the memo layer the
    /// payload query answers the member's COMPLETE payload from the base
    /// index — same payload, one derivation.
    #[test]
    fn coalesced_union_identities_answer_the_member_payload_through_the_memo() {
        let overlay = module_overlay(|_| {});
        let int_identity = overlay
            .canonicalize_type(&TypeAnnotation::Union(vec![basic("int"), basic("i64")]))
            .expect("coalescing union canonicalizes");
        assert_eq!(
            overlay.payload_of(int_identity),
            Ok(FrozenPayloadDescriptor::Primitive(
                FrozenPrimitive::SignedInteger(IntegerWidth::W64)
            ))
        );
        let any_identity = overlay
            .canonicalize_type(&TypeAnnotation::Union(vec![basic("any"), basic("any")]))
            .expect("coalescing union canonicalizes");
        match overlay.payload_of(any_identity) {
            Ok(FrozenPayloadDescriptor::Erased { bounds }) => {
                assert!(
                    bounds.is_empty(),
                    "`any` keeps the complete empty bound set"
                );
            }
            other => panic!("`any` must keep its Erased payload, got {other:?}"),
        }
    }

    /// Finding-2 latent hazard at the BASE index: an alias-fixpoint-interned
    /// `erased:dyn …` identity (category Erased, base-resolvable) must NOT
    /// reflect to an empty bound set — the named bounded-erased rejection
    /// fires at the base level too; only the `any` leaf answers the complete
    /// empty bound set.
    #[test]
    fn alias_interned_dyn_erased_identity_rejects_at_the_base_index() {
        let dyn_show = TypeAnnotation::Dyn(vec![TypePath::simple("Show")]);
        let freeze = freeze_of(|compiler| {
            add_trait(compiler, "Show");
            compiler
                .type_aliases
                .insert("Erasable".to_string(), "dyn Show".to_string());
            compiler
                .type_inference
                .env
                .define_type_alias("Erasable", &dyn_show, None);
        });
        let identity = freeze
            .identity_of("Erasable")
            .expect("alias fixpoint interns the dyn target");
        assert_eq!(freeze.category_of(identity), Ok(FrozenTypeCategory::Erased));
        let error = freeze
            .payload_of(identity)
            .expect_err("bounded erased must reject until B2");
        assert!(
            error.contains("Erased bound-set payload"),
            "named bounded-erased diagnostic missing: {error}"
        );
        // `any` keeps answering the complete AND empty bound set.
        let any = freeze.identity_of("any").expect("any identity");
        assert!(matches!(
            freeze.payload_of(any),
            Ok(FrozenPayloadDescriptor::Erased { .. })
        ));
    }

    // ── heap-value builders ──────────────────────────────────────────────

    fn storage_of(value: &HeapValue) -> &TypedObjectStorage {
        let HeapValue::TypedObject(ptr) = value else {
            panic!("descriptor must be a TypedObject, got {:?}", value.kind());
        };
        unsafe { &*ptr.as_ptr() }
    }

    fn schema_name_of(storage: &TypedObjectStorage) -> String {
        shape_runtime::type_schema::lookup_schema_by_id_public(storage.schema_id as u32)
            .expect("descriptor schema id must resolve")
            .name
            .clone()
    }

    /// Read `__variant` (field 0) and `__payload_0` (field 1) from an
    /// enum-layout descriptor object.
    fn variant_and_payload(storage: &TypedObjectStorage) -> (i64, KindedSlot) {
        let variant = storage
            .clone_field_kinded(0)
            .and_then(|slot| slot.as_i64())
            .expect("__variant must be an int");
        let payload = storage
            .clone_field_kinded(1)
            .expect("__payload_0 must be readable");
        (variant, payload)
    }

    /// `build_frozen_type_heap_value` composes schema-correct NESTED typed
    /// objects: FrozenType{__variant: catalog ordinal, __payload_0:
    /// FrozenPrimitive{__variant, __payload_0: width-domain enum}} — typed
    /// descriptor data all the way down, no rendered type-name strings.
    #[test]
    fn builder_produces_schema_correct_nested_primitive_descriptor() {
        let overlay = semantic_freeze::overlay_for_tests(&BytecodeCompiler::new());

        // int → Primitive(SignedInteger(W64)), FrozenType variant ordinal 0.
        let int_identity = overlay.identity_of("int").expect("int identity");
        let frozen = payloads::build_frozen_type_heap_value(int_identity, &overlay)
            .expect("int payload builds");
        let frozen_storage = storage_of(&frozen);
        assert_eq!(schema_name_of(frozen_storage), COMPTIME_FROZEN_TYPE_SCHEMA);
        let (variant, payload) = variant_and_payload(frozen_storage);
        assert_eq!(variant, 0, "Primitive is catalog ordinal 0");

        let primitive_storage = payload
            .as_typed_object_storage()
            .expect("payload must be a typed object");
        assert_eq!(
            schema_name_of(primitive_storage),
            COMPTIME_FROZEN_PRIMITIVE_SCHEMA
        );
        let (primitive_variant, width_slot) = variant_and_payload(primitive_storage);
        // SignedInteger is declaration index 3 in the FrozenPrimitive catalog.
        assert_eq!(primitive_variant, 3);
        let width_storage = width_slot
            .as_typed_object_storage()
            .expect("width payload must be a typed object");
        assert_eq!(schema_name_of(width_storage), INTEGER_WIDTH_SCHEMA_NAME);
        let (width_variant, _) = (
            width_storage
                .clone_field_kinded(0)
                .and_then(|slot| slot.as_i64())
                .expect("width __variant"),
            (),
        );
        assert_eq!(width_variant, 3, "W64 is IntegerWidth declaration index 3");

        // bigint → SignedInteger(Arbitrary): width variant 4.
        let bigint_identity = overlay.identity_of("bigint").expect("bigint identity");
        let frozen = payloads::build_frozen_type_heap_value(bigint_identity, &overlay)
            .expect("bigint payload builds");
        let (_, payload) = variant_and_payload(storage_of(&frozen));
        let (_, width_slot) =
            variant_and_payload(payload.as_typed_object_storage().expect("primitive object"));
        let width_storage = width_slot
            .as_typed_object_storage()
            .expect("width payload must be a typed object");
        let arbitrary = width_storage
            .clone_field_kinded(0)
            .and_then(|slot| slot.as_i64())
            .expect("width __variant");
        assert_eq!(
            arbitrary, 4,
            "Arbitrary is IntegerWidth declaration index 4"
        );

        // number → BinaryFloat(W64): FloatWidth schema, variant 1.
        let number_identity = overlay.identity_of("number").expect("number identity");
        let frozen = payloads::build_frozen_type_heap_value(number_identity, &overlay)
            .expect("number payload builds");
        let (_, payload) = variant_and_payload(storage_of(&frozen));
        let primitive_storage = payload.as_typed_object_storage().expect("primitive object");
        let (primitive_variant, width_slot) = variant_and_payload(primitive_storage);
        assert_eq!(primitive_variant, 5, "BinaryFloat is declaration index 5");
        let width_storage = width_slot
            .as_typed_object_storage()
            .expect("width payload must be a typed object");
        assert_eq!(schema_name_of(width_storage), FLOAT_WIDTH_SCHEMA_NAME);

        // bool → scalar member: Null payload slot (no width domain).
        let bool_identity = overlay.identity_of("bool").expect("bool identity");
        let frozen = payloads::build_frozen_type_heap_value(bool_identity, &overlay)
            .expect("bool payload builds");
        let (_, payload) = variant_and_payload(storage_of(&frozen));
        let primitive_storage = payload.as_typed_object_storage().expect("primitive object");
        let (primitive_variant, scalar_payload) = variant_and_payload(primitive_storage);
        assert_eq!(primitive_variant, 1, "Bool is declaration index 1");
        assert_eq!(scalar_payload.kind(), NativeKind::Null);
    }

    /// never → FrozenType{__variant: 1, __payload_0: FrozenNever{}} and
    /// any → FrozenType{__variant: 9, __payload_0: FrozenErased{bounds: []}}
    /// — ordinal-pinned variant ids (1/9, never dense).
    #[test]
    fn builder_produces_never_and_erased_descriptors_at_pinned_ordinals() {
        let overlay = semantic_freeze::overlay_for_tests(&BytecodeCompiler::new());

        let never_identity = overlay.identity_of("never").expect("never identity");
        let frozen = payloads::build_frozen_type_heap_value(never_identity, &overlay)
            .expect("never payload builds");
        let frozen_storage = storage_of(&frozen);
        let (variant, payload) = variant_and_payload(frozen_storage);
        assert_eq!(variant, 1, "Never is catalog ordinal 1");
        let never_storage = payload
            .as_typed_object_storage()
            .expect("payload must be a typed object");
        assert_eq!(schema_name_of(never_storage), COMPTIME_FROZEN_NEVER_SCHEMA);

        let any_identity = overlay.identity_of("any").expect("any identity");
        let frozen = payloads::build_frozen_type_heap_value(any_identity, &overlay)
            .expect("any payload builds");
        let frozen_storage = storage_of(&frozen);
        let (variant, payload) = variant_and_payload(frozen_storage);
        assert_eq!(variant, 9, "Erased is catalog ordinal 9, never dense 2");
        let erased_storage = payload
            .as_typed_object_storage()
            .expect("payload must be a typed object");
        assert_eq!(
            schema_name_of(erased_storage),
            COMPTIME_FROZEN_ERASED_SCHEMA
        );
        // The bound set is the empty array (the only reachable form).
        let bounds = erased_storage
            .clone_field_kinded(0)
            .expect("bounds must be readable");
        assert_eq!(
            bounds.kind(),
            NativeKind::Ptr(shape_value::heap_value::HeapKind::TypedArray)
        );
    }

    /// The builder inherits the R1 rejection: a scoped Parameter identity
    /// (and every other non-enabled category) is a named compile-time
    /// rejection at the builder too — never a partial descriptor.
    #[test]
    fn builder_rejects_non_enabled_categories_with_the_named_diagnostic() {
        let overlay = FreezeOverlay::new(freeze_of(|_| {}), "map", &["T".to_string()]);
        let t = overlay.identity_of("T").expect("T identity");
        let error = payloads::build_frozen_type_heap_value(t, &overlay)
            .map(|_| ())
            .expect_err("Parameter must reject at the builder");
        assert_eq!(
            error,
            pending_payload_rejection(FrozenTypeCategory::Parameter)
        );
    }
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

// ============================================================================
// ADR-009 B4 (Stage 2, Dec 54): uniform nominal application — constructor
// descriptors + apply/refine/type_argument over the SAME frozen identities A2
// mints. Identity-equality against the A2 applied spelling is the load-bearing
// invariant; refine round-trips; the rejection matrix carries named
// diagnostics.
// ============================================================================
mod b4 {
    use super::*;
    use crate::compiler::comptime_builtins::semantic_freeze::ENUM_HEAD_PARAM_KIND_UNRECOVERABLE_DIAGNOSTIC;
    use shape_ast::ast::TypeParam;
    use shape_runtime::type_schema::EnumVariantInfo;

    /// A user struct declaring an ordered mix of type and const generic
    /// parameters, registered exactly as `predeclare_struct_schema` does.
    fn add_mixed_generic_struct(compiler: &mut BytecodeCompiler, name: &str, params: &[TypeParam]) {
        add_struct(compiler, name);
        compiler.struct_generic_info.insert(
            name.to_string(),
            crate::compiler::StructGenericInfo {
                type_params: params.to_vec(),
                runtime_field_types: std::collections::HashMap::new(),
            },
        );
    }

    fn type_param(name: &str) -> TypeParam {
        TypeParam::Type {
            name: name.to_string(),
            span: shape_ast::ast::Span::DUMMY,
            doc_comment: None,
            default_type: None,
            trait_bounds: Vec::new(),
        }
    }

    fn const_param(name: &str) -> TypeParam {
        TypeParam::Const {
            name: name.to_string(),
            span: shape_ast::ast::Span::DUMMY,
            doc_comment: None,
            ty: basic("int"),
            default: None,
        }
    }

    /// Load-bearing invariant (both directions): a `Type`-argument application
    /// through `canonical_apply` reproduces the EXACT descriptor and identity
    /// of the A2 `type_ref(Head<Args>)` spelling — for builtins AND a user
    /// generic. `apply == type_ref(Option<int>)` both ways.
    #[test]
    fn apply_type_argument_is_identity_equal_to_the_a2_applied_spelling() {
        let overlay = module_overlay(|compiler| add_generic_struct(compiler, "Wrapper", &["T"]));
        let int_id = overlay.identity_of("int").expect("int identity");
        let string_id = overlay.identity_of("string").expect("string identity");

        // Arity-1 heads (builtins + a user generic).
        for head in ["Option", "Array", "Future", "Set", "Wrapper"] {
            let constructor = canonical_constructor(head, &overlay).expect("constructor mints");
            let via_apply = canonical_apply(&constructor, &[AppliedArg::Type(int_id)], &overlay)
                .expect("apply succeeds");
            let via_a2 = canon(&overlay, &applied(head, vec![basic("int")]));
            assert_eq!(
                via_apply.descriptor, via_a2.descriptor,
                "{head}: apply must reproduce the A2 applied descriptor"
            );
            assert_eq!(
                via_apply.identity, via_a2.identity,
                "{head}: identity(apply) == identity(type_ref(Head<int>))"
            );
            assert_eq!(via_apply.category, FrozenTypeCategory::Nominal);
        }

        // Arity-2 heads.
        for head in ["Result", "HashMap"] {
            let constructor = canonical_constructor(head, &overlay).expect("constructor mints");
            let via_apply = canonical_apply(
                &constructor,
                &[AppliedArg::Type(int_id), AppliedArg::Type(string_id)],
                &overlay,
            )
            .expect("apply succeeds");
            let via_a2 = canon(
                &overlay,
                &applied(head, vec![basic("int"), basic("string")]),
            );
            assert_eq!(via_apply.descriptor, via_a2.descriptor, "{head}");
            assert_eq!(via_apply.identity, via_a2.identity, "{head}");
        }
    }

    /// refine(apply(...)) recovers the head identity and ordered argument
    /// identities; type_argument reads them back by checked index.
    #[test]
    fn refine_round_trips_head_and_ordered_args() {
        let overlay = module_overlay(|_| {});
        let int_id = overlay.identity_of("int").expect("int identity");
        let string_id = overlay.identity_of("string").expect("string identity");
        let constructor = canonical_constructor("Result", &overlay).expect("Result constructor");
        let applied_type = canonical_apply(
            &constructor,
            &[AppliedArg::Type(int_id), AppliedArg::Type(string_id)],
            &overlay,
        )
        .expect("apply succeeds");

        let refined = canonical_refine(&applied_type.descriptor).expect("applied refines");
        assert_eq!(refined.head_identity, constructor.head_identity);
        assert_eq!(refined.arg_identities, vec![int_id, string_id]);
        assert_eq!(type_argument(&refined, 0), Ok(int_id));
        assert_eq!(type_argument(&refined, 1), Ok(string_id));
        let error = type_argument(&refined, 2).expect_err("index 2 out of range");
        assert!(
            error.contains("out of range"),
            "named out-of-range diag: {error}"
        );
        assert!(error.contains('2'));
    }

    /// refine over a bare-nominal / non-applied descriptor is `None` (round
    /// trips only over genuine applications) — never an error, never partial.
    #[test]
    fn refine_returns_none_for_bare_nominal_and_non_applied() {
        // A zero-generic struct registered as production does (empty
        // `struct_generic_info`, so `param_kinds_of` answers Ok(empty) — a
        // bare `add_struct` omits generic_info and would look like an enum
        // head).
        let overlay = module_overlay(|compiler| add_generic_struct(compiler, "User", &[]));
        // Bare nominal leaf descriptor.
        let user = canon(&overlay, &basic("User"));
        assert_eq!(canonical_refine(&user.descriptor), None);
        // Primitive leaf.
        let int = canon(&overlay, &basic("int"));
        assert_eq!(canonical_refine(&int.descriptor), None);
        // A composite that is not an application (tuple).
        let tuple = canon(
            &overlay,
            &TypeAnnotation::Tuple(vec![basic("int"), basic("string")]),
        );
        assert_eq!(canonical_refine(&tuple.descriptor), None);
        // A zero-argument application is the bare nominal — refine is None.
        let constructor = canonical_constructor("User", &overlay).expect("User constructor");
        let zero = canonical_apply(&constructor, &[], &overlay).expect("zero-arg apply");
        assert_eq!(
            zero.identity, user.identity,
            "zero-arg application IS the bare nominal"
        );
        assert_eq!(canonical_refine(&zero.descriptor), None);
    }

    /// The constructor descriptor is distinct from the bare nominal leaf so a
    /// TypeConstructorRef is never conflated with a TypeRef.
    #[test]
    fn constructor_identity_is_distinct_from_bare_nominal() {
        let overlay = module_overlay(|_| {});
        let constructor = canonical_constructor("Option", &overlay).expect("Option constructor");
        assert_eq!(
            Some(constructor.head_identity),
            overlay.identity_of("Option")
        );
        assert_ne!(constructor.identity, constructor.head_identity);
        assert_eq!(
            constructor.descriptor,
            format!("constructor:{}", leaf_hex(&overlay, "Option"))
        );
    }

    /// A const-generic application checks the const arg against the declared
    /// Const position and round-trips through refine/type_argument. There is
    /// no A2 spelling for const applications (the parser rejects them), so the
    /// only contract is internal round-trip consistency.
    #[test]
    fn const_generic_application_checks_and_round_trips() {
        let overlay = module_overlay(|compiler| {
            add_mixed_generic_struct(compiler, "Matrix", &[type_param("T"), const_param("N")]);
        });
        let int_id = overlay.identity_of("int").expect("int identity");
        let constructor = canonical_constructor("Matrix", &overlay).expect("Matrix constructor");
        let const_four = canonical_const_arg(4);
        let applied_type = canonical_apply(
            &constructor,
            &[AppliedArg::Type(int_id), const_four],
            &overlay,
        )
        .expect("mixed type+const apply succeeds");
        assert_eq!(applied_type.category, FrozenTypeCategory::Nominal);

        let refined = canonical_refine(&applied_type.descriptor).expect("applied refines");
        assert_eq!(refined.head_identity, constructor.head_identity);
        assert_eq!(refined.arg_identities, vec![int_id, const_four.identity()]);
        assert_eq!(type_argument(&refined, 1), Ok(const_four.identity()));

        // Distinct const values mint distinct applications.
        let other = canonical_apply(
            &constructor,
            &[AppliedArg::Type(int_id), canonical_const_arg(5)],
            &overlay,
        )
        .expect("apply succeeds");
        assert_ne!(applied_type.identity, other.identity);
    }

    #[test]
    fn constructor_rejects_non_nominal_head() {
        let overlay = module_overlay(|_| {});
        let error = canonical_constructor("int", &overlay)
            .expect_err("a primitive is not a type constructor");
        assert!(error.contains("int"), "names the head: {error}");
        assert!(
            error.contains("nominal"),
            "reuses the non-Nominal wall: {error}"
        );
    }

    #[test]
    fn constructor_rejects_unknown_head() {
        let overlay = module_overlay(|_| {});
        let error =
            canonical_constructor("Nonexistent", &overlay).expect_err("unknown head rejects");
        assert!(
            error.contains("unknown semantic type identity"),
            "unknown-identity family: {error}"
        );
        assert!(error.contains("Nonexistent"));
    }

    #[test]
    fn apply_rejects_wrong_arity_with_named_counts() {
        let overlay = module_overlay(|_| {});
        let constructor = canonical_constructor("Option", &overlay).expect("Option constructor");
        let int_id = overlay.identity_of("int").expect("int identity");
        let error = canonical_apply(
            &constructor,
            &[AppliedArg::Type(int_id), AppliedArg::Type(int_id)],
            &overlay,
        )
        .expect_err("arity mismatch rejects");
        assert!(
            error.contains("expects 1 type argument(s), but 2 were provided"),
            "named declared-vs-provided counts: {error}"
        );
        assert!(error.contains("Option"), "names the head: {error}");
    }

    #[test]
    fn apply_rejects_wrong_kind_distinguishing_type_and_const() {
        let overlay = module_overlay(|compiler| {
            add_mixed_generic_struct(compiler, "Matrix", &[type_param("T"), const_param("N")]);
        });
        let int_id = overlay.identity_of("int").expect("int identity");

        // A const argument supplied to a Type parameter (Option<T>).
        let option = canonical_constructor("Option", &overlay).expect("Option constructor");
        let error = canonical_apply(&option, &[canonical_const_arg(4)], &overlay)
            .expect_err("const arg to type param rejects");
        assert!(
            error.contains("Type"),
            "names the declared type kind: {error}"
        );
        assert!(
            error.contains("Const"),
            "names the supplied const kind: {error}"
        );

        // A type argument supplied to a Const parameter (Matrix<T, const N>).
        let matrix = canonical_constructor("Matrix", &overlay).expect("Matrix constructor");
        let error = canonical_apply(
            &matrix,
            &[AppliedArg::Type(int_id), AppliedArg::Type(int_id)],
            &overlay,
        )
        .expect_err("type arg to const param rejects");
        assert!(
            error.contains("argument 1"),
            "names the offending position: {error}"
        );
        assert!(error.contains("Const") && error.contains("Type"));
    }

    /// A generic enum head has no recoverable kinds — apply surfaces-and-stops
    /// with the named diagnostic (never a guessed kind).
    #[test]
    fn apply_on_generic_enum_head_surfaces_and_stops() {
        let overlay = module_overlay(|compiler| {
            compiler
                .type_tracker
                .schema_registry_mut()
                .register_enum_scoped(
                    "Tree",
                    vec![
                        EnumVariantInfo::new("Leaf", 0, 0),
                        EnumVariantInfo::new("Node", 1, 1),
                    ],
                );
        });
        let constructor = canonical_constructor("Tree", &overlay).expect("Tree constructor mints");
        let int_id = overlay.identity_of("int").expect("int identity");
        let error = canonical_apply(&constructor, &[AppliedArg::Type(int_id)], &overlay)
            .expect_err("enum head apply surfaces-and-stops");
        assert_eq!(error, ENUM_HEAD_PARAM_KIND_UNRECOVERABLE_DIAGNOSTIC);
    }

    // ========================================================================
    // S2 carrier layer: the opaque runtime carriers + orchestration that wire
    // the S1 model into comptime execution. Every carrier is schema-name-
    // checked on decode (forgery-blocking); every identity crosses as int
    // halves; the applied identity is EQUAL to the A2 spelling.
    // ========================================================================
    mod carriers {
        use super::*;
        use shape_value::v2::typed_array::ELEM_TYPE_TYPED_OBJECT;

        /// A carrier `HeapValue` as a decodable `KindedSlot`. Test-only: moves
        /// the single owned share into the slot (the `forget` cancels the
        /// `HeapValue` Drop so the slot owns exactly one share — balanced).
        fn carrier_slot(hv: HeapValue) -> KindedSlot {
            match hv {
                HeapValue::TypedObject(ptr) => {
                    let raw = ptr.0;
                    std::mem::forget(ptr);
                    KindedSlot::from_typed_object_raw(raw)
                }
                _ => panic!("carrier must be a typed object"),
            }
        }

        /// A checked argument array (`TypedArray<*const TypedObjectStorage>`) of
        /// carrier storages. Test-only: leaks the element shares for the
        /// lifetime of the test (apply only borrows them).
        fn args_array(carriers: Vec<HeapValue>) -> KindedSlot {
            let array =
                TypedArray::<*const TypedObjectStorage>::with_capacity(carriers.len() as u32);
            unsafe {
                stamp_elem_type(array as *mut u8, ELEM_TYPE_TYPED_OBJECT);
                for hv in carriers {
                    match hv {
                        HeapValue::TypedObject(ptr) => {
                            let raw = ptr.0;
                            std::mem::forget(ptr);
                            TypedArray::push(array, raw);
                        }
                        _ => panic!("apply argument must be a typed object"),
                    }
                }
            }
            KindedSlot::new(
                ValueSlot::from_raw(array as usize as u64),
                NativeKind::Ptr(HeapKind::TypedArray),
            )
        }

        fn applied_identity_field(slot: &KindedSlot) -> FrozenTypeIdentity {
            let (schema, storage) =
                reserved_storage(slot, COMPTIME_APPLIED_TYPE_SCHEMA, "AppliedType").unwrap();
            identity_halves_of(&schema, storage, "identity_high", "identity_low").unwrap()
        }

        /// The `TypeConstructorRef` carrier round-trips its head identity; a
        /// foreign-schema carrier is a named forgery rejection.
        #[test]
        fn type_constructor_carrier_round_trips_and_forgery_rejects() {
            let overlay = module_overlay(|_| {});
            let head = overlay.identity_of("Option").expect("Option identity");
            let carrier =
                build_type_constructor_ref_heap_value(head, &overlay).expect("constructor carrier");
            let slot = carrier_slot(carrier);
            assert_eq!(type_constructor_head_from_ref(&slot).unwrap(), head);

            // A TypeRef carrier is NOT a TypeConstructorRef — schema-name check
            // blocks the forgery.
            let type_ref = build_frozen_type_ref_heap_value(head, &overlay).expect("type_ref");
            let forged = carrier_slot(type_ref);
            let error = type_constructor_head_from_ref(&forged)
                .expect_err("a TypeRef cannot pose as a TypeConstructorRef");
            assert!(error.contains("TypeConstructorRef"), "{error}");
        }

        /// `build_type_constructor_ref_heap_value` rejects a non-nominal head
        /// and an unknown/INVALID head (R5 / R6).
        #[test]
        fn type_constructor_carrier_rejects_non_nominal_and_unknown() {
            let overlay = module_overlay(|_| {});
            let int_id = overlay.identity_of("int").expect("int identity");
            let error = build_type_constructor_ref_heap_value(int_id, &overlay)
                .expect_err("primitive head rejects");
            assert!(error.contains("nominal"), "{error}");

            let error = build_type_constructor_ref_heap_value(FrozenTypeIdentity::INVALID, &overlay)
                .expect_err("INVALID head rejects");
            assert!(error.contains("unknown semantic type identity"), "{error}");
        }

        /// The load-bearing invariant through the CARRIER path: applying an int
        /// TypeRef carrier to an Option constructor carrier yields an
        /// AppliedType whose identity EQUALS the A2 `type_ref(Option<int>)`
        /// spelling; refine recovers the head + args; type_argument re-issues
        /// the int TypeRef.
        #[test]
        fn apply_over_carriers_is_identity_equal_to_a2_and_round_trips() {
            let overlay = module_overlay(|_| {});
            let int_id = overlay.identity_of("int").expect("int identity");
            let head = overlay.identity_of("Option").expect("Option identity");
            let expected = canon(&overlay, &applied("Option", vec![basic("int")]));

            let constructor = carrier_slot(
                build_type_constructor_ref_heap_value(head, &overlay).expect("constructor"),
            );
            let int_carrier =
                build_frozen_type_ref_heap_value(int_id, &overlay).expect("int type_ref");
            let args = args_array(vec![int_carrier]);

            let applied_slot =
                carrier_slot(apply_to_constructor(&constructor, &args, &overlay).expect("apply"));
            assert_eq!(
                applied_identity_field(&applied_slot),
                expected.identity,
                "identity(apply) == identity(type_ref(Option<int>))"
            );

            let decoded = decode_applied_type(&applied_slot).expect("decode applied");
            assert_eq!(decoded.head_identity, head);
            assert_eq!(decoded.arg_identities, vec![int_id]);

            // refine recovers the application; the extracted argument re-issues
            // the int TypeRef identity.
            let refined = refine_application(&applied_slot, &constructor)
                .expect("refine ok")
                .expect("head matches");
            assert_eq!(applied_identity_field(&carrier_slot(refined)), expected.identity);

            let arg0 = carrier_slot(
                applied_type_argument(&applied_slot, 0, &overlay).expect("type_argument(0)"),
            );
            assert_eq!(frozen_identity_from_ref(&arg0, "test").unwrap(), int_id);

            // Out-of-range index is the named rejection.
            let error = applied_type_argument(&applied_slot, 1, &overlay)
                .expect_err("index 1 out of range");
            assert!(error.contains("out of range"), "{error}");
        }

        /// refine returns None on a head mismatch and on a bare-nominal
        /// (non-AppliedType) receiver — never an error, never partial (R7).
        #[test]
        fn refine_over_carriers_returns_none_on_mismatch_and_bare_nominal() {
            let overlay = module_overlay(|_| {});
            let int_id = overlay.identity_of("int").expect("int identity");
            let option_head = overlay.identity_of("Option").expect("Option identity");
            let result_head = overlay.identity_of("Result").expect("Result identity");

            let option_ctor = carrier_slot(
                build_type_constructor_ref_heap_value(option_head, &overlay).expect("Option ctor"),
            );
            let result_ctor = carrier_slot(
                build_type_constructor_ref_heap_value(result_head, &overlay).expect("Result ctor"),
            );
            let int_carrier =
                build_frozen_type_ref_heap_value(int_id, &overlay).expect("int type_ref");
            let applied = carrier_slot(
                apply_to_constructor(&option_ctor, &args_array(vec![int_carrier]), &overlay)
                    .expect("apply"),
            );

            // Head mismatch → None.
            assert!(
                refine_application(&applied, &result_ctor)
                    .expect("refine ok")
                    .is_none()
            );

            // A bare-nominal TypeRef carrier (not an AppliedType) → None.
            let bare = carrier_slot(
                build_frozen_type_ref_heap_value(option_head, &overlay).expect("bare Option"),
            );
            assert!(
                refine_application(&bare, &option_ctor)
                    .expect("refine ok")
                    .is_none()
            );
        }

        /// A const-generic application through the carrier path: `const_arg`
        /// supplies a `Const` argument to a `Matrix<T, const N>` head, and the
        /// application round-trips through refine.
        #[test]
        fn const_generic_apply_over_carriers_round_trips() {
            let overlay = module_overlay(|compiler| {
                add_mixed_generic_struct(
                    compiler,
                    "Matrix",
                    &[type_param("T"), const_param("N")],
                );
            });
            let int_id = overlay.identity_of("int").expect("int identity");
            let head = overlay.identity_of("Matrix").expect("Matrix identity");
            let const_four_id = canonical_const_arg(4).identity();

            let constructor = carrier_slot(
                build_type_constructor_ref_heap_value(head, &overlay).expect("Matrix ctor"),
            );
            let int_carrier =
                build_frozen_type_ref_heap_value(int_id, &overlay).expect("int type_ref");
            let const_carrier = build_const_arg_ref_heap_value(4).expect("const_arg(4)");
            let args = args_array(vec![int_carrier, const_carrier]);

            let applied =
                carrier_slot(apply_to_constructor(&constructor, &args, &overlay).expect("apply"));
            let decoded = decode_applied_type(&applied).expect("decode");
            assert_eq!(decoded.head_identity, head);
            assert_eq!(decoded.arg_identities, vec![int_id, const_four_id]);
        }
    }
}
