//! FFI function reference building.
//!
//! Populates `FFIFuncRefs` with Cranelift `FuncRef` handles for the subset
//! of FFI entry points the v2 JIT codegen pipeline actually references.
//! Historically this builder declared ~240 functions; the V6 cleanup pruned
//! it to the live native-typed surface.

use super::setup::JITCompiler;
use crate::ffi_refs::FFIFuncRefs;
use cranelift::prelude::*;
use cranelift_module::Module;

impl JITCompiler {
    #[inline(always)]
    pub(super) fn build_ffi_refs(&mut self, builder: &mut FunctionBuilder) -> FFIFuncRefs {
        // Helper closure-style shorthand: declare an already-registered FFI
        // function (via `ffi_funcs[key]`) into the current Cranelift func.
        macro_rules! r {
            ($key:expr) => {
                self.module
                    .declare_func_in_func(self.ffi_funcs[$key], builder.func)
            };
        }

        FFIFuncRefs {
            // Object / property access
            get_prop: r!("jit_get_prop"),
            set_prop: r!("jit_set_prop"),

            // Call dispatch
            call_value: r!("jit_call_value"),
            call_method: r!("jit_call_method"),

            // Arrays
            new_array: r!("jit_new_array"),
            array_push_elem: r!("jit_array_push_elem"),

            // Print fallback
            print: r!("jit_print"),

            // Closure construction
            make_closure: r!("jit_make_closure"),
            finalize_heap_closure: r!("jit_finalize_heap_closure"),

            // Generic arithmetic / comparison trampolines (minimal dynamic
            // dispatch surface retained after the V3/V4 cleanup).
            generic_add: r!("jit_generic_add"),
            generic_sub: r!("jit_generic_sub"),
            generic_mul: r!("jit_generic_mul"),
            generic_div: r!("jit_generic_div"),
            generic_mod: r!("jit_generic_mod"),
            generic_eq: r!("jit_generic_eq"),
            generic_neq: r!("jit_generic_neq"),
            generic_lt: r!("jit_generic_lt"),
            generic_le: r!("jit_generic_le"),
            generic_gt: r!("jit_generic_gt"),
            generic_ge: r!("jit_generic_ge"),

            // TypedObject allocation + field store
            typed_object_alloc: r!("jit_typed_object_alloc"),
            typed_object_set_field: r!("jit_typed_object_set_field"),

            // Arc refcount primitives
            arc_retain: r!("jit_arc_retain"),
            arc_release: r!("jit_arc_release"),

            // v2 typed-array allocators
            v2_array_new_f64: r!("jit_v2_array_new_f64"),
            v2_array_new_i64: r!("jit_v2_array_new_i64"),
            v2_array_new_i32: r!("jit_v2_array_new_i32"),
            v2_array_new_bool: r!("jit_v2_array_new_bool"),

            // v2 typed-array push
            v2_array_push_f64: r!("jit_v2_array_push_f64"),
            v2_array_push_i64: r!("jit_v2_array_push_i64"),
            v2_array_push_i32: r!("jit_v2_array_push_i32"),
            v2_array_push_bool: r!("jit_v2_array_push_bool"),

            // v2 struct allocator
            v2_alloc_struct: r!("jit_v2_alloc_struct"),

            // v2 SIMD reductions
            v2_array_sum_f64: r!("jit_v2_array_sum_f64"),
            v2_array_sum_i64: r!("jit_v2_array_sum_i64"),
            v2_array_min_f64: r!("jit_v2_array_min_f64"),
            v2_array_max_f64: r!("jit_v2_array_max_f64"),
            v2_array_mean_f64: r!("jit_v2_array_mean_f64"),
            v2_array_sum_squares_f64: r!("jit_v2_array_sum_squares_f64"),

            // v2 SIMD scalar element-wise ops
            v2_array_scale_f64: r!("jit_v2_array_scale_f64"),
            v2_array_add_scalar_f64: r!("jit_v2_array_add_scalar_f64"),

            // v2 SIMD binary element-wise ops
            v2_array_add_f64: r!("jit_v2_array_add_f64"),
            v2_array_mul_f64: r!("jit_v2_array_mul_f64"),

            // v2 typed HashMap<string, ...>
            v2_map_get_str_i64: r!("jit_v2_map_get_str_i64"),
            v2_map_get_str_f64: r!("jit_v2_map_get_str_f64"),
            v2_map_has_str: r!("jit_v2_map_has_str"),
            v2_map_set_str_i64: r!("jit_v2_map_set_str_i64"),
            v2_map_len: r!("jit_v2_map_len"),
        }
    }
}
