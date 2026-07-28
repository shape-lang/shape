//! The single allocation seam for typed heap carriers (#194, ADR-018 §4).
//!
//! Every typed heap carrier in the runtime — `TypedArray<T>`, `StringObj`,
//! `DecimalObj`, the raw closure block, `TypedObjectStorage`, the JIT's
//! struct/string/SIMD buffers, the VM's native cells — used to reach for
//! `std::alloc::{alloc, alloc_zeroed, realloc, dealloc}` directly, spread
//! across thirteen files. That had three consequences worth ending:
//!
//! 1. **The heap-growth ceiling was bypassable.** `alloc_budget` was consulted
//!    at exactly one of the growth paths (`TypedArray::grow`); the sibling
//!    growth path for `HashMapData` values and every fresh-allocation path
//!    were unmetered. A ceiling that only some allocations honour does not
//!    bound anything.
//! 2. **Allocation behaviour was unobservable.** There was no place to count
//!    allocations, so "does this refactor change how much we allocate" was not
//!    a question the test suite could answer.
//! 3. **Region allocation had nowhere to land.** Per-region bump allocation
//!    (#195) needs one place to decide where a block comes from. Thirteen
//!    files of direct `std::alloc` calls is thirteen places.
//!
//! This module is that one place. It is deliberately thin: system-allocator
//! semantics, no arena, no caching, no size classes. The pointer it returns is
//! the pointer `std::alloc` returned, with the same provenance and the same
//! null-on-failure contract — callers keep whatever null handling they already
//! had (`assert!`, `handle_alloc_error`, or a refusal path).
//!
//! ## The region extension point (#195)
//!
//! The API takes no region parameter, and that is the point. When regions
//! arrive, the region a block comes from is selected by an ambient
//! thread-local region context read *inside* [`alloc_block`], and
//! [`dealloc_block`] — which already receives the block's `Layout` — is the
//! single site that can classify a pointer as region-owned (bulk-retired, so
//! its `dealloc` is a no-op) versus system-owned. Neither change resigns a
//! single call site. Adding an explicit `alloc_block_in(region, layout)` for
//! call sites that want to name a region stays available on top; what matters
//! is that the ~40 existing sites do not have to thread a parameter they have
//! no opinion about.
//!
//! ## What this seam does NOT do
//!
//! It does not track ownership, does not know what a block holds, and does not
//! free anything on its own. Layout correctness at `dealloc` remains the
//! caller's obligation exactly as before — the seam cannot check that the
//! `Layout` handed to [`dealloc_block`] matches the one that allocated the
//! block, and a mismatch is UB just as it was with a direct `dealloc` call.

use super::alloc_budget::{self, AllocBudgetExceeded};
use std::alloc::Layout;
use std::cell::Cell;

thread_local! {
    /// Count of blocks handed out by this seam on the current thread.
    static ALLOC_COUNT: Cell<u64> = const { Cell::new(0) };
    /// Count of blocks returned to the allocator through this seam.
    static DEALLOC_COUNT: Cell<u64> = const { Cell::new(0) };
    /// Count of in-place growth attempts (`realloc`) through this seam. A
    /// `realloc` is neither a fresh block nor a freed one, so it is counted
    /// separately rather than folded into either total.
    static REALLOC_COUNT: Cell<u64> = const { Cell::new(0) };
}

/// Seam-observed allocation counters for the current thread.
///
/// These exist so allocation *behaviour* is testable: a carrier operation's
/// allocation count is an assertable fact, not something inferred from timing
/// or from reading the implementation. Because the seam is the only path to
/// the allocator for typed heap carriers (enforced by the `check-alloc-seam`
/// gate), these counts are the carrier allocation counts.
///
/// They deliberately do NOT count Rust-side `Vec`/`String`/`Box` traffic —
/// only the raw typed-carrier blocks this seam hands out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AllocCounts {
    /// Blocks allocated (fresh allocations, zeroed or not).
    pub allocs: u64,
    /// Blocks deallocated.
    pub deallocs: u64,
    /// In-place growth attempts.
    pub reallocs: u64,
}

impl AllocCounts {
    /// Blocks allocated but not yet freed, as observed by this seam. Saturates
    /// at zero rather than underflowing when a block allocated before a
    /// [`reset_counts`] is freed after it.
    pub fn live(self) -> u64 {
        self.allocs.saturating_sub(self.deallocs)
    }
}

/// Read the current thread's seam counters.
pub fn counts() -> AllocCounts {
    AllocCounts {
        allocs: ALLOC_COUNT.with(|c| c.get()),
        deallocs: DEALLOC_COUNT.with(|c| c.get()),
        reallocs: REALLOC_COUNT.with(|c| c.get()),
    }
}

/// Zero the current thread's seam counters. Intended for tests that want to
/// measure one operation.
pub fn reset_counts() {
    ALLOC_COUNT.with(|c| c.set(0));
    DEALLOC_COUNT.with(|c| c.set(0));
    REALLOC_COUNT.with(|c| c.set(0));
}

/// Consult the per-buffer heap ceiling for a block of `layout`, recording a
/// breach if it is over.
///
/// Returns `Err` when the block exceeds the ceiling. Every entry point in this
/// module calls this, which is what makes the ceiling unbypassable: there is
/// no path to the allocator for a typed heap carrier that does not pass
/// through here.
///
/// Recording is unconditional; *refusing* is not. Whether a breach refuses the
/// allocation or merely records it is the caller's decision, expressed by
/// which entry point it uses — see [`alloc_block`] versus [`try_alloc_block`].
#[inline]
fn meter(layout: Layout) -> Result<(), AllocBudgetExceeded> {
    match alloc_budget::check_size(layout.size() as u64) {
        Ok(()) => Ok(()),
        Err(e) => {
            alloc_budget::record_breach(e);
            Err(e)
        }
    }
}

/// Allocate a block, for callers with no refusal channel.
///
/// A breach of the heap ceiling is *recorded* (the VM surfaces it as a clean
/// `VMError` at the next dispatch-loop safepoint) but does not refuse the
/// allocation. That asymmetry is deliberate. Callers of this entry point —
/// `StringObj::new`, `TypedArray::with_capacity`, the JIT's struct
/// materialisation — have no way to report failure to their own caller; they
/// `assert!` on null. Refusing here would convert a memory-ceiling breach into
/// a process abort, which is precisely the outcome `alloc_budget`'s
/// record-and-continue design was built to avoid (a `panic!` on a serve node
/// kills every in-flight request). So the allocation proceeds, the VM learns
/// about it at the next safepoint, and execution ends with a surfaced error
/// instead of exit 101.
///
/// This is strictly more coverage than before, not less: these paths
/// previously did not consult the ceiling at all.
///
/// Returns null on allocator failure, exactly as `std::alloc::alloc` does.
#[inline]
pub fn alloc_block(layout: Layout) -> *mut u8 {
    let _ = meter(layout);
    ALLOC_COUNT.with(|c| c.set(c.get() + 1));
    // SAFETY: `Layout` is non-zero-sized by the caller's construction; the
    // zero-size case is the caller's to avoid, as it was with a direct call.
    unsafe { std::alloc::alloc(layout) }
}

/// Allocate a zeroed block. See [`alloc_block`] for the breach semantics.
#[inline]
pub fn alloc_zeroed_block(layout: Layout) -> *mut u8 {
    let _ = meter(layout);
    ALLOC_COUNT.with(|c| c.set(c.get() + 1));
    // SAFETY: as [`alloc_block`].
    unsafe { std::alloc::alloc_zeroed(layout) }
}

/// Allocate a block, refusing on a heap-ceiling breach.
///
/// For callers that *can* report failure — the growth paths, which leave
/// capacity unchanged and let the VM surface the recorded breach. Refusing is
/// what bounds a runaway: a doubling growth loop trips the ceiling and stops
/// there instead of climbing until the host OOM-killer reaps the process.
#[inline]
pub fn try_alloc_block(layout: Layout) -> Result<*mut u8, AllocBudgetExceeded> {
    meter(layout)?;
    ALLOC_COUNT.with(|c| c.set(c.get() + 1));
    // SAFETY: as [`alloc_block`].
    Ok(unsafe { std::alloc::alloc(layout) })
}

/// Grow a block in place where the allocator can, refusing on a heap-ceiling
/// breach.
///
/// `old_layout` must be the layout the block was allocated with, and
/// `new_size` must be at least `old_layout.size()`. On refusal the block is
/// untouched and still owned by the caller at `old_layout`.
///
/// # Safety
/// `ptr` must denote a live block allocated through this seam with exactly
/// `old_layout`, and `new_size` must form a valid `Layout` with
/// `old_layout.align()`.
#[inline]
pub unsafe fn try_realloc_block(
    ptr: *mut u8,
    old_layout: Layout,
    new_size: usize,
) -> Result<*mut u8, AllocBudgetExceeded> {
    // Meter the size the block is growing TO. `alloc_budget` models an
    // absolute per-buffer ceiling, not a cumulative budget, so the new size is
    // the quantity under test and no credit is owed on free.
    meter(unsafe { Layout::from_size_align_unchecked(new_size, old_layout.align()) })?;
    REALLOC_COUNT.with(|c| c.set(c.get() + 1));
    // SAFETY: forwarded from this function's contract.
    Ok(unsafe { std::alloc::realloc(ptr, old_layout, new_size) })
}

/// Grow a block, for callers with no refusal channel.
///
/// The [`alloc_block`] breach semantics applied to growth: a ceiling breach is
/// recorded for the VM to surface, but the growth proceeds rather than
/// refusing, because these callers report success by returning normally and
/// have no way to say "capacity denied".
///
/// # Safety
/// As [`try_realloc_block`].
#[inline]
pub unsafe fn realloc_block(ptr: *mut u8, old_layout: Layout, new_size: usize) -> *mut u8 {
    let _ = meter(unsafe { Layout::from_size_align_unchecked(new_size, old_layout.align()) });
    REALLOC_COUNT.with(|c| c.set(c.get() + 1));
    // SAFETY: forwarded from this function's contract.
    unsafe { std::alloc::realloc(ptr, old_layout, new_size) }
}

/// Return a block to the allocator.
///
/// # Safety
/// `ptr` must denote a live block allocated through this seam with exactly
/// `layout`. A layout mismatch is UB — the seam does not and cannot check it.
#[inline]
pub unsafe fn dealloc_block(ptr: *mut u8, layout: Layout) {
    DEALLOC_COUNT.with(|c| c.set(c.get() + 1));
    // SAFETY: forwarded from this function's contract.
    unsafe { std::alloc::dealloc(ptr, layout) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v2::alloc_budget::BudgetGuard;

    /// Round-trip a block through the seam and confirm the counters move.
    #[test]
    fn alloc_dealloc_roundtrip_counts() {
        reset_counts();
        let layout = Layout::from_size_align(64, 8).unwrap();
        let p = alloc_block(layout);
        assert!(!p.is_null());
        assert_eq!(counts().allocs, 1, "one block handed out");
        assert_eq!(counts().live(), 1, "block is live before free");
        unsafe { dealloc_block(p, layout) };
        assert_eq!(counts().deallocs, 1);
        assert_eq!(counts().live(), 0, "block retired");
    }

    #[test]
    fn zeroed_block_is_zeroed_and_counted() {
        reset_counts();
        let layout = Layout::from_size_align(32, 8).unwrap();
        let p = alloc_zeroed_block(layout);
        assert!(!p.is_null());
        let slice = unsafe { std::slice::from_raw_parts(p, 32) };
        assert!(slice.iter().all(|&b| b == 0), "alloc_zeroed must zero");
        assert_eq!(counts().allocs, 1);
        unsafe { dealloc_block(p, layout) };
    }

    /// The ceiling refuses an over-size block on the refusal-channel entry
    /// point, and the refusal costs no allocation.
    #[test]
    fn try_alloc_refuses_over_ceiling() {
        let _g = BudgetGuard::new(Some(100));
        reset_counts();
        let layout = Layout::from_size_align(101, 8).unwrap();
        let r = try_alloc_block(layout);
        assert!(r.is_err(), "over-ceiling block must be refused");
        assert_eq!(counts().allocs, 0, "a refused allocation allocates nothing");
        assert!(
            alloc_budget::take_breach().is_some(),
            "refusal records a breach for the VM to surface"
        );
    }

    /// The no-refusal-channel entry point records the breach but still
    /// allocates — turning a ceiling breach into a `panic!` on these paths is
    /// the outcome the record-and-continue design exists to prevent.
    #[test]
    fn alloc_block_records_breach_without_refusing() {
        let _g = BudgetGuard::new(Some(100));
        reset_counts();
        let layout = Layout::from_size_align(101, 8).unwrap();
        let p = alloc_block(layout);
        assert!(
            !p.is_null(),
            "must not refuse: caller has no failure channel"
        );
        assert!(
            alloc_budget::take_breach().is_some(),
            "breach is recorded even though the allocation proceeded"
        );
        unsafe { dealloc_block(p, layout) };
    }

    /// Under the ceiling, nothing is recorded and nothing is refused.
    #[test]
    fn under_ceiling_is_transparent() {
        let _g = BudgetGuard::new(Some(1024));
        reset_counts();
        let layout = Layout::from_size_align(512, 8).unwrap();
        let p = try_alloc_block(layout).expect("under the ceiling must succeed");
        assert!(
            alloc_budget::take_breach().is_none(),
            "no breach under ceiling"
        );
        unsafe { dealloc_block(p, layout) };
    }

    /// With no ceiling installed (the CLI default) the seam is a pass-through.
    #[test]
    fn unlimited_default_allocates_freely() {
        let _g = BudgetGuard::new(None);
        reset_counts();
        let layout = Layout::from_size_align(1 << 20, 8).unwrap();
        let p = try_alloc_block(layout).expect("no ceiling means no refusal");
        assert!(alloc_budget::take_breach().is_none());
        unsafe { dealloc_block(p, layout) };
    }

    /// Growth is metered against the size the block grows TO, and a refused
    /// growth leaves the original block intact and usable.
    #[test]
    fn realloc_refusal_leaves_block_intact() {
        let _g = BudgetGuard::new(Some(100));
        let layout = Layout::from_size_align(64, 8).unwrap();
        let p = alloc_block(layout);
        unsafe { std::ptr::write_bytes(p, 0xAB, 64) };
        let _ = alloc_budget::take_breach();

        let r = unsafe { try_realloc_block(p, layout, 200) };
        assert!(r.is_err(), "growth past the ceiling must be refused");
        assert!(alloc_budget::take_breach().is_some());
        // The original block is untouched and still ours at `layout`.
        let byte = unsafe { *p };
        assert_eq!(byte, 0xAB, "refused growth must not disturb the block");
        unsafe { dealloc_block(p, layout) };
    }

    #[test]
    fn realloc_growth_under_ceiling_preserves_contents() {
        let _g = BudgetGuard::new(Some(4096));
        let layout = Layout::from_size_align(64, 8).unwrap();
        let p = alloc_block(layout);
        unsafe { std::ptr::write_bytes(p, 0xCD, 64) };
        let grown = unsafe { try_realloc_block(p, layout, 128) }.expect("under ceiling");
        assert!(!grown.is_null());
        let slice = unsafe { std::slice::from_raw_parts(grown, 64) };
        assert!(
            slice.iter().all(|&b| b == 0xCD),
            "realloc preserves contents"
        );
        unsafe { dealloc_block(grown, Layout::from_size_align(128, 8).unwrap()) };
    }

    /// Counters are per-thread, so one thread's measurement cannot be
    /// perturbed by another thread's allocations — this is what lets the
    /// allocation-count tripwire run under a parallel test harness.
    #[test]
    fn counters_are_thread_local() {
        reset_counts();
        let layout = Layout::from_size_align(16, 8).unwrap();
        std::thread::spawn(move || {
            let p = alloc_block(layout);
            unsafe { dealloc_block(p, layout) };
            assert_eq!(counts().allocs, 1, "the spawned thread sees its own count");
        })
        .join()
        .unwrap();
        assert_eq!(
            counts().allocs,
            0,
            "another thread's allocations must not appear in this thread's count"
        );
    }
}
