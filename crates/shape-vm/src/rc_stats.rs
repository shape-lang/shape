//! ADR-018 §3 (#190): dynamic retain/release counters.
//!
//! Instrumentation only, compiled in exclusively under the `rc-stats` feature
//! (off by default) so the default build carries no counter and no atomic in
//! the refcount path. It exists to answer one question the ticket asks and
//! nothing else can: how many refcount operations the elision actually removes
//! from a running program, as opposed to how many instructions it removes from
//! the bytecode.
//!
//! A "retain" is one `clone_with_kind` call and a "release" is one
//! `drop_with_kind` call — the two operations the opcodes this pass cancels
//! (`CloneLocal`, `DropLocal`, `LoadLocal` + `DropCall`) drive. Both are
//! counted for every kind, including inline scalars, because the pass removes
//! the call itself rather than only its atomic.

use std::sync::atomic::{AtomicU64, Ordering};

static RETAINS: AtomicU64 = AtomicU64::new(0);
static RELEASES: AtomicU64 = AtomicU64::new(0);

#[inline]
pub fn note_retain() {
    RETAINS.fetch_add(1, Ordering::Relaxed);
}

#[inline]
pub fn note_release() {
    RELEASES.fetch_add(1, Ordering::Relaxed);
}

/// `(retains, releases)` since the last [`reset`].
pub fn snapshot() -> (u64, u64) {
    (
        RETAINS.load(Ordering::Relaxed),
        RELEASES.load(Ordering::Relaxed),
    )
}

pub fn reset() {
    RETAINS.store(0, Ordering::Relaxed);
    RELEASES.store(0, Ordering::Relaxed);
}
