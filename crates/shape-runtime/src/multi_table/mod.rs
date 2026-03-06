//! Multi-series data alignment and analysis module
//!
//! This module provides functionality for:
//! - Aligning row data across multiple data sources/series
//! - Handling different time ranges and gaps
//! - Supporting various alignment modes (intersection, union, etc.)
//! - Temporal joins for time-series data

pub mod alignment;
pub mod analysis;
pub mod config;
pub mod functions;
pub mod types;

// Re-export commonly used types
pub use alignment::{align_intersection, align_left, align_union};
pub use analysis::MultiTableAnalysis;
pub use config::{AlignmentConfig, AlignmentMode, GapFillMethod};
pub use types::{AlignedData, AlignmentMetadata, Divergence, JoinType};
