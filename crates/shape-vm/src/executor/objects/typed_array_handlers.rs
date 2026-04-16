//! Typed array handler re-exports for the objects module.
//!
//! Bridges the low-level `typed_handlers::array_detect` and
//! `typed_handlers::typed_array` infrastructure into the objects module
//! namespace. Method dispatch code in `method_registry.rs` and
//! `typed_array_methods.rs` can import from here instead of reaching
//! across to the sibling executor module.

pub use crate::executor::typed_handlers::array_detect::{
    as_native_typed_array, stamp_elem_type, NativeElemType, NativeTypedArrayView,
    ELEM_TYPE_BOOL, ELEM_TYPE_F64, ELEM_TYPE_I32, ELEM_TYPE_I64,
};

pub use crate::executor::typed_handlers::typed_array::{
    op_typed_array_alloc_bool, op_typed_array_alloc_f64, op_typed_array_alloc_i32,
    op_typed_array_alloc_i64, op_typed_array_get_bool, op_typed_array_get_f64,
    op_typed_array_get_i32, op_typed_array_get_i64, op_typed_array_len,
    op_typed_array_push_bool, op_typed_array_push_f64, op_typed_array_push_i32,
    op_typed_array_push_i64, op_typed_array_set_bool, op_typed_array_set_f64,
    op_typed_array_set_i32, op_typed_array_set_i64,
};
