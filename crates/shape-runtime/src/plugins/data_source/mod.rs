//! Plugin Data Source Wrapper
//!
//! Provides a Rust-friendly wrapper around the C ABI data source interface.

mod providers;
mod query;
mod schema;

// Re-export schema types
pub use schema::{ParsedOutputField, ParsedOutputSchema, ParsedQueryParam, ParsedQuerySchema};

use std::ffi::c_void;
use shape_value::ValueWordExt;

use serde_json::Value;
use shape_abi_v1::DataSourceVTable;
use shape_ast::error::{Result, ShapeError};

/// Wrapper around a plugin data source
///
/// Provides Rust-friendly access to plugin functionality including
/// self-describing schema for LSP autocomplete and validation.
pub struct PluginDataSource {
    /// Plugin name
    name: String,
    /// Vtable pointer (static lifetime)
    vtable: &'static DataSourceVTable,
    /// Instance pointer (owned by this struct)
    instance: *mut c_void,
    /// Cached query schema
    query_schema: ParsedQuerySchema,
    /// Cached output schema
    output_schema: ParsedOutputSchema,
}

impl PluginDataSource {
    /// Create a new plugin data source instance
    ///
    /// # Arguments
    /// * `name` - Plugin name
    /// * `vtable` - Data source vtable (must be static)
    /// * `config` - Configuration value (will be MessagePack encoded)
    pub fn new(name: String, vtable: &'static DataSourceVTable, config: &Value) -> Result<Self> {
        // Serialize config to MessagePack
        let config_bytes = rmp_serde::to_vec(config).map_err(|e| ShapeError::RuntimeError {
            message: format!("Failed to serialize plugin config: {}", e),
            location: None,
        })?;

        // Initialize the plugin instance
        let init_fn = vtable.init.ok_or_else(|| ShapeError::RuntimeError {
            message: format!("Plugin '{}' has no init function", name),
            location: None,
        })?;

        let instance = unsafe { init_fn(config_bytes.as_ptr(), config_bytes.len()) };
        if instance.is_null() {
            return Err(ShapeError::RuntimeError {
                message: format!("Plugin '{}' init returned null", name),
                location: None,
            });
        }

        // Parse query schema
        let query_schema = query::parse_query_schema_from_vtable(vtable, instance)?;

        // Parse output schema
        let output_schema = query::parse_output_schema_from_vtable(vtable, instance)?;

        Ok(Self {
            name,
            vtable,
            instance,
            query_schema,
            output_schema,
        })
    }

    /// Get the query schema for LSP autocomplete and validation
    pub fn get_query_schema(&self) -> &ParsedQuerySchema {
        &self.query_schema
    }

    /// Get the output schema for LSP autocomplete
    pub fn get_output_schema(&self) -> &ParsedOutputSchema {
        &self.output_schema
    }

    /// Get the plugin name
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Validate a query before execution
    pub fn validate_query(&self, query: &Value) -> Result<()> {
        query::validate_query(self.vtable, self.instance, query)
    }

    /// Load historical data
    pub fn load(&self, query: &Value) -> Result<Value> {
        providers::load(self.vtable, self.instance, &self.name, query)
    }

    /// Subscribe to streaming data
    ///
    /// # Arguments
    /// * `query` - Query parameters
    /// * `callback` - Called for each data point
    ///
    /// # Returns
    /// Subscription ID (use to unsubscribe)
    pub fn subscribe<F>(&self, query: &Value, callback: F) -> Result<u64>
    where
        F: Fn(Value) + Send + Sync + 'static,
    {
        providers::subscribe(self.vtable, self.instance, &self.name, query, callback)
    }

    /// Unsubscribe from streaming data
    pub fn unsubscribe(&self, subscription_id: u64) -> Result<()> {
        providers::unsubscribe(self.vtable, self.instance, &self.name, subscription_id)
    }

    /// Query the schema for a specific data source.
    ///
    /// This enables runtime schema discovery - the plugin returns what columns
    /// are available for a given source ID.
    ///
    /// # Arguments
    /// * `source_id` - The source identifier (e.g., symbol, table name, device ID)
    ///
    /// # Returns
    /// The plugin schema with column information
    pub fn get_source_schema(&self, source_id: &str) -> Result<shape_abi_v1::PluginSchema> {
        providers::get_source_schema(self.vtable, self.instance, &self.name, source_id)
    }

    /// Check if this plugin supports schema discovery
    pub fn supports_schema_discovery(&self) -> bool {
        self.vtable.get_source_schema.is_some()
    }

    /// Check if this plugin supports binary loading (ABI v2)
    pub fn supports_binary(&self) -> bool {
        self.vtable.load_binary.is_some()
    }

    /// Load historical data in binary columnar format (ABI v2)
    pub fn load_binary(
        &self,
        query: &Value,
        granularity: crate::progress::ProgressGranularity,
        progress_handle: Option<&crate::progress::ProgressHandle>,
    ) -> Result<shape_value::ValueWord> {
        providers::load_binary(
            self.vtable,
            self.instance,
            &self.name,
            query,
            granularity,
            progress_handle,
        )
    }
}

impl Drop for PluginDataSource {
    fn drop(&mut self) {
        if let Some(drop_fn) = self.vtable.drop {
            unsafe { drop_fn(self.instance) };
        }
    }
}

// SAFETY: The instance pointer is only accessed through the vtable functions
// which are required to be thread-safe by the plugin contract.
unsafe impl Send for PluginDataSource {}
unsafe impl Sync for PluginDataSource {}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_abi_v1::ParamType;

    #[test]
    fn test_parsed_query_schema_default() {
        let schema = ParsedQuerySchema {
            params: Vec::new(),
            example_query: None,
        };
        assert!(schema.params.is_empty());
        assert!(schema.example_query.is_none());
    }

    #[test]
    fn test_parsed_query_param_creation() {
        let param = ParsedQueryParam {
            name: "symbol".to_string(),
            description: "Trading symbol".to_string(),
            param_type: ParamType::String,
            required: true,
            default_value: None,
            allowed_values: None,
            nested_schema: None,
        };

        assert_eq!(param.name, "symbol");
        assert!(param.required);
        assert!(matches!(param.param_type, ParamType::String));
    }

    #[test]
    fn test_parsed_query_param_with_defaults() {
        let param = ParsedQueryParam {
            name: "timeframe".to_string(),
            description: "Data timeframe".to_string(),
            param_type: ParamType::String,
            required: false,
            default_value: Some(serde_json::json!("1d")),
            allowed_values: Some(vec![
                serde_json::json!("1m"),
                serde_json::json!("1h"),
                serde_json::json!("1d"),
            ]),
            nested_schema: None,
        };

        assert!(!param.required);
        assert!(param.default_value.is_some());
        assert_eq!(param.allowed_values.as_ref().unwrap().len(), 3);
    }

    #[test]
    fn test_parsed_query_param_with_nested_schema() {
        let nested = ParsedQuerySchema {
            params: vec![ParsedQueryParam {
                name: "field".to_string(),
                description: "Nested field".to_string(),
                param_type: ParamType::String,
                required: true,
                default_value: None,
                allowed_values: None,
                nested_schema: None,
            }],
            example_query: None,
        };

        let param = ParsedQueryParam {
            name: "filter".to_string(),
            description: "Filter object".to_string(),
            param_type: ParamType::Object,
            required: false,
            default_value: None,
            allowed_values: None,
            nested_schema: Some(Box::new(nested)),
        };

        assert!(param.nested_schema.is_some());
        assert_eq!(param.nested_schema.as_ref().unwrap().params.len(), 1);
    }

    #[test]
    fn test_parsed_output_schema() {
        let schema = ParsedOutputSchema {
            fields: vec![
                ParsedOutputField {
                    name: "timestamp".to_string(),
                    field_type: ParamType::String,
                    description: "Unix timestamp".to_string(),
                },
                ParsedOutputField {
                    name: "value".to_string(),
                    field_type: ParamType::Number,
                    description: "Measurement value".to_string(),
                },
            ],
        };

        assert_eq!(schema.fields.len(), 2);
        assert_eq!(schema.fields[0].name, "timestamp");
        assert_eq!(schema.fields[1].name, "value");
    }

    #[test]
    fn test_parsed_query_schema_with_params() {
        let schema = ParsedQuerySchema {
            params: vec![
                ParsedQueryParam {
                    name: "symbol".to_string(),
                    description: "Symbol".to_string(),
                    param_type: ParamType::String,
                    required: true,
                    default_value: None,
                    allowed_values: None,
                    nested_schema: None,
                },
                ParsedQueryParam {
                    name: "start_date".to_string(),
                    description: "Start date".to_string(),
                    param_type: ParamType::String,
                    required: false,
                    default_value: None,
                    allowed_values: None,
                    nested_schema: None,
                },
            ],
            example_query: Some(serde_json::json!({"symbol": "AAPL"})),
        };

        assert_eq!(schema.params.len(), 2);
        assert!(schema.example_query.is_some());

        // Check first param is required
        assert!(schema.params[0].required);
        assert!(!schema.params[1].required);
    }

    #[test]
    fn test_param_type_variants() {
        // Test all ParamType variants are usable
        let string_type = ParamType::String;
        let number_type = ParamType::Number;
        let bool_type = ParamType::Bool;
        let string_array_type = ParamType::StringArray;
        let number_array_type = ParamType::NumberArray;
        let object_type = ParamType::Object;

        assert!(matches!(string_type, ParamType::String));
        assert!(matches!(number_type, ParamType::Number));
        assert!(matches!(bool_type, ParamType::Bool));
        assert!(matches!(string_array_type, ParamType::StringArray));
        assert!(matches!(number_array_type, ParamType::NumberArray));
        assert!(matches!(object_type, ParamType::Object));
    }
}
