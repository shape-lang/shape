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
}

/// Install a finite per-buffer byte ceiling for the current thread. Pass
/// `None` to clear (unlimited). Returns the previous ceiling so callers can
/// restore it (nested executions).
pub fn set_ceiling(bytes: Option<u64>) -> Option<u64> {
    BUFFER_CEILING.with(|c| c.replace(bytes))
}

/// Current per-buffer ceiling (for diagnostics / tests). `None` = unlimited.
pub fn ceiling() -> Option<u64> {
    BUFFER_CEILING.with(|c| c.get())
}

/// Check whether a single buffer of `new_size_bytes` is permitted. Returns
/// `Ok(())` when there is no ceiling (unlimited) or the buffer fits; returns
/// `Err` when it would exceed the ceiling. Pure — mutates no state — so it is
/// stable across retries and needs no matching credit on free.
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
}
