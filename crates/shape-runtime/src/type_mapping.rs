//! Type mapping system for validating DataFrame structure
//!
//! This module provides type mappings that define the expected structure
//! of DataFrames loaded for specific types. Type mappings enable:
//! - Compile-time validation of data structure
//! - Custom column name mappings
//! - Extensibility for user-defined types
//!
//! NOTE: No built-in types are registered. Domain-specific types should be
//! defined in stdlib and registered at application startup.

use crate::data::DataFrame;
use shape_ast::error::{Result, ShapeError};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// Maps DataFrame columns to a user-defined type
///
/// Specifies which columns are required and how they map to type fields.
///
/// # Example
///
/// ```ignore
/// // For a SensorReading type:
/// let mapping = TypeMapping::new("SensorReading".to_string())
///     .add_field("temperature", "temp_c")
///     .add_field("humidity", "humidity_pct")
///     .add_required("temp_c")
///     .add_required("humidity_pct");
/// ```
#[derive(Debug, Clone)]
pub struct TypeMapping {
    /// Type name (e.g., "SensorReading", "LogEntry", "DataPoint")
    pub type_name: String,

    /// Field name → DataFrame column name mapping
    /// Allows renaming: { "temp": "temperature_celsius" }
    pub field_to_column: HashMap<String, String>,

    /// List of required DataFrame columns
    /// These must exist in the DataFrame for validation to pass
    pub required_columns: Vec<String>,
}

impl TypeMapping {
    /// Create a new type mapping
    pub fn new(type_name: String) -> Self {
        Self {
            type_name,
            field_to_column: HashMap::new(),
            required_columns: Vec::new(),
        }
    }

    // NOTE: The ohlcv() factory method has been removed.
    // Finance-specific types like "Candle" should be defined in shape-stdlib,
    // not in the core library. Types are registered at CLI startup from stdlib.

    /// Add a field mapping
    pub fn add_field(mut self, field: &str, column: &str) -> Self {
        self.field_to_column
            .insert(field.to_string(), column.to_string());
        self
    }

    /// Add a required column
    pub fn add_required(mut self, column: &str) -> Self {
        self.required_columns.push(column.to_string());
        self
    }

    /// Apply custom column mapping
    ///
    /// Extends the existing mapping with custom field→column pairs.
    pub fn with_mapping(mut self, custom: HashMap<String, String>) -> Self {
        self.field_to_column.extend(custom);
        self
    }

    /// Validate that DataFrame has all required columns
    ///
    /// # Arguments
    ///
    /// * `df` - DataFrame to validate
    ///
    /// # Returns
    ///
    /// Ok if all required columns exist, error otherwise
    ///
    /// # Example
    ///
    /// ```ignore
    /// let mapping = TypeMapping::ohlcv();
    /// mapping.validate(&dataframe)?;  // Checks for open, high, low, close, volume
    /// ```
    pub fn validate(&self, df: &DataFrame) -> Result<()> {
        let mut missing = Vec::new();

        for col in &self.required_columns {
            if !df.has_column(col) {
                missing.push(col.clone());
            }
        }

        if !missing.is_empty() {
            return Err(ShapeError::RuntimeError {
                message: format!(
                    "DataFrame missing required columns for type '{}': {}",
                    self.type_name,
                    missing.join(", ")
                ),
                location: None,
            });
        }

        Ok(())
    }

    /// Get column name for a type field
    ///
    /// # Arguments
    ///
    /// * `field` - Type field name
    ///
    /// # Returns
    ///
    /// DataFrame column name if mapping exists
    pub fn get_column(&self, field: &str) -> Option<&str> {
        self.field_to_column.get(field).map(|s| s.as_str())
    }
}

/// Registry of type mappings
///
/// Maintains mappings for all known types (built-in and user-defined).
/// Thread-safe for concurrent access.
#[derive(Clone)]
pub struct TypeMappingRegistry {
    /// Map of type name → type mapping
    mappings: Arc<RwLock<HashMap<String, TypeMapping>>>,
}

impl TypeMappingRegistry {
    /// Create a new empty registry
    ///
    /// NOTE: No built-in types are registered. Finance-specific types like
    /// "Candle" should be registered at CLI/application startup from stdlib.
    pub fn new() -> Self {
        Self {
            mappings: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register a type mapping
    ///
    /// # Arguments
    ///
    /// * `type_name` - Name of the type
    /// * `mapping` - TypeMapping specifying required columns
    pub fn register(&self, type_name: &str, mapping: TypeMapping) {
        let mut mappings = self.mappings.write().unwrap();
        mappings.insert(type_name.to_string(), mapping);
    }

    /// Get type mapping by name
    ///
    /// # Arguments
    ///
    /// * `type_name` - Type name to lookup
    ///
    /// # Returns
    ///
    /// TypeMapping if found, None otherwise
    pub fn get(&self, type_name: &str) -> Option<TypeMapping> {
        let mappings = self.mappings.read().unwrap();
        mappings.get(type_name).cloned()
    }

    /// Check if a type mapping exists
    pub fn has(&self, type_name: &str) -> bool {
        let mappings = self.mappings.read().unwrap();
        mappings.contains_key(type_name)
    }

    /// List all registered type names
    pub fn list_types(&self) -> Vec<String> {
        let mappings = self.mappings.read().unwrap();
        mappings.keys().cloned().collect()
    }

    /// Unregister a type mapping
    pub fn unregister(&self, type_name: &str) -> bool {
        let mut mappings = self.mappings.write().unwrap();
        mappings.remove(type_name).is_some()
    }

    /// Clear all type mappings
    pub fn clear(&self) {
        let mut mappings = self.mappings.write().unwrap();
        mappings.clear();
    }
}

impl Default for TypeMappingRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::Timeframe;

    #[test]
    fn test_generic_mapping() {
        // Test creating a generic type mapping
        let mapping = TypeMapping::new("SensorData".to_string())
            .add_field("temp", "temperature")
            .add_field("humid", "humidity")
            .add_required("temperature")
            .add_required("humidity");

        assert_eq!(mapping.type_name, "SensorData");
        assert_eq!(mapping.required_columns.len(), 2);
        assert!(
            mapping
                .required_columns
                .contains(&"temperature".to_string())
        );
        assert!(mapping.required_columns.contains(&"humidity".to_string()));
        assert_eq!(mapping.get_column("temp"), Some("temperature"));
    }

    #[test]
    fn test_validate_success() {
        // Create a generic type mapping
        let mapping = TypeMapping::new("Metrics".to_string())
            .add_required("value")
            .add_required("count");

        let mut df = DataFrame::new("TEST", Timeframe::d1());
        df.add_column("value", vec![100.0, 101.0]);
        df.add_column("count", vec![5.0, 6.0]);

        assert!(mapping.validate(&df).is_ok());
    }

    #[test]
    fn test_validate_missing_column() {
        // Create a generic type mapping with required columns
        let mapping = TypeMapping::new("DataPoint".to_string())
            .add_required("value")
            .add_required("count");

        let mut df = DataFrame::new("TEST", Timeframe::d1());
        df.add_column("value", vec![100.0]);
        // Missing: count

        assert!(mapping.validate(&df).is_err());
    }

    #[test]
    fn test_custom_mapping() {
        let mapping = TypeMapping::new("CustomType".to_string())
            .add_field("price", "close")
            .add_field("size", "volume")
            .add_required("close")
            .add_required("volume");

        assert_eq!(mapping.type_name, "CustomType");
        assert_eq!(mapping.get_column("price"), Some("close"));
        assert_eq!(mapping.get_column("size"), Some("volume"));
        assert_eq!(mapping.required_columns.len(), 2);
    }

    #[test]
    fn test_registry_operations() {
        let registry = TypeMappingRegistry::new();

        // Registry starts empty - no built-in types
        assert!(!registry.has("Candle"));
        assert_eq!(registry.list_types().len(), 0);

        // Register a type
        let custom = TypeMapping::new("DataPoint".to_string());
        registry.register("DataPoint", custom);

        assert!(registry.has("DataPoint"));
        assert_eq!(registry.list_types().len(), 1);

        // Unregister
        assert!(registry.unregister("DataPoint"));
        assert!(!registry.has("DataPoint"));
    }

    #[test]
    fn test_registry_clear() {
        let registry = TypeMappingRegistry::new();
        registry.clear();

        assert_eq!(registry.list_types().len(), 0);
    }
}
