//! Dynamic chart renderer driven by type metadata
//!
//! This module implements the "Consumer" side of the Type Metadata Protocol.
//! It takes a WireValue and its associated TypeMetadata to decide how to
//! render a chart without knowing any domain-specific types.

use anyhow::{Result, anyhow};
use shape_viz_core::ChartConfig;
use shape_wire::{WireValue, metadata::TypeInfo};

/// Dynamic renderer that switches behavior based on metadata
pub struct DynamicChartRenderer {}

impl DynamicChartRenderer {
    pub fn new(_config: ChartConfig) -> Self {
        Self {}
    }

    /// Render a chart from a wire value and its metadata
    ///
    /// NOTE: Chart rendering from WireTable (Arrow IPC) is not yet implemented.
    /// The old WireSeries columnar format was removed during the Series→Column migration.
    pub async fn render(&self, _value: &WireValue, _type_info: &TypeInfo) -> Result<Vec<u8>> {
        Err(anyhow!(
            "Chart rendering from DataTable not yet implemented (requires Arrow IPC deserialization)"
        ))
    }
}
