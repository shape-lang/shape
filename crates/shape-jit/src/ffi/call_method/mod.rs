// Heap allocation audit (PR-9 V8 Gap Closure):
//   Category A (NaN-boxed returns): 2 sites
//     jit_box(HK_JIT_OBJECT, ...) — group/groupBy result object
//     jit_box(HK_ARRAY, ...) — group values inside object
//   Category B (intermediate/consumed): 0 sites
//   Category C (heap islands): 1 site (group/groupBy)
//!
//! Method Call FFI Functions for JIT
//!
//! Dispatches method calls on various types (array, string, object, series, etc.)
//! Split into type-specific helper modules for maintainability.

use crate::context::JITContext;
// crate::jit_array::JitArray removed — see jit_array.rs SURFACE comment.
// Method dispatch on HK_ARRAY receivers surfaces per ADR-006 §2.7.4 /
// W10 jit-playbook §5.
use crate::ffi::jit_kinds::*;
use crate::ffi::value_ffi::*;
use shape_runtime::context::ExecutionContext;
use std::collections::HashMap;

// Module declarations
pub mod array;
pub mod duration;
pub mod matrix;
pub mod number;
pub mod object;
pub mod result;
pub mod string;
pub mod time;

// Re-export the individual method handlers
pub use array::call_array_method;
pub use duration::call_duration_method;
pub use matrix::call_matrix_method;
pub use number::call_number_method;
pub use object::call_object_method;
pub use result::call_result_method;
pub use string::call_string_method;
pub use time::call_time_method;

// ============================================================================
// User-Defined Method Support
// ============================================================================

/// Determine the type name of a JIT NaN-boxed receiver value.
///
/// For TypedObjects, uses the schema_id to look up the type name from the
/// ExecutionContext's type_schema_registry. For other types, returns a static
/// type name string.
unsafe fn receiver_type_name(receiver_bits: u64, exec_ctx: &ExecutionContext) -> Option<String> {
    use crate::ffi::typed_object::jit_typed_object_schema_id;

    if is_number(receiver_bits) {
        return Some("number".to_string());
    }
    if receiver_bits == TAG_BOOL_TRUE || receiver_bits == TAG_BOOL_FALSE {
        return Some("bool".to_string());
    }
    if receiver_bits == TAG_NULL || receiver_bits == TAG_NONE {
        return None;
    }

    match heap_kind(receiver_bits) {
        Some(HK_STRING) => Some("string".to_string()),
        Some(HK_ARRAY) => Some("Array".to_string()),
        Some(HK_TYPED_OBJECT) => {
            // Look up the schema name from the type_schema_registry
            let schema_id = jit_typed_object_schema_id(receiver_bits);
            if schema_id == 0 {
                return None;
            }
            let registry = exec_ctx.type_schema_registry();
            registry.get_by_id(schema_id).map(|s| s.name.clone())
        }
        Some(HK_JIT_OBJECT) => Some("object".to_string()),
        Some(HK_DURATION) => Some("Duration".to_string()),
        Some(HK_TIME) => Some("DateTime".to_string()),
        _ => None,
    }
}

/// Search the JITContext's function_names table for a function with the given
/// UFCS name (e.g. "Point::distance") and return its index.
unsafe fn find_function_by_name(ctx_ref: &JITContext, ufcs_name: &str) -> Option<usize> {
    if ctx_ref.function_names_ptr.is_null() || ctx_ref.function_names_len == 0 {
        return None;
    }
    let names = unsafe {
        std::slice::from_raw_parts(ctx_ref.function_names_ptr, ctx_ref.function_names_len)
    };
    for (idx, name) in names.iter().enumerate() {
        if name == ufcs_name {
            return Some(idx);
        }
    }
    None
}

/// Try to call a user-defined method from impl blocks via UFCS dispatch.
///
/// User-defined methods (from `extend` / `impl` blocks) are compiled as functions
/// named `"TypeName::method_name"`. This function:
/// 1. Determines the receiver type name from the NaN-boxed bits
/// 2. Constructs the UFCS name `"TypeName::method_name"`
/// 3. Looks up the function index in function_names
/// 4. Calls the function via function_table, passing (receiver, ...args)
/// 5. Returns the result as NaN-boxed u64
///
/// Returns Some(result) if the method was found and executed, None otherwise.
unsafe fn try_call_user_method(
    ctx: *const JITContext,
    receiver_bits: u64,
    method_name: &str,
    args: &[u64],
) -> Option<u64> {
    let ctx_ref = unsafe { &*ctx };

    // Need execution context to access the type schema registry
    if ctx_ref.exec_context_ptr.is_null() {
        return None;
    }
    let exec_ctx = unsafe { &*(ctx_ref.exec_context_ptr as *const ExecutionContext) };

    // Determine the receiver's type name
    let type_name = unsafe { receiver_type_name(receiver_bits, exec_ctx) }?;

    // Construct UFCS function name: "TypeName::method_name"
    let ufcs_name = format!("{}::{}", type_name, method_name);

    // Look up the function index in the JIT function table
    let func_idx = unsafe { find_function_by_name(ctx_ref, &ufcs_name) }?;

    // Check that we have a valid function table entry
    if ctx_ref.function_table.is_null() || func_idx >= ctx_ref.function_table_len {
        return None;
    }

    // Read the raw pointer from the function table. A null entry means the
    // function was not JIT-compiled (interpreted only).
    let raw_fn_ptr = unsafe { *(ctx_ref.function_table as *const *const u8).add(func_idx) };
    if raw_fn_ptr.is_null() {
        return None;
    }
    let fn_ptr = unsafe { *ctx_ref.function_table.add(func_idx) };

    // Push receiver + args onto the JIT stack for the function call.
    // UFCS convention: first parameter is `self` (the receiver), then the rest.
    let ctx_mut = unsafe { &mut *(ctx as *mut JITContext) };
    ctx_mut.stack[ctx_mut.stack_ptr] = receiver_bits;
    ctx_mut.stack_ptr += 1;
    for &arg in args {
        ctx_mut.stack[ctx_mut.stack_ptr] = arg;
        ctx_mut.stack_ptr += 1;
    }

    // Call the JIT-compiled function
    let _result_code = unsafe { fn_ptr(ctx_mut) };

    // Pop result from stack
    if ctx_mut.stack_ptr > 0 {
        ctx_mut.stack_ptr -= 1;
        Some(ctx_mut.stack[ctx_mut.stack_ptr])
    } else {
        Some(TAG_NULL)
    }
}

// ============================================================================
// Main Dispatcher
// ============================================================================
//
// W12-jit-call-method-shell-rebuild (Phase 3 cluster-0 Round 10 / 8B.2,
// 2026-05-13). The shell now reads receiver + args kinds from the
// §2.7.7 / Q9 `JITContext.stack_kinds` parallel-kind track at every pop,
// per the producer-side classification at MIR-emit time
// (`mir_compiler/terminators.rs:202-247`). When the receiver kind decodes
// to a delegated-to-VM kind (the 8 Round 9 typed-Arc collection kinds +
// Round 7A Result/Option Arc carriers + scalar kinds for unified VM
// method dispatch), the shell builds `(u64, NativeKind)` pair-slices and
// calls into the new public `VirtualMachine::jit_trampoline_call_method`
// (sibling to `jit_trampoline_call_closure` at
// `crates/shape-vm/src/executor/call_convention.rs`) — the §2.7.5
// cross-crate stable FFI consumer.
//
// **Deleted in this rebuild:**
//
// - The kind-blind `heap_kind(receiver_bits)`-driven NaN-box dispatch
//   cascade (pre-§2.7.10 `match heap_kind(receiver_bits)` at the prior
//   shell body) — forbidden under §2.7.7 #4 / #7 (`is_heap()` probe on
//   raw bits). Kind comes from the producing call signature now.
// - The `dispatch_method_via_trampoline` extern-C `todo!()` stub —
//   replaced by the principled `VirtualMachine::jit_trampoline_call_method`
//   delegation per audit §2.1's load-bearing delegation insight.
//
// **Preserved fast path (JIT-internal kind, not a kind-decode):**
//
// The higher-order JIT array methods (find/filter/map/etc.) special-case
// stays IF the receiver kind on the parallel track tells us the slot
// carries opaque JIT-format bits (kind = `UInt64`, the documented §2.7.5
// I64-wide raw bits carrier). For JIT-format `HK_ARRAY` NaN-boxed
// receivers paired with closure callbacks, the `jit_control_*` FFI bodies
// dispatch callback execution via the JIT function table — VM delegation
// would lose this perf path. The receiver's JIT-format-array
// classification still uses `is_heap_kind(receiver_bits, HK_ARRAY)` for
// the inner discrimination, but only under the `UInt64` carrier-kind
// guard — i.e. only when the producing site explicitly stamped the slot
// as opaque-bits-no-classification. Not a §2.7.7 #4 / #7 violation: the
// outer dispatch comes from the parallel-kind track; the inner read is
// a JIT-format struct-field load on a known-opaque-bits slot. Migrating
// to fully kinded arrays is W10 jit-playbook §5 territory.

pub extern "C" fn jit_call_method(ctx: *mut JITContext, stack_count: usize) -> u64 {
    use crate::ffi::stack_kind_code;
    use shape_value::{HeapKind, NativeKind};

    unsafe {
        if ctx.is_null() || stack_count < 3 {
            return TAG_NULL;
        }

        let ctx_ref = &mut *ctx;
        let debug = std::env::var_os("SHAPE_JIT_DEBUG").is_some();

        // ── Pop arg_count ──────────────────────────────────────────────
        // ABI: the MIR producer stores `arg_count` as a raw i64 with
        // parallel-kind `UInt64` (sentinel slot — `terminators.rs:259`).
        // We decode it directly as usize — no NaN-box.
        if ctx_ref.stack_ptr == 0 {
            return TAG_NULL;
        }
        ctx_ref.stack_ptr -= 1;
        let arg_count = ctx_ref.stack[ctx_ref.stack_ptr] as usize;
        ctx_ref.stack_kinds[ctx_ref.stack_ptr] = stack_kind_code::SENTINEL;

        // ── Pop method_name ────────────────────────────────────────────
        // The MIR producer pushes the method name as a raw
        // `Box::into_raw(Box::new(UnifiedValue<Arc<String>>))` pointer
        // (via `box_string` at `terminators.rs:235`) with the parallel-
        // kind track stamped `NativeKind::String` per §2.7.7 / Q9 at
        // `terminators.rs:243-246`. The JIT-internal `unbox_string`
        // reads `&Arc<String>` from the unified-heap allocation. This is
        // a field read on a known-classified slot (kind track says
        // String), NOT a §2.7.7 #4 / #7 tag-decode on raw bits — the
        // kind IS the discriminator. Pre-Round-10 the bits were validated
        // via `is_heap_kind(method_bits, HK_STRING)` (a NaN-box
        // discrimination); under §2.7.5 strict-typed unified-heap the
        // bits are raw `Box::into_raw` pointers without the NaN-box
        // wrapper, so the parallel-kind track is the producer-side
        // classification source.
        if ctx_ref.stack_ptr == 0 {
            return TAG_NULL;
        }
        ctx_ref.stack_ptr -= 1;
        let method_bits = ctx_ref.stack[ctx_ref.stack_ptr];
        let method_kind_code = ctx_ref.stack_kinds[ctx_ref.stack_ptr];
        ctx_ref.stack_kinds[ctx_ref.stack_ptr] = stack_kind_code::SENTINEL;
        let method_kind = match stack_kind_code::decode(method_kind_code) {
            Some(k) => k,
            None => {
                if debug {
                    eprintln!(
                        "[jit-call-method] SURFACE §2.7.7 / Q9: method-name \
                         kind-byte {} at stack[{}] is SENTINEL / reserved. \
                         The producing call site at terminators.rs:243 must \
                         stamp NativeKind::String — no Bool-default.",
                        method_kind_code, ctx_ref.stack_ptr
                    );
                }
                return TAG_NULL;
            }
        };
        if !matches!(method_kind, NativeKind::String) {
            if debug {
                eprintln!(
                    "[jit-call-method] SURFACE: method-name kind {:?} != \
                     NativeKind::String. Producer-site contract violated \
                     (terminators.rs:243 must stamp String).",
                    method_kind
                );
            }
            return TAG_NULL;
        }
        let method_name: String = unbox_string(method_bits).to_string();
        if debug {
            eprintln!(
                "[jit-call-method] arg_count={} method='{}' stack_ptr={}",
                arg_count, method_name, ctx_ref.stack_ptr
            );
        }

        // ── Pop args paired with their parallel-track kinds ───────────
        // Reverse pop order, then reverse to source order. The §2.7.7 /
        // Q9 lockstep invariant: each `(bits, kind)` pair lives at the
        // same slot index.
        let mut arg_pairs: Vec<(u64, NativeKind)> = Vec::with_capacity(arg_count);
        for _ in 0..arg_count {
            if ctx_ref.stack_ptr == 0 {
                return TAG_NULL;
            }
            ctx_ref.stack_ptr -= 1;
            let bits = ctx_ref.stack[ctx_ref.stack_ptr];
            let code = ctx_ref.stack_kinds[ctx_ref.stack_ptr];
            ctx_ref.stack_kinds[ctx_ref.stack_ptr] = stack_kind_code::SENTINEL;
            let kind = match stack_kind_code::decode(code) {
                Some(k) => k,
                None => {
                    if debug {
                        eprintln!(
                            "[jit-call-method] SURFACE §2.7.7 / Q9: arg \
                             kind-byte {} at stack[{}] is SENTINEL / \
                             reserved. The producing call site at \
                             `mir_compiler/terminators.rs` must stamp \
                             a concrete NativeKind per ADR-006 §2.7.5 \
                             producer-side classification — no Bool-\
                             default fallback (§2.7.7 #9).",
                            code, ctx_ref.stack_ptr
                        );
                    }
                    return TAG_NULL;
                }
            };
            arg_pairs.push((bits, kind));
        }
        arg_pairs.reverse();

        // ── Pop receiver paired with its parallel-track kind ──────────
        if ctx_ref.stack_ptr == 0 {
            return TAG_NULL;
        }
        ctx_ref.stack_ptr -= 1;
        let receiver_bits = ctx_ref.stack[ctx_ref.stack_ptr];
        let receiver_code = ctx_ref.stack_kinds[ctx_ref.stack_ptr];
        ctx_ref.stack_kinds[ctx_ref.stack_ptr] = stack_kind_code::SENTINEL;
        let receiver_kind = match stack_kind_code::decode(receiver_code) {
            Some(k) => k,
            None => {
                if debug {
                    eprintln!(
                        "[jit-call-method] SURFACE §2.7.7 / Q9: receiver \
                         kind-byte {} at stack[{}] is SENTINEL / \
                         reserved. The producing call site must stamp \
                         the receiver's NativeKind per ADR-006 §2.7.5. \
                         No Bool-default fallback (§2.7.7 #9).",
                        receiver_code, ctx_ref.stack_ptr
                    );
                }
                return TAG_NULL;
            }
        };
        if debug {
            eprintln!(
                "[jit-call-method] method='{}' receiver_kind={:?} receiver_code={} \
                 receiver_bits={:#x}",
                method_name, receiver_kind, receiver_code, receiver_bits
            );
        }

        // ── Classification: delegate to VM or fall back to JIT-format ──
        //
        // The receiver kind from the §2.7.7 / Q9 parallel-kind track is
        // the §2.7.10 / Q11 dispatch discriminator. Kinds whose carriers
        // are kinded `Arc::into_raw(Arc<XData>)` (Round 7A Result/Option
        // + Round 9 typed-Arc collections HashSet/HashMap/Deque/
        // PriorityQueue/Channel/Mutex/Atomic/Lazy) route through the VM
        // trampoline's PHF dispatch tables in
        // `crates/shape-vm/src/executor/objects/method_registry.rs` —
        // ~73 already-kinded `MethodFnV2` entries per audit §2.1.
        //
        // Scalar kinds (Int64/Float64/Bool/String) also delegate to VM
        // for uniformity — the VM has full scalar method registries
        // (`NUMBER_METHODS` / `BOOL_METHODS` / `STRING_METHODS`).
        //
        // `UInt64` carrier kind: this is the §2.7.5 documented "I64-wide
        // raw bits without further classification" carrier. JIT-format
        // arrays / objects / etc. land here when MIR cannot prove a
        // precise kind. Fall back to legacy JIT-format dispatch — the
        // JIT-internal `is_heap_kind(receiver_bits, HK_*)` probe on
        // the heap-allocation kind field discriminates these.
        let delegated = match receiver_kind {
            NativeKind::Ptr(HeapKind::HashSet)
            | NativeKind::Ptr(HeapKind::HashMap)
            | NativeKind::Ptr(HeapKind::Deque)
            | NativeKind::Ptr(HeapKind::PriorityQueue)
            | NativeKind::Ptr(HeapKind::Channel)
            | NativeKind::Ptr(HeapKind::Mutex)
            | NativeKind::Ptr(HeapKind::Atomic)
            | NativeKind::Ptr(HeapKind::Lazy)
            | NativeKind::Ptr(HeapKind::Result)
            | NativeKind::Ptr(HeapKind::Option)
            | NativeKind::Float64
            | NativeKind::NullableFloat64
            | NativeKind::Int8
            | NativeKind::NullableInt8
            | NativeKind::UInt8
            | NativeKind::NullableUInt8
            | NativeKind::Int16
            | NativeKind::NullableInt16
            | NativeKind::UInt16
            | NativeKind::NullableUInt16
            | NativeKind::Int32
            | NativeKind::NullableInt32
            | NativeKind::UInt32
            | NativeKind::NullableUInt32
            | NativeKind::Int64
            | NativeKind::NullableInt64
            | NativeKind::NullableUInt64
            | NativeKind::IntSize
            | NativeKind::NullableIntSize
            | NativeKind::UIntSize
            | NativeKind::NullableUIntSize
            | NativeKind::Bool => true,
            // String: deliberately NOT delegated — JIT-format string
            // method registries (`call_string_method`) operate on
            // NaN-boxed JIT String carriers (`box_string` returns
            // `Arc<String>` raw pointer with the JIT NaN-box tag wrapper
            // for kind classification at the heap-header `kind` field).
            // VM-side `STRING_METHODS` would expect the kinded Arc
            // shape. Routing through JIT-format path preserves the
            // existing string method tests. This is a §2.7.5 carrier-
            // shape mismatch territory — full kinded String migration
            // is W10 jit-playbook §5.
            NativeKind::String => false,
            // UInt64: §2.7.5 carrier kind for opaque JIT bits. Fall
            // through to legacy JIT-format dispatch.
            NativeKind::UInt64 => false,
            // Other Ptr(*) kinds — TypedArray, TypedObject, String
            // (heap), Closure, TraitObject, etc. — fall through to
            // legacy JIT-format dispatch. The kinded path for these
            // is W10 jit-playbook §5 / §2.7.4 territory.
            NativeKind::Ptr(_) => false,
        };

        if delegated {
            if debug {
                eprintln!(
                    "[jit-call-method] delegating '{}' to VM, recv kind={:?} \
                     recv_bits={:#x} arg_count={}",
                    method_name, receiver_kind, receiver_bits, arg_count
                );
            }
            // VM-trampoline delegation per §2.7.5 cross-crate stable FFI.
            // The pair-slice form is single-direction at the boundary;
            // the VM converts to `&[KindedSlot]` internally before
            // `dispatch_method_kinded`. The JIT pre-incremented each
            // share via `retain_func_for_place` on the producing read;
            // the VM's transient KindedSlot carriers adopt those shares
            // and release on scope exit per §2.7.7 retain-on-read +
            // drop-on-write discipline (see
            // `VirtualMachine::jit_trampoline_call_method`'s ownership
            // contract docstring).
            let receiver_pair = (receiver_bits, receiver_kind);
            let result = super::control::with_trampoline_vm_mut(|vm| {
                vm.jit_trampoline_call_method(
                    &method_name,
                    receiver_pair,
                    &arg_pairs,
                    None,
                )
            });
            match result {
                Some(Ok(bits)) => return bits,
                Some(Err(e)) => {
                    if debug {
                        eprintln!(
                            "[jit-call-method] VM trampoline returned \
                             error for '{}' on receiver kind {:?}: {:?}",
                            method_name, receiver_kind, e
                        );
                    }
                    return TAG_NULL;
                }
                None => {
                    if debug {
                        eprintln!(
                            "[jit-call-method] VM trampoline unavailable \
                             — TRAMPOLINE_VM is null. '{}' on receiver \
                             kind {:?} surfaces.",
                            method_name, receiver_kind
                        );
                    }
                    return TAG_NULL;
                }
            }
        }

        // ── Legacy JIT-format dispatch (UInt64 carrier kind path) ─────
        //
        // The receiver kind on the §2.7.7 / Q9 parallel-kind track is
        // `UInt64` (or another non-delegated kind) — the slot carries
        // opaque JIT-format bits. The JIT-internal heap allocator
        // (`jit_box(HK_*, ...)` / `unified_box`) embeds the `kind: u16`
        // discriminator at offset 0 of the heap allocation per ADR-006
        // §2.7.5; the inner `heap_kind(receiver_bits)` probe is a
        // field-load on that known-opaque-bits allocation, NOT a
        // §2.7.7 #4 / #7 forbidden tag-decode on raw bits for kind
        // determination.
        let args: Vec<u64> = arg_pairs.iter().map(|(b, _)| *b).collect();

        // Higher-order array methods (find/filter/map/reduce/...) need
        // closure callback execution via `jit_control_*` FFI bodies —
        // preserved for JIT-format `HK_ARRAY` receivers.
        if is_heap_kind(receiver_bits, HK_ARRAY) {
            match method_name.as_str() {
                "find" | "findIndex" | "some" | "every" | "filter" | "map"
                | "count" | "group" | "groupBy" | "reduce" => {
                    if args.is_empty() {
                        return TAG_NULL;
                    }
                    let predicate = args[0];
                    let working_array_bits = receiver_bits;

                    if method_name == "reduce" {
                        let (callback, initial) = if args.len() > 1 {
                            (args[1], args[0])
                        } else {
                            (args[0], box_number(0.0))
                        };
                        ctx_ref.stack[ctx_ref.stack_ptr] = working_array_bits;
                        ctx_ref.stack_ptr += 1;
                        ctx_ref.stack[ctx_ref.stack_ptr] = callback;
                        ctx_ref.stack_ptr += 1;
                        ctx_ref.stack[ctx_ref.stack_ptr] = initial;
                        ctx_ref.stack_ptr += 1;
                        ctx_ref.stack[ctx_ref.stack_ptr] = box_number(3.0);
                        ctx_ref.stack_ptr += 1;
                        return super::control::jit_control_reduce(ctx);
                    }

                    ctx_ref.stack[ctx_ref.stack_ptr] = working_array_bits;
                    ctx_ref.stack_ptr += 1;
                    ctx_ref.stack[ctx_ref.stack_ptr] = predicate;
                    ctx_ref.stack_ptr += 1;
                    ctx_ref.stack[ctx_ref.stack_ptr] = box_number(2.0);
                    ctx_ref.stack_ptr += 1;

                    let result = match method_name.as_str() {
                        "find" => super::control::jit_control_find(ctx),
                        "findIndex" => super::control::jit_control_find_index(ctx),
                        "some" => super::control::jit_control_some(ctx),
                        "every" => super::control::jit_control_every(ctx),
                        "filter" => super::control::jit_control_filter(ctx),
                        "map" => super::control::jit_control_map(ctx),
                        "count" => {
                            // SURFACE (W10 jit-playbook §5 / ADR-006
                            // §2.7.4): count = filter(pred).length —
                            // the .length read decoded the deleted
                            // JitArray layout.
                            let _ = super::control::jit_control_filter(ctx);
                            todo!(
                                "phase-2c §2.7.4 / W10 jit-playbook §5: \
                                 JitArray rebuild — .count() on array."
                            )
                        }
                        "group" | "groupBy" => {
                            let _ = (predicate, working_array_bits);
                            todo!(
                                "phase-2c §2.7.4 / W10 jit-playbook §5: \
                                 JitArray rebuild — .group()/.groupBy() \
                                 on array."
                            )
                        }
                        _ => TAG_NULL,
                    };

                    return result;
                }
                _ => {}
            }
        }

        // Built-in JIT-format method dispatch. Each `call_*_method` body
        // reads from the receiver's known-opaque heap-allocation kind
        // field (NOT raw-bits tag-decode for kind determination).
        let builtin_result = if is_ok_tag(receiver_bits) || is_err_tag(receiver_bits) {
            call_result_method(receiver_bits, &method_name, &args)
        } else if is_number(receiver_bits) {
            call_number_method(receiver_bits, &method_name, &args)
        } else if is_inline_function(receiver_bits) {
            TAG_NULL
        } else {
            match heap_kind(receiver_bits) {
                Some(HK_ARRAY) => call_array_method(receiver_bits, &method_name, &args),
                Some(HK_STRING) => call_string_method(receiver_bits, &method_name, &args),
                Some(HK_JIT_OBJECT) => call_object_method(receiver_bits, &method_name, &args),
                Some(HK_DURATION) => call_duration_method(receiver_bits, &method_name, &args),
                Some(HK_COLUMN_REF) => TAG_NULL,
                Some(HK_MATRIX) => call_matrix_method(receiver_bits, &method_name, &args),
                Some(HK_TIME) => call_time_method(receiver_bits, &method_name, &args),
                _ => TAG_NULL,
            }
        };

        // User-defined method dispatch (UFCS — `"TypeName::method"`
        // functions in the JIT function table).
        if builtin_result == TAG_NULL {
            if let Some(user_result) = try_call_user_method(ctx, receiver_bits, &method_name, &args)
            {
                return user_result;
            }
        }

        // The pre-§2.7.10 `dispatch_method_via_trampoline` extern-C
        // `todo!()` (and the `_ => TAG_NULL` cascade fall-through to it)
        // is deleted. Method dispatch on VM-allocated objects now routes
        // through the §2.7.10 / Q11 kinded `vm.jit_trampoline_call_method`
        // path above when the receiver kind is one of the delegated-to-VM
        // kinds; the legacy JIT-format dispatch handles JIT-internal
        // opaque receivers (UInt64 carrier kind) per the producer-side
        // classification.

        builtin_result
    }
}
