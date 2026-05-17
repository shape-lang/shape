//! Typed HashMap FFI helpers for v2 JIT codegen.
//!
//! ## Status: SURFACE (ADR-006 §2.7.4 / W10 jit-playbook §5)
//!
//! Pre-strict-typing this module operated on `ValueWord`-encoded `u64`
//! bits whose heap variant was `HeapValue::HashMap(Box<HashMapData>)`,
//! decoding `map_bits` via `ValueWord::as_hashmap()` (deleted) and
//! computing `key_bits.vw_hash()` (deleted, decoded `tag_bits` from raw
//! bits). Both ends are the W-series defection-attractor pipeline:
//! `ValueWord::as_*` decoded a kind from `tag_bits` (CLAUDE.md
//! "Forbidden Patterns": "Runtime tag_bits dispatch") and the result
//! re-encoded the value as a `ValueWord` (deleted constructor).
//!
//! The strict-typing rebuild target reads the map directly as
//! `Arc<HashMapData>` from a JIT-stamped slot whose `kind ==
//! NativeKind::Ptr(HeapKind::HashMap)` (no decode), then dispatches
//! per-element-kind on the body of the operation:
//!
//! - `get_str_i64(map: Arc<HashMapData>, key: Arc<String>) -> KindedSlot`
//!   (where the result `KindedSlot` carries `kind = NativeKind::Int64`
//!   on hit and `NativeKind::None` on miss — the typed `Option<i64>`
//!   shape per ADR-006 §2.7.6 / Q8).
//! - `set_str_i64(map: Arc<HashMapData>, key: Arc<String>, value: KindedSlot)`
//!   (where the value's `kind` is checked to match the map's element
//!   schema at the JIT-emitted call signature).
//!
//! `HashMapData`'s storage shape was rebuilt for strict typing: keys
//! are `Arc<TypedBuffer<Arc<String>>>`, values are
//! `Arc<TypedBuffer<Arc<HeapValue>>>` (`shape-value/src/heap_value.rs:490`).
//! The legacy `keys: Vec<ValueWord>` / `values: Vec<u64>` shape this
//! module assumed is gone.
//!
//! Until the `KindedSlot`-based JIT FFI shape lands (W11 / deeper
//! Phase-2c — coordinated with `mir_compiler/*` per-opcode lowering
//! and `ffi_symbols/*` registration), the entry points here are
//! removed. Cranelift `call` sites that reference these symbols fail
//! to link, surfacing the upstream blocker.
//!
//! ## Forbidden under any rebuild
//!
//! - `key_bits.vw_hash()` / `ValueWord::as_*` (deleted W-series
//!   classifier path — CLAUDE.md "Forbidden Patterns").
//! - "typed-map bridge" / "hashmap shim" / "key-decode helper" framing
//!   for any kind-supplying shim (CLAUDE.md "Renames to refuse on
//!   sight" — broader family rule).
//! - Bool-default fallback for unknown value kind (W10 jit-playbook
//!   §3 / §5 surface-and-stop).
