//! JIT array — Route A close.
//!
//! ## Route A close (ADR-006 §2.7.14 / W11-jit-new-array)
//!
//! Q15 resolved Route A: kinded per-element-kind `Arc<TypedArrayData>`
//! monomorphization. The deleted `UnifiedArray` heap layout
//! (`#[repr(C)]` with `data` / `len` / `cap` / `typed_data` /
//! `element_kind` byte) is gone. Under Route A the JIT and VM share
//! the `shape_value::v2::typed_array::TypedArray<T>` carrier (24-byte
//! header: `HeapHeader` at offset 0, `*mut T` at offset 8, `len: u32`
//! at offset 16, `cap: u32` at offset 20). Element kind is stamped at
//! the producing call-site (`jit_v2_array_new_<kind>`) and not stored
//! on the heap object — it lives on the `HeapValue::TypedArray(arc)`
//! variant the `HeapHeader.kind` field discriminates (§2.7.6 / Q8
//! single-discriminator).
//!
//! Cranelift offsets for the typed-array layout live inline at the
//! consumer sites (`mir_compiler/v2_array.rs`'s `DATA_PTR_OFFSET = 8`
//! and `LEN_OFFSET = 16`). The five legacy offset constants
//! (`DATA_OFFSET`, `LEN_OFFSET`, `CAP_OFFSET`, `TYPED_DATA_OFFSET`,
//! `ELEMENT_KIND_OFFSET`) that the W10-cascade close marked
//! `#[allow(dead_code)]` are deleted — under Route A there is no
//! ELEMENT_KIND byte on the heap, and the remaining offsets are no
//! longer the JIT's canonical reference shape.
//!
//! ## Forbidden under any future expansion
//!
//! - `UnifiedArray` revival under any renamed shape (CLAUDE.md
//!   "Renames to refuse on sight" — broader-family regex).
//! - `tag_bits`-based element decoder.
//! - Bool-default fallback for unknown element kind.
//! - Re-introducing an ELEMENT_KIND byte on the heap object — the
//!   discriminator lives on `HeapValue::TypedArray(arc)` per ADR-005
//!   §1 single-discriminator.

// No public items — Route A close (W11-jit-new-array): consumers use
// `shape_value::v2::typed_array::TypedArray<T>` directly via the
// existing `jit_v2_array_*` FFI surface and the inline-codegen helpers
// in `mir_compiler/v2_array.rs`.
