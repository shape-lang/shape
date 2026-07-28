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

mod specialization_overlay;

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

/// ADR-009 B5: register a user struct WITH ordered typed fields (the exact
/// two stores the freeze reads: `struct_types` field order +
/// `struct_generic_info.runtime_field_types` annotations).
fn add_struct_with_fields(
    compiler: &mut BytecodeCompiler,
    name: &str,
    fields: &[(&str, TypeAnnotation)],
) {
    compiler.struct_types.insert(
        name.to_string(),
        (
            fields.iter().map(|(f, _)| (*f).to_string()).collect(),
            shape_ast::ast::Span::DUMMY,
        ),
    );
    compiler.struct_generic_info.insert(
        name.to_string(),
        crate::compiler::StructGenericInfo {
            type_params: Vec::new(),
            runtime_field_types: fields
                .iter()
                .map(|(f, ty)| ((*f).to_string(), ty.clone()))
                .collect(),
        },
    );
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

/// ADR-009 B5 (S2): register a GENERIC user struct WITH ordered typed fields
/// whose annotations may reference the declared type parameters (the exact two
/// stores the freeze reads for the applied-substitution path — `struct_types`
/// field order + `struct_generic_info.{type_params, runtime_field_types}`).
fn add_generic_struct_with_fields(
    compiler: &mut BytecodeCompiler,
    name: &str,
    params: &[&str],
    fields: &[(&str, TypeAnnotation)],
) {
    compiler.struct_types.insert(
        name.to_string(),
        (
            fields.iter().map(|(f, _)| (*f).to_string()).collect(),
            shape_ast::ast::Span::DUMMY,
        ),
    );
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
            runtime_field_types: fields
                .iter()
                .map(|(f, ty)| ((*f).to_string(), ty.clone()))
                .collect(),
        },
    );
}

/// ADR-009 B5 (S2, Dec 55 R10): reflecting an APPLIED user struct substitutes
/// its declared type parameters with the applied arguments BEFORE issuing the
/// field descriptors — the substituted field VALUE-type identity is the applied
/// type's identity, never the un-substituted parameter. `Page<User>` field
/// `items: Array<T>` becomes `Array<User>`; `total: int` is unchanged.
#[test]
fn applied_user_struct_substitutes_field_types_before_issuance() {
    use super::payloads::{FrozenPayloadDescriptor, NominalDescriptor};
    let overlay = module_overlay(|compiler| {
        add_struct_with_fields(compiler, "User", &[("id", basic("int"))]);
        add_generic_struct_with_fields(
            compiler,
            "Page",
            &["T"],
            &[
                ("items", applied("Array", vec![basic("T")])),
                ("total", basic("int")),
            ],
        );
    });
    // Applied Page<User> canonicalizes + site-interns in the overlay memo (the
    // same interning `type_ref(Page<User>)` performs), so the overlay's payload
    // query answers the SUBSTITUTED shape.
    let applied_id = overlay
        .canonicalize_type(&applied("Page", vec![basic("User")]))
        .expect("Page<User> canonicalizes");
    let expected_items = canon(&overlay, &applied("Array", vec![basic("User")])).identity;
    let int_id = overlay.identity_of("int").expect("int frozen");

    let payload = overlay
        .payload_of(applied_id)
        .expect("an applied user struct must substitute + issue its Struct shape");
    let FrozenPayloadDescriptor::Nominal(NominalDescriptor::Struct { fields, .. }) = payload else {
        panic!("Page<User> is a 2-field struct shape, got {payload:?}");
    };
    assert!(
        fields.iter().any(|f| f.type_identity == expected_items),
        "the items field type must be the SUBSTITUTED Array<User> identity, not Array<T>"
    );
    assert!(
        fields.iter().any(|f| f.type_identity == int_id),
        "the total:int field must remain int under substitution"
    );
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

/// ADR-009 B6: the callable's PRESERVED structural descriptor (the widened
/// composite memo input) records ordered per-position params with their
/// passing mode and name. Names are identity-insignificant; passing mode is
/// identity-significant through the borrow wrapper already present in the
/// canonical parameter member and is not duplicated as another string field.
#[test]
fn callable_structural_descriptor_records_names_and_identity_significant_modes() {
    // `PassingMode` is in scope via `use super::*`.
    let overlay = module_overlay(|_| {});

    let borrow = |mutable: bool, inner: &str| TypeAnnotation::Borrow {
        mutable,
        inner: Box::new(basic(inner)),
    };
    let named = |name: &str, annotation: TypeAnnotation| FunctionParam {
        name: Some(name.to_string()),
        optional: false,
        type_annotation: annotation,
    };
    // fn(a: int, b: &string, c: &mut int) -> bool
    let sig = TypeAnnotation::Function {
        params: vec![
            named("a", basic("int")),
            named("b", borrow(false, "string")),
            named("c", borrow(true, "int")),
        ],
        returns: Box::new(basic("bool")),
    };
    let canonical = canon(&overlay, &sig);
    let descriptor = canonical
        .callable
        .as_ref()
        .expect("callable canonicalization preserves a structural descriptor");
    assert_eq!(descriptor.params.len(), 3);
    assert_eq!(descriptor.params[0].mode, PassingMode::Move);
    assert_eq!(descriptor.params[1].mode, PassingMode::SharedBorrow);
    assert_eq!(descriptor.params[2].mode, PassingMode::ExclusiveBorrow);
    assert_eq!(descriptor.params[0].name.as_deref(), Some("a"));
    // The borrow param's VALUE-type identity is the referent (int / string),
    // NOT the reference wrapper.
    assert_eq!(descriptor.params[1].type_identity, overlay.identity_of("string").unwrap());
    assert_eq!(descriptor.params[2].type_identity, overlay.identity_of("int").unwrap());
    assert_eq!(descriptor.returns, overlay.identity_of("bool").unwrap());

    // Renaming every parameter is identity-neutral (names insignificant) and
    // the mode axis is exactly the borrow wrapper the grammar already embeds.
    let renamed = TypeAnnotation::Function {
        params: vec![
            named("x", basic("int")),
            named("y", borrow(false, "string")),
            named("z", borrow(true, "int")),
        ],
        returns: Box::new(basic("bool")),
    };
    assert_eq!(canon(&overlay, &renamed).identity, canonical.identity);
}

/// ADR-009 B6 R2 (Dec 63): a callable parameter modeled with the
/// compiler-internal `Any` top type (bare or `Array<Any>`) is the named
/// Any-erasure rejection; lowercase `any` (the enabled Erased leaf) is not.
#[test]
fn r2_callable_param_erased_to_any_is_the_named_rejection() {
    // `CALLABLE_PARAM_ERASED_TO_ANY_DIAGNOSTIC` is in scope via `use super::*`.
    let overlay = module_overlay(|_| {});

    // fn(Array<Any>) -> bool — the homogeneous top-typed param modeling.
    let array_any = canonicalize_type_annotation(
        &callable(vec![applied("Array", vec![basic("Any")])], basic("bool")),
        &overlay,
    )
    .expect_err("a callable parameter typed Array<Any> must reject");
    assert_eq!(array_any, CALLABLE_PARAM_ERASED_TO_ANY_DIAGNOSTIC);

    // Bare `Any` parameter — same rejection.
    let bare_any = canonicalize_type_annotation(
        &callable(vec![basic("Any")], basic("bool")),
        &overlay,
    )
    .expect_err("a callable parameter typed Any must reject");
    assert_eq!(bare_any, CALLABLE_PARAM_ERASED_TO_ANY_DIAGNOSTIC);

    // Lowercase `any` is the enabled Erased leaf — a callable param typed `any`
    // canonicalizes (NOT the R2 rejection).
    canon(&overlay, &callable(vec![basic("any")], basic("bool")));
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

/// ADR-009 B3 (S2): the existential package descriptor is
/// `exists:{arity}:{inner_hex}`, where the inner descriptor canonicalizes the
/// witnesses POSITIONALLY (`witness:{index}`). This pins the B4/B7 ABI
/// substrate: the identity is alpha-invariant, arity-significant, and distinct
/// from the concrete instantiation of the same head.
#[test]
fn existential_package_descriptor_embeds_positional_witness_identities() {
    let overlay = module_overlay(|compiler| {
        add_struct(compiler, "Owner");
        add_generic_struct(compiler, "Pair", &["A", "B"]);
    });

    let package = canon(
        &overlay,
        &TypeAnnotation::Existential {
            witnesses: vec!["I".to_string(), "F".to_string()],
            inner: Box::new(applied("Pair", vec![basic("I"), basic("F")])),
        },
    );
    assert_eq!(package.category, FrozenTypeCategory::Existential);

    let w0 = identity_hex(FrozenTypeIdentity::from_canonical_descriptor("witness:0"));
    let w1 = identity_hex(FrozenTypeIdentity::from_canonical_descriptor("witness:1"));
    let inner_descriptor = format!("applied:{}<{w0},{w1}>", leaf_hex(&overlay, "Pair"));
    let inner_identity = FrozenTypeIdentity::from_canonical_descriptor(&inner_descriptor);
    assert_eq!(
        package.descriptor,
        format!("exists:2:{}", identity_hex(inner_identity))
    );

    // The package is distinct from the concrete instantiation Pair<Owner,Owner>.
    let concrete = canon(&overlay, &applied("Pair", vec![basic("Owner"), basic("Owner")]));
    assert_ne!(package.identity, concrete.identity);
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
    let hole = tyvar_to_annotation(&TypeVar::new("T3".to_string()));

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
        tyvar_to_annotation(&TypeVar::new("T9".to_string())),
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

    /// Rejection-matrix row R1: each remaining non-enabled category has ONE
    /// named per-category diagnostic — naming the category, stating the
    /// payload descriptor has not landed, and pointing at `type_category` —
    /// never a partial descriptor. Parameter is asserted end-to-end through
    /// a scoped overlay identity (`parameter:{owner}:{name}`). (ADR-009 B5:
    /// Nominal is now ENABLED — a base user struct answers a positive
    /// `FrozenNominal` descriptor; see `base_frozen_nominal_answers_its_shape`.)
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

        // ADR-009 B7 Slice 2: Parameter is now ENABLED — Existential is the
        // SOLE remaining R1 rejection. A scoped overlay Parameter identity
        // answers a complete `TypeParamDescriptor` (see
        // `scoped_parameter_answers_its_stable_identity_payload`), never an R1
        // rejection.
        assert!(
            !FROZEN_TYPE_ENABLED_PAYLOAD_CATEGORIES.contains(&FrozenTypeCategory::Existential),
            "Existential must stay the sole non-enabled category"
        );
        assert!(
            FROZEN_TYPE_ENABLED_PAYLOAD_CATEGORIES.contains(&FrozenTypeCategory::Parameter),
            "Parameter must now be enabled (B7 Slice 2)"
        );
    }

    /// ADR-009 B7 Slice 2 (Dec 50/94): a scoped generic-parameter overlay
    /// identity answers a COMPLETE `TypeParamDescriptor` off the SAME stable
    /// `parameter:{owner}:{name}` identity it interns — with a provably-empty
    /// bound set (`FrozenParameterBound` is uninhabited until B2), never an
    /// inference hole and never a partial descriptor. Identity STABILITY across
    /// two same-owner overlays and DISTINCTNESS across owners are proven at the
    /// payload level (mirroring `parameter_identity_is_scoped_by_owning_function`
    /// at the identity level).
    #[test]
    fn scoped_parameter_answers_its_stable_identity_payload() {
        use super::payloads::{FrozenPayloadDescriptor, TypeParamDescriptor};

        let map_a = FreezeOverlay::new(freeze_of(|_| {}), "map", &["T".to_string()]);
        let map_b = FreezeOverlay::new(freeze_of(|_| {}), "map", &["T".to_string()]);
        let filter = FreezeOverlay::new(freeze_of(|_| {}), "filter", &["T".to_string()]);

        let map_t = map_a.identity_of("T").expect("map T identity");
        let expected = Ok(FrozenPayloadDescriptor::Parameter(TypeParamDescriptor {
            identity: map_t,
            bounds: Vec::new(),
        }));
        assert_eq!(map_a.payload_of(map_t), expected);
        // Same owner ⇒ same payload identity (stable across overlays, the unit
        // mirror of the per-instantiation e2e in frozen_type.rs).
        assert_eq!(map_b.payload_of(map_t), expected);

        // Distinct owner ⇒ distinct identity ⇒ distinct payload.
        let filter_t = filter.identity_of("T").expect("filter T identity");
        assert_ne!(filter_t, map_t, "distinct owners mint distinct identities");
        assert_eq!(
            filter.payload_of(filter_t),
            Ok(FrozenPayloadDescriptor::Parameter(TypeParamDescriptor {
                identity: filter_t,
                bounds: Vec::new(),
            }))
        );
    }

    /// ADR-009 B5 (Dec 55): a base-frozen user nominal answers its complete
    /// sealed `FrozenNominal` declaration-shape descriptor — a zero-field
    /// struct is the non-decomposable `Opaque` shape (the S1 field-count
    /// classification), never a partial descriptor and never an R1 rejection.
    #[test]
    fn base_frozen_nominal_answers_its_shape() {
        use super::payloads::{FrozenPayloadDescriptor, NominalDescriptor};
        let freeze = freeze_of(|compiler| add_struct(compiler, "Alpha"));
        let alpha = freeze.identity_of("Alpha").expect("Alpha identity");
        match freeze.payload_of(alpha).expect("Nominal must answer a shape") {
            FrozenPayloadDescriptor::Nominal(NominalDescriptor::Opaque { owner }) => {
                assert_eq!(owner, alpha, "the shape owner is the reflected identity");
            }
            other => panic!("a zero-field struct must be Opaque, got {other:?}"),
        }
    }

    /// ADR-009 B5 (Dec 55/57): a multi-field struct is `Struct` with owner-bound
    /// member identities (never source-name strings) and each field's
    /// canonicalized VALUE-type identity — a single-field struct is `Newtype`
    /// whose inner identity IS the wrapped field's frozen identity.
    #[test]
    fn nominal_shape_classification_and_member_identities() {
        use super::payloads::{FrozenPayloadDescriptor, NominalDescriptor};

        // Multi-field → Struct with two field descriptors carrying int / string.
        let freeze = freeze_of(|compiler| {
            add_struct_with_fields(
                compiler,
                "Point",
                &[("x", basic("int")), ("y", basic("string"))],
            )
        });
        let point = freeze.identity_of("Point").expect("Point identity");
        let int_id = freeze.identity_of("int").expect("int identity");
        let string_id = freeze.identity_of("string").expect("string identity");
        match freeze.payload_of(point).expect("Point answers a shape") {
            FrozenPayloadDescriptor::Nominal(NominalDescriptor::Struct { owner, fields }) => {
                assert_eq!(owner, point);
                assert_eq!(fields.len(), 2);
                assert_eq!(fields[0].type_identity, int_id);
                assert_eq!(fields[1].type_identity, string_id);
                // Member identities are owner-bound + distinct — NEVER equal to
                // any type identity (Dec 57: a member token is a distinct
                // identity kind from the field's value type).
                assert_ne!(fields[0].member, fields[1].member);
                assert_ne!(fields[0].member, int_id);
            }
            other => panic!("a two-field struct must be Struct, got {other:?}"),
        }

        // Single-field → Newtype whose inner identity is the wrapped field type.
        let freeze = freeze_of(|compiler| {
            add_struct_with_fields(compiler, "UserId", &[("value", basic("int"))])
        });
        let user_id = freeze.identity_of("UserId").expect("UserId identity");
        let int_id = freeze.identity_of("int").expect("int identity");
        match freeze.payload_of(user_id).expect("UserId answers a shape") {
            FrozenPayloadDescriptor::Nominal(NominalDescriptor::Newtype { owner, inner }) => {
                assert_eq!(owner, user_id);
                assert_eq!(inner, int_id, "the newtype inner is the wrapped field type");
            }
            other => panic!("a single-field struct must be Newtype, got {other:?}"),
        }
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

    /// ADR-009 B7 (Dec 50/94): `payload_of` answers the site-interned composites
    /// layer with COMPLETE structural descriptors — the same identity the
    /// overlay minted one call earlier, reconstructed from the widened memo
    /// without inverting the one-way hash. Tuple carries ordered element
    /// identities; Record carries hygienic member + value-type identities +
    /// optionality; Reference carries mutability + referent; Union carries the
    /// deduped member identities. (Formerly the composite R1 rejections.)
    #[test]
    fn site_interned_composites_answer_full_structural_payloads() {
        use super::payloads::{
            FrozenPayloadDescriptor, ReferenceDescriptor, RecordDescriptor, TupleDescriptor,
            UnionDescriptor,
        };
        let overlay = module_overlay(|compiler| add_struct(compiler, "User"));
        let int_id = overlay.identity_of("int").expect("int identity");
        let string_id = overlay.identity_of("string").expect("string identity");
        let user_id = overlay.identity_of("User").expect("User identity");

        // Tuple: ordered element identities (position IS the index).
        let tuple_id = overlay
            .canonicalize_type(&TypeAnnotation::Tuple(vec![basic("int"), basic("string")]))
            .expect("tuple canonicalizes");
        assert_eq!(
            overlay.payload_of(tuple_id),
            Ok(FrozenPayloadDescriptor::Tuple(TupleDescriptor {
                elements: vec![int_id, string_id],
            }))
        );

        // Record: one field carrying int; the hygienic member identity is
        // owner-bound + distinct from the value-type identity (Dec 57).
        let record_id = overlay
            .canonicalize_type(&TypeAnnotation::Object(vec![record_field(
                "x",
                false,
                basic("int"),
            )]))
            .expect("record canonicalizes");
        match overlay.payload_of(record_id) {
            Ok(FrozenPayloadDescriptor::Record(RecordDescriptor { fields })) => {
                assert_eq!(fields.len(), 1);
                assert_eq!(fields[0].type_identity, int_id);
                assert!(!fields[0].optional);
                assert_ne!(fields[0].member, int_id, "member is not the value type");
            }
            other => panic!("record must answer a Record payload, got {other:?}"),
        }

        // Reference: shared borrow of a user nominal.
        let reference_id = overlay
            .canonicalize_type(&TypeAnnotation::Borrow {
                mutable: false,
                inner: Box::new(basic("User")),
            })
            .expect("reference canonicalizes");
        assert_eq!(
            overlay.payload_of(reference_id),
            Ok(FrozenPayloadDescriptor::Reference(ReferenceDescriptor {
                mutable: false,
                referent: user_id,
            }))
        );

        // Union: two deduped members (byte-sorted by identity hex).
        let union_id = overlay
            .canonicalize_type(&TypeAnnotation::Union(vec![basic("int"), basic("string")]))
            .expect("union canonicalizes");
        match overlay.payload_of(union_id) {
            Ok(FrozenPayloadDescriptor::Union(UnionDescriptor { members })) => {
                assert_eq!(members.len(), 2);
                assert!(members.contains(&int_id));
                assert!(members.contains(&string_id));
            }
            other => panic!("union must answer a Union payload, got {other:?}"),
        }

        // ADR-009 E5 CKPT-2 (A8-OUT flip): a site-interned APPLIED builtin enum
        // (`Option<int>`) is a Nominal composite that now RESOLVES to its
        // arity-only `Enum` descriptor via the builtin template — the former
        // applied-substitution-pending rejection is gone for the three
        // applicable-head families. (The dedicated CKPT-2 pins in
        // `e1_s5_ckpt2_descriptor_substitution` cover the Enum shape + arity +
        // A7 payload recovery in full.)
        let applied_identity = overlay
            .canonicalize_type(&applied("Option", vec![basic("int")]))
            .expect("applied nominal canonicalizes");
        assert_eq!(overlay.category_of(applied_identity), Ok(FrozenTypeCategory::Nominal));
        match overlay
            .payload_of(applied_identity)
            .expect("Option<int> now descriptor-substitutes (CKPT-2 A8-OUT)")
        {
            FrozenPayloadDescriptor::Nominal(super::payloads::NominalDescriptor::Enum {
                variants,
                ..
            }) => {
                assert_eq!(variants.len(), 2, "Option reflects None + Some");
            }
            other => panic!("Option<int> must resolve to an Enum descriptor, got {other:?}"),
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

    // ── ADR-009 B6: callable signature descriptor payload ────────────────

    /// A `FunctionParam` with an explicit name/optionality/annotation.
    fn param(name: Option<&str>, optional: bool, annotation: TypeAnnotation) -> FunctionParam {
        FunctionParam {
            name: name.map(str::to_string),
            optional,
            type_annotation: annotation,
        }
    }

    fn function(params: Vec<FunctionParam>, returns: TypeAnnotation) -> TypeAnnotation {
        TypeAnnotation::Function {
            params,
            returns: Box::new(returns),
        }
    }

    /// B6 core: a site-interned callable answers a COMPLETE `FrozenCallable`
    /// payload from the widened composite memo — ordered params with stable
    /// type identities, optionality flags, and passing modes derived from the
    /// borrow annotation, plus the return identity. Reconstructed WITHOUT
    /// inverting the one-way SHA-256 identity (which drops names while modes
    /// remain encoded by the canonical borrow wrapper).
    #[test]
    fn site_interned_callable_answers_full_payload() {
        use super::payloads::{CallableDescriptor, ParamDescriptor};
        use shape_runtime::comptime_reflection::PassingMode;

        let overlay = module_overlay(|_| {});
        // (count: int, label?: string, &mut int, &string) -> bool
        let annotation = function(
            vec![
                param(Some("count"), false, basic("int")),
                param(Some("label"), true, basic("string")),
                param(
                    None,
                    false,
                    TypeAnnotation::Borrow {
                        mutable: true,
                        inner: Box::new(basic("int")),
                    },
                ),
                param(
                    None,
                    false,
                    TypeAnnotation::Borrow {
                        mutable: false,
                        inner: Box::new(basic("string")),
                    },
                ),
            ],
            basic("bool"),
        );
        let identity = overlay
            .canonicalize_type(&annotation)
            .expect("callable canonicalizes");
        assert_eq!(
            overlay.category_of(identity),
            Ok(FrozenTypeCategory::Callable)
        );

        let int_id = overlay.identity_of("int").expect("int identity");
        let string_id = overlay.identity_of("string").expect("string identity");
        let bool_id = overlay.identity_of("bool").expect("bool identity");
        let expected = CallableDescriptor {
            params: vec![
                ParamDescriptor {
                    name: Some("count".to_string()),
                    type_identity: int_id,
                    optional: false,
                    mode: PassingMode::Move,
                },
                ParamDescriptor {
                    name: Some("label".to_string()),
                    type_identity: string_id,
                    optional: true,
                    mode: PassingMode::Move,
                },
                ParamDescriptor {
                    name: None,
                    // Borrowed parameter: mode carries the borrow, type is the referent.
                    type_identity: int_id,
                    optional: false,
                    mode: PassingMode::ExclusiveBorrow,
                },
                ParamDescriptor {
                    name: None,
                    type_identity: string_id,
                    optional: false,
                    mode: PassingMode::SharedBorrow,
                },
            ],
            returns: bool_id,
        };
        assert_eq!(
            overlay.payload_of(identity),
            Ok(FrozenPayloadDescriptor::Callable(expected))
        );
    }

    /// A module-level alias whose target is a callable type (`type Handler =
    /// (int) -> bool`) interns a Callable identity into the BASE index; its
    /// full signature descriptor is preserved in the same rebuild, so the base
    /// `payload_of` answers a complete `FrozenCallable` — never a partial
    /// descriptor (symmetric with the overlay memo).
    #[test]
    fn base_interned_callable_alias_answers_full_payload() {
        let handler = callable(vec![basic("int")], basic("bool"));
        let freeze = freeze_of(|compiler| {
            compiler
                .type_aliases
                .insert("Handler".to_string(), format!("{handler:?}"));
            compiler
                .type_inference
                .env
                .define_type_alias("Handler", &handler, None);
        });
        let identity = freeze
            .identity_of("Handler")
            .expect("alias fixpoint interns the callable target");
        assert_eq!(freeze.category_of(identity), Ok(FrozenTypeCategory::Callable));
        let FrozenPayloadDescriptor::Callable(descriptor) =
            freeze.payload_of(identity).expect("callable payload")
        else {
            panic!("expected a Callable payload");
        };
        assert_eq!(descriptor.params.len(), 1);
        assert_eq!(
            descriptor.params[0].type_identity,
            freeze.identity_of("int").expect("int identity")
        );
        assert_eq!(
            descriptor.returns,
            freeze.identity_of("bool").expect("bool identity")
        );
    }

    /// Parameter NAMES are identity-insignificant (grammar §Callable), yet the
    /// preserved structure keeps them for hygienic `param(#name)` resolution.
    #[test]
    fn callable_identity_is_name_insignificant_but_structure_keeps_names() {
        let overlay = module_overlay(|_| {});
        let named = function(vec![param(Some("a"), false, basic("int"))], basic("bool"));
        let anon = function(vec![param(None, false, basic("int"))], basic("bool"));

        assert_eq!(
            canon(&overlay, &named).identity,
            canon(&overlay, &anon).identity,
            "param names must not affect the canonical identity"
        );

        let identity = overlay.canonicalize_type(&named).expect("callable canonicalizes");
        let FrozenPayloadDescriptor::Callable(descriptor) =
            overlay.payload_of(identity).expect("callable payload")
        else {
            panic!("expected a Callable payload");
        };
        assert_eq!(descriptor.params[0].name.as_deref(), Some("a"));
    }

    /// Positional parameter identity is stable and order-significant: the
    /// descriptor lists params in signature order and reordering re-hashes.
    #[test]
    fn callable_param_positions_are_stable_and_order_significant() {
        let overlay = module_overlay(|_| {});
        let annotation = callable(vec![basic("int"), basic("string")], basic("bool"));
        let identity = overlay.canonicalize_type(&annotation).expect("canonicalizes");
        let FrozenPayloadDescriptor::Callable(descriptor) =
            overlay.payload_of(identity).expect("callable payload")
        else {
            panic!("expected a Callable payload");
        };
        let int_id = overlay.identity_of("int").expect("int identity");
        let string_id = overlay.identity_of("string").expect("string identity");
        assert_eq!(descriptor.params.len(), 2);
        assert_eq!(descriptor.params[0].type_identity, int_id);
        assert_eq!(descriptor.params[1].type_identity, string_id);

        let flipped = callable(vec![basic("string"), basic("int")], basic("bool"));
        assert_ne!(
            canon(&overlay, &annotation).identity,
            canon(&overlay, &flipped).identity
        );
    }

    /// R3 (Dec 52 — ordering): descriptor issuance for an UNRESOLVED signature
    /// is rejected by the ONE freeze-boundary predicate
    /// (`annotation_has_unresolved_inference_variable`) BEFORE any descriptor
    /// is formed and interned — a hole at ANY depth (param, return, nested)
    /// fires the named Dec-52 diagnostic, and nothing is memoized.
    #[test]
    fn callable_with_unresolved_inference_variable_rejects_before_issuance() {
        let overlay = module_overlay(|_| {});
        let holed_param = function(
            vec![param(
                None,
                false,
                tyvar_to_annotation(&TypeVar::new("P".to_string())),
            )],
            basic("bool"),
        );
        let holed_return = function(
            vec![param(None, false, basic("int"))],
            tyvar_to_annotation(&TypeVar::new("R".to_string())),
        );
        let holed_nested = callable(
            vec![TypeAnnotation::Tuple(vec![
                basic("int"),
                tyvar_to_annotation(&TypeVar::new("N".to_string())),
            ])],
            basic("bool"),
        );
        for annotation in [holed_param, holed_return, holed_nested] {
            let error = canonicalize_type_annotation(&annotation, &overlay)
                .expect_err("unresolved signature must reject issuance");
            assert!(
                error.contains("unresolved inference variable"),
                "Dec 52 freeze-boundary diagnostic missing: {error}"
            );
            // The rejection fires BEFORE any descriptor forms — no FrozenCallable
            // is issued, and nothing is interned into the composite memo.
            assert!(
                overlay.canonicalize_type(&annotation).is_err(),
                "an unresolved signature must never mint an identity"
            );
        }
    }

    /// The heap-value builder lowers a callable to a schema-correct nested
    /// descriptor: `FrozenType{__variant: 6, __payload_0: FrozenCallable{params:
    /// [ParamDescriptor…], returns_identity_high/low}}` — ordinal-pinned variant
    /// id (6, never dense), typed nested objects, no rendered type-name strings.
    #[test]
    fn builder_produces_schema_correct_callable_descriptor() {
        let overlay = module_overlay(|_| {});
        let annotation = callable(vec![basic("int")], basic("bool"));
        let identity = overlay.canonicalize_type(&annotation).expect("canonicalizes");

        let frozen = payloads::build_frozen_type_heap_value(identity, &overlay)
            .expect("callable payload builds");
        let frozen_storage = storage_of(&frozen);
        assert_eq!(schema_name_of(frozen_storage), COMPTIME_FROZEN_TYPE_SCHEMA);
        let (variant, payload) = variant_and_payload(frozen_storage);
        assert_eq!(
            variant,
            i64::from(FrozenTypeCategory::Callable.catalog_ordinal()),
            "Callable is catalog ordinal 6, never dense"
        );

        let callable_storage = payload
            .as_typed_object_storage()
            .expect("payload must be a typed object");
        assert_eq!(
            schema_name_of(callable_storage),
            shape_runtime::type_schema::builtin_schemas::COMPTIME_FROZEN_CALLABLE_SCHEMA
        );
        // params (field 0) is a TypedArray; returns halves (fields 1,2) match bool.
        let params = callable_storage
            .clone_field_kinded(0)
            .expect("params must be readable");
        assert_eq!(
            params.kind(),
            NativeKind::Ptr(shape_value::heap_value::HeapKind::TypedArray)
        );
        let bool_id = overlay.identity_of("bool").expect("bool identity");
        let returns_high = callable_storage
            .clone_field_kinded(1)
            .and_then(|slot| slot.as_i64())
            .expect("returns_identity_high");
        let returns_low = callable_storage
            .clone_field_kinded(2)
            .and_then(|slot| slot.as_i64())
            .expect("returns_identity_low");
        assert_eq!(returns_high, bool_id.high);
        assert_eq!(returns_low, bool_id.low);
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

    /// ADR-009 B7 Slice 2: the builder lowers a scoped Parameter identity to a
    /// complete `FrozenParameter` heap value — catalog ordinal 2, carrying the
    /// parameter's identity halves + a provably-empty (typed-array) bound set,
    /// never a partial descriptor and never an R1 rejection.
    #[test]
    fn builder_lowers_scoped_parameter_to_the_frozen_parameter_payload() {
        use shape_runtime::type_schema::builtin_schemas::COMPTIME_FROZEN_PARAMETER_SCHEMA;

        let overlay = FreezeOverlay::new(freeze_of(|_| {}), "map", &["T".to_string()]);
        let t = overlay.identity_of("T").expect("T identity");
        let frozen = payloads::build_frozen_type_heap_value(t, &overlay)
            .expect("Parameter payload builds");
        let frozen_storage = storage_of(&frozen);
        let (variant, payload) = variant_and_payload(frozen_storage);
        assert_eq!(variant, 2, "Parameter is catalog ordinal 2");
        let parameter_storage = payload
            .as_typed_object_storage()
            .expect("payload must be a typed object");
        assert_eq!(
            schema_name_of(parameter_storage),
            COMPTIME_FROZEN_PARAMETER_SCHEMA
        );
        // identity_high/low carry the queried parameter identity; the bound set
        // is the empty typed array (the only reachable form until B2).
        assert_eq!(
            parameter_storage.clone_field_kinded(0).map(|s| s.as_i64()),
            Some(Some(t.high)),
            "identity_high must be the parameter's own frozen identity high half"
        );
        assert_eq!(
            parameter_storage.clone_field_kinded(1).map(|s| s.as_i64()),
            Some(Some(t.low)),
            "identity_low must be the parameter's own frozen identity low half"
        );
        let bounds = parameter_storage
            .clone_field_kinded(2)
            .expect("bounds must be readable");
        assert_eq!(
            bounds.kind(),
            NativeKind::Ptr(shape_value::heap_value::HeapKind::TypedArray)
        );
    }
}

/// ADR-009 §4.1 "one kind vocabulary" (ticket E5 / #21, Tranche 1): DELETION
/// sentinel for the legacy `type_info` reflection vocabulary.
///
/// The `type_info(T)` builtin and its `TypeKindLabel` / `classify_legacy_type_info`
/// / `build_type_info_heap_value` / `__ComptimeTypeInfo` carrier were DELETED
/// (successor: the typed `reflect(type_ref(T))` / `type_category(...)` surface).
/// This sentinel (file-read, same pattern as `executor/tests/no_dynamic.rs`)
/// makes the successor structurally impossible to walk back: it fails the build
/// if any of the deleted DEFINITION forms reappear. It matches definition forms,
/// not bare symbol names, so tombstone comments that describe the deleted code by
/// name (allowed per CLAUDE.md) do not trip it.
#[test]
fn legacy_type_info_vocabulary_is_gone() {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let read = |relative: &str| {
        std::fs::read_to_string(manifest.join(relative))
            .unwrap_or_else(|error| panic!("sentinel could not read {relative}: {error}"))
    };

    // 1. The legacy classifier/record-builder definitions are DELETED from the
    //    reflection module. Definition forms (`enum ` / `fn `), so a tombstone
    //    comment naming the symbols in backticks does not match.
    let reflection = read("src/compiler/comptime_builtins/type_reflection.rs");
    for needle in [
        "enum TypeKindLabel",
        "fn classify_legacy_type_info",
        "fn build_type_info_heap_value",
    ] {
        assert!(
            !reflection.contains(needle),
            "legacy type_info definition `{needle}` reappeared in type_reflection.rs \
             — the E5 deletion (successor: reflect(type_ref(T))) must stay deleted"
        );
    }

    // 2. The `__ComptimeTypeInfo` carrier schema is DELETED. The quoted schema
    //    name (its registration / whitelist / has_type key) must not reappear in
    //    the schema-registration files.
    for relative in [
        "../shape-runtime/src/type_schema/builtin_schemas.rs",
        "src/compiler/post_inference_verify.rs",
    ] {
        let source = read(relative);
        assert!(
            !source.contains("\"__ComptimeTypeInfo\""),
            "the deleted `__ComptimeTypeInfo` carrier schema reappeared in {relative} \
             (registration/whitelist) — it must stay deleted"
        );
    }

    // 3. The `type_info` and `implements` comptime-builtin metadata rows are
    //    DELETED. The catalog-row form (`name: "..."`) must not reappear.
    let metadata = read("../shape-runtime/src/builtin_metadata.rs");
    for needle in ["name: \"type_info\"", "name: \"implements\""] {
        assert!(
            !metadata.contains(needle),
            "a deleted comptime-builtin metadata row (`{needle}`) reappeared in \
             builtin_metadata.rs — type_info/implements stay deleted"
        );
    }

    // 4. The `implements` intrinsic declaration is DELETED from the stdlib
    //    prelude source.
    let intrinsics = read("../shape-runtime/stdlib-src/core/intrinsics.shape");
    assert!(
        !intrinsics.contains("fn implements("),
        "the deleted `implements` intrinsic declaration reappeared in intrinsics.shape \
         — its successor is find_impl(type_ref(T), trait_ref(Tr))"
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
// ADR-009 E5 CKPT-2 (A8-OUT): DESCRIPTOR substitution for applied
// builtins/enums. `substituted_applied_nominal` now answers a COMPLETE,
// NON-FABRICATING descriptor for the two applicable-head families the struct
// path did not cover — a builtin head (container ⇒ Opaque / Option,Result ⇒
// arity-only Enum) and a user ENUM head (reuse the param-agnostic base
// descriptor). SOUNDNESS INVARIANT: no branch fabricates a member type; every
// applied type ARGUMENT (container element/key/value AND enum payload) is
// recovered by the orthogonal `type_argument` query (A7-uniform), never stated
// in the descriptor. SP-5 is the wrong-descriptor control: under A8-OUT
// `Result<int,string>` and `Result<string,int>` produce IDENTICAL descriptors,
// so the swap is visible ONLY via `arg_identities` order — the
// soundness-critical enum-payload mis-type surface under A8-OUT is
// `type_argument`, NOT `payload_of`. F2 covers alias-of-applied (no reflect
// asymmetry); the A3 pin covers the Phantom hole (a generic head whose fields
// do not reference the parameter must NOT reflect monomorphic).
// ============================================================================
mod e1_s5_ckpt2_descriptor_substitution {
    use super::payloads::{FrozenPayloadDescriptor, NominalDescriptor};
    use super::*;
    use shape_runtime::type_schema::EnumVariantInfo;

    // SP-2: `HashMap<string,int>` — a 2-arg container ⇒ `Opaque{owner:
    // HashMap-head}`. Both args are recovered via `type_argument` IN ORDER
    // (catches a drop/reorder); neither is stated in the descriptor.
    #[test]
    fn e1_s5_ckpt2_sp2_hashmap_two_arg_container_opaque_args_in_order() {
        let overlay = module_overlay(|_| {});
        let applied_id = overlay
            .canonicalize_type(&applied("HashMap", vec![basic("string"), basic("int")]))
            .expect("HashMap<string,int> canonicalizes");
        let string_id = overlay.identity_of("string").expect("string frozen");
        let int_id = overlay.identity_of("int").expect("int frozen");
        let refined = overlay
            .applied_nominal_of(applied_id)
            .expect("HashMap<string,int> is a site-interned applied form");

        match overlay.payload_of(applied_id).expect("HashMap descriptor") {
            FrozenPayloadDescriptor::Nominal(NominalDescriptor::Opaque { owner }) => {
                assert_eq!(owner, refined.head_identity, "owner: HashMap head");
            }
            other => panic!("HashMap<string,int> must be an Opaque container, got {other:?}"),
        }
        assert_eq!(refined.arg_identities.len(), 2, "HashMap carries two args");
        assert_eq!(
            type_argument(&refined, 0),
            Ok(string_id),
            "arg 0 = string, IN ORDER (A7 recovery)"
        );
        assert_eq!(
            type_argument(&refined, 1),
            Ok(int_id),
            "arg 1 = int, IN ORDER (catches a drop/reorder)"
        );
    }

    // SP-3 (headline): `Result<int,string>` ⇒ arity-only `Enum{owner:
    // Result-head}` with TRUE variant names (locked via the owner-bound member
    // identity) + arities. NO payload type in the descriptor (A8-OUT).
    #[test]
    fn e1_s5_ckpt2_sp3_result_enum_descriptor_arity_only() {
        let overlay = module_overlay(|_| {});
        let applied_id = overlay
            .canonicalize_type(&applied("Result", vec![basic("int"), basic("string")]))
            .expect("Result<int,string> canonicalizes");
        let refined = overlay
            .applied_nominal_of(applied_id)
            .expect("Result<int,string> is a site-interned applied form");
        let ok_member = FrozenTypeIdentity::from_canonical_descriptor("member:Result:Ok");
        let err_member = FrozenTypeIdentity::from_canonical_descriptor("member:Result:Err");

        match overlay.payload_of(applied_id).expect("Result descriptor") {
            FrozenPayloadDescriptor::Nominal(NominalDescriptor::Enum { owner, variants }) => {
                assert_eq!(owner, refined.head_identity, "owner: Result head");
                assert_eq!(variants.len(), 2, "Result has exactly Ok + Err");
                let ok = variants
                    .iter()
                    .find(|v| v.member == ok_member)
                    .expect("the Ok variant is present (name-bound member identity)");
                assert_eq!(ok.payload_arity, 1, "Ok carries one payload");
                let err = variants
                    .iter()
                    .find(|v| v.member == err_member)
                    .expect("the Err variant is present (name-bound member identity)");
                assert_eq!(err.payload_arity, 1, "Err carries one payload");
            }
            other => panic!("Result<int,string> must be an Enum, got {other:?}"),
        }

        // A8-OUT: BOTH payloads are recovered via `type_argument` (arg 0 = Ok's,
        // arg 1 = Err's), never stated in the descriptor.
        let int_id = overlay.identity_of("int").expect("int frozen");
        let string_id = overlay.identity_of("string").expect("string frozen");
        assert_eq!(type_argument(&refined, 0), Ok(int_id), "Ok payload via type_argument");
        assert_eq!(
            type_argument(&refined, 1),
            Ok(string_id),
            "Err payload via type_argument"
        );
    }

    // SP-4: `Option<int>` ⇒ `Enum` with `None` arity 0 / `Some` arity 1; the
    // Some payload recovered via `type_argument` (A8-OUT).
    #[test]
    fn e1_s5_ckpt2_sp4_option_enum_descriptor_none_some_arities() {
        let overlay = module_overlay(|_| {});
        let applied_id = overlay
            .canonicalize_type(&applied("Option", vec![basic("int")]))
            .expect("Option<int> canonicalizes");
        let refined = overlay
            .applied_nominal_of(applied_id)
            .expect("Option<int> is a site-interned applied form");
        let none_member = FrozenTypeIdentity::from_canonical_descriptor("member:Option:None");
        let some_member = FrozenTypeIdentity::from_canonical_descriptor("member:Option:Some");

        match overlay.payload_of(applied_id).expect("Option descriptor") {
            FrozenPayloadDescriptor::Nominal(NominalDescriptor::Enum { owner, variants }) => {
                assert_eq!(owner, refined.head_identity, "owner: Option head");
                assert_eq!(variants.len(), 2, "Option has None + Some");
                let none = variants
                    .iter()
                    .find(|v| v.member == none_member)
                    .expect("the None variant is present");
                assert_eq!(none.payload_arity, 0, "None is a unit variant");
                let some = variants
                    .iter()
                    .find(|v| v.member == some_member)
                    .expect("the Some variant is present");
                assert_eq!(some.payload_arity, 1, "Some carries one payload");
            }
            other => panic!("Option<int> must be an Enum, got {other:?}"),
        }
        let int_id = overlay.identity_of("int").expect("int frozen");
        assert_eq!(
            type_argument(&refined, 0),
            Ok(int_id),
            "the Some payload is recovered via type_argument (A8-OUT)"
        );
    }

    // SP-5 (WRONG-DESCRIPTOR control): under A8-OUT the enum-payload swap is NOT
    // visible at the descriptor level — `Result<int,string>` and
    // `Result<string,int>` produce IDENTICAL descriptors (owner + arity-only
    // variants are param-agnostic). The swap IS visible ONLY via the
    // `arg_identities` order (`type_argument`), which is therefore the
    // soundness-critical enum-payload mis-type surface under A8-OUT. A mis-bind
    // that leaked payload types INTO the descriptor would make the two
    // descriptors differ — this pin proves they do not, and that the order
    // channel distinguishes them.
    #[test]
    fn e1_s5_ckpt2_sp5_result_swap_equal_descriptor_visible_only_via_arg_order() {
        let overlay = module_overlay(|_| {});
        let ab = overlay
            .canonicalize_type(&applied("Result", vec![basic("int"), basic("string")]))
            .expect("Result<int,string> canonicalizes");
        let ba = overlay
            .canonicalize_type(&applied("Result", vec![basic("string"), basic("int")]))
            .expect("Result<string,int> canonicalizes");
        assert_ne!(ab, ba, "the two applied identities themselves differ");

        let desc_ab = overlay.payload_of(ab).expect("Result<int,string> descriptor");
        let desc_ba = overlay.payload_of(ba).expect("Result<string,int> descriptor");
        assert_eq!(
            desc_ab, desc_ba,
            "A8-OUT: the swap is NOT visible at the descriptor level (arity-only \
             Enum is param-agnostic) — a mis-bind leaking payload types would fail here"
        );

        // The swap IS visible via the orthogonal arg channel.
        let refined_ab = overlay.applied_nominal_of(ab).expect("ab applied form");
        let refined_ba = overlay.applied_nominal_of(ba).expect("ba applied form");
        assert_ne!(
            refined_ab.arg_identities, refined_ba.arg_identities,
            "the swap IS visible via arg_identities order (the soundness surface)"
        );
        let int_id = overlay.identity_of("int").expect("int frozen");
        let string_id = overlay.identity_of("string").expect("string frozen");
        assert_eq!(type_argument(&refined_ab, 0), Ok(int_id));
        assert_eq!(type_argument(&refined_ab, 1), Ok(string_id));
        assert_eq!(type_argument(&refined_ba, 0), Ok(string_id));
        assert_eq!(type_argument(&refined_ba, 1), Ok(int_id));
    }

    // SP-6 (user ENUM Branch B): an applied user generic enum
    // `Either<int,string>` reuses the param-AGNOSTIC arity-only base descriptor
    // (member ids + arities are `T`-free). The applied descriptor is EQUAL to
    // the base head descriptor; the payloads are recovered via `type_argument`.
    #[test]
    fn e1_s5_ckpt2_sp6_user_enum_reuses_arity_only_base_descriptor() {
        let overlay = module_overlay(|compiler| {
            compiler.type_tracker.schema_registry_mut().register_enum_scoped(
                "Either",
                vec![
                    EnumVariantInfo::new("Left", 0, 1),
                    EnumVariantInfo::new("Right", 1, 1),
                ],
            );
        });
        let applied_id = overlay
            .canonicalize_type(&applied("Either", vec![basic("int"), basic("string")]))
            .expect("Either<int,string> canonicalizes");
        let refined = overlay
            .applied_nominal_of(applied_id)
            .expect("Either<int,string> is a site-interned applied form");
        let base_id = overlay.identity_of("Either").expect("Either head frozen");

        let applied_desc = overlay.payload_of(applied_id).expect("applied Either descriptor");
        let base_desc = overlay.payload_of(base_id).expect("base Either descriptor");
        assert_eq!(
            applied_desc, base_desc,
            "Branch B reuse-base: the applied descriptor IS the arity-only base \
             enum descriptor (SOUND under A8-OUT — param-agnostic)"
        );
        match applied_desc {
            FrozenPayloadDescriptor::Nominal(NominalDescriptor::Enum { owner, variants }) => {
                assert_eq!(owner, refined.head_identity, "owner: Either head");
                assert_eq!(variants.len(), 2, "Either has Left + Right");
                assert!(
                    variants.iter().all(|v| v.payload_arity == 1),
                    "Left / Right each carry one payload"
                );
            }
            other => panic!("Either<int,string> must be an Enum via Branch B, got {other:?}"),
        }
        let int_id = overlay.identity_of("int").expect("int frozen");
        let string_id = overlay.identity_of("string").expect("string frozen");
        assert_eq!(type_argument(&refined, 0), Ok(int_id), "Left payload via type_argument");
        assert_eq!(
            type_argument(&refined, 1),
            Ok(string_id),
            "Right payload via type_argument"
        );
    }

    // SP-7 (nested / A2 identity-indirected): `Array<Result<int,string>>` ⇒
    // `Opaque{owner: Array-head}`; its single arg IS the `Result<int,string>`
    // applied identity, whose own `payload_of` is the Result `Enum` — the
    // recursion is identity-indirected over the finite args and TERMINATES
    // (never an eager field expansion).
    #[test]
    fn e1_s5_ckpt2_sp7_nested_container_over_enum_terminates() {
        let overlay = module_overlay(|_| {});
        let outer = overlay
            .canonicalize_type(&TypeAnnotation::Array(Box::new(applied(
                "Result",
                vec![basic("int"), basic("string")],
            ))))
            .expect("Array<Result<int,string>> canonicalizes");
        // Canonicalize the inner form directly so it is site-interned (its own
        // memo entry backs `payload_of` regardless of outer recursion).
        let result_id = overlay
            .canonicalize_type(&applied("Result", vec![basic("int"), basic("string")]))
            .expect("Result<int,string> canonicalizes");
        let refined_outer = overlay
            .applied_nominal_of(outer)
            .expect("outer is a site-interned applied form");

        match overlay.payload_of(outer).expect("outer descriptor") {
            FrozenPayloadDescriptor::Nominal(NominalDescriptor::Opaque { owner }) => {
                assert_eq!(owner, refined_outer.head_identity, "owner: Array head");
            }
            other => panic!("Array<Result<..>> must be an Opaque container, got {other:?}"),
        }
        assert_eq!(refined_outer.arg_identities.len(), 1, "one outer arg");
        let arg0 = type_argument(&refined_outer, 0).expect("outer arg 0");
        assert_eq!(
            arg0, result_id,
            "the outer arg IS the Result<int,string> applied identity (A2 identity-indirected)"
        );
        match overlay.payload_of(arg0).expect("inner descriptor terminates") {
            FrozenPayloadDescriptor::Nominal(NominalDescriptor::Enum { variants, .. }) => {
                assert_eq!(variants.len(), 2, "the inner Result reflects its Enum shape");
            }
            other => panic!("payload_of(inner arg) must be the Result Enum, got {other:?}"),
        }
    }

    // SP-8 (non-perturbation invariant): a NON-generic base enum
    // `Color{Red,Green(int),Blue}` reflects its exact member identities +
    // arities — CKPT-2's Branch A/B/F2 do NOT touch the base enum path, so the
    // Color descriptor is unchanged (member ids name-derived, arities from the
    // freeze projection).
    #[test]
    fn e1_s5_ckpt2_sp8_nongeneric_enum_descriptor_unchanged() {
        let overlay = module_overlay(|compiler| {
            compiler.type_tracker.schema_registry_mut().register_enum_scoped(
                "Color",
                vec![
                    EnumVariantInfo::new("Red", 0, 0),
                    EnumVariantInfo::new("Green", 1, 1),
                    EnumVariantInfo::new("Blue", 2, 0),
                ],
            );
        });
        let color_id = overlay.identity_of("Color").expect("Color head frozen");
        let red = FrozenTypeIdentity::from_canonical_descriptor("member:Color:Red");
        let green = FrozenTypeIdentity::from_canonical_descriptor("member:Color:Green");
        let blue = FrozenTypeIdentity::from_canonical_descriptor("member:Color:Blue");

        match overlay.payload_of(color_id).expect("Color descriptor") {
            FrozenPayloadDescriptor::Nominal(NominalDescriptor::Enum { owner, variants }) => {
                assert_eq!(owner, color_id, "a base enum's owner is its own head identity");
                assert_eq!(variants.len(), 3, "Red + Green + Blue");
                let arity = |m| variants.iter().find(|v| v.member == m).map(|v| v.payload_arity);
                assert_eq!(arity(red), Some(0), "Red is a unit variant");
                assert_eq!(arity(green), Some(1), "Green carries one payload");
                assert_eq!(arity(blue), Some(0), "Blue is a unit variant");
            }
            other => panic!("Color must be an Enum, got {other:?}"),
        }
    }

    // A3 + PHANTOM guard: a bare GENERIC struct head must be the named
    // unapplied-generic-head rejection — NOT a monomorphic descriptor. Two
    // cases: `Box<T>{value:T}` (param USED in a field — already excluded because
    // `value:T` fails base canonicalization) AND `Phantom<T>{tag:int}` (param
    // UNUSED — all fields canonicalize under the base, the F3 hole this pin
    // closes: without the generic-head exclusion it would land `Struct{tag:int}`
    // and both reflect + spell as MONOMORPHIC, bypassing A3).
    #[test]
    fn e1_s5_ckpt2_bare_generic_struct_head_stays_unapplied_rejection_incl_phantom() {
        let overlay = module_overlay(|compiler| {
            add_generic_struct_with_fields(compiler, "Box", &["T"], &[("value", basic("T"))]);
            add_generic_struct_with_fields(compiler, "Phantom", &["T"], &[("tag", basic("int"))]);
        });
        for head in ["Box", "Phantom"] {
            let id = overlay
                .canonicalize_type(&basic(head))
                .unwrap_or_else(|e| panic!("bare {head} head canonicalizes: {e}"));
            assert_eq!(
                overlay.payload_of(id),
                Err(payloads::unapplied_generic_head_rejection()),
                "the bare generic head {head} must be the named A3 rejection, NOT a \
                 monomorphic descriptor (Phantom: params unused in fields)"
            );
            assert_eq!(
                overlay.bare_nominal_name_of(id),
                None,
                "{head} must not spell as a bare monomorphic nominal"
            );
        }
    }

    // F2 (alias-of-applied builtin): `type Ints = Array<int>` resolves to the
    // transparent applied identity; reflecting the alias substitutes lazily via
    // the base arm (`base_applied_nominals`), so alias-of-applied reflects
    // exactly as the direct `reflect(Array<int>)` does — no reflect asymmetry.
    #[test]
    fn e1_s5_ckpt2_f2_alias_of_applied_builtin_reflects_via_base_arm() {
        let target = applied("Array", vec![basic("int")]);
        let overlay = module_overlay(|compiler| {
            compiler
                .type_aliases
                .insert("Ints".to_string(), format!("{target:?}"));
            compiler
                .type_inference
                .env
                .define_type_alias("Ints", &target, None);
        });
        let ints_id = overlay.identity_of("Ints").expect("Ints alias resolves");
        let array_int = overlay
            .canonicalize_type(&applied("Array", vec![basic("int")]))
            .expect("Array<int> canonicalizes");
        assert_eq!(ints_id, array_int, "alias transparency: Ints == Array<int>");
        match overlay
            .payload_of(ints_id)
            .expect("alias-of-applied reflects via the base arm (F2), never a pending gap")
        {
            FrozenPayloadDescriptor::Nominal(NominalDescriptor::Opaque { .. }) => {}
            other => panic!(
                "type Ints = Array<int> must reflect as Opaque (no reflect asymmetry), got {other:?}"
            ),
        }
    }

    // F2 (alias-of-applied struct): `type PageOfInt = Page<int>` reflects its
    // SUBSTITUTED Struct shape via the base arm — the pre-existing struct-path
    // asymmetry (direct `Page<int>` substituted while the alias pended) is
    // closed symmetrically with the builtin case.
    #[test]
    fn e1_s5_ckpt2_f2_alias_of_applied_struct_reflects_via_base_arm() {
        let target = applied("Page", vec![basic("int")]);
        let overlay = module_overlay(|compiler| {
            add_generic_struct_with_fields(
                compiler,
                "Page",
                &["T"],
                &[
                    ("items", applied("Array", vec![basic("T")])),
                    ("total", basic("int")),
                ],
            );
            compiler
                .type_aliases
                .insert("PageOfInt".to_string(), format!("{target:?}"));
            compiler
                .type_inference
                .env
                .define_type_alias("PageOfInt", &target, None);
        });
        let page_of_int = overlay.identity_of("PageOfInt").expect("PageOfInt resolves");
        let expected_items = overlay
            .canonicalize_type(&applied("Array", vec![basic("int")]))
            .expect("Array<int> canonicalizes");
        match overlay
            .payload_of(page_of_int)
            .expect("alias-of-applied struct reflects its substituted shape via the base arm (F2)")
        {
            FrozenPayloadDescriptor::Nominal(NominalDescriptor::Struct { fields, .. }) => {
                assert_eq!(fields.len(), 2, "Page has 2 fields");
                assert!(
                    fields.iter().any(|f| f.type_identity == expected_items),
                    "the items field is SUBSTITUTED to Array<int> via the base arm, not Array<T>"
                );
            }
            other => panic!(
                "type PageOfInt = Page<int> must reflect its substituted Struct shape, got {other:?}"
            ),
        }
    }
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

        /// ADR-009 B5 (Dec 56): the `RepresentationAccess` authority carrier
        /// round-trips, and `reflect_repr` over a matching (TypeRef, authority)
        /// pair answers the complete Nominal payload. A non-authority slot in the
        /// authority position is the named R6 rejection.
        #[test]
        fn representation_access_carrier_gates_reflect_repr() {
            use super::super::payloads::FrozenPayloadDescriptor;
            let overlay = module_overlay(|compiler| {
                add_generic_struct_with_fields(
                    compiler,
                    "User",
                    &[],
                    &[("id", basic("int")), ("name", basic("string"))],
                );
            });
            let user_id = overlay.identity_of("User").expect("User identity");

            let type_ref =
                carrier_slot(build_frozen_type_ref_heap_value(user_id, &overlay).expect("type_ref"));
            let access = carrier_slot(
                build_representation_access_heap_value(user_id, &overlay).expect("access"),
            );

            // Matching authority → complete Nominal payload.
            let frozen = frozen_type_from_repr_ref(&type_ref, &access, &overlay)
                .expect("authorized reflect_repr");
            let payload = overlay
                .payload_of(user_id)
                .expect("nominal payload for the frozen struct");
            assert!(matches!(payload, FrozenPayloadDescriptor::Nominal(_)));
            drop(frozen);

            // A TypeRef in the authority position is NOT a RepresentationAccess:
            // schema-name check → named R6 rejection, never usable authority.
            let type_ref2 =
                carrier_slot(build_frozen_type_ref_heap_value(user_id, &overlay).expect("type_ref"));
            let forged = carrier_slot(
                build_frozen_type_ref_heap_value(user_id, &overlay).expect("forged authority"),
            );
            let error = frozen_type_from_repr_ref(&type_ref2, &forged, &overlay)
                .expect_err("a TypeRef cannot authorize representation reflection");
            assert!(
                error.contains("requires explicit RepresentationAccess<T> authority"),
                "{error}"
            );
        }

        /// ADR-009 B5 (Dec 56): authority is not ambient — a capability minted
        /// for one type cannot decompose another. `reflect_repr(type_ref(Other),
        /// access_for_User)` is the named cross-type rejection.
        #[test]
        fn representation_access_is_bound_to_its_own_type() {
            let overlay = module_overlay(|compiler| {
                add_generic_struct_with_fields(compiler, "User", &[], &[("id", basic("int"))]);
                add_generic_struct_with_fields(compiler, "Other", &[], &[("x", basic("int"))]);
            });
            let user_id = overlay.identity_of("User").expect("User identity");
            let other_id = overlay.identity_of("Other").expect("Other identity");

            let other_ref =
                carrier_slot(build_frozen_type_ref_heap_value(other_id, &overlay).expect("type_ref"));
            let user_access = carrier_slot(
                build_representation_access_heap_value(user_id, &overlay).expect("access"),
            );
            let error = frozen_type_from_repr_ref(&other_ref, &user_access, &overlay)
                .expect_err("a User authority cannot reflect Other");
            assert!(error.contains("bound to a different type identity"), "{error}");
        }

        /// ADR-009 B5 (Dec 56): the mint re-validates the identity through the
        /// freeze — an unknown/INVALID identity cannot become authority.
        #[test]
        fn representation_access_mint_rejects_unknown_identity() {
            let overlay = module_overlay(|_| {});
            let error = build_representation_access_heap_value(FrozenTypeIdentity::INVALID, &overlay)
                .expect_err("INVALID identity mints no authority");
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
