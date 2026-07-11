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
