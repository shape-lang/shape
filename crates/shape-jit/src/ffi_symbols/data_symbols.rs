//! Data Access FFI Symbol Registration
//!
//! This module handles registration and declaration of data access FFI symbols
//! for the JIT compiler, including series operations, data field access,
//! time functions, and simulation.

use cranelift::prelude::*;
use cranelift_jit::JITBuilder;
use cranelift_jit::JITModule;
use cranelift_module::{FuncId, Linkage, Module};
use std::collections::HashMap;

use super::super::ffi::data::{
    jit_eval_data_datetime_ref, jit_eval_data_relative, jit_get_field, jit_get_field_typed,
    jit_get_row_ref, jit_get_row_timestamp, jit_load_col_bool, jit_load_col_f64, jit_load_col_i64,
    jit_load_col_str, jit_row_get_field, jit_set_field_typed,
};
use super::data_access::{jit_align_series, jit_get_all_rows};
use super::helpers::{
    jit_eval_datetime_expr, jit_eval_time_reference, jit_intrinsic_series, jit_series_method,
};
use super::intrinsics::{
    jit_time_current_time, jit_time_last_row, jit_time_range, jit_time_symbol,
};
use super::series::{
    jit_intrinsic_rolling_std, jit_series_cumsum, jit_series_fillna, jit_series_rolling_mean,
    jit_series_rolling_std, jit_series_rolling_sum, jit_series_shift,
};
use super::simulation::jit_run_simulation;

/// Register data access FFI symbols with the JIT builder
pub fn register_data_symbols(builder: &mut JITBuilder) {
    // Generic DataFrame access (industry-agnostic)
    builder.symbol("jit_get_field", jit_get_field as *const u8);
    builder.symbol("jit_get_row_ref", jit_get_row_ref as *const u8);
    builder.symbol("jit_row_get_field", jit_row_get_field as *const u8);
    builder.symbol("jit_get_row_timestamp", jit_get_row_timestamp as *const u8);
    builder.symbol(
        "jit_eval_data_datetime_ref",
        jit_eval_data_datetime_ref as *const u8,
    );
    builder.symbol(
        "jit_eval_data_relative",
        jit_eval_data_relative as *const u8,
    );

    // Type-specialized field access (JIT optimization)
    builder.symbol("jit_get_field_typed", jit_get_field_typed as *const u8);
    builder.symbol("jit_set_field_typed", jit_set_field_typed as *const u8);

    // Typed column access (Arrow-backed LoadCol* opcodes)
    builder.symbol("jit_load_col_f64", jit_load_col_f64 as *const u8);
    builder.symbol("jit_load_col_i64", jit_load_col_i64 as *const u8);
    builder.symbol("jit_load_col_bool", jit_load_col_bool as *const u8);
    builder.symbol("jit_load_col_str", jit_load_col_str as *const u8);

    // Data access operations
    builder.symbol("jit_get_all_rows", jit_get_all_rows as *const u8);
    builder.symbol("jit_align_series", jit_align_series as *const u8);

    builder.symbol("jit_series_shift", jit_series_shift as *const u8);
    builder.symbol("jit_series_fillna", jit_series_fillna as *const u8);
    builder.symbol(
        "jit_series_rolling_mean",
        jit_series_rolling_mean as *const u8,
    );
    builder.symbol(
        "jit_series_rolling_sum",
        jit_series_rolling_sum as *const u8,
    );
    builder.symbol(
        "jit_series_rolling_std",
        jit_series_rolling_std as *const u8,
    );
    builder.symbol(
        "jit_intrinsic_rolling_std",
        jit_intrinsic_rolling_std as *const u8,
    );
    builder.symbol("jit_series_cumsum", jit_series_cumsum as *const u8);

    // Time functions
    builder.symbol("jit_time_current_time", jit_time_current_time as *const u8);
    builder.symbol("jit_time_symbol", jit_time_symbol as *const u8);
    builder.symbol("jit_time_last_row", jit_time_last_row as *const u8);
    builder.symbol("jit_time_range", jit_time_range as *const u8);

    // Helper functions
    builder.symbol("jit_intrinsic_series", jit_intrinsic_series as *const u8);
    builder.symbol("jit_series_method", jit_series_method as *const u8);
    builder.symbol(
        "jit_eval_datetime_expr",
        jit_eval_datetime_expr as *const u8,
    );
    builder.symbol(
        "jit_eval_time_reference",
        jit_eval_time_reference as *const u8,
    );

    // Simulation
    builder.symbol("jit_run_simulation", jit_run_simulation as *const u8);
}

/// Declare data access FFI function signatures in the module
pub fn declare_data_functions(module: &mut JITModule, ffi_funcs: &mut HashMap<String, FuncId>) {
    // jit_get_field(ctx, row_offset, column_index) -> u64 (boxed f64)
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ctx
        sig.params.push(AbiParam::new(types::I32)); // row_offset
        sig.params.push(AbiParam::new(types::I32)); // column_index
        sig.returns.push(AbiParam::new(types::I64)); // result (NaN-boxed f64)
        let func_id = module
            .declare_function("jit_get_field", Linkage::Import, &sig)
            .expect("Failed to declare jit_get_field");
        ffi_funcs.insert("jit_get_field".to_string(), func_id);
    }

    // jit_get_row_ref(ctx, row_offset) -> u64 (TAG_INT with row index)
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ctx
        sig.params.push(AbiParam::new(types::I32)); // row_offset
        sig.returns.push(AbiParam::new(types::I64)); // result (TAG_INT)
        let func_id = module
            .declare_function("jit_get_row_ref", Linkage::Import, &sig)
            .expect("Failed to declare jit_get_row_ref");
        ffi_funcs.insert("jit_get_row_ref".to_string(), func_id);
    }

    // jit_row_get_field(ctx, row_ref, column_index) -> u64 (boxed f64)
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ctx
        sig.params.push(AbiParam::new(types::I64)); // row_ref (TAG_INT)
        sig.params.push(AbiParam::new(types::I32)); // column_index
        sig.returns.push(AbiParam::new(types::I64)); // result (NaN-boxed f64)
        let func_id = module
            .declare_function("jit_row_get_field", Linkage::Import, &sig)
            .expect("Failed to declare jit_row_get_field");
        ffi_funcs.insert("jit_row_get_field".to_string(), func_id);
    }

    // jit_get_row_timestamp(ctx, row_offset) -> u64 (TAG_TIME)
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ctx
        sig.params.push(AbiParam::new(types::I32)); // row_offset
        sig.returns.push(AbiParam::new(types::I64)); // result (TAG_TIME)
        let func_id = module
            .declare_function("jit_get_row_timestamp", Linkage::Import, &sig)
            .expect("Failed to declare jit_get_row_timestamp");
        ffi_funcs.insert("jit_get_row_timestamp".to_string(), func_id);
    }

    // jit_get_field_typed(obj, type_id, field_idx, offset) -> u64 (field value)
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // obj (NaN-boxed)
        sig.params.push(AbiParam::new(types::I64)); // type_id
        sig.params.push(AbiParam::new(types::I64)); // field_idx
        sig.params.push(AbiParam::new(types::I64)); // offset
        sig.returns.push(AbiParam::new(types::I64)); // result (NaN-boxed)
        let func_id = module
            .declare_function("jit_get_field_typed", Linkage::Import, &sig)
            .expect("Failed to declare jit_get_field_typed");
        ffi_funcs.insert("jit_get_field_typed".to_string(), func_id);
    }

    // jit_set_field_typed(obj, value, type_id, field_idx, offset) -> u64 (obj)
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // obj (NaN-boxed)
        sig.params.push(AbiParam::new(types::I64)); // value (NaN-boxed)
        sig.params.push(AbiParam::new(types::I64)); // type_id
        sig.params.push(AbiParam::new(types::I64)); // field_idx
        sig.params.push(AbiParam::new(types::I64)); // offset
        sig.returns.push(AbiParam::new(types::I64)); // result (obj)
        let func_id = module
            .declare_function("jit_set_field_typed", Linkage::Import, &sig)
            .expect("Failed to declare jit_set_field_typed");
        ffi_funcs.insert("jit_set_field_typed".to_string(), func_id);
    }

    // Typed column access: jit_load_col_f64/i64/bool/str(ctx, col_id, row_ref) -> u64
    for name in [
        "jit_load_col_f64",
        "jit_load_col_i64",
        "jit_load_col_bool",
        "jit_load_col_str",
    ] {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ctx
        sig.params.push(AbiParam::new(types::I32)); // col_id
        sig.params.push(AbiParam::new(types::I64)); // row_ref (TAG_INT)
        sig.returns.push(AbiParam::new(types::I64)); // result (NaN-boxed)
        let func_id = module
            .declare_function(name, Linkage::Import, &sig)
            .unwrap_or_else(|_| panic!("Failed to declare {}", name));
        ffi_funcs.insert(name.to_string(), func_id);
    }

    // jit_eval_data_datetime_ref(ctx, expr) -> u64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ctx
        sig.params.push(AbiParam::new(types::I64)); // expr
        sig.returns.push(AbiParam::new(types::I64)); // result
        let func_id = module
            .declare_function("jit_eval_data_datetime_ref", Linkage::Import, &sig)
            .expect("Failed to declare jit_eval_data_datetime_ref");
        ffi_funcs.insert("jit_eval_data_datetime_ref".to_string(), func_id);
    }

    // jit_eval_data_relative(ctx, expr, offset) -> u64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ctx
        sig.params.push(AbiParam::new(types::I64)); // expr
        sig.params.push(AbiParam::new(types::I32)); // offset
        sig.returns.push(AbiParam::new(types::I64)); // result
        let func_id = module
            .declare_function("jit_eval_data_relative", Linkage::Import, &sig)
            .expect("Failed to declare jit_eval_data_relative");
        ffi_funcs.insert("jit_eval_data_relative".to_string(), func_id);
    }

    // jit_intrinsic_series(ctx: *mut JITContext, field_name: u64) -> u64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ctx
        sig.params.push(AbiParam::new(types::I64)); // field_name
        sig.returns.push(AbiParam::new(types::I64)); // result
        let func_id = module
            .declare_function("jit_intrinsic_series", Linkage::Import, &sig)
            .expect("Failed to declare jit_intrinsic_series");
        ffi_funcs.insert("jit_intrinsic_series".to_string(), func_id);
    }

    // jit_series_method(ctx: *mut JITContext, arg_count: usize) -> u64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ctx
        sig.params.push(AbiParam::new(types::I64)); // arg_count
        sig.returns.push(AbiParam::new(types::I64)); // result
        let func_id = module
            .declare_function("jit_series_method", Linkage::Import, &sig)
            .expect("Failed to declare jit_series_method");
        ffi_funcs.insert("jit_series_method".to_string(), func_id);
    }

    // jit_eval_datetime_expr(ctx: *mut JITContext, args: *const u64, arg_count: usize) -> u64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ctx
        sig.params.push(AbiParam::new(types::I64)); // args (unused)
        sig.params.push(AbiParam::new(types::I64)); // arg_count
        sig.returns.push(AbiParam::new(types::I64)); // result
        let func_id = module
            .declare_function("jit_eval_datetime_expr", Linkage::Import, &sig)
            .expect("Failed to declare jit_eval_datetime_expr");
        ffi_funcs.insert("jit_eval_datetime_expr".to_string(), func_id);
    }

    // jit_eval_time_reference(ctx: *mut JITContext, time_expr: u64) -> u64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ctx
        sig.params.push(AbiParam::new(types::I64)); // time_expr
        sig.returns.push(AbiParam::new(types::I64)); // result
        let func_id = module
            .declare_function("jit_eval_time_reference", Linkage::Import, &sig)
            .expect("Failed to declare jit_eval_time_reference");
        ffi_funcs.insert("jit_eval_time_reference".to_string(), func_id);
    }

    // Series operation functions (series_bits: u64, arg: u64) -> u64
    for name in [
        "jit_series_shift",
        "jit_series_fillna",
        "jit_series_rolling_mean",
        "jit_series_rolling_sum",
        "jit_series_rolling_std",
        "jit_intrinsic_rolling_std",
    ] {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // series_bits
        sig.params.push(AbiParam::new(types::I64)); // second arg
        sig.returns.push(AbiParam::new(types::I64)); // result
        let func_id = module
            .declare_function(name, Linkage::Import, &sig)
            .unwrap_or_else(|_| panic!("Failed to declare {}", name));
        ffi_funcs.insert(name.to_string(), func_id);
    }

    // jit_series_cumsum(series_bits: u64) -> u64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // series_bits
        sig.returns.push(AbiParam::new(types::I64)); // result
        let func_id = module
            .declare_function("jit_series_cumsum", Linkage::Import, &sig)
            .expect("Failed to declare jit_series_cumsum");
        ffi_funcs.insert("jit_series_cumsum".to_string(), func_id);
    }

    // jit_time_current_time(ctx: *mut JITContext) -> u64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ctx
        sig.returns.push(AbiParam::new(types::I64)); // result
        let func_id = module
            .declare_function("jit_time_current_time", Linkage::Import, &sig)
            .expect("Failed to declare jit_time_current_time");
        ffi_funcs.insert("jit_time_current_time".to_string(), func_id);
    }

    // jit_time_symbol(ctx: *mut JITContext) -> u64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ctx
        sig.returns.push(AbiParam::new(types::I64)); // result
        let func_id = module
            .declare_function("jit_time_symbol", Linkage::Import, &sig)
            .expect("Failed to declare jit_time_symbol");
        ffi_funcs.insert("jit_time_symbol".to_string(), func_id);
    }

    // jit_time_last_row(ctx: *mut JITContext) -> u64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ctx
        sig.returns.push(AbiParam::new(types::I64)); // result
        let func_id = module
            .declare_function("jit_time_last_row", Linkage::Import, &sig)
            .expect("Failed to declare jit_time_last_row");
        ffi_funcs.insert("jit_time_last_row".to_string(), func_id);
    }

    // jit_time_range(start: u64, end: u64, step: u64) -> u64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // start
        sig.params.push(AbiParam::new(types::I64)); // end
        sig.params.push(AbiParam::new(types::I64)); // step
        sig.returns.push(AbiParam::new(types::I64)); // result
        let func_id = module
            .declare_function("jit_time_range", Linkage::Import, &sig)
            .expect("Failed to declare jit_time_range");
        ffi_funcs.insert("jit_time_range".to_string(), func_id);
    }

    // jit_get_all_rows(ctx: *mut JITContext) -> u64 (data array)
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ctx
        sig.returns.push(AbiParam::new(types::I64)); // result
        let func_id = module
            .declare_function("jit_get_all_rows", Linkage::Import, &sig)
            .expect("Failed to declare jit_get_all_rows");
        ffi_funcs.insert("jit_get_all_rows".to_string(), func_id);
    }

    // jit_align_series(ctx: *mut JITContext, symbols: u64, mode: u64) -> u64 (object)
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ctx
        sig.params.push(AbiParam::new(types::I64)); // symbols
        sig.params.push(AbiParam::new(types::I64)); // mode
        sig.returns.push(AbiParam::new(types::I64)); // result
        let func_id = module
            .declare_function("jit_align_series", Linkage::Import, &sig)
            .expect("Failed to declare jit_align_series");
        ffi_funcs.insert("jit_align_series".to_string(), func_id);
    }

    // jit_run_simulation(ctx, config_bits) -> u64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ctx
        sig.params.push(AbiParam::new(types::I64)); // config_bits
        sig.returns.push(AbiParam::new(types::I64)); // result
        let func_id = module
            .declare_function("jit_run_simulation", Linkage::Import, &sig)
            .expect("Failed to declare jit_run_simulation");
        ffi_funcs.insert("jit_run_simulation".to_string(), func_id);
    }
}
