//! TypedObject FFI — v2-raw `*mut TypedObjectStorage` carrier.
//!
//! Wave-7 jit-typed-pointer-migration Phase C (2026-07-07): the JIT-private
//! inline-cell `TypedObject` struct (its own `schema_id`/`ref_count` header +
//! inline field cells + custom 64-byte-aligned allocator) is DELETED. Every
//! JIT TypedObject producer and consumer is now on the SAME carrier the VM
//! produces: the `#[repr(C)]` `shape_value::heap_value::TypedObjectStorage`
//! (`HeapHeader` at offset 0, out-of-line slot buffer, refcount via the
//! offset-0 header, self-describing `field_kinds` + `heap_mask`). This is the
//! GC-sound carrier — `cycle_capable_direct_header` reads the offset-0 header
//! and `for_each_heap_child` walks `heap_mask` + `field_kinds` — so a JIT
//! object participates in Bacon–Rajan cycle collection identically to the VM
//! tier.
//!
//! Field addressing on this carrier: the storage pointer IS the slot bits (no
//! NaN-box, no `UNIFIED_PTR_MASK`); load the out-of-line slot buffer base at
//! `storage + JIT_OFFSET_SLOT_DATA` (16), then field `i` lives at
//! `[slot_data + i*8]`. See `mir_compiler/places.rs` (inline hot path) and
//! `field_access.rs` (FFI consumers).
//!
//! The single live producer is `jit_typed_object_alloc` (`allocation.rs`),
//! which allocates via `TypedObjectStorage::_new` with schema-derived
//! `field_kinds`/`heap_mask`. Retain/release route through the offset-0 header
//! (`jit_v2_typed_object_retain`/`_release` in `ffi/v2`); the deleted
//! JIT-private manual `inc_ref`/`dec_ref` split-counter is gone.

mod allocation;
mod field_access;

pub use allocation::*;
pub use field_access::*;
