//! Alert Pipeline System
//!
//! Provides alert generation, routing, and delivery for Shape.
//!
//! # Overview
//!
//! The alert system enables:
//! - Real-time alert generation from Shape code
//! - Tag-based routing to multiple sinks
//! - Plugin-based output sinks (webhooks, email, etc.)
//! - Dead-letter queue for failed deliveries
//!
//! # Example
//!
//! ```ignore
//! // In Shape code
//! alert("Price Alert", "Price crossed threshold", {
//!     severity: "warning",
//!     tags: ["price", "btc"],
//!     data: { price: current_price }
//! });
//! ```

mod router;
mod sinks;
mod types;

pub use router::AlertRouter;
pub use sinks::AlertSink;
pub use types::{Alert, AlertSeverity};
