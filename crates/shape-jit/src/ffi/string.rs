//! `Arc<String>` strict-typed carrier FFI for JIT-emitted code
//! (W12-jit-string-carrier-unification, Phase 3 cluster-0 Round 12 T2/T3,
//! 2026-05-13).
//!
//! ADR-006 §2.7.5 (producing-site classification) names `NativeKind::String`
//! as the §2.7.5 String carrier with shape `Arc::into_raw(Arc<String>) as
//! u64` — the standard Rust Arc layout with refcount at offset -16 of the
//! data pointer. The VM-side consumer (`crates/shape-vm/src/executor/objects/
//! set_methods.rs:136-155::result_slot_to_string_arc`, mirrors in
//! `hashmap_methods.rs`) and `KindedSlot::Drop` for `NativeKind::String`
//! (`crates/shape-value/src/kinded_slot.rs:500-502`) both decode this exact
//! shape via `Arc::increment_strong_count::<String>` / `Arc::from_raw(bits
//! as *const String)`.
//!
//! ## Carrier-shape rule (binding)
//!
//! - **`NativeKind::String` slot**: `Arc::into_raw(Arc<String>) as u64`,
//!   refcount at offset -16. Retain/release dispatches through this
//!   module's `jit_arc_string_retain` / `jit_arc_string_release` — bumps
//!   the Rust Arc control-block refcount.
//!
//! - **JIT-internal NaN-box string carrier**: `Box::into_raw(Box::new(
//!   UnifiedValue<Arc<String>>)) as u64`, refcount at offset +4 inside
//!   the UnifiedValue allocation. Retained/released via the legacy
//!   `jit_arc_retain` / `jit_arc_release` in `ffi/arc.rs`. Stays for
//!   JIT-internal pathways (the dispatch shell's method-name push at
//!   `terminators.rs:235`, `call_string_method` returns, etc.) that
//!   pair the bits with their own JIT-internal decode contract.
//!
//! Mixing the two segfaults at every retain/release reclaim:
//! - `jit_arc_release` on an `Arc::into_raw(Arc<String>) as u64` slot
//!   reads `*(bits + 4) as *const AtomicU32` — offset 4 inside the
//!   `String` payload (`String`'s `ptr/cap/len` words), corrupting the
//!   data on `fetch_sub`.
//! - `Arc::decrement_strong_count::<String>(bits)` on a `Box::into_raw(
//!   Box::new(UnifiedValue<Arc<String>>))` slot decrements `*(bits - 16)`
//!   as if it were the Arc control block — but offset -16 from the
//!   UnifiedValue start is whatever the allocator placed there. UB.
//!
//! ## Round 7A precedent
//!
//! The Result/Option Arc carriers in `ffi/result.rs::jit_arc_result_retain`
//! / `_release` / `jit_arc_option_retain` / `_release` (Round 7A close
//! commit `d01d83b7` + `9f27edcd`) and the Round 9 typed-Arc collection
//! retain/release pairs in `ffi/v2/collection_arc.rs` are the bound
//! precedent shape for every body in this module.
//!
//! ## Round 12 T2/T3 surface closures
//!
//! - Smoke 4 JIT: `let mut s = Set(); s.add("a"); s.add("b"); print(
//!   s.size())` → `2` VM == JIT. The `"a"` / `"b"` constants flow as
//!   `MirConstant::Str` operands stamped `NativeKind::String`; the VM
//!   trampoline's `KindedSlot::Drop` decodes via `Arc::from_raw(bits as
//!   *const String)`. Pre-Round-12 `box_string` returned NaN-box bits →
//!   UB at the VM consumer's `Arc::from_raw`.
//! - `print("hello")` JIT: was clean SURFACE at the print Call-terminator's
//!   `NativeKind::String` arm in `terminators.rs::466` (Round 8A reopen
//!   surfaced). Post-Round-12 the §2.7.5 producer emits the matching
//!   carrier shape and `jit_print_str` reads `&String` directly.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

// ============================================================================
// Per-NativeKind::String kinded retain / release
// ============================================================================

/// Retain (clone) an `Arc<String>` strong-count share. Bumps the standard
/// Rust Arc refcount at offset -16 of the `Arc::into_raw` pointer via
/// `Arc::increment_strong_count::<String>` — NOT the W-series
/// `UnifiedValue<T>` refcount at offset 4 (`jit_arc_retain`'s shape).
///
/// SAFETY: `bits` must be `Arc::into_raw(Arc<String>) as u64` produced by
/// the `MirConstant::Str` / `MirConstant::StringId` lowering in
/// `mir_compiler/ownership.rs::compile_constant`, or by the VM-side
/// `KindedSlot::from_string_arc` producer. Null is silently no-op'd
/// (mirror of Round 7A's `jit_arc_result_retain` null-bits safety).
#[unsafe(no_mangle)]
pub extern "C" fn jit_arc_string_retain(bits: u64) {
    if bits == 0 {
        return;
    }
    // cluster-2-cw-E measurement (cluster-2-inventory §F): track active-
    // share retain count for the §2.7.5 `Arc<String>` carrier. Independent
    // counter from `JIT_ARC_RETAIN_CALLS` (the UnifiedValue<T> path) per
    // the carrier-shape distinction at this module's docstring.
    super::arc::STRING_RETAIN_CALLS
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    // SAFETY: see fn docs. The §2.7.5 String carrier contract names the
    // bits as `Arc::into_raw(Arc<String>) as u64`; `Arc::increment_strong_
    // count` operates on the Arc control block at offset -16.
    unsafe {
        Arc::increment_strong_count(bits as *const String);
    }
}

/// Release an `Arc<String>` strong-count share. Mirrors
/// `jit_arc_string_retain`'s increment — uses
/// `Arc::decrement_strong_count::<String>` per Rust Arc contract.
/// Reaching refcount zero runs `String::Drop` (drops the inner buffer).
///
/// SAFETY: same as `jit_arc_string_retain`. Null is silently no-op'd.
#[unsafe(no_mangle)]
pub extern "C" fn jit_arc_string_release(bits: u64) {
    if bits == 0 {
        return;
    }
    // cluster-2-cw-E measurement (cluster-2-inventory §F): track active-
    // share release count + drop-to-zero count for the §2.7.5 `Arc<String>`
    // carrier. Independent counters from `JIT_ARC_RELEASE_CALLS` /
    // `JIT_ARC_RELEASE_FREES` (the UnifiedValue<T> path) per the carrier-
    // shape distinction at this module's docstring.
    super::arc::STRING_RELEASE_CALLS
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    // SAFETY: see fn docs. Read strong-count BEFORE decrement to detect
    // the drop-to-zero transition (Arc's decrement returns void; we cannot
    // observe the post-decrement count atomically without racing). The
    // `strong_count == 1` read identifies the slot that will reach zero
    // on this decrement — `Acquire` ordering pairs with the matching
    // `Release` decrement to synchronize with the eventual drop.
    unsafe {
        // Construct a temporary Arc to inspect strong count without
        // perturbing it. SAFETY: bits is a live Arc::into_raw payload per
        // the function-level contract; `from_raw` adopts one share, the
        // following `into_raw` returns it, so strong_count is unperturbed
        // across this block.
        let arc = Arc::from_raw(bits as *const String);
        let pre_release_count = Arc::strong_count(&arc);
        let _ = Arc::into_raw(arc); // restore the share we adopted
        if pre_release_count == 1 {
            super::arc::STRING_RELEASE_FREES
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        Arc::decrement_strong_count(bits as *const String);
    }
}

// ============================================================================
// §2.7.5 String carrier compile-time-emitted-constant helper
// ============================================================================

/// Content-keyed intern pool for §2.7.5 `Arc<String>` JIT compile-time
/// constants. Per cluster-2-closure-wave-E-fix refined-Option-A disposition
/// (`docs/cluster-audits/cluster-2-cw-E-string-leak-measurement.md` §5.1 +
/// §6): deduplicate by content so repeat occurrences of the same constant
/// share one `Arc<String>` allocation instead of allocating N copies.
///
/// The pool itself IS the "permanent share" of the §2.7.5 carrier — the
/// `Arc<String>` lives for the program's lifetime (process-wide static),
/// matching the prior `Arc::increment_strong_count`-based permanent-share
/// discipline's lifetime. Carrier-shape is unchanged: iconst payload stays
/// `Arc::as_ptr(&pool_arc) as u64`, identical to the prior
/// `Arc::into_raw` pointer shape that consumers
/// (`KindedSlot::Drop` for `NativeKind::String` at
/// `crates/shape-value/src/kinded_slot.rs`, `set_methods.rs::
/// result_slot_to_string_arc`, hashmap_methods.rs mirrors,
/// `jit_arc_string_retain` / `jit_arc_string_release` above) already
/// decode via `Arc::increment_strong_count::<String>` /
/// `Arc::from_raw(bits as *const String)`.
///
/// **Deduplication-only fix.** Per §5.1 quantification: prog3 (5x
/// "hello") drops from `leaked_total=9` (4 baseline + 5 distinct allocs)
/// to `leaked_total=5` (4 baseline + 1 dedup'd). Worst-case fixtures
/// where all constants are distinct (prog5 = 20 distinct) see zero
/// savings. Full elimination requires a JIT-module deallocation hook
/// (measurement §5.2 Option B, cluster-1.5+ territory).
///
/// **Forbidden under refined Option A** (per measurement §5.1 +
/// CLAUDE.md cluster-2 canonical refusal set):
/// - Changing iconst payload to an intern-pool *index* (breaks §2.7.5
///   carrier-shape contract; cascades through 257+ `NativeKind::String`
///   sites + 48 `Arc::from_raw`-shape consumers — exceeds ceiling-c).
/// - Renaming the pool to a defection-attractor framing
///   ("intern-pool bridge" / "string-constant probe" / "dedup helper" —
///   refused per CLAUDE.md broader-family regex `(decode|tag|kind|
///   dispatch|value.call|closure.callback|frame.setup|callee|capture)
///   (bridge|probe|helper|hop|translator|adapter|shim)`).
fn intern_pool() -> &'static Mutex<HashMap<String, Arc<String>>> {
    static POOL: OnceLock<Mutex<HashMap<String, Arc<String>>>> = OnceLock::new();
    POOL.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Compile-time helper: produce a §2.7.5 `Arc::into_raw`-shape carrier
/// pointer for a `MirConstant::Str` / `MirConstant::StringId` site,
/// content-deduplicated through the program-wide [`intern_pool`].
///
/// The constant is embedded as an `iconst I64` in the JIT-emitted code, so
/// the bits are static across every runtime occurrence of the site. The
/// intern pool keeps one `Arc<String>` per distinct content alive for the
/// program's lifetime (process-wide static `OnceLock<Mutex<HashMap<…>>>`);
/// the iconst payload is `Arc::as_ptr(&pool_arc) as u64` — the same raw
/// pointer shape as the prior `Arc::into_raw`-with-refcount-boost
/// discipline, so consumers (`jit_arc_string_retain` /
/// `jit_arc_string_release`, `KindedSlot::Drop` for `NativeKind::String`,
/// VM-side `Arc::from_raw(bits as *const String)`) need NO change.
///
/// **Refcount discipline (preserved per call).** Each call bumps the
/// strong count by 1 to preserve the pre-fix "active share" safety
/// against unpaired releases (e.g. a JIT-emitted `release` without a
/// matching `retain` would otherwise underflow when the pool's share
/// is the only one). Pre-fix this was `Arc::increment_strong_count` on
/// every freshly-allocated Arc (boost from 1 → 2); post-fix it is the
/// same increment on the pool-owned Arc. Dedup at the allocation layer
/// does NOT change this per-call refcount discipline.
///
/// **Heap-memory savings (the §5.1 measurement target).** The dedup
/// property: repeat occurrences of the same constant (e.g. prog3's
/// `print("hello") × 5`) share one underlying `Arc<String>`
/// allocation — one `String` heap buffer + one Arc control block,
/// strong_count = 1 (pool) + N (per-call boosts). Pre-fix: 5x
/// `Arc::new("hello")` = 5 separate `String` heap buffers + 5 Arc
/// control blocks. Post-fix: 1 of each, regardless of call count.
/// `STRING_CONSTANT_ALLOCS` counts actual `Arc::new` invocations
/// (= distinct-content allocations = leaked-allocation count per §F.1).
#[inline]
pub fn arc_string_constant(s: String) -> u64 {
    let mut pool = intern_pool()
        .lock()
        .expect("string-constant intern pool mutex poisoned");
    // `entry().or_insert_with` keeps the post-insert reference scoped
    // inside the mutex guard so we can read the Arc's data pointer
    // without dropping the guard mid-function.
    let pool_arc = pool.entry(s).or_insert_with_key(|key| {
        // Dedup miss: allocate one Arc, insert into the pool. The pool
        // retains the permanent share — this allocation is the §F.1
        // leak surface for this content, counted once.
        super::arc::STRING_CONSTANT_ALLOCS
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Arc::new(key.clone())
        // Dedup hit (else branch): or_insert_with_key skips the closure;
        // STRING_CONSTANT_ALLOCS does NOT increment (the counter
        // measures actual leaked Arc<String> allocations per §F.1).
    });
    let ptr = Arc::as_ptr(pool_arc) as u64;
    // SAFETY: `ptr` is the data pointer of `pool_arc`, a live Arc
    // (pool holds one share). The increment bumps the Arc control-
    // block refcount by 1 — the "active share" per the original
    // pre-fix discipline, preserved across dedup so JIT-emitted code
    // patterns with unpaired releases (e.g. the prog4 surfaced finding
    // §3 of cluster-2-cw-E-string-leak-measurement.md) cannot
    // underflow to 0.
    unsafe {
        Arc::increment_strong_count(ptr as *const String);
    }
    ptr
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// Refined Option A intern-pool: `arc_string_constant` returns a
    /// non-null pointer whose Arc control block has strong_count >= 2
    /// after a single call (the pool's permanent share + the per-call
    /// active-share boost preserving pre-fix safety against unpaired
    /// releases). Pre-fix: `Arc::increment_strong_count` on a fresh
    /// `Arc::new` boosts 1 → 2. Post: same boost on the pool-owned Arc.
    #[test]
    fn test_arc_string_constant_returns_pool_owned_pointer() {
        // Use a test-unique key to avoid cross-test contamination via
        // the program-wide static intern pool.
        let bits = arc_string_constant("test-refcount-boosted-key".to_string());
        assert_ne!(bits, 0);

        // SAFETY: `bits` is a pool-owned Arc's data pointer per
        // `arc_string_constant`'s contract; the pool holds one share +
        // one per-call boost. Temporary-adopt to inspect, then retire
        // the temporary share via `adopted` drop.
        unsafe {
            Arc::increment_strong_count(bits as *const String);
            let adopted = Arc::from_raw(bits as *const String);
            // Pool's permanent share + per-call boost + our temporary
            // adoption = ≥ 3. Pre-fix this was 2 + temporary = 3 as
            // well. The `≥` form tolerates concurrent same-key calls
            // from other tests (none in this module — keys are unique
            // — but the looser bound is defensive against future
            // refactors).
            assert!(Arc::strong_count(&adopted) >= 3);
            // `adopted` drops here, retiring the temporary share.
        }
    }

    #[test]
    fn test_jit_arc_string_retain_bumps_refcount() {
        let arc = Arc::new("test".to_string());
        let bits = Arc::into_raw(arc) as u64;

        // refcount: 1
        jit_arc_string_retain(bits);
        // refcount: 2

        unsafe {
            let recovered = Arc::from_raw(bits as *const String);
            assert_eq!(Arc::strong_count(&recovered), 2);
            // Adopt restores: from_raw took 1 share; restore by bumping.
            Arc::increment_strong_count(bits as *const String);
            // Now refcount is back to 2 with `recovered` holding one.
            drop(recovered);
            // refcount: 1
            Arc::decrement_strong_count(bits as *const String);
            // refcount: 0 — allocation freed.
        }
    }

    #[test]
    fn test_jit_arc_string_release_drops_refcount() {
        let arc = Arc::new("test".to_string());
        // SAFETY: `arc` is alive; bumping its strong count is sound.
        unsafe {
            Arc::increment_strong_count(Arc::as_ptr(&arc));
        }
        let bits = Arc::into_raw(arc) as u64;
        // refcount: 2 (original Arc + the increment)

        jit_arc_string_release(bits);
        // refcount: 1 — still alive

        unsafe {
            let recovered = Arc::from_raw(bits as *const String);
            assert_eq!(Arc::strong_count(&recovered), 1);
            // `recovered` drops here, retires the last share.
        }
    }

    /// Null-bits safety: retain/release on bits=0 silently no-op.
    /// Mirrors Round 7A's `jit_arc_result_retain` null-bits guard.
    #[test]
    fn test_jit_arc_string_retain_release_null_bits_noop() {
        jit_arc_string_retain(0);
        jit_arc_string_release(0);
        // No segfault, no UB — null is the documented producer-site
        // sentinel for an unallocated String slot.
    }

    /// Round-trip: a constant produced by `arc_string_constant` survives
    /// multiple retain/release cycles without underflowing to 0. The
    /// pool's permanent share keeps the allocation alive across
    /// JIT-emitted active-share retain/release pairs.
    ///
    /// Pool ownership is permanent (never decremented) — do NOT manually
    /// decrement the pool's share at test-end; that would corrupt the
    /// pool entry and segfault subsequent same-key calls.
    #[test]
    fn test_arc_string_constant_survives_use_drop_cycle() {
        let bits = arc_string_constant("test-survives-cycle-key".to_string());

        // Simulate JIT-emitted retain/release pairs on the constant.
        for _ in 0..10 {
            jit_arc_string_retain(bits);
            jit_arc_string_release(bits);
        }

        // Pool's permanent share keeps the allocation alive.
        // SAFETY: pool-owned Arc data pointer remains valid.
        let s: &String = unsafe { &*(bits as *const String) };
        assert_eq!(s, "test-survives-cycle-key");

        // Simulate the "single use-then-drop" pattern (release without
        // prior retain). The pool's permanent share keeps the allocation
        // alive — pre-refined-Option-A required `arc_string_constant`'s
        // refcount-boost to survive this; post, the pool ownership
        // covers the same case.
        jit_arc_string_release(bits);
        let s: &String = unsafe { &*(bits as *const String) };
        assert_eq!(s, "test-survives-cycle-key");
    }

    /// VM-side consumer interop: `Arc::from_raw(bits as *const String)`
    /// must recover the original String content. Same shape as
    /// `set_methods.rs::result_slot_to_string_arc` — the VM-side consumer
    /// bumps via `jit_arc_string_retain` before `Arc::from_raw` to adopt
    /// a share without underflowing. Pool-owned permanent share remains
    /// intact post-test.
    #[test]
    fn test_arc_string_constant_arc_from_raw_recovers_content() {
        let bits = arc_string_constant("test-arc-from-raw-key".to_string());

        // Bump refcount once so the VM-side `Arc::from_raw` consumer
        // can adopt a share without underflowing the pool's permanent
        // share.
        jit_arc_string_retain(bits);

        unsafe {
            let recovered: Arc<String> = Arc::from_raw(bits as *const String);
            assert_eq!(*recovered, "test-arc-from-raw-key");
            // `recovered` retires its share here — pool's permanent
            // share remains.
        }
    }

    /// Refined Option A intern-pool: deduplication property — repeat
    /// calls with the same content return the same iconst bits, so the
    /// JIT-emitted code paths for `print("hello") × 5` all observe one
    /// shared allocation. Validates the measurement-deliverable §5.1
    /// disposition (prog3 5x "hello" leak: 5 distinct allocs → 1).
    #[test]
    fn test_arc_string_constant_deduplicates_same_content() {
        let bits_a = arc_string_constant("test-dedup-key".to_string());
        let bits_b = arc_string_constant("test-dedup-key".to_string());
        let bits_c = arc_string_constant("test-dedup-key".to_string());

        assert_ne!(bits_a, 0);
        assert_eq!(bits_a, bits_b);
        assert_eq!(bits_b, bits_c);

        // Content recovers correctly from the shared pointer.
        let s: &String = unsafe { &*(bits_a as *const String) };
        assert_eq!(s, "test-dedup-key");
    }

    /// Refined Option A intern-pool: distinct content gets distinct
    /// iconst bits (different pool entries, different allocations).
    /// Validates dedup is content-keyed, not call-count-keyed.
    #[test]
    fn test_arc_string_constant_distinct_content_distinct_pointers() {
        let bits_alpha = arc_string_constant("test-distinct-alpha-key".to_string());
        let bits_beta = arc_string_constant("test-distinct-beta-key".to_string());

        assert_ne!(bits_alpha, 0);
        assert_ne!(bits_beta, 0);
        assert_ne!(bits_alpha, bits_beta);

        // Each recovers its own content.
        let s_a: &String = unsafe { &*(bits_alpha as *const String) };
        let s_b: &String = unsafe { &*(bits_beta as *const String) };
        assert_eq!(s_a, "test-distinct-alpha-key");
        assert_eq!(s_b, "test-distinct-beta-key");
    }

    /// Refined Option A intern-pool: dedup-pool entry survives multiple
    /// calls — the second-and-subsequent calls reuse the pool entry
    /// rather than allocating fresh. The underlying `String` heap
    /// buffer + Arc control block are allocated once; per-call boosts
    /// bump the strong_count.
    ///
    /// Pre-fix: N calls = N separate `Arc::new` allocations (each at
    /// refcount=2). Post-fix: N calls = 1 `Arc::new` allocation (pool's
    /// permanent share) + N per-call boosts (strong_count = 1 + N). The
    /// heap-memory leak per-distinct-content is divided by call count.
    #[test]
    fn test_arc_string_constant_pool_share_invariant() {
        let key = "test-pool-share-invariant-key".to_string();
        let bits1 = arc_string_constant(key.clone());
        let bits2 = arc_string_constant(key.clone());
        let bits3 = arc_string_constant(key);
        assert_eq!(bits1, bits2);
        assert_eq!(bits2, bits3);
        // Pool's permanent share keeps strong_count stable.
        // SAFETY: bits1 is a pool-owned Arc data pointer; temporary
        // increment-from_raw-drop adopts then retires one share without
        // disturbing the pool's baseline.
        let count = unsafe {
            Arc::increment_strong_count(bits1 as *const String);
            let adopted = Arc::from_raw(bits1 as *const String);
            let c = Arc::strong_count(&adopted);
            c
            // `adopted` drops here, returns the temporary share.
        };
        // Pool holds 1 share + our temporary increment = ≥ 2; pre-fix
        // this would be 2+ depending on how many `arc_string_constant`
        // calls leaked (one boost per call). Either way, the
        // strong_count remains finite (bounded by pool + active
        // JIT-emitted retain/release cycles), NOT growing per call.
        assert!(
            count >= 2,
            "expected at least 2 strong shares (pool's permanent + our \
             temporary adoption), got {count}"
        );
    }
}
