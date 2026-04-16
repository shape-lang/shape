//! FFI Functions for JIT-compiled Code
//!
//! External C functions that are called from JIT-compiled code to perform
//! operations that cannot be done inline (heap allocation, complex logic, etc.)

pub mod arc;
pub mod array;
pub mod data;
pub mod jit_kinds;
pub mod object;
// DELETED: Finance-specific indicator JIT module
// pub mod indicator;
pub mod async_ops;
pub mod call_method;
pub mod control;
pub mod conversion;
pub mod gc;
pub mod generic_builtin;
pub mod iterator;
pub mod join;
pub mod math;
pub mod references;
pub mod result;
pub mod simd;
pub mod typed_object;
pub mod v2;
pub mod value_ffi;
pub mod v2_array;
pub mod v2_core;
pub mod v2_math;
pub mod v2_string_ffi;
pub mod v2_struct;
pub mod v2_typed;
pub mod window;

// Re-export all FFI functions for easy access
pub use jit_kinds::*;
pub use value_ffi::*;
pub use array::*;
pub use data::*;
pub use object::*;
// DELETED: Finance-specific indicator exports
// pub use indicator::*;
pub use async_ops::*;
pub use call_method::jit_call_method;
pub use control::*;
pub use conversion::*;
pub use gc::*;
pub use generic_builtin::*;
pub use iterator::*;
pub use join::*;
pub use math::*;
pub use references::*;
pub use result::*;
pub use simd::*;
pub use typed_object::*;
pub use v2_typed::*;
pub use window::*;
