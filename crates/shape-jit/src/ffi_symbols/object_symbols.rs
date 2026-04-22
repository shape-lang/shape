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
    jit_print, jit_string_concat, jit_to_number, jit_to_string, jit_type_check, jit_typeof,
};
#[allow(deprecated)]
use super::super::ffi::object::{
    jit_alloc_owned_mut_cell, jit_alloc_shared_cell, jit_arc_shared_release,
    jit_arc_shared_retain, jit_finalize_heap_closure, jit_format, jit_get_prop,
    jit_hashmap_shape_id, jit_hashmap_value_at, jit_length, jit_make_closure, jit_new_object,
    jit_object_rest, jit_set_prop, jit_shared_lock_contended, jit_shared_unlock_contended,
};
use super::super::ffi::typed_object::{jit_typed_merge_object, jit_typed_object_alloc};
use super::super::ffi::typed_object::jit_typed_object_get_field;
use super::super::ffi::typed_object::jit_typed_object_set_field;
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
    builder.symbol("jit_print", jit_print as *const u8);
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
