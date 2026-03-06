//! Backward compatibility re-export for feature_matrix module
//!
//! This module has been split into smaller modules under `feature_tests/`
//! for better organization and maintainability. This file provides backward
//! compatibility by re-exporting all public items from the new structure.

pub use crate::feature_tests::*;
