//! Integration tests for VM snapshots and resume-from-snapshot.
//!
//! Shape supports snapshotting VM state to disk and resuming execution
//! from a snapshot. The ShapeTest builder exposes `.with_snapshots()`
//! for enabling a temporary snapshot store.

mod advanced;
mod basic;
