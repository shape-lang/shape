//! Object FFI Functions for JIT
//!
//! This module provides FFI functions for creating and manipulating objects,
//! arrays, closures, and performing property access operations in the JIT.
//!
//! ## Modules
//!
//! - `object_ops` - Object creation, manipulation, and metadata operations
//! - `property_access` - Property access for objects, arrays, strings, series, and other types
//! - `conversion` - Conversion between NaN-boxed bits and runtime Values
//! - `format` - String formatting with template substitution
//! - `closure` - Closure creation with captured values
//! - `pattern` - Pattern matching helpers for Result/Option types

pub mod closure;
pub mod conversion;
pub mod format;
pub mod object_ops;
pub mod pattern;
pub mod property_access;

// Re-export all public functions for backward compatibility
pub use object_ops::{jit_new_object, jit_object_rest, jit_set_prop};

pub use property_access::{jit_get_prop, jit_hashmap_shape_id, jit_hashmap_value_at, jit_length};

pub use conversion::{
    jit_bits_to_nanboxed, jit_bits_to_nanboxed_with_ctx, jit_bits_to_typed_scalar,
    nanboxed_to_jit_bits, typed_scalar_to_jit_bits,
};

pub use format::jit_format;

#[allow(deprecated)]
pub use closure::jit_make_closure;

pub use closure::jit_finalize_heap_closure;

// Track A.1D: OwnedMutable capture cell allocator (Box<ValueWord>). Called
// from `MirToIR::emit_heap_closure` to materialise the cell pointer that
// gets installed into the closure's `Ptr` capture slot.
pub use closure::jit_alloc_owned_mut_cell;

// Track A.1E: Shared capture FFI helpers.
//   - `jit_arc_shared_retain`        — Arc strong-count retain for
//                                       `emit_heap_closure`'s Shared branch.
//   - `jit_shared_lock_contended`    — slow-path lock spin-wait.
//   - `jit_shared_unlock_contended`  — slow-path unlock release store.
pub use closure::{
    jit_arc_shared_retain, jit_shared_lock_contended, jit_shared_unlock_contended,
};

// Session 1 Commit 3: Outer-scope Shared-cell lifecycle helpers.
//   - `jit_alloc_shared_cell`        — allocates a fresh `Arc<SharedCell>`
//                                       with the given `ValueWord` initial
//                                       bits; returns the raw pointer bits
//                                       of the sole strong share.
//   - `jit_arc_shared_release`       — consumes exactly one strong share
//                                       (outer-scope reclaim); null-safe.
pub use closure::{jit_alloc_shared_cell, jit_arc_shared_release};

pub use pattern::{jit_pattern_check_constructor, jit_pattern_extract_constructor};
