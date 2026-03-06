//! Intersection type support for type schemas
//!
//! This module provides functionality for merging multiple type schemas
//! into intersection types (A + B), with field collision detection.

use super::SchemaError;
use super::field_types::{FieldDef, FieldType};
use super::schema::TypeSchema;
use std::collections::HashMap;

impl TypeSchema {
    /// Create an intersection type schema by merging multiple schemas.
    /// Returns an error if any field names collide.
    pub fn from_intersection(
        name: impl Into<String>,
        schemas: &[&TypeSchema],
    ) -> Result<Self, SchemaError> {
        let name = name.into();

        // Check for field collisions
        let mut seen_fields: HashMap<&str, &str> = HashMap::new();
        for schema in schemas {
            for field in &schema.fields {
                if let Some(existing_type) = seen_fields.get(field.name.as_str()) {
                    return Err(SchemaError::FieldCollision {
                        field_name: field.name.clone(),
                        type1: existing_type.to_string(),
                        type2: schema.name.clone(),
                    });
                }
                seen_fields.insert(&field.name, &schema.name);
            }
        }

        // Collect all fields with their source types
        let mut all_fields: Vec<(String, FieldType)> = Vec::new();
        let mut field_sources: HashMap<String, String> = HashMap::new();
        let mut component_types: Vec<String> = Vec::new();

        for schema in schemas {
            component_types.push(schema.name.clone());
            for field in &schema.fields {
                all_fields.push((field.name.clone(), field.field_type.clone()));
                field_sources.insert(field.name.clone(), schema.name.clone());
            }
        }

        // Build the merged schema
        let id = super::next_schema_id();
        let mut fields = Vec::with_capacity(all_fields.len());
        let mut field_map = HashMap::with_capacity(all_fields.len());
        let mut offset = 0;

        for (index, (field_name, field_type)) in all_fields.into_iter().enumerate() {
            let alignment = field_type.alignment();
            offset = (offset + alignment - 1) & !(alignment - 1);

            let field = FieldDef::new(&field_name, field_type.clone(), offset, index as u16);
            field_map.insert(field_name, index);
            offset += field_type.size();
            fields.push(field);
        }

        let data_size = (offset + 7) & !7;

        Ok(Self {
            id,
            name,
            fields,
            field_map,
            data_size,
            component_types: Some(component_types),
            field_sources,
            enum_info: None,
            content_hash: None,
        })
    }

    /// Check if this schema is an intersection type
    pub fn is_intersection(&self) -> bool {
        self.component_types.is_some()
    }

    /// Get the component types if this is an intersection
    pub fn get_component_types(&self) -> Option<&[String]> {
        self.component_types.as_deref()
    }

    /// Get the source type for a field (for decomposition)
    pub fn field_source(&self, field_name: &str) -> Option<&str> {
        self.field_sources.get(field_name).map(|s| s.as_str())
    }

    /// Get fields belonging to a specific component type (for decomposition)
    pub fn fields_for_component(&self, component_name: &str) -> Vec<&FieldDef> {
        self.fields
            .iter()
            .filter(|f| self.field_sources.get(&f.name).map(|s| s.as_str()) == Some(component_name))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_intersection_merge_success() {
        // type A = { x: number }
        let schema_a = TypeSchema::new("A", vec![("x".to_string(), FieldType::F64)]);

        // type B = { y: string }
        let schema_b = TypeSchema::new("B", vec![("y".to_string(), FieldType::String)]);

        // type AB = A + B
        let merged = TypeSchema::from_intersection("AB", &[&schema_a, &schema_b])
            .expect("Should merge without collision");

        assert_eq!(merged.name, "AB");
        assert_eq!(merged.field_count(), 2);
        assert!(merged.has_field("x"));
        assert!(merged.has_field("y"));
        assert!(merged.is_intersection());

        // Check component tracking
        let components = merged.get_component_types().unwrap();
        assert_eq!(components, &["A", "B"]);

        // Check field sources for decomposition
        assert_eq!(merged.field_source("x"), Some("A"));
        assert_eq!(merged.field_source("y"), Some("B"));
    }

    #[test]
    fn test_intersection_field_collision() {
        // type A = { x: number }
        let schema_a = TypeSchema::new("A", vec![("x".to_string(), FieldType::F64)]);

        // type B = { x: string }  // Same field name, different type
        let schema_b = TypeSchema::new("B", vec![("x".to_string(), FieldType::String)]);

        // type AB = A + B should fail with collision error
        let result = TypeSchema::from_intersection("AB", &[&schema_a, &schema_b]);
        assert!(result.is_err());

        match result {
            Err(SchemaError::FieldCollision {
                field_name,
                type1,
                type2,
            }) => {
                assert_eq!(field_name, "x");
                assert_eq!(type1, "A");
                assert_eq!(type2, "B");
            }
            _ => panic!("Expected FieldCollision error"),
        }
    }

    #[test]
    fn test_intersection_three_types() {
        // type A = { a: number }
        let schema_a = TypeSchema::new("A", vec![("a".to_string(), FieldType::F64)]);
        // type B = { b: string }
        let schema_b = TypeSchema::new("B", vec![("b".to_string(), FieldType::String)]);
        // type C = { c: bool }
        let schema_c = TypeSchema::new("C", vec![("c".to_string(), FieldType::Bool)]);

        // type ABC = A + B + C
        let merged = TypeSchema::from_intersection("ABC", &[&schema_a, &schema_b, &schema_c])
            .expect("Should merge three types");

        assert_eq!(merged.field_count(), 3);
        assert!(merged.has_field("a"));
        assert!(merged.has_field("b"));
        assert!(merged.has_field("c"));

        let components = merged.get_component_types().unwrap();
        assert_eq!(components, &["A", "B", "C"]);
    }

    #[test]
    fn test_intersection_fields_for_component() {
        // type A = { x: number, y: number }
        let schema_a = TypeSchema::new(
            "A",
            vec![
                ("x".to_string(), FieldType::F64),
                ("y".to_string(), FieldType::F64),
            ],
        );

        // type B = { z: string }
        let schema_b = TypeSchema::new("B", vec![("z".to_string(), FieldType::String)]);

        let merged = TypeSchema::from_intersection("AB", &[&schema_a, &schema_b]).unwrap();

        // Test decomposition - get fields belonging to each component
        let a_fields = merged.fields_for_component("A");
        assert_eq!(a_fields.len(), 2);
        assert!(a_fields.iter().any(|f| f.name == "x"));
        assert!(a_fields.iter().any(|f| f.name == "y"));

        let b_fields = merged.fields_for_component("B");
        assert_eq!(b_fields.len(), 1);
        assert!(b_fields.iter().any(|f| f.name == "z"));
    }

    #[test]
    fn test_intersection_data_size() {
        let schema_a = TypeSchema::new(
            "A",
            vec![
                ("a1".to_string(), FieldType::F64),
                ("a2".to_string(), FieldType::I64),
            ],
        );

        let schema_b = TypeSchema::new("B", vec![("b1".to_string(), FieldType::Bool)]);

        let merged = TypeSchema::from_intersection("AB", &[&schema_a, &schema_b]).unwrap();

        // 3 fields * 8 bytes each = 24 bytes
        assert_eq!(merged.data_size, 24);

        // Check offsets are computed correctly
        assert_eq!(merged.field_offset("a1"), Some(0));
        assert_eq!(merged.field_offset("a2"), Some(8));
        assert_eq!(merged.field_offset("b1"), Some(16));
    }
}
