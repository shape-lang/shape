// ============================================================================
// Market Data FFI Functions
// ============================================================================
//
// Per ADR-006 §2.7.5, the JIT-FFI boundary carries raw `u64` plus a parallel
// `NativeKind` companion stamped at JIT compile time from the call signature.
// These extern "C" entry-points retain the raw `u64` ABI shape so the
// Cranelift call sites in the JIT codegen don't need to change today.

use crate::context::JITContext;
use crate::ffi::value_ffi::*;
// crate::jit_array::JitArray removed — see jit_array.rs SURFACE comment.
// `jit_get_all_rows` returned a `JitArray`-of-data-row indices; it
// surfaces per ADR-006 §2.7.4 / W10 jit-playbook §5.

// DELETED: jit_market_list_instruments - Finance-specific, moved to stdlib/finance/data.shape

// DELETED: jit_market_set_instrument - Finance-specific, moved to stdlib/finance/data.shape

// DELETED: jit_market_init_instruments - Finance-specific, moved to stdlib/finance/data.shape

// DELETED: jit_market_last_rows - Finance-specific, moved to stdlib/finance/data.shape
// Generic replacement: Use jit_get_all_rows() and slice in Shape

/// Get all data rows from the execution context.
///
/// SURFACE (W10 jit-playbook §5 / ADR-006 §2.7.4): result allocation
/// went through the deleted `JitArray::from_vec(...).heap_box()`.
/// Kinded rebuild allocates a `TypedArray<i64>` (data-row indices)
/// per ADR-006 §2.7.6/Q8 and returns through the kinded carrier.
pub extern "C" fn jit_get_all_rows(_ctx: *mut JITContext) -> u64 {
    todo!(
        "phase-2c §2.7.4 / W10 jit-playbook §5: JitArray rebuild — \
         jit_get_all_rows. Result allocation needs `TypedArray<i64>` \
         per ADR-006 §2.7.6/Q8."
    )
}

/// Align multiple symbols by dataset ID
/// `align_series(["ES1!_1m", "NQ1!_1m"], "intersection")` -> Object with aligned data.
///
/// **Phase-2c surface (ADR-006 §2.7.4 / §2.7.5).** The pre-bulldozer body
/// constructed `ValueWord::from_string` / `ValueWord::from_array` /
/// `vmarray_from_vec` arguments to call
/// `shape_runtime::multi_table::functions::align_tables`. Per the
/// strict-typing redesign:
///
/// - `ValueWord` is deleted; the FFI carrier is raw `u64` plus a parallel
///   `NativeKind` companion supplied from the JIT call signature
///   (ADR-006 §2.7.5).
/// - `align_tables` itself migrated to `&[KindedSlot]` and currently returns
///   a deferred error pending Phase 2c kind threading +
///   ModuleContext-access decision (see
///   `crates/shape-runtime/src/multi_table/functions.rs:30`).
///
/// The JIT-side rebuild (assemble `KindedSlot` arguments at the FFI shell
/// from the JIT-emitted per-arg `NativeKind`, dispatch into the kinded
/// `align_tables`) lands alongside that runtime rebuild. Until then this
/// entry-point surface-and-stops per W10 playbook §5.
pub extern "C" fn jit_align_series(
    _ctx: *mut JITContext,
    _symbols_bits: u64,
    _mode_bits: u64,
) -> u64 {
    todo!("phase-2c — see ADR-006 §2.7.4: align_tables kind-threaded rebuild");
}
