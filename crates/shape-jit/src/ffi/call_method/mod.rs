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
use shape_value::{HeapKind, NativeKind};

// Module declarations
pub mod array;
pub mod duration;
pub mod matrix;
pub mod number;
pub mod object;
pub mod string;
pub mod time;

// Re-export the individual method handlers
pub use array::call_array_method;
pub use duration::call_duration_method;
pub use matrix::call_matrix_method;
pub use number::call_number_method;
pub use object::call_object_method;
pub use string::call_string_method;
pub use time::call_time_method;

/// Kinded `NativeKind::Ptr(HeapKind::*)` receivers are not JIT-format
/// NaN-boxed heap objects. Current JIT method dispatch keeps every such
/// label on the legacy fallback/surface path; this exhaustive classifier
/// prevents new HeapKind labels from inheriting that policy silently.
#[inline]
fn classify_kinded_ptr_receiver_for_jit_format_surface(hk: HeapKind) {
    match hk {
        HeapKind::String
        | HeapKind::TypedObject
        | HeapKind::Closure
        | HeapKind::Decimal
        | HeapKind::BigInt
        | HeapKind::DataTable
        | HeapKind::Future
        | HeapKind::TaskGroup
        | HeapKind::TypedArray
        | HeapKind::Temporal
        | HeapKind::TableView
        | HeapKind::Content
        | HeapKind::Instant
        | HeapKind::IoHandle
        | HeapKind::NativeScalar
        | HeapKind::NativeView
        | HeapKind::Char
        | HeapKind::HashMap
        | HeapKind::FilterExpr
        | HeapKind::Reference
        | HeapKind::SharedCell
        | HeapKind::HashSet
        | HeapKind::Iterator
        | HeapKind::Deque
        | HeapKind::Channel
        | HeapKind::PriorityQueue
        | HeapKind::Range
        | HeapKind::Result
        | HeapKind::Option
        | HeapKind::TraitObject
        | HeapKind::Mutex
        | HeapKind::Atomic
        | HeapKind::Lazy
        | HeapKind::ModuleFn
        | HeapKind::Matrix
        | HeapKind::MatrixSlice => {}
    }
}

// ============================================================================
// User-Defined Method Support
// ============================================================================

/// Determine the type name of a JIT receiver value via kind-from-parallel-
/// track dispatch (ADR-006 §2.7.5 / §2.7.7 / Q9, §2.7.9 / Q11, §2.7.10).
///
/// W17-narrow (Phase 3 cluster-0 Round 15, 2026-05-13): replaces the prior
/// 5-arm NaN-box tag-bit cascade (`is_number` / `TAG_BOOL_*` / `TAG_NULL` /
/// `heap_kind` match) with classification driven by the receiver's
/// `NativeKind` companion popped from the §2.7.7 / Q9 parallel-kind track
/// at `jit_call_method`'s dispatch entry (line 332-350). The prior tag-bit
/// predicates all return wrong answers on raw `Box::into_raw` carriers
/// because the §2.7.5 stamp-at-compile-time discipline removed the NaN-
/// box tag wrap (empirically verified by the W17-narrow audit §6:
/// `box_typed_object` returns `0x56c5…` with high bits clear, so
/// `is_number()` returned true on every TypedObject receiver and dispatch
/// fell through to `"number"`).
///
/// For `Ptr(HeapKind::TypedObject)` the schema id is recovered via a
/// direct `(*ptr).schema_id` field read after unboxing the JIT-internal
/// UnifiedValue prefix — same kind-from-parallel-track path the field-
/// access fast path uses (`field_access.rs::jit_typed_object_get_field`).
/// For UInt64-carrier opaque-bits receivers the inner kind discriminator
/// is read directly from the heap-allocation prefix at offset 0 via
/// `read_heap_kind` (a field-load on the JitAlloc / UnifiedValue prefix,
/// NOT a tag-bit predicate on raw bits — §2.7.5 explicitly carves this
/// out as "*not* tag-bit dispatch — it reads a field from a heap-resident
/// struct that the producing call placed there").
unsafe fn receiver_type_name(
    receiver_bits: u64,
    receiver_kind: NativeKind,
    exec_ctx: &ExecutionContext,
) -> Option<String> {
    use crate::ffi::typed_object::jit_typed_object_schema_id;

    match receiver_kind {
        // Scalar kinds — fixed type names.
        NativeKind::Float64
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
        | NativeKind::NullableUIntSize => Some("number".to_string()),
        NativeKind::Bool => Some("bool".to_string()),
        // R5b-2-bool-null-sentinel-cluster (ADR-006 §2.7 + §2.7.7/Q9,
        // 2026-05-19): null receivers have no method dispatch; surface
        // type-name as `"null"`.
        NativeKind::Null => Some("null".to_string()),
        NativeKind::String => Some("string".to_string()),
        // Round 19 S1.5 W12-nativekind-scalar-additions (2026-05-14):
        // ADR-006 §2.7.5 amendment adds F32 + Char as scalar variants.
        // F32 receivers report as `"number"` (same fix-point as F64);
        // Char receivers report as `"char"` (matches the existing
        // `NativeKind::Ptr(HeapKind::Char)` arm below).
        NativeKind::Float32 => Some("number".to_string()),
        NativeKind::Char => Some("char".to_string()),
        // Wave 2 Agent B W12-StringV2-DecimalV2-NativeKind-additions
        // (2026-05-14): v2-raw heap-pointer carriers report the same
        // type-name surface as their Arc-wrapped siblings.
        NativeKind::StringV2 => Some("string".to_string()),
        NativeKind::DecimalV2 => Some("decimal".to_string()),

        // Typed heap pointer kinds — straight kind→name map per the
        // surviving HeapKind discriminants.
        NativeKind::Ptr(HeapKind::String) => Some("string".to_string()),
        NativeKind::Ptr(HeapKind::TypedObject) => {
            // Resolve the schema name via the JIT-internal TypedObject's
            // `(*ptr).schema_id` field — `jit_typed_object_schema_id` is
            // post-W17-narrow correct on raw `Box::into_raw` carriers
            // (its prior `is_typed_object` gate was dropped in the same
            // round). Schema lookup follows the same two-tier shape as
            // `object/property_access.rs::HK_TYPED_OBJECT` (the W12-jit-
            // binop-after-heap-read-kind-tracker close): try the global
            // stdlib registry first, then fall back to the trampoline VM's
            // bytecode program registry (where user-defined types like X
            // live). Both halves are required because `ExecutionContext`'s
            // direct registry only covers global stdlib schemas, not the
            // per-program user-defined ones.
            let schema_id = jit_typed_object_schema_id(receiver_bits);
            if schema_id == 0 {
                return None;
            }
            let global = shape_runtime::type_schema::lookup_schema_by_id_public(schema_id)
                .map(|s| s.name.clone());
            if global.is_some() {
                return global;
            }
            let _ = exec_ctx;
            super::control::with_trampoline_vm(|vm| {
                vm.program()
                    .type_schema_registry
                    .get_by_id(schema_id)
                    .map(|s| s.name.clone())
            })
            .flatten()
        }
        NativeKind::Ptr(HeapKind::TypedArray) => Some("Array".to_string()),
        NativeKind::Ptr(HeapKind::Decimal) => Some("decimal".to_string()),
        NativeKind::Ptr(HeapKind::BigInt) => Some("bigint".to_string()),
        NativeKind::Ptr(HeapKind::DataTable) => Some("Table".to_string()),
        NativeKind::Ptr(HeapKind::HashMap) => Some("HashMap".to_string()),
        NativeKind::Ptr(HeapKind::HashSet) => Some("Set".to_string()),
        NativeKind::Ptr(HeapKind::Future) => Some("Future".to_string()),
        NativeKind::Ptr(HeapKind::TaskGroup) => Some("TaskGroup".to_string()),
        NativeKind::Ptr(HeapKind::Closure) => Some("Closure".to_string()),
        NativeKind::Ptr(HeapKind::Temporal) => Some("Temporal".to_string()),
        NativeKind::Ptr(HeapKind::TableView) => Some("TableView".to_string()),
        NativeKind::Ptr(HeapKind::Content) => Some("Content".to_string()),
        NativeKind::Ptr(HeapKind::Instant) => Some("Instant".to_string()),
        NativeKind::Ptr(HeapKind::IoHandle) => Some("IoHandle".to_string()),
        NativeKind::Ptr(HeapKind::Char) => Some("char".to_string()),
        NativeKind::Ptr(HeapKind::Iterator) => Some("Iterator".to_string()),
        NativeKind::Ptr(HeapKind::Deque) => Some("Deque".to_string()),
        NativeKind::Ptr(HeapKind::Channel) => Some("Channel".to_string()),
        NativeKind::Ptr(HeapKind::PriorityQueue) => Some("PriorityQueue".to_string()),
        NativeKind::Ptr(HeapKind::Range) => Some("Range".to_string()),
        NativeKind::Ptr(HeapKind::Result) => Some("Result".to_string()),
        NativeKind::Ptr(HeapKind::Option) => Some("Option".to_string()),
        NativeKind::Ptr(HeapKind::TraitObject) => Some("TraitObject".to_string()),
        NativeKind::Ptr(HeapKind::Mutex) => Some("Mutex".to_string()),
        NativeKind::Ptr(HeapKind::Atomic) => Some("Atomic".to_string()),
        NativeKind::Ptr(HeapKind::Lazy) => Some("Lazy".to_string()),
        NativeKind::Ptr(HeapKind::ModuleFn) => Some("ModuleFn".to_string()),
        // ADR-006 §2.7.22 amendment (Round 18 S3, 2026-05-13).
        NativeKind::Ptr(HeapKind::Matrix) => Some("Matrix".to_string()),
        NativeKind::Ptr(HeapKind::MatrixSlice) => Some("Vec<number>".to_string()),
        // Pure-discriminator kinds with no method receiver shape — see
        // ADR-006 §2.7.9 (FilterExpr), §2.7.12 (SharedCell), §2.7.13
        // (Reference), §2.7.14 (NativeScalar / NativeView).
        NativeKind::Ptr(HeapKind::FilterExpr)
        | NativeKind::Ptr(HeapKind::Reference)
        | NativeKind::Ptr(HeapKind::SharedCell)
        | NativeKind::Ptr(HeapKind::NativeScalar)
        | NativeKind::Ptr(HeapKind::NativeView) => None,

        // UInt64 carrier — opaque JIT-format bits whose inner kind lives in
        // the JitAlloc / UnifiedValue prefix at offset 0. Read the prefix
        // via `read_heap_kind` (§2.7.5 "not tag-bit dispatch — field-load
        // from a heap-resident struct"). The null-pointer check guards
        // against UInt64-carrier callers that legitimately stamp a
        // sentinel value (e.g. arg_count) — those don't reach this
        // function in practice but the defensive null guard is cheap.
        NativeKind::UInt64 => {
            if receiver_bits == 0 || receiver_bits == TAG_NULL || receiver_bits == TAG_NONE {
                return None;
            }
            // SAFETY: the `NativeKind::UInt64` arm is the documented
            // JIT-format opaque-bits carrier. Callers of this unsafe helper
            // must only reach this branch with `receiver_bits` pointing at a
            // live JitAlloc / UnifiedValue allocation whose first field is
            // the heap-kind prefix.
            match unsafe { read_heap_kind(receiver_bits) } {
                HK_STRING => Some("string".to_string()),
                HK_ARRAY => Some("Array".to_string()),
                HK_TYPED_OBJECT => {
                    let schema_id = jit_typed_object_schema_id(receiver_bits);
                    if schema_id == 0 {
                        return None;
                    }
                    let global = shape_runtime::type_schema::lookup_schema_by_id_public(schema_id)
                        .map(|s| s.name.clone());
                    if global.is_some() {
                        return global;
                    }
                    let _ = exec_ctx;
                    super::control::with_trampoline_vm(|vm| {
                        vm.program()
                            .type_schema_registry
                            .get_by_id(schema_id)
                            .map(|s| s.name.clone())
                    })
                    .flatten()
                }
                HK_JIT_OBJECT => Some("object".to_string()),
                HK_DURATION => Some("Duration".to_string()),
                HK_TIME => Some("DateTime".to_string()),
                _ => None,
            }
        }
    }
}

/// Search the JITContext's function_names table for a function with the given
/// method function name (e.g. "Point::distance" or "Point.distance") and return
/// its index.
unsafe fn find_function_by_name(ctx_ref: &JITContext, function_name: &str) -> Option<usize> {
    if ctx_ref.function_names_ptr.is_null() || ctx_ref.function_names_len == 0 {
        return None;
    }
    // SAFETY: the caller provides a live JITContext; the null/len guard above
    // ensures the raw names table is present before constructing the slice.
    let names = unsafe {
        std::slice::from_raw_parts(ctx_ref.function_names_ptr, ctx_ref.function_names_len)
    };
    for (idx, name) in names.iter().enumerate() {
        if name == function_name {
            return Some(idx);
        }
    }
    None
}

/// Try to call a user-defined method from impl / extend blocks.
///
/// User-defined methods are compiled as functions named either
/// `"TypeName::method_name"` (impl-style) or `"TypeName.method_name"`
/// (extend-style). This function:
/// 1. Determines the receiver type name from the receiver's `NativeKind`
///    (kind-from-parallel-track per §2.7.7 / Q9) and, for typed-object /
///    UInt64 carriers, the schema id / heap-prefix `kind: u16` field at
///    offset 0 of the JIT allocation (§2.7.5 "*not* tag-bit dispatch —
///    field-load from a heap-resident struct").
/// 2. Constructs impl-style and extend-style method function names
/// 3. Looks up the function index in function_names
/// 4. Calls the function via function_table, passing (receiver, ...args)
/// 5. Returns the result as raw u64 bits
///
/// Returns Some(result) if the method was found and executed, None otherwise.
///
/// W17-narrow (Phase 3 cluster-0 Round 15, 2026-05-13): `receiver_kind`
/// is threaded through from `jit_call_method`'s dispatch entry's
/// parallel-kind pop (line 332-350) so `receiver_type_name` can classify
/// without re-decoding tag bits (the W-series defection-attractor pattern).
unsafe fn try_call_user_method(
    ctx: *const JITContext,
    receiver_bits: u64,
    receiver_kind: NativeKind,
    method_name: &str,
    arg_pairs: &[(u64, NativeKind)],
) -> Option<u64> {
    use crate::ffi::stack_kind_code;

    // SAFETY: callers only invoke this from the live `jit_call_method`
    // dispatch frame after checking the JITContext pointer for null.
    let ctx_ref = unsafe { &*ctx };

    // Need execution context to access the type schema registry
    if ctx_ref.exec_context_ptr.is_null() {
        return None;
    }
    // SAFETY: the null guard above proves `exec_context_ptr` is present for
    // the duration of this dispatch.
    let exec_ctx = unsafe { &*(ctx_ref.exec_context_ptr as *const ExecutionContext) };

    // Determine the receiver's type name
    // SAFETY: `receiver_bits` and `receiver_kind` are the pair popped from the
    // JIT stack's lockstep kind track by `jit_call_method`.
    let type_name = match unsafe { receiver_type_name(receiver_bits, receiver_kind, exec_ctx) } {
        Some(type_name) => type_name,
        None => {
            if std::env::var_os("SHAPE_DEBUG_FIELD_STAMPS").is_some()
                && method_name == "summary"
            {
                eprintln!(
                    "[method-debug] no receiver type for method={} bits={} kind={:?}",
                    method_name, receiver_bits, receiver_kind
                );
            }
            return None;
        }
    };

    // Construct both method function name forms. `impl` methods use
    // `Type::method`; `extend Type { method ... }` desugars to `Type.method`.
    // Try the historical impl-style form first to preserve existing dispatch
    // precedence, then the extend-style form used by generated extension
    // methods such as `User.to_json`.
    let impl_name = format!("{}::{}", type_name, method_name);
    let extend_name = format!("{}.{}", type_name, method_name);
    let candidates = [impl_name, extend_name];

    // Look up the function index in the JIT function table.
    // SAFETY: `ctx_ref` is the live context borrowed above; the helper
    // validates the raw names table before reading it.
    let (func_idx, resolved_name) = match candidates.iter().find_map(|candidate| {
        unsafe { find_function_by_name(ctx_ref, candidate) }.map(|idx| (idx, candidate.as_str()))
    }) {
        Some(found) => found,
        None => {
            if std::env::var_os("SHAPE_DEBUG_FIELD_STAMPS").is_some()
                && method_name == "summary"
            {
                eprintln!(
                    "[method-debug] method={} type={} candidates={:?} names_len={}",
                    method_name, type_name, candidates, ctx_ref.function_names_len
                );
                if !ctx_ref.function_names_ptr.is_null() {
                    let names = unsafe {
                        std::slice::from_raw_parts(
                            ctx_ref.function_names_ptr,
                            ctx_ref.function_names_len,
                        )
                    };
                    eprintln!("[method-debug] function names={:?}", names);
                }
            }
            return None;
        }
    };

    // Check that we have a valid function table entry
    if ctx_ref.function_table.is_null() || func_idx >= ctx_ref.function_table_len {
        tracing::debug!(
            target: "shape_jit",
            method_name = %method_name,
            resolved_name = %resolved_name,
            func_idx,
            function_table_len = ctx_ref.function_table_len,
            "jit-call-method resolved a user method but no native function-table \
             entry is addressable; raising pending_call_error instead of \
             returning TAG_NULL as a value",
        );
        super::control::set_jit_runtime_error(format!(
            "JIT method dispatch for `{}` resolved `{}` but no native \
             function-table entry was available",
            method_name, resolved_name,
        ));
        let ctx_mut = unsafe { &mut *(ctx as *mut JITContext) };
        ctx_mut.pending_call_error = 1;
        return Some(TAG_NULL);
    }

    // Read the raw pointer from the function table. A null entry means the
    // function was not JIT-compiled (interpreted only).
    // SAFETY: `function_table` is non-null and `func_idx` is proven in-bounds
    // by the guard above.
    let raw_fn_ptr = unsafe { *(ctx_ref.function_table as *const *const u8).add(func_idx) };
    if raw_fn_ptr.is_null() {
        tracing::debug!(
            target: "shape_jit",
            method_name = %method_name,
            resolved_name = %resolved_name,
            func_idx,
            "jit-call-method resolved a user method whose native function-table \
             entry is null; raising pending_call_error instead of returning \
             TAG_NULL as a value",
        );
        super::control::set_jit_runtime_error(format!(
            "JIT method dispatch for `{}` resolved `{}` but that method was \
             not JIT-compiled",
            method_name, resolved_name,
        ));
        let ctx_mut = unsafe { &mut *(ctx as *mut JITContext) };
        ctx_mut.pending_call_error = 1;
        return Some(TAG_NULL);
    }
    if std::env::var_os("SHAPE_DEBUG_FIELD_STAMPS").is_some() && method_name == "summary" {
        eprintln!(
            "[method-debug] resolved method={} resolved={} func_idx={} table_len={} raw_null={}",
            method_name,
            resolved_name,
            func_idx,
            ctx_ref.function_table_len,
            raw_fn_ptr.is_null()
        );
    }

    // W14.2-E-followup-jit-trait-method-arity-soundness fix (2026-05-19,
    // v0.3-gating SOUNDNESS BUG): the JIT-compiled UFCS callee was emitted
    // with the extended Cranelift signature
    // `fn(ctx_ptr, capture_0..N, param_0..M) -> i32`
    // (`compile_function_with_user_funcs` at `compiler/program.rs:258-265`,
    // appended params per `effective_arity = captures_count + arity`). Its
    // entry-block parameter init at `compiler/program.rs:496-528` reads
    // each MIR param slot from `entry_params[native_idx]` — the SYSTEM V
    // register/stack ABI, NOT `ctx.stack`. The prior `fn_ptr(ctx_mut)`
    // call transmuted the function pointer as `JittedStrategyFn` (a single-
    // arg shape) and silently dropped every receiver/arg slot. The callee
    // then read uninitialized SystemV-passing registers/stack frame for
    // `self` and each user param — the empirical garbage NaN-bits for
    // n>=1 (e.g. `d.dbl(21)` = `189861470636784`) and SEGFAULT for string
    // args (registers held callee-saved garbage that decoded to wild
    // `*const Arc<String>` pointers).
    //
    // Per ADR-006 §2.7.5 producer-side classification: the receiver +
    // each `arg_pairs[i]` already carry the kind stamped at the
    // `mir_compiler/terminators.rs` push (line ~342-372 for args, line
    // ~510-535 for receiver). The kind half is sourced from §2.7.7/Q9
    // parallel-track decode at the dispatch shell entry (lines ~482-501,
    // ~513-527). The data half flows through this helper's typed-fn
    // transmute selector via the kinded raw-bits slice — identical shape
    // to `jit_call_value`'s bare-function fast path at
    // `ffi/control/mod.rs:534-545` and `:709-732`.
    //
    // §2.7.7/Q9 lockstep invariant: the callee's own MIR-compiled body
    // re-establishes its parallel-kind track from its own FrameDescriptor
    // when it begins execution (same shape as the bare-function path's
    // contract). The dispatch shell's parallel-kind track at indices
    // popped (receiver / arg_pairs / method_name / arg_count) was already
    // reset to SENTINEL at the pop sites above; we don't write the JIT-
    // stack push half of the lockstep here because the callee doesn't
    // read from `ctx.stack` — passing through the native ABI bypasses
    // the stack entirely.

    // Reset the JIT stack frame for the callee. The callee's first action
    // is to write its return value to `ctx.stack[0]` and bump `stack_ptr`
    // to 1 (see `mir_compiler/terminators.rs::TerminatorKind::Return` at
    // line 1714-1718). Matches the §2.7.11/Q12 bare-function dispatch
    // contract at `ffi/control/mod.rs:716`.
    // SAFETY: this dispatch owns the active JIT frame; no other mutable access
    // to the context is live while resetting the callee stack frame.
    let ctx_mut = unsafe { &mut *(ctx as *mut JITContext) };
    let _ = stack_kind_code::SENTINEL; // silence unused-import warning in this fn
    ctx_mut.stack_ptr = 0;

    // Build the native-arg slice: receiver as the first user param
    // (`self`), followed by each user arg. Impl and extend method bodies
    // both compile with `self` as their first formal parameter (when
    // present); for n=0-arg methods the receiver is still the first param.
    // Matches the JIT-compiled callee's `effective_arity = captures_count +
    // arity` per `compile_function_with_user_funcs` (captures_count = 0 for
    // non-closure method bodies).
    let mut native_args: Vec<u64> = Vec::with_capacity(arg_pairs.len() + 1);
    native_args.push(receiver_bits);
    for &(bits, _kind) in arg_pairs {
        native_args.push(bits);
    }

    // Call the JIT-compiled function through the native ABI dispatch
    // helper. The signal value is ignored — error-path deopt is not yet
    // routed through this trait-method surface (the bare-function path
    // ignores it identically at `ffi/control/mod.rs:535,:717`).
    // SAFETY: `raw_fn_ptr` came from the validated JIT function table entry
    // above, and `native_args` matches the UFCS receiver-plus-args ABI.
    let _result_code =
        unsafe { crate::ffi::control::call_jit_fn_with_args(raw_fn_ptr, ctx_mut, &native_args) };
    if std::env::var_os("SHAPE_DEBUG_FIELD_STAMPS").is_some() && method_name == "summary" {
        let result0 = if ctx_mut.stack_ptr > 0 {
            Some(ctx_mut.stack[0])
        } else {
            None
        };
        eprintln!(
            "[method-debug] after call method={} stack_ptr={} return_tag={} result0={:?}",
            method_name, ctx_mut.stack_ptr, ctx_mut.return_type_tag, result0
        );
    }

    // Pop result from stack. The callee stored the return value at
    // `ctx.stack[0]` and set `stack_ptr = 1` per the §2.7.5 typed-return
    // contract; clear the kind track slot back to SENTINEL on pop to
    // preserve the §2.7.7 / Q9 invariant for the slot the caller will
    // reuse.
    if ctx_mut.stack_ptr > 0 {
        ctx_mut.stack_ptr -= 1;
        let result = ctx_mut.stack[ctx_mut.stack_ptr];
        ctx_mut.stack_kinds[ctx_mut.stack_ptr] = stack_kind_code::SENTINEL;
        Some(result)
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
                tracing::debug!(
                    target: "shape_jit",
                    method_kind_code,
                    stack_ptr = ctx_ref.stack_ptr,
                    "jit-call-method SURFACE \u{a7}2.7.7 / Q9: method-name \
                     kind-byte is SENTINEL / reserved. The producing call \
                     site at terminators.rs:243 must stamp NativeKind::String \
                     \u{2014} no Bool-default.",
                );
                return TAG_NULL;
            }
        };
        if !matches!(method_kind, NativeKind::String) {
            tracing::debug!(
                target: "shape_jit",
                method_kind = ?method_kind,
                "jit-call-method SURFACE: method-name kind != \
                 NativeKind::String. Producer-site contract violated \
                 (terminators.rs:243 must stamp String).",
            );
            return TAG_NULL;
        }
        let method_name: String = unbox_string(method_bits).to_string();
        tracing::debug!(
            target: "shape_jit",
            arg_count,
            method_name = %method_name,
            stack_ptr = ctx_ref.stack_ptr,
            "jit-call-method dispatch",
        );

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
                    tracing::debug!(
                        target: "shape_jit",
                        code,
                        stack_ptr = ctx_ref.stack_ptr,
                        "jit-call-method SURFACE \u{a7}2.7.7 / Q9: arg \
                         kind-byte is SENTINEL / reserved. The producing \
                         call site at `mir_compiler/terminators.rs` must \
                         stamp a concrete NativeKind per ADR-006 \u{a7}2.7.5 \
                         producer-side classification \u{2014} no Bool-default \
                         fallback (\u{a7}2.7.7 #9).",
                    );
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
                tracing::debug!(
                    target: "shape_jit",
                    receiver_code,
                    stack_ptr = ctx_ref.stack_ptr,
                    "jit-call-method SURFACE \u{a7}2.7.7 / Q9: receiver \
                     kind-byte is SENTINEL / reserved. The producing call \
                     site must stamp the receiver's NativeKind per ADR-006 \
                     \u{a7}2.7.5. No Bool-default fallback (\u{a7}2.7.7 #9).",
                );
                return TAG_NULL;
            }
        };
        tracing::debug!(
            target: "shape_jit",
            method_name = %method_name,
            receiver_kind = ?receiver_kind,
            receiver_code,
            receiver_bits,
            "jit-call-method receiver classified",
        );

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
            // r5c-2-gz-CP9 (v0.3 NO-KNOWN-INCORRECTNESS γ item-9): typed-
            // array receivers that reach this dispatch shell delegate to
            // the VM trampoline. The structurally cheap typed-array
            // methods (`length`/`len`/`push`/`first`/`last`/`sum`/`min`/
            // `max`/...) are intercepted inline by `try_emit_v2_array_
            // method` in `mir_compiler/terminators.rs` and never reach
            // here. The methods that DO fall through to `jit_call_method`
            // with a `Ptr(TypedArray)` receiver — `count` / `group` /
            // `groupBy` / `contains` — previously hit the legacy
            // JIT-format dispatch, where the `builtin_result` cascade
            // has no JIT-format registry for `Ptr(_)` carriers and the
            // `Ptr(_) => TAG_NULL` arm returned a silent placeholder.
            // The JIT-compiled caller then wrote `TAG_NULL` into a
            // heap-kinded destination: `groupBy().sum()` SIGSEGV'd
            // (ec=139), `count(pred)` printed garbage
            // (`-1407374883553280`), `contains(x)` silently returned
            // `false` — every one a VM/JIT divergence producing garbage
            // where the bytecode VM cleanly errors (`handle_count_v2` /
            // `handle_group_by_v2` ckpt2 SURFACE; "no method" for
            // `contains`). Delegating to the VM trampoline routes these
            // through `dispatch_method_kinded` — the VM's authoritative
            // PHF registry — so an unimplemented/missing method surfaces
            // a clean `Err`, which the trampoline's `Some(Err(_))` arm
            // turns into a `pending_call_error` deopt (W12 compile-
            // failure → interpreter fall-through). Net result: VM == JIT.
            // The full kinded typed-array JIT-format method registry is
            // W10 jit-playbook §5 / §2.7.4 territory; until it lands,
            // VM delegation is the correct (non-garbage) behaviour.
            | NativeKind::Ptr(HeapKind::TypedArray)
            // Wave 1b SEAM B (2026-06-15): `Ptr(HeapKind::Iterator)`
            // receivers are handled by the surface-and-stop deopt ABOVE
            // (they never reach this delegation match) — a mid-JIT-frame VM
            // trampoline delegation is unsound for the iterator carrier
            // (closure-arg carrier mismatch + share-crossing race). See the
            // `Ptr(Iterator)` surface block above the `delegated` match.
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
            | NativeKind::Bool
            // Round 19 S1.5 W12-nativekind-scalar-additions (2026-05-14):
            // F32 receivers delegate to VM (NUMBER_METHODS); Char
            // receivers delegate to VM (CHAR_METHODS — the existing
            // receiver kind for char methods).
            | NativeKind::Float32
            | NativeKind::Char => true,
            // Wave 2 Agent B W12-StringV2-DecimalV2-NativeKind-additions
            // (2026-05-14): v2-raw heap-pointer carriers delegate to VM —
            // same routing rationale as the §H.4 H-c amendment: producer
            // (Agent A2) emits v2-raw slots, consumer (this dispatch
            // shell) routes them to the VM-side method registry where the
            // method-handler bodies dispatch on the StringV2 / DecimalV2
            // kind label to read the carrier's payload. The JIT-format
            // path expects Arc-wrapped carriers; VM-side handlers are
            // carrier-aware.
            NativeKind::StringV2 | NativeKind::DecimalV2 => true,
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
            NativeKind::Ptr(hk) => {
                classify_kinded_ptr_receiver_for_jit_format_surface(hk);
                false
            }
            // R5b-2-bool-null-sentinel-cluster (ADR-006 §2.7 +
            // §2.7.7/Q9, 2026-05-19): null receivers delegate to VM
            // which surfaces a TypeError uniformly.
            NativeKind::Null => true,
        };

        // ── Surface-and-stop: JIT-format closure arg cannot cross to VM ──
        //
        // r5c-2-gz-CP9 (v0.3 NO-KNOWN-INCORRECTNESS γ item-9). A
        // higher-order method on a typed-array receiver (`groupBy` /
        // `count` / `find` / `filter` / ... with a `|x| ...` predicate)
        // carries the closure as an argument. The MIR producer stamps
        // that arg's `NativeKind` from `slot_kinds`; for some call sites
        // the inferred kind is `Ptr(HeapKind::Closure)` even though the
        // JIT lowered the closure to a JIT-format NaN-boxed inline-
        // function carrier (a tagged `0xfffd…` bit-pattern), NOT a
        // v2-raw `*mut ClosureRaw` heap pointer. Delegating such an arg
        // to the VM trampoline builds `KindedSlot::new(from_raw(bits),
        // Ptr(Closure))`; when the VM-side method handler surfaces an
        // `Err` and the transient `kinded_args` Vec drops, the
        // `Ptr(Closure)` arm of `drop_with_kind` dereferences the
        // NaN-boxed bits as a heap pointer → SIGSEGV (empirically: array
        // `groupBy(|x| ...)` crashed ec=139 inside the trampoline's
        // `kinded_args` drop). The JIT-format closure carrier and the
        // VM's v2-raw `Ptr(Closure)` carrier are structurally distinct;
        // they cannot meet at the FFI boundary without a forbidden
        // carrier-translation bridge (CLAUDE.md §Renames to refuse).
        //
        // The honest fix is the W12 compile-failure → interpreter
        // fall-through: raise a structured `pending_call_error` and
        // deopt the JIT frame. The bytecode interpreter then re-runs the
        // method call with its own (carrier-correct) closure handling
        // and produces the VM's behaviour — for array `groupBy` that is
        // a clean `Stack overflow` (the in-Shape `vec.shape` `groupBy`
        // self-recurses) / for unimplemented methods a clean SURFACE
        // `Err`. Net result: VM == JIT — both cleanly error, neither
        // SIGSEGVs. A real JIT-format typed-array higher-order method
        // path is W10 jit-playbook §5 / §2.7.4 territory.
        if matches!(receiver_kind, NativeKind::Ptr(HeapKind::TypedArray))
            && arg_pairs
                .iter()
                .any(|(_, k)| matches!(k, NativeKind::Ptr(HeapKind::Closure)))
        {
            tracing::debug!(
                target: "shape_jit",
                method_name = %method_name,
                "jit-call-method SURFACE: typed-array higher-order method \
                 with a JIT-format closure arg cannot delegate to the VM \
                 trampoline (carrier-shape mismatch) \u{2014} raising \
                 pending_call_error for MIR-emitted deopt to interpreter \
                 fall-through (W12 pattern)",
            );
            super::control::set_jit_runtime_error(format!(
                "JIT codegen for typed-array `.{}()` with a closure \
                 argument is unimplemented \u{2014} deopting to interpreter",
                method_name,
            ));
            ctx_ref.pending_call_error = 1;
            return TAG_NULL;
        }

        if delegated {
            tracing::debug!(
                target: "shape_jit",
                method_name = %method_name,
                receiver_kind = ?receiver_kind,
                receiver_bits,
                arg_count,
                "jit-call-method delegating to VM",
            );
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
                vm.jit_trampoline_call_method(&method_name, receiver_pair, &arg_pairs, None)
            });
            match result {
                Some(Ok(bits)) => return bits,
                Some(Err(e)) => {
                    // r5c-2-bz-b-jit-err-surface: the VM-side method handler
                    // surfaced a clean `Err` (e.g. `Set.add()` with a non-
                    // string key). The JIT-compiled caller must NOT continue
                    // with a value-shaped placeholder — a heap-kinded
                    // destination place would feed it into a refcount-retain
                    // (`jit_arc_result_retain`'s `bits != 0` guard passes
                    // `TAG_NULL`, then dereferences `TAG_NULL - 16`). Instead
                    // raise `pending_call_error` so the MIR-emitted post-call
                    // check deopts the JIT frame; the VM produces the clean
                    // error. The returned bits are never consumed.
                    tracing::debug!(
                        target: "shape_jit",
                        method_name = %method_name,
                        receiver_kind = ?receiver_kind,
                        error = ?e,
                        "jit-call-method VM trampoline returned error \u{2014} \
                         raising pending_call_error for MIR-emitted deopt",
                    );
                    super::control::set_jit_runtime_error(e.to_string());
                    ctx_ref.pending_call_error = 1;
                    return TAG_NULL;
                }
                None => {
                    tracing::debug!(
                        target: "shape_jit",
                        method_name = %method_name,
                        receiver_kind = ?receiver_kind,
                        "jit-call-method VM trampoline unavailable \u{2014} \
                         TRAMPOLINE_VM is null. Surfaces.",
                    );
                    super::control::set_jit_runtime_error(format!(
                        "JIT method dispatch for `{}` could not reach the \
                         interpreter trampoline",
                        method_name,
                    ));
                    ctx_ref.pending_call_error = 1;
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
            // ── Surface-and-stop: unimplemented JIT-format array methods ──
            //
            // r5c-2-gz-CP9 (v0.3 NO-KNOWN-INCORRECTNESS γ item-9): the
            // JIT-format `count` / `group` / `groupBy` legacy paths were
            // `todo!()` stubs. `jit_call_method` is an `extern "C"`
            // function — a `todo!()` panic unwinding across the FFI
            // boundary is undefined behaviour: empirically `groupBy` +
            // `.sum()`/`.len()` SIGSEGV'd (ec=139) and `count` printed
            // garbage (`-1407374883553280`, ec=0) where the bytecode VM
            // cleanly SURFACEs (`handle_group_by_v2` / the `count` PHF
            // SURFACE error). That is a VM/JIT divergence producing
            // garbage/crashes.
            //
            // The honest fix is the W12 compile-failure → interpreter
            // fall-through pattern (`docs/cluster-audits/v0.3-w12-jit-
            // mode-semantics-close.md`), NOT partial codegen: a real
            // JIT implementation would only re-create the divergence
            // while the VM still SURFACEs. We raise a structured
            // `pending_call_error` BEFORE touching the JIT stack (the
            // prior `count` arm pushed onto + part-consumed the stack via
            // `jit_control_filter` then `todo!()`'d, corrupting it). The
            // MIR-emitted post-call check deopts the JIT frame; the
            // bytecode interpreter then produces the VM's clean SURFACE
            // error. Net result: VM == JIT — both cleanly error, neither
            // produces garbage or SIGSEGVs.
            match method_name.as_str() {
                "count" | "group" | "groupBy" => {
                    tracing::debug!(
                        target: "shape_jit",
                        method_name = %method_name,
                        "jit-call-method SURFACE: array `count`/`group`/\
                         `groupBy` JIT-format codegen unimplemented \u{2014} \
                         raising pending_call_error for MIR-emitted deopt \
                         to interpreter fall-through (W12 pattern)",
                    );
                    super::control::set_jit_runtime_error(format!(
                        "JIT codegen for array `.{}()` is unimplemented \
                         \u{2014} deopting to interpreter",
                        method_name,
                    ));
                    ctx_ref.pending_call_error = 1;
                    return TAG_NULL;
                }
                _ => {}
            }
            match method_name.as_str() {
                "find" | "findIndex" | "some" | "every" | "filter" | "map" | "reduce" => {
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
                        // `count` / `group` / `groupBy` are surfaced-and-
                        // stopped above before the JIT stack is touched.
                        _ => TAG_NULL,
                    };

                    return result;
                }
                _ => {}
            }
        }

        // Built-in JIT-format method dispatch — kind-from-parallel-track
        // per ADR-006 §2.7.5 / §2.7.7 / Q9, §2.7.10 / Q11.
        //
        // W17-narrow (Phase 3 cluster-0 Round 15, 2026-05-13): replaced
        // the prior 6-arm tag-bit cascade (`is_ok_tag` / `is_err_tag` /
        // `is_number` / `is_inline_function` / `heap_kind` cascade for
        // HK_ARRAY / HK_STRING / HK_JIT_OBJECT / …) with classification
        // driven by the receiver's `NativeKind` companion (already
        // popped from the §2.7.7 / Q9 parallel-kind track at line
        // 332-350). The prior predicates all required `is_heap()` /
        // `is_tagged()` / `is_number()` checks on raw bits — those
        // return wrong answers on §2.7.5 raw `Box::into_raw` carriers
        // (audit §6 empirical evidence). For UInt64-carrier opaque-bits
        // receivers the inner discriminator is read directly from the
        // JitAlloc / UnifiedValue prefix at offset 0 via `read_heap_kind`
        // — a field-load on the heap-resident struct, NOT a tag-bit
        // predicate (§2.7.5 carves this out: "*not* tag-bit dispatch —
        // it reads a field from a heap-resident struct that the producing
        // call placed there").
        let builtin_result = match receiver_kind {
            // §2.7.5 typed Arc<String> raw-pointer carrier. The JIT-
            // format `call_string_method` still expects the legacy
            // NaN-boxed UnifiedValue<Arc<String>> wrapper shape; the
            // kinded String migration is W10 jit-playbook §5 territory.
            // Routing through call_string_method preserves the existing
            // JIT-format string method tests.
            NativeKind::String => call_string_method(receiver_bits, &method_name, &args),
            // §2.7.5 typed-Arc heap carriers — these are the non-
            // delegated `Ptr(_)` arms (TypedObject / TypedArray / Closure
            // / TraitObject / etc.). Method dispatch on these via the
            // JIT-format legacy path lands at the user-method UFCS
            // fallback below — there are no JIT-format builtin method
            // registries for these kinds. The W10 jit-playbook §5
            // kinded-array migration will fill this surface in a
            // future cluster.
            NativeKind::Ptr(hk) => {
                classify_kinded_ptr_receiver_for_jit_format_surface(hk);
                TAG_NULL
            }
            // UInt64 carrier — discriminate via the heap-prefix
            // `kind: u16` field-load. This is the canonical path for
            // legacy JIT-format kinds (HK_ARRAY / HK_JIT_OBJECT /
            // HK_DURATION / HK_TIME / HK_MATRIX / HK_OK / HK_ERR / …)
            // whose producing allocator (`jit_box` / `unified_box`)
            // places the kind discriminator at offset 0 of the
            // allocation.
            NativeKind::UInt64 => {
                if receiver_bits == 0
                    || receiver_bits == TAG_NULL
                    || receiver_bits == TAG_NONE
                {
                    TAG_NULL
                } else if receiver_bits & 0x1 != 0 || receiver_bits < 0x1000 {
                    // v0.3.3 c6-binop-ref-operand-segfault defense-in-depth
                    // (Wave 1 Round 2, 2026-05-28). The §2.7.7/Q9 parallel-
                    // kind track classified the receiver as UInt64 (opaque
                    // JIT-format heap bits), but the bits are not a
                    // plausible `Box::into_raw`-returned `JitAlloc<T>` /
                    // `UnifiedValue<T>` pointer: either not 2-byte aligned
                    // (the `read_heap_kind` u16-read alignment requirement)
                    // or smaller than a typical heap allocation address
                    // (the bits look like a raw scalar value, e.g. the
                    // empirically-observed `0x5` from `f(&a) + &a` where
                    // a borrowed-int-cell payload reached this shell).
                    // Pre-fix: `read_heap_kind` would dereference the
                    // malformed bits and panic-abort with Rust's
                    // `misaligned pointer dereference` UB guard → SIGSEGV /
                    // SIGABRT across the extern-"C" FFI boundary. The
                    // compiler-side `&`-as-binop-operand gate in
                    // `crates/shape-vm/src/compiler/expressions/
                    // binary_ops.rs::compile_expr_binary_op` makes the
                    // documented repro unreachable in well-formed Shape,
                    // but this defense-in-depth surface-and-stop (per
                    // c4-4B SURFACE-and-deopt precedent) prevents future
                    // regression if a new path emerges. Routes through
                    // the `pending_call_error` deopt-to-interpreter
                    // pattern; the VM then surfaces the actual cause.
                    tracing::debug!(
                        target: "shape_jit",
                        method_name = %method_name,
                        receiver_bits,
                        "jit-call-method SURFACE: UInt64 carrier kind with \
                         malformed receiver bits (not a plausible heap \
                         pointer: misaligned for u16 read or below \
                         minimum heap address). Likely a misclassified \
                         scalar or reference operand reaching the legacy \
                         heap-prefix dispatch. Raising pending_call_error \
                         for MIR-emitted deopt to interpreter fall-\
                         through (W12 pattern + v0.3.3 c6 defense-in-\
                         depth).",
                    );
                    super::control::set_jit_runtime_error(format!(
                        "JIT method dispatch for `.{}()` reached the \
                         heap-prefix path with malformed receiver bits \
                         — deopting to interpreter",
                        method_name,
                    ));
                    ctx_ref.pending_call_error = 1;
                    return TAG_NULL;
                } else {
                    match read_heap_kind(receiver_bits) {
                        HK_OK | HK_ERR | HK_SOME => {
                            tracing::debug!(
                                target: "shape_jit",
                                method_name = %method_name,
                                receiver_bits,
                                "jit-call-method SURFACE: legacy Result/Option \
                                 carrier (HK_OK/HK_ERR/HK_SOME) reached the \
                                 UInt64 JIT-format dispatch path. Strict \
                                 Result/Option receivers must be stamped as \
                                 Ptr(HeapKind::Result) / Ptr(HeapKind::Option) \
                                 and delegated to the VM trampoline; deopting \
                                 to interpreter instead of using the retired \
                                 UnifiedValue<u64> method helper.",
                            );
                            super::control::set_jit_runtime_error(format!(
                                "JIT method dispatch for `.{}()` reached a \
                                 legacy Result/Option carrier — deopting to \
                                 interpreter",
                                method_name,
                            ));
                            ctx_ref.pending_call_error = 1;
                            return TAG_NULL;
                        }
                        HK_ARRAY => call_array_method(receiver_bits, &method_name, &args),
                        HK_STRING => call_string_method(receiver_bits, &method_name, &args),
                        HK_JIT_OBJECT => call_object_method(receiver_bits, &method_name, &args),
                        HK_DURATION => {
                            call_duration_method(receiver_bits, &method_name, &args)
                        }
                        HK_COLUMN_REF => TAG_NULL,
                        HK_MATRIX => call_matrix_method(receiver_bits, &method_name, &args),
                        HK_TIME => call_time_method(receiver_bits, &method_name, &args),
                        _ => TAG_NULL,
                    }
                }
            }
            // Scalar / numeric kinds — all delegated to VM above (lines
            // 380-432) so they don't reach this cascade in practice;
            // returning TAG_NULL is defensive (a stack-pop-then-re-
            // classification bug would have surfaced before here).
            NativeKind::Float64
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
            | NativeKind::Bool
            // Round 19 S1.5 W12-nativekind-scalar-additions (2026-05-14):
            // F32 / Char already delegated to VM above; defensive
            // TAG_NULL arm if they reach this fallthrough.
            | NativeKind::Float32
            | NativeKind::Char
            // Wave 2 Agent B W12-StringV2-DecimalV2-NativeKind-additions
            // (2026-05-14): StringV2 / DecimalV2 already delegated to VM
            // above; defensive TAG_NULL arm if they reach this
            // fallthrough.
            | NativeKind::StringV2
            | NativeKind::DecimalV2 => TAG_NULL,
            // R5b-2-bool-null-sentinel-cluster (ADR-006 §2.7 +
            // §2.7.7/Q9, 2026-05-19): null receivers delegate to VM
            // (defensive TAG_NULL arm).
            NativeKind::Null => TAG_NULL,
        };

        // User-defined method dispatch (UFCS — `"TypeName::method"`
        // functions in the JIT function table). The receiver kind from
        // the §2.7.7 / Q9 parallel-kind track flows into
        // `receiver_type_name` so dispatch classifies on the producing
        // call's stamp, not on tag-bit decode.
        if std::env::var_os("SHAPE_DEBUG_FIELD_STAMPS").is_some()
            && method_name == "summary"
        {
            eprintln!(
                "[method-debug] before ufcs method={} receiver_kind={:?} builtin_result={} tag_null={} will_try={}",
                method_name,
                receiver_kind,
                builtin_result,
                TAG_NULL,
                builtin_result == TAG_NULL
            );
        }
        if builtin_result == TAG_NULL {
            if let Some(user_result) =
                try_call_user_method(ctx, receiver_bits, receiver_kind, &method_name, &arg_pairs)
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
