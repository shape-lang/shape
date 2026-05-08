//! Generic load query for industry-agnostic data loading.
//!
//! Phase 1.B (ADR-006 §2.7.4 audit-accuracy ruling): the
//! `to_data_query` body decoded `&ValueWord` parameters via tag-bit
//! dispatch (`as_str()`, `as_time()`, `as_timeframe()`, `as_duration()`,
//! `as_f64()`) that no longer exists. The kind-threaded rebuild
//! lands in Phase 2c when the per-position `NativeKind` is threaded
//! from the schema; until then the body returns a deferred error and
//! the type signature is preserved at the [`KindedSlot`] shape per
//! ADR-006 §2.7.5.

use super::DataQuery;
use shape_ast::error::{Result, ShapeError};
use shape_value::KindedSlot;
use std::collections::HashMap;

/// Generic data load request (industry-agnostic).
#[derive(Debug, Clone, Default)]
pub struct LoadQuery {
    /// Provider name (e.g., "data", "api", "warehouse"). If `None`,
    /// uses the default provider.
    pub provider: Option<String>,

    /// Generic parameters (arbitrary key-value). `KindedSlot`'s explicit
    /// `Drop`/`Clone` impls dispatch on `NativeKind` so each parameter's
    /// heap refcount is released on teardown.
    pub params: HashMap<String, KindedSlot>,

    /// Target type name for validation (e.g., "Candle", "TickData").
    pub target_type: Option<String>,

    /// Optional column mapping override.
    pub column_mapping: Option<HashMap<String, String>>,
}

impl LoadQuery {
    /// Create a new empty load query.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the provider name.
    pub fn with_provider(mut self, name: &str) -> Self {
        self.provider = Some(name.to_string());
        self
    }

    /// Add a parameter.
    pub fn with_param(mut self, key: &str, value: KindedSlot) -> Self {
        self.params.insert(key.to_string(), value);
        self
    }

    /// Set the target type for validation.
    pub fn with_type(mut self, type_name: &str) -> Self {
        self.target_type = Some(type_name.to_string());
        self
    }

    /// Set the column mapping.
    pub fn with_column_mapping(mut self, mapping: HashMap<String, String>) -> Self {
        self.column_mapping = Some(mapping);
        self
    }

    /// Convert to a provider-specific [`DataQuery`].
    ///
    /// Phase 1.B: the param-decode helpers (`value_to_timestamp`,
    /// `as_timeframe`/`as_duration` decoders) are deferred to Phase 2c.
    /// Until then, this method returns a deferred error rather than
    /// silently produce a malformed `DataQuery`.
    pub fn to_data_query(&self) -> Result<DataQuery> {
        Err(ShapeError::RuntimeError {
            message: "LoadQuery::to_data_query: pending Phase 2c kind-threaded param decode — see ADR-006 §2.7.4".to_string(),
            location: None,
        })
    }
}

#[cfg(test)]
mod tests {
    // Pre-bulldozer behavioural tests covered the `to_data_query`
    // happy-path / error-path / range / limit shapes. They return
    // alongside the kind-threaded rebuild in Phase 2c.

    use super::*;

    #[test]
    fn test_basic_query_metadata() {
        // Metadata-only test that doesn't invoke the deferred body.
        let query = LoadQuery::new()
            .with_provider("data")
            .with_type("Candle");

        assert_eq!(query.provider, Some("data".to_string()));
        assert_eq!(query.target_type, Some("Candle".to_string()));
    }
}
