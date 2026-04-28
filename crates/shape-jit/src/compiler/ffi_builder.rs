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
            // Track A.1D: OwnedMutable capture cell allocator.
            alloc_owned_mut_cell: r!("jit_alloc_owned_mut_cell"),
            // Track A.1E: Shared capture FFI helpers.
            arc_shared_retain: r!("jit_arc_shared_retain"),
            shared_lock_contended: r!("jit_shared_lock_contended"),
            shared_unlock_contended: r!("jit_shared_unlock_contended"),

            // Session 1 Commit 3: outer-scope Shared-cell lifecycle.
            alloc_shared_cell: r!("jit_alloc_shared_cell"),
            arc_shared_release: r!("jit_arc_shared_release"),

            // Wave C.1: per-FieldKind closure-cell FFI helpers (D1 ABI).
            // OwnedMutable allocators
            alloc_owned_mut_cell_i64: r!("jit_alloc_owned_mut_cell_i64"),
            alloc_owned_mut_cell_u64: r!("jit_alloc_owned_mut_cell_u64"),
            alloc_owned_mut_cell_f64: r!("jit_alloc_owned_mut_cell_f64"),
            alloc_owned_mut_cell_i32: r!("jit_alloc_owned_mut_cell_i32"),
            alloc_owned_mut_cell_u32: r!("jit_alloc_owned_mut_cell_u32"),
            alloc_owned_mut_cell_i16: r!("jit_alloc_owned_mut_cell_i16"),
            alloc_owned_mut_cell_u16: r!("jit_alloc_owned_mut_cell_u16"),
            alloc_owned_mut_cell_i8: r!("jit_alloc_owned_mut_cell_i8"),
            alloc_owned_mut_cell_u8: r!("jit_alloc_owned_mut_cell_u8"),
            alloc_owned_mut_cell_bool: r!("jit_alloc_owned_mut_cell_bool"),
            alloc_owned_mut_cell_ptr: r!("jit_alloc_owned_mut_cell_ptr"),
            // OwnedMutable readers
            read_owned_mut_cell_i64: r!("jit_read_owned_mut_cell_i64"),
            read_owned_mut_cell_u64: r!("jit_read_owned_mut_cell_u64"),
            read_owned_mut_cell_f64: r!("jit_read_owned_mut_cell_f64"),
            read_owned_mut_cell_i32: r!("jit_read_owned_mut_cell_i32"),
            read_owned_mut_cell_u32: r!("jit_read_owned_mut_cell_u32"),
            read_owned_mut_cell_i16: r!("jit_read_owned_mut_cell_i16"),
            read_owned_mut_cell_u16: r!("jit_read_owned_mut_cell_u16"),
            read_owned_mut_cell_i8: r!("jit_read_owned_mut_cell_i8"),
            read_owned_mut_cell_u8: r!("jit_read_owned_mut_cell_u8"),
            read_owned_mut_cell_bool: r!("jit_read_owned_mut_cell_bool"),
            read_owned_mut_cell_ptr: r!("jit_read_owned_mut_cell_ptr"),
            // OwnedMutable writers
            write_owned_mut_cell_i64: r!("jit_write_owned_mut_cell_i64"),
            write_owned_mut_cell_u64: r!("jit_write_owned_mut_cell_u64"),
            write_owned_mut_cell_f64: r!("jit_write_owned_mut_cell_f64"),
            write_owned_mut_cell_i32: r!("jit_write_owned_mut_cell_i32"),
            write_owned_mut_cell_u32: r!("jit_write_owned_mut_cell_u32"),
            write_owned_mut_cell_i16: r!("jit_write_owned_mut_cell_i16"),
            write_owned_mut_cell_u16: r!("jit_write_owned_mut_cell_u16"),
            write_owned_mut_cell_i8: r!("jit_write_owned_mut_cell_i8"),
            write_owned_mut_cell_u8: r!("jit_write_owned_mut_cell_u8"),
            write_owned_mut_cell_bool: r!("jit_write_owned_mut_cell_bool"),
            write_owned_mut_cell_ptr: r!("jit_write_owned_mut_cell_ptr"),
            // Shared readers
            read_shared_cell_i64: r!("jit_read_shared_cell_i64"),
            read_shared_cell_u64: r!("jit_read_shared_cell_u64"),
            read_shared_cell_f64: r!("jit_read_shared_cell_f64"),
            read_shared_cell_i32: r!("jit_read_shared_cell_i32"),
            read_shared_cell_u32: r!("jit_read_shared_cell_u32"),
            read_shared_cell_i16: r!("jit_read_shared_cell_i16"),
            read_shared_cell_u16: r!("jit_read_shared_cell_u16"),
            read_shared_cell_i8: r!("jit_read_shared_cell_i8"),
            read_shared_cell_u8: r!("jit_read_shared_cell_u8"),
            read_shared_cell_bool: r!("jit_read_shared_cell_bool"),
            read_shared_cell_ptr: r!("jit_read_shared_cell_ptr"),
            // Shared writers
            write_shared_cell_i64: r!("jit_write_shared_cell_i64"),
            write_shared_cell_u64: r!("jit_write_shared_cell_u64"),
            write_shared_cell_f64: r!("jit_write_shared_cell_f64"),
            write_shared_cell_i32: r!("jit_write_shared_cell_i32"),
            write_shared_cell_u32: r!("jit_write_shared_cell_u32"),
            write_shared_cell_i16: r!("jit_write_shared_cell_i16"),
            write_shared_cell_u16: r!("jit_write_shared_cell_u16"),
            write_shared_cell_i8: r!("jit_write_shared_cell_i8"),
            write_shared_cell_u8: r!("jit_write_shared_cell_u8"),
            write_shared_cell_bool: r!("jit_write_shared_cell_bool"),
            write_shared_cell_ptr: r!("jit_write_shared_cell_ptr"),

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

            // v2 typed-array push (generic dispatcher — see ffi_refs.rs)
            v2_array_push: r!("jit_v2_array_push"),

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

            // F5.a/F5.b: string `+` FFI (used by MIR BinaryOp::Add on
            // SlotKind::String operands, incl. f-string desugared chains).
            string_concat: r!("jit_string_concat"),

            // v2 typed HashMap<string, ...>
            v2_map_get_str_i64: r!("jit_v2_map_get_str_i64"),
            v2_map_get_str_f64: r!("jit_v2_map_get_str_f64"),
            v2_map_has_str: r!("jit_v2_map_has_str"),
            v2_map_set_str_i64: r!("jit_v2_map_set_str_i64"),
            v2_map_len: r!("jit_v2_map_len"),
        }
    }
}
