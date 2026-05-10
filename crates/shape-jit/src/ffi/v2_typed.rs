//! v2 Typed FFI Functions for JIT — legacy thunks.
//!
//! ## Status: SURFACE (ADR-006 §2.7.4 / W10 jit-playbook §5)
//!
//! Pre-strict-typing this module held `jit_v2_*_legacy` /
//! `jit_v2_*_nanboxed` thunks for the NaN-boxed-input path —
//! marked `#[allow(dead_code)]`/`#[no_mangle]`-stripped because the
//! production v2 path lives in `ffi/v2/mod.rs` (operating on
//! `*const TypedArray<f64>` / `*mut TypedArray<f64>` directly,
//! bypassing NaN-boxing). The thunks decoded `tag_bits::HEAP_PTR_MASK`
//! out of the input `ptr_bits` to extract a raw allocation pointer
//! and dispatched against the deleted `JitArray` heap layout.
//!
//! Both ends are deleted W-series shapes:
//!
//! 1. `JitArray` was an alias for `shape_value::unified_array::UnifiedArray`,
//!    bulldozed in commit `0270dd4` (see `jit_array.rs` for the full
//!    SURFACE comment).
//! 2. `tag_bits::HEAP_PTR_MASK` is the `tag_bits` projection that
//!    CLAUDE.md "Forbidden Patterns" calls out as deleted runtime
//!    `tag_bits` dispatch (the W-series defection-attractor).
//!
//! These thunks were already dead code in the production execution
//! path (the live v2 entry points are in `ffi/v2/mod.rs`); they are
//! removed in bulk here. The strict-typing rebuild target — if any
//! of these legacy shapes ever needs to come back — is a per-kind
//! typed path on the §2.7.5 `(u64 bits, NativeKind kind)` carrier
//! shape, with the kind stamped at JIT compile time and no `tag_bits`
//! projection in the body.
//!
//! ## Forbidden under any rebuild
//!
//! - `tag_bits::HEAP_PTR_MASK` / `PAYLOAD_MASK` / `HEAP_KIND_*` literal
//!   projections (CLAUDE.md "Forbidden Patterns": "Runtime tag_bits
//!   dispatch").
//! - Re-introducing `JitArray` under a renamed-but-equivalent shape
//!   (CLAUDE.md "Renames to refuse on sight" — broader family rule).
//! - Bool-default fallback when `is_heap_kind(...)` would have been
//!   false (W10 jit-playbook §3 / §5 surface-and-stop).
