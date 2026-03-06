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

pub use closure::jit_make_closure;

pub use pattern::{jit_pattern_check_constructor, jit_pattern_extract_constructor};
