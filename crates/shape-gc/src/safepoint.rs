//! Safepoint coordination between mutator threads and GC.
//!
//! The GC sets a flag when it needs to collect. Mutator threads poll this flag
//! at safepoints (loop back-edges, function returns, interrupt checks) and
//! cooperate by scanning their roots and waiting for the GC to complete.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex};

/// Shared safepoint state.
pub struct SafepointState {
    /// Set to true by the GC thread when a collection is requested.
    gc_requested: AtomicBool,
    /// Set to true while the GC is actively running.
    gc_active: AtomicBool,
    /// Mutex + condvar for threads waiting on GC completion.
    gc_done: Mutex<bool>,
    gc_done_cv: Condvar,
}

impl SafepointState {
    pub fn new() -> Self {
        Self {
            gc_requested: AtomicBool::new(false),
            gc_active: AtomicBool::new(false),
            gc_done: Mutex::new(false),
            gc_done_cv: Condvar::new(),
        }
    }

    /// Request a GC collection. Called by the allocator when threshold is exceeded.
    pub fn request_gc(&self) {
        self.gc_requested.store(true, Ordering::Release);
    }

    /// Check if a GC has been requested. Used in the safepoint poll fast path.
    #[inline(always)]
    pub fn is_gc_requested(&self) -> bool {
        self.gc_requested.load(Ordering::Acquire)
    }

    /// Signal that the GC is now actively collecting.
    pub fn gc_begin(&self) {
        self.gc_active.store(true, Ordering::Release);
        let mut done = self.gc_done.lock().unwrap();
        *done = false;
    }

    /// Signal that the GC has completed.
    pub fn gc_end(&self) {
        self.gc_requested.store(false, Ordering::Release);
        self.gc_active.store(false, Ordering::Release);
        let mut done = self.gc_done.lock().unwrap();
        *done = true;
        self.gc_done_cv.notify_all();
    }

    /// Wait for the GC to complete. Called by mutator threads at safepoints.
    pub fn wait_for_gc(&self) {
        if !self.gc_active.load(Ordering::Acquire) {
            return;
        }
        let mut done = self.gc_done.lock().unwrap();
        while !*done {
            done = self.gc_done_cv.wait(done).unwrap();
        }
    }

    /// Check if the GC is actively collecting.
    pub fn is_gc_active(&self) -> bool {
        self.gc_active.load(Ordering::Acquire)
    }
}

impl Default for SafepointState {
    fn default() -> Self {
        Self::new()
    }
}

/// Poll the safepoint. This is the function called at every safepoint
/// (interrupt checks, loop headers, function returns).
///
/// Fast path: single atomic load. Only enters slow path if GC is requested.
#[inline(always)]
pub fn safepoint_poll(state: &SafepointState) {
    if state.is_gc_requested() {
        safepoint_slow_path(state);
    }
}

/// Slow path: the GC has been requested. Wait for it to complete.
#[cold]
fn safepoint_slow_path(state: &SafepointState) {
    state.wait_for_gc();
}

/// FFI-callable safepoint poll for JIT-compiled code.
///
/// # Safety
/// `state_ptr` must be a valid pointer to a SafepointState.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_safepoint_poll(state_ptr: *const SafepointState) {
    if state_ptr.is_null() {
        return;
    }
    let state = unsafe { &*state_ptr };
    safepoint_poll(state);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_safepoint_not_requested() {
        let state = SafepointState::new();
        assert!(!state.is_gc_requested());
        // Should return immediately
        safepoint_poll(&state);
    }

    #[test]
    fn test_safepoint_request_and_complete() {
        let state = SafepointState::new();
        state.request_gc();
        assert!(state.is_gc_requested());

        state.gc_begin();
        assert!(state.is_gc_active());

        state.gc_end();
        assert!(!state.is_gc_requested());
        assert!(!state.is_gc_active());

        // Poll should return immediately now
        safepoint_poll(&state);
    }
}
