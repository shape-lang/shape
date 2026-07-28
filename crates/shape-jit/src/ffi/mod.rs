//! FFI Functions for JIT-compiled Code
//!
//! External C functions that are called from JIT-compiled code to perform
//! operations that cannot be done inline (heap allocation, complex logic, etc.)

pub mod arc;
pub mod array;
pub mod data;
pub mod jit_kinds;
pub mod jit_release;
pub mod object;
pub mod stack_kind_code;
// DELETED: Finance-specific indicator JIT module
// pub mod indicator;
pub mod async_ops;
pub mod call_method;
pub mod control;
pub mod conversion;
pub mod formatting;
pub mod gc;
pub mod generic_builtin;
pub mod iterator;
pub mod join;
pub mod math;
pub mod references;
pub mod result;
pub mod simd;
// W12-jit-string-carrier-unification (Phase 3 cluster-0 Round 12 T2/T3,
// 2026-05-13). §2.7.5 `Arc<String>` strict-typed carrier retain/release +
// compile-time constant helper. See module header for the carrier-shape
// rule binding.
pub mod string;
pub mod typed_object;
pub mod v2;
pub mod value_ffi;
// #117 / R15: the native-entry callback the witness reads dispatch from.
pub mod witness;
// V2.b: v2_array (v1 TypedArrayHeader FFI) deleted — canonical FFI is `v2/mod.rs`
pub mod v2_core;
pub mod v2_math;
pub mod v2_string_ffi;
pub mod v2_struct;
pub mod v2_typed;

// Re-export all FFI functions for easy access
pub use array::*;
pub use data::*;
pub use jit_kinds::*;
pub use object::*;
pub use value_ffi::*;
// DELETED: Finance-specific indicator exports
// pub use indicator::*;
pub use async_ops::*;
pub use call_method::jit_call_method;
pub use control::*;
pub use conversion::*;
pub use gc::*;
pub use generic_builtin::*;
pub use iterator::*;
pub use math::*;
pub use references::*;
pub use result::*;
pub use simd::*;
pub use typed_object::*;
