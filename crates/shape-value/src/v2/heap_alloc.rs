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

/// Seam counters, compiled only where something reads them.
///
/// A thread-local increment per allocation and per free sounds free and is
/// not: unconditional counters cost ~8% on the charter's `alloc_tree` workload
/// and ~5% on `alloc_object_graph`, measured before/after on the committed
/// suite. Instrumentation that expensive does not belong in a shipped build,
/// and #194 is meant to be a pure refactor — so the counters compile away
/// unless a test or the `alloc-stats` feature asks for them, leaving the
/// production path carrying only the ceiling check the ticket requires.
#[cfg(any(test, feature = "alloc-stats"))]
mod counters {
    use std::cell::Cell;

    thread_local! {
        /// Count of blocks handed out by this seam on the current thread.
        pub(super) static ALLOC_COUNT: Cell<u64> = const { Cell::new(0) };
        /// Count of blocks returned to the allocator through this seam.
        pub(super) static DEALLOC_COUNT: Cell<u64> = const { Cell::new(0) };
        /// Count of in-place growth attempts (`realloc`) through this seam. A
        /// `realloc` is neither a fresh block nor a freed one, so it is counted
        /// separately rather than folded into either total.
        pub(super) static REALLOC_COUNT: Cell<u64> = const { Cell::new(0) };
    }
}

#[cfg(any(test, feature = "alloc-stats"))]
#[inline]
fn note_alloc() {
    counters::ALLOC_COUNT.with(|c| c.set(c.get() + 1));
}

#[cfg(not(any(test, feature = "alloc-stats")))]
#[inline(always)]
fn note_alloc() {}

#[cfg(any(test, feature = "alloc-stats"))]
#[inline]
fn note_dealloc() {
    counters::DEALLOC_COUNT.with(|c| c.set(c.get() + 1));
}

#[cfg(not(any(test, feature = "alloc-stats")))]
#[inline(always)]
fn note_dealloc() {}

#[cfg(any(test, feature = "alloc-stats"))]
#[inline]
fn note_realloc() {
    counters::REALLOC_COUNT.with(|c| c.set(c.get() + 1));
}

#[cfg(not(any(test, feature = "alloc-stats")))]
#[inline(always)]
fn note_realloc() {}

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
#[cfg(any(test, feature = "alloc-stats"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AllocCounts {
    /// Blocks allocated (fresh allocations, zeroed or not).
    pub allocs: u64,
    /// Blocks deallocated.
    pub deallocs: u64,
    /// In-place growth attempts.
    pub reallocs: u64,
}

#[cfg(any(test, feature = "alloc-stats"))]
impl AllocCounts {
    /// Blocks allocated but not yet freed, as observed by this seam. Saturates
    /// at zero rather than underflowing when a block allocated before a
    /// [`reset_counts`] is freed after it.
    pub fn live(self) -> u64 {
        self.allocs.saturating_sub(self.deallocs)
    }
}

/// Read the current thread's seam counters.
#[cfg(any(test, feature = "alloc-stats"))]
pub fn counts() -> AllocCounts {
    AllocCounts {
        allocs: counters::ALLOC_COUNT.with(|c| c.get()),
        deallocs: counters::DEALLOC_COUNT.with(|c| c.get()),
        reallocs: counters::REALLOC_COUNT.with(|c| c.get()),
    }
}

/// Zero the current thread's seam counters. Intended for tests that want to
/// measure one operation.
#[cfg(any(test, feature = "alloc-stats"))]
pub fn reset_counts() {
    counters::ALLOC_COUNT.with(|c| c.set(0));
    counters::DEALLOC_COUNT.with(|c| c.set(0));
    counters::REALLOC_COUNT.with(|c| c.set(0));
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
    note_alloc();
    // SAFETY: `Layout` is non-zero-sized by the caller's construction; the
    // zero-size case is the caller's to avoid, as it was with a direct call.
    unsafe { std::alloc::alloc(layout) }
}

/// Allocate a zeroed block. See [`alloc_block`] for the breach semantics.
#[inline]
pub fn alloc_zeroed_block(layout: Layout) -> *mut u8 {
    let _ = meter(layout);
    note_alloc();
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
    note_alloc();
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
    note_realloc();
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
    note_realloc();
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
    note_dealloc();
    // SAFETY: forwarded from this function's contract.
    unsafe { std::alloc::dealloc(ptr, layout) }
}

/// Allocation-count and layout-identity differential for the typed heap
/// carriers (#194 tripwire 1).
///
/// Every number in this module is a fact about how the runtime allocates,
/// pinned so a refactor cannot change it silently. The counts were the same
/// before the seam existed — the seam replaced each direct `std::alloc` call
/// one-for-one — so asserting them here is asserting equality with the
/// pre-seam behaviour, and any future change to a count has to be made
/// deliberately by editing an expectation.
///
/// The layout assertions exist for the same reason from the other direction:
/// the JIT emits raw loads at literal byte offsets into these carriers, so a
/// field moving is a silent miscompile rather than a build failure. `size_of`
/// and `offset_of` are checked here so the failure is loud and local.
#[cfg(test)]
mod alloc_differential {
    use super::*;
    use crate::v2::decimal_obj::DecimalObj;
    use crate::v2::string_obj::StringObj;
    use crate::v2::typed_array::TypedArray;
    use std::mem::{align_of, offset_of, size_of};

    // ── Layout identity ─────────────────────────────────────────────────────

    /// `TypedArray<T>` is 24 bytes with header@0, data@8, len@16, cap@20 for
    /// every monomorphization. The JIT hard-codes these offsets and
    /// `free_v2_typed_array_memory_only` frees element-type-erased on the
    /// strength of the size being fixed, so both facts are load-bearing.
    #[test]
    fn typed_array_layout_is_pinned() {
        macro_rules! assert_layout {
            ($t:ty) => {
                assert_eq!(size_of::<TypedArray<$t>>(), 24, "TypedArray size");
                assert_eq!(align_of::<TypedArray<$t>>(), 8, "TypedArray align");
                assert_eq!(offset_of!(TypedArray<$t>, header), 0, "header offset");
                assert_eq!(offset_of!(TypedArray<$t>, data), 8, "data offset");
                assert_eq!(offset_of!(TypedArray<$t>, len), 16, "len offset");
                assert_eq!(offset_of!(TypedArray<$t>, cap), 20, "cap offset");
            };
        }
        assert_layout!(f64);
        assert_layout!(i64);
        assert_layout!(i32);
        assert_layout!(u8);
        assert_layout!(f32);
        assert_layout!(char);
        assert_layout!(*const u8);
    }

    #[test]
    fn string_and_decimal_layout_is_pinned() {
        assert_eq!(size_of::<StringObj>(), 24, "StringObj size");
        assert_eq!(align_of::<StringObj>(), 8, "StringObj align");
        assert_eq!(offset_of!(StringObj, header), 0);
        assert_eq!(offset_of!(StringObj, data), StringObj::OFFSET_DATA);
        assert_eq!(offset_of!(StringObj, len), StringObj::OFFSET_LEN);

        assert_eq!(offset_of!(DecimalObj, header), 0);
        assert_eq!(offset_of!(DecimalObj, value), DecimalObj::OFFSET_VALUE);
    }

    // ── Allocation counts ───────────────────────────────────────────────────

    /// An empty array is one block: the 24-byte struct. With no capacity there
    /// is no element buffer to allocate.
    #[test]
    fn empty_typed_array_allocates_one_block() {
        reset_counts();
        let arr = TypedArray::<f64>::new();
        assert_eq!(counts().allocs, 1, "struct only — no buffer at cap 0");
        unsafe { TypedArray::drop_array(arr) };
        assert_eq!(counts().deallocs, 1);
        assert_eq!(counts().live(), 0);
    }

    /// An array WITH capacity is two blocks: the struct and a separate element
    /// buffer. This is the double allocation #194 set out to collapse; the
    /// count is asserted here so that if the collapse lands, it lands as a
    /// deliberate edit to this expectation and not as an unnoticed drift.
    #[test]
    fn typed_array_with_capacity_allocates_struct_plus_buffer() {
        reset_counts();
        let arr = TypedArray::<f64>::with_capacity(8);
        assert_eq!(
            counts().allocs,
            2,
            "header and element buffer are separate allocations"
        );
        unsafe { TypedArray::drop_array(arr) };
        assert_eq!(counts().deallocs, 2, "both blocks are freed");
        assert_eq!(counts().live(), 0);
    }

    #[test]
    fn from_slice_allocates_struct_plus_buffer() {
        reset_counts();
        let arr = TypedArray::<f64>::from_slice(&[1.0, 2.0, 3.0]);
        assert_eq!(counts().allocs, 2);
        assert_eq!(unsafe { TypedArray::len(arr) }, 3);
        unsafe { TypedArray::drop_array(arr) };
        assert_eq!(counts().live(), 0);
    }

    /// Growth from capacity 0 allocates a fresh buffer; growth from a non-zero
    /// capacity reallocs in place. The doubling schedule (0 → 4 → 8 → 16)
    /// means the number of growths for N pushes is fixed, so the counts below
    /// are exact rather than bounds.
    #[test]
    fn push_growth_schedule_is_pinned() {
        reset_counts();
        let arr = TypedArray::<f64>::new();
        assert_eq!(counts().allocs, 1, "struct");

        // First push: cap 0 → 4, a fresh buffer allocation.
        unsafe { TypedArray::push(arr, 1.0) };
        assert_eq!(counts().allocs, 2, "first growth allocates the buffer");
        assert_eq!(counts().reallocs, 0);

        // Pushes 2..=4 fit in the capacity already granted.
        for i in 2..=4 {
            unsafe { TypedArray::push(arr, i as f64) };
        }
        assert_eq!(counts().allocs, 2, "no allocation while capacity remains");
        assert_eq!(counts().reallocs, 0);

        // Push 5 doubles 4 → 8 via realloc.
        unsafe { TypedArray::push(arr, 5.0) };
        assert_eq!(counts().reallocs, 1, "growth past capacity reallocs");

        // Pushes 6..=8 fit; push 9 doubles 8 → 16.
        for i in 6..=8 {
            unsafe { TypedArray::push(arr, i as f64) };
        }
        assert_eq!(counts().reallocs, 1);
        unsafe { TypedArray::push(arr, 9.0) };
        assert_eq!(counts().reallocs, 2);

        assert_eq!(unsafe { TypedArray::len(arr) }, 9);
        assert_eq!(unsafe { TypedArray::capacity(arr) }, 16);
        unsafe { TypedArray::drop_array(arr) };
        assert_eq!(counts().live(), 0, "struct + buffer both retired");
    }

    /// A non-empty string is two blocks (struct + byte buffer); an empty one
    /// is a single block, because there are no bytes to hold.
    #[test]
    fn string_obj_allocation_counts() {
        reset_counts();
        let s = StringObj::new("hello");
        assert_eq!(counts().allocs, 2, "struct + byte buffer");
        assert_eq!(unsafe { StringObj::as_str(s) }, "hello");
        unsafe { StringObj::drop(s) };
        assert_eq!(counts().deallocs, 2);

        reset_counts();
        let empty = StringObj::new("");
        assert_eq!(counts().allocs, 1, "empty string allocates no buffer");
        unsafe { StringObj::drop(empty) };
        assert_eq!(counts().live(), 0);
    }

    /// A decimal is a single block — the value is stored inline, with no
    /// nested allocation.
    #[test]
    fn decimal_obj_allocation_counts() {
        reset_counts();
        let d = DecimalObj::new(rust_decimal::Decimal::new(1234, 2));
        assert_eq!(counts().allocs, 1);
        unsafe { DecimalObj::drop(d) };
        assert_eq!(counts().deallocs, 1);
        assert_eq!(counts().live(), 0);
    }

    /// Allocating and freeing many transient arrays leaves nothing live. This
    /// is the shape a leak would break: the counts are cumulative, so an
    /// unfreed block per iteration shows up as a non-zero `live()`.
    #[test]
    fn transient_arrays_leave_nothing_live() {
        reset_counts();
        for _ in 0..100 {
            let arr = TypedArray::<i64>::from_slice(&[1, 2, 3, 4]);
            unsafe { TypedArray::drop_array(arr) };
        }
        assert_eq!(counts().allocs, 200, "two blocks per array");
        assert_eq!(counts().live(), 0, "every block retired");
    }
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
