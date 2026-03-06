//! Array FFI Symbol Registration
//!
//! This module handles registration and declaration of array-related FFI symbols
//! for the JIT compiler.

use cranelift::prelude::*;
use cranelift_jit::JITBuilder;
use cranelift_jit::JITModule;
use cranelift_module::{FuncId, Linkage, Module};
use std::collections::HashMap;

use super::super::ffi::array::{
    jit_array_filled, jit_array_first, jit_array_get, jit_array_info, jit_array_last,
    jit_array_max, jit_array_min, jit_array_pop, jit_array_push, jit_array_push_elem,
    jit_array_push_element, jit_array_push_local, jit_array_reserve_local, jit_array_reverse,
    jit_array_zip, jit_hof_array_alloc, jit_hof_array_push, jit_make_range, jit_new_array,
    jit_range, jit_slice,
};

/// Register array FFI symbols with the JIT builder
pub fn register_array_symbols(builder: &mut JITBuilder) {
    builder.symbol("jit_new_array", jit_new_array as *const u8);
    builder.symbol("jit_array_get", jit_array_get as *const u8);
    builder.symbol("jit_array_push", jit_array_push as *const u8);
    builder.symbol("jit_array_pop", jit_array_pop as *const u8);
    builder.symbol("jit_array_push_elem", jit_array_push_elem as *const u8);
    builder.symbol("jit_array_zip", jit_array_zip as *const u8);
    builder.symbol("jit_array_first", jit_array_first as *const u8);
    builder.symbol("jit_array_last", jit_array_last as *const u8);
    builder.symbol("jit_array_min", jit_array_min as *const u8);
    builder.symbol("jit_array_max", jit_array_max as *const u8);
    builder.symbol("jit_slice", jit_slice as *const u8);
    builder.symbol("jit_range", jit_range as *const u8);
    builder.symbol("jit_make_range", jit_make_range as *const u8);
    builder.symbol("jit_array_info", jit_array_info as *const u8);
    builder.symbol("jit_array_push_local", jit_array_push_local as *const u8);
    builder.symbol(
        "jit_array_reserve_local",
        jit_array_reserve_local as *const u8,
    );
    builder.symbol("jit_array_filled", jit_array_filled as *const u8);
    builder.symbol("jit_array_reverse", jit_array_reverse as *const u8);
    builder.symbol(
        "jit_array_push_element",
        jit_array_push_element as *const u8,
    );
    builder.symbol("jit_hof_array_alloc", jit_hof_array_alloc as *const u8);
    builder.symbol("jit_hof_array_push", jit_hof_array_push as *const u8);
}

/// Declare array FFI function signatures in the module
pub fn declare_array_functions(module: &mut JITModule, ffi_funcs: &mut HashMap<String, FuncId>) {
    // jit_new_array(ctx: *mut JITContext, count: usize) -> u64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ctx
        sig.params.push(AbiParam::new(types::I64)); // count
        sig.returns.push(AbiParam::new(types::I64)); // result
        let func_id = module
            .declare_function("jit_new_array", Linkage::Import, &sig)
            .expect("Failed to declare jit_new_array");
        ffi_funcs.insert("jit_new_array".to_string(), func_id);
    }

    // jit_array_get(arr_bits: u64, idx_bits: u64) -> u64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // arr_bits
        sig.params.push(AbiParam::new(types::I64)); // idx_bits
        sig.returns.push(AbiParam::new(types::I64)); // result
        let func_id = module
            .declare_function("jit_array_get", Linkage::Import, &sig)
            .expect("Failed to declare jit_array_get");
        ffi_funcs.insert("jit_array_get".to_string(), func_id);
    }

    // jit_array_push(ctx: *mut JITContext, count: i64) -> u64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ctx
        sig.params.push(AbiParam::new(types::I64)); // count
        sig.returns.push(AbiParam::new(types::I64)); // result
        let func_id = module
            .declare_function("jit_array_push", Linkage::Import, &sig)
            .expect("Failed to declare jit_array_push");
        ffi_funcs.insert("jit_array_push".to_string(), func_id);
    }

    // jit_array_pop(arr_bits) -> u64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        let func_id = module
            .declare_function("jit_array_pop", Linkage::Import, &sig)
            .expect("Failed to declare jit_array_pop");
        ffi_funcs.insert("jit_array_pop".to_string(), func_id);
    }

    // jit_array_push_elem(arr_bits, value_bits) -> u64 (returns array)
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        let func_id = module
            .declare_function("jit_array_push_elem", Linkage::Import, &sig)
            .expect("Failed to declare jit_array_push_elem");
        ffi_funcs.insert("jit_array_push_elem".to_string(), func_id);
    }

    // jit_array_push_local(arr_bits, value_bits) -> u64 (mutates in-place, O(1) amortized)
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        let func_id = module
            .declare_function("jit_array_push_local", Linkage::Import, &sig)
            .expect("Failed to declare jit_array_push_local");
        ffi_funcs.insert("jit_array_push_local".to_string(), func_id);
    }

    // jit_array_reserve_local(arr_bits, min_capacity) -> u64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        let func_id = module
            .declare_function("jit_array_reserve_local", Linkage::Import, &sig)
            .expect("Failed to declare jit_array_reserve_local");
        ffi_funcs.insert("jit_array_reserve_local".to_string(), func_id);
    }

    // jit_array_zip(arr1_bits, arr2_bits) -> u64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        let func_id = module
            .declare_function("jit_array_zip", Linkage::Import, &sig)
            .expect("Failed to declare jit_array_zip");
        ffi_funcs.insert("jit_array_zip".to_string(), func_id);
    }

    // Array accessor functions: jit_array_first, jit_array_last, jit_array_min, jit_array_max
    for name in [
        "jit_array_first",
        "jit_array_last",
        "jit_array_min",
        "jit_array_max",
    ] {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        let func_id = module
            .declare_function(name, Linkage::Import, &sig)
            .unwrap_or_else(|_| panic!("Failed to declare {}", name));
        ffi_funcs.insert(name.to_string(), func_id);
    }

    // jit_slice(arr_bits, start_bits, end_bits) -> u64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // arr_bits
        sig.params.push(AbiParam::new(types::I64)); // start_bits
        sig.params.push(AbiParam::new(types::I64)); // end_bits
        sig.returns.push(AbiParam::new(types::I64)); // result
        let func_id = module
            .declare_function("jit_slice", Linkage::Import, &sig)
            .expect("Failed to declare jit_slice");
        ffi_funcs.insert("jit_slice".to_string(), func_id);
    }

    // jit_range(start_bits, end_bits) -> u64 (creates array from range)
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // start_bits
        sig.params.push(AbiParam::new(types::I64)); // end_bits
        sig.returns.push(AbiParam::new(types::I64)); // result
        let func_id = module
            .declare_function("jit_range", Linkage::Import, &sig)
            .expect("Failed to declare jit_range");
        ffi_funcs.insert("jit_range".to_string(), func_id);
    }

    // jit_make_range(start_bits, end_bits) -> u64 (creates Range object)
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // start_bits
        sig.params.push(AbiParam::new(types::I64)); // end_bits
        sig.returns.push(AbiParam::new(types::I64)); // result
        let func_id = module
            .declare_function("jit_make_range", Linkage::Import, &sig)
            .expect("Failed to declare jit_make_range");
        ffi_funcs.insert("jit_make_range".to_string(), func_id);
    }

    // jit_array_filled(size_bits: u64, value_bits: u64) -> u64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // size_bits
        sig.params.push(AbiParam::new(types::I64)); // value_bits
        sig.returns.push(AbiParam::new(types::I64)); // result
        let func_id = module
            .declare_function("jit_array_filled", Linkage::Import, &sig)
            .expect("Failed to declare jit_array_filled");
        ffi_funcs.insert("jit_array_filled".to_string(), func_id);
    }

    // jit_array_reverse(arr_bits: u64) -> u64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        let func_id = module
            .declare_function("jit_array_reverse", Linkage::Import, &sig)
            .expect("Failed to declare jit_array_reverse");
        ffi_funcs.insert("jit_array_reverse".to_string(), func_id);
    }

    // jit_array_push_element(arr_bits: u64, element_bits: u64) -> u64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        let func_id = module
            .declare_function("jit_array_push_element", Linkage::Import, &sig)
            .expect("Failed to declare jit_array_push_element");
        ffi_funcs.insert("jit_array_push_element".to_string(), func_id);
    }

    // jit_hof_array_alloc(capacity: u64) -> u64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // capacity
        sig.returns.push(AbiParam::new(types::I64)); // result (NaN-boxed array)
        let func_id = module
            .declare_function("jit_hof_array_alloc", Linkage::Import, &sig)
            .expect("Failed to declare jit_hof_array_alloc");
        ffi_funcs.insert("jit_hof_array_alloc".to_string(), func_id);
    }

    // jit_hof_array_push(arr_bits: u64, value_bits: u64) -> u64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // arr_bits
        sig.params.push(AbiParam::new(types::I64)); // value_bits
        sig.returns.push(AbiParam::new(types::I64)); // result (same arr_bits)
        let func_id = module
            .declare_function("jit_hof_array_push", Linkage::Import, &sig)
            .expect("Failed to declare jit_hof_array_push");
        ffi_funcs.insert("jit_hof_array_push".to_string(), func_id);
    }

    // jit_array_info(array_bits: u64) -> ArrayInfo { data_ptr: u64, length: u64 }
    // Returns #[repr(C)] struct with two u64 fields via multiple return values
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // array_bits
        sig.returns.push(AbiParam::new(types::I64)); // data_ptr
        sig.returns.push(AbiParam::new(types::I64)); // length
        let func_id = module
            .declare_function("jit_array_info", Linkage::Import, &sig)
            .expect("Failed to declare jit_array_info");
        ffi_funcs.insert("jit_array_info".to_string(), func_id);
    }
}
