//! ARC reference counting FFI for JIT-compiled code.
//!
//! ## Route A unblock (ADR-006 §2.7.14 / W11-jit-new-array)
//!
//! The pre-strict-typing entry points took a kind-blind `u64 bits` and
//! decoded `tag_bits` from the raw u64 to recover a kind for Arc
//! retain/release dispatch — the §2.7.7 #4 / #7 forbidden pattern
//! (CLAUDE.md "Forbidden Patterns" #4). The kind-aware §2.7.5 rebuild
//! ("kind stamped at JIT compile time from the call signature") is
//! the design target.
//!
//! ## Why this module is conservatively no-op'd today
//!
//! The MIR caller side (`mir_compiler/rvalues.rs::Rvalue::Clone` and
//! the `ownership.rs` retain emitter) does NOT yet thread the
//! per-slot `NativeKind` to this FFI — it emits the bare call on every
//! non-`is_native_slot` slot, including `NativeKind::Int64` slots that
//! carry a raw inline integer rather than a heap pointer. Decoding the
//! kind here from `ptr` would require a tag-bit dispatch
//! (CLAUDE.md "Forbidden Patterns" #4 — "Runtime tag_bits dispatch").
//!
//! The W11-jit-new-array Route A unblock takes the conservative route:
//! both entry points are **no-ops** at this layer. Heap-shaped values
//! still leak on Drop (a refcount that never decrements), which is the
//! same memory-safety boundary every prior W-series jit-FFI deferral
//! lived behind. Smoke 1 (no heap allocations) is unblocked. Real
//! retain/release plumbing requires the §2.7.5 kind-stamping at the
//! MIR call site, tracked as a W11-jit-new-array follow-up.
//!
//! ## Forbidden
//!
//! - Bool-default fallback for unknown kind at the FFI boundary
//!   (CLAUDE.md "Forbidden rationalizations").
//! - `tag_bits` decode in the body to recover a kind from `bits`
//!   (CLAUDE.md "Forbidden Patterns" #4).
//! - Treating `ptr` as a `*const HeapHeader` unconditionally and
//!   bumping its refcount: in the current ABI the same parameter
//!   carries raw `i64` integer payloads from typed-int slots
//!   (`NativeKind::Int64`) — interpreting their bits as a pointer
//!   was the segfault path observed during W11 close
//!   (`misaligned pointer dereference: address must be a multiple of
//!   0x4 but is 0x5` for `let mut x = 0 ; for i in 0..5 { x = x + i }`).
//! - "ARC bridge" / "retain helper" / "kind-injection adapter" framing
//!   (CLAUDE.md "Renames to refuse on sight" — broader family rule).

/// W11-jit-new-array unblock: typed-Arc retain is a no-op pending the
/// §2.7.5 kind-stamping rebuild at the MIR call site.
///
/// Under the current MIR lowering, `ownership.rs::compile_operand`
/// emits `arc_retain(val)` on every non-`is_native_slot` Cranelift
/// value — including `NativeKind::Int64` raw-int slots. Without a
/// kind side-channel parameter, this body cannot distinguish a heap
/// pointer from a raw integer; treating either uniformly as a
/// `*const HeapHeader` segfaults on the int case (`misaligned pointer
/// dereference` at address 0x5 for a Cranelift-stored `i64 = 5`).
///
/// Memory consequence: every JIT-routed heap allocation leaks (its
/// refcount never decrements). Same boundary as every prior W-series
/// FFI deferral. Tracked as a W11-jit-new-array follow-up per
/// ADR-006 §2.7.5.
#[unsafe(no_mangle)]
pub extern "C" fn jit_arc_retain(_ptr: *const u8) {
    // No-op (see module docs).
}

/// W11-jit-new-array unblock: typed-Arc release is a no-op pending the
/// §2.7.5 kind-stamping rebuild. Same rationale as `jit_arc_retain`.
#[unsafe(no_mangle)]
pub extern "C" fn jit_arc_release(_ptr: *const u8) {
    // No-op (see module docs).
}
