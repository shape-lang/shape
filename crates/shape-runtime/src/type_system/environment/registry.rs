//! Type Registry
//!
//! Manages type aliases, interfaces, enum definitions, and record schemas
//! for the type system.

use serde::{Deserialize, Serialize};
use shape_ast::ast::{EnumDef, Expr, InterfaceDef, TraitDef, TypeAnnotation};
use std::collections::HashMap;

const DEFAULT_IMPL_NAME: &str = "__default__";

/// A field in a record schema
#[derive(Debug, Clone)]
pub struct RecordField {
    /// Field name
    pub name: String,
    /// Field type annotation
    pub type_annotation: TypeAnnotation,
}

/// A record schema defining the fields of a record type
#[derive(Debug, Clone, Default)]
pub struct RecordSchema {
    /// Fields in this record
    pub fields: HashMap<String, RecordField>,
}

impl RecordSchema {
    /// Create a new empty record schema
    pub fn new() -> Self {
        Self {
            fields: HashMap::new(),
        }
    }

    /// Add a field to the schema
    pub fn add_field(&mut self, name: &str, type_annotation: TypeAnnotation) {
        self.fields.insert(
            name.to_string(),
            RecordField {
                name: name.to_string(),
                type_annotation,
            },
        );
    }

    /// Look up a field by name
    pub fn get_field(&self, name: &str) -> Option<&RecordField> {
        self.fields.get(name)
    }

    /// Check if a field exists
    pub fn has_field(&self, name: &str) -> bool {
        self.fields.contains_key(name)
    }

    /// Get all field names
    pub fn field_names(&self) -> impl Iterator<Item = &str> {
        self.fields.keys().map(|s| s.as_str())
    }
}

/// A type alias with optional meta parameter overrides
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypeAliasEntry {
    /// The underlying type annotation
    pub type_annotation: TypeAnnotation,
    /// Meta parameter overrides: type Percent4 = Percent { decimals: 4 }
    pub meta_param_overrides: Option<HashMap<String, Expr>>,
}

/// A registered trait implementation: (trait_name, target_type) → method names
#[derive(Debug, Clone)]
pub struct TraitImplEntry {
    /// The trait being implemented
    pub trait_name: String,
    /// The target type name (e.g., "Table", "Vec")
    pub target_type: String,
    /// Optional named implementation selector (`impl Trait for Type as Name`)
    pub impl_name: Option<String>,
    /// Method names provided by this impl
    pub method_names: Vec<String>,
    /// Associated type bindings: name → concrete type
    pub associated_types: HashMap<String, TypeAnnotation>,
}

/// A blanket implementation: `impl<T: Bound> Trait for T { ... }`
///
/// When checking if type X implements Trait, and no concrete impl exists,
/// we check blanket impls: if X satisfies all required_bounds, the blanket
/// impl applies.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct BlanketImplEntry {
    /// The trait being implemented
    pub trait_name: String,
    /// Required trait bounds the type parameter must satisfy
    pub required_bounds: Vec<String>,
    /// Method names provided by this blanket impl
    pub method_names: Vec<String>,
}

/// Registry for type aliases, interfaces, enums, traits, and record schemas
#[derive(Debug, Clone, Default)]
pub struct TypeRegistry {
    /// Type aliases with optional meta parameter overrides
    type_aliases: HashMap<String, TypeAliasEntry>,
    /// Interface definitions
    interfaces: HashMap<String, InterfaceDef>,
    /// Trait definitions
    traits: HashMap<String, TraitDef>,
    /// Trait implementations: key = "TraitName::TargetType"
    trait_impls: HashMap<String, TraitImplEntry>,
    /// Blanket implementations: key = trait name
    blanket_impls: HashMap<String, Vec<BlanketImplEntry>>,
    /// Enum definitions for exhaustiveness checking
    enum_defs: HashMap<String, EnumDef>,
    /// Record schemas for generic row/record types
    record_schemas: HashMap<String, RecordSchema>,
}

impl TypeRegistry {
    fn trait_impl_key(trait_name: &str, target_type: &str, impl_name: Option<&str>) -> String {
        format!(
            "{}::{}::{}",
            trait_name,
            target_type,
            impl_name.unwrap_or(DEFAULT_IMPL_NAME)
        )
    }

    /// Create a new type registry with default schemas
    pub fn new() -> Self {
        let mut registry = Self {
            type_aliases: HashMap::new(),
            interfaces: HashMap::new(),
            traits: HashMap::new(),
            trait_impls: HashMap::new(),
            blanket_impls: HashMap::new(),
            enum_defs: HashMap::new(),
            record_schemas: HashMap::new(),
        };

        // Register default record schemas
        registry.register_default_schemas();
        registry
    }

    /// Register default record schemas (like the "row" type for OHLCV data)
    fn register_default_schemas(&mut self) {
        // Register "row" schema for time-series data row
        // This replaces hardcoded OHLCV field checks
        let mut row_schema = RecordSchema::new();
        row_schema.add_field("open", TypeAnnotation::Basic("number".to_string()));
        row_schema.add_field("high", TypeAnnotation::Basic("number".to_string()));
        row_schema.add_field("low", TypeAnnotation::Basic("number".to_string()));
        row_schema.add_field("close", TypeAnnotation::Basic("number".to_string()));
        row_schema.add_field("volume", TypeAnnotation::Basic("number".to_string()));
        row_schema.add_field("time", TypeAnnotation::Basic("timestamp".to_string()));
        self.record_schemas.insert("row".to_string(), row_schema);
    }

    /// Define a type alias with optional meta parameter overrides
    pub fn define_type_alias(
        &mut self,
        name: &str,
        ty: &TypeAnnotation,
        meta_param_overrides: Option<HashMap<String, Expr>>,
    ) {
        self.type_aliases.insert(
            name.to_string(),
            TypeAliasEntry {
                type_annotation: ty.clone(),
                meta_param_overrides,
            },
        );
    }

    /// Look up a type alias
    pub fn lookup_type_alias(&self, name: &str) -> Option<&TypeAliasEntry> {
        self.type_aliases.get(name)
    }

    /// Get the meta parameter overrides for a type alias
    pub fn get_type_alias_meta_overrides(&self, name: &str) -> Option<&HashMap<String, Expr>> {
        self.type_aliases
            .get(name)
            .and_then(|entry| entry.meta_param_overrides.as_ref())
    }

    /// Define an interface
    pub fn define_interface(&mut self, interface: &InterfaceDef) {
        self.interfaces
            .insert(interface.name.clone(), interface.clone());
    }

    /// Look up an interface
    pub fn lookup_interface(&self, name: &str) -> Option<&InterfaceDef> {
        self.interfaces.get(name)
    }

    /// Define a trait
    pub fn define_trait(&mut self, trait_def: &TraitDef) {
        self.traits
            .insert(trait_def.name.clone(), trait_def.clone());
    }

    /// Look up a trait
    pub fn lookup_trait(&self, name: &str) -> Option<&TraitDef> {
        self.traits.get(name)
    }

    /// Register a trait implementation
    ///
    /// Validates that all required trait methods are present (name + arity check).
    /// Returns an error message if validation fails.
    pub fn register_trait_impl(
        &mut self,
        trait_name: &str,
        target_type: &str,
        method_names: Vec<String>,
    ) -> Result<(), String> {
        self.register_trait_impl_with_assoc_types_named(
            trait_name,
            target_type,
            None,
            method_names,
            HashMap::new(),
        )
    }

    /// Register a named trait implementation:
    /// `impl Trait for Type as ImplName { ... }`
    pub fn register_trait_impl_named(
        &mut self,
        trait_name: &str,
        target_type: &str,
        impl_name: &str,
        method_names: Vec<String>,
    ) -> Result<(), String> {
        self.register_trait_impl_with_assoc_types_named(
            trait_name,
            target_type,
            Some(impl_name),
            method_names,
            HashMap::new(),
        )
    }

    /// Register a trait implementation with associated type bindings.
    ///
    /// Validates required methods, associated types, and coherence,
    /// and that all required associated types are bound.
    pub fn register_trait_impl_with_assoc_types(
        &mut self,
        trait_name: &str,
        target_type: &str,
        method_names: Vec<String>,
        associated_types: HashMap<String, TypeAnnotation>,
    ) -> Result<(), String> {
        self.register_trait_impl_with_assoc_types_named(
            trait_name,
            target_type,
            None,
            method_names,
            associated_types,
        )
    }

    /// Register a trait implementation with optional impl name and associated types.
    pub fn register_trait_impl_with_assoc_types_named(
        &mut self,
        trait_name: &str,
        target_type: &str,
        impl_name: Option<&str>,
        method_names: Vec<String>,
        associated_types: HashMap<String, TypeAnnotation>,
    ) -> Result<(), String> {
        // Validate against trait definition if it exists
        // Clone required data out to avoid holding a borrow on self.traits
        if let Some(trait_def) = self.traits.get(trait_name) {
            use shape_ast::ast::{InterfaceMember, TraitMember};
            let required_methods: Vec<String> = trait_def
                .members
                .iter()
                .filter_map(|m| match m {
                    TraitMember::Required(InterfaceMember::Method { name, .. }) => {
                        Some(name.clone())
                    }
                    _ => None,
                })
                .collect();

            // Collect required associated type names from the trait definition
            let required_assoc_types: Vec<String> = trait_def
                .members
                .iter()
                .filter_map(|m| match m {
                    TraitMember::AssociatedType { name, .. } => Some(name.clone()),
                    _ => None,
                })
                .collect();

            // Check all required methods are present
            for required in &required_methods {
                if !method_names.iter().any(|m| m == required) {
                    return Err(format!(
                        "impl {} for {} is missing required method '{}'",
                        trait_name, target_type, required
                    ));
                }
            }

            // Check all required associated types are bound
            for required_at in &required_assoc_types {
                if !associated_types.contains_key(required_at) {
                    return Err(format!(
                        "impl {} for {} is missing associated type '{}'",
                        trait_name, target_type, required_at
                    ));
                }
            }
        }

        // Coherence check:
        // - default impl: only one default per (trait, type)
        // - named impl: only one impl per (trait, type, impl_name)
        let key = Self::trait_impl_key(trait_name, target_type, impl_name);
        if let Some(existing) = self.trait_impls.get(&key) {
            // Idempotent re-registration is allowed when the impl shape matches exactly.
            // This keeps built-in pre-registered impl metadata compatible with stdlib
            // modules that declare the same impls explicitly.
            let mut existing_methods = existing.method_names.clone();
            existing_methods.sort();
            let mut incoming_methods = method_names.clone();
            incoming_methods.sort();
            if existing_methods == incoming_methods && existing.associated_types == associated_types
            {
                return Ok(());
            }
            return if let Some(name) = impl_name {
                Err(format!(
                    "conflicting implementations: '{}' is already implemented for '{}' as '{}'",
                    trait_name, target_type, name
                ))
            } else {
                Err(format!(
                    "conflicting implementations: '{}' is already implemented for '{}'",
                    trait_name, target_type
                ))
            };
        }

        self.trait_impls.insert(
            key,
            TraitImplEntry {
                trait_name: trait_name.to_string(),
                target_type: target_type.to_string(),
                impl_name: impl_name.map(|s| s.to_string()),
                method_names,
                associated_types,
            },
        );
        Ok(())
    }

    /// Register a blanket implementation: `impl<T: Bound> Trait for T`
    pub fn register_blanket_impl(
        &mut self,
        trait_name: &str,
        required_bounds: Vec<String>,
        method_names: Vec<String>,
    ) {
        self.blanket_impls
            .entry(trait_name.to_string())
            .or_insert_with(Vec::new)
            .push(BlanketImplEntry {
                trait_name: trait_name.to_string(),
                required_bounds,
                method_names,
            });
    }

    /// Check if a type implements a trait (direct impl or blanket impl)
    pub fn type_implements_trait(&self, type_name: &str, trait_name: &str) -> bool {
        let mut visited = std::collections::HashSet::new();
        self.type_implements_trait_inner(type_name, trait_name, &mut visited)
    }

    /// Inner recursive check with cycle detection to prevent infinite recursion
    /// from circular blanket impls (e.g., `impl<T: TraitX> TraitX for T`).
    fn type_implements_trait_inner(
        &self,
        type_name: &str,
        trait_name: &str,
        visited: &mut std::collections::HashSet<String>,
    ) -> bool {
        // Check direct impl first (default OR any named impl)
        if self
            .trait_impls
            .values()
            .any(|entry| entry.trait_name == trait_name && entry.target_type == type_name)
        {
            return true;
        }

        // Cycle detection: if we're already checking this (type, trait) pair, bail out
        let visit_key = format!("{}::{}", type_name, trait_name);
        if !visited.insert(visit_key) {
            return false;
        }

        // Check blanket impls: if type satisfies all required bounds, it matches
        if let Some(blankets) = self.blanket_impls.get(trait_name) {
            for blanket in blankets {
                let all_bounds_satisfied = blanket
                    .required_bounds
                    .iter()
                    .all(|bound| self.type_implements_trait_inner(type_name, bound, visited));
                if all_bounds_satisfied {
                    return true;
                }
            }
        }

        false
    }

    /// Look up a trait implementation
    pub fn lookup_trait_impl(&self, trait_name: &str, type_name: &str) -> Option<&TraitImplEntry> {
        let key = Self::trait_impl_key(trait_name, type_name, None);
        self.trait_impls.get(&key)
    }

    /// Look up a named trait implementation.
    pub fn lookup_trait_impl_named(
        &self,
        trait_name: &str,
        type_name: &str,
        impl_name: &str,
    ) -> Option<&TraitImplEntry> {
        let key = Self::trait_impl_key(trait_name, type_name, Some(impl_name));
        self.trait_impls.get(&key)
    }

    /// Resolve an associated type: given a trait, an implementing type, and
    /// the associated type name, return the concrete type annotation.
    pub fn resolve_associated_type(
        &self,
        trait_name: &str,
        type_name: &str,
        assoc_type_name: &str,
    ) -> Option<&TypeAnnotation> {
        self.lookup_trait_impl(trait_name, type_name)
            .and_then(|entry| entry.associated_types.get(assoc_type_name))
    }

    /// Resolve an associated type from a named impl.
    pub fn resolve_associated_type_named(
        &self,
        trait_name: &str,
        type_name: &str,
        impl_name: &str,
        assoc_type_name: &str,
    ) -> Option<&TypeAnnotation> {
        self.lookup_trait_impl_named(trait_name, type_name, impl_name)
            .and_then(|entry| entry.associated_types.get(assoc_type_name))
    }

    /// Get all trait implementation keys ("TraitName::TypeName") as a set
    pub fn trait_impl_keys(&self) -> std::collections::HashSet<String> {
        let mut keys = std::collections::HashSet::new();
        for entry in self.trait_impls.values() {
            // Legacy key (still used by comptime implements() callers/tests)
            keys.insert(format!("{}::{}", entry.trait_name, entry.target_type));
            // Canonical key with impl selector (default or named)
            keys.insert(Self::trait_impl_key(
                &entry.trait_name,
                &entry.target_type,
                entry.impl_name.as_deref(),
            ));
        }
        keys
    }

    /// Register an enum definition for exhaustiveness checking
    pub fn register_enum(&mut self, enum_def: &EnumDef) {
        self.enum_defs
            .insert(enum_def.name.clone(), enum_def.clone());
    }

    /// Look up an enum definition by name
    pub fn get_enum(&self, name: &str) -> Option<&EnumDef> {
        self.enum_defs.get(name)
    }

    /// Register a record schema
    pub fn register_record_schema(&mut self, name: &str, schema: RecordSchema) {
        self.record_schemas.insert(name.to_string(), schema);
    }

    /// Look up a record schema
    pub fn lookup_record_schema(&self, name: &str) -> Option<&RecordSchema> {
        self.record_schemas.get(name)
    }

    /// Get field type from a record schema
    pub fn get_record_field_type(
        &self,
        schema_name: &str,
        field_name: &str,
    ) -> Option<&TypeAnnotation> {
        self.record_schemas
            .get(schema_name)
            .and_then(|schema| schema.get_field(field_name))
            .map(|field| &field.type_annotation)
    }

    /// Check if a record schema has a field
    pub fn record_has_field(&self, schema_name: &str, field_name: &str) -> bool {
        self.record_schemas
            .get(schema_name)
            .map(|schema| schema.has_field(field_name))
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_ast::ast::{FunctionParam, InterfaceMember, TraitDef, TraitMember, TypeAnnotation};

    /// Helper: build a simple trait with one required method
    fn make_trait(name: &str, methods: Vec<&str>) -> TraitDef {
        TraitDef {
            name: name.to_string(),
            doc_comment: None,
            type_params: None,
            members: methods
                .into_iter()
                .map(|m| {
                    TraitMember::Required(InterfaceMember::Method {
                        name: m.to_string(),
                        optional: false,
                        params: vec![FunctionParam {
                            name: Some("self".to_string()),
                            type_annotation: TypeAnnotation::Basic("Self".to_string()),
                            optional: false,
                        }],
                        return_type: TypeAnnotation::Basic("string".to_string()),
                        is_async: false,
                        span: Span::DUMMY,
                        doc_comment: None,
                    })
                })
                .collect(),
            annotations: vec![],
        }
    }

    // ---------------------------------------------------------------
    // Coherence checking tests
    // ---------------------------------------------------------------

    #[test]
    fn duplicate_impl_is_rejected() {
        let mut reg = TypeRegistry::new();
        reg.define_trait(&make_trait("Display", vec!["to_string"]));

        assert!(
            reg.register_trait_impl("Display", "MyType", vec!["to_string".into()])
                .is_ok()
        );

        // Idempotent re-registration with identical methods should succeed
        assert!(
            reg.register_trait_impl("Display", "MyType", vec!["to_string".into()])
                .is_ok(),
            "identical re-registration should be idempotent"
        );

        // Second impl with *different* methods for the same (trait, type) pair should fail
        let result = reg.register_trait_impl(
            "Display",
            "MyType",
            vec!["to_string".into(), "extra_method".into()],
        );
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(
            msg.contains("conflicting implementations"),
            "Expected coherence error, got: {}",
            msg
        );
        assert!(msg.contains("Display"), "Expected trait name in error");
        assert!(msg.contains("MyType"), "Expected type name in error");
    }

    #[test]
    fn named_impl_can_coexist_with_default_impl() {
        let mut reg = TypeRegistry::new();
        reg.define_trait(&make_trait("Display", vec!["to_string"]));

        assert!(
            reg.register_trait_impl("Display", "User", vec!["to_string".into()])
                .is_ok()
        );
        assert!(
            reg.register_trait_impl_named(
                "Display",
                "User",
                "JsonDisplay",
                vec!["to_string".into()]
            )
            .is_ok()
        );

        assert!(reg.lookup_trait_impl("Display", "User").is_some());
        assert!(
            reg.lookup_trait_impl_named("Display", "User", "JsonDisplay")
                .is_some()
        );
    }

    #[test]
    fn duplicate_named_impl_registration_is_idempotent() {
        let mut reg = TypeRegistry::new();
        reg.define_trait(&make_trait("Display", vec!["to_string"]));

        assert!(
            reg.register_trait_impl_named(
                "Display",
                "User",
                "JsonDisplay",
                vec!["to_string".into()]
            )
            .is_ok()
        );

        assert!(
            reg.register_trait_impl_named(
                "Display",
                "User",
                "JsonDisplay",
                vec!["to_string".into()]
            )
            .is_ok(),
            "same named impl metadata should be idempotent"
        );
    }

    #[test]
    fn conflicting_named_impl_is_rejected() {
        let mut reg = TypeRegistry::new();
        reg.define_trait(&make_trait("Display", vec!["to_string"]));

        assert!(
            reg.register_trait_impl_named(
                "Display",
                "User",
                "JsonDisplay",
                vec!["to_string".into()]
            )
            .is_ok()
        );

        let mut associated = HashMap::new();
        associated.insert(
            "Dummy".to_string(),
            TypeAnnotation::Basic("int".to_string()),
        );
        let err = reg
            .register_trait_impl_with_assoc_types_named(
                "Display",
                "User",
                Some("JsonDisplay"),
                vec!["to_string".into()],
                associated,
            )
            .expect_err("conflicting duplicate named impl should fail");
        assert!(err.contains("conflicting implementations"));
        assert!(err.contains("JsonDisplay"));
    }

    #[test]
    fn type_implements_trait_with_named_impl_only() {
        let mut reg = TypeRegistry::new();
        reg.define_trait(&make_trait("Display", vec!["to_string"]));

        assert!(
            reg.register_trait_impl_named("Display", "Widget", "Pretty", vec!["to_string".into()])
                .is_ok()
        );

        assert!(reg.type_implements_trait("Widget", "Display"));
        assert!(!reg.type_implements_trait("OtherType", "Display"));
    }

    #[test]
    fn same_trait_different_types_is_allowed() {
        let mut reg = TypeRegistry::new();
        reg.define_trait(&make_trait("Display", vec!["to_string"]));

        assert!(
            reg.register_trait_impl("Display", "TypeA", vec!["to_string".into()])
                .is_ok()
        );
        assert!(
            reg.register_trait_impl("Display", "TypeB", vec!["to_string".into()])
                .is_ok()
        );
    }

    #[test]
    fn different_traits_same_type_is_allowed() {
        let mut reg = TypeRegistry::new();
        reg.define_trait(&make_trait("TraitX", vec!["method_x"]));
        reg.define_trait(&make_trait("TraitY", vec!["method_y"]));

        assert!(
            reg.register_trait_impl("TraitX", "MyType", vec!["method_x".into()])
                .is_ok()
        );
        assert!(
            reg.register_trait_impl("TraitY", "MyType", vec!["method_y".into()])
                .is_ok()
        );
    }

    // ---------------------------------------------------------------
    // Associated types tests
    // ---------------------------------------------------------------

    /// Helper: build a trait with one required method AND one associated type
    fn make_trait_with_assoc_type(
        name: &str,
        methods: Vec<&str>,
        assoc_types: Vec<&str>,
    ) -> TraitDef {
        let mut members: Vec<TraitMember> = methods
            .into_iter()
            .map(|m| {
                TraitMember::Required(InterfaceMember::Method {
                    name: m.to_string(),
                    optional: false,
                    params: vec![FunctionParam {
                        name: Some("self".to_string()),
                        type_annotation: TypeAnnotation::Basic("Self".to_string()),
                        optional: false,
                    }],
                    return_type: TypeAnnotation::Basic("string".to_string()),
                    is_async: false,
                    span: Span::DUMMY,
                    doc_comment: None,
                })
            })
            .collect();

        for at in assoc_types {
            members.push(TraitMember::AssociatedType {
                name: at.to_string(),
                bounds: vec![],
                span: Span::DUMMY,
                doc_comment: None,
            });
        }

        TraitDef {
            name: name.to_string(),
            doc_comment: None,
            type_params: None,
            members,
            annotations: vec![],
        }
    }

    #[test]
    fn associated_type_impl_succeeds() {
        let mut reg = TypeRegistry::new();
        reg.define_trait(&make_trait_with_assoc_type(
            "Iterator",
            vec!["next"],
            vec!["Item"],
        ));

        let mut assoc = HashMap::new();
        assoc.insert(
            "Item".to_string(),
            TypeAnnotation::Basic("number".to_string()),
        );

        assert!(
            reg.register_trait_impl_with_assoc_types(
                "Iterator",
                "Range",
                vec!["next".into()],
                assoc,
            )
            .is_ok()
        );

        // Verify the associated type can be resolved
        let resolved = reg.resolve_associated_type("Iterator", "Range", "Item");
        assert!(resolved.is_some());
        assert!(matches!(resolved.unwrap(), TypeAnnotation::Basic(s) if s == "number"));
    }

    #[test]
    fn associated_type_impl_fails_when_missing() {
        let mut reg = TypeRegistry::new();
        reg.define_trait(&make_trait_with_assoc_type(
            "Iterator",
            vec!["next"],
            vec!["Item"],
        ));

        // Register without providing the associated type binding
        let result = reg.register_trait_impl("Iterator", "Range", vec!["next".into()]);
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(
            msg.contains("missing associated type 'Item'"),
            "Expected associated type error, got: {}",
            msg
        );
    }

    #[test]
    fn associated_type_resolution_returns_none_for_unknown() {
        let reg = TypeRegistry::new();
        assert!(
            reg.resolve_associated_type("Iterator", "Range", "Item")
                .is_none()
        );
    }

    // ---------------------------------------------------------------
    // Blanket implementation tests
    // ---------------------------------------------------------------

    #[test]
    fn blanket_impl_applies_when_bounds_satisfied() {
        let mut reg = TypeRegistry::new();
        reg.define_trait(&make_trait("Serializable", vec!["to_bytes"]));
        reg.define_trait(&make_trait("Printable", vec!["to_string"]));

        // Register blanket: impl<T: Serializable> Printable for T
        reg.register_blanket_impl(
            "Printable",
            vec!["Serializable".to_string()],
            vec!["to_string".to_string()],
        );

        // MyType doesn't implement Serializable yet → Printable should NOT apply
        assert!(!reg.type_implements_trait("MyType", "Printable"));

        // Now implement Serializable for MyType
        assert!(
            reg.register_trait_impl("Serializable", "MyType", vec!["to_bytes".into()])
                .is_ok()
        );

        // Now Printable should apply via blanket impl
        assert!(reg.type_implements_trait("MyType", "Printable"));
    }

    #[test]
    fn blanket_impl_does_not_apply_when_bounds_not_satisfied() {
        let mut reg = TypeRegistry::new();
        reg.define_trait(&make_trait("Serializable", vec!["to_bytes"]));
        reg.define_trait(&make_trait("Printable", vec!["to_string"]));

        // Register blanket: impl<T: Serializable> Printable for T
        reg.register_blanket_impl(
            "Printable",
            vec!["Serializable".to_string()],
            vec!["to_string".to_string()],
        );

        // OtherType has no impls → blanket should not apply
        assert!(!reg.type_implements_trait("OtherType", "Printable"));
    }

    #[test]
    fn blanket_impl_with_multiple_bounds() {
        let mut reg = TypeRegistry::new();
        reg.define_trait(&make_trait("TraitA", vec!["method_a"]));
        reg.define_trait(&make_trait("TraitB", vec!["method_b"]));
        reg.define_trait(&make_trait("Combined", vec!["combined"]));

        // Register blanket: impl<T: TraitA + TraitB> Combined for T
        reg.register_blanket_impl(
            "Combined",
            vec!["TraitA".to_string(), "TraitB".to_string()],
            vec!["combined".to_string()],
        );

        // Only implement TraitA → Combined should NOT apply
        assert!(
            reg.register_trait_impl("TraitA", "Widget", vec!["method_a".into()])
                .is_ok()
        );
        assert!(!reg.type_implements_trait("Widget", "Combined"));

        // Now implement TraitB too → Combined should apply
        assert!(
            reg.register_trait_impl("TraitB", "Widget", vec!["method_b".into()])
                .is_ok()
        );
        assert!(reg.type_implements_trait("Widget", "Combined"));
    }

    #[test]
    fn direct_impl_takes_precedence_over_blanket() {
        let mut reg = TypeRegistry::new();
        reg.define_trait(&make_trait("Serializable", vec!["to_bytes"]));
        reg.define_trait(&make_trait("Printable", vec!["to_string"]));

        // Register blanket: impl<T: Serializable> Printable for T
        reg.register_blanket_impl(
            "Printable",
            vec!["Serializable".to_string()],
            vec!["to_string".to_string()],
        );

        // Register direct impl of Printable for SpecialType
        assert!(
            reg.register_trait_impl("Printable", "SpecialType", vec!["to_string".into()])
                .is_ok()
        );

        // Should be satisfied via direct impl (even without Serializable)
        assert!(reg.type_implements_trait("SpecialType", "Printable"));
    }

    #[test]
    fn blanket_impl_chains_through_other_blankets() {
        let mut reg = TypeRegistry::new();
        reg.define_trait(&make_trait("Base", vec!["base_method"]));
        reg.define_trait(&make_trait("Mid", vec!["mid_method"]));
        reg.define_trait(&make_trait("Top", vec!["top_method"]));

        // impl<T: Base> Mid for T
        reg.register_blanket_impl(
            "Mid",
            vec!["Base".to_string()],
            vec!["mid_method".to_string()],
        );

        // impl<T: Mid> Top for T
        reg.register_blanket_impl(
            "Top",
            vec!["Mid".to_string()],
            vec!["top_method".to_string()],
        );

        // Implement Base for Foo
        assert!(
            reg.register_trait_impl("Base", "Foo", vec!["base_method".into()])
                .is_ok()
        );

        // Mid should apply via first blanket
        assert!(reg.type_implements_trait("Foo", "Mid"));

        // Top should apply via second blanket (chained through Mid)
        assert!(reg.type_implements_trait("Foo", "Top"));
    }

    #[test]
    fn blanket_impl_no_infinite_recursion_on_missing_bounds() {
        let mut reg = TypeRegistry::new();
        reg.define_trait(&make_trait("TraitX", vec!["method_x"]));

        // impl<T: TraitX> TraitX for T — circular, but should not infinite loop
        // because the direct check happens first and returns false for unregistered types
        reg.register_blanket_impl(
            "TraitX",
            vec!["TraitX".to_string()],
            vec!["method_x".to_string()],
        );

        // Should return false without stack overflow
        assert!(!reg.type_implements_trait("SomeType", "TraitX"));
    }

    #[test]
    fn multiple_blanket_impls_for_same_trait() {
        let mut reg = TypeRegistry::new();
        reg.define_trait(&make_trait("Hashable", vec!["hash"]));
        reg.define_trait(&make_trait("Equatable", vec!["eq"]));
        reg.define_trait(&make_trait("Comparable", vec!["compare"]));

        // Two blanket paths to Comparable:
        // impl<T: Hashable> Comparable for T
        reg.register_blanket_impl(
            "Comparable",
            vec!["Hashable".to_string()],
            vec!["compare".to_string()],
        );
        // impl<T: Equatable> Comparable for T
        reg.register_blanket_impl(
            "Comparable",
            vec!["Equatable".to_string()],
            vec!["compare".to_string()],
        );

        // TypeH implements Hashable → should get Comparable
        assert!(
            reg.register_trait_impl("Hashable", "TypeH", vec!["hash".into()])
                .is_ok()
        );
        assert!(reg.type_implements_trait("TypeH", "Comparable"));

        // TypeE implements Equatable → should also get Comparable
        assert!(
            reg.register_trait_impl("Equatable", "TypeE", vec!["eq".into()])
                .is_ok()
        );
        assert!(reg.type_implements_trait("TypeE", "Comparable"));
    }

    #[test]
    fn multiple_associated_types() {
        let mut reg = TypeRegistry::new();
        reg.define_trait(&make_trait_with_assoc_type(
            "Collection",
            vec!["len"],
            vec!["Item", "Key"],
        ));

        let mut assoc = HashMap::new();
        assoc.insert(
            "Item".to_string(),
            TypeAnnotation::Basic("string".to_string()),
        );
        assoc.insert("Key".to_string(), TypeAnnotation::Basic("int".to_string()));

        assert!(
            reg.register_trait_impl_with_assoc_types(
                "Collection",
                "HashMap",
                vec!["len".into()],
                assoc,
            )
            .is_ok()
        );

        assert!(matches!(
            reg.resolve_associated_type("Collection", "HashMap", "Item"),
            Some(TypeAnnotation::Basic(s)) if s == "string"
        ));
        assert!(matches!(
            reg.resolve_associated_type("Collection", "HashMap", "Key"),
            Some(TypeAnnotation::Basic(s)) if s == "int"
        ));
    }
}
