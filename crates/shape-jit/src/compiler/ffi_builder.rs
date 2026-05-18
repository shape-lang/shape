//! FFI function reference building.
//!
//! Populates `FFIFuncRefs` with Cranelift `FuncRef` handles for the subset
//! of FFI entry points the v2 JIT codegen pipeline actually references.
//! Historically this builder declared ~240 functions; the V6 cleanup pruned
//! it to the live native-typed surface.
//!
//! ## Route A close (ADR-006 §2.7.14 / W11-jit-new-array)
//!
//! W11-jit-new-array adopted Route A: kinded `Arc<TypedArrayData>`
//! per-element-kind monomorphization. The kind-blind `jit_new_array` /
//! `jit_array_push_elem` symbols (the deleted ValueWord-shape ABI) are
//! no longer referenced from `FFIFuncRefs`. Array allocation routes
//! through the per-kind `v2_array_new_<f64,i64,i32,bool>` allocators and
//! the size-dispatched `v2_array_push` push helper. Consumers that lack
//! a proven element kind surface-and-stop at JIT compile time per the
//! `Route A surface-and-stop` marker in the MIR lowering — the
//! kind-blind fallback would resurrect the deleted UnifiedArray heap
//! layout (§2.7.14 forbidden list).
//!
//! `r!()` keeps the per-key fallback so that any future agent who
//! re-introduces a deleted symbol gets a structured `RuntimeError`
//! at JIT-init time instead of an unhandled `HashMap` index panic.
//! Map-FFI consumers (the `jit_v2_map_*` family) remain on the same
//! Q15-style deferral until W11-jit-carrier-conversion lands.

use super::setup::JITCompiler;
use crate::ffi_refs::FFIFuncRefs;
use cranelift::prelude::*;
use cranelift_module::Module;

impl JITCompiler {
    #[inline(always)]
    pub(super) fn build_ffi_refs(
        &mut self,
        builder: &mut FunctionBuilder,
    ) -> Result<FFIFuncRefs, String> {
        // Helper: declare an already-registered FFI function by key. On a
        // missing key we return an Err carrying the §2.7.14 marker so the
        // caller can propagate a clean RuntimeError instead of panicking
        // on `HashMap::index`.
        macro_rules! r {
            ($key:expr) => {{
                let key: &str = $key;
                match self.ffi_funcs.get(key) {
                    Some(&func_id) => self.module.declare_func_in_func(func_id, builder.func),
                    None => {
                        return Err(format!(
                            "phase-2c §2.7.14 / W10 jit-playbook §5: JitArray rebuild required \
                             for JIT execution path — FFI symbol `{}` is not registered. \
                             The deleted UnifiedArray / JitArray heap layout removed the \
                             implementations behind several `register_*_symbols` modules \
                             (array_symbols, arc_symbols, v2 typed-map family). The kinded \
                             `TypedArray<T>` rebuild (ADR-006 §2.7.14 Q15) re-introduces \
                             these entries with element/key kinds threaded from the JIT \
                             call signature per §2.7.5. See \
                             docs/adr/006-value-and-memory-model.md §2.7.14.",
                            key
                        ));
                    }
                }
            }};
        }

        Ok(FFIFuncRefs {
            // Object / property access
            get_prop: r!("jit_get_prop"),
            set_prop: r!("jit_set_prop"),

            // Call dispatch
            call_value: r!("jit_call_value"),
            call_method: r!("jit_call_method"),

            // Arrays
            //
            // Route A (ADR-006 §2.7.14 / W11-jit-new-array close): the
            // kind-blind `jit_new_array` / `jit_array_push_elem` symbols are
            // deleted. Allocation routes through `v2_array_new_<kind>`
            // (registered below) per the producing call signature's element
            // kind; push routes through the kinded `v2_array_push`
            // dispatcher. Consumers that lack a kind source surface-and-stop
            // at JIT compile time per §2.7.5.
            //
            // Kinded print entries (W11-jit-new-array scalar arms +
            // W12-jit-print-heap-arm-classification heap arms). The
            // kind-blind `r!("jit_print")` lookup DELETED in Round 8A
            // reopen (2026-05-13).
            print_i64: r!("jit_print_i64"),
            print_f64: r!("jit_print_f64"),
            print_bool: r!("jit_print_bool"),
            // W12-jit-print-heap-arm-classification (Phase 3 cluster-0
            // Round 8A, 2026-05-13): heap-arm kinded print entries —
            // ADR-006 §2.7.5 stamp-at-compile-time. The MIR-side
            // Call-terminator dispatch in `mir_compiler/terminators.rs`
            // routes the operand's `NativeKind` to the matching FuncRef
            // (`String` → `print_str`, `Ptr(HeapKind::TypedObject)` →
            // `print_typed_object`, `Ptr(HeapKind::Option)` →
            // `print_option`, `Ptr(HeapKind::Result)` → `print_result`).
            // Each entry takes `(ctx_ptr, bits)` so the FFI body can
            // resolve the type schema registry for TypedObject field-
            // name rendering and route through the canonical VM-side
            // `ValueFormatter::format_kinded`.
            print_str: r!("jit_print_str"),
            print_typed_object: r!("jit_print_typed_object"),
            print_option: r!("jit_print_option"),
            print_result: r!("jit_print_result"),
            // Phase 3 cluster-2 Round 3 cw-D-fam12 (2026-05-16): Scalar
            // Char + Concurrency Mutex/Atomic/Lazy/Channel kinded print
            // entries. ADR-006 §2.7.5 stamp-at-compile-time + §2.7.25
            // concurrency-primitive printing convention. The MIR-side
            // Call-terminator dispatch routes the operand's `NativeKind`
            // to the matching FuncRef.
            print_char: r!("jit_print_char"),
            print_mutex: r!("jit_print_mutex"),
            print_atomic: r!("jit_print_atomic"),
            print_lazy: r!("jit_print_lazy"),
            print_channel: r!("jit_print_channel"),
            // Phase 3 cluster-2 Round 4 cw-D-fam3 (2026-05-16):
            // Collection family kinded print entries. ADR-006 §2.7.5
            // stamp-at-compile-time + §2.7.5.B Family 3 amendment
            // extension. The MIR-side Call-terminator dispatch routes
            // the operand's `NativeKind` to the matching FuncRef
            // (`Ptr(HeapKind::HashMap)` → `print_hashmap`, etc.).
            // ADR-006 §2.7.5.B 2026-05-16
            print_hashmap: r!("jit_print_hashmap"),
            print_hashset: r!("jit_print_hashset"),
            print_deque: r!("jit_print_deque"),
            print_priority_queue: r!("jit_print_priority_queue"),
            print_range: r!("jit_print_range"),
            print_iterator: r!("jit_print_iterator"),
            // W11-fup-C (Phase 3d, 2026-05-18): v2-raw TypedArray<T> kinded
            // print entry (ADR-006 §2.7.5). The MIR-side Call-terminator
            // dispatch routes operands with stamped
            // `NativeKind::Ptr(HeapKind::TypedArray)` to this FuncRef.
            print_typed_array: r!("jit_print_typed_array"),

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
            // ckpt-6-prime Group X JIT FFI String/Decimal BUILD (2026-05-15)
            v2_array_new_string: r!("jit_new_typed_array_string"),
            v2_array_new_decimal: r!("jit_new_typed_array_decimal"),

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
            // NativeKind::String operands, incl. f-string desugared chains).
            string_concat: r!("jit_string_concat"),

            // W11-fup-A (Phase 3d, 2026-05-18): typed-pow FFI entry points
            // for the MIR `BinOp::Pow` JIT codegen path. Native f64 and
            // i64 ABI; symbols registered in `ffi_symbols/math_symbols.rs`.
            pow_f64: r!("jit_pow_f64"),
            pow_i64: r!("jit_pow_i64"),

            // ADR-006 §2.7.5 — kinded EnumStore producers
            // (W12-jit-aggregate-non-array, 2026-05-12). Dispatched from
            // the EnumStore consumer based on the MIR statement's
            // `variant_name` field.
            make_ok: r!("jit_make_ok"),
            make_err: r!("jit_make_err"),
            make_some: r!("jit_make_some"),

            // ADR-006 §2.7.17 / Q18 — Arc-shape Result/Option producers +
            // accessors (W12-jit-result-option-trinity, Phase 3 cluster-0
            // Round 7A, 2026-05-12). These are the trinity's strict-typed
            // EnumStore producers + match-codegen consumers — the legacy
            // `make_ok` / `make_err` / `make_some` NaN-box family is NOT
            // referenced by the new trinity codegen path (those FuncRefs
            // remain for the JIT↔VM trampoline conversion in
            // ffi/conversion.rs only).
            v2_make_result_ok: r!("jit_v2_make_result_ok"),
            v2_make_result_err: r!("jit_v2_make_result_err"),
            v2_make_option_some: r!("jit_v2_make_option_some"),
            v2_make_option_none: r!("jit_v2_make_option_none"),
            arc_result_is_ok: r!("jit_arc_result_is_ok"),
            arc_result_is_err: r!("jit_arc_result_is_err"),
            arc_result_payload: r!("jit_arc_result_payload"),
            arc_option_is_some: r!("jit_arc_option_is_some"),
            arc_option_is_none: r!("jit_arc_option_is_none"),
            arc_option_payload: r!("jit_arc_option_payload"),
            arc_result_retain: r!("jit_arc_result_retain"),
            arc_result_release: r!("jit_arc_result_release"),
            arc_option_retain: r!("jit_arc_option_retain"),
            arc_option_release: r!("jit_arc_option_release"),

            // ADR-006 §2.7.5 / §2.7.25 — Typed-Arc collection allocators
            // (W12-jit-collection-arc-ffi-ctors-and-refcount, Phase 3
            // cluster-0 Round 9 / 8B.1, 2026-05-13). Bodies in
            // `ffi/v2/collection_arc.rs` — each produces
            // `Arc::into_raw(Arc<XData>) as u64`. The MIR EnumStore
            // consumer for collection ctors dispatches here (Round 10
            // / 8B.2 work — until then these are inert at the program
            // surface but the FuncRefs are populated so consumers can
            // pre-resolve them).
            v2_make_hashset: r!("jit_v2_make_hashset"),
            v2_make_hashmap: r!("jit_v2_make_hashmap"),
            v2_make_deque: r!("jit_v2_make_deque"),
            v2_make_priorityqueue: r!("jit_v2_make_priorityqueue"),
            v2_make_channel: r!("jit_v2_make_channel"),
            v2_make_atomic: r!("jit_v2_make_atomic"),
            v2_make_lazy: r!("jit_v2_make_lazy"),
            v2_make_mutex: r!("jit_v2_make_mutex"),

            // ADR-006 §2.7.5 / §2.7.17 — Per-HeapKind kinded retain/
            // release for the 8 typed-Arc collection carriers. The
            // ownership-aware codegen path in `mir_compiler/ownership.rs`
            // (`retain_func_for_place` / `release_func_for_place`)
            // dispatches per slot's proven `NativeKind::Ptr(HeapKind::*)`
            // to the matching FuncRef. The legacy `arc_retain` /
            // `arc_release` fallback stays for kinds NOT in the
            // typed-Arc family (Array / TypedObject / etc. still use
            // the `UnifiedValue<T>` HeapHeader-refcount shape).
            arc_hashset_retain: r!("jit_arc_hashset_retain"),
            arc_hashset_release: r!("jit_arc_hashset_release"),
            arc_hashmap_retain: r!("jit_arc_hashmap_retain"),
            arc_hashmap_release: r!("jit_arc_hashmap_release"),
            arc_deque_retain: r!("jit_arc_deque_retain"),
            arc_deque_release: r!("jit_arc_deque_release"),
            arc_priorityqueue_retain: r!("jit_arc_priorityqueue_retain"),
            arc_priorityqueue_release: r!("jit_arc_priorityqueue_release"),
            arc_channel_retain: r!("jit_arc_channel_retain"),
            arc_channel_release: r!("jit_arc_channel_release"),
            arc_mutex_retain: r!("jit_arc_mutex_retain"),
            arc_mutex_release: r!("jit_arc_mutex_release"),
            arc_atomic_retain: r!("jit_arc_atomic_retain"),
            arc_atomic_release: r!("jit_arc_atomic_release"),
            arc_lazy_retain: r!("jit_arc_lazy_retain"),
            arc_lazy_release: r!("jit_arc_lazy_release"),

            // ADR-006 §2.7.5 — `Arc<String>` strict-typed carrier
            // retain/release (W12-jit-string-carrier-unification, Phase 3
            // cluster-0 Round 12 T2/T3, 2026-05-13). Bodies in
            // `ffi/string.rs`. Refcount discipline at slots whose
            // `NativeKind` is `String` dispatches here per the
            // `ownership::retain_func_for_place` / `release_func_for_place`
            // String arm.
            arc_string_retain: r!("jit_arc_string_retain"),
            arc_string_release: r!("jit_arc_string_release"),

            // v2 typed HashMap<string, ...>
            //
            // SURFACE (ADR-006 §2.7.14 Q15 / W11-jit-carrier-conversion):
            // gated on the kinded `Arc<HashMapData>` + `KindedSlot` rebuild;
            // FuncRef slots and r!() lookups are deleted in lockstep with
            // the v2_typed_map call-site surface-and-stop.
        })
    }
}
