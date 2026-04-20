//! Small-string interning (Phase D.4).
//!
//! Extracted from `value_word.rs` in Phase R6.3.
//!
//! ## Design rationale
//!
//! True small-string optimization (SSO) in the sense of "pack the bytes inline
//! in the 8-byte ValueWord" is not feasible in the current layout: all 8
//! NaN-boxing tag values (0b000..0b111) are already consumed (see `tag_bits`)
//! and only 48 bits of payload are available, which is too few bytes to be
//! useful (strings <= 6 bytes is a rounding error).
//!
//! Multi-slot SSO (spreading bytes across 2-3 adjacent stack slots) would
//! require compiler support for multi-slot string bindings and invasive
//! changes to the executor + JIT — not worth it as an isolated change.
//!
//! Instead, we collapse the common case of **repeated short strings** via
//! a process-global intern pool. Programs allocate `Arc<String>` over and
//! over for the same content (field names, enum tags, short literals like
//! "ok", "id", "name"). With interning, N copies share a single allocation
//! and the Arc refcount does the rest.
//!
//! ## Behavioural contract
//!
//! - `ValueWord::from_string(s)` still returns a `ValueWord` wrapping
//!   `Arc<String>`. Callers observe no change: `as_string()` / `as_heap_ref()`
//!   return the same `&str` content. Mutation is already impossible via
//!   `Arc<String>` (no `Arc::make_mut` is called on interned strings in the
//!   codebase — all string ops produce a new `String`).
//! - Long strings (len > `INTERN_THRESHOLD`) bypass the pool entirely: the
//!   hash/lookup cost isn't justified for long unique payloads, and the
//!   memory win would be marginal.
//! - The pool is bounded by `INTERN_CAP` entries. When full, new lookups
//!   fall through to the no-intern path — we never evict, keeping all live
//!   `Arc<String>` refs valid.
//! - The pool uses `std::sync::LazyLock<Mutex<...>>` (same pattern as
//!   `shape_graph::GLOBAL_SHAPE_TABLE`). A `HashMap<Arc<String>, ()>` (set
//!   semantics keyed by the Arc's string content) would work, but using
//!   `HashMap<Arc<String>, Arc<String>>` lets us return the *canonical* Arc
//!   without rebuilding one.
//!
//! ## Future work
//!
//! A fully-inline SSO (store up to ~22 bytes inline across a 24-byte heap
//! object with its own refcount) would eliminate the outer `Arc` allocation
//! entirely for short strings. That's a bigger change — it touches the
//! HeapValue representation, VM executor string reads, JIT FFI, and wire
//! serialization. Revisit once the `StringObj` / `UnifiedString` v2 paths
//! are the primary runtime representation.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

/// Strings with byte length <= this value are candidates for interning.
/// Chosen to cover common field names, enum tags, and short literals
/// (e.g. "ok", "err", "id", "name", "type", "value") while excluding
/// long user content where the hash cost dominates.
pub const INTERN_THRESHOLD: usize = 32;

/// Hard cap on pool size. When reached, new strings bypass interning.
/// Sized to comfortably fit all stdlib field names + enum tags + common
/// literals across a large program. Entries are never evicted once
/// inserted (the pool owns an Arc ref keeping the string alive).
pub const INTERN_CAP: usize = 8192;

static POOL: LazyLock<Mutex<HashMap<Arc<String>, Arc<String>>>> =
    LazyLock::new(|| Mutex::new(HashMap::with_capacity(256)));

/// Return the canonical `Arc<String>` for `s` if `s` is short enough to
/// intern; otherwise return `s` unchanged. Callers should always use
/// the returned Arc — it may be a different (shared) pointer than the
/// input.
#[inline]
pub fn intern_short_string(s: Arc<String>) -> Arc<String> {
    if s.len() > INTERN_THRESHOLD {
        return s;
    }
    // Acquire the lock. If the mutex is poisoned (another thread panicked
    // while holding it), fall through without interning rather than
    // propagating the panic — interning is an optimization, not a
    // correctness requirement.
    let mut pool = match POOL.lock() {
        Ok(guard) => guard,
        Err(_) => return s,
    };
    if let Some(existing) = pool.get(&s) {
        return existing.clone();
    }
    if pool.len() >= INTERN_CAP {
        return s;
    }
    pool.insert(s.clone(), s.clone());
    s
}

/// Test-only: current pool size. Pool entries are never cleared, so
/// tests should use deltas (not absolute values) to verify growth.
#[cfg(test)]
pub(crate) fn __test_pool_len() -> usize {
    POOL.lock().map(|p| p.len()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    // The intern pool is a process-global resource and tests run in parallel,
    // so assertions must be robust to cross-test pollution. We use string
    // contents that are unlikely to appear in other tests (unique prefixes
    // per test) and check relative behavior ("two calls with the same content
    // return Arc-equal results") rather than absolute pool sizes where
    // possible.

    #[test]
    fn test_intern_short_strings_share_allocation() {
        // Two separately-allocated `Arc<String>` with identical content should
        // deduplicate to a single canonical Arc after going through the pool.
        let a = intern_short_string(Arc::new("intern_test_share_name".to_string()));
        let b = intern_short_string(Arc::new("intern_test_share_name".to_string()));
        assert!(Arc::ptr_eq(&a, &b), "interned short strings must share allocation");
        assert_eq!(&*a, "intern_test_share_name");
    }

    #[test]
    fn test_intern_long_strings_bypass_pool() {
        // A string longer than INTERN_THRESHOLD bypasses the pool.
        // Use a unique prefix so other parallel tests can't collide.
        let long = format!("intern_test_long_{}", "x".repeat(INTERN_THRESHOLD + 1));
        assert!(long.len() > INTERN_THRESHOLD);
        let a = intern_short_string(Arc::new(long.clone()));
        let b = intern_short_string(Arc::new(long.clone()));
        // Both pass through untouched; they are NOT the same allocation.
        assert!(!Arc::ptr_eq(&a, &b), "long strings must not be interned");
        assert_eq!(&*a, &*b);
    }

    #[test]
    fn test_intern_threshold_boundary() {
        // Exactly at the threshold: interned. (Use a fixed-length unique
        // string — "aaaa..." padded to exactly THRESHOLD bytes.)
        let at: String = std::iter::repeat('a').take(INTERN_THRESHOLD).collect();
        let a1 = intern_short_string(Arc::new(at.clone()));
        let a2 = intern_short_string(Arc::new(at.clone()));
        assert!(Arc::ptr_eq(&a1, &a2), "len == threshold must intern");

        // One past the threshold: NOT interned.
        let over: String = std::iter::repeat('b').take(INTERN_THRESHOLD + 1).collect();
        let b1 = intern_short_string(Arc::new(over.clone()));
        let b2 = intern_short_string(Arc::new(over.clone()));
        assert!(!Arc::ptr_eq(&b1, &b2), "len > threshold must not intern");
    }

    #[test]
    fn test_intern_preserves_content_across_many_calls() {
        // Repeatedly interning varied short strings returns correct content
        // on every call, including for repeated inputs.
        let inputs = [
            "intern_test_many_a",
            "intern_test_many_b",
            "intern_test_many_c",
            "intern_test_many_a", // duplicate
            "intern_test_many_b", // duplicate
        ];
        let mut results = Vec::new();
        for s in inputs {
            results.push(intern_short_string(Arc::new(s.to_string())));
        }
        for (r, s) in results.iter().zip(inputs.iter()) {
            assert_eq!(&***r, *s);
        }
        // Duplicates must be Arc-equal to their first occurrence.
        assert!(Arc::ptr_eq(&results[0], &results[3]), "dup 'a' must share Arc");
        assert!(Arc::ptr_eq(&results[1], &results[4]), "dup 'b' must share Arc");
    }
}
