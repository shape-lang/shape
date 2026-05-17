//! Wave 5b (phase-1b-vm): special-op bodies migrated or deferred.
//!
//! - `builtin_snapshot` and `builtin_exit` are now inlined directly in
//!   `vm_impl/builtins.rs` (terminal-action shapes don't return a value
//!   the dispatch shape can carry — `Snapshot` raises `VMError::Suspended`
//!   and `Exit` calls `std::process::exit`).
//! - `builtin_print`, `builtin_format`, `builtin_format_with_meta`,
//!   `builtin_format_with_spec`, `builtin_reflect`, `builtin_control_fold`,
//!   `builtin_make_content_*`, `builtin_apply_content_style`,
//!   `builtin_make_table_from_rows` are deferred to Waves 5d/5e (formatter
//!   path lives in `executor/printing.rs`; content builders depend on
//!   `shape_runtime::content_builders` not yet migrated).
//!
//! This file is intentionally empty post-Wave 5b — the previous bodies
//! were already dead under the `todo!`-stubbed dispatch (they call into
//! deleted `ValueWord` machinery). Kept as a module placeholder so the
//! `mod special_ops;` declaration in `builtins/mod.rs` keeps resolving
//! until the deferred bodies land in Waves 5d/5e.
