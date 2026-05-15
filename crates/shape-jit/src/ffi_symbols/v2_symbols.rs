//! v2 typed FFI symbol registration for JIT compiler.
//!
//! Registers native-typed v2 FFI functions with the JIT builder and declares
//! their Cranelift signatures. KEY DIFFERENCE from v1: return types use native
//! Cranelift types (F64, I32) instead of everything being I64.

use cranelift::prelude::*;
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{FuncId, Linkage, Module};
use std::collections::HashMap;

use super::super::ffi::v2;
use super::super::ffi::v2::collection_arc;

/// Register all v2 FFI symbols with the JIT builder.
pub fn register_v2_symbols(builder: &mut JITBuilder) {
    // Array — f64
    builder.symbol("jit_v2_array_new_f64", v2::jit_v2_array_new_f64 as *const u8);
    builder.symbol("jit_v2_array_get_f64", v2::jit_v2_array_get_f64 as *const u8);
    builder.symbol("jit_v2_array_set_f64", v2::jit_v2_array_set_f64 as *const u8);
    builder.symbol("jit_v2_array_len_f64", v2::jit_v2_array_len_f64 as *const u8);

    // Array — i64
    builder.symbol("jit_v2_array_new_i64", v2::jit_v2_array_new_i64 as *const u8);
    builder.symbol("jit_v2_array_get_i64", v2::jit_v2_array_get_i64 as *const u8);
    builder.symbol("jit_v2_array_set_i64", v2::jit_v2_array_set_i64 as *const u8);
    builder.symbol("jit_v2_array_len_i64", v2::jit_v2_array_len_i64 as *const u8);

    // Array — i32
    builder.symbol("jit_v2_array_new_i32", v2::jit_v2_array_new_i32 as *const u8);
    builder.symbol("jit_v2_array_get_i32", v2::jit_v2_array_get_i32 as *const u8);
    builder.symbol("jit_v2_array_set_i32", v2::jit_v2_array_set_i32 as *const u8);
    builder.symbol("jit_v2_array_len_i32", v2::jit_v2_array_len_i32 as *const u8);

    // Array — bool (encoded as u8 internally)
    builder.symbol("jit_v2_array_new_bool", v2::jit_v2_array_new_bool as *const u8);
    builder.symbol("jit_v2_array_get_bool", v2::jit_v2_array_get_bool as *const u8);
    builder.symbol("jit_v2_array_set_bool", v2::jit_v2_array_set_bool as *const u8);
    builder.symbol("jit_v2_array_len_bool", v2::jit_v2_array_len_bool as *const u8);

    // ADR-006 §2.7.5 + §2.7.24 Q25.A SUPERSEDED + audit deliverable (b)
    // §4.1.B — v2-raw `TypedArray<*const StringObj>` / `TypedArray<*const
    // DecimalObj>` heap-element allocators. Phase 3 cluster-0+1 Wave 3
    // Stabilize Round 2 V3-S5 ckpt-6-prime Group X JIT FFI String/Decimal
    // BUILD (2026-05-15). Bodies live in `ffi/v2/mod.rs` mirroring
    // `jit_v2_array_new_<scalar>`'s shape; per-element refcount discipline
    // is the caller's responsibility per the VM-side
    // `NewStringV2` / `TypedArrayPushString` per-element transfer convention
    // at `crates/shape-vm/src/executor/v2_handlers/array.rs:803-858`.
    builder.symbol(
        "jit_new_typed_array_string",
        v2::jit_new_typed_array_string as *const u8,
    );
    builder.symbol(
        "jit_new_typed_array_decimal",
        v2::jit_new_typed_array_decimal as *const u8,
    );

    // Generic typed-array push dispatcher (R7.2 consolidation)
    builder.symbol("jit_v2_array_push", v2::jit_v2_array_push as *const u8);

    // Struct field access
    builder.symbol("jit_v2_field_load_f64", v2::jit_v2_field_load_f64 as *const u8);
    builder.symbol("jit_v2_field_load_i64", v2::jit_v2_field_load_i64 as *const u8);
    builder.symbol("jit_v2_field_load_i32", v2::jit_v2_field_load_i32 as *const u8);
    builder.symbol("jit_v2_field_load_ptr", v2::jit_v2_field_load_ptr as *const u8);
    builder.symbol("jit_v2_field_store_f64", v2::jit_v2_field_store_f64 as *const u8);
    builder.symbol("jit_v2_field_store_i64", v2::jit_v2_field_store_i64 as *const u8);
    builder.symbol("jit_v2_field_store_i32", v2::jit_v2_field_store_i32 as *const u8);
    builder.symbol("jit_v2_field_store_ptr", v2::jit_v2_field_store_ptr as *const u8);

    // Refcount
    builder.symbol("jit_v2_retain", v2::jit_v2_retain as *const u8);
    builder.symbol("jit_v2_release", v2::jit_v2_release as *const u8);

    // Struct allocation
    builder.symbol("jit_v2_alloc_struct", v2::jit_v2_alloc_struct as *const u8);

    // SIMD reductions (Phase C.3)
    builder.symbol("jit_v2_array_sum_f64", v2::jit_v2_array_sum_f64 as *const u8);
    builder.symbol("jit_v2_array_sum_i64", v2::jit_v2_array_sum_i64 as *const u8);

    // SIMD reductions — min / max / mean / sum-of-squares (f64)
    builder.symbol("jit_v2_array_min_f64", v2::jit_v2_array_min_f64 as *const u8);
    builder.symbol("jit_v2_array_max_f64", v2::jit_v2_array_max_f64 as *const u8);
    builder.symbol("jit_v2_array_mean_f64", v2::jit_v2_array_mean_f64 as *const u8);
    builder.symbol(
        "jit_v2_array_sum_squares_f64",
        v2::jit_v2_array_sum_squares_f64 as *const u8,
    );

    // SIMD element-wise scalar ops (allocating, f64)
    builder.symbol(
        "jit_v2_array_scale_f64",
        v2::jit_v2_array_scale_f64 as *const u8,
    );
    builder.symbol(
        "jit_v2_array_add_scalar_f64",
        v2::jit_v2_array_add_scalar_f64 as *const u8,
    );

    // SIMD element-wise binary ops (allocating, f64)
    builder.symbol("jit_v2_array_add_f64", v2::jit_v2_array_add_f64 as *const u8);
    builder.symbol("jit_v2_array_mul_f64", v2::jit_v2_array_mul_f64 as *const u8);

    // ADR-006 §2.7.5 / §2.7.25 — Typed-Arc collection allocators
    // (W12-jit-collection-arc-ffi-ctors-and-refcount, Phase 3 cluster-0
    // Round 9 / 8B.1, 2026-05-13). Bodies live in
    // `ffi/v2/collection_arc.rs`. Each ctor returns
    // `Arc::into_raw(Arc<XData>) as u64` — the carrier-shape rule
    // (audit §5) bans mixing this layout with W11's `Box<UnifiedValue<T>>`
    // HeapHeader carriers; the per-HeapKind retain/release entries
    // registered below operate on the Arc control block at offset -16,
    // never the offset-4 UnifiedValue path.
    builder.symbol("jit_v2_make_hashset", collection_arc::jit_v2_make_hashset as *const u8);
    builder.symbol("jit_v2_make_hashmap", collection_arc::jit_v2_make_hashmap as *const u8);
    builder.symbol("jit_v2_make_deque", collection_arc::jit_v2_make_deque as *const u8);
    builder.symbol(
        "jit_v2_make_priorityqueue",
        collection_arc::jit_v2_make_priorityqueue as *const u8,
    );
    builder.symbol("jit_v2_make_channel", collection_arc::jit_v2_make_channel as *const u8);
    builder.symbol("jit_v2_make_atomic", collection_arc::jit_v2_make_atomic as *const u8);
    builder.symbol("jit_v2_make_lazy", collection_arc::jit_v2_make_lazy as *const u8);
    builder.symbol("jit_v2_make_mutex", collection_arc::jit_v2_make_mutex as *const u8);

    // Per-HeapKind kinded retain/release. Refcount discipline at slots
    // whose `NativeKind` is `Ptr(HeapKind::HashSet|HashMap|Deque|
    // PriorityQueue|Channel|Mutex|Atomic|Lazy)` dispatches HERE instead
    // of the legacy `jit_arc_retain` / `jit_arc_release` — see
    // `mir_compiler/ownership.rs::retain_func_for_place` /
    // `release_func_for_place` for the dispatch arms.
    builder.symbol(
        "jit_arc_hashset_retain",
        collection_arc::jit_arc_hashset_retain as *const u8,
    );
    builder.symbol(
        "jit_arc_hashset_release",
        collection_arc::jit_arc_hashset_release as *const u8,
    );
    builder.symbol(
        "jit_arc_hashmap_retain",
        collection_arc::jit_arc_hashmap_retain as *const u8,
    );
    builder.symbol(
        "jit_arc_hashmap_release",
        collection_arc::jit_arc_hashmap_release as *const u8,
    );
    builder.symbol(
        "jit_arc_deque_retain",
        collection_arc::jit_arc_deque_retain as *const u8,
    );
    builder.symbol(
        "jit_arc_deque_release",
        collection_arc::jit_arc_deque_release as *const u8,
    );
    builder.symbol(
        "jit_arc_priorityqueue_retain",
        collection_arc::jit_arc_priorityqueue_retain as *const u8,
    );
    builder.symbol(
        "jit_arc_priorityqueue_release",
        collection_arc::jit_arc_priorityqueue_release as *const u8,
    );
    builder.symbol(
        "jit_arc_channel_retain",
        collection_arc::jit_arc_channel_retain as *const u8,
    );
    builder.symbol(
        "jit_arc_channel_release",
        collection_arc::jit_arc_channel_release as *const u8,
    );
    builder.symbol(
        "jit_arc_mutex_retain",
        collection_arc::jit_arc_mutex_retain as *const u8,
    );
    builder.symbol(
        "jit_arc_mutex_release",
        collection_arc::jit_arc_mutex_release as *const u8,
    );
    builder.symbol(
        "jit_arc_atomic_retain",
        collection_arc::jit_arc_atomic_retain as *const u8,
    );
    builder.symbol(
        "jit_arc_atomic_release",
        collection_arc::jit_arc_atomic_release as *const u8,
    );
    builder.symbol(
        "jit_arc_lazy_retain",
        collection_arc::jit_arc_lazy_retain as *const u8,
    );
    builder.symbol(
        "jit_arc_lazy_release",
        collection_arc::jit_arc_lazy_release as *const u8,
    );

    // Typed HashMap<string, ...> access — SURFACE per ADR-006 §2.7.4 /
    // W10 jit-playbook §5. The deleted ValueWord-shape map FFI
    // (jit_v2_map_get_str_i64 / get_str_f64 / has_str / set_str_i64 /
    // len) is gone; the strict-typing rebuild routes through
    // `Arc<HashMapData>` + `KindedSlot` per ADR-006 §2.7.5 / §2.7.6 /
    // Q8 — see `ffi/v2/typed_map.rs` SURFACE comment. Symbol
    // registration is a no-op until the kinded entries land; the
    // declarations in `declare_v2_functions` below are also a no-op
    // for the same set so unresolved symbols never reach the JIT.
}

/// Helper: declare a function and insert into the map.
fn declare(
    module: &mut JITModule,
    ffi_funcs: &mut HashMap<String, FuncId>,
    name: &str,
    sig: &Signature,
) {
    if let Ok(func_id) = module.declare_function(name, Linkage::Import, sig) {
        ffi_funcs.insert(name.to_string(), func_id);
    }
}

/// Declare all v2 FFI function signatures in the Cranelift module.
pub fn declare_v2_functions(module: &mut JITModule, ffi_funcs: &mut HashMap<String, FuncId>) {
    // ========================================================================
    // Array — f64
    // ========================================================================

    // jit_v2_array_new_f64(capacity: u32) -> ptr
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I32)); // capacity
        sig.returns.push(AbiParam::new(types::I64)); // ptr
        declare(module, ffi_funcs, "jit_v2_array_new_f64", &sig);
    }

    // jit_v2_array_get_f64(arr: ptr, index: i64) -> f64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // arr ptr
        sig.params.push(AbiParam::new(types::I64)); // index
        sig.returns.push(AbiParam::new(types::F64)); // NATIVE F64
        declare(module, ffi_funcs, "jit_v2_array_get_f64", &sig);
    }

    // jit_v2_array_set_f64(arr: ptr, index: i64, val: f64) -> void
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // arr ptr
        sig.params.push(AbiParam::new(types::I64)); // index
        sig.params.push(AbiParam::new(types::F64)); // val NATIVE F64
        declare(module, ffi_funcs, "jit_v2_array_set_f64", &sig);
    }

    // jit_v2_array_push(arr: ptr, bits: i64, elem_size: i8) -> void
    // Generic typed-array push dispatcher (R7.2 consolidation). Callers zero/
    // sign-extend the element value to I64 before the call and pass the byte
    // size as an I8 immediate; the FFI body routes to the matching
    // `TypedArray::push` instantiation.
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // arr ptr
        sig.params.push(AbiParam::new(types::I64)); // elem bits
        sig.params.push(AbiParam::new(types::I8));  // elem byte size
        declare(module, ffi_funcs, "jit_v2_array_push", &sig);
    }

    // jit_v2_array_len_f64(arr: ptr) -> u32
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // arr ptr
        sig.returns.push(AbiParam::new(types::I32)); // len
        declare(module, ffi_funcs, "jit_v2_array_len_f64", &sig);
    }

    // ========================================================================
    // Array — i64
    // ========================================================================

    // jit_v2_array_new_i64(capacity: u32) -> ptr
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I32));
        sig.returns.push(AbiParam::new(types::I64));
        declare(module, ffi_funcs, "jit_v2_array_new_i64", &sig);
    }

    // jit_v2_array_get_i64(arr: ptr, index: i64) -> i64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        declare(module, ffi_funcs, "jit_v2_array_get_i64", &sig);
    }

    // jit_v2_array_set_i64(arr: ptr, index: i64, val: i64) -> void
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        declare(module, ffi_funcs, "jit_v2_array_set_i64", &sig);
    }

    // jit_v2_array_len_i64(arr: ptr) -> u32
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I32));
        declare(module, ffi_funcs, "jit_v2_array_len_i64", &sig);
    }

    // ========================================================================
    // Array — i32
    // ========================================================================

    // jit_v2_array_new_i32(capacity: u32) -> ptr
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I32));
        sig.returns.push(AbiParam::new(types::I64));
        declare(module, ffi_funcs, "jit_v2_array_new_i32", &sig);
    }

    // jit_v2_array_get_i32(arr: ptr, index: i64) -> i32
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I32)); // NATIVE I32
        declare(module, ffi_funcs, "jit_v2_array_get_i32", &sig);
    }

    // jit_v2_array_set_i32(arr: ptr, index: i64, val: i32) -> void
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I32)); // val NATIVE I32
        declare(module, ffi_funcs, "jit_v2_array_set_i32", &sig);
    }

    // jit_v2_array_len_i32(arr: ptr) -> u32
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I32));
        declare(module, ffi_funcs, "jit_v2_array_len_i32", &sig);
    }

    // ========================================================================
    // Array — bool (encoded as u8 internally)
    // ========================================================================

    // jit_v2_array_new_bool(capacity: u32) -> ptr
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I32));
        sig.returns.push(AbiParam::new(types::I64));
        declare(module, ffi_funcs, "jit_v2_array_new_bool", &sig);
    }

    // jit_v2_array_get_bool(arr: ptr, index: i64) -> u8
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I8));
        declare(module, ffi_funcs, "jit_v2_array_get_bool", &sig);
    }

    // jit_v2_array_set_bool(arr: ptr, index: i64, val: u8) -> void
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I8));
        declare(module, ffi_funcs, "jit_v2_array_set_bool", &sig);
    }

    // jit_v2_array_len_bool(arr: ptr) -> u32
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I32));
        declare(module, ffi_funcs, "jit_v2_array_len_bool", &sig);
    }

    // ========================================================================
    // Array — *const StringObj / *const DecimalObj (v2-raw heap-element)
    // ========================================================================
    //
    // ADR-006 §2.7.5 + §2.7.24 Q25.A SUPERSEDED + audit deliverable (b)
    // §4.1.B — v2-raw `TypedArray<*const StringObj>` / `TypedArray<*const
    // DecimalObj>` heap-element allocators. Phase 3 cluster-0+1 Wave 3
    // Stabilize Round 2 V3-S5 ckpt-6-prime Group X JIT FFI String/Decimal
    // BUILD (2026-05-15). Mirrors `jit_v2_array_new_<scalar>` ABI:
    // `(capacity: u32) -> *mut TypedArray<*const T>` (carrier returned as
    // I64 raw bits).

    // jit_new_typed_array_string(capacity: u32) -> ptr
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I32)); // capacity
        sig.returns.push(AbiParam::new(types::I64)); // *mut TypedArray<*const StringObj>
        declare(module, ffi_funcs, "jit_new_typed_array_string", &sig);
    }

    // jit_new_typed_array_decimal(capacity: u32) -> ptr
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I32)); // capacity
        sig.returns.push(AbiParam::new(types::I64)); // *mut TypedArray<*const DecimalObj>
        declare(module, ffi_funcs, "jit_new_typed_array_decimal", &sig);
    }

    // ========================================================================
    // Struct field access
    // ========================================================================

    // jit_v2_field_load_f64(ptr, offset: u32) -> f64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // ptr
        sig.params.push(AbiParam::new(types::I32)); // offset
        sig.returns.push(AbiParam::new(types::F64));
        declare(module, ffi_funcs, "jit_v2_field_load_f64", &sig);
    }

    // jit_v2_field_load_i64(ptr, offset: u32) -> i64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I32));
        sig.returns.push(AbiParam::new(types::I64));
        declare(module, ffi_funcs, "jit_v2_field_load_i64", &sig);
    }

    // jit_v2_field_load_i32(ptr, offset: u32) -> i32
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I32));
        sig.returns.push(AbiParam::new(types::I32));
        declare(module, ffi_funcs, "jit_v2_field_load_i32", &sig);
    }

    // jit_v2_field_load_ptr(ptr, offset: u32) -> ptr
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I32));
        sig.returns.push(AbiParam::new(types::I64));
        declare(module, ffi_funcs, "jit_v2_field_load_ptr", &sig);
    }

    // jit_v2_field_store_f64(ptr, offset: u32, val: f64) -> void
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I32));
        sig.params.push(AbiParam::new(types::F64));
        declare(module, ffi_funcs, "jit_v2_field_store_f64", &sig);
    }

    // jit_v2_field_store_i64(ptr, offset: u32, val: i64) -> void
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I32));
        sig.params.push(AbiParam::new(types::I64));
        declare(module, ffi_funcs, "jit_v2_field_store_i64", &sig);
    }

    // jit_v2_field_store_i32(ptr, offset: u32, val: i32) -> void
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I32));
        sig.params.push(AbiParam::new(types::I32));
        declare(module, ffi_funcs, "jit_v2_field_store_i32", &sig);
    }

    // jit_v2_field_store_ptr(ptr, offset: u32, val: ptr) -> void
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I32));
        sig.params.push(AbiParam::new(types::I64));
        declare(module, ffi_funcs, "jit_v2_field_store_ptr", &sig);
    }

    // ========================================================================
    // Refcount
    // ========================================================================

    // jit_v2_retain(ptr) -> void
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        declare(module, ffi_funcs, "jit_v2_retain", &sig);
    }

    // jit_v2_release(ptr) -> void
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        declare(module, ffi_funcs, "jit_v2_release", &sig);
    }

    // ========================================================================
    // Struct allocation
    // ========================================================================

    // jit_v2_alloc_struct(size: u32, kind: u16) -> ptr
    // Note: u16 is promoted to i32 in C ABI
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I32)); // size
        sig.params.push(AbiParam::new(types::I32)); // kind (u16 promoted to i32)
        sig.returns.push(AbiParam::new(types::I64)); // ptr
        declare(module, ffi_funcs, "jit_v2_alloc_struct", &sig);
    }

    // ========================================================================
    // SIMD reductions (Phase C.3)
    // ========================================================================

    // jit_v2_array_sum_f64(arr: ptr) -> f64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // arr ptr
        sig.returns.push(AbiParam::new(types::F64)); // NATIVE F64
        declare(module, ffi_funcs, "jit_v2_array_sum_f64", &sig);
    }

    // jit_v2_array_sum_i64(arr: ptr) -> i64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // arr ptr
        sig.returns.push(AbiParam::new(types::I64));
        declare(module, ffi_funcs, "jit_v2_array_sum_i64", &sig);
    }

    // ========================================================================
    // SIMD reductions — min / max / mean / sum-of-squares (f64)
    // ========================================================================

    // jit_v2_array_min_f64(arr: ptr) -> f64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::F64));
        declare(module, ffi_funcs, "jit_v2_array_min_f64", &sig);
    }

    // jit_v2_array_max_f64(arr: ptr) -> f64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::F64));
        declare(module, ffi_funcs, "jit_v2_array_max_f64", &sig);
    }

    // jit_v2_array_mean_f64(arr: ptr) -> f64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::F64));
        declare(module, ffi_funcs, "jit_v2_array_mean_f64", &sig);
    }

    // jit_v2_array_sum_squares_f64(arr: ptr) -> f64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::F64));
        declare(module, ffi_funcs, "jit_v2_array_sum_squares_f64", &sig);
    }

    // ========================================================================
    // SIMD element-wise scalar ops (allocating, f64)
    // ========================================================================

    // jit_v2_array_scale_f64(arr: ptr, factor: f64) -> ptr
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::F64));
        sig.returns.push(AbiParam::new(types::I64));
        declare(module, ffi_funcs, "jit_v2_array_scale_f64", &sig);
    }

    // jit_v2_array_add_scalar_f64(arr: ptr, offset: f64) -> ptr
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::F64));
        sig.returns.push(AbiParam::new(types::I64));
        declare(module, ffi_funcs, "jit_v2_array_add_scalar_f64", &sig);
    }

    // ========================================================================
    // SIMD element-wise binary ops (allocating, f64)
    // ========================================================================

    // jit_v2_array_add_f64(a: ptr, b: ptr) -> ptr
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        declare(module, ffi_funcs, "jit_v2_array_add_f64", &sig);
    }

    // jit_v2_array_mul_f64(a: ptr, b: ptr) -> ptr
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        declare(module, ffi_funcs, "jit_v2_array_mul_f64", &sig);
    }

    // ========================================================================
    // Typed-Arc collection allocators (Round 9 / 8B.1, ADR-006 §2.7.5 / §2.7.25)
    // ========================================================================
    //
    // 5 zero-arg ctors: `() -> i64` (returns the raw u64 Arc::into_raw bits).
    for name in [
        "jit_v2_make_hashset",
        "jit_v2_make_hashmap",
        "jit_v2_make_deque",
        "jit_v2_make_priorityqueue",
        "jit_v2_make_channel",
    ] {
        let mut sig = module.make_signature();
        sig.returns.push(AbiParam::new(types::I64));
        declare(module, ffi_funcs, name, &sig);
    }

    // Single-kind ctors:
    // jit_v2_make_atomic(i: i64) -> i64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        declare(module, ffi_funcs, "jit_v2_make_atomic", &sig);
    }
    // jit_v2_make_lazy(closure_bits: i64) -> i64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        declare(module, ffi_funcs, "jit_v2_make_lazy", &sig);
    }

    // Carrier-pair ctor:
    // jit_v2_make_mutex(bits: i64, kind: i8) -> i64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // bits
        sig.params.push(AbiParam::new(types::I8));  // kind code
        sig.returns.push(AbiParam::new(types::I64));
        declare(module, ffi_funcs, "jit_v2_make_mutex", &sig);
    }

    // Per-HeapKind kinded retain (each takes `bits: i64` and returns void)
    // — operates on the Arc control block refcount at offset -16 per
    // Rust Arc contract, NOT the W11 UnifiedValue<T> HeapHeader at offset 4.
    for name in [
        "jit_arc_hashset_retain",
        "jit_arc_hashset_release",
        "jit_arc_hashmap_retain",
        "jit_arc_hashmap_release",
        "jit_arc_deque_retain",
        "jit_arc_deque_release",
        "jit_arc_priorityqueue_retain",
        "jit_arc_priorityqueue_release",
        "jit_arc_channel_retain",
        "jit_arc_channel_release",
        "jit_arc_mutex_retain",
        "jit_arc_mutex_release",
        "jit_arc_atomic_retain",
        "jit_arc_atomic_release",
        "jit_arc_lazy_retain",
        "jit_arc_lazy_release",
    ] {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        declare(module, ffi_funcs, name, &sig);
    }

    // ========================================================================
    // Typed HashMap<string, ...> access — SURFACE
    // ========================================================================
    //
    // SURFACE (W10 jit-playbook §5 / ADR-006 §2.7.4): the deleted
    // ValueWord-shape map FFI (jit_v2_map_get_str_i64 / get_str_f64 /
    // has_str / set_str_i64 / len) is gone — see `ffi/v2/typed_map.rs`
    // SURFACE comment. The declarations are dropped to keep the JIT
    // module link step clean; consumers that try to call these
    // symbols will fail at the Cranelift `call` lookup, which is the
    // deletion-fate signal §5 calls for. Kinded rebuild lands the
    // declarations alongside the kinded `Arc<HashMapData>` +
    // `KindedSlot` entry-points per ADR-006 §2.7.5 / §2.7.6 / Q8.
}
