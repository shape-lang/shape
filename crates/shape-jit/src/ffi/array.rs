//! Array FFI Functions for JIT.
//!
//! ## Route A close (ADR-006 §2.7.14 / W11-jit-new-array)
//!
//! Q15 resolved Route A: kinded per-element-kind `Arc<TypedArrayData>`
//! monomorphization. Allocation goes through `v2_array_new_<kind>`
//! (defined in `ffi/v2/mod.rs`); push goes through `v2_array_push`
//! dispatched by element byte size; indexed get/set/len/SIMD ops are
//! inlined directly against the `TypedArray<T>` layout from
//! `mir_compiler/v2_array.rs`. The legacy kind-blind `jit_new_array`
//! / `jit_array_get` / `jit_array_push` / `jit_array_pop` / etc.
//! entries (the ValueWord-shape ABI) are not re-introduced.
//!
//! The `ArrayInfo` carrier struct below is kept in its `#[repr(C)]`
//! shape because downstream symbol-table consumers (the deleted
//! `jit_array_info` entry's signature in any future probe) and non-
//! array consumers reference it through the public type. The
//! `is_inline_bool` helper is similarly preserved as a §2.7.5 carrier
//! that's not a tag-bit decode (it tests against the JIT's stable
//! inline-bool constants from a known-Bool slot).
//!
//! ## Forbidden under any future expansion
//!
//! - `JitArray` revival under any renamed shape (CLAUDE.md "Renames
//!   to refuse on sight" — broader-family regex).
//! - Bool-default fallback for unknown element kind.
//! - `tag_bits`-based element decoder.

use super::value_ffi::{TAG_BOOL_FALSE, TAG_BOOL_TRUE};

// ============================================================================
// Public ABI carrier preserved for downstream symbol-table compatibility.
// ============================================================================

/// JIT-FFI carrier struct for legacy array-info consumers.
///
/// Preserved in its `#[repr(C)]` shape so the Cranelift signature
/// declaration in `ffi_symbols/array_symbols.rs` (if any future
/// kinded-info entry registers) has a stable target type. The body
/// that previously populated this is gone; under Route A consumers
/// inline-load the `data` / `len` fields from the `TypedArray<T>`
/// layout directly.
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
