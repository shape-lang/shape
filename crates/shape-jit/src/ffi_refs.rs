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

    // Array allocator + hot per-element push.
    //
    // Route A (ADR-006 §2.7.14 / W11-jit-new-array close): the kind-blind
    // `jit_new_array` / `jit_array_push_elem` FuncRefs are deleted. The
    // kinded `Arc<TypedArrayData>` allocator surface is the existing
    // `v2_array_new_<kind>` family (below), and the kinded push surface is
    // `v2_array_push` dispatched by element byte size. Call sites that
    // lack a proven element kind surface-and-stop per §2.7.5.
    //
    // Builtin print fallback (used by emit_print).
    pub(crate) print: FuncRef,
    // W11-jit-new-array (ADR-006 §2.7.5): per-kind print entry points
    // dispatched by the MIR-side print emitter when the operand's
    // `NativeKind` is statically known. The kind-blind `print` fallback
    // is reserved for receivers whose kind the MIR could not prove
    // (heap arms remain a §2.7.5 follow-up).
    pub(crate) print_i64: FuncRef,
    pub(crate) print_f64: FuncRef,
    pub(crate) print_bool: FuncRef,

    // Closure construction (Phase H2: typed closure block → Arc<Closure>).
    pub(crate) make_closure: FuncRef,
    pub(crate) finalize_heap_closure: FuncRef,
    // Track A.1D: OwnedMutable capture cell allocator. Called from
    // `emit_heap_closure` once per `CaptureKind::OwnedMutable` capture to
    // obtain the `*mut ValueWord` pointer installed into the Ptr slot.
    pub(crate) alloc_owned_mut_cell: FuncRef,
    // Track A.1E: Shared capture FFI helpers.
    //   `arc_shared_retain`          — per-capture Arc strong-count retain
    //                                  in `emit_heap_closure`'s Shared branch.
    //   `shared_lock_contended`      — spin-wait fallback for the inline
    //                                  CAS lock acquire.
    //   `shared_unlock_contended`    — release store fallback for the
    //                                  inline CAS unlock.
    pub(crate) arc_shared_retain: FuncRef,
    pub(crate) shared_lock_contended: FuncRef,
    pub(crate) shared_unlock_contended: FuncRef,

    // Session 1 Commit 3: outer-scope Shared-cell lifecycle helpers.
    //   `alloc_shared_cell`          — `Arc<SharedCell>` allocator called
    //                                   from `MirToIR::initialize_shared_local_slots`
    //                                   to materialise the cell behind a
    //                                   SharedCow local slot at function
    //                                   entry. Mirrors the interpreter's
    //                                   `op_alloc_shared_local`.
    //   `arc_shared_release`         — matching release, called from
    //                                   `emit_drop` on SharedCow local
    //                                   slots.  Mirrors `op_drop_shared_local`.
    pub(crate) alloc_shared_cell: FuncRef,
    pub(crate) arc_shared_release: FuncRef,

    // Wave C.1: per-FieldKind closure-cell FFI helpers (D1 native ABI).
    // 33 OwnedMutable handles (alloc/read/write × 11 FieldKinds) + 22
    // Shared handles (read/write × 11 FieldKinds) = 55 FuncRefs. Cell
    // pointers cross the FFI boundary as `i64`; payloads use native
    // Cranelift types where direct (F64/I64), I32 for 4-byte ints, and
    // I32 widened from sub-32 (i16/u16/i8/u8/bool). See
    // `crates/shape-jit/src/ffi/object/closure.rs` for ABI details.
    //
    // OwnedMutable allocators
    pub(crate) alloc_owned_mut_cell_i64: FuncRef,
    pub(crate) alloc_owned_mut_cell_u64: FuncRef,
    pub(crate) alloc_owned_mut_cell_f64: FuncRef,
    pub(crate) alloc_owned_mut_cell_i32: FuncRef,
    pub(crate) alloc_owned_mut_cell_u32: FuncRef,
    pub(crate) alloc_owned_mut_cell_i16: FuncRef,
    pub(crate) alloc_owned_mut_cell_u16: FuncRef,
    pub(crate) alloc_owned_mut_cell_i8: FuncRef,
    pub(crate) alloc_owned_mut_cell_u8: FuncRef,
    pub(crate) alloc_owned_mut_cell_bool: FuncRef,
    pub(crate) alloc_owned_mut_cell_ptr: FuncRef,
    // OwnedMutable readers
    pub(crate) read_owned_mut_cell_i64: FuncRef,
    pub(crate) read_owned_mut_cell_u64: FuncRef,
    pub(crate) read_owned_mut_cell_f64: FuncRef,
    pub(crate) read_owned_mut_cell_i32: FuncRef,
    pub(crate) read_owned_mut_cell_u32: FuncRef,
    pub(crate) read_owned_mut_cell_i16: FuncRef,
    pub(crate) read_owned_mut_cell_u16: FuncRef,
    pub(crate) read_owned_mut_cell_i8: FuncRef,
    pub(crate) read_owned_mut_cell_u8: FuncRef,
    pub(crate) read_owned_mut_cell_bool: FuncRef,
    pub(crate) read_owned_mut_cell_ptr: FuncRef,
    // OwnedMutable writers
    pub(crate) write_owned_mut_cell_i64: FuncRef,
    pub(crate) write_owned_mut_cell_u64: FuncRef,
    pub(crate) write_owned_mut_cell_f64: FuncRef,
    pub(crate) write_owned_mut_cell_i32: FuncRef,
    pub(crate) write_owned_mut_cell_u32: FuncRef,
    pub(crate) write_owned_mut_cell_i16: FuncRef,
    pub(crate) write_owned_mut_cell_u16: FuncRef,
    pub(crate) write_owned_mut_cell_i8: FuncRef,
    pub(crate) write_owned_mut_cell_u8: FuncRef,
    pub(crate) write_owned_mut_cell_bool: FuncRef,
    pub(crate) write_owned_mut_cell_ptr: FuncRef,
    // Shared readers (alloc/release reuse `alloc_shared_cell` /
    // `arc_shared_release` above).
    pub(crate) read_shared_cell_i64: FuncRef,
    pub(crate) read_shared_cell_u64: FuncRef,
    pub(crate) read_shared_cell_f64: FuncRef,
    pub(crate) read_shared_cell_i32: FuncRef,
    pub(crate) read_shared_cell_u32: FuncRef,
    pub(crate) read_shared_cell_i16: FuncRef,
    pub(crate) read_shared_cell_u16: FuncRef,
    pub(crate) read_shared_cell_i8: FuncRef,
    pub(crate) read_shared_cell_u8: FuncRef,
    pub(crate) read_shared_cell_bool: FuncRef,
    pub(crate) read_shared_cell_ptr: FuncRef,
    // Shared writers
    pub(crate) write_shared_cell_i64: FuncRef,
    pub(crate) write_shared_cell_u64: FuncRef,
    pub(crate) write_shared_cell_f64: FuncRef,
    pub(crate) write_shared_cell_i32: FuncRef,
    pub(crate) write_shared_cell_u32: FuncRef,
    pub(crate) write_shared_cell_i16: FuncRef,
    pub(crate) write_shared_cell_u16: FuncRef,
    pub(crate) write_shared_cell_i8: FuncRef,
    pub(crate) write_shared_cell_u8: FuncRef,
    pub(crate) write_shared_cell_bool: FuncRef,
    pub(crate) write_shared_cell_ptr: FuncRef,

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

    // F5.a/F5.b: string `+` lowering.
    //   `string_concat` — takes two NaN-boxed string operands (`u64`) and
    //   returns a fresh unified-heap string (`u64`). Called from
    //   `mir_compiler::rvalues::compile_rvalue` when both operand slots
    //   carry `NativeKind::String` under `BinOp::Add`. Covers the desugared
    //   chain emitted by `f"..."` formatted strings as well.
    pub(crate) string_concat: FuncRef,

    // ADR-006 §2.7.5 — kinded EnumStore producers
    // (W12-jit-aggregate-non-array, 2026-05-12). Three entry points
    // matching the VM-side `BuiltinFunction::OkCtor` / `ErrCtor` /
    // `SomeCtor` shapes (`crates/shape-vm/src/executor/vm_impl/
    // builtins.rs:551-586`). The JIT-side bodies use the existing
    // `box_ok` / `box_err` / `box_some` heap-pointer encoding (legacy
    // NaN-box shape with HK_OK / HK_ERR / HK_SOME prefix); conversion
    // to the post-strict-typing `Arc<ResultData>` / `Arc<OptionData>`
    // carrier happens at the JIT↔VM boundary via the existing
    // `jit_bits_to_nanboxed` conversion infrastructure
    // (`crates/shape-jit/src/ffi/conversion.rs:246-258` — same path as
    // `jit_unwrap_ok` / `jit_is_ok` etc.).
    //
    // The JIT EnumStore consumer dispatches on the MIR statement's
    // `variant_name` field to pick the right entry. Slot kind stamped
    // from the conduit (`concrete_types[container_slot]` →
    // `Ptr(HeapKind::Result)` / `Ptr(HeapKind::Option)`); no
    // Bool-default per §2.7.7 #9.
    pub(crate) make_ok: FuncRef,
    pub(crate) make_err: FuncRef,
    pub(crate) make_some: FuncRef,

    // ADR-006 §2.7.17 / Q18 — Arc-shape Result/Option producers
    // (W12-jit-result-option-trinity, Phase 3 cluster-0 Round 7A,
    // 2026-05-12). These produce `Arc::into_raw(Arc<ResultData>) as u64`
    // / `Arc::into_raw(Arc<OptionData>) as u64` directly per the strict-
    // typed §2.7.17 carrier — matching the VM-side `BuiltinFunction::
    // OkCtor` / `ErrCtor` / `SomeCtor` / `NoneCtor` output. The producer
    // signature is `(payload_bits: u64, payload_kind_code: u8) -> u64`
    // where the kind code is the §2.7.7 / Q9 parallel-track byte
    // (`stack_kind_code::encode(payload_kind)`) stamped at JIT-compile
    // time from the EnumStore operand's MIR-inferred kind. Replaces the
    // legacy `make_ok` / `make_err` / `make_some` NaN-box family at the
    // strict-typed EnumStore consumer (those FFI fields above remain
    // referenced by ffi/conversion.rs for the JIT↔VM trampoline boundary).
    pub(crate) v2_make_result_ok: FuncRef,
    pub(crate) v2_make_result_err: FuncRef,
    pub(crate) v2_make_option_some: FuncRef,
    pub(crate) v2_make_option_none: FuncRef,

    // ADR-006 §2.7.17 — Arc-shape Result/Option predicates + payload
    // extractors. Read `is_ok` / `is_some` from the `*const ResultData` /
    // `*const OptionData` borrow directly — NO NaN-box tag decode, NO
    // `is_heap_kind` probe (§2.7.7 #4 / #7 forbidden per CLAUDE.md
    // "Forbidden code" — runtime tag_bits dispatch deleted with the
    // W-series). Used by the JIT `Rvalue::EnumTest` / `Rvalue::EnumPayload`
    // consumer in `mir_compiler/rvalues.rs`.
    pub(crate) arc_result_is_ok: FuncRef,
    pub(crate) arc_result_is_err: FuncRef,
    pub(crate) arc_result_payload: FuncRef,
    pub(crate) arc_option_is_some: FuncRef,
    pub(crate) arc_option_is_none: FuncRef,
    pub(crate) arc_option_payload: FuncRef,

    // Arc-shape kinded retain/release for `Arc<ResultData>` /
    // `Arc<OptionData>` carriers (W12-jit-result-option-trinity,
    // 2026-05-12). The legacy `arc_retain` / `arc_release` operate on
    // the `UnifiedValue<T>` refcount layout (offset 4) and corrupt the
    // typed-Arc allocations (whose refcount lives at offset -16 per
    // Rust Arc contract). Refcount sites for Result/Option-kinded
    // slots dispatch HERE instead of the legacy entries.
    pub(crate) arc_result_retain: FuncRef,
    pub(crate) arc_result_release: FuncRef,
    pub(crate) arc_option_retain: FuncRef,
    pub(crate) arc_option_release: FuncRef,

    // v2 typed HashMap<string, ...> access.
    //
    // SURFACE (ADR-006 §2.7.14 Q15 / W11-jit-carrier-conversion sub-cluster):
    // the kind-blind `jit_v2_map_*` symbols (deleted ValueWord-shape map FFI)
    // are gated on the kinded `Arc<HashMapData>` + `KindedSlot` rebuild. The
    // FuncRef slots are deleted from this struct in lockstep with the v2_map
    // call-site surface-and-stop in `mir_compiler/v2_typed_map.rs`. The
    // declarations in `ffi_symbols/v2_symbols.rs::declare_v2_functions` are
    // already a no-op for the same set.
}
