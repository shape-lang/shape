//! Path utility implementations for the io module.
//!
//! Phase 2c partial: registrations DEFERRED. The 5 path utility functions
//! (io.join, io.dirname, io.basename, io.extension, io.resolve) include
//! one varargs case (`io.join(parts...)`) that needs a varargs-marshal
//! sub-cluster decision and one `Array<string>`-input case that needs an
//! `Array<string>` FromSlot impl (the cluster #3 option β precedent
//! applies, but `TypedArrayData` has no String variant — the storage
//! shape is its own architectural call).
//!
//! Other 4 functions are mechanical migrations once the varargs decision
//! lands; they're held with the cluster so the io.join/Array<string>
//! work doesn't fragment them.
//!
//! Surfaced for next session as the `stdlib_io path-mass cluster, varargs
//! sub-decision` follow-up workstream. The original ValueWord-based
//! function bodies have been deleted to make the absence visible (per
//! the bulldozer-pattern precedent — simulation engine deletion, etc.).

use crate::module_exports::ModuleExports;

/// Register path-utility functions on the io module. Currently empty
/// pending the varargs-marshal decision (see module-level doc comment).
pub fn register_path_io(_module: &mut ModuleExports) {
    // Deferred: io.join, io.dirname, io.basename, io.extension, io.resolve.
}
