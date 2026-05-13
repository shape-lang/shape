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
    jit_print, jit_print_bool, jit_print_f64, jit_print_i64, jit_print_option,
    jit_print_result, jit_print_str, jit_print_typed_object, jit_string_concat,
    jit_to_number, jit_to_string, jit_type_check, jit_typeof,
};
#[allow(deprecated)]
use super::super::ffi::object::{
    jit_alloc_owned_mut_cell, jit_alloc_owned_mut_cell_bool, jit_alloc_owned_mut_cell_f64,
    jit_alloc_owned_mut_cell_i8, jit_alloc_owned_mut_cell_i16, jit_alloc_owned_mut_cell_i32,
    jit_alloc_owned_mut_cell_i64, jit_alloc_owned_mut_cell_ptr, jit_alloc_owned_mut_cell_u8,
    jit_alloc_owned_mut_cell_u16, jit_alloc_owned_mut_cell_u32, jit_alloc_owned_mut_cell_u64,
    jit_alloc_shared_cell, jit_arc_shared_release, jit_arc_shared_retain,
    jit_finalize_heap_closure, jit_format, jit_get_prop, jit_hashmap_shape_id,
    jit_hashmap_value_at, jit_length, jit_make_closure, jit_new_object, jit_object_rest,
    jit_read_owned_mut_cell_bool, jit_read_owned_mut_cell_f64, jit_read_owned_mut_cell_i8,
    jit_read_owned_mut_cell_i16, jit_read_owned_mut_cell_i32, jit_read_owned_mut_cell_i64,
    jit_read_owned_mut_cell_ptr, jit_read_owned_mut_cell_u8, jit_read_owned_mut_cell_u16,
    jit_read_owned_mut_cell_u32, jit_read_owned_mut_cell_u64, jit_read_shared_cell_bool,
    jit_read_shared_cell_f64, jit_read_shared_cell_i8, jit_read_shared_cell_i16,
    jit_read_shared_cell_i32, jit_read_shared_cell_i64, jit_read_shared_cell_ptr,
    jit_read_shared_cell_u8, jit_read_shared_cell_u16, jit_read_shared_cell_u32,
    jit_read_shared_cell_u64, jit_set_prop, jit_shared_lock_contended,
    jit_shared_unlock_contended, jit_write_owned_mut_cell_bool, jit_write_owned_mut_cell_f64,
    jit_write_owned_mut_cell_i8, jit_write_owned_mut_cell_i16, jit_write_owned_mut_cell_i32,
    jit_write_owned_mut_cell_i64, jit_write_owned_mut_cell_ptr, jit_write_owned_mut_cell_u8,
    jit_write_owned_mut_cell_u16, jit_write_owned_mut_cell_u32, jit_write_owned_mut_cell_u64,
    jit_write_shared_cell_bool, jit_write_shared_cell_f64, jit_write_shared_cell_i8,
    jit_write_shared_cell_i16, jit_write_shared_cell_i32, jit_write_shared_cell_i64,
    jit_write_shared_cell_ptr, jit_write_shared_cell_u8, jit_write_shared_cell_u16,
    jit_write_shared_cell_u32, jit_write_shared_cell_u64,
};
use super::super::ffi::typed_object::{jit_typed_merge_object, jit_typed_object_alloc};
use super::helpers::jit_format_error;

/// Register object FFI symbols with the JIT builder
#[allow(deprecated)]
pub fn register_object_symbols(builder: &mut JITBuilder) {
    builder.symbol("jit_new_object", jit_new_object as *const u8);
    builder.symbol("jit_get_prop", jit_get_prop as *const u8);
    builder.symbol("jit_set_prop", jit_set_prop as *const u8);
    builder.symbol("jit_length", jit_length as *const u8);
    builder.symbol("jit_typeof", jit_typeof as *const u8);
    builder.symbol("jit_type_check", jit_type_check as *const u8);
    builder.symbol("jit_to_string", jit_to_string as *const u8);
    builder.symbol("jit_to_number", jit_to_number as *const u8);
    // F5.a/F5.b: string `+` for `"a" + "b"` and `f"..."`-desugared concat chains.
    builder.symbol("jit_string_concat", jit_string_concat as *const u8);
    // ADR-006 §2.7.5 — kinded EnumStore producers (W12-jit-aggregate-non-array)
    builder.symbol(
        "jit_make_ok",
        super::super::ffi::result::jit_make_ok as *const u8,
    );
    builder.symbol(
        "jit_make_err",
        super::super::ffi::result::jit_make_err as *const u8,
    );
    builder.symbol(
        "jit_make_some",
        super::super::ffi::result::jit_make_some as *const u8,
    );
    // ADR-006 §2.7.17 / Q18 — Arc-shape Result/Option producers + accessors
    // (W12-jit-result-option-trinity, Phase 3 cluster-0 Round 7A, 2026-05-12).
    // Match the VM-side `BuiltinFunction::OkCtor` / `ErrCtor` / `SomeCtor` /
    // `NoneCtor` output shape — `Arc::into_raw(Arc<ResultData>) as u64` /
    // `Arc::into_raw(Arc<OptionData>) as u64` with kind labels
    // `NativeKind::Ptr(HeapKind::Result)` / `NativeKind::Ptr(HeapKind::Option)`.
    // Replaces the legacy `jit_make_ok` etc. NaN-box producer family at the
    // JIT EnumStore consumer (the production-code consumer migration gap the
    // pre-trinity result.rs:178-200 deletion comment documented).
    builder.symbol(
        "jit_v2_make_result_ok",
        super::super::ffi::result::jit_v2_make_result_ok as *const u8,
    );
    builder.symbol(
        "jit_v2_make_result_err",
        super::super::ffi::result::jit_v2_make_result_err as *const u8,
    );
    builder.symbol(
        "jit_v2_make_option_some",
        super::super::ffi::result::jit_v2_make_option_some as *const u8,
    );
    builder.symbol(
        "jit_v2_make_option_none",
        super::super::ffi::result::jit_v2_make_option_none as *const u8,
    );
    builder.symbol(
        "jit_arc_result_is_ok",
        super::super::ffi::result::jit_arc_result_is_ok as *const u8,
    );
    builder.symbol(
        "jit_arc_result_is_err",
        super::super::ffi::result::jit_arc_result_is_err as *const u8,
    );
    builder.symbol(
        "jit_arc_result_payload",
        super::super::ffi::result::jit_arc_result_payload as *const u8,
    );
    builder.symbol(
        "jit_arc_option_is_some",
        super::super::ffi::result::jit_arc_option_is_some as *const u8,
    );
    builder.symbol(
        "jit_arc_option_is_none",
        super::super::ffi::result::jit_arc_option_is_none as *const u8,
    );
    builder.symbol(
        "jit_arc_option_payload",
        super::super::ffi::result::jit_arc_option_payload as *const u8,
    );
    // Arc-shape kinded retain/release per ADR-006 §2.7.17 / Q18.
    // Required because the legacy `jit_arc_retain`/`jit_arc_release`
    // operate on the `UnifiedValue<T>` refcount layout at offset 4,
    // which would corrupt `Arc<ResultData>`/`Arc<OptionData>` allocations
    // (whose refcount lives at offset -16 per Rust Arc contract).
    builder.symbol(
        "jit_arc_result_retain",
        super::super::ffi::result::jit_arc_result_retain as *const u8,
    );
    builder.symbol(
        "jit_arc_result_release",
        super::super::ffi::result::jit_arc_result_release as *const u8,
    );
    builder.symbol(
        "jit_arc_option_retain",
        super::super::ffi::result::jit_arc_option_retain as *const u8,
    );
    builder.symbol(
        "jit_arc_option_release",
        super::super::ffi::result::jit_arc_option_release as *const u8,
    );
    builder.symbol("jit_print", jit_print as *const u8);
    // W11-jit-new-array (ADR-006 §2.7.5 stamp-at-compile-time): per-kind
    // print entry points dispatched by the MIR-side print emitter when
    // the operand's `NativeKind` is statically known. Replaces the
    // deleted kind-blind tag-decode in `format_value_word` for scalar
    // operands.
    builder.symbol("jit_print_i64", jit_print_i64 as *const u8);
    builder.symbol("jit_print_f64", jit_print_f64 as *const u8);
    builder.symbol("jit_print_bool", jit_print_bool as *const u8);
    // W12-jit-print-heap-arm-classification (Phase 3 cluster-0 Round 8A,
    // 2026-05-13): per-HeapKind kinded print entries. ADR-006 §2.7.5
    // stamp-at-compile-time — the MIR-side Call-terminator print dispatch
    // routes to these when the operand `NativeKind` is a heap arm
    // (`String` / `Ptr(HeapKind::TypedObject)` / `Ptr(HeapKind::Option)` /
    // `Ptr(HeapKind::Result)`). Each reads typed `*const T` field
    // projections, no NaN-box tag decode, no `is_heap_kind` probe
    // (§2.7.7 #4 / #7 forbidden). Routes through `ValueFormatter::
    // format_kinded` so VM == JIT identical output.
    builder.symbol("jit_print_str", jit_print_str as *const u8);
    builder.symbol(
        "jit_print_typed_object",
        jit_print_typed_object as *const u8,
    );
    builder.symbol("jit_print_option", jit_print_option as *const u8);
    builder.symbol("jit_print_result", jit_print_result as *const u8);
    builder.symbol("jit_make_closure", jit_make_closure as *const u8);
    // Closure-spec Phase H2: TypedClosureHeader finalizer used by
    // `MirToIR::emit_heap_closure` to convert the raw typed block into a
    // NaN-boxed `Arc<HeapValue::Closure>` for downstream dispatch. Replaces
    // `jit_make_closure` on the `MakeClosureHeap` lowering path.
    builder.symbol(
        "jit_finalize_heap_closure",
        jit_finalize_heap_closure as *const u8,
    );
    // Track A.1D: allocator for `CaptureKind::OwnedMutable` capture cells.
    // `MirToIR::emit_heap_closure` calls this per OwnedMutable capture to
    // get a fresh `Box::into_raw`'d `*mut ValueWord` pointer, then stores
    // it into the closure's Ptr slot.
    builder.symbol(
        "jit_alloc_owned_mut_cell",
        jit_alloc_owned_mut_cell as *const u8,
    );
    // Track A.1E: Shared capture FFI helpers.
    //   `jit_arc_shared_retain`        — per-capture strong-count retain
    //                                    in `emit_heap_closure`'s Shared
    //                                    branch. Mirrors the interpreter's
    //                                    `Arc::increment_strong_count` in
    //                                    `op_make_closure`.
    //   `jit_shared_lock_contended`    — spin-wait fallback when the
    //                                    inline CAS lock (0→1) fails.
    //   `jit_shared_unlock_contended`  — release store fallback when the
    //                                    inline CAS unlock fails.
    builder.symbol(
        "jit_arc_shared_retain",
        jit_arc_shared_retain as *const u8,
    );
    builder.symbol(
        "jit_shared_lock_contended",
        jit_shared_lock_contended as *const u8,
    );
    builder.symbol(
        "jit_shared_unlock_contended",
        jit_shared_unlock_contended as *const u8,
    );
    // Session 1 Commit 3: outer-scope Shared-cell lifecycle helpers.
    //   `jit_alloc_shared_cell`      — allocates a fresh Arc<SharedCell>
    //                                   when MirToIR initializes a
    //                                   SharedCow local slot.
    //   `jit_arc_shared_release`     — consumes one strong share when a
    //                                   SharedCow slot is dropped.
    builder.symbol(
        "jit_alloc_shared_cell",
        jit_alloc_shared_cell as *const u8,
    );
    builder.symbol(
        "jit_arc_shared_release",
        jit_arc_shared_release as *const u8,
    );

    // Wave C.1: per-FieldKind closure-cell FFI helpers. 33 OwnedMutable
    // (alloc/read/write × 11 FieldKinds) + 22 Shared (read/write × 11
    // FieldKinds) = 55 distinct symbols.  See
    // `crates/shape-jit/src/ffi/object/closure.rs` for the wrapper
    // implementations and the `D1 native ABI` block doc.
    //
    // OwnedMutable allocators
    builder.symbol(
        "jit_alloc_owned_mut_cell_i64",
        jit_alloc_owned_mut_cell_i64 as *const u8,
    );
    builder.symbol(
        "jit_alloc_owned_mut_cell_u64",
        jit_alloc_owned_mut_cell_u64 as *const u8,
    );
    builder.symbol(
        "jit_alloc_owned_mut_cell_f64",
        jit_alloc_owned_mut_cell_f64 as *const u8,
    );
    builder.symbol(
        "jit_alloc_owned_mut_cell_i32",
        jit_alloc_owned_mut_cell_i32 as *const u8,
    );
    builder.symbol(
        "jit_alloc_owned_mut_cell_u32",
        jit_alloc_owned_mut_cell_u32 as *const u8,
    );
    builder.symbol(
        "jit_alloc_owned_mut_cell_i16",
        jit_alloc_owned_mut_cell_i16 as *const u8,
    );
    builder.symbol(
        "jit_alloc_owned_mut_cell_u16",
        jit_alloc_owned_mut_cell_u16 as *const u8,
    );
    builder.symbol(
        "jit_alloc_owned_mut_cell_i8",
        jit_alloc_owned_mut_cell_i8 as *const u8,
    );
    builder.symbol(
        "jit_alloc_owned_mut_cell_u8",
        jit_alloc_owned_mut_cell_u8 as *const u8,
    );
    builder.symbol(
        "jit_alloc_owned_mut_cell_bool",
        jit_alloc_owned_mut_cell_bool as *const u8,
    );
    builder.symbol(
        "jit_alloc_owned_mut_cell_ptr",
        jit_alloc_owned_mut_cell_ptr as *const u8,
    );
    // OwnedMutable readers
    builder.symbol(
        "jit_read_owned_mut_cell_i64",
        jit_read_owned_mut_cell_i64 as *const u8,
    );
    builder.symbol(
        "jit_read_owned_mut_cell_u64",
        jit_read_owned_mut_cell_u64 as *const u8,
    );
    builder.symbol(
        "jit_read_owned_mut_cell_f64",
        jit_read_owned_mut_cell_f64 as *const u8,
    );
    builder.symbol(
        "jit_read_owned_mut_cell_i32",
        jit_read_owned_mut_cell_i32 as *const u8,
    );
    builder.symbol(
        "jit_read_owned_mut_cell_u32",
        jit_read_owned_mut_cell_u32 as *const u8,
    );
    builder.symbol(
        "jit_read_owned_mut_cell_i16",
        jit_read_owned_mut_cell_i16 as *const u8,
    );
    builder.symbol(
        "jit_read_owned_mut_cell_u16",
        jit_read_owned_mut_cell_u16 as *const u8,
    );
    builder.symbol(
        "jit_read_owned_mut_cell_i8",
        jit_read_owned_mut_cell_i8 as *const u8,
    );
    builder.symbol(
        "jit_read_owned_mut_cell_u8",
        jit_read_owned_mut_cell_u8 as *const u8,
    );
    builder.symbol(
        "jit_read_owned_mut_cell_bool",
        jit_read_owned_mut_cell_bool as *const u8,
    );
    builder.symbol(
        "jit_read_owned_mut_cell_ptr",
        jit_read_owned_mut_cell_ptr as *const u8,
    );
    // OwnedMutable writers
    builder.symbol(
        "jit_write_owned_mut_cell_i64",
        jit_write_owned_mut_cell_i64 as *const u8,
    );
    builder.symbol(
        "jit_write_owned_mut_cell_u64",
        jit_write_owned_mut_cell_u64 as *const u8,
    );
    builder.symbol(
        "jit_write_owned_mut_cell_f64",
        jit_write_owned_mut_cell_f64 as *const u8,
    );
    builder.symbol(
        "jit_write_owned_mut_cell_i32",
        jit_write_owned_mut_cell_i32 as *const u8,
    );
    builder.symbol(
        "jit_write_owned_mut_cell_u32",
        jit_write_owned_mut_cell_u32 as *const u8,
    );
    builder.symbol(
        "jit_write_owned_mut_cell_i16",
        jit_write_owned_mut_cell_i16 as *const u8,
    );
    builder.symbol(
        "jit_write_owned_mut_cell_u16",
        jit_write_owned_mut_cell_u16 as *const u8,
    );
    builder.symbol(
        "jit_write_owned_mut_cell_i8",
        jit_write_owned_mut_cell_i8 as *const u8,
    );
    builder.symbol(
        "jit_write_owned_mut_cell_u8",
        jit_write_owned_mut_cell_u8 as *const u8,
    );
    builder.symbol(
        "jit_write_owned_mut_cell_bool",
        jit_write_owned_mut_cell_bool as *const u8,
    );
    builder.symbol(
        "jit_write_owned_mut_cell_ptr",
        jit_write_owned_mut_cell_ptr as *const u8,
    );
    // Shared readers
    builder.symbol(
        "jit_read_shared_cell_i64",
        jit_read_shared_cell_i64 as *const u8,
    );
    builder.symbol(
        "jit_read_shared_cell_u64",
        jit_read_shared_cell_u64 as *const u8,
    );
    builder.symbol(
        "jit_read_shared_cell_f64",
        jit_read_shared_cell_f64 as *const u8,
    );
    builder.symbol(
        "jit_read_shared_cell_i32",
        jit_read_shared_cell_i32 as *const u8,
    );
    builder.symbol(
        "jit_read_shared_cell_u32",
        jit_read_shared_cell_u32 as *const u8,
    );
    builder.symbol(
        "jit_read_shared_cell_i16",
        jit_read_shared_cell_i16 as *const u8,
    );
    builder.symbol(
        "jit_read_shared_cell_u16",
        jit_read_shared_cell_u16 as *const u8,
    );
    builder.symbol(
        "jit_read_shared_cell_i8",
        jit_read_shared_cell_i8 as *const u8,
    );
    builder.symbol(
        "jit_read_shared_cell_u8",
        jit_read_shared_cell_u8 as *const u8,
    );
    builder.symbol(
        "jit_read_shared_cell_bool",
        jit_read_shared_cell_bool as *const u8,
    );
    builder.symbol(
        "jit_read_shared_cell_ptr",
        jit_read_shared_cell_ptr as *const u8,
    );
    // Shared writers
    builder.symbol(
        "jit_write_shared_cell_i64",
        jit_write_shared_cell_i64 as *const u8,
    );
    builder.symbol(
        "jit_write_shared_cell_u64",
        jit_write_shared_cell_u64 as *const u8,
    );
    builder.symbol(
        "jit_write_shared_cell_f64",
        jit_write_shared_cell_f64 as *const u8,
    );
    builder.symbol(
        "jit_write_shared_cell_i32",
        jit_write_shared_cell_i32 as *const u8,
    );
    builder.symbol(
        "jit_write_shared_cell_u32",
        jit_write_shared_cell_u32 as *const u8,
    );
    builder.symbol(
        "jit_write_shared_cell_i16",
        jit_write_shared_cell_i16 as *const u8,
    );
    builder.symbol(
        "jit_write_shared_cell_u16",
        jit_write_shared_cell_u16 as *const u8,
    );
    builder.symbol(
        "jit_write_shared_cell_i8",
        jit_write_shared_cell_i8 as *const u8,
    );
    builder.symbol(
        "jit_write_shared_cell_u8",
        jit_write_shared_cell_u8 as *const u8,
    );
    builder.symbol(
        "jit_write_shared_cell_bool",
        jit_write_shared_cell_bool as *const u8,
    );
    builder.symbol(
        "jit_write_shared_cell_ptr",
        jit_write_shared_cell_ptr as *const u8,
    );

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

    // F5.a/F5.b: jit_string_concat(a_bits: u64, b_bits: u64) -> u64
    // Signature matches the two-operand MIR BinaryOp::Add lowering for
    // string slots. Result is a fresh unified-heap string (refcount 1).
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        let func_id = module
            .declare_function("jit_string_concat", Linkage::Import, &sig)
            .expect("Failed to declare jit_string_concat");
        ffi_funcs.insert("jit_string_concat".to_string(), func_id);
    }

    // ADR-006 §2.7.5 — kinded EnumStore producers
    // (W12-jit-aggregate-non-array, 2026-05-12). Signature:
    // `(inner_bits: u64) -> u64`. The JIT EnumStore consumer widens
    // every operand to I64 bits (via `widen_to_i64`) per the §2.7.5
    // stable-FFI carrier convention before calling. Return is the
    // heap-pointer bits with HK_OK / HK_ERR / HK_SOME prefix tag —
    // `jit_bits_to_nanboxed` at the JIT↔VM boundary converts to
    // `Arc<ResultData>` / `Arc<OptionData>` (`crates/shape-jit/src/
    // ffi/conversion.rs:246-258`).
    for name in ["jit_make_ok", "jit_make_err", "jit_make_some"] {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // inner_bits
        sig.returns.push(AbiParam::new(types::I64));
        let func_id = module
            .declare_function(name, Linkage::Import, &sig)
            .unwrap_or_else(|e| panic!("Failed to declare {}: {:?}", name, e));
        ffi_funcs.insert(name.to_string(), func_id);
    }

    // ADR-006 §2.7.17 / Q18 — Arc-shape Result/Option producers
    // (W12-jit-result-option-trinity, Phase 3 cluster-0 Round 7A,
    // 2026-05-12). Signature:
    // `(payload_bits: u64, payload_kind_code: i8) -> u64`.
    // The payload_kind_code is the §2.7.7 / Q9 parallel-track byte encoding
    // (`crates/shape-jit/src/ffi/stack_kind_code.rs`) stamped at JIT-compile
    // time from the EnumStore operand's MIR-inferred kind per §2.7.5.
    // Return is `Arc::into_raw(Arc<ResultData>) as u64` /
    // `Arc::into_raw(Arc<OptionData>) as u64` with kind labels
    // `Ptr(HeapKind::Result)` / `Ptr(HeapKind::Option)`.
    for name in [
        "jit_v2_make_result_ok",
        "jit_v2_make_result_err",
        "jit_v2_make_option_some",
    ] {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // payload_bits
        sig.params.push(AbiParam::new(types::I8));  // payload_kind_code
        sig.returns.push(AbiParam::new(types::I64));
        let func_id = module
            .declare_function(name, Linkage::Import, &sig)
            .unwrap_or_else(|e| panic!("Failed to declare {}: {:?}", name, e));
        ffi_funcs.insert(name.to_string(), func_id);
    }
    // jit_v2_make_option_none takes no payload (None has no payload).
    {
        let mut sig = module.make_signature();
        sig.returns.push(AbiParam::new(types::I64));
        let func_id = module
            .declare_function("jit_v2_make_option_none", Linkage::Import, &sig)
            .expect("Failed to declare jit_v2_make_option_none");
        ffi_funcs.insert("jit_v2_make_option_none".to_string(), func_id);
    }
    // ADR-006 §2.7.17 — Arc-shape predicates: read is_ok / is_err / is_some /
    // is_none from `*const ResultData` / `*const OptionData` directly.
    // Signature: `(bits: u64) -> u8` (native bool).
    for name in [
        "jit_arc_result_is_ok",
        "jit_arc_result_is_err",
        "jit_arc_option_is_some",
        "jit_arc_option_is_none",
    ] {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I8));
        let func_id = module
            .declare_function(name, Linkage::Import, &sig)
            .unwrap_or_else(|e| panic!("Failed to declare {}: {:?}", name, e));
        ffi_funcs.insert(name.to_string(), func_id);
    }
    // Arc-shape payload extractors: clone the inner KindedSlot's share +
    // return its raw bits. Signature: `(bits: u64) -> u64`.
    for name in ["jit_arc_result_payload", "jit_arc_option_payload"] {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        let func_id = module
            .declare_function(name, Linkage::Import, &sig)
            .unwrap_or_else(|e| panic!("Failed to declare {}: {:?}", name, e));
        ffi_funcs.insert(name.to_string(), func_id);
    }
    // Arc-shape retain/release: bump/decrement the standard Rust Arc
    // refcount at offset -16 per Arc contract. Signature: `(bits: u64)`.
    // Required by `refcount_disposition` when the slot kind is
    // `Ptr(HeapKind::Result)` or `Ptr(HeapKind::Option)` — the legacy
    // `jit_arc_retain`/`jit_arc_release` for `UnifiedValue<T>` corrupt
    // these allocations.
    for name in [
        "jit_arc_result_retain",
        "jit_arc_result_release",
        "jit_arc_option_retain",
        "jit_arc_option_release",
    ] {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        let func_id = module
            .declare_function(name, Linkage::Import, &sig)
            .unwrap_or_else(|e| panic!("Failed to declare {}: {:?}", name, e));
        ffi_funcs.insert(name.to_string(), func_id);
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

    // W11-jit-new-array kinded print entries (ADR-006 §2.7.5).
    // jit_print_i64(value: i64) -> void
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        let func_id = module
            .declare_function("jit_print_i64", Linkage::Import, &sig)
            .expect("Failed to declare jit_print_i64");
        ffi_funcs.insert("jit_print_i64".to_string(), func_id);
    }
    // jit_print_f64(value: f64) -> void
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::F64));
        let func_id = module
            .declare_function("jit_print_f64", Linkage::Import, &sig)
            .expect("Failed to declare jit_print_f64");
        ffi_funcs.insert("jit_print_f64".to_string(), func_id);
    }
    // jit_print_bool(value: u8) -> void
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I8));
        let func_id = module
            .declare_function("jit_print_bool", Linkage::Import, &sig)
            .expect("Failed to declare jit_print_bool");
        ffi_funcs.insert("jit_print_bool".to_string(), func_id);
    }

    // W12-jit-print-heap-arm-classification (Phase 3 cluster-0 Round 8A,
    // 2026-05-13): heap-arm kinded print entries (ADR-006 §2.7.5).
    //
    // Each takes `(ctx_ptr: *const JITContext, bits: u64)` — `ctx_ptr` is
    // I64 carrying the JIT context pointer so the FFI body can resolve
    // the type schema registry for TypedObject field-name rendering, and
    // `bits` is the typed-Arc raw pointer (`Arc::into_raw(Arc<T>) as u64`).
    // The kind is implicit in the chosen entry by §2.7.5 stamp-at-compile-
    // time discipline; no kind-code parameter, no Bool-default fallback.
    //
    // jit_print_str(ctx_ptr: *const JITContext, bits: u64) -> void
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ctx_ptr
        sig.params.push(AbiParam::new(types::I64)); // bits (Arc<String>)
        let func_id = module
            .declare_function("jit_print_str", Linkage::Import, &sig)
            .expect("Failed to declare jit_print_str");
        ffi_funcs.insert("jit_print_str".to_string(), func_id);
    }
    // jit_print_typed_object(ctx_ptr: *const JITContext, bits: u64) -> void
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ctx_ptr
        sig.params.push(AbiParam::new(types::I64)); // bits (Arc<TypedObjectStorage>)
        let func_id = module
            .declare_function("jit_print_typed_object", Linkage::Import, &sig)
            .expect("Failed to declare jit_print_typed_object");
        ffi_funcs.insert("jit_print_typed_object".to_string(), func_id);
    }
    // jit_print_option(ctx_ptr: *const JITContext, bits: u64) -> void
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ctx_ptr
        sig.params.push(AbiParam::new(types::I64)); // bits (Arc<OptionData>)
        let func_id = module
            .declare_function("jit_print_option", Linkage::Import, &sig)
            .expect("Failed to declare jit_print_option");
        ffi_funcs.insert("jit_print_option".to_string(), func_id);
    }
    // jit_print_result(ctx_ptr: *const JITContext, bits: u64) -> void
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ctx_ptr
        sig.params.push(AbiParam::new(types::I64)); // bits (Arc<ResultData>)
        let func_id = module
            .declare_function("jit_print_result", Linkage::Import, &sig)
            .expect("Failed to declare jit_print_result");
        ffi_funcs.insert("jit_print_result".to_string(), func_id);
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

    // Closure-spec Phase H2: jit_finalize_heap_closure(header_ptr, function_id,
    // captures_count, layout_ptr) -> u64. Converts a `TypedClosureHeader`
    // block allocated by `emit_heap_closure` into a NaN-boxed
    // `Arc<HeapValue::Closure>` for downstream dispatch. See
    // docs/v2-closure-specialization.md §13 H2.
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // header_ptr
        sig.params.push(AbiParam::new(types::I32)); // function_id (u16 promoted)
        sig.params.push(AbiParam::new(types::I32)); // captures_count (u16 promoted)
        sig.params.push(AbiParam::new(types::I64)); // layout_ptr
        sig.returns.push(AbiParam::new(types::I64));
        let func_id = module
            .declare_function("jit_finalize_heap_closure", Linkage::Import, &sig)
            .expect("Failed to declare jit_finalize_heap_closure");
        ffi_funcs.insert("jit_finalize_heap_closure".to_string(), func_id);
    }

    // Track A.1D: jit_alloc_owned_mut_cell(initial: u64) -> *mut u64.
    // Allocates a `Box<u64>` (ValueWord cell) from the initial bits and
    // returns the raw pointer. `MirToIR::emit_heap_closure` calls this per
    // `CaptureKind::OwnedMutable` capture; `release_typed_closure` reclaims
    // the pointer via `Box::from_raw` on closure drop.
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // initial (ValueWord bits)
        sig.returns.push(AbiParam::new(types::I64)); // *mut u64 (cell ptr)
        let func_id = module
            .declare_function("jit_alloc_owned_mut_cell", Linkage::Import, &sig)
            .expect("Failed to declare jit_alloc_owned_mut_cell");
        ffi_funcs.insert("jit_alloc_owned_mut_cell".to_string(), func_id);
    }

    // Track A.1E: jit_arc_shared_retain(ptr: u64) -> u64.
    // Increments the strong count of the `Arc<SharedCell>` pointed to by
    // `ptr` and returns `ptr` unchanged. Called from
    // `MirToIR::emit_heap_closure` once per `CaptureKind::Shared` capture
    // to mint the closure's own Arc share on top of the outer slot's
    // share. `release_typed_closure` balances each retain via
    // `Arc::from_raw` on closure drop.
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ptr (*const SharedCell)
        sig.returns.push(AbiParam::new(types::I64)); // ptr (same, chained)
        let func_id = module
            .declare_function("jit_arc_shared_retain", Linkage::Import, &sig)
            .expect("Failed to declare jit_arc_shared_retain");
        ffi_funcs.insert("jit_arc_shared_retain".to_string(), func_id);
    }

    // Track A.1E: jit_shared_lock_contended(ptr: u64).
    // Slow-path lock acquire for Shared capture reads/writes. Called
    // when the JIT's inline CAS on the state byte (0→1) fails.
    // Spins until the byte transitions to `1` under `Acquire` ordering.
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ptr (*const SharedCell)
        let func_id = module
            .declare_function("jit_shared_lock_contended", Linkage::Import, &sig)
            .expect("Failed to declare jit_shared_lock_contended");
        ffi_funcs.insert("jit_shared_lock_contended".to_string(), func_id);
    }

    // Track A.1E: jit_shared_unlock_contended(ptr: u64).
    // Slow-path unlock for Shared capture reads/writes. In the current
    // spinlock design this is just a release store; the branch is kept
    // for ABI parity with the inline CAS structure.
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ptr (*const SharedCell)
        let func_id = module
            .declare_function("jit_shared_unlock_contended", Linkage::Import, &sig)
            .expect("Failed to declare jit_shared_unlock_contended");
        ffi_funcs.insert("jit_shared_unlock_contended".to_string(), func_id);
    }

    // Session 1 Commit 3: jit_alloc_shared_cell(initial_bits: u64) -> u64.
    // Allocates a fresh `Arc<SharedCell>` seeded with `initial_bits` and
    // returns the raw pointer bits of the sole strong share.
    // `MirToIR::initialize_shared_local_slots` calls this at function
    // entry to materialise the cell that backs every SharedCow local.
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // initial_bits (ValueWord)
        sig.returns.push(AbiParam::new(types::I64)); // *const SharedCell
        let func_id = module
            .declare_function("jit_alloc_shared_cell", Linkage::Import, &sig)
            .expect("Failed to declare jit_alloc_shared_cell");
        ffi_funcs.insert("jit_alloc_shared_cell".to_string(), func_id);
    }

    // Session 1 Commit 3: jit_arc_shared_release(ptr: u64).
    // Consumes exactly one strong share of the `Arc<SharedCell>` at
    // `ptr`. `ptr == 0` is a no-op (matches the interpreter's
    // `op_drop_shared_local` null-pointer guard). The JIT emits this
    // on MIR `Drop(Local(slot))` when the slot is a SharedCow local.
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ptr (*const SharedCell)
        let func_id = module
            .declare_function("jit_arc_shared_release", Linkage::Import, &sig)
            .expect("Failed to declare jit_arc_shared_release");
        ffi_funcs.insert("jit_arc_shared_release".to_string(), func_id);
    }

    // Wave C.1: per-FieldKind closure-cell FFI helpers — Cranelift
    // signatures matching the wrappers in
    // `crates/shape-jit/src/ffi/object/closure.rs`.
    //
    // Type lookup per FieldKind (ABI):
    //   F64               -> types::F64
    //   I64 / U64 / Ptr   -> types::I64
    //   I32 / U32         -> types::I32
    //   I16 / U16 / I8 / U8 / Bool -> types::I32 (sub-32 widened at boundary)
    //
    // Cell pointers (the `ptr: i64` arg in readers/writers, and return of
    // allocators) are always types::I64.
    fn declare_owned_alloc(
        module: &mut JITModule,
        ffi_funcs: &mut HashMap<String, FuncId>,
        name: &'static str,
        param: types::Type,
    ) {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(param)); // initial native value
        sig.returns.push(AbiParam::new(types::I64)); // *mut T cell ptr
        let func_id = module
            .declare_function(name, Linkage::Import, &sig)
            .unwrap_or_else(|_| panic!("Failed to declare {}", name));
        ffi_funcs.insert(name.to_string(), func_id);
    }
    fn declare_cell_read(
        module: &mut JITModule,
        ffi_funcs: &mut HashMap<String, FuncId>,
        name: &'static str,
        ret: types::Type,
    ) {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // cell ptr
        sig.returns.push(AbiParam::new(ret)); // native value
        let func_id = module
            .declare_function(name, Linkage::Import, &sig)
            .unwrap_or_else(|_| panic!("Failed to declare {}", name));
        ffi_funcs.insert(name.to_string(), func_id);
    }
    fn declare_cell_write(
        module: &mut JITModule,
        ffi_funcs: &mut HashMap<String, FuncId>,
        name: &'static str,
        param: types::Type,
    ) {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // cell ptr
        sig.params.push(AbiParam::new(param)); // native value
        let func_id = module
            .declare_function(name, Linkage::Import, &sig)
            .unwrap_or_else(|_| panic!("Failed to declare {}", name));
        ffi_funcs.insert(name.to_string(), func_id);
    }

    // OwnedMutable allocators
    declare_owned_alloc(module, ffi_funcs, "jit_alloc_owned_mut_cell_i64", types::I64);
    declare_owned_alloc(module, ffi_funcs, "jit_alloc_owned_mut_cell_u64", types::I64);
    declare_owned_alloc(module, ffi_funcs, "jit_alloc_owned_mut_cell_f64", types::F64);
    declare_owned_alloc(module, ffi_funcs, "jit_alloc_owned_mut_cell_i32", types::I32);
    declare_owned_alloc(module, ffi_funcs, "jit_alloc_owned_mut_cell_u32", types::I32);
    declare_owned_alloc(module, ffi_funcs, "jit_alloc_owned_mut_cell_i16", types::I32);
    declare_owned_alloc(module, ffi_funcs, "jit_alloc_owned_mut_cell_u16", types::I32);
    declare_owned_alloc(module, ffi_funcs, "jit_alloc_owned_mut_cell_i8", types::I32);
    declare_owned_alloc(module, ffi_funcs, "jit_alloc_owned_mut_cell_u8", types::I32);
    declare_owned_alloc(module, ffi_funcs, "jit_alloc_owned_mut_cell_bool", types::I32);
    declare_owned_alloc(module, ffi_funcs, "jit_alloc_owned_mut_cell_ptr", types::I64);
    // OwnedMutable readers
    declare_cell_read(module, ffi_funcs, "jit_read_owned_mut_cell_i64", types::I64);
    declare_cell_read(module, ffi_funcs, "jit_read_owned_mut_cell_u64", types::I64);
    declare_cell_read(module, ffi_funcs, "jit_read_owned_mut_cell_f64", types::F64);
    declare_cell_read(module, ffi_funcs, "jit_read_owned_mut_cell_i32", types::I32);
    declare_cell_read(module, ffi_funcs, "jit_read_owned_mut_cell_u32", types::I32);
    declare_cell_read(module, ffi_funcs, "jit_read_owned_mut_cell_i16", types::I32);
    declare_cell_read(module, ffi_funcs, "jit_read_owned_mut_cell_u16", types::I32);
    declare_cell_read(module, ffi_funcs, "jit_read_owned_mut_cell_i8", types::I32);
    declare_cell_read(module, ffi_funcs, "jit_read_owned_mut_cell_u8", types::I32);
    declare_cell_read(module, ffi_funcs, "jit_read_owned_mut_cell_bool", types::I32);
    declare_cell_read(module, ffi_funcs, "jit_read_owned_mut_cell_ptr", types::I64);
    // OwnedMutable writers
    declare_cell_write(module, ffi_funcs, "jit_write_owned_mut_cell_i64", types::I64);
    declare_cell_write(module, ffi_funcs, "jit_write_owned_mut_cell_u64", types::I64);
    declare_cell_write(module, ffi_funcs, "jit_write_owned_mut_cell_f64", types::F64);
    declare_cell_write(module, ffi_funcs, "jit_write_owned_mut_cell_i32", types::I32);
    declare_cell_write(module, ffi_funcs, "jit_write_owned_mut_cell_u32", types::I32);
    declare_cell_write(module, ffi_funcs, "jit_write_owned_mut_cell_i16", types::I32);
    declare_cell_write(module, ffi_funcs, "jit_write_owned_mut_cell_u16", types::I32);
    declare_cell_write(module, ffi_funcs, "jit_write_owned_mut_cell_i8", types::I32);
    declare_cell_write(module, ffi_funcs, "jit_write_owned_mut_cell_u8", types::I32);
    declare_cell_write(module, ffi_funcs, "jit_write_owned_mut_cell_bool", types::I32);
    declare_cell_write(module, ffi_funcs, "jit_write_owned_mut_cell_ptr", types::I64);
    // Shared readers (alloc/release reuse the existing generic helpers
    // `jit_alloc_shared_cell` / `jit_arc_shared_release`).
    declare_cell_read(module, ffi_funcs, "jit_read_shared_cell_i64", types::I64);
    declare_cell_read(module, ffi_funcs, "jit_read_shared_cell_u64", types::I64);
    declare_cell_read(module, ffi_funcs, "jit_read_shared_cell_f64", types::F64);
    declare_cell_read(module, ffi_funcs, "jit_read_shared_cell_i32", types::I32);
    declare_cell_read(module, ffi_funcs, "jit_read_shared_cell_u32", types::I32);
    declare_cell_read(module, ffi_funcs, "jit_read_shared_cell_i16", types::I32);
    declare_cell_read(module, ffi_funcs, "jit_read_shared_cell_u16", types::I32);
    declare_cell_read(module, ffi_funcs, "jit_read_shared_cell_i8", types::I32);
    declare_cell_read(module, ffi_funcs, "jit_read_shared_cell_u8", types::I32);
    declare_cell_read(module, ffi_funcs, "jit_read_shared_cell_bool", types::I32);
    declare_cell_read(module, ffi_funcs, "jit_read_shared_cell_ptr", types::I64);
    // Shared writers
    declare_cell_write(module, ffi_funcs, "jit_write_shared_cell_i64", types::I64);
    declare_cell_write(module, ffi_funcs, "jit_write_shared_cell_u64", types::I64);
    declare_cell_write(module, ffi_funcs, "jit_write_shared_cell_f64", types::F64);
    declare_cell_write(module, ffi_funcs, "jit_write_shared_cell_i32", types::I32);
    declare_cell_write(module, ffi_funcs, "jit_write_shared_cell_u32", types::I32);
    declare_cell_write(module, ffi_funcs, "jit_write_shared_cell_i16", types::I32);
    declare_cell_write(module, ffi_funcs, "jit_write_shared_cell_u16", types::I32);
    declare_cell_write(module, ffi_funcs, "jit_write_shared_cell_i8", types::I32);
    declare_cell_write(module, ffi_funcs, "jit_write_shared_cell_u8", types::I32);
    declare_cell_write(module, ffi_funcs, "jit_write_shared_cell_bool", types::I32);
    declare_cell_write(module, ffi_funcs, "jit_write_shared_cell_ptr", types::I64);

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
