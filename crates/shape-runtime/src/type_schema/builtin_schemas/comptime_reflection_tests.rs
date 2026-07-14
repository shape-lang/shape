use super::*;

#[test]
fn opaque_type_ref_schema_contains_only_identity_halves() {
    let mut registry = TypeSchemaRegistry::new();
    register_builtin_schemas(&mut registry);

    let type_ref = registry.get(COMPTIME_FROZEN_TYPE_REF_SCHEMA).unwrap();
    let fields: Vec<_> = type_ref
        .fields
        .iter()
        .map(|field| field.name.as_str())
        .collect();
    assert_eq!(fields, ["identity_high", "identity_low"]);
    assert!(
        !fields
            .iter()
            .any(|name| matches!(*name, "name" | "kind" | "source"))
    );
}

/// ADR-009 (ticket B2, slice S3): the reserved `TraitRef` carrier is opaque —
/// identity halves only, no name/kind/source text (Dec 49: no name-based
/// lookup survives into the carrier).
#[test]
fn opaque_trait_ref_schema_contains_only_identity_halves() {
    let mut registry = TypeSchemaRegistry::new();
    register_builtin_schemas(&mut registry);

    let trait_ref = registry.get(COMPTIME_FROZEN_TRAIT_REF_SCHEMA).unwrap();
    let fields: Vec<_> = trait_ref
        .fields
        .iter()
        .map(|field| field.name.as_str())
        .collect();
    assert_eq!(fields, ["identity_high", "identity_low"]);
    assert!(
        !fields
            .iter()
            .any(|name| matches!(*name, "name" | "kind" | "source"))
    );
}

/// ADR-009 (ticket B2, slice S3): the reserved `ImplRef` carrier ties
/// evidence to the exact `(type, trait)` identity pair AND to the exact
/// (possibly named) impl whose canonical identity enters generated-artifact
/// descriptor fingerprints (Dec 49). Identity halves only — no text fields.
#[test]
fn opaque_impl_ref_schema_ties_evidence_to_the_exact_pair_and_impl() {
    let mut registry = TypeSchemaRegistry::new();
    register_builtin_schemas(&mut registry);

    let impl_ref = registry.get(COMPTIME_FROZEN_IMPL_REF_SCHEMA).unwrap();
    let fields: Vec<_> = impl_ref
        .fields
        .iter()
        .map(|field| field.name.as_str())
        .collect();
    assert_eq!(
        fields,
        [
            "trait_identity_high",
            "trait_identity_low",
            "type_identity_high",
            "type_identity_low",
            "impl_identity_high",
            "impl_identity_low",
        ]
    );
    assert!(
        !fields
            .iter()
            .any(|name| matches!(*name, "name" | "kind" | "source" | "trait" | "impl_name"))
    );
}

/// ADR-009 B1 S1: the four payload-descriptor schemas register with
/// unspellable (`\u{1}`-prefixed) names, so Shape source can never construct
/// a lookalike nominal carrier (rejection-matrix row R7).
#[test]
fn frozen_descriptor_schemas_register_with_unspellable_names() {
    let mut registry = TypeSchemaRegistry::new();
    register_builtin_schemas(&mut registry);

    for name in [
        COMPTIME_FROZEN_TYPE_SCHEMA,
        COMPTIME_FROZEN_PRIMITIVE_SCHEMA,
        COMPTIME_FROZEN_NEVER_SCHEMA,
        COMPTIME_FROZEN_ERASED_SCHEMA,
    ] {
        assert!(name.contains('\u{1}'), "schema name must be unspellable");
        assert!(
            registry.has_type(name),
            "descriptor schema {name:?} must be registered"
        );
    }
}

/// The `FrozenType` sealed sum declares EXACTLY the enabled payload variants
/// (Primitive/Never/Erased), each with payload arity 1, with variant ids
/// pinned to the Dec 50/94 catalog ordinals (0/1/9) — not densely
/// renumbered — for comptime-ABI stability across later B tickets. There is
/// no Unknown/Any arm and no partially-declared non-enabled variant.
#[test]
fn frozen_type_schema_has_exactly_the_ordinal_pinned_enabled_variants() {
    let mut registry = TypeSchemaRegistry::new();
    register_builtin_schemas(&mut registry);

    let info = registry
        .get(COMPTIME_FROZEN_TYPE_SCHEMA)
        .and_then(|schema| schema.get_enum_info())
        .expect("FrozenType descriptor schema must be an enum");
    let actual: Vec<_> = info
        .variants
        .iter()
        .map(|variant| (variant.name.as_str(), variant.id, variant.payload_fields))
        .collect();
    // ADR-009 B6: Callable is an enabled payload variant (catalog ordinal 6).
    // ADR-009 B5: Nominal is an enabled payload variant (catalog ordinal 3).
    // Ordinals are the catalog declaration positions, never densely renumbered.
    assert_eq!(
        actual,
        [
            ("Primitive", 0, 1),
            ("Never", 1, 1),
            ("Erased", 9, 1),
            ("Callable", 6, 1),
            ("Nominal", 3, 1),
            // ADR-009 B7: the four composite payloads at their catalog ordinals
            // (Tuple=4, Record=5, Reference=7, Union=8) — declaration order
            // follows the enabled-catalog append order, ids stay ordinal-pinned.
            ("Tuple", 4, 1),
            ("Record", 5, 1),
            ("Reference", 7, 1),
            ("Union", 8, 1),
            // ADR-009 B7 Slice 2: Parameter at catalog ordinal 2, completing the
            // ten-category catalog.
            ("Parameter", 2, 1),
        ]
    );
    assert_eq!(info.variant_id("Unknown"), None);
    assert_eq!(info.variant_id("Any"), None);
    // ADR-009 B7 Slice 2: Parameter is now enabled; only Existential (B3-S3)
    // stays non-enabled and must NOT be declared as a stub variant.
    for pending in ["Existential"] {
        assert_eq!(
            info.variant_id(pending),
            None,
            "{pending} payload ticket has not landed; it must not be declared"
        );
    }
}

/// The `FrozenPrimitive` schema is generated from the shared runtime
/// catalog (`FROZEN_PRIMITIVE_VARIANTS`): same names, same order, same
/// width/domain payload arities. No second hand-written variant list.
#[test]
fn frozen_primitive_schema_matches_the_runtime_catalog() {
    let mut registry = TypeSchemaRegistry::new();
    register_builtin_schemas(&mut registry);

    let info = registry
        .get(COMPTIME_FROZEN_PRIMITIVE_SCHEMA)
        .and_then(|schema| schema.get_enum_info())
        .expect("FrozenPrimitive descriptor schema must be an enum");
    let actual: Vec<_> = info
        .variants
        .iter()
        .map(|variant| (variant.name.as_str(), variant.id, variant.payload_fields))
        .collect();
    let expected: Vec<_> = crate::comptime_reflection::FROZEN_PRIMITIVE_VARIANTS
        .iter()
        .enumerate()
        .map(|(id, variant)| (variant.name, id as u16, variant.payload_arity))
        .collect();
    assert_eq!(actual, expected);
    assert_eq!(info.variant_id("Unknown"), None);
}

/// `FrozenNever` is a zero-field marker struct; `FrozenErased` carries only
/// the bound-set array (reachable today only as the empty set via `any` —
/// A2 unlanded; no field pretends dyn-Trait bounds exist).
#[test]
fn frozen_never_and_erased_schemas_have_the_declared_shape() {
    let mut registry = TypeSchemaRegistry::new();
    register_builtin_schemas(&mut registry);

    let never = registry.get(COMPTIME_FROZEN_NEVER_SCHEMA).unwrap();
    assert_eq!(never.field_count(), 0);

    let erased = registry.get(COMPTIME_FROZEN_ERASED_SCHEMA).unwrap();
    let fields: Vec<_> = erased
        .fields
        .iter()
        .map(|field| field.name.as_str())
        .collect();
    assert_eq!(fields, ["bounds"]);
}

/// ADR-009 B1 S2: the width-domain enums (`IntegerWidth` / `FloatWidth`,
/// the `FrozenPrimitive` family payloads) register as unit-variant enums
/// generated from the shared runtime catalog (`IntegerWidth::ALL` /
/// `FloatWidth::ALL`). Spellable names, following the `FrozenTypeCategory`
/// precedent — user comptime code matches their variants; the lift wall
/// keeps the values out of runtime code.
#[test]
fn width_domain_schemas_match_the_runtime_catalog() {
    use crate::comptime_reflection::{
        FLOAT_WIDTH_SCHEMA_NAME, FloatWidth, INTEGER_WIDTH_SCHEMA_NAME, IntegerWidth,
    };
    let mut registry = TypeSchemaRegistry::new();
    register_builtin_schemas(&mut registry);

    let integer = registry
        .get(INTEGER_WIDTH_SCHEMA_NAME)
        .and_then(|schema| schema.get_enum_info())
        .expect("IntegerWidth schema must be an enum");
    let actual: Vec<_> = integer
        .variants
        .iter()
        .map(|variant| (variant.name.as_str(), variant.id, variant.payload_fields))
        .collect();
    let expected: Vec<_> = IntegerWidth::ALL
        .into_iter()
        .enumerate()
        .map(|(id, width)| (width.variant_name(), id as u16, 0))
        .collect();
    assert_eq!(actual, expected);
    assert_eq!(integer.variant_id("Unknown"), None);

    let float = registry
        .get(FLOAT_WIDTH_SCHEMA_NAME)
        .and_then(|schema| schema.get_enum_info())
        .expect("FloatWidth schema must be an enum");
    let actual: Vec<_> = float
        .variants
        .iter()
        .map(|variant| (variant.name.as_str(), variant.id, variant.payload_fields))
        .collect();
    let expected: Vec<_> = FloatWidth::ALL
        .into_iter()
        .enumerate()
        .map(|(id, width)| (width.variant_name(), id as u16, 0))
        .collect();
    assert_eq!(actual, expected);
}

/// Dec 50/94 required rejection: no descriptor schema exposes a string
/// `kind` field (the `info.kind == "record"` form must have nothing to
/// resolve against) and no nullable/optional category field.
#[test]
fn no_descriptor_schema_has_a_string_kind_field() {
    let mut registry = TypeSchemaRegistry::new();
    register_builtin_schemas(&mut registry);

    for name in [
        COMPTIME_FROZEN_TYPE_SCHEMA,
        COMPTIME_FROZEN_PRIMITIVE_SCHEMA,
        COMPTIME_FROZEN_NEVER_SCHEMA,
        COMPTIME_FROZEN_ERASED_SCHEMA,
        COMPTIME_FROZEN_TYPE_REF_SCHEMA,
        "FrozenTypeCategory",
    ] {
        let schema = registry.get(name).unwrap();
        assert!(
            !schema.fields.iter().any(|field| field.name == "kind"),
            "descriptor schema {name:?} must not expose a string kind field"
        );
        assert!(
            !schema
                .fields
                .iter()
                .any(|field| field.name.contains("category")),
            "descriptor schema {name:?} must not expose a nullable category field"
        );
    }
}

#[test]
fn frozen_type_category_schema_matches_the_runtime_catalog() {
    let mut registry = TypeSchemaRegistry::new();
    register_builtin_schemas(&mut registry);

    let categories = registry
        .get("FrozenTypeCategory")
        .and_then(|schema| schema.get_enum_info())
        .unwrap();
    let actual: Vec<_> = categories
        .variants
        .iter()
        .map(|variant| (variant.name.as_str(), variant.id))
        .collect();
    let expected: Vec<_> = FrozenTypeCategory::ALL
        .into_iter()
        .enumerate()
        .map(|(id, category)| (category.variant_name(), id as u16))
        .collect();
    assert_eq!(actual, expected);
    assert_eq!(categories.variant_id("Unknown"), None);
}

/// ADR-009 B5 (Dec 55-59): the nominal-shape descriptor family round-trips —
/// `FrozenNominal` carries only the sealed `NominalShape` enum; the shape
/// descriptors carry owner + owner-bound member identities (no source-name
/// string field, Dec 57 R1); `NominalShape` / `FieldInitialization` are the
/// sealed catalog enums with the exact catalog variants.
#[test]
fn nominal_shape_descriptor_schemas_round_trip_typed_and_name_free() {
    let mut registry = TypeSchemaRegistry::new();
    register_builtin_schemas(&mut registry);

    // FrozenNominal is the sealed-shape wrapper only.
    let frozen_nominal = registry.get(COMPTIME_FROZEN_NOMINAL_SCHEMA).unwrap();
    let fields: Vec<_> = frozen_nominal
        .fields
        .iter()
        .map(|f| f.name.as_str())
        .collect();
    assert_eq!(fields, ["shape"]);

    // No shape descriptor exposes a source-name / kind string field (Dec 57).
    for schema_name in [
        COMPTIME_STRUCT_DESCRIPTOR_SCHEMA,
        COMPTIME_ENUM_DESCRIPTOR_SCHEMA,
        COMPTIME_NEWTYPE_DESCRIPTOR_SCHEMA,
        COMPTIME_OPAQUE_TYPE_DESCRIPTOR_SCHEMA,
        COMPTIME_FIELD_DESCRIPTOR_SCHEMA,
        COMPTIME_VARIANT_DESCRIPTOR_SCHEMA,
        COMPTIME_ASSOCIATED_CONST_DESCRIPTOR_SCHEMA,
    ] {
        let schema = registry
            .get(schema_name)
            .unwrap_or_else(|| panic!("{schema_name} registered"));
        for field in &schema.fields {
            assert!(
                !matches!(field.name.as_str(), "name" | "kind" | "source" | "type"),
                "{schema_name} must not carry a name/kind/source string field, got {}",
                field.name
            );
        }
    }

    // NominalShape is the exact sealed declaration-shape catalog.
    let nominal_shape = registry
        .get(crate::comptime_reflection::NOMINAL_SHAPE_SCHEMA_NAME)
        .and_then(|schema| schema.get_enum_info())
        .unwrap();
    let actual: Vec<_> = nominal_shape
        .variants
        .iter()
        .map(|variant| (variant.name.as_str(), variant.id, variant.payload_fields))
        .collect();
    assert_eq!(
        actual,
        [
            ("Struct", 0, 1),
            ("Enum", 1, 1),
            ("Newtype", 2, 1),
            ("Opaque", 3, 1)
        ]
    );

    // FieldInitialization is the exact sealed Dec 59 disposition.
    let field_init = registry
        .get(crate::comptime_reflection::FIELD_INITIALIZATION_SCHEMA_NAME)
        .and_then(|schema| schema.get_enum_info())
        .unwrap();
    let init_names: Vec<_> = field_init
        .variants
        .iter()
        .map(|variant| variant.name.as_str())
        .collect();
    assert_eq!(init_names, ["Required", "Defaulted"]);
}
