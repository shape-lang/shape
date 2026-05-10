//! ARC reference counting FFI for JIT-compiled code.
//!
//! ## Status: SURFACE (ADR-006 §2.7.4 / W10 jit-playbook §5)
//!
//! The pre-strict-typing entry points were:
//!
//! ```ignore
//! pub extern "C" fn jit_arc_retain(bits: u64) -> u64
//! pub extern "C" fn jit_arc_release(bits: u64)
//! ```
//!
//! Both delegated to the deleted `shape_value::value_word_drop`
//! `vw_clone` / `vw_drop` kind-blind W-series helpers that decoded
//! `tag_bits` from the raw `u64` to recover a kind for Arc
//! retain/release dispatch. CLAUDE.md "Forbidden Patterns" lists
//! this exact shape under the deleted runtime tag-bit dispatch entry.
//! The W-series defection-attractor family forbids any
//! "decode/tag/dispatch helper/bridge/probe" framing these helpers
//! would need to come back under.
//!
//! ## Strict-typing rebuild target
//!
//! Per ADR-006 §2.7.5, the JIT-FFI boundary carries `(u64 bits,
//! NativeKind kind)` with `kind` stamped at JIT compile time from the
//! call signature. The kind-aware retain/release entry points have to
//! take the kind as a stable-FFI companion (e.g. a `u32` packed
//! discriminator the JIT codegen and the FFI shim agree on) and
//! dispatch on it via the canonical kind-aware retain/release —
//! `shape_value::KindedSlot::clone()` / `Drop` (which mirror
//! `shape-vm/src/executor/vm_impl/stack.rs::clone_with_kind` /
//! `drop_with_kind`).
//!
//! ## Why this is W11 / deeper Phase-2c, not a W10 mechanical fix
//!
//! 1. **Stable-FFI kind encoding is not yet defined**: `NativeKind`
//!    is not `#[repr(C)]` and there is no canonical `u8`/`u32`
//!    packed-discriminator encoding for the JIT-FFI boundary. Picking
//!    one is an architectural decision — the same choice the W10
//!    Band 1 close-out flagged as the §2.7.5 "stamped at JIT compile
//!    time from the call signature" surface.
//!
//! 2. **Caller-side codegen change**: every Cranelift `call` site that
//!    emits `arc_retain(val)` / `arc_release(val)` (the principal
//!    user is `mir_compiler/rvalues.rs::Rvalue::Clone` and the
//!    matching `Rvalue::Drop` lowering) has to start emitting a
//!    second i32/i64 argument carrying the kind. That work lives in
//!    W10-mir-compiler's territory and is the upstream blocker for
//!    this module's body.
//!
//! 3. **`KindedSlot::clone()` is the right body, but the caller
//!    contract does not yet supply `kind`**: writing the body now
//!    would need either a Bool-default kind (CLAUDE.md "Forbidden
//!    rationalizations") or a `tag_bits`-decode (CLAUDE.md "Forbidden
//!    Patterns"), so the body has to wait for the kind to flow in.
//!
//! Until those land, the public entry points are removed: every JIT
//! call site that referenced `jit_arc_retain` / `jit_arc_release`
//! now fails to link with a clear "unresolved symbol" error pointing
//! back to this module — the deletion-fate signal the playbook §5
//! calls for.
//!
//! ## Forbidden under any rebuild
//!
//! - Bool-default for unknown kind at the FFI boundary (W10 jit-
//!   playbook §3 / §5 surface-and-stop, CLAUDE.md "Bool-default
//!   fallback").
//! - `tag_bits` decode in the body to recover a kind from `bits`
//!   (CLAUDE.md "Forbidden Patterns" / "tag_bits restoration").
//! - Any "ARC bridge" / "retain helper" / "kind-injection adapter"
//!   framing for the kind-supplying shim (CLAUDE.md "Renames to
//!   refuse on sight" — broader family rule).
