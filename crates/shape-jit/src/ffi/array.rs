//! Array FFI Functions for JIT.
//!
//! ## Status: SURFACE (ADR-006 §2.7.4 / W10 jit-playbook §5)
//!
//! Every entry point in this module was structured around the JIT's
//! `JitArray` C-layout heap object (data pointer + len + cap + a typed
//! mirror + element-kind byte) — formerly aliased to
//! `shape_value::unified_array::UnifiedArray` and deleted along with
//! the `tags::HEAP_KIND_*` / `is_tagged` / `unified_heap_ptr`
//! machinery in commit `0270dd4`. See `jit_array.rs` for the full
//! deletion / rebuild commentary.
//!
//! Until the strict-typing JIT array representation lands (W11 /
//! deeper Phase-2c), the entry points here are removed in bulk: the
//! Cranelift FFI registration in `ffi_symbols/array_symbols.rs` (W10-
//! ffi-symbols sub-cluster) and the consumer call sites in
//! `mir_compiler/*` will fail to find these symbols, surfacing the
//! upstream blocker as a clear "unresolved symbol" — the deletion-
//! fate signal the playbook §5 calls for.
//!
//! ## What the rebuild has to choose
//!
//! Per ADR-006 §2.7.5 the JIT-FFI carries `(u64 bits, NativeKind kind)`.
//! For arrays specifically:
//!
//! 1. **Element kind** — every entry that produces a fresh array
//!    (`jit_new_array`, `jit_array_filled`, `jit_range`, etc.) takes a
//!    static `NativeKind` for the elements at JIT compile time;
//!    runtime is no longer responsible for tracking element kind on
//!    the heap object.
//! 2. **Storage shape** — option A: monomorphize per element kind
//!    (`Arc<TypedArrayData>` per arm, matching `shape_value::v2::typed_array::TypedArray<T>`
//!    and the §2.7.6/Q8 cardinality bound on heap kinds). Option B:
//!    keep the unified `Vec<u64>` shape but extend with a parallel
//!    `Vec<NativeKind>` track per the §2.7.7 / §2.7.8 cell-storage
//!    pattern.
//! 3. **Method dispatch** — the receiver-side bodies (`first`, `last`,
//!    `min`, `max`, `push`, `reverse`, `filled`, `range`, `slice`,
//!    `zip`, `info`, `pop`) move to the §2.7.10/Q11
//!    `&[KindedSlot] -> Result<KindedSlot, VMError>` pattern (already
//!    landed for VM-side method dispatch); the JIT FFI shim wraps that.
//!
//! ## Forbidden under any rebuild
//!
//! - `tag_bits::HEAP_KIND_ARRAY` literal — the discriminator now lives
//!   on `HeapKind::TypedArray` and is read from the heap header, not
//!   reconstructed from a bit pattern (CLAUDE.md "Forbidden Patterns":
//!   "Runtime tag_bits dispatch").
//! - "JitArray bridge" / "UnifiedArray shim" / "element-kind helper"
//!   framing for any kind-supplying shim (CLAUDE.md "Renames to refuse
//!   on sight" — broader family rule).
//! - Bool-default fallback for unknown element kind (W10 jit-playbook
//!   §3 / §5 surface-and-stop).

use super::value_ffi::{TAG_BOOL_FALSE, TAG_BOOL_TRUE};

// ============================================================================
// Public ABI carrier preserved for downstream symbol-table compatibility.
// ============================================================================

/// JIT-FFI carrier struct returned by the (now-removed) `jit_array_info`
/// entry. Kept in its `#[repr(C)]` shape so the Cranelift signature
/// declaration in `ffi_symbols/array_symbols.rs` still has a target
/// type for the rebuild — the body that populated it is gone.
#[repr(C)]
pub struct ArrayInfo {
    pub data_ptr: u64,
    pub length: u64,
}

// ============================================================================
// Helper used by other (still-living) FFI sites to materialize a fresh
// element-kind label without tag-bit decode. Kept as a §2.7.5 carrier.
// ============================================================================

/// Returns `true` iff `value_bits` carries an inline-true / inline-false
/// constant per `value_ffi::TAG_BOOL_*`. Distinct from the deleted
/// `tag_bits::is_tagged`/`get_tag` shape: the input is a JIT-stamped
/// boolean payload from a known-Bool slot, not a runtime classifier.
#[inline]
pub fn is_inline_bool(value_bits: u64) -> bool {
    value_bits == TAG_BOOL_TRUE || value_bits == TAG_BOOL_FALSE
}
