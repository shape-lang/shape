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
use shape_value::{HeapKind, KindedSlot, NativeKind, ValueSlot, encoding::ERROR_PLACEHOLDER_BITS};

/// What a method dispatch produced (ADR-020 / #239 §4.1, the METHOD channel).
///
/// The value arm carries the kind its PRODUCER stamped — the VM's
/// `dispatch_method_kinded` result for a delegated receiver, or the callee's
/// own §2.7.7/Q9 `stack_kinds[0]` stamp for a JIT-compiled UFCS method. There
/// is no third source: the JIT-format NaN-box cascade cannot supply a kind, so
/// a path that would return its bits produces [`MethodOutcome::Failed`] rather
/// than a fabricated label (#236 / R-G7).
///
/// Note what is NOT here, and why. The value channel's `Outcome` has a
/// `NoValue` arm because a unit-returning callee legitimately produces nothing
/// (ADR-020 §3.3), which is why it needs a `_void` monomorph. A method always
/// produces a value: `dispatch_method_kinded` returns a `KindedSlot` on every
/// success path, and `infer_unit_slots` only ever unit-classifies destinations
/// of `MirConstant::Function` callees — never a `MirConstant::Method` one. So
/// the method channel has three ABI classes, not four, and a `Void`
/// classification for a method destination is a contradiction the emit site
/// surfaces rather than an entry point that exists.
pub(crate) enum MethodOutcome {
    /// A value, WITH the kind its producer stamped. The `KindedSlot` owns the
    /// share (O1); whoever consumes it transfers that share onward without a
    /// second claim.
    Value(KindedSlot),
    /// The dispatch aborted. `pending_call_error` is already set, and the
    /// emitted post-call check deopts the frame before the value is read.
    Failed,
}

// Module declarations
pub mod duration;
pub mod matrix;
pub mod number;
pub mod object;
pub mod string;
pub mod time;

// Re-export the individual method handlers
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
        | HeapKind::ForeignRef
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
        | NativeKind::Ptr(HeapKind::ForeignRef)
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
) -> Option<MethodOutcome> {
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
            if std::env::var_os("SHAPE_DEBUG_FIELD_STAMPS").is_some() && method_name == "summary" {
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
            if std::env::var_os("SHAPE_DEBUG_FIELD_STAMPS").is_some() && method_name == "summary" {
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
        return Some(MethodOutcome::Failed);
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
        return Some(MethodOutcome::Failed);
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
    // `fn(ctx_ptr, param_0..param_{arity-1}) -> i32`
    // (`compile_function_with_user_funcs` in `compiler/program.rs`, one
    // appended I64 param per `Function.arity`). For a CLOSURE the leading
    // `captures_count` of those params ARE the captures — `arity` already
    // includes them, so `captures_count` is NOT an addend (adding it
    // double-counted the captures and was the cause of the "no capturing
    // closure reaches native JIT" defect; see the lockstep comments on both
    // signature sites). Its entry-block parameter init reads
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
    // Matches the JIT-compiled callee's native ABI
    // `fn(ctx, param_0..param_{arity-1})` per
    // `compile_function_with_user_funcs`. Method bodies are not closures
    // (`captures_count == 0`), so `arity` here is exactly `self` + user args
    // and this receiver-plus-args construction is unchanged.
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
    //
    // ADR-020 / #239 §4.1: read the kind BEFORE clearing the slot. #239 §6.6
    // made `Return` lowering stamp `stack_kinds[0]` in lockstep with the data
    // push — the one push in `terminators.rs` that had skipped the §2.7.7/Q9
    // lockstep. So a JIT-compiled UFCS callee's return kind is available here
    // and this function no longer has to hand back an unlabelled `u64`.
    if ctx_mut.stack_ptr > 0 {
        ctx_mut.stack_ptr -= 1;
        let result = ctx_mut.stack[ctx_mut.stack_ptr];
        let code = ctx_mut.stack_kinds[ctx_mut.stack_ptr];
        ctx_mut.stack_kinds[ctx_mut.stack_ptr] = stack_kind_code::SENTINEL;
        let Some(kind) = stack_kind_code::decode(code) else {
            super::control::set_jit_runtime_error(format!(
                "JIT method dispatch for `{}` resolved `{}`, but the callee left its \
                 return slot's §2.7.7/Q9 kind byte unstamped (SENTINEL / reserved). \
                 The return-ABI class cannot be checked against the destination \
                 without it, and no kind may be fabricated from the bits \
                 (ADR-020 §4.1, #236/R-G7); native execution aborted.",
                method_name, resolved_name,
            ));
            ctx_mut.pending_call_error = 1;
            return Some(MethodOutcome::Failed);
        };
        Some(MethodOutcome::Value(KindedSlot::new(
            ValueSlot::from_raw(result),
            kind,
        )))
    } else {
        // The callee produced nothing. Every method call has a destination
        // expecting a value (`unit_slots` is derived from named-function ABI
        // only, so a method destination is never unit-classified), so this is
        // a callee/declaration disagreement rather than a legal void return —
        // the method-channel analogue of `abort_missing_value` on the value
        // channel. Before this conversion it was `Some(TAG_NULL)`, i.e. the
        // same answer as a genuine null.
        super::control::set_jit_runtime_error(format!(
            "JIT method dispatch for `{}` resolved `{}` but the callee left no value \
             on the JIT stack, while the destination slot expects one; native \
             execution aborted.",
            method_name, resolved_name,
        ));
        ctx_mut.pending_call_error = 1;
        Some(MethodOutcome::Failed)
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

/// Does this receiver kind route to the VM trampoline's PHF registry
/// (`dispatch_method_kinded`), rather than the legacy JIT-format NaN-box
/// cascade?
///
/// ADR-020 / #239 §4.1. The two sides of this question used to be answered in
/// two places — a `delegated` match inside the shell, and a pair of
/// receiver-kind guards (STAGE-StringJIT, STAGE-F3) at the MIR emit site that
/// hard-coded which receivers the shell would mishandle. That is the
/// correspondence hazard §6.8 records: two independently-maintained copies of
/// one classification, agreeing by intention rather than by construction. This
/// function is the single answer; the shell is its only consumer, and the emit
/// site no longer needs to predict it because the shell now returns a KIND
/// rather than unlabelled bits.
///
/// A kind delegates iff the VM's `resolve_method_handler` can dispatch it from
/// the carrier the JIT actually holds:
///
/// * scalars and `Null` — the VM has the full scalar registries
///   (`NUMBER_METHODS` / `BOOL_METHODS` / `CHAR_METHODS`), and a null receiver
///   gets the VM's uniform TypeError;
/// * `String` / `StringV2` / `DecimalV2` — §2.7.5 raw-carrier receivers. This
///   is the STAGE-StringJIT flip: a `NativeKind::String` slot carries
///   `Arc::into_raw(Arc<String>)` (see `ownership.rs::retain_func_for_kind`,
///   whose String arm is `Arc::increment_strong_count::<String>`), which is
///   exactly what `STRING_METHODS` expects and is NOT what the JIT-format
///   `call_string_method` expects. Routing String to the legacy cascade was
///   the carrier mismatch STAGE-StringJIT existed to refuse;
/// * the typed-Arc collection carriers, `Result` / `Option`, and `TypedArray`;
/// * the seven VM-only typed-Arc receivers of STAGE-F3 — `Temporal`,
///   `Instant`, `Decimal`, `BigInt`, `DataTable`, `TableView`, `Content`. The
///   VM dispatches all seven through its PHF registry; the JIT-format cascade
///   has no registry for them at all and hit a silent `Ptr(_) => TAG_NULL`.
///   STAGE-F3's own text names this routing as the fix.
///
/// It does NOT delegate for `UInt64` (the documented opaque-JIT-bits carrier,
/// whose receivers are genuine JIT-format NaN-box allocations) or for the
/// remaining `Ptr(_)` labels including `TypedObject` — those reach
/// `try_call_user_method`, which dispatches user methods natively through the
/// JIT function table and is a faster path than the trampoline, not a broken
/// one.
pub(crate) fn delegates_to_vm_trampoline(kind: NativeKind) -> bool {
    match kind {
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
        // r5c-2-gz-CP9: typed-array receivers that reach the shell (`count` /
        // `group` / `groupBy` / `contains`; the cheap ones are intercepted
        // inline by `try_emit_v2_array_method` and never arrive) delegate so
        // an unimplemented method surfaces a clean VM `Err` instead of the
        // silent `Ptr(_) => TAG_NULL` placeholder.
        | NativeKind::Ptr(HeapKind::TypedArray)
        // ── STAGE-F3, retired ────────────────────────────────────────────
        // The seven VM-only typed-Arc receivers. Their methods live only in
        // the VM's PHF registry, so the JIT-format cascade returned TAG_NULL
        // into a proven-`Int64` destination while the live receiver was
        // dropped through the wrong carrier at frame teardown (rc=139 on
        // `fn f(d: DateTime) -> int { d.unix_timestamp() + 1 }`). Delegation
        // is the routing STAGE-F3's own text prescribed.
        | NativeKind::Ptr(HeapKind::Temporal)
        | NativeKind::Ptr(HeapKind::Instant)
        | NativeKind::Ptr(HeapKind::Decimal)
        | NativeKind::Ptr(HeapKind::BigInt)
        | NativeKind::Ptr(HeapKind::DataTable)
        | NativeKind::Ptr(HeapKind::TableView)
        | NativeKind::Ptr(HeapKind::Content)
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
        | NativeKind::Float32
        | NativeKind::Char
        // ── STAGE-StringJIT / STAGE-M1, retired ──────────────────────────
        // §2.7.5 raw-carrier string receivers. See the docstring above: the
        // carrier the JIT holds is the one `STRING_METHODS` reads, and the
        // one `call_string_method` does not.
        | NativeKind::String
        | NativeKind::StringV2
        | NativeKind::DecimalV2
        // R5b-2-bool-null-sentinel-cluster: null receivers get the VM's
        // uniform TypeError.
        | NativeKind::Null => true,
        // The opaque-JIT-bits carrier — genuine JIT-format NaN-box
        // allocations, discriminated by the heap-prefix `kind: u16` field.
        NativeKind::UInt64 => false,
        // Remaining `Ptr(_)` labels, incl. `TypedObject`: the legacy path,
        // whose real work is `try_call_user_method`'s native UFCS dispatch.
        NativeKind::Ptr(hk) => {
            classify_kinded_ptr_receiver_for_jit_format_surface(hk);
            false
        }
    }
}

/// The method-dispatch shell (ADR-020 / #239 §4.1).
///
/// Returns a [`MethodOutcome`] rather than a `u64`. `jit_call_method` used to
/// return `-> u64`, and the kind was thrown away at
/// `VirtualMachine::jit_trampoline_call_method` under the comment "the JIT
/// caller knows the static return kind from the callee method signature" — it
/// did not, because the signature had nowhere to put one. That is why every
/// scalar-returning method on a `string` receiver had to whole-function deopt:
/// there was no channel to bring a raw scalar back in, only `box_number`'s
/// NaN-boxed f64 and the `TAG_BOOL_*` sentinels.
///
/// The four `jit_call_method_*` entry points below are this function plus a
/// return-ABI class assertion; the class is selected at the emit site from the
/// destination slot's PROVEN kind (`call_return_abi_class`), never here.
fn call_method_kinded(ctx: *mut JITContext, stack_count: usize) -> MethodOutcome {
    use crate::ffi::stack_kind_code;
    use shape_value::{HeapKind, NativeKind};

    unsafe {
        if ctx.is_null() || stack_count < 3 {
            // #234 B1: unreachable absent a JIT codegen bug, and there is no
            // context to record `pending_call_error` in — the context IS what
            // is null. Memory-safe by #234.
            return MethodOutcome::Failed;
        }

        let ctx_ref = &mut *ctx;

        // ── Pop arg_count ──────────────────────────────────────────────
        // ABI: the MIR producer stores `arg_count` as a raw i64 with
        // parallel-kind `UInt64` (sentinel slot — `terminators.rs:259`).
        // We decode it directly as usize — no NaN-box.
        if ctx_ref.stack_ptr == 0 {
            // #234 c1: corrupted JIT state — record the error so the caller
            // deopts to the interpreter instead of computing on garbage.
            crate::ffi::control::set_jit_runtime_error(
                "jit_call_method: operand stack empty while popping arg_count — deopting to interpreter".to_string(),
            );
            ctx_ref.pending_call_error = 1;
            return MethodOutcome::Failed;
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
            // #234 c1: corrupted JIT state — record the error so the caller
            // deopts to the interpreter instead of computing on garbage.
            crate::ffi::control::set_jit_runtime_error(
                "jit_call_method: operand stack empty while popping the method name — deopting to interpreter".to_string(),
            );
            ctx_ref.pending_call_error = 1;
            return MethodOutcome::Failed;
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
                ctx_ref.pending_call_error = 1;
                return MethodOutcome::Failed;
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
            ctx_ref.pending_call_error = 1;
            return MethodOutcome::Failed;
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
                // #234 c1: corrupted JIT state — record the error so the caller
                // deopts to the interpreter instead of computing on garbage.
                crate::ffi::control::set_jit_runtime_error(
                    "jit_call_method: operand stack exhausted mid-argument-pop — deopting to interpreter".to_string(),
                );
                ctx_ref.pending_call_error = 1;
                return MethodOutcome::Failed;
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
                    ctx_ref.pending_call_error = 1;
                    return MethodOutcome::Failed;
                }
            };
            arg_pairs.push((bits, kind));
        }
        arg_pairs.reverse();

        // ── Pop receiver paired with its parallel-track kind ──────────
        if ctx_ref.stack_ptr == 0 {
            // #234 c1: corrupted JIT state — record the error so the caller
            // deopts to the interpreter instead of computing on garbage.
            crate::ffi::control::set_jit_runtime_error(
                "jit_call_method: operand stack empty while popping the receiver — deopting to interpreter".to_string(),
            );
            ctx_ref.pending_call_error = 1;
            return MethodOutcome::Failed;
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
                ctx_ref.pending_call_error = 1;
                return MethodOutcome::Failed;
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
        // ADR-020 / #239 §4.1: ONE classifier, shared with the emit-side
        // documentation of what it means (`delegates_to_vm_trampoline`). The
        // 130-line inline `match` this replaces was the same decision written
        // out here, with the receivers it got wrong pinned by two separate
        // emit-site guards (STAGE-StringJIT, STAGE-F3) in another crate module.
        let delegated = delegates_to_vm_trampoline(receiver_kind);

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
            return MethodOutcome::Failed;
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
                // ADR-020 / #239 §4.1: the VM hands back its `KindedSlot`
                // intact. The kind is `dispatch_method_kinded`'s own answer —
                // the PHF handler's declared result kind — so the monomorph's
                // `classes_agree` check compares two producer-stamped facts.
                // This used to be `Some(Ok(bits)) => return bits`, with the
                // kind dropped one frame down in the VM.
                Some(Ok(slot)) => return MethodOutcome::Value(slot),
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
                    return MethodOutcome::Failed;
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
                    return MethodOutcome::Failed;
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
                    return MethodOutcome::Failed;
                }
                _ => {}
            }
            match method_name.as_str() {
                "find" | "findIndex" | "some" | "every" | "filter" | "map" | "reduce" => {
                    if args.is_empty() {
                        // ADR-020 / #239 §5: this was a bare bail — a
                        // placeholder returned as if it were a value, with no
                        // error recorded, so the emitted post-call check had
                        // nothing to deopt on. Name it and set the flag.
                        super::control::set_jit_runtime_error(format!(
                            "JIT method dispatch for array `.{}()` reached the \
                             higher-order path with no callback argument \
                             — deopting to interpreter",
                            method_name,
                        ));
                        ctx_ref.pending_call_error = 1;
                        return MethodOutcome::Failed;
                    }
                    let predicate = args[0];
                    let working_array_bits = receiver_bits;

                    let _ = (predicate, working_array_bits);
                    // ADR-020 / #239 §4.1. The `jit_control_*` helpers return
                    // JIT-format NaN-box dialect bits — `box_number` results
                    // and `TAG_*` sentinels — with no kind to accompany them.
                    // Under the converted channel every value the shell hands
                    // back carries the kind its producer stamped, and there is
                    // no producer here to ask: labelling these bits would be
                    // fabricating a `NativeKind` from a carrier that has none
                    // (#236 / R-G7), and it is exactly the mislabelling that
                    // made `s.indexOf(..)` return `f64::to_bits(2.0)` read as
                    // an i64.
                    //
                    // #189 measured this arm unreached by ordinary Shape:
                    // every array method probed under `--mode jit` arrives
                    // stamped `Ptr(HeapKind::TypedArray)` and delegates to the
                    // VM instead. It is kept as the structured bail its
                    // siblings use rather than deleted with the rest of the
                    // dialect, so a producer-side stamp regression deopts
                    // cleanly instead of writing dialect bits into a typed
                    // slot. Deleting it belongs with the dialect (§7).
                    super::control::set_jit_runtime_error(format!(
                        "JIT codegen for higher-order array `.{}()` on a \
                         legacy JIT-format receiver produces a NaN-boxed \
                         result with no NativeKind — deopting to interpreter",
                        method_name,
                    ));
                    ctx_ref.pending_call_error = 1;
                    return MethodOutcome::Failed;
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
                    return MethodOutcome::Failed;
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
                            return MethodOutcome::Failed;
                        }
                        // #189: this arm used to call `call_array_method`,
                        // whose entire body was a `todo!()`. A `todo!()` in a
                        // production dispatch table is a process ABORT, not a
                        // refusal: nothing above it can catch it, the VM never
                        // gets the chance to run the construct correctly, and
                        // the user sees SIGABRT instead of a program. That is a
                        // crash bug independent of any performance question,
                        // which is why it is fixed before the carrier work.
                        //
                        // It is now the same structured per-function bail its
                        // siblings above use (legacy Result/Option carrier,
                        // malformed receiver bits): name the reason, raise
                        // `pending_call_error` so the MIR-emitted check deopts
                        // the JIT frame, and let the W12 fall-through run the
                        // method on the interpreter, whose array handlers are
                        // carrier-correct. VM == JIT, never an abort.
                        //
                        // Reaching here means an array receiver arrived on the
                        // legacy `UInt64` JIT-format path carrying the deleted
                        // `unified_box(HK_ARRAY, ..)` shape rather than being
                        // stamped `Ptr(HeapKind::TypedArray)` and delegated to
                        // the VM trampoline earlier. No ordinary Shape source
                        // was found that reaches it (every array method probed
                        // under `--mode jit` takes the stamped path), so this
                        // is defence in depth on a path that must not abort if
                        // a producer-side stamp ever regresses.
                        HK_ARRAY => {
                            tracing::debug!(
                                target: "shape_jit",
                                method_name = %method_name,
                                receiver_bits,
                                "jit-call-method SURFACE: array receiver reached \
                                 the legacy UInt64 JIT-format dispatch path \
                                 (deleted `unified_box(HK_ARRAY, ..)` carrier). \
                                 Strict array receivers must be stamped \
                                 Ptr(HeapKind::TypedArray) and delegated to the \
                                 VM trampoline; deopting to the interpreter \
                                 instead of aborting.",
                            );
                            super::control::set_jit_runtime_error(format!(
                                "JIT method dispatch for `.{}()` reached a \
                                 legacy array carrier — deopting to interpreter",
                                method_name,
                            ));
                            ctx_ref.pending_call_error = 1;
                            return MethodOutcome::Failed;
                        }
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
        if std::env::var_os("SHAPE_DEBUG_FIELD_STAMPS").is_some() && method_name == "summary" {
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
                // A JIT-compiled UFCS method. Its return kind comes off the
                // callee's own §2.7.7/Q9 return-slot stamp — a producer fact,
                // read inside `try_call_user_method` before the slot is reset.
                return user_result;
            }
        }

        // ADR-020 / #239 §4.1 — the JIT-format cascade cannot supply a kind.
        //
        // Reaching here means the legacy `builtin_result` cascade produced
        // something (`box_number`, a `TAG_BOOL_*` sentinel, a `box_string`
        // carrier) and no UFCS method claimed the call. Those are NaN-box
        // dialect bits; there is no `NativeKind` to accompany them and none may
        // be invented (#236 / R-G7). Handing them back labelled would recreate
        // exactly the defect STAGE-StringJIT refused at compile time —
        // `s.indexOf("l")` reaching an `Int64` slot as `f64::to_bits(2.0)`,
        // rc=0 silent-wrong.
        //
        // So the dialect's own results are a clean deopt: the interpreter runs
        // the method through its carrier-correct registry, VM == JIT. The
        // cascade is retained rather than deleted because its `TAG_NULL` answer
        // is still the "not a builtin, try UFCS" signal above; deleting it
        // belongs with the rest of the dialect (§7), not here.
        //
        // A `TAG_NULL` cascade result with no UFCS match means the method did
        // not resolve at all. That also deopts — it used to be returned as a
        // value, which is the bare-bail shape §5 inventories.
        super::control::set_jit_runtime_error(format!(
            "JIT method dispatch for `.{}()` on a {:?} receiver produced a \
             legacy JIT-format result with no NativeKind (or resolved to no \
             method at all) — deopting to interpreter",
            method_name, receiver_kind,
        ));
        ctx_ref.pending_call_error = 1;
        MethodOutcome::Failed
    }
}

/// Method call whose destination slot's proven kind is `Scalar`-class.
///
/// Returns the raw scalar — no `box_number`, no `TAG_BOOL_*`. This is the
/// monomorph that retires STAGE-StringJIT: `fn f(s: string) -> int {
/// s.indexOf("l") }` now has a channel that can carry the `2` back as a `2`.
pub extern "C" fn jit_call_method_i64(ctx: *mut JITContext, stack_count: usize) -> i64 {
    use crate::return_abi_class::{ReturnAbiClass, classes_agree};
    const CLASS: ReturnAbiClass = ReturnAbiClass::Scalar;
    match call_method_kinded(ctx, stack_count) {
        MethodOutcome::Value(k) => {
            if !classes_agree(CLASS, k.kind) {
                crate::ffi::control::abort_class_disagreement(
                    ctx,
                    "jit_call_method",
                    CLASS,
                    k.kind,
                );
                // `k` drops here, retiring the share it owns — the value never
                // reaches the destination, so nothing else can retire it.
                return ERROR_PLACEHOLDER_BITS as i64;
            }
            let bits = k.slot.raw();
            // Scalar: no share to transfer. `KindedSlot::drop` is a no-op on a
            // scalar kind, so forgetting and dropping are equivalent; forget
            // deliberately, so this reads the same as the pointer monomorph.
            std::mem::forget(k);
            bits as i64
        }
        MethodOutcome::Failed => ERROR_PLACEHOLDER_BITS as i64,
    }
}

/// Method call whose destination slot's proven kind is `Float`-class.
///
/// Returns an `f64` in an FP register. This is where `box_number` dies on the
/// method path: the value never becomes a bit pattern at the Cranelift
/// boundary, so there is nothing for a consumer to mis-read as an integer.
pub extern "C" fn jit_call_method_f64(ctx: *mut JITContext, stack_count: usize) -> f64 {
    use crate::return_abi_class::{ReturnAbiClass, classes_agree};
    const CLASS: ReturnAbiClass = ReturnAbiClass::Float;
    match call_method_kinded(ctx, stack_count) {
        MethodOutcome::Value(k) => {
            if !classes_agree(CLASS, k.kind) {
                crate::ffi::control::abort_class_disagreement(
                    ctx,
                    "jit_call_method",
                    CLASS,
                    k.kind,
                );
                return 0.0;
            }
            let bits = k.slot.raw();
            std::mem::forget(k);
            f64::from_bits(bits)
        }
        MethodOutcome::Failed => 0.0,
    }
}

/// Method call whose destination slot's proven kind is `Pointer`-class.
///
/// **O1/O2.** Returns exactly one owned share. The emit site releases the
/// destination's old value and stores this one WITHOUT retaining; the `forget`
/// is what makes that balance — dropping the `KindedSlot` instead would retire
/// the share the destination is about to own.
///
/// This is also what retires STAGE-M1: a string-RETURNING method on a proven
/// `NativeKind::String` receiver now goes through the VM's `STRING_METHODS`,
/// which produces the raw-Arc carrier every §2.7.5 `String` consumer decodes,
/// instead of `box_string`'s NaN-boxed carrier whose `Arc::decrement_strong_count`
/// dereferenced mantissa bits.
pub extern "C" fn jit_call_method_ptr(
    ctx: *mut JITContext,
    stack_count: usize,
) -> *mut std::ffi::c_void {
    use crate::return_abi_class::{ReturnAbiClass, classes_agree};
    const CLASS: ReturnAbiClass = ReturnAbiClass::Pointer;
    match call_method_kinded(ctx, stack_count) {
        MethodOutcome::Value(k) => {
            if !classes_agree(CLASS, k.kind) {
                crate::ffi::control::abort_class_disagreement(
                    ctx,
                    "jit_call_method",
                    CLASS,
                    k.kind,
                );
                return std::ptr::null_mut();
            }
            let bits = k.slot.raw();
            std::mem::forget(k);
            bits as *mut std::ffi::c_void
        }
        MethodOutcome::Failed => std::ptr::null_mut(),
    }
}

#[cfg(test)]
mod delegation_classifier_tests {
    use super::delegates_to_vm_trampoline;
    use shape_value::{HeapKind, NativeKind};

    /// The receivers STAGE-StringJIT and STAGE-M1 refused to emit for.
    ///
    /// Both guards existed because a `NativeKind::String` receiver was routed
    /// to the JIT-format `call_string_method`, which reads the NaN-boxed
    /// `unified_box(HK_STRING, Arc<String>)` carrier — while a `String`-kinded
    /// slot actually holds `Arc::into_raw(Arc<String>)` (see
    /// `ownership.rs::retain_func_for_kind`, whose String arm is
    /// `Arc::increment_strong_count::<String>`). The VM's `STRING_METHODS`
    /// reads exactly the carrier the JIT holds.
    ///
    /// Flipping this back to `false` reinstates that mismatch silently — the
    /// guards that used to catch it downstream are deleted — so it is pinned
    /// here rather than left to the corpus, which measured this whole family
    /// as VACUOUS before the conversion.
    #[test]
    fn string_carrier_receivers_delegate_to_the_vm() {
        for k in [
            NativeKind::String,
            NativeKind::StringV2,
            NativeKind::DecimalV2,
        ] {
            assert!(
                delegates_to_vm_trampoline(k),
                "{k:?} must delegate: the JIT-format string registry reads a \
                 NaN-box carrier this slot does not hold"
            );
        }
    }

    /// STAGE-F3's seven VM-only typed-Arc receivers.
    ///
    /// The guard's own text prescribed this routing: these methods live only in
    /// the VM's PHF registry, and the JIT-format cascade had no registry for
    /// them, so they hit a silent `Ptr(_) => TAG_NULL` whose placeholder fed a
    /// proven-`Int64` destination while the live receiver was dropped through
    /// the wrong carrier at frame teardown (rc=139).
    #[test]
    fn the_seven_vm_only_typed_arc_receivers_delegate() {
        for hk in [
            HeapKind::Temporal,
            HeapKind::Instant,
            HeapKind::Decimal,
            HeapKind::BigInt,
            HeapKind::DataTable,
            HeapKind::TableView,
            HeapKind::Content,
        ] {
            assert!(
                delegates_to_vm_trampoline(NativeKind::Ptr(hk)),
                "Ptr({hk:?}) must delegate — STAGE-F3's routing"
            );
        }
    }

    /// The classification is not vacuously "everything delegates".
    ///
    /// `UInt64` is the documented opaque-JIT-bits carrier and its receivers are
    /// genuine NaN-box allocations; `TypedObject` reaches
    /// `try_call_user_method`'s native UFCS dispatch, which is faster than the
    /// trampoline rather than broken. A test that only asserted the `true`
    /// cases would pass against a `fn(_) -> true` stub.
    #[test]
    fn the_jit_format_carriers_do_not_delegate() {
        assert!(!delegates_to_vm_trampoline(NativeKind::UInt64));
        assert!(!delegates_to_vm_trampoline(NativeKind::Ptr(
            HeapKind::TypedObject
        )));
    }
}

#[cfg(test)]
mod legacy_array_carrier_bail_tests {
    use super::*;
    use crate::context::JITContext;
    use crate::ffi::jit_kinds::unified_box;
    use crate::ffi::stack_kind_code;
    use crate::ffi::value_ffi::HK_ARRAY;
    use crate::ffi::value_ffi::box_string;
    use shape_value::NativeKind;

    /// #189: the repaired `HK_ARRAY` arm, DRIVEN.
    ///
    /// Before this change the arm called `call_array_method`, whose whole body
    /// was `todo!()` — reaching it aborted the process, so there was no
    /// behaviour to assert and nothing upstream could recover. This test
    /// reaches the arm on purpose and asserts the structured bail its siblings
    /// use: `TAG_NULL` returned, `pending_call_error` raised so the
    /// MIR-emitted check deopts the JIT frame, and a message left for
    /// `JITExecutor` to surface. The test completing at all is the load-bearing
    /// half — under the old code the process would not survive to assert.
    ///
    /// The path is not reachable from ordinary Shape source today (array
    /// receivers are stamped `Ptr(HeapKind::TypedArray)` and delegated to the
    /// VM trampoline long before this cascade; every array method probed under
    /// `--mode jit` takes that path). This drives it directly, which is the
    /// only way to prove the arm behaves rather than aborts.
    #[test]
    fn a_legacy_array_carrier_deopts_instead_of_aborting() {
        let mut ctx = JITContext::default();
        // Stack contract (`jit_call_method`): [receiver, method_name, arg_count].
        // The receiver carries the deleted `unified_box(HK_ARRAY, ..)` shape on
        // the legacy `UInt64` JIT-format path — exactly the classification that
        // used to land in `todo!()`.
        let receiver = unified_box(HK_ARRAY, 0u64);
        ctx.stack[0] = receiver;
        ctx.stack_kinds[0] = stack_kind_code::encode(NativeKind::UInt64);
        ctx.stack[1] = box_string("reverse".to_string());
        ctx.stack_kinds[1] = stack_kind_code::encode(NativeKind::String);
        ctx.stack[2] = 0; // arg_count
        ctx.stack_kinds[2] = stack_kind_code::encode(NativeKind::UInt64);
        ctx.stack_ptr = 3;

        let _ = super::super::control::take_jit_runtime_error();
        // ADR-020 / #239 §4.1: driven through the `_i64` monomorph. Which of the
        // three is used is immaterial to what this test pins — the bail happens
        // inside `call_method_kinded`, before any class assertion — and `_i64`
        // is the one whose placeholder is still comparable to the pre-conversion
        // `ERROR_PLACEHOLDER_BITS` this assertion was written against.
        let result = jit_call_method_i64(&mut ctx as *mut JITContext, 3);

        assert_eq!(
            result, ERROR_PLACEHOLDER_BITS as i64,
            "the bail leaves the ruled placeholder in the value channel (#234). \
             Nothing reads it — `pending_call_error` below is the signal — and \
             it is 0 so that a leak onto a heap-kinded destination hits the \
             `bits == 0` guard in `jit_arc_result_retain` instead of passing it"
        );
        assert_eq!(
            ctx.pending_call_error, 1,
            "pending_call_error must be raised so the MIR-emitted check deopts \
             the JIT frame and the interpreter runs the method"
        );
        let message = super::super::control::take_jit_runtime_error()
            .expect("the bail must leave a message for JITExecutor to surface");
        assert!(
            message.contains("reverse"),
            "the surfaced message must name the method that could not be \
             dispatched, got {message:?}"
        );
        // Non-vacuity: several bails in this cascade raise
        // `pending_call_error` and name the method, so a test that only
        // checked those two facts would pass without ever reaching the
        // repaired arm. The malformed-receiver-bits bail earlier in the
        // cascade is the specific near-miss. Pin the arm's own wording.
        assert!(
            message.contains("legacy array carrier"),
            "the test must reach the repaired HK_ARRAY arm, not the \
             malformed-receiver-bits bail that precedes it; got {message:?}"
        );
    }
}
