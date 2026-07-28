//! Thread-local per-buffer heap-growth ceiling for bounded VM execution.
//!
//! The Shape VM proves and stores values as raw native buffers (`TypedArray`,
//! etc.) that grow via a doubling `realloc`. Because byte size grows
//! geometrically while the instruction count grows only linearly, an
//! instruction cap alone cannot bound the RSS of an allocation-heavy runaway
//! loop — a single doubling `realloc` can jump several gigabytes in ONE
//! instruction. To make the canonical runaway (an unbounded `arr.push(...)` in
//! a non-terminating loop — one buffer growing without bound) fail *in-process*
//! rather than climbing until the host OOM-killer reaps the whole process at
//! tens of GB, the VM installs a per-execution byte ceiling here and the
//! low-level growth paths consult it before each allocation.
//!
//! ## Model: a ceiling on any single buffer, not a cumulative budget
//!
//! [`check_size`] tests the *absolute* new size of one buffer against the
//! ceiling. It does NOT decrement a running total, so it never needs a
//! matching credit on free — which means it cannot false-positive on a
//! legitimate loop that allocates and frees many transient buffers (their
//! sizes are checked independently and never accumulate). The unbounded
//! single-growing-buffer runaway is bounded exactly: that buffer's size climbs
//! geometrically and trips the ceiling. This caps resource consumption only; a
//! program whose every buffer stays under the ceiling allocates exactly as
//! before and produces an identical result. `None` (the default) = unlimited,
//! preserving trusted CLI execution.

use std::cell::Cell;

thread_local! {
    /// Maximum bytes any single heap buffer may occupy on the current
    /// thread's VM execution. `None` = unlimited.
    static BUFFER_CEILING: Cell<Option<u64>> = const { Cell::new(None) };

    /// A pending memory-ceiling breach recorded by a low-level growth path
    /// (`TypedArray::grow`) that has no fallible return channel of its own
    /// (it is called from `extern "C"` JIT trampolines and dozens of
    /// infallible `push` sites). Instead of aborting the whole process with a
    /// `panic!` — which on a serve node would kill every in-flight request —
    /// the growth path RECORDS the breach here and refuses to grow (so the
    /// offending buffer is bounded exactly at the ceiling), and the VM
    /// surfaces it as a clean `VMError` at the next dispatch-loop safepoint /
    /// the post-run boundary. Cleared at the start of each execution by
    /// [`set_ceiling`] so a breach never leaks across runs.
    static PENDING_BREACH: Cell<Option<AllocBudgetExceeded>> = const { Cell::new(None) };
}

/// Install a finite per-buffer byte ceiling for the current thread. Pass
/// `None` to clear (unlimited). Returns the previous ceiling so callers can
/// restore it (nested executions). Also clears any pending breach so a
/// prior execution's recorded breach never leaks into this one.
pub fn set_ceiling(bytes: Option<u64>) -> Option<u64> {
    PENDING_BREACH.with(|c| c.set(None));
    BUFFER_CEILING.with(|c| c.replace(bytes))
}

/// Record a memory-ceiling breach detected on an infallible growth path.
/// Keeps the FIRST breach if one is already pending (that is the one closest
/// to the cause). Idempotent and cheap.
///
/// `#[cold]` + `#[inline]`: reached only on a ceiling breach, which ends the
/// execution, so keeping it off the allocation fast path's straight-line code
/// is worth more than the call it saves.
#[cold]
#[inline]
pub fn record_breach(e: AllocBudgetExceeded) {
    PENDING_BREACH.with(|c| {
        if c.get().is_none() {
            c.set(Some(e));
        }
    });
}

/// Take (clear + return) any pending memory-ceiling breach recorded on this
/// thread. The VM calls this at dispatch-loop safepoints and at the post-run
/// boundary to convert a recorded breach into a clean surfaced `VMError`
/// (never a panic). `None` = no breach pending.
pub fn take_breach() -> Option<AllocBudgetExceeded> {
    PENDING_BREACH.with(|c| c.take())
}

/// Current per-buffer ceiling (for diagnostics / tests). `None` = unlimited.
pub fn ceiling() -> Option<u64> {
    BUFFER_CEILING.with(|c| c.get())
}

/// Check whether a single buffer of `new_size_bytes` is permitted. Returns
/// `Ok(())` when there is no ceiling (unlimited) or the buffer fits; returns
/// `Err` when it would exceed the ceiling. Pure — mutates no state — so it is
/// stable across retries and needs no matching credit on free.
///
/// `#[inline]` because since #194 this runs on EVERY typed-carrier allocation
/// rather than only on `TypedArray::grow`, and its callers live in other
/// crates: without the attribute each allocation pays an un-inlinable
/// cross-crate call to reach a single thread-local load.
///
/// Honesty about the evidence: the attribute is justified on first principles,
/// NOT by measurement. #194 does carry a reproducible ~11% regression on the
/// charter's `startup_hello` workload (nearly pure allocation, so a constant
/// per-allocation cost shows at its highest proportion there), but adding this
/// attribute did not measurably reduce it — the machine was too contended to
/// resolve a few percent. Do not cite `#[inline]` here as a fix for that
/// regression; its cause is still open.
#[inline]
pub fn check_size(new_size_bytes: u64) -> Result<(), AllocBudgetExceeded> {
    BUFFER_CEILING.with(|c| match c.get() {
        None => Ok(()),
        Some(ceiling) => {
            if new_size_bytes > ceiling {
                Err(AllocBudgetExceeded {
                    requested: new_size_bytes,
                    ceiling,
                })
            } else {
                Ok(())
            }
        }
    })
}

/// Error returned by [`check_size`] when a buffer would exceed the ceiling.
#[derive(Debug, Clone, Copy)]
pub struct AllocBudgetExceeded {
    pub requested: u64,
    pub ceiling: u64,
}

impl std::fmt::Display for AllocBudgetExceeded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Shape memory limit exceeded: a single heap buffer reached {} bytes, \
             over the {}-byte per-execution ceiling (likely an unbounded \
             allocating loop)",
            self.requested, self.ceiling
        )
    }
}

/// RAII guard that installs a ceiling on construction and restores the prior
/// ceiling on drop. Use to scope a ceiling to a single VM execution.
pub struct BudgetGuard {
    prev: Option<u64>,
}

impl BudgetGuard {
    pub fn new(bytes: Option<u64>) -> Self {
        let prev = set_ceiling(bytes);
        Self { prev }
    }
}

impl Drop for BudgetGuard {
    fn drop(&mut self) {
        set_ceiling(self.prev);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unlimited_by_default() {
        let _g = BudgetGuard::new(None);
        assert!(check_size(1 << 40).is_ok());
    }

    #[test]
    fn ceiling_bounds_single_buffer() {
        let _g = BudgetGuard::new(Some(100));
        assert!(check_size(100).is_ok()); // at the ceiling is allowed
        assert!(check_size(101).is_err()); // over the ceiling fails
    }

    #[test]
    fn transient_buffers_do_not_accumulate() {
        // Many independent buffers each under the ceiling are all permitted —
        // sizes are checked independently, never summed. This is what keeps a
        // legitimate allocate/free loop from false-tripping.
        let _g = BudgetGuard::new(Some(100));
        for _ in 0..1000 {
            assert!(check_size(80).is_ok());
        }
    }

    #[test]
    fn guard_restores_prior_ceiling() {
        let _outer = BudgetGuard::new(Some(1000));
        {
            let _inner = BudgetGuard::new(Some(10));
            assert_eq!(ceiling(), Some(10));
        }
        assert_eq!(ceiling(), Some(1000));
    }

    #[test]
    fn breach_record_and_take_roundtrip() {
        let _g = BudgetGuard::new(Some(100));
        assert!(take_breach().is_none(), "no breach pending initially");
        record_breach(AllocBudgetExceeded {
            requested: 200,
            ceiling: 100,
        });
        let taken = take_breach().expect("breach must be pending after record");
        assert_eq!(taken.requested, 200);
        assert!(take_breach().is_none(), "take clears the pending breach");
    }

    #[test]
    fn record_keeps_first_breach() {
        let _g = BudgetGuard::new(Some(100));
        record_breach(AllocBudgetExceeded {
            requested: 200,
            ceiling: 100,
        });
        record_breach(AllocBudgetExceeded {
            requested: 999,
            ceiling: 100,
        });
        // The first breach (closest to the cause) is retained.
        assert_eq!(take_breach().unwrap().requested, 200);
    }

    #[test]
    fn set_ceiling_clears_stale_breach() {
        // Models a serve worker: a breach recorded on request N must not leak
        // into request N+1. Installing the next run's ceiling clears it.
        let _g = BudgetGuard::new(Some(100));
        record_breach(AllocBudgetExceeded {
            requested: 200,
            ceiling: 100,
        });
        // Next request's guard install (set_ceiling) drops the stale breach.
        let _next = BudgetGuard::new(Some(100));
        assert!(
            take_breach().is_none(),
            "stale breach must not leak into the next execution"
        );
    }
}
