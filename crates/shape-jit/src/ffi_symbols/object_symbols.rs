//! Object FFI Symbol Registration
//!
//! This module handles registration and declaration of object-related FFI symbols
//! for the JIT compiler.

use cranelift::prelude::*;
use cranelift_jit::JITBuilder;
use cranelift_jit::JITModule;
use cranelift_module::{FuncId, Linkage, Module};
use std::collections::HashMap;

use super::super::ffi::conversion::{
    jit_print, jit_to_number, jit_to_string, jit_type_check, jit_typeof,
};
use super::super::ffi::object::{
    jit_format, jit_get_prop, jit_hashmap_shape_id, jit_hashmap_value_at, jit_length,
    jit_make_closure, jit_new_object, jit_object_rest, jit_set_prop,
};
use super::super::ffi::typed_object::{jit_typed_merge_object, jit_typed_object_alloc};
use super::super::ffi::typed_object::jit_typed_object_get_field;
use super::super::ffi::typed_object::jit_typed_object_set_field;
use super::helpers::jit_format_error;

/// Register object FFI symbols with the JIT builder
pub fn register_object_symbols(builder: &mut JITBuilder) {
    builder.symbol("jit_new_object", jit_new_object as *const u8);
    builder.symbol("jit_get_prop", jit_get_prop as *const u8);
    builder.symbol("jit_set_prop", jit_set_prop as *const u8);
    builder.symbol("jit_length", jit_length as *const u8);
    builder.symbol("jit_typeof", jit_typeof as *const u8);
    builder.symbol("jit_type_check", jit_type_check as *const u8);
    builder.symbol("jit_to_string", jit_to_string as *const u8);
    builder.symbol("jit_to_number", jit_to_number as *const u8);
    builder.symbol("jit_print", jit_print as *const u8);
    builder.symbol("jit_make_closure", jit_make_closure as *const u8);
    builder.symbol("jit_object_rest", jit_object_rest as *const u8);
    builder.symbol("jit_format", jit_format as *const u8);
    builder.symbol("jit_format_error", jit_format_error as *const u8);
    builder.symbol(
        "jit_typed_object_alloc",
        jit_typed_object_alloc as *const u8,
    );
    builder.symbol(
        "jit_typed_merge_object",
        jit_typed_merge_object as *const u8,
    );
    builder.symbol(
        "jit_typed_object_get_field",
        super::super::ffi::typed_object::jit_typed_object_get_field as *const u8,
    );
    builder.symbol(
        "jit_typed_object_set_field",
        super::super::ffi::typed_object::jit_typed_object_set_field as *const u8,
    );
    builder.symbol("jit_hashmap_shape_id", jit_hashmap_shape_id as *const u8);
    builder.symbol("jit_hashmap_value_at", jit_hashmap_value_at as *const u8);
}

/// Declare object FFI function signatures in the module
pub fn declare_object_functions(module: &mut JITModule, ffi_funcs: &mut HashMap<String, FuncId>) {
    // jit_new_object(ctx: *mut JITContext, field_count: usize) -> u64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ctx
        sig.params.push(AbiParam::new(types::I64)); // field_count
        sig.returns.push(AbiParam::new(types::I64)); // result
        let func_id = module
            .declare_function("jit_new_object", Linkage::Import, &sig)
            .expect("Failed to declare jit_new_object");
        ffi_funcs.insert("jit_new_object".to_string(), func_id);
    }

    // jit_get_prop(obj_bits: u64, key_bits: u64) -> u64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // obj_bits
        sig.params.push(AbiParam::new(types::I64)); // key_bits
        sig.returns.push(AbiParam::new(types::I64)); // result
        let func_id = module
            .declare_function("jit_get_prop", Linkage::Import, &sig)
            .expect("Failed to declare jit_get_prop");
        ffi_funcs.insert("jit_get_prop".to_string(), func_id);
    }

    // jit_set_prop(obj_bits: u64, key_bits: u64, value_bits: u64) -> u64 (returns modified container)
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // obj_bits
        sig.params.push(AbiParam::new(types::I64)); // key_bits
        sig.params.push(AbiParam::new(types::I64)); // value_bits
        sig.returns.push(AbiParam::new(types::I64)); // modified container
        let func_id = module
            .declare_function("jit_set_prop", Linkage::Import, &sig)
            .expect("Failed to declare jit_set_prop");
        ffi_funcs.insert("jit_set_prop".to_string(), func_id);
    }

    // jit_length(obj_bits: u64) -> u64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // obj_bits
        sig.returns.push(AbiParam::new(types::I64)); // result
        let func_id = module
            .declare_function("jit_length", Linkage::Import, &sig)
            .expect("Failed to declare jit_length");
        ffi_funcs.insert("jit_length".to_string(), func_id);
    }

    // jit_typeof(value_bits: u64) -> u64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // value_bits
        sig.returns.push(AbiParam::new(types::I64)); // result (boxed string)
        let func_id = module
            .declare_function("jit_typeof", Linkage::Import, &sig)
            .expect("Failed to declare jit_typeof");
        ffi_funcs.insert("jit_typeof".to_string(), func_id);
    }

    // jit_type_check(value_bits: u64, type_name_bits: u64) -> u64 (bool)
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // value_bits
        sig.params.push(AbiParam::new(types::I64)); // type_name_bits
        sig.returns.push(AbiParam::new(types::I64)); // result (bool)
        let func_id = module
            .declare_function("jit_type_check", Linkage::Import, &sig)
            .expect("Failed to declare jit_type_check");
        ffi_funcs.insert("jit_type_check".to_string(), func_id);
    }

    // jit_to_string(value_bits) -> u64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        let func_id = module
            .declare_function("jit_to_string", Linkage::Import, &sig)
            .expect("Failed to declare jit_to_string");
        ffi_funcs.insert("jit_to_string".to_string(), func_id);
    }

    // jit_to_number(value_bits) -> u64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        let func_id = module
            .declare_function("jit_to_number", Linkage::Import, &sig)
            .expect("Failed to declare jit_to_number");
        ffi_funcs.insert("jit_to_number".to_string(), func_id);
    }

    // jit_print(value_bits: u64) -> void
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // value_bits
        let func_id = module
            .declare_function("jit_print", Linkage::Import, &sig)
            .expect("Failed to declare jit_print");
        ffi_funcs.insert("jit_print".to_string(), func_id);
    }

    // jit_make_closure(ctx, func_idx, capture_count) -> u64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ctx
        sig.params.push(AbiParam::new(types::I64)); // func_idx
        sig.params.push(AbiParam::new(types::I64)); // capture_count
        sig.returns.push(AbiParam::new(types::I64));
        let func_id = module
            .declare_function("jit_make_closure", Linkage::Import, &sig)
            .expect("Failed to declare jit_make_closure");
        ffi_funcs.insert("jit_make_closure".to_string(), func_id);
    }

    // jit_object_rest(obj_bits: u64, keys_bits: u64) -> u64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // obj_bits
        sig.params.push(AbiParam::new(types::I64)); // keys_bits
        sig.returns.push(AbiParam::new(types::I64)); // result object
        let func_id = module
            .declare_function("jit_object_rest", Linkage::Import, &sig)
            .expect("Failed to declare jit_object_rest");
        ffi_funcs.insert("jit_object_rest".to_string(), func_id);
    }

    // jit_format(ctx: *mut JITContext, arg_count: usize) -> u64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ctx pointer
        sig.params.push(AbiParam::new(types::I64)); // arg_count
        sig.returns.push(AbiParam::new(types::I64)); // result string
        let func_id = module
            .declare_function("jit_format", Linkage::Import, &sig)
            .expect("Failed to declare jit_format");
        ffi_funcs.insert("jit_format".to_string(), func_id);
    }

    // jit_format_error(ctx: *mut JITContext) -> u64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ctx
        sig.returns.push(AbiParam::new(types::I64)); // error string
        let func_id = module
            .declare_function("jit_format_error", Linkage::Import, &sig)
            .expect("Failed to declare jit_format_error");
        ffi_funcs.insert("jit_format_error".to_string(), func_id);
    }

    // jit_typed_object_alloc(schema_id, data_size) -> u64 (TypedObject)
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I32)); // schema_id (u32)
        sig.params.push(AbiParam::new(types::I64)); // data_size
        sig.returns.push(AbiParam::new(types::I64)); // result (NaN-boxed TypedObject)
        let func_id = module
            .declare_function("jit_typed_object_alloc", Linkage::Import, &sig)
            .expect("Failed to declare jit_typed_object_alloc");
        ffi_funcs.insert("jit_typed_object_alloc".to_string(), func_id);
    }

    // jit_typed_object_get_field(obj_bits, offset) -> u64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        let func_id = module
            .declare_function("jit_typed_object_get_field", Linkage::Import, &sig)
            .expect("Failed to declare jit_typed_object_get_field");
        ffi_funcs.insert("jit_typed_object_get_field".to_string(), func_id);
    }

    // jit_typed_object_set_field(obj_bits, offset, value) -> u64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // obj_bits
        sig.params.push(AbiParam::new(types::I64)); // offset
        sig.params.push(AbiParam::new(types::I64)); // value
        sig.returns.push(AbiParam::new(types::I64)); // result (obj)
        let func_id = module
            .declare_function("jit_typed_object_set_field", Linkage::Import, &sig)
            .expect("Failed to declare jit_typed_object_set_field");
        ffi_funcs.insert("jit_typed_object_set_field".to_string(), func_id);
    }

    // jit_typed_merge_object(target_schema_id, left_size, right_size, left_obj, right_obj) -> u64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I32)); // target_schema_id (u32)
        sig.params.push(AbiParam::new(types::I64)); // left_size
        sig.params.push(AbiParam::new(types::I64)); // right_size
        sig.params.push(AbiParam::new(types::I64)); // left_obj (NaN-boxed)
        sig.params.push(AbiParam::new(types::I64)); // right_obj (NaN-boxed)
        sig.returns.push(AbiParam::new(types::I64)); // result (NaN-boxed TypedObject)
        let func_id = module
            .declare_function("jit_typed_merge_object", Linkage::Import, &sig)
            .expect("Failed to declare jit_typed_merge_object");
        ffi_funcs.insert("jit_typed_merge_object".to_string(), func_id);
    }

    // jit_hashmap_shape_id(obj_bits: u64) -> u32 (shape_id, 0 = no shape)
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // obj_bits
        sig.returns.push(AbiParam::new(types::I32)); // shape_id (u32)
        let func_id = module
            .declare_function("jit_hashmap_shape_id", Linkage::Import, &sig)
            .expect("Failed to declare jit_hashmap_shape_id");
        ffi_funcs.insert("jit_hashmap_shape_id".to_string(), func_id);
    }

    // jit_hashmap_value_at(obj_bits: u64, slot_index: u64) -> u64 (NaN-boxed value)
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // obj_bits
        sig.params.push(AbiParam::new(types::I64)); // slot_index
        sig.returns.push(AbiParam::new(types::I64)); // result (NaN-boxed value)
        let func_id = module
            .declare_function("jit_hashmap_value_at", Linkage::Import, &sig)
            .expect("Failed to declare jit_hashmap_value_at");
        ffi_funcs.insert("jit_hashmap_value_at".to_string(), func_id);
    }
}
