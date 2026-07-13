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
