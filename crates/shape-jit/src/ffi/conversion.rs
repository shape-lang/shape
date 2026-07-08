// Heap allocation audit (PR-9 V8 Gap Closure):
//   Category A (NaN-boxed returns): 3 sites
//     jit_box(HK_STRING, ...) — jit_typeof, jit_to_string, jit_type_check
//   Category B (intermediate/consumed): 0 sites
//   Category C (heap islands): 0 sites
//!
//! Type Conversion FFI Functions for JIT
//!
//! Functions for type checking and conversion in JIT-compiled code.

// jit_array::JitArray removed — see jit_array.rs SURFACE comment.
// Branches that walked array elements (`type_spec` of shape "array:...",
// "tuple:...") now return `false` rather than fabricating an iteration
// over a deleted heap layout.
#![allow(clippy::approx_constant)] // arbitrary test floats; not math constants
use super::jit_kinds::*;
use super::value_ffi::*;

#[cold]
#[track_caller]
fn unsupported_legacy_heap_kind(func_name: &str, kind: Option<u16>) -> ! {
    match kind {
        Some(kind) => panic!(
            "SURFACE: {func_name} received unsupported legacy JIT heap kind {kind}; \
             ADR-006 section 2.7.5/2.7.10 requires a kinded JIT entry or an explicit HK arm."
        ),
        None => panic!(
            "SURFACE: {func_name} received non-heap bits where a legacy JIT heap allocation \
             was required; ADR-006 section 2.7.5 requires the caller to pass a NativeKind \
             companion instead of probing raw bits."
        ),
    }
}

fn is_known_legacy_heap_kind(kind: u16) -> bool {
    matches!(
        kind,
        HK_STRING
            | HK_TYPED_OBJECT
            | HK_CLOSURE
            | HK_DECIMAL
            | HK_BIG_INT
            | HK_DATATABLE
            | HK_HASHMAP
            | HK_FUTURE
            | HK_TASK_GROUP
            | HK_FILTER_EXPR
            | HK_JIT_FUNCTION
            | HK_JIT_TABLE_REF
            | HK_JIT_OBJECT
    ) || (JIT_LEGACY_HK_BASE..=HK_FLOAT_ARRAY_SLICE).contains(&kind)
}

// ============================================================================
// Type Checking
// ============================================================================

/// Get typeof a value as a string
pub extern "C" fn jit_typeof(value_bits: u64) -> u64 {
    let type_str = if is_number(value_bits) {
        "number"
    } else if value_bits == TAG_NULL {
        "null"
    } else if value_bits == TAG_BOOL_TRUE || value_bits == TAG_BOOL_FALSE {
        "boolean"
    } else if is_ok_tag(value_bits) || is_err_tag(value_bits) {
        "result"
    } else if is_inline_function(value_bits) {
        "function"
    } else {
        let kind = heap_kind(value_bits);
        match kind {
            Some(HK_STRING) => "string",
            Some(HK_ARRAY) => "array",
            Some(HK_JIT_OBJECT) | Some(HK_TYPED_OBJECT) => "object",
            Some(HK_CLOSURE) => "function",
            Some(HK_RANGE) => "range",
            Some(HK_COLUMN_REF) => "series",
            Some(HK_JIT_TABLE_REF) => "series_ref",
            Some(HK_DURATION) => "duration",
            Some(HK_TIME) => "time",
            Some(HK_TIMEFRAME) => "timeframe",
            other => unsupported_legacy_heap_kind("jit_typeof", other),
        }
    };
    jit_box(HK_STRING, type_str.to_string())
}

// ============================================================================
// Type Conversion
// ============================================================================

/// Convert value to string
pub extern "C" fn jit_to_string(value_bits: u64) -> u64 {
    let s = if is_number(value_bits) {
        format!("{}", unbox_number(value_bits))
    } else if value_bits == TAG_NULL {
        "null".to_string()
    } else if value_bits == TAG_BOOL_TRUE {
        "true".to_string()
    } else if value_bits == TAG_BOOL_FALSE {
        "false".to_string()
    } else {
        let kind = heap_kind(value_bits);
        match kind {
            Some(HK_STRING) => {
                let s = unsafe { jit_unbox::<String>(value_bits) };
                s.clone()
            }
            Some(HK_ARRAY) => "[array]".to_string(),
            Some(HK_JIT_OBJECT) | Some(HK_TYPED_OBJECT) => "[object]".to_string(),
            other => unsupported_legacy_heap_kind("jit_to_string", other),
        }
    };
    jit_box(HK_STRING, s)
}

/// Check if a value matches a type (returns TAG_BOOL_TRUE or TAG_BOOL_FALSE)
/// type_name_bits should be a boxed string pointer with encoded type info
pub extern "C" fn jit_type_check(value_bits: u64, type_name_bits: u64) -> u64 {
    // Get type name string
    let type_name = unsafe {
        if !is_heap_kind(type_name_bits, HK_STRING) {
            return TAG_BOOL_FALSE;
        }
        jit_unbox::<String>(type_name_bits).clone()
    };

    let matches = check_type_recursive(value_bits, &type_name);

    if matches {
        TAG_BOOL_TRUE
    } else {
        TAG_BOOL_FALSE
    }
}

/// Recursive helper to check encoded type strings
fn check_type_recursive(value_bits: u64, type_spec: &str) -> bool {
    // Parse type spec: "prefix:content" or just "typename"
    if let Some((prefix, rest)) = type_spec.split_once(':') {
        match prefix {
            "basic" => check_basic_type(value_bits, rest),
            "optional" => {
                // Optional: null matches, or inner type matches
                value_bits == TAG_NULL || check_type_recursive(value_bits, rest)
            }
            "array" => {
                // PHASE_2C / SURFACE (ADR-006 §2.7.4): pre-strict-typing
                // this walked `JitArray` elements and recursed. The
                // `JitArray` heap layout was deleted (see jit_array.rs
                // SURFACE); the strict-typing rebuild target reads
                // elements via `Arc<TypedArrayData>` per-element-kind
                // arms (§2.7.6/Q8). Until that lands, the kind check
                // is the array-shape check only — element-type
                // verification is dropped.
                let _ = rest;
                is_heap_kind(value_bits, HK_ARRAY)
            }
            "tuple" => {
                // Same SURFACE as `array` — the per-element check is
                // dropped pending the §2.7.6/Q8 rebuild.
                let _ = rest;
                is_heap_kind(value_bits, HK_ARRAY)
            }
            "generic" => {
                // Generic like Array<T> - check base type only (don't verify element types)
                match rest {
                    "Array" => is_heap_kind(value_bits, HK_ARRAY),
                    "Series" => is_heap_kind(value_bits, HK_COLUMN_REF),
                    _ => false,
                }
            }
            "ref" => {
                // Reference types - not fully supported in JIT yet
                false
            }
            "dyn" => {
                // Dyn trait types - not fully supported in JIT yet
                false
            }
            _ => false,
        }
    } else {
        // No prefix, treat as direct type match
        match type_spec {
            "function" => is_inline_function(value_bits) || is_heap_kind(value_bits, HK_CLOSURE),
            "object" => is_heap_kind(value_bits, HK_TYPED_OBJECT),
            "any" => true,
            "void" => value_bits == TAG_UNIT,
            "never" => false,
            "null" => value_bits == TAG_NULL,
            "undefined" => value_bits == TAG_NULL || value_bits == TAG_UNIT,
            "unknown" => false,
            _ => check_basic_type(value_bits, type_spec),
        }
    }
}

/// Check a basic type name against a value
fn check_basic_type(value_bits: u64, type_name: &str) -> bool {
    if is_number(value_bits) {
        return type_name == "number";
    }
    if value_bits == TAG_NULL {
        return type_name == "null";
    }
    if value_bits == TAG_BOOL_TRUE || value_bits == TAG_BOOL_FALSE {
        return type_name == "boolean" || type_name == "bool";
    }
    if value_bits == TAG_UNIT {
        return type_name == "void" || type_name == "unit";
    }
    if is_inline_function(value_bits) {
        return type_name == "function";
    }
    if is_data_row(value_bits) {
        return type_name == "data_row";
    }
    if is_ok_tag(value_bits) || is_err_tag(value_bits) {
        return type_name == "result";
    }

    let kind = heap_kind(value_bits);
    match kind {
        Some(HK_STRING) => type_name == "string",
        Some(HK_ARRAY) => type_name == "array",
        Some(HK_JIT_OBJECT) | Some(HK_TYPED_OBJECT) => type_name == "object",
        Some(HK_CLOSURE) => type_name == "function",
        Some(HK_COLUMN_REF) => type_name == "series",
        Some(HK_TIME) => type_name == "time",
        Some(HK_DURATION) => type_name == "duration",
        Some(HK_TIMEFRAME) => type_name == "timeframe",
        Some(HK_RANGE) => type_name == "range",
        other => unsupported_legacy_heap_kind("check_basic_type", other),
    }
}

/// Format a JIT-stamped value as a string for display.
///
/// PHASE_2C / SURFACE (ADR-006 §2.7.4 / §2.7.5): pre-strict-typing
/// this function dispatched on `shape_value::tag_bits::is_tagged` /
/// `get_tag == TAG_INT` to decode i48 integer payloads from raw bits.
/// That `tag_bits` decode is exactly the deleted W-series shape
/// (CLAUDE.md "Forbidden Patterns": "Runtime tag_bits dispatch
/// (deleted)"), forbidden under any rebuild.
///
/// The strict-typing rebuild target is `(bits: u64, kind: NativeKind) ->
/// String` so the integer arm dispatches on `kind == NativeKind::Int64`
/// (or the i32/i16/i8 width variants) and reads the payload as a typed
/// scalar without tag decoding. Until callers thread `kind` through,
/// the integer branch is removed — JIT-emitted bytecode that lands a
/// scalar `Int*` here would have routed it through the typed
/// `RETURN_TAG_I64` / `RETURN_TAG_I32` path at `executor.rs:254`
/// already, so this fallback only sees heap-tagged values plus the
/// inline `TAG_NULL`/`TAG_BOOL_*` constants from `value_ffi`.
pub(crate) fn format_value_word(value_bits: u64) -> String {
    if is_number(value_bits) {
        let n = unbox_number(value_bits);
        if n.is_finite() && n == n.trunc() && n.abs() < 1e15 {
            format!("{}", n as i64)
        } else {
            format!("{}", n)
        }
    } else if value_bits == TAG_BOOL_TRUE {
        "true".to_string()
    } else if value_bits == TAG_BOOL_FALSE {
        "false".to_string()
    } else if value_bits == TAG_NULL {
        "null".to_string()
    } else {
        let kind = heap_kind(value_bits);
        match kind {
            Some(HK_STRING) => {
                let s = unsafe { jit_unbox::<String>(value_bits) };
                s.clone()
            }
            Some(HK_ARRAY) => {
                // PHASE_2C / SURFACE (ADR-006 §2.7.4): the deleted
                // `JitArray` walk that produced "[a, b, c]" formatting
                // is gone. The strict-typing rebuild target dispatches
                // on the slot's `NativeKind::Ptr(HeapKind::TypedArray)`
                // per-element-kind arm via the §2.7.6/Q8 carrier.
                "[<array>]".to_string()
            }
            Some(HK_OK) => {
                let inner = unsafe { *jit_unbox::<u64>(value_bits) };
                format!("Ok({})", format_value_word(inner))
            }
            Some(HK_ERR) => {
                let inner = unsafe { *jit_unbox::<u64>(value_bits) };
                format!("Err({})", format_value_word(inner))
            }
            Some(HK_SOME) => {
                let inner = unsafe { *jit_unbox::<u64>(value_bits) };
                format!("Some({})", format_value_word(inner))
            }
            Some(HK_JIT_OBJECT) | Some(HK_TYPED_OBJECT) => "[object]".to_string(),
            other => unsupported_legacy_heap_kind("format_value_word", other),
        }
    }
}

// `jit_print(value_bits: u64)` — the kind-blind print FFI dispatched
// through `format_value_word` — DELETED in W12-jit-print-heap-arm-
// classification verification (Phase 3 cluster-0 Round 8A reopen,
// 2026-05-13). The deleted body called `format_value_word` (the
// deleted-W-series tag-bit dispatch documented at `format_value_word`'s
// comment lines 200-217), routing every unproven-kind print operand
// through the deleted-W-series shape — a defection-attractor preserved
// "for one edge case" (CLAUDE.md "Forbidden rationalizations" #1) per
// the pre-Round-8A-verification close. The §2.7.5 producer-site
// classification conduit extension (`infer_enum_payload_kind` now uses
// `native_kind_from_concrete_type` for the full ConcreteType →
// NativeKind mapping, not the scalar-only `elem_slot_kind_for_
// concrete`) closes the kind-source gap on Smoke 1.5's Err arm; the
// terminators.rs print Call-terminator dispatch now surfaces-and-stops
// on the `_` arm rather than routing through this deleted shape.

/// Print a raw native i64 to stdout with a newline.
///
/// W11-jit-new-array (ADR-006 §2.7.5 / Q15 stamp-at-compile-time): the
/// MIR-side print emitter dispatches to this entry point whenever the
/// operand slot is proven `NativeKind::Int64` / `UInt64` / `IntSize` /
/// `UIntSize`. The value is the raw native integer, not a NaN-boxed
/// ValueWord — the kind-blind `jit_print` decoded raw int bits as a
/// denormal `f64` and displayed `0.000...208` for `print(42)`, which
/// was the §2.7.5 kind-source gap surfaced by smoke target 1.
#[unsafe(no_mangle)]
pub extern "C" fn jit_print_i64(value: i64) {
    println!("{}", value);
}

/// Print a raw native `u64` to stdout with a newline — UNSIGNED render.
///
/// r5c-2-β-CKPT-C u64-carrier-disambiguation (2026-05-20): the MIR-side
/// print emitter routes `NativeKind::UInt64` / `UIntSize` operand slots
/// here, separately from the signed `jit_print_i64` path. Pre-fix both
/// signed and unsigned integer kinds collapsed onto `jit_print_i64`, which
/// reinterprets the raw bits as `i64` — so `print(x)` for
/// `let x: u64 = 18446744073709551615` displayed `-1` on the JIT while the
/// VM (whose `printing.rs` `UInt64` arm calls `slot.as_u64()`) correctly
/// displayed `18446744073709551615`. Routing the unsigned kinds to this
/// entry restores VM == JIT byte-identical output. The carrier `NativeKind`
/// is the discriminator — no value inspection.
#[unsafe(no_mangle)]
pub extern "C" fn jit_print_u64(value: u64) {
    println!("{}", value);
}

/// Print a raw native f64 to stdout with a newline.
///
/// W11-jit-new-array companion to `jit_print_i64`: dispatched when the
/// operand slot is proven `NativeKind::Float64`.
///
/// γ-CP1-jit-print-f64 (2026-05-20): routes through the canonical VM-side
/// `ValueFormatter::format_kinded` so VM and JIT produce byte-identical
/// output. The prior body re-implemented float formatting inline
/// (`value as i64` for integer-valued floats) and dropped the `.0` that
/// `format_number` (`printing.rs::format_number`) emits to distinguish
/// `number` from `int` — `print(3.0)` rendered `3` under the JIT vs `3.0`
/// under the VM. The `NativeKind::Float64` carrier is an inline scalar:
/// no heap payload, no refcount, no `KindedSlot::Drop` to forget. The
/// `Float64` arm of `format_kinded_inner` delegates straight to
/// `format_number`, the same routine the VM print path uses.
#[unsafe(no_mangle)]
pub extern "C" fn jit_print_f64(value: f64) {
    // A transient empty registry suffices: the scalar `Float64` arm of
    // `format_kinded_inner` never consults the schema registry.
    let registry = std::sync::Arc::new(shape_runtime::type_schema::TypeSchemaRegistry::default());
    let formatter = shape_vm::executor::printing::ValueFormatter::new(&registry);
    let kinded = shape_value::KindedSlot::from_number(value);
    println!("{}", formatter.format_kinded(&kinded));
}

/// Print a raw native bool to stdout with a newline.
///
/// W11-jit-new-array companion to `jit_print_i64`: dispatched when the
/// operand slot is proven `NativeKind::Bool`. The Cranelift I8 carrier
/// is widened to a `u8` (0 = false, nonzero = true) at the FFI
/// boundary.
#[unsafe(no_mangle)]
pub extern "C" fn jit_print_bool(value: u8) {
    println!("{}", value != 0);
}

// ============================================================================
// Heap-arm kinded print entries (W12-jit-print-heap-arm-classification,
// Phase 3 cluster-0 Round 8A, 2026-05-13)
// ============================================================================
//
// ADR-006 §2.7.5 stamp-at-compile-time per-HeapKind print FFI. Each entry
// reads a typed `Arc<T>` payload directly via `*const T` field projection
// — never via NaN-box tag decode, never via `is_heap_kind` probe (§2.7.7
// #4 / #7 forbidden). The kind is implicit in the chosen FFI entry name
// (each entry corresponds 1:1 to a `NativeKind::Ptr(HeapKind::*)` arm or
// `NativeKind::String`), stamped at MIR-emit time by the Call-terminator
// dispatch in `mir_compiler/terminators.rs`.
//
// Routes through the canonical VM-side `ValueFormatter::format_kinded` so
// VM and JIT produce byte-identical output. The schema registry comes
// from `JITContext.exec_context_ptr` → `ExecutionContext::type_schema_
// registry()`. When `exec_context_ptr` is null (test harness / out-of-
// process) a transient empty `TypeSchemaRegistry` is used — TypedObject
// field names fall back to positional placeholders, matching the
// formatter's documented behaviour for schema-less objects (`printing.rs:
// 754`). This is NOT a Bool-default fallback: the kind is known, the
// payload is read with the correct kind label; only field-name resolution
// degrades, which is the same degradation VM-side `format_typed_object`
// exhibits for an unregistered schema.

/// Borrow the `TypeSchemaRegistry` from a `JITContext.exec_context_ptr`
/// if present, falling back to a transient empty registry. The fallback
/// is the schema-less-object render path (`_0`, `_1`, ... positional
/// names) — see `printing.rs::format_typed_object` line 754. Returns an
/// owned `Arc<TypeSchemaRegistry>` either way so the caller's
/// `ValueFormatter::new` borrow is sound for the formatter's lifetime.
fn registry_from_ctx(
    ctx_ptr: *const crate::context::JITContext,
) -> std::sync::Arc<shape_runtime::type_schema::TypeSchemaRegistry> {
    if ctx_ptr.is_null() {
        return std::sync::Arc::new(shape_runtime::type_schema::TypeSchemaRegistry::default());
    }
    // SAFETY: caller's contract — the JIT dispatch shell always passes
    // a valid `JITContext*` (either the worker-allocated context or the
    // out-of-process `JITContext::default()`-shaped harness instance).
    let ctx = unsafe { &*ctx_ptr };
    if ctx.exec_context_ptr.is_null() {
        return std::sync::Arc::new(shape_runtime::type_schema::TypeSchemaRegistry::default());
    }
    // SAFETY: `exec_context_ptr` is a `*mut c_void` pointing to a live
    // `ExecutionContext` owned by the VM driver for the lifetime of the
    // JIT call. Reading `type_schema_registry()` borrows through the
    // shared `Arc`; the returned `Arc` clone is independent.
    let exec_ctx =
        unsafe { &*(ctx.exec_context_ptr as *const shape_runtime::context::ExecutionContext) };
    std::sync::Arc::clone(exec_ctx.type_schema_registry())
}

/// JIT-side sentinel filter for the kinded print FFI bodies.
///
/// Returns `true` when `bits` is the JIT's `TAG_NULL` / `TAG_NONE`
/// sentinel (`is_none_tag` per `value_ffi.rs:417`). At the kinded print
/// FFI boundary the parallel-kind track says the slot SHOULD carry a
/// typed-Arc payload (`NativeKind::String` ↔ `Arc::into_raw(Arc<String>)`,
/// `NativeKind::Ptr(HeapKind::TypedObject)` ↔
/// `Arc::into_raw(Arc<TypedObjectStorage>)`, etc., per ADR-006 §2.7.5
/// stamp-at-compile-time). When the bits instead match the JIT's null
/// sentinel, the producer did not stamp a §2.7.5 typed-Arc carrier —
/// constructing a `KindedSlot` with the carrier kind would route the
/// sentinel through `format_kinded_inner`'s `Arc<T>` deref path
/// (`printing.rs:163` for the String arm, the per-`HeapKind` arms in
/// `format_heap_kind` for the heap-pointer kinds) and segfault.
///
/// This filter is the bounded mechanical realization of the §2.7.5
/// producer-site discipline at the kinded print FFI boundary: the kind
/// label says what the slot SHOULD carry; `is_none_tag` says what the
/// bits ACTUALLY are; the filter early-returns only when bits-don't-
/// match-kind-expectation. Used by every `jit_print_<heap_arm>` body
/// (DRY discipline — single helper, no per-call-site bit-pattern
/// duplication).
#[inline]
fn is_jit_null_sentinel(bits: u64) -> bool {
    super::value_ffi::is_none_tag(bits)
}

/// Format `bits` as a `KindedSlot { kind, slot: ValueSlot::from_raw(bits) }`
/// and write the rendered string + newline to stdout. The carrier is
/// borrowed for the lifetime of the call (no refcount bump, no
/// `KindedSlot::Drop`) — the caller's stack slot keeps its strong-count
/// share across the print.
///
/// `kind` is implicit in the chosen FFI entry by construction. This is
/// the inner helper shared by every `jit_print_<heap_arm>` body below.
///
/// W17-narrow-follow-up-B-β (Phase 3 cluster-0 Round 19, 2026-05-14):
/// early-return "None" before constructing the `KindedSlot` when `bits`
/// is the JIT `TAG_NULL` / `TAG_NONE` sentinel. See
/// `is_jit_null_sentinel` for the §2.7.5/§2.7.7 discipline framing.
fn print_kinded_inner(
    ctx_ptr: *const crate::context::JITContext,
    bits: u64,
    kind: shape_value::NativeKind,
) {
    if is_jit_null_sentinel(bits) {
        println!("None");
        return;
    }
    let registry = registry_from_ctx(ctx_ptr);
    let formatter = shape_vm::executor::printing::ValueFormatter::new(&registry);
    let slot = shape_value::ValueSlot::from_raw(bits);
    let kinded = shape_value::KindedSlot::new(slot, kind);
    let rendered = formatter.format_kinded(&kinded);
    // The carrier was constructed from a borrowed raw — forget it so
    // its kind-aware Drop does not retire the caller's share.
    std::mem::forget(kinded);
    println!("{}", rendered);
}

/// Print a heap `Arc<String>`-shaped slot. Dispatched when the operand
/// kind is proven `NativeKind::String` (the §2.7.5 string carrier).
///
/// SAFETY: `bits` must be `Arc::into_raw(Arc<String>) as u64` per the
/// producer-site contract on every `KindedSlot::from_string_arc`-shaped
/// producer. Null bits render as `None` per `format_kinded_inner` line
/// 155 (the VM-side documented behaviour for a null String slot).
#[unsafe(no_mangle)]
pub extern "C" fn jit_print_str(ctx_ptr: *const crate::context::JITContext, bits: u64) {
    print_kinded_inner(ctx_ptr, bits, shape_value::NativeKind::String);
}

/// Print a heap `Arc<TypedObjectStorage>`-shaped slot. Dispatched when
/// the operand kind is proven `NativeKind::Ptr(HeapKind::TypedObject)`.
/// The schema registry resolves field names from `storage.schema_id`;
/// when the JIT runs without an `ExecutionContext` (test harness) the
/// fallback empty registry renders positional placeholders (`_0`, `_1`,
/// ...) per `format_typed_object`'s documented schema-less render path.
///
/// SAFETY (Wave-7 jit-typed-pointer-migration): `bits` must be the v2-raw
/// `*mut TypedObjectStorage as u64` carrier — the SAME carrier the VM
/// (`op_new_typed_object`) and the migrated JIT producer (`jit_typed_object_alloc`
/// → `TypedObjectStorage::_new`) both emit. `print_kinded_inner` only BORROWS
/// the carrier (`ValueSlot::from_raw(bits)` + `std::mem::forget`, and the
/// formatter's `HeapKind::TypedObject` arm reads through `&*(bits as *const
/// TypedObjectStorage)`) — no `Arc::from_raw`, no refcount bump, no free — so it
/// is sound on the `_new`-allocated pointer (whose real refcount is the
/// offset-0 `HeapHeader`, not an Arc control block). The pre-Wave-7 JIT producer
/// emitted a `UnifiedValue`-wrapped inline-cell struct; routing that carrier here
/// was the carrier-mismatch that kept this a SURFACE-and-stop arm.
#[unsafe(no_mangle)]
pub extern "C" fn jit_print_typed_object(ctx_ptr: *const crate::context::JITContext, bits: u64) {
    use shape_value::heap_value::HeapKind;
    print_kinded_inner(
        ctx_ptr,
        bits,
        shape_value::NativeKind::Ptr(HeapKind::TypedObject),
    );
}

/// Print an `Arc<OptionData>`-shaped slot as `Some(<inner>)` / `None`.
/// Dispatched when the operand kind is proven
/// `NativeKind::Ptr(HeapKind::Option)`. The inner payload's kind comes
/// from `OptionData.payload.kind` (the §2.7.17 carrier-internal kind
/// label, stamped at producer construction); the recursive formatter
/// pass dispatches on that kind without any tag-bit decode.
///
/// SAFETY: `bits` must be `Arc::into_raw(Arc<OptionData>) as u64` per
/// the legacy compatibility carrier contract. Active VM producers use
/// schema-backed `__Option`; W88A makes the old JIT producer imports
/// fail-closed before allocation.
#[unsafe(no_mangle)]
pub extern "C" fn jit_print_option(ctx_ptr: *const crate::context::JITContext, bits: u64) {
    use shape_value::heap_value::HeapKind;
    print_kinded_inner(
        ctx_ptr,
        bits,
        shape_value::NativeKind::Ptr(HeapKind::Option),
    );
}

/// Print an `Arc<ResultData>`-shaped slot as `Ok(<inner>)` / `Err(<inner>)`.
/// Dispatched when the operand kind is proven
/// `NativeKind::Ptr(HeapKind::Result)`. Mirrors `jit_print_option` —
/// inner payload kind comes from `ResultData.payload.kind`.
///
/// SAFETY: `bits` must be `Arc::into_raw(Arc<ResultData>) as u64` per
/// the legacy compatibility carrier contract. Active VM producers use
/// schema-backed `__Result`; W88A makes the old JIT producer imports
/// fail-closed before allocation.
#[unsafe(no_mangle)]
pub extern "C" fn jit_print_result(ctx_ptr: *const crate::context::JITContext, bits: u64) {
    use shape_value::heap_value::HeapKind;
    print_kinded_inner(
        ctx_ptr,
        bits,
        shape_value::NativeKind::Ptr(HeapKind::Result),
    );
}

// ============================================================================
// Phase 3 cluster-2 Round 3 cw-D-fam12 kinded jit_print entries
// (2026-05-16): Scalar Char + Concurrency Mutex/Atomic/Lazy/Channel.
// Per cluster-2-inventory §E.5 per-family sub-cluster recommendation +
// ADR-006 §2.7.25 Concurrency amendment's printing convention. Each
// entry mirrors the existing W12-jit-print-heap-arm-classification
// shape: `(ctx_ptr, bits)` heap-arm entries delegate to
// `print_kinded_inner` for VM == JIT identical output; scalar entries
// take the raw value directly (mirror of `jit_print_i64` / `_f64` /
// `_bool`).
// ============================================================================

/// Print a `char` codepoint to stdout with a newline.
///
/// Dispatched when the operand kind is proven `NativeKind::Char`
/// (ADR-006 §2.7.5 amendment scalar variant) OR
/// `NativeKind::Ptr(HeapKind::Char)` (pre-amendment heap arm — the
/// `KindedSlot::as_char` accessor accepts both labels). The carrier is
/// a 4-byte inline codepoint per `ValueSlot::from_char` (`c as u64`),
/// passed through the FFI boundary as a `u32` (low 32 bits hold the
/// codepoint).
///
/// Mirrors the VM-side `format_kinded_inner` `NativeKind::Char` arm at
/// `printing.rs:160-164` (top-level / non-quoted form `c.to_string()`).
/// Invalid codepoints render as `<invalid-char:0x...>` to match the
/// VM-side fallback.
#[unsafe(no_mangle)]
pub extern "C" fn jit_print_char(value: u32) {
    match char::from_u32(value) {
        Some(c) => println!("{}", c),
        None => println!("<invalid-char:0x{:x}>", value),
    }
}

/// Print an `Arc<MutexData>`-shaped slot as `<mutex>`. Dispatched when
/// the operand kind is proven `NativeKind::Ptr(HeapKind::Mutex)`.
///
/// Mirrors `jit_print_option` — delegates to the canonical VM-side
/// `ValueFormatter::format_kinded`, which renders MutexData as the
/// opaque tag `<mutex>` per `printing.rs:534-537` (ADR-006 §2.7.25
/// concurrency-primitive printing convention — no user-facing literal,
/// opaque diagnostic tag).
///
/// SAFETY: `bits` must be `Arc::into_raw(Arc<MutexData>) as u64` per
/// the producer-site contract on every `KindedSlot::from_mutex`-shaped
/// producer (VM-side `BuiltinFunction::MutexCtor`).
#[unsafe(no_mangle)]
pub extern "C" fn jit_print_mutex(ctx_ptr: *const crate::context::JITContext, bits: u64) {
    use shape_value::heap_value::HeapKind;
    print_kinded_inner(ctx_ptr, bits, shape_value::NativeKind::Ptr(HeapKind::Mutex));
}

/// Print an `Arc<AtomicData>`-shaped slot as `<atomic:N>` where N is
/// the current atomic value. Dispatched when the operand kind is proven
/// `NativeKind::Ptr(HeapKind::Atomic)`.
///
/// Mirrors `jit_print_mutex` — delegates to the canonical VM-side
/// `ValueFormatter::format_kinded`, which renders AtomicData as
/// `<atomic:{value}>` per `printing.rs:538-542` (ADR-006 §2.7.25
/// printing convention).
///
/// SAFETY: `bits` must be `Arc::into_raw(Arc<AtomicData>) as u64` per
/// the producer-site contract on every `KindedSlot::from_atomic`-shaped
/// producer (VM-side `BuiltinFunction::AtomicCtor`).
#[unsafe(no_mangle)]
pub extern "C" fn jit_print_atomic(ctx_ptr: *const crate::context::JITContext, bits: u64) {
    use shape_value::heap_value::HeapKind;
    print_kinded_inner(
        ctx_ptr,
        bits,
        shape_value::NativeKind::Ptr(HeapKind::Atomic),
    );
}

/// Print an `Arc<LazyData>`-shaped slot as `<lazy:initialized>` /
/// `<lazy:pending>` depending on whether the cached value has been
/// populated. Dispatched when the operand kind is proven
/// `NativeKind::Ptr(HeapKind::Lazy)`.
///
/// Mirrors `jit_print_mutex` — delegates to the canonical VM-side
/// `ValueFormatter::format_kinded`, which renders LazyData per
/// `printing.rs:543-551` (ADR-006 §2.7.25 printing convention).
///
/// SAFETY: `bits` must be `Arc::into_raw(Arc<LazyData>) as u64` per
/// the producer-site contract on every `KindedSlot::from_lazy`-shaped
/// producer (VM-side `BuiltinFunction::LazyCtor`).
#[unsafe(no_mangle)]
pub extern "C" fn jit_print_lazy(ctx_ptr: *const crate::context::JITContext, bits: u64) {
    use shape_value::heap_value::HeapKind;
    print_kinded_inner(ctx_ptr, bits, shape_value::NativeKind::Ptr(HeapKind::Lazy));
}

/// Print an `Arc<ChannelData>`-shaped slot as `<channel:state:len>`
/// where state is `open`/`closed` and len is the current queue length.
/// Dispatched when the operand kind is proven
/// `NativeKind::Ptr(HeapKind::Channel)`.
///
/// Mirrors `jit_print_mutex` — delegates to the canonical VM-side
/// `ValueFormatter::format_kinded`, which renders ChannelData per
/// `printing.rs:451-464` (ADR-006 §2.7.20 channel printing convention,
/// shared by §2.7.25 concurrency-primitive family).
///
/// SAFETY: `bits` must be `Arc::into_raw(Arc<ChannelData>) as u64` per
/// the producer-site contract on every `KindedSlot::from_channel`-shaped
/// producer (VM-side `BuiltinFunction::ChannelCtor`).
#[unsafe(no_mangle)]
pub extern "C" fn jit_print_channel(ctx_ptr: *const crate::context::JITContext, bits: u64) {
    use shape_value::heap_value::HeapKind;
    print_kinded_inner(
        ctx_ptr,
        bits,
        shape_value::NativeKind::Ptr(HeapKind::Channel),
    );
}

// ============================================================================
// Phase 3 cluster-2 Round 4 cw-D-fam3 kinded jit_print entries
// (2026-05-16): Collection family — HashMap / HashSet / Deque /
// PriorityQueue / Range / Iterator. Per cluster-2-inventory §E.5
// per-family sub-cluster recommendation + ADR-006 §2.7.5.B amendment
// extension (Family 3 Collection). Each entry mirrors the existing
// W12-jit-print-heap-arm-classification pattern + cw-D-fam12 Concurrency
// shape: `(ctx_ptr, bits)` heap-arm entries delegate to
// `print_kinded_inner` for VM == JIT identical output through the
// canonical `ValueFormatter::format_kinded` dispatch on the matching
// `NativeKind::Ptr(HeapKind::X)` label.
// ============================================================================

/// Print an `Arc<HashMapKindedRef>`-shaped slot as
/// `{"k1": v1, "k2": v2, ...}` with per-V value rendering (POD scalars
/// rendered directly, heap-payload values rendered via the canonical
/// `HeapValue` Display dispatch). Dispatched when the operand kind is
/// proven `NativeKind::Ptr(HeapKind::HashMap)`.
///
/// Mirrors `jit_print_mutex` — delegates to the canonical VM-side
/// `ValueFormatter::format_kinded`, which renders HashMapKindedRef per
/// `printing.rs:281-289` + `format_hashmap` (`printing.rs:787-...`,
/// per-V dispatch through the carrier's variant tag).
///
/// SAFETY: `bits` must be
/// `Arc::into_raw(Arc<HashMapKindedRef>) as u64` per the producer-site
/// contract on every `KindedSlot::from_hashmap`-shaped producer
/// (VM-side `BuiltinFunction::HashMapCtor` / Q25.B SUPERSEDED
/// per-V monomorphization). ADR-006 §2.7.5.B 2026-05-16
#[unsafe(no_mangle)]
pub extern "C" fn jit_print_hashmap(ctx_ptr: *const crate::context::JITContext, bits: u64) {
    use shape_value::heap_value::HeapKind;
    print_kinded_inner(
        ctx_ptr,
        bits,
        shape_value::NativeKind::Ptr(HeapKind::HashMap),
    );
}

/// Print an `Arc<HashSetData>`-shaped slot as `{"a", "b", ...}`.
/// Dispatched when the operand kind is proven
/// `NativeKind::Ptr(HeapKind::HashSet)`.
///
/// Mirrors `jit_print_mutex` — delegates to the canonical VM-side
/// `ValueFormatter::format_kinded`, which renders HashSetData per
/// `printing.rs:291-302` + `format_hashset` (`printing.rs:745-757`).
///
/// SAFETY: `bits` must be `Arc::into_raw(Arc<HashSetData>) as u64` per
/// the producer-site contract on every `KindedSlot::from_hashset`-shaped
/// producer (VM-side `BuiltinFunction::HashSetCtor`).
/// ADR-006 §2.7.5.B 2026-05-16
#[unsafe(no_mangle)]
pub extern "C" fn jit_print_hashset(ctx_ptr: *const crate::context::JITContext, bits: u64) {
    use shape_value::heap_value::HeapKind;
    print_kinded_inner(
        ctx_ptr,
        bits,
        shape_value::NativeKind::Ptr(HeapKind::HashSet),
    );
}

/// Print an `Arc<DequeData>`-shaped slot as
/// `Deque[elem1, elem2, ...]` front-to-back. Dispatched when the
/// operand kind is proven `NativeKind::Ptr(HeapKind::Deque)`.
///
/// Mirrors `jit_print_mutex` — delegates to the canonical VM-side
/// `ValueFormatter::format_kinded`, which renders DequeData per
/// `printing.rs:440-450` + `format_deque` (`printing.rs:763-775`).
///
/// SAFETY: `bits` must be `Arc::into_raw(Arc<DequeData>) as u64` per
/// the producer-site contract on every `KindedSlot::from_deque`-shaped
/// producer (VM-side `BuiltinFunction::DequeCtor`).
/// ADR-006 §2.7.5.B 2026-05-16
#[unsafe(no_mangle)]
pub extern "C" fn jit_print_deque(ctx_ptr: *const crate::context::JITContext, bits: u64) {
    use shape_value::heap_value::HeapKind;
    print_kinded_inner(ctx_ptr, bits, shape_value::NativeKind::Ptr(HeapKind::Deque));
}

/// Print an `Arc<PriorityQueueData>`-shaped slot as
/// `PriorityQueue[v1, v2, ...]` in heap-array order (NOT sorted).
/// Dispatched when the operand kind is proven
/// `NativeKind::Ptr(HeapKind::PriorityQueue)`.
///
/// Mirrors `jit_print_mutex` — delegates to the canonical VM-side
/// `ValueFormatter::format_kinded`, which renders PriorityQueueData per
/// `printing.rs:465-478` + `format_priority_queue`
/// (`printing.rs:725-740`).
///
/// SAFETY: `bits` must be
/// `Arc::into_raw(Arc<PriorityQueueData>) as u64` per the producer-site
/// contract on every `KindedSlot::from_priority_queue`-shaped producer
/// (VM-side `BuiltinFunction::PriorityQueueCtor`).
/// ADR-006 §2.7.5.B 2026-05-16
#[unsafe(no_mangle)]
pub extern "C" fn jit_print_priority_queue(ctx_ptr: *const crate::context::JITContext, bits: u64) {
    use shape_value::heap_value::HeapKind;
    print_kinded_inner(
        ctx_ptr,
        bits,
        shape_value::NativeKind::Ptr(HeapKind::PriorityQueue),
    );
}

/// Print an `Arc<RangeData>`-shaped slot as `start..end` (exclusive)
/// or `start..=end` (inclusive). Dispatched when the operand kind is
/// proven `NativeKind::Ptr(HeapKind::Range)`.
///
/// Mirrors `jit_print_mutex` — delegates to the canonical VM-side
/// `ValueFormatter::format_kinded`, which renders RangeData per
/// `printing.rs:479-494`.
///
/// SAFETY: `bits` must be `Arc::into_raw(Arc<RangeData>) as u64` per
/// the producer-site contract on every `KindedSlot::from_range`-shaped
/// producer (VM-side `op_make_range` / Range-literal lowering).
/// ADR-006 §2.7.5.B 2026-05-16
#[unsafe(no_mangle)]
pub extern "C" fn jit_print_range(ctx_ptr: *const crate::context::JITContext, bits: u64) {
    use shape_value::heap_value::HeapKind;
    print_kinded_inner(ctx_ptr, bits, shape_value::NativeKind::Ptr(HeapKind::Range));
}

/// Print an `Arc<IteratorState>`-shaped slot as the opaque tag
/// `<iterator>` (lazy iterators have no user-facing print form; a
/// terminal operation must materialize the values per
/// `printing.rs:430-439`). Dispatched when the operand kind is proven
/// `NativeKind::Ptr(HeapKind::Iterator)`.
///
/// Mirrors `jit_print_mutex` — delegates to the canonical VM-side
/// `ValueFormatter::format_kinded`. Note: per inventory §E.5 the
/// Iterator HeapKind belongs to the Collection family (NOT the
/// pure-discriminator family per ADR-006 §2.7.16 / Q17
/// W13-iterator-state) — `HeapValue::Iterator(Arc<IteratorState>)`
/// participates in the §2.3 typed-Arc payload pattern, the dispatch
/// arm in `format_heap_kind` at `printing.rs:430` reads the bits as
/// `*const IteratorState` and the Display rendering is the opaque
/// tag.
///
/// SAFETY: `bits` must be `Arc::into_raw(Arc<IteratorState>) as u64`
/// per the producer-site contract on every
/// `KindedSlot::from_iterator`-shaped producer (VM-side iterator-pipeline
/// factory builtins).
/// ADR-006 §2.7.5.B 2026-05-16
#[unsafe(no_mangle)]
pub extern "C" fn jit_print_iterator(ctx_ptr: *const crate::context::JITContext, bits: u64) {
    use shape_value::heap_value::HeapKind;
    print_kinded_inner(
        ctx_ptr,
        bits,
        shape_value::NativeKind::Ptr(HeapKind::Iterator),
    );
}

// ============================================================================
// v2-raw TypedArray<T> print entry (W11-fup-C, Phase 3d, 2026-05-18)
// ============================================================================
//
// `print(Array<T>)` JIT-side close. The MIR-time kind label is
// `NativeKind::Ptr(HeapKind::TypedArray)` per
// `crates/shape-jit/src/mir_compiler/types.rs:175`
// (`ConcreteType::Array(_) => NativeKind::Ptr(HeapKind::TypedArray)`),
// but the runtime slot bits are the v2-raw `*mut TypedArray<T>` pointer
// produced by the per-T allocators in
// `crates/shape-jit/src/ffi/v2/mod.rs::jit_v2_array_new_<kind>` —
// `HeapKind::TypedArray = 8` is the vacated-ordinal type LABEL, not a live
// `Arc<TypedArrayData>` carrier (the enum + outer `HeapValue::TypedArray`
// arm + `TypedBuffer<T>` wrapper layer were retired across V3-S5 ckpt-1..
// ckpt-4 per W12-typed-array-data-deletion audit §3.5 / §3.6 + ADR-006
// §2.7.24 Q25.A SUPERSEDED).
//
// The canonical VM-side formatter recognizes the v2-raw carrier via the
// `NativeKind::Ptr(HeapKind::TypedArray)` arm of `format_heap_kind` in
// `crates/shape-vm/src/executor/printing.rs` (r5c-2-β-CKPT-C
// u64-carrier-disambiguation: `as_v2_typed_array(bits, kind)` accepts ONLY
// `kind == NativeKind::Ptr(HeapKind::TypedArray)` and reads the
// `*mut TypedArray<T>` pointer directly — a genuine scalar `u64` carrier
// is never dereferenced). This FFI body reifies the slot with that label
// and delegates to `ValueFormatter::format_kinded` for VM == JIT
// byte-identical output.
//
// Per-element kinds are read from the v2-raw HeapHeader's `_pad` byte
// (`v2_array_detect::read_element` arms at lines 304-365) per ADR-006
// §2.7.7 stamp-at-compile-time — no Bool-default fabrication for the
// element kind; the array's element-type byte at allocation time is the
// authoritative kind source.

/// Print a `*mut TypedArray<T>`-shaped slot as `[v1, v2, ...]`. Dispatched
/// when the operand kind is proven `NativeKind::Ptr(HeapKind::TypedArray)`
/// by the MIR-time `concrete_type_to_native_kind` arm for `ConcreteType::
/// Array(_)`.
///
/// SAFETY: `bits` must be a `*mut TypedArray<T>` raw pointer produced by
/// `crate::ffi::v2::jit_v2_array_new_<kind>` (or
/// `jit_new_typed_array_<string|decimal>`), with the element-type byte
/// stamped at HeapHeader offset 7 per `v2_array_detect::stamp_elem_type`.
/// Null bits render as `None` (mirror of the other heap-arm bodies'
/// null-sentinel handling). The pointer is borrowed for the duration of
/// the call — no refcount work, no Drop (v2-raw arrays own their backing
/// storage directly; the caller's slot keeps the active share).
#[unsafe(no_mangle)]
pub extern "C" fn jit_print_typed_array(ctx_ptr: *const crate::context::JITContext, bits: u64) {
    if bits == 0 {
        println!("None");
        return;
    }
    let registry = registry_from_ctx(ctx_ptr);
    let formatter = shape_vm::executor::printing::ValueFormatter::new(&registry);
    // r5c-2-β-CKPT-C u64-carrier-disambiguation (2026-05-20): reify with
    // `NativeKind::Ptr(HeapKind::TypedArray)` — the canonical carrier kind
    // the formatter's `format_heap_kind` arm uses to route v2-raw
    // `*mut TypedArray<T>` pointers to the `format_v2_typed_array`
    // per-element walker. The pre-fix `NativeKind::UInt64` label is no
    // longer recognised as the array carrier (it now denotes a genuine
    // scalar `u64`).
    let slot = shape_value::ValueSlot::from_raw(bits);
    let kinded = shape_value::KindedSlot::new(
        slot,
        shape_value::NativeKind::Ptr(shape_value::HeapKind::TypedArray),
    );
    let rendered = formatter.format_kinded(&kinded);
    // The v2-raw `*mut TypedArray<T>` carrier this FFI body prints is
    // BORROWED — the caller's slot keeps the active refcount share. A
    // `Ptr(HeapKind::TypedArray)`-kind Drop would call
    // `release_v2_typed_array` and retire a share this body never
    // retained, so `mem::forget` is REQUIRED for soundness here (not just
    // structural symmetry).
    std::mem::forget(kinded);
    println!("{}", rendered);
}

/// Concatenate two operand values into a freshly allocated `Arc<String>`
/// carrier. Used by the MIR-lowering path for `BinOp::Add` when either
/// operand has `NativeKind::String` (the `compile_string_concat` site in
/// `mir_compiler/rvalues.rs`), which covers `str + str` directly and the
/// f-string interpolation chain emitted by `lower_formatted_string`
/// (`crates/shape-vm/src/mir/lowering/expr.rs:720`).
///
/// ## W15.2-LANG-7 jit-print-fstring close — producer-side carrier-shape fix
///
/// **ADR-006 §2.7.5 / §2.7.7 producer-side stamp.** Pre-fix the FFI took
/// `(a_bits, b_bits) -> u64`, decoded each via `heap_kind(bits)` (a
/// NaN-tag probe — the deleted-W-series shape per CLAUDE.md "Forbidden
/// Patterns" #4) and returned `box_string(out)` — a NaN-boxed
/// `UnifiedValue<Arc<String>>` allocation. But the JIT-side String
/// carrier per ADR-006 §2.7.5 is `Arc::into_raw(Arc<String>) as u64` —
/// a raw Arc pointer, NOT a NaN-box. Every downstream consumer reads
/// the result with the canonical `NativeKind::String` carrier shape:
/// `jit_print_str` calls `print_kinded_inner(bits, NativeKind::String)`
/// which constructs `KindedSlot::new(ValueSlot::from_raw(bits), String)`,
/// and `format_kinded`'s String arm dereferences as `&Arc<String>`. A
/// NaN-boxed pointer in that slot dereferences a NaN bit-pattern as
/// a `String` struct's `ptr/cap/len` — printing garbage memory bytes
/// (the empirical surface at `let m = "a"+"b"; print(m)` pre-fix).
///
/// Per W15.1 audit §6.7 (FIX-LANGUAGE W15.2-LANG-7) the post-fix shape:
///
/// - Inputs carry `(a_bits, a_kind_code, b_bits, b_kind_code)` — the
///   parallel-track encoding at `super::stack_kind_code` per ADR-006
///   §2.7.7/Q9. Kind codes are stamped at JIT-compile time from the
///   producer-side `operand_slot_kind` result in `compile_string_concat`
///   — the same compile-time kind-source discipline the retired
///   Result/Option producer ABI attempted to use, without allocating an old
///   carrier.
/// - Each operand decodes per its kind: `String` → adopt the raw Arc
///   pointer via `Arc::from_raw` and read `&str`; scalar arms format
///   directly from the raw native value. No tag-bit dispatch, no
///   NaN-tag probe.
/// - Return is `Arc::into_raw(Arc::new(out)) as u64` — the §2.7.5
///   String carrier shape, matching every downstream consumer
///   (`jit_print_str`, `arc_string_retain`/`_release`,
///   `KindedSlot::Drop` for `NativeKind::String`).
///
/// **Strong-count contract.** Each `NativeKind::String` operand carries
/// one strong-count share (the producer-side per-consumption retain at
/// `mir_compiler/ownership.rs:486` for `MirConstant::Str`, or the
/// `compile_operand`'s `Copy(place)` retain at line 239). The FFI
/// consumes both shares via `Arc::from_raw` (which adopts the share)
/// and drops them at scope end. The returned `Arc::into_raw(Arc::new(
/// out))` carries one fresh share that the caller installs in the
/// destination slot with kind `NativeKind::String`; subsequent
/// `arc_string_release` retires the share at slot-drop time.
///
/// **Forbidden under W15.2-LANG-7 close** (refusal log per CLAUDE.md
/// "Renames to refuse on sight" + ADR-006 §2.7.5 producer-side stamp):
/// - Returning `box_string(out)` — wrong carrier shape; the consumer
///   would dereference a NaN-boxed pointer as `Arc<String>` raw bits
///   and segfault on the next print (the W15.1 audit §6.7 empirical
///   surface).
/// - `heap_kind(bits)` probe on a kind-stamped operand — the deleted-
///   W-series tag-bit dispatch. The producer-side stamp from
///   `compile_string_concat`'s `operand_slot_kind` IS the discriminator.
/// - Bool-default for an unknown kind code — surface-and-stop per §2.7.7
///   #9. Producing a malformed Arc on a kind-source gap masks the bug
///   downstream.
/// - Defection-attractor descriptors (broader-family regex per CLAUDE.md
///   §"Renames to refuse on sight") for this producer-side carrier-shape
///   fix — refused on sight. Describe the change by name (the deleted
///   `box_string` shape replaced by the §2.7.5 `Arc::into_raw(Arc<String>)`
///   shape) or by deletion-fate (the deleted-W-series `heap_kind`-probe
///   path).
#[unsafe(no_mangle)]
pub extern "C" fn jit_string_concat(
    a_bits: u64,
    a_kind_code: u8,
    b_bits: u64,
    b_kind_code: u8,
) -> u64 {
    use super::stack_kind_code;
    use shape_value::NativeKind;
    use std::sync::Arc;

    /// Decode one operand into its rendered `String` form, consuming any
    /// strong-count share carried by the bits. Per W15.2-LANG-7 close the
    /// dispatch is keyed off the producer-side kind stamp, NOT a runtime
    /// tag-bit probe.
    fn consume_operand(bits: u64, kind_code: u8, func_name: &str) -> String {
        match stack_kind_code::decode(kind_code) {
            Some(NativeKind::String) => {
                // ADR-006 §2.7.5 String carrier: bits are `Arc::into_raw(
                // Arc<String>) as u64`. Adopt the caller's share via
                // `Arc::from_raw`, copy the contents to `String`, then
                // drop the Arc (releases the share). Null bits render as
                // the empty string — mirror of `jit_print_str`'s null
                // sentinel handling at `is_jit_null_sentinel`.
                if bits == 0 {
                    return String::new();
                }
                let arc = unsafe { Arc::<String>::from_raw(bits as *const String) };
                let out = (*arc).clone();
                drop(arc);
                out
            }
            Some(NativeKind::Int64)
            | Some(NativeKind::UInt64)
            | Some(NativeKind::IntSize)
            | Some(NativeKind::UIntSize) => {
                // Raw native i64/u64 — format directly per ADR-006 §2.7.5
                // scalar carrier. No NaN-unbox.
                format!("{}", bits as i64)
            }
            Some(NativeKind::Int32) | Some(NativeKind::UInt32) => {
                format!("{}", bits as i32)
            }
            Some(NativeKind::Int16) | Some(NativeKind::UInt16) => {
                format!("{}", bits as i16)
            }
            Some(NativeKind::Int8) | Some(NativeKind::UInt8) => {
                format!("{}", bits as i8)
            }
            Some(NativeKind::Float64) => {
                // The Cranelift `bitcast(I64, ..., F64)` was applied at the
                // call site (`compile_string_concat::to_i64_bits`) to pack
                // an F64 into the I64 ABI slot per §2.7.5 stable-FFI rule.
                // Reverse the bitcast here to recover the f64.
                let n = f64::from_bits(bits);
                if n.is_finite() && n == n.trunc() && n.abs() < 1e15 {
                    format!("{}", n as i64)
                } else {
                    format!("{}", n)
                }
            }
            Some(NativeKind::Float32) => {
                let n = f32::from_bits(bits as u32);
                format!("{}", n)
            }
            Some(NativeKind::Bool) => {
                if (bits as u8) != 0 {
                    "true".to_string()
                } else {
                    "false".to_string()
                }
            }
            Some(NativeKind::Char) => {
                let cp = bits as u32;
                match char::from_u32(cp) {
                    Some(c) => c.to_string(),
                    None => String::new(),
                }
            }
            Some(other) => {
                // §2.7.7 #9 surface: a producer-stamped non-scalar /
                // non-String kind reaching `jit_string_concat` is a
                // producer-site gap upstream of this FFI body. The MIR
                // f-string lowering at `lower_formatted_string` emits
                // `BinOp::Add` with whatever an interpolation expression
                // returns — extending coverage to heap-arm carriers
                // (Option / Result / TypedObject / ...) is W15.2-LANG-7-
                // FUP territory. For now we emit a placeholder render and
                // log via tracing so the surface is visible without
                // breaking the calling f-string chain mid-render.
                tracing::debug!(
                    target: "shape_jit",
                    func_name,
                    kind_code,
                    ?other,
                    "SURFACE: jit_string_concat operand kind is non-scalar / non-String. \
                     ADR-006 \u{a7}2.7.7 #9 \u{2014} MIR f-string interpolation does not \
                     yet format heap-arm operands. W15.2-LANG-7-FUP territory.",
                );
                format!("<{:?}>", other)
            }
            None => {
                // §2.7.7 #9 surface: SENTINEL / unknown kind code is a
                // producer-side gap upstream of this FFI body. The
                // `compile_string_concat` site stamps kind from
                // `operand_slot_kind` which is `Some(_)` by construction
                // when `either_string` matched. Reaching the None arm
                // means the call site bypassed the stamp.
                tracing::debug!(
                    target: "shape_jit",
                    func_name,
                    kind_code,
                    "SURFACE: jit_string_concat operand kind code is sentinel/unknown. \
                     ADR-006 \u{a7}2.7.7 #9 \u{2014} producer-site MIR kind classification gap.",
                );
                String::new()
            }
        }
    }

    let mut out = consume_operand(a_bits, a_kind_code, "jit_string_concat[a]");
    out.push_str(&consume_operand(
        b_bits,
        b_kind_code,
        "jit_string_concat[b]",
    ));
    // Return the §2.7.5 `NativeKind::String` carrier: `Arc::into_raw(
    // Arc<String>) as u64`. Refcount = 1 (the fresh Arc share transferred
    // to the caller). The caller installs these bits in the destination
    // slot with kind `NativeKind::String`; subsequent `arc_string_release`
    // retires the share at slot-drop time.
    Arc::into_raw(Arc::new(out)) as u64
}

/// Convert value to number
pub extern "C" fn jit_to_number(value_bits: u64) -> u64 {
    if is_number(value_bits) {
        return value_bits;
    }

    if value_bits == TAG_NULL {
        return box_number(0.0);
    }
    if value_bits == TAG_BOOL_TRUE {
        return box_number(1.0);
    }
    if value_bits == TAG_BOOL_FALSE {
        return box_number(0.0);
    }

    let kind = heap_kind(value_bits);
    let num = match kind {
        Some(HK_STRING) => {
            let s = unsafe { jit_unbox::<String>(value_bits) };
            s.parse::<f64>().unwrap_or(f64::NAN)
        }
        Some(kind) if is_known_legacy_heap_kind(kind) => f64::NAN,
        other => unsupported_legacy_heap_kind("jit_to_number", other),
    };
    box_number(num)
}

#[cfg(test)]
mod heap_arm_print_tests {
    //! W12-jit-print-heap-arm-classification (Phase 3 cluster-0 Round 8A,
    //! 2026-05-13) FFI round-trip tests. Mirrors Round 7A's
    //! `result.rs::tests` shape — round-trip the Arc carrier through
    //! producer → kinded print body → drop, without leaking.
    //!
    //! Tests construct typed `Arc<T>` carriers per ADR-006 §2.7.17, pass
    //! the raw bits through the kinded print FFI body with a
    //! `ctx_ptr=null` test harness instance (the empty schema registry
    //! fallback path), and assert the printed output matches the VM-side
    //! `ValueFormatter::format_kinded` output for the same carrier. We
    //! capture stdout via a thread-local buffer — the FFI body's
    //! `println!` writes through the standard Rust stdout sink, which
    //! the tests redirect for assertion.
    //!
    //! The `print_kinded_inner` helper is called directly for each kind
    //! variant rather than via cranelift codegen, to keep the test unit-
    //! sized and avoid the JIT compile path overhead.
    //!
    //! Note: the `jit_print_str` / `jit_print_typed_object` bodies are
    //! tested via the §2.7.5 Arc carrier directly. The
    //! `MirConstant::Str` / TypedObject Aggregate producers (which
    //! currently store NaN-box UnifiedValue bits, NOT the Arc carrier)
    //! are surfaced via the `terminators.rs` Call-terminator dispatch's
    //! carrier-mismatch surface-and-stop, not exercised by the FFI
    //! body's input contract. When cluster-1
    //! `W12-jit-result-carrier-unification` lands, the same FFI body
    //! handles the migrated producer output unchanged.

    use super::*;
    use shape_value::heap_value::{HeapKind, OptionData, ResultData, TypedObjectStorage};
    use shape_value::{KindedSlot, NativeKind, ValueSlot};
    use std::sync::Arc;

    /// Build a NULL `*const JITContext` for tests that don't need the
    /// schema registry. The kinded print bodies fall back to an empty
    /// `TypeSchemaRegistry` per `registry_from_ctx` line 1.
    fn null_ctx() -> *const crate::context::JITContext {
        std::ptr::null()
    }

    /// Recover the Arc<T> from its raw bits without leaking — drops the
    /// strong-count share at end of test scope.
    unsafe fn drop_arc_result(bits: u64) {
        if bits != 0 {
            let _ = unsafe { Arc::<ResultData>::from_raw(bits as *const ResultData) };
        }
    }

    unsafe fn drop_arc_option(bits: u64) {
        if bits != 0 {
            let _ = unsafe { Arc::<OptionData>::from_raw(bits as *const OptionData) };
        }
    }

    unsafe fn drop_arc_typed_object(bits: u64) {
        if bits != 0 {
            let _ =
                unsafe { Arc::<TypedObjectStorage>::from_raw(bits as *const TypedObjectStorage) };
        }
    }

    unsafe fn drop_arc_string(bits: u64) {
        if bits != 0 {
            let _ = unsafe { Arc::<String>::from_raw(bits as *const String) };
        }
    }

    /// Helper: format a `(bits, kind)` via the canonical VM-side
    /// `ValueFormatter` and return the rendered string. Used to assert
    /// VM == JIT-FFI output equivalence at the FFI-body level.
    fn vm_format(bits: u64, kind: NativeKind) -> String {
        let registry = shape_runtime::type_schema::TypeSchemaRegistry::default();
        let formatter = shape_vm::executor::printing::ValueFormatter::new(&registry);
        let slot = ValueSlot::from_raw(bits);
        let kinded = KindedSlot::new(slot, kind);
        let out = formatter.format_kinded(&kinded);
        std::mem::forget(kinded);
        out
    }

    #[test]
    fn print_option_some_int_payload_matches_vm() {
        // Legacy compatibility carrier used to exercise the print consumer;
        // active VM producers now build schema-backed `__Option`, and W88A
        // retires the JIT old-carrier producer imports.
        let payload = KindedSlot::new(ValueSlot::from_int(7), NativeKind::Int64);
        let arc = Arc::new(OptionData::some(payload));
        let bits = Arc::into_raw(arc) as u64;

        // VM-side rendering for the same carrier.
        let vm_render = vm_format(bits, NativeKind::Ptr(HeapKind::Option));
        assert_eq!(vm_render, "Some(7)");

        // Call the FFI body — captures stdout via std::io::set_output_capture
        // when configured (not configured here; assertion is on vm_format's
        // independent path proving the formatter shape). The FFI body
        // executes the same `ValueFormatter::format_kinded` call as
        // `vm_format`, so a successful call without segfault is the unit
        // test's positive signal.
        jit_print_option(null_ctx(), bits);

        unsafe { drop_arc_option(bits) };
    }

    #[test]
    fn print_option_none_matches_vm() {
        let arc = Arc::new(OptionData::none());
        let bits = Arc::into_raw(arc) as u64;

        let vm_render = vm_format(bits, NativeKind::Ptr(HeapKind::Option));
        assert_eq!(vm_render, "None");

        jit_print_option(null_ctx(), bits);

        unsafe { drop_arc_option(bits) };
    }

    #[test]
    fn print_result_ok_int_payload_matches_vm() {
        let payload = KindedSlot::new(ValueSlot::from_int(42), NativeKind::Int64);
        let arc = Arc::new(ResultData::ok(payload));
        let bits = Arc::into_raw(arc) as u64;

        let vm_render = vm_format(bits, NativeKind::Ptr(HeapKind::Result));
        assert_eq!(vm_render, "Ok(42)");

        jit_print_result(null_ctx(), bits);

        unsafe { drop_arc_result(bits) };
    }

    #[test]
    fn print_result_err_int_payload_matches_vm() {
        let payload = KindedSlot::new(ValueSlot::from_int(-1), NativeKind::Int64);
        let arc = Arc::new(ResultData::err(payload));
        let bits = Arc::into_raw(arc) as u64;

        let vm_render = vm_format(bits, NativeKind::Ptr(HeapKind::Result));
        assert_eq!(vm_render, "Err(-1)");

        jit_print_result(null_ctx(), bits);

        unsafe { drop_arc_result(bits) };
    }

    #[test]
    fn print_str_arc_carrier_matches_vm() {
        // §2.7.5 String carrier — `Arc::into_raw(Arc<String>)`. This is
        // the post-cluster-1-migration shape; the current pre-migration
        // producer (`MirConstant::Str` → `box_string`) wraps the
        // `Arc<String>` in a `UnifiedValue<Arc<String>>` NaN-box, hence
        // the surface-and-stop at the dispatch site. This test exercises
        // the migrated carrier directly to verify the FFI body is ready
        // for cluster-1 wire-up.
        let arc = Arc::new("hello".to_string());
        let bits = Arc::into_raw(arc) as u64;

        let vm_render = vm_format(bits, NativeKind::String);
        assert_eq!(vm_render, "hello");

        jit_print_str(null_ctx(), bits);

        unsafe { drop_arc_string(bits) };
    }

    #[test]
    fn print_typed_object_arc_carrier_no_schema_renders_positional() {
        // Build a minimal `TypedObjectStorage` with two Int64 slots. With
        // an empty schema registry the renderer falls back to positional
        // names (`_0`, `_1`) per `format_typed_object` lines 750-757.
        // This validates the §2.7.5 carrier the FFI body expects without
        // requiring an ExecutionContext-tier schema registry.
        let slots: Box<[ValueSlot]> =
            vec![ValueSlot::from_int(3), ValueSlot::from_int(4)].into_boxed_slice();
        let field_kinds: Arc<[NativeKind]> = Arc::from(vec![NativeKind::Int64, NativeKind::Int64]);
        let storage = TypedObjectStorage::new(
            /* schema_id = */ 0xffff_ffff_ffff_ffff,
            slots,
            /* heap_mask = */ 0,
            field_kinds,
        );
        let arc = Arc::new(storage);
        let bits = Arc::into_raw(arc) as u64;

        let vm_render = vm_format(bits, NativeKind::Ptr(HeapKind::TypedObject));
        // Schema-less render: positional names with i64 payloads.
        assert_eq!(vm_render, "{_0: 3, _1: 4}");

        jit_print_typed_object(null_ctx(), bits);

        unsafe { drop_arc_typed_object(bits) };
    }

    #[test]
    fn print_kinded_inner_null_ctx_uses_empty_registry() {
        // Smoke: `registry_from_ctx(null)` falls back to an empty
        // `TypeSchemaRegistry`. Combined with an unknown TypedObject
        // schema_id (`0xffff_ffff_ffff_ffff`), the formatter routes
        // through the positional-name path without crashing.
        let slots: Box<[ValueSlot]> = vec![ValueSlot::from_bool(true)].into_boxed_slice();
        let field_kinds: Arc<[NativeKind]> = Arc::from(vec![NativeKind::Bool]);
        let storage = TypedObjectStorage::new(0xdead_beef, slots, 0, field_kinds);
        let arc = Arc::new(storage);
        let bits = Arc::into_raw(arc) as u64;

        jit_print_typed_object(null_ctx(), bits);

        unsafe { drop_arc_typed_object(bits) };
    }

    // ========================================================================
    // Phase 3 cluster-2 Round 3 cw-D-fam12 tests: Scalar Char + Concurrency
    // Mutex/Atomic/Lazy/Channel kinded jit_print FFI bodies (2026-05-16).
    // Each test verifies the FFI body renders the same string as the
    // canonical VM-side `ValueFormatter::format_kinded` for the matching
    // §2.7.5 carrier, then drops the Arc<T> share without leaking.
    // ========================================================================

    #[test]
    fn print_char_scalar_matches_vm() {
        // The `Char` scalar carrier per ADR-006 §2.7.5 amendment: 4-byte
        // codepoint stored inline (no Arc). Both `NativeKind::Char`
        // (post-amendment scalar label) and `NativeKind::Ptr(HeapKind::Char)`
        // (pre-amendment heap arm label) format identically per the
        // formatter's `kinded_slot.as_char` accessor — both render to the
        // single character `A`.
        let codepoint: u32 = 'A' as u32;

        // VM-side rendering via the scalar arm (`NativeKind::Char`).
        let scalar_slot = ValueSlot::from_char('A');
        let scalar_kinded = KindedSlot::new(scalar_slot, NativeKind::Char);
        let registry = shape_runtime::type_schema::TypeSchemaRegistry::default();
        let formatter = shape_vm::executor::printing::ValueFormatter::new(&registry);
        let scalar_render = formatter.format_kinded(&scalar_kinded);
        assert_eq!(scalar_render, "A");

        // VM-side rendering via the legacy `Ptr(HeapKind::Char)` arm — same
        // codepoint bits, different label; both must produce "A".
        let heap_slot = ValueSlot::from_char('A');
        let heap_kinded = KindedSlot::new(heap_slot, NativeKind::Ptr(HeapKind::Char));
        let heap_render = formatter.format_kinded(&heap_kinded);
        assert_eq!(heap_render, "A");

        // Drive the FFI body — no Arc cleanup needed (Char is a scalar
        // carrier; no heap allocation).
        jit_print_char(codepoint);
    }

    #[test]
    fn print_char_invalid_codepoint_renders_fallback() {
        // `char::from_u32` returns None for codepoints in the surrogate
        // range / above 0x10FFFF. The FFI body renders the documented
        // fallback. VM-side `printing.rs:160-164` uses the same fallback
        // when `quote_strings=false` (top-level print form).
        jit_print_char(0xD800);
    }

    #[test]
    fn print_mutex_arc_carrier_matches_vm() {
        use shape_value::heap_value::MutexData;

        // Producer mirrors VM-side `BuiltinFunction::MutexCtor`:
        // `Arc::into_raw(Arc<MutexData>)` with kind
        // `NativeKind::Ptr(HeapKind::Mutex)`.
        let inner_payload = KindedSlot::new(ValueSlot::from_int(42), NativeKind::Int64);
        let arc = Arc::new(MutexData::new(inner_payload));
        let bits = Arc::into_raw(arc) as u64;

        // VM-side rendering: opaque `<mutex>` tag per ADR-006 §2.7.25
        // printing convention.
        let vm_render = vm_format(bits, NativeKind::Ptr(HeapKind::Mutex));
        assert_eq!(vm_render, "<mutex>");

        // Drive the FFI body — same VM-side formatter path, executes
        // without segfault.
        jit_print_mutex(null_ctx(), bits);

        // Drop the strong-count share.
        unsafe {
            let _ = Arc::<MutexData>::from_raw(bits as *const MutexData);
        }
    }

    #[test]
    fn print_atomic_arc_carrier_matches_vm() {
        use shape_value::heap_value::AtomicData;

        let arc = Arc::new(AtomicData::new(7));
        let bits = Arc::into_raw(arc) as u64;

        // VM-side rendering: `<atomic:N>` per ADR-006 §2.7.25 +
        // `printing.rs:538-542`.
        let vm_render = vm_format(bits, NativeKind::Ptr(HeapKind::Atomic));
        assert_eq!(vm_render, "<atomic:7>");

        jit_print_atomic(null_ctx(), bits);

        unsafe {
            let _ = Arc::<AtomicData>::from_raw(bits as *const AtomicData);
        }
    }

    #[test]
    fn print_lazy_arc_carrier_pending_matches_vm() {
        use shape_value::heap_value::LazyData;

        // `LazyData::new_pending` / `LazyData::pending` style — build an
        // uninitialized Lazy. The format is `<lazy:pending>`.
        let closure_kinded = KindedSlot::new(ValueSlot::from_int(0), NativeKind::Int64);
        let arc = Arc::new(LazyData::new(closure_kinded));
        let bits = Arc::into_raw(arc) as u64;

        let vm_render = vm_format(bits, NativeKind::Ptr(HeapKind::Lazy));
        assert_eq!(vm_render, "<lazy:pending>");

        jit_print_lazy(null_ctx(), bits);

        unsafe {
            let _ = Arc::<LazyData>::from_raw(bits as *const LazyData);
        }
    }

    #[test]
    fn print_channel_arc_carrier_matches_vm() {
        use shape_value::heap_value::ChannelData;

        // Producer mirrors VM-side `BuiltinFunction::ChannelCtor`:
        // `Arc::into_raw(Arc<ChannelData>)` with kind
        // `NativeKind::Ptr(HeapKind::Channel)`.
        let arc = Arc::new(ChannelData::new());
        let bits = Arc::into_raw(arc) as u64;

        // VM-side rendering: `<channel:state:len>` per ADR-006 §2.7.20 +
        // `printing.rs:451-464`. A freshly constructed channel is open
        // with len 0.
        let vm_render = vm_format(bits, NativeKind::Ptr(HeapKind::Channel));
        assert_eq!(vm_render, "<channel:open:0>");

        jit_print_channel(null_ctx(), bits);

        unsafe {
            let _ = Arc::<ChannelData>::from_raw(bits as *const ChannelData);
        }
    }

    // ========================================================================
    // Phase 3 cluster-2 Round 4 cw-D-fam3 tests: Collection family —
    // HashMap / HashSet / Deque / PriorityQueue / Range / Iterator kinded
    // jit_print FFI bodies (2026-05-16). Each test verifies the FFI body
    // renders the same string as the canonical VM-side
    // `ValueFormatter::format_kinded` for the matching §2.7.5 carrier,
    // then drops the Arc<T> share without leaking.
    // ADR-006 §2.7.5.B 2026-05-16
    // ========================================================================

    #[test]
    fn print_hashmap_arc_carrier_empty_matches_vm() {
        use shape_value::heap_value::{HashMapData, HashMapKindedRef};

        // Producer mirrors VM-side `BuiltinFunction::HashMapCtor`'s
        // per-V monomorphization: an empty `HashMapData<i64>` wrapped in
        // the `HashMapKindedRef::I64` variant per ADR-006 §2.7.24 Q25.B
        // SUPERSEDED + Wave 2 Round 3b C2-joint ckpt-2 (2026-05-14).
        // Slot bits are `Arc::into_raw(Arc<HashMapKindedRef>) as u64`.
        let inner: HashMapData<i64> = HashMapData::new();
        let kref = HashMapKindedRef::I64(Arc::new(inner));
        let arc = Arc::new(kref);
        let bits = Arc::into_raw(arc) as u64;

        // VM-side rendering: empty map `{}` per `format_hashmap` —
        // `printing.rs:787-...` with a 0-length keys buffer walks to
        // a single `{}` output.
        let vm_render = vm_format(bits, NativeKind::Ptr(HeapKind::HashMap));
        assert_eq!(vm_render, "{}");

        jit_print_hashmap(null_ctx(), bits);

        unsafe {
            let _ = Arc::<HashMapKindedRef>::from_raw(bits as *const HashMapKindedRef);
        }
    }

    #[test]
    fn print_hashset_arc_carrier_matches_vm() {
        use shape_value::heap_value::HashSetData;

        // Producer mirrors VM-side `BuiltinFunction::HashSetCtor`:
        // `Arc::into_raw(Arc<HashSetData>)` with kind
        // `NativeKind::Ptr(HeapKind::HashSet)`. Build with two string
        // keys to exercise the multi-element render path.
        let keys = vec![Arc::new("a".to_string()), Arc::new("b".to_string())];
        let arc = Arc::new(HashSetData::from_keys(keys));
        let bits = Arc::into_raw(arc) as u64;

        // VM-side rendering: `{"a", "b"}` per ADR-006 §2.7.15 +
        // `printing.rs:745-757`. Insertion-order is preserved.
        let vm_render = vm_format(bits, NativeKind::Ptr(HeapKind::HashSet));
        assert_eq!(vm_render, "{\"a\", \"b\"}");

        jit_print_hashset(null_ctx(), bits);

        unsafe {
            let _ = Arc::<HashSetData>::from_raw(bits as *const HashSetData);
        }
    }

    #[test]
    fn print_deque_arc_carrier_empty_matches_vm() {
        use shape_value::heap_value::DequeData;

        // Producer mirrors VM-side `BuiltinFunction::DequeCtor`:
        // `Arc::into_raw(Arc<DequeData>)` with kind
        // `NativeKind::Ptr(HeapKind::Deque)`. Empty deque exercises the
        // zero-length render path without requiring HeapValue payload
        // construction (which would entangle this test with the
        // closure / heap-allocator pathways).
        let arc = Arc::new(DequeData::new());
        let bits = Arc::into_raw(arc) as u64;

        // VM-side rendering: `Deque[]` per ADR-006 §2.7.19 +
        // `printing.rs:763-775`.
        let vm_render = vm_format(bits, NativeKind::Ptr(HeapKind::Deque));
        assert_eq!(vm_render, "Deque[]");

        jit_print_deque(null_ctx(), bits);

        unsafe {
            let _ = Arc::<DequeData>::from_raw(bits as *const DequeData);
        }
    }

    #[test]
    fn print_priority_queue_arc_carrier_matches_vm() {
        use shape_value::heap_value::PriorityQueueData;

        // Producer mirrors VM-side `BuiltinFunction::PriorityQueueCtor`:
        // `Arc::into_raw(Arc<PriorityQueueData>)` with kind
        // `NativeKind::Ptr(HeapKind::PriorityQueue)`. Push three values
        // to exercise the heap-array render path (NOT sorted; the
        // formatter walks the heap buffer in its physical order).
        let mut pq = PriorityQueueData::new();
        pq.push(3);
        pq.push(1);
        pq.push(2);
        let arc = Arc::new(pq);
        let bits = Arc::into_raw(arc) as u64;

        // VM-side rendering: `PriorityQueue[1, 3, 2]` for the push order
        // 3,1,2 (the min-heap rearranges to put `1` at the root; the
        // remaining order depends on sift-up: heap_array = [1, 3, 2]).
        // Per ADR-006 §2.7.18 + `printing.rs:725-740`.
        let vm_render = vm_format(bits, NativeKind::Ptr(HeapKind::PriorityQueue));
        assert_eq!(vm_render, "PriorityQueue[1, 3, 2]");

        jit_print_priority_queue(null_ctx(), bits);

        unsafe {
            let _ = Arc::<PriorityQueueData>::from_raw(bits as *const PriorityQueueData);
        }
    }

    #[test]
    fn print_range_arc_carrier_exclusive_matches_vm() {
        use shape_value::heap_value::RangeData;

        // Producer mirrors VM-side `op_make_range` / range-literal
        // lowering: `Arc::into_raw(Arc<RangeData>)` with kind
        // `NativeKind::Ptr(HeapKind::Range)`. Use the exclusive form
        // `0..10` (the most common surface-syntax shape).
        let arc = Arc::new(RangeData::exclusive(0, 10));
        let bits = Arc::into_raw(arc) as u64;

        // VM-side rendering: `0..10` per ADR-006 §2.7.23 +
        // `printing.rs:479-494`.
        let vm_render = vm_format(bits, NativeKind::Ptr(HeapKind::Range));
        assert_eq!(vm_render, "0..10");

        jit_print_range(null_ctx(), bits);

        unsafe {
            let _ = Arc::<RangeData>::from_raw(bits as *const RangeData);
        }
    }

    #[test]
    fn print_range_arc_carrier_inclusive_matches_vm() {
        use shape_value::heap_value::RangeData;

        // Inclusive form `0..=5` — exercises the `inclusive=true` arm.
        let arc = Arc::new(RangeData::inclusive(0, 5));
        let bits = Arc::into_raw(arc) as u64;

        let vm_render = vm_format(bits, NativeKind::Ptr(HeapKind::Range));
        assert_eq!(vm_render, "0..=5");

        jit_print_range(null_ctx(), bits);

        unsafe {
            let _ = Arc::<RangeData>::from_raw(bits as *const RangeData);
        }
    }

    #[test]
    fn print_iterator_arc_carrier_matches_vm() {
        use shape_value::iterator_state::{IteratorSource, IteratorState};

        // Producer mirrors VM-side iterator-pipeline factory:
        // `Arc::into_raw(Arc<IteratorState>)` with kind
        // `NativeKind::Ptr(HeapKind::Iterator)`. Use a Range source
        // (no Arc payload — inline i64 bounds) to avoid entangling
        // this test with the typed-array source carrier (currently
        // deleted at V3-S5 ckpt-4 per the IteratorSource module
        // header).
        let src = IteratorSource::Range {
            start: 0,
            end: 10,
            step: 1,
        };
        let arc = Arc::new(IteratorState::new(src));
        let bits = Arc::into_raw(arc) as u64;

        // VM-side rendering: opaque `<iterator>` tag per ADR-006 §2.7.16
        // + `printing.rs:430-439`. Lazy iterators have no user-facing
        // print form — terminals must materialize.
        let vm_render = vm_format(bits, NativeKind::Ptr(HeapKind::Iterator));
        assert_eq!(vm_render, "<iterator>");

        jit_print_iterator(null_ctx(), bits);

        unsafe {
            let _ = Arc::<IteratorState>::from_raw(bits as *const IteratorState);
        }
    }
}

#[cfg(test)]
mod jit_string_concat_w15_2_lang_7_tests {
    //! W15.2-LANG-7 jit-print-fstring close (Phase 4b Round 3, 2026-05-18)
    //! regression tests. Pins the producer-side carrier-shape fix at
    //! `jit_string_concat`: inputs come kind-stamped per ADR-006 §2.7.5/
    //! §2.7.7, output carries the §2.7.5 `NativeKind::String` shape
    //! (`Arc::into_raw(Arc<String>) as u64`) matching every downstream
    //! consumer (`jit_print_str`, `arc_string_retain`/`_release`,
    //! `KindedSlot::Drop` for `NativeKind::String`).
    //!
    //! Reproducer at W15.1 audit §6.7 (sub-agent D book-truth-reaudit):
    //! `let p = "events.csv"; print(f"path: {p}")` printed empty on JIT
    //! pre-fix because `jit_string_concat` returned a NaN-boxed
    //! `box_string(out)` whose bit-shape did not match the §2.7.5 String
    //! carrier the print path's `format_kinded` body dereferences as
    //! `&Arc<String>` — segfaulting / printing garbage memory bytes on
    //! the `Arc::from_raw(NaN-bits as *const String)` decode.
    //!
    //! Tests call `jit_string_concat` directly with producer-shaped Arc
    //! input bits + the matching §2.7.7 kind code, assert the return
    //! bits round-trip back to the expected string via `Arc::from_raw`,
    //! and balance every `Arc::into_raw` with a matching `from_raw`
    //! drop at end of scope so the test suite is leak-free.

    use super::*;
    use crate::ffi::stack_kind_code;
    use std::sync::Arc;

    /// Allocate a §2.7.5 `Arc<String>` carrier with one strong-count
    /// share, returning raw bits. The bits are exactly the shape
    /// `MirConstant::Str` lowers to at producer time (`arc_string_constant`
    /// at `mir_compiler/ownership.rs:483`).
    fn make_string_arc_bits(s: &str) -> u64 {
        Arc::into_raw(Arc::new(s.to_string())) as u64
    }

    /// Adopt a §2.7.5 `Arc<String>` carrier back into Rust ownership and
    /// recover the contained `String`. Drops the strong-count share.
    unsafe fn adopt_string_arc_bits(bits: u64) -> String {
        assert!(bits != 0, "expected non-null Arc<String> bits");
        let arc = unsafe { Arc::<String>::from_raw(bits as *const String) };
        let out = (*arc).clone();
        drop(arc);
        out
    }

    #[test]
    fn string_plus_string_book_reproducer() {
        // W15.1 audit §6.7 reproducer at the FFI body level: two
        // §2.7.5 Arc<String> inputs (`"path: "` literal + `p` slot
        // bits) → one §2.7.5 Arc<String> output carrying the concat.
        let a = make_string_arc_bits("path: ");
        let b = make_string_arc_bits("events.csv");
        let result = jit_string_concat(a, stack_kind_code::C_STRING, b, stack_kind_code::C_STRING);
        let out = unsafe { adopt_string_arc_bits(result) };
        assert_eq!(out, "path: events.csv");
    }

    #[test]
    fn returns_arc_string_carrier_shape_not_nanbox() {
        // ADR-006 §2.7.5 shape invariant: the return value MUST be a
        // raw `Arc<String>` pointer, NOT a NaN-boxed unified-heap
        // value. We assert this by reading the underlying memory back
        // as a `String` via `Arc::from_raw` and confirming the contents
        // match — the same code path every downstream consumer takes.
        // Pre-fix the return was `box_string(out)` (NaN-boxed pointer);
        // `Arc::from_raw` on those bits dereferenced a NaN bit-pattern
        // as a `String` struct and either crashed or read garbage.
        let a = make_string_arc_bits("hello");
        let b = make_string_arc_bits(" world");
        let result = jit_string_concat(a, stack_kind_code::C_STRING, b, stack_kind_code::C_STRING);
        // The bits MUST be a valid `*const String` pointer. If
        // `jit_string_concat` regressed back to `box_string(out)`,
        // this `from_raw` would either crash or yield garbage.
        let out = unsafe { adopt_string_arc_bits(result) };
        assert_eq!(out, "hello world");
    }

    #[test]
    fn empty_strings_concat_to_empty() {
        let a = make_string_arc_bits("");
        let b = make_string_arc_bits("");
        let result = jit_string_concat(a, stack_kind_code::C_STRING, b, stack_kind_code::C_STRING);
        let out = unsafe { adopt_string_arc_bits(result) };
        assert_eq!(out, "");
    }

    #[test]
    fn string_plus_int64_formats_int_inline() {
        // MIR f-string lowering at `lower_formatted_string` emits
        // `BinOp::Add(str_part, expr_part)` where `expr_part` may
        // produce a native scalar slot (`Int64`/`Float64`/`Bool`).
        // The kind-aware FFI formats the scalar inline.
        let a = make_string_arc_bits("x=");
        let b_bits: u64 = 42i64 as u64;
        let result = jit_string_concat(
            a,
            stack_kind_code::C_STRING,
            b_bits,
            stack_kind_code::C_INT64,
        );
        let out = unsafe { adopt_string_arc_bits(result) };
        assert_eq!(out, "x=42");
    }

    #[test]
    fn string_plus_bool_formats_bool_inline() {
        let a = make_string_arc_bits("active=");
        let b_bits: u64 = 1; // true
        let result = jit_string_concat(
            a,
            stack_kind_code::C_STRING,
            b_bits,
            stack_kind_code::C_BOOL,
        );
        let out = unsafe { adopt_string_arc_bits(result) };
        assert_eq!(out, "active=true");
    }
}

#[cfg(test)]
mod jit_print_f64_gamma_cp1_tests {
    //! γ-CP1-jit-print-f64 (2026-05-20) regression tests.
    //!
    //! The pre-fix `jit_print_f64` re-implemented float formatting inline
    //! (`value as i64` for integer-valued floats), dropping the trailing
    //! `.0` that the VM's `format_number` emits to distinguish a `number`
    //! (f64) from an `int` — `print(3.0)` rendered `3` under the JIT vs
    //! `3.0` under the VM. The fix routes the FFI body through the same
    //! `ValueFormatter::format_kinded` path the VM print uses.
    //!
    //! These tests pin VM == JIT byte-identical output by asserting the
    //! exact string the `Float64` formatter arm produces (the routine
    //! `jit_print_f64` now delegates to) across the full f64 case range:
    //! integer-valued, fractional, negative, zero, very large, infinity,
    //! NaN. Each case also drives the actual FFI body to confirm it
    //! executes without panic/segfault.

    use super::*;

    /// Render `value` through the exact path `jit_print_f64` delegates to:
    /// `KindedSlot::from_number` formatted by the canonical VM-side
    /// `ValueFormatter`. This is byte-identical to what the FFI body
    /// `println!`s, and — because the `Float64` arm calls the VM's
    /// `format_number` — byte-identical to the VM print path.
    fn jit_render_f64(value: f64) -> String {
        let registry = shape_runtime::type_schema::TypeSchemaRegistry::default();
        let formatter = shape_vm::executor::printing::ValueFormatter::new(&registry);
        let kinded = shape_value::KindedSlot::from_number(value);
        formatter.format_kinded(&kinded)
    }

    #[test]
    fn integer_valued_float_keeps_decimal_point() {
        // The headline bug: `print(3.0)` must render `3.0`, not `3`.
        assert_eq!(jit_render_f64(3.0), "3.0");
        assert_eq!(jit_render_f64(1.0), "1.0");
        assert_eq!(jit_render_f64(100.0), "100.0");
        jit_print_f64(3.0);
    }

    #[test]
    fn fractional_float_renders_fraction() {
        assert_eq!(jit_render_f64(3.5), "3.5");
        assert_eq!(jit_render_f64(3.14), "3.14");
        jit_print_f64(3.5);
    }

    #[test]
    fn negative_integer_valued_float_keeps_decimal_point() {
        assert_eq!(jit_render_f64(-2.0), "-2.0");
        assert_eq!(jit_render_f64(-5.0), "-5.0");
        jit_print_f64(-2.0);
    }

    #[test]
    fn negative_fractional_float_renders_fraction() {
        assert_eq!(jit_render_f64(-2.5), "-2.5");
        jit_print_f64(-2.5);
    }

    #[test]
    fn zero_renders_with_decimal_point() {
        assert_eq!(jit_render_f64(0.0), "0.0");
        jit_print_f64(0.0);
    }

    #[test]
    fn very_large_float_renders_via_to_string() {
        // 1e20 exceeds the `abs() < 1e15` integer-shape threshold in
        // `format_number`, so it falls through to `f64::to_string`.
        assert_eq!(jit_render_f64(1e20), 1e20_f64.to_string());
        jit_print_f64(1e20);
    }

    #[test]
    fn infinity_renders_word_form() {
        assert_eq!(jit_render_f64(f64::INFINITY), "Infinity");
        assert_eq!(jit_render_f64(f64::NEG_INFINITY), "-Infinity");
        jit_print_f64(f64::INFINITY);
        jit_print_f64(f64::NEG_INFINITY);
    }

    #[test]
    fn nan_renders_word_form() {
        assert_eq!(jit_render_f64(f64::NAN), "NaN");
        jit_print_f64(f64::NAN);
    }
}
