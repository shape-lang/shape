//! JIT array — surface-and-stop pending strict-typing rebuild.
//!
//! ## Status
//!
//! `JitArray` was previously a type alias for the deleted
//! `shape_value::unified_array::UnifiedArray` (1,134 LoC, removed in
//! commit `0270dd4` "phase-2: delete dead dynamic infrastructure
//! files in shape-value"). The deletion bulldozed the
//! `tags::HEAP_KIND_*` / `is_tagged` / `unified_heap_ptr` machinery
//! the layout was built on; the v2 runtime reads `TypedArray<T>` /
//! typed pointers directly (`docs/runtime-v2-spec.md`), so no surviving
//! shape-value type matches the JIT's `#[repr(C)]` `Vec<u64>`-of-bits
//! layout the Cranelift IR emits offsets into.
//!
//! ## Surface (ADR-006 §2.7.4 / W10 jit-playbook §5)
//!
//! Per ADR-006 §2.7.5, the JIT-FFI carries raw `u64` plus a parallel
//! `NativeKind` companion stamped at JIT compile time. The legacy
//! `JitArray` packed an element kind byte + a typed-mirror pointer
//! into the `#[repr(C)]` heap object — equivalent to per-element
//! `NativeKind` storage on the heap. Rebuilding this under
//! strict-typing is an architectural decision, not a mechanical
//! translation: the JIT's array representation has to either (a)
//! adopt monomorphized `TypedArray<T>` per element kind (matching
//! `shape_value::v2::typed_array::TypedArray<T>`, 24 bytes/header,
//! one allocation per concrete element type), or (b) carry a parallel
//! `Vec<NativeKind>` track alongside the `Vec<u64>` data buffer per
//! the §2.7.7 / §2.7.8 cell-storage pattern.
//!
//! Either route is multi-day work that crosses Cranelift codegen
//! (offsets and field reads), the FFI layer (every consumer in
//! `ffi/array.rs`, `ffi/iterator.rs`, `ffi/object/*`, `ffi/control/*`,
//! `ffi/call_method/*`, `mir_compiler/*`), and the FFI-symbol
//! registration. It is the cluster the W10 audit flagged as a
//! "deeper Phase-2c" / W11 concern.
//!
//! Until that rebuild lands, the public alias and offset constants
//! are kept as `pub` items declared but *not implemented* — every
//! consumer in shape-jit that referenced `JitArray::*` constructors
//! or methods now fails to compile with a clear "unresolved item"
//! error pointing back to this module. That is the
//! deletion-fate signal the playbook §5 calls for; consumers should
//! either wait for the rebuild or surface their own `todo!()` per
//! ADR-006 §2.7.4.
//!
//! ## Forbidden
//!
//! - Do NOT re-introduce `UnifiedArray` under a renamed-but-equivalent
//!   shape ("UnifiedArray shim" / "tag-bit array carrier" / "boundary
//!   array view" / "JitArray bridge" — all defection-attractor framing
//!   per CLAUDE.md "Renames to refuse on sight").
//! - Do NOT add a `tag_bits`-based element decoder.
//! - Do NOT add a Bool-default fallback for unknown element kinds.

// Inline Cranelift offset constants (relative to the `data` field
// region after an 8-byte header). Preserved so the codegen modules
// that compute these from `JitAlloc<JitArray>::DATA_OFFSET + N` keep
// the same arithmetic shape pending the rebuild; flagged as
// `#[allow(dead_code)]` because the strict-typing rebuild may pick a
// different layout entirely. Replaced (not removed) when the
// architectural decision lands.
#[allow(dead_code)]
pub const DATA_OFFSET: i32 = 0;
#[allow(dead_code)]
pub const LEN_OFFSET: i32 = 8;
#[allow(dead_code)]
pub const CAP_OFFSET: i32 = 16;
#[allow(dead_code)]
pub const TYPED_DATA_OFFSET: i32 = 24;
#[allow(dead_code)]
pub const ELEMENT_KIND_OFFSET: i32 = 32;
