//! FFI function references for Cranelift codegen.
//!
//! This struct bundles the native-typed FFI entry points that the JIT
//! compiler actually references during codegen. Historically it carried ~240
//! `FuncRef` fields covering every legacy NaN-boxed helper; the V6 cleanup
//! (part of the v2 spec alignment) pruned the dead weight so only the
//! v2-native entry points remain.
//!
//! R7.1 deleted 11 `generic_*` dispatch-fallback fields (48 → 37).
//! R7.2 consolidated 4 typed-array push helpers into 1 (37 → 34).
//! R7.3 audited every remaining field: all 34 have ≥1 live caller in the
//! MIR lowering path (see `mir_compiler/{statements,terminators,v2_array,
//! v2_typed_map}.rs`). No further trimming is justified without
//! consolidating caller-side dispatch, which is out of scope for R7.
//!
//! Steady-state FuncRef count: 34. Further reduction would require FFI
//! consolidation work beyond the R7 audit's mandate.
//!
//! New FFI helpers should be registered here AND in
//! `crates/shape-jit/src/ffi_symbols/` (declare + register), and then the
//! `FFIFuncRefs` builder in `crates/shape-jit/src/compiler/ffi_builder.rs`
//! should populate the field.

use cranelift::codegen::ir::FuncRef;

/// Bundle of Cranelift `FuncRef` handles for native-typed FFI calls used by
/// the v2 JIT codegen pipeline.
pub struct FFIFuncRefs {
    // Object / property access
    pub(crate) get_prop: FuncRef,
    pub(crate) set_prop: FuncRef,

    // Call dispatch (value/method path — the other foreign-call variants were
    // retired with the legacy NaN-boxed dispatch helpers).
    pub(crate) call_value: FuncRef,
    pub(crate) call_method: FuncRef,

    // Array allocator + hot per-element push used by v2 lowerings.
    pub(crate) new_array: FuncRef,
    pub(crate) array_push_elem: FuncRef,

    // Builtin print fallback (used by emit_print).
    pub(crate) print: FuncRef,

    // Closure construction (Phase H2: typed closure block → Arc<Closure>).
    pub(crate) make_closure: FuncRef,
    pub(crate) finalize_heap_closure: FuncRef,
    // Track A.1D: OwnedMutable capture cell allocator. Called from
    // `emit_heap_closure` once per `CaptureKind::OwnedMutable` capture to
    // obtain the `*mut ValueWord` pointer installed into the Ptr slot.
    pub(crate) alloc_owned_mut_cell: FuncRef,

    // TypedObject allocation + field store (used by struct lowering).
    pub(crate) typed_object_alloc: FuncRef,
    pub(crate) typed_object_set_field: FuncRef,

    // Arc refcount primitives (used by ownership-aware JIT paths).
    pub(crate) arc_retain: FuncRef,
    pub(crate) arc_release: FuncRef,

    // v2 typed-array allocators (used by v2 lowerings).
    pub(crate) v2_array_new_f64: FuncRef,
    pub(crate) v2_array_new_i64: FuncRef,
    pub(crate) v2_array_new_i32: FuncRef,
    pub(crate) v2_array_new_bool: FuncRef,

    // v2 typed-array element push — single generic helper that dispatches
    // on the `elem_size` byte immediate. Callers zero/sign-extend the native
    // value to I64 before the call; the FFI body routes to the matching
    // TypedArray::push instantiation. (Get/set/len remain inlined in
    // Cranelift directly against the native buffer layout.)
    pub(crate) v2_array_push: FuncRef,

    // v2 struct allocator.
    pub(crate) v2_alloc_struct: FuncRef,

    // v2 SIMD reductions (f64/i64 sum/min/max/mean/sum-of-squares).
    pub(crate) v2_array_sum_f64: FuncRef,
    pub(crate) v2_array_sum_i64: FuncRef,
    pub(crate) v2_array_min_f64: FuncRef,
    pub(crate) v2_array_max_f64: FuncRef,
    pub(crate) v2_array_mean_f64: FuncRef,
    pub(crate) v2_array_sum_squares_f64: FuncRef,

    // v2 SIMD element-wise scalar ops (allocating, f64).
    pub(crate) v2_array_scale_f64: FuncRef,
    pub(crate) v2_array_add_scalar_f64: FuncRef,

    // v2 SIMD element-wise binary ops (allocating, f64).
    pub(crate) v2_array_add_f64: FuncRef,
    pub(crate) v2_array_mul_f64: FuncRef,

    // v2 typed HashMap<string, ...> access.
    pub(crate) v2_map_get_str_i64: FuncRef,
    pub(crate) v2_map_get_str_f64: FuncRef,
    pub(crate) v2_map_has_str: FuncRef,
    pub(crate) v2_map_set_str_i64: FuncRef,
    pub(crate) v2_map_len: FuncRef,
}
