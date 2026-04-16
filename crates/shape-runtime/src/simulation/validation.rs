//! TypedObject Validation for High-Performance Simulation
//!
//! This module provides validation functions to check if a Value is suitable
//! for TypedObject optimization, enabling high-performance simulation (>10M ticks/sec).

use crate::type_schema::{SchemaId, TypeSchemaRegistry};
use shape_ast::error::{Result, ShapeError};
use shape_value::{ValueWord, ValueWordExt};

/// Checks if a Value is suitable for TypedObject optimization.
///
/// For high-performance simulation (>10M ticks/sec), the state must be an Object
/// that can be converted to a TypedObject. This requires:
/// 1. The value is a TypedObject
/// 2. A corresponding `TypeSchema` exists (from a `type` declaration)
///
/// Returns `Ok(schema_id)` if the value can be optimized, `Err` otherwise.
pub fn validate_typed_state(
    value: &ValueWord,
    _registry: &TypeSchemaRegistry,
) -> Result<Option<SchemaId>> {
    if value.as_typed_object().is_some() {
        // TypedObject can potentially match a schema
        // We can't know the schema just from the Value - the caller
        // must provide the type name from the declaration context
        Ok(None) // Schema lookup requires type name
    } else if value.is_none() {
        // None is acceptable as initial state (will be initialized)
        Ok(None)
    } else {
        Err(ShapeError::RuntimeError {
            message: format!(
                "Simulation state must be an Object (from type declaration) for optimal performance. \
                     Got: {}. Use 'type MyState {{ ... }}' to declare a typed state.",
                value.type_name()
            ),
            location: None,
        })
    }
}

/// Validates that a Value is a typed Object with a known schema.
///
/// This is the strict validation used by high-performance simulation kernels.
/// It requires the state to have a registered TypeSchema for JIT optimization.
///
/// # Arguments
/// * `value` - The value to validate
/// * `type_name` - Expected type name (from declaration)
/// * `registry` - Type schema registry
///
/// # Returns
/// * `Ok(schema_id)` - The schema ID for TypedObject allocation
/// * `Err` - If the value cannot be optimized
pub fn require_typed_state_with_schema(
    value: &ValueWord,
    type_name: &str,
    registry: &TypeSchemaRegistry,
) -> Result<SchemaId> {
    // First, ensure it's a TypedObject
    if value.as_typed_object().is_none() {
        return Err(ShapeError::RuntimeError {
            message: format!(
                "Simulation state must be a '{}' object. Got: {}",
                type_name,
                value.type_name()
            ),
            location: None,
        });
    }

    // Look up the schema
    match registry.get(type_name) {
        Some(schema) => Ok(schema.id),
        None => Err(ShapeError::RuntimeError {
            message: format!(
                "Type '{}' not found in schema registry. \
                 Declare it with 'type {} {{ ... }}' before using in simulation.",
                type_name, type_name
            ),
            location: None,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_validate_typed_state() {
        let registry = TypeSchemaRegistry::new();

        // TypedObject should be valid
        let obj = crate::type_schema::typed_object_from_pairs(&[]);
        assert!(validate_typed_state(&obj, &registry).is_ok());

        // None should be valid
        assert!(validate_typed_state(&ValueWord::none(), &registry).is_ok());

        // Number should be invalid
        let num = ValueWord::from_f64(42.0);
        assert!(validate_typed_state(&num, &registry).is_err());
    }

    #[test]
    fn test_require_typed_state_with_schema() {
        use crate::type_schema::TypeSchemaBuilder;

        let mut registry = TypeSchemaRegistry::new();

        // Register a type
        let schema = TypeSchemaBuilder::new("TestState")
            .f64_field("value")
            .build();
        registry.register(schema);

        // Valid object with registered type
        let obj = crate::type_schema::typed_object_from_pairs(&[]);
        assert!(require_typed_state_with_schema(&obj, "TestState", &registry).is_ok());

        // Invalid: unregistered type
        assert!(require_typed_state_with_schema(&obj, "UnknownType", &registry).is_err());

        // Invalid: not an object
        let num = ValueWord::from_f64(42.0);
        assert!(require_typed_state_with_schema(&num, "TestState", &registry).is_err());
    }
}
