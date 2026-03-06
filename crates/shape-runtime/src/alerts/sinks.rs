//! Alert Sinks
//!
//! Output sinks for alert delivery.

use super::types::Alert;
use shape_ast::error::Result;

/// Trait for alert output sinks
///
/// Implement this trait to create custom alert destinations.
///
/// CLI-specific sinks (like ConsoleSink) should be implemented in shape-cli.
pub trait AlertSink: Send + Sync {
    /// Get the sink name
    fn name(&self) -> &str;

    /// Send an alert to this sink
    fn send(&self, alert: &Alert) -> Result<()>;

    /// Flush any pending alerts
    fn flush(&self) -> Result<()> {
        Ok(()) // Default: no-op
    }

    /// Get the tags this sink handles
    ///
    /// Returns an empty slice to handle all alerts.
    fn handles_tags(&self) -> &[String] {
        &[]
    }
}
