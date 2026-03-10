//! FFI function reference building

use super::setup::JITCompiler;
use crate::translator::FFIFuncRefs;
use cranelift::prelude::*;
use cranelift_module::Module;

impl JITCompiler {
    #[inline(always)]
    pub(super) fn build_ffi_refs(&mut self, builder: &mut FunctionBuilder) -> FFIFuncRefs {
        FFIFuncRefs {
            new_array: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_new_array"], builder.func),
            new_object: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_new_object"], builder.func),
            get_prop: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_get_prop"], builder.func),
            set_prop: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_set_prop"], builder.func),
            length: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_length"], builder.func),
            array_get: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_array_get"], builder.func),
            call_function: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_call_function"], builder.func),
            call_value: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_call_value"], builder.func),
            call_foreign: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_call_foreign"], builder.func),
            call_foreign_native: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_call_foreign_native"], builder.func),
            call_foreign_dynamic: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_call_foreign_dynamic"], builder.func),
            call_foreign_native_0: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_call_foreign_native_0"], builder.func),
            call_foreign_native_1: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_call_foreign_native_1"], builder.func),
            call_foreign_native_2: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_call_foreign_native_2"], builder.func),
            call_foreign_native_3: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_call_foreign_native_3"], builder.func),
            call_foreign_native_4: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_call_foreign_native_4"], builder.func),
            call_foreign_native_5: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_call_foreign_native_5"], builder.func),
            call_foreign_native_6: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_call_foreign_native_6"], builder.func),
            call_foreign_native_7: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_call_foreign_native_7"], builder.func),
            call_foreign_native_8: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_call_foreign_native_8"], builder.func),
            iter_next: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_iter_next"], builder.func),
            iter_done: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_iter_done"], builder.func),
            call_method: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_call_method"], builder.func),
            type_of: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_typeof"], builder.func),
            type_check: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_type_check"], builder.func),
            make_ok: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_make_ok"], builder.func),
            make_err: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_make_err"], builder.func),
            is_ok: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_is_ok"], builder.func),
            is_err: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_is_err"], builder.func),
            is_result: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_is_result"], builder.func),
            unwrap_ok: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_unwrap_ok"], builder.func),
            unwrap_err: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_unwrap_err"], builder.func),
            unwrap_or: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_unwrap_or"], builder.func),
            result_inner: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_result_inner"], builder.func),
            make_some: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_make_some"], builder.func),
            is_some: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_is_some"], builder.func),
            is_none: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_is_none"], builder.func),
            unwrap_some: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_unwrap_some"], builder.func),
            array_first: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_array_first"], builder.func),
            array_last: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_array_last"], builder.func),
            array_min: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_array_min"], builder.func),
            array_max: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_array_max"], builder.func),
            slice: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_slice"], builder.func),
            range: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_range"], builder.func),
            make_range: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_make_range"], builder.func),
            to_string: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_to_string"], builder.func),
            to_number: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_to_number"], builder.func),
            print: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_print"], builder.func),
            sin: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_sin"], builder.func),
            cos: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_cos"], builder.func),
            tan: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_tan"], builder.func),
            asin: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_asin"], builder.func),
            acos: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_acos"], builder.func),
            atan: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_atan"], builder.func),
            exp: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_exp"], builder.func),
            ln: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_ln"], builder.func),
            log: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_log"], builder.func),
            pow: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_pow"], builder.func),
            control_fold: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_control_fold"], builder.func),
            control_reduce: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_control_reduce"], builder.func),
            control_map: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_control_map"], builder.func),
            control_filter: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_control_filter"], builder.func),
            control_foreach: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_control_foreach"], builder.func),
            control_find: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_control_find"], builder.func),
            control_find_index: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_control_find_index"], builder.func),
            control_some: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_control_some"], builder.func),
            control_every: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_control_every"], builder.func),
            array_push: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_array_push"], builder.func),
            array_pop: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_array_pop"], builder.func),
            array_push_elem: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_array_push_elem"], builder.func),
            array_push_local: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_array_push_local"], builder.func),
            array_reserve_local: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_array_reserve_local"], builder.func),
            array_zip: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_array_zip"], builder.func),
            array_filled: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_array_filled"], builder.func),
            array_reverse: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_array_reverse"], builder.func),
            array_push_element: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_array_push_element"], builder.func),
            make_closure: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_make_closure"], builder.func),
            eval_datetime_expr: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_eval_datetime_expr"], builder.func),
            eval_time_reference: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_eval_time_reference"], builder.func),
            eval_data_datetime_ref: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_eval_data_datetime_ref"], builder.func),
            eval_data_relative: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_eval_data_relative"], builder.func),
            intrinsic_series: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_intrinsic_series"], builder.func),
            series_method: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_series_method"], builder.func),
            format_error: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_format_error"], builder.func),
            object_rest: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_object_rest"], builder.func),
            format: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_format"], builder.func),
            generic_add: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_generic_add"], builder.func),
            generic_sub: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_generic_sub"], builder.func),
            generic_mul: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_generic_mul"], builder.func),
            generic_div: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_generic_div"], builder.func),
            generic_eq: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_generic_eq"], builder.func),
            generic_neq: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_generic_neq"], builder.func),
            series_shift: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_series_shift"], builder.func),
            series_fillna: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_series_fillna"], builder.func),
            series_rolling_mean: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_series_rolling_mean"], builder.func),
            series_rolling_sum: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_series_rolling_sum"], builder.func),
            series_rolling_std: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_series_rolling_std"], builder.func),
            intrinsic_rolling_std: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_intrinsic_rolling_std"], builder.func),
            series_cumsum: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_series_cumsum"], builder.func),
            series_gt: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_series_gt"], builder.func),
            series_lt: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_series_lt"], builder.func),
            series_gte: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_series_gte"], builder.func),
            series_lte: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_series_lte"], builder.func),
            intrinsic_sum: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_intrinsic_sum"], builder.func),
            intrinsic_mean: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_intrinsic_mean"], builder.func),
            intrinsic_min: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_intrinsic_min"], builder.func),
            intrinsic_max: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_intrinsic_max"], builder.func),
            intrinsic_std: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_intrinsic_std"], builder.func),
            intrinsic_variance: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_intrinsic_variance"], builder.func),
            intrinsic_median: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_intrinsic_median"], builder.func),
            intrinsic_percentile: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_intrinsic_percentile"], builder.func),
            intrinsic_correlation: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_intrinsic_correlation"], builder.func),
            intrinsic_covariance: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_intrinsic_covariance"], builder.func),
            series_rolling_min: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_series_rolling_min"], builder.func),
            series_rolling_max: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_series_rolling_max"], builder.func),
            series_ema: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_series_ema"], builder.func),
            series_diff: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_series_diff"], builder.func),
            series_pct_change: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_series_pct_change"], builder.func),
            series_cumprod: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_series_cumprod"], builder.func),
            series_clip: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_series_clip"], builder.func),
            series_broadcast: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_series_broadcast"], builder.func),
            series_highest_index: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_series_highest_index"], builder.func),
            series_lowest_index: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_series_lowest_index"], builder.func),
            time_current_time: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_time_current_time"], builder.func),
            time_symbol: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_time_symbol"], builder.func),
            time_last_row: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_time_last_row"], builder.func),
            time_range: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_time_range"], builder.func),
            get_all_rows: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_get_all_rows"], builder.func),
            align_series: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_align_series"], builder.func),
            run_simulation: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_run_simulation"], builder.func),
            intrinsic_vec_abs: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_intrinsic_vec_abs"], builder.func),
            intrinsic_vec_sqrt: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_intrinsic_vec_sqrt"], builder.func),
            intrinsic_vec_ln: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_intrinsic_vec_ln"], builder.func),
            intrinsic_vec_exp: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_intrinsic_vec_exp"], builder.func),
            intrinsic_vec_add: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_intrinsic_vec_add"], builder.func),
            intrinsic_vec_sub: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_intrinsic_vec_sub"], builder.func),
            intrinsic_vec_mul: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_intrinsic_vec_mul"], builder.func),
            intrinsic_vec_div: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_intrinsic_vec_div"], builder.func),
            intrinsic_vec_max: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_intrinsic_vec_max"], builder.func),
            intrinsic_vec_min: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_intrinsic_vec_min"], builder.func),
            intrinsic_matmul_vec: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_intrinsic_matmul_vec"], builder.func),
            intrinsic_matmul_mat: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_intrinsic_matmul_mat"], builder.func),
            // Raw pointer SIMD operations
            simd_add: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_simd_add"], builder.func),
            simd_sub: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_simd_sub"], builder.func),
            simd_mul: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_simd_mul"], builder.func),
            simd_div: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_simd_div"], builder.func),
            simd_max: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_simd_max"], builder.func),
            simd_min: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_simd_min"], builder.func),
            simd_add_scalar: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_simd_add_scalar"], builder.func),
            simd_sub_scalar: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_simd_sub_scalar"], builder.func),
            simd_mul_scalar: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_simd_mul_scalar"], builder.func),
            simd_div_scalar: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_simd_div_scalar"], builder.func),
            simd_gt: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_simd_gt"], builder.func),
            simd_lt: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_simd_lt"], builder.func),
            simd_gte: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_simd_gte"], builder.func),
            simd_lte: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_simd_lte"], builder.func),
            simd_eq: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_simd_eq"], builder.func),
            simd_neq: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_simd_neq"], builder.func),
            simd_free: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_simd_free"], builder.func),
            get_field: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_get_field"], builder.func),
            get_row_ref: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_get_row_ref"], builder.func),
            row_get_field: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_row_get_field"], builder.func),
            get_row_timestamp: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_get_row_timestamp"], builder.func),
            get_field_typed: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_get_field_typed"], builder.func),
            set_field_typed: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_set_field_typed"], builder.func),
            typed_object_alloc: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_typed_object_alloc"], builder.func),
            typed_merge_object: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_typed_merge_object"], builder.func),
            load_col_f64: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_load_col_f64"], builder.func),
            load_col_i64: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_load_col_i64"], builder.func),
            load_col_bool: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_load_col_bool"], builder.func),
            load_col_str: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_load_col_str"], builder.func),
            gc_safepoint: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_gc_safepoint"], builder.func),
            set_index_ref: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_set_index_ref"], builder.func),
            array_info: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_array_info"], builder.func),
            hof_array_alloc: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_hof_array_alloc"], builder.func),
            hof_array_push: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_hof_array_push"], builder.func),
            spawn_task: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_spawn_task"], builder.func),
            join_init: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_join_init"], builder.func),
            join_await: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_join_await"], builder.func),
            cancel_task: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_cancel_task"], builder.func),
            async_scope_enter: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_async_scope_enter"], builder.func),
            async_scope_exit: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_async_scope_exit"], builder.func),
            // Shape guard operations (HashMap hidden class)
            hashmap_shape_id: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_hashmap_shape_id"], builder.func),
            hashmap_value_at: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_hashmap_value_at"], builder.func),
            // Generic builtin trampoline
            generic_builtin: self
                .module
                .declare_func_in_func(self.ffi_funcs["jit_generic_builtin"], builder.func),
        }
    }
}
