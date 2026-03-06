//! Tri-color mark phase for the GC.
//!
//! Colors:
//! - **White**: Unmarked, potentially reclaimable after marking completes.
//! - **Gray**: Reachable, but children not yet scanned.
//! - **Black**: Reachable, all children scanned.
//!
//! Algorithm:
//! 1. All objects start white.
//! 2. Root objects are colored gray and pushed to the worklist.
//! 3. Process gray objects: for each, trace children (color white->gray), color self black.
//! 4. After worklist is empty, all white objects are garbage.
//!
//! ## Incremental marking
//!
//! Instead of processing the entire gray worklist in one STW pause, the marker
//! exposes `mark_step(budget)` which processes a bounded number of objects per
//! mutator safepoint.  An SATB write barrier (see `barrier.rs`) records
//! references overwritten by the mutator during marking.  Mark termination is a
//! short STW pause that drains the SATB buffers and re-processes any new grays
//! until convergence.

use crate::barrier::SatbBuffer;
use crate::header::{GcColor, GcHeader};
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};

/// Marker phase state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkPhase {
    /// No marking cycle is active.
    Idle,
    /// Incremental marking is in progress (gray objects remain).
    Marking,
    /// Mark termination: draining SATB buffers and finalizing.
    Terminating,
    /// Marking complete; ready for sweep.
    Complete,
}

/// The marker manages the gray worklist and tri-color state.
pub struct Marker {
    /// Gray set (objects reachable but children not scanned).
    worklist: Vec<*mut u8>,
    /// Set of all objects marked live (for sweep to check).
    live_set: HashSet<usize>,
    /// Whether a marking cycle is currently active.
    ///
    /// This is an `AtomicBool` so that the write barrier can check it without
    /// requiring a mutable reference to the Marker.
    is_marking: AtomicBool,
    /// Current phase of the marking cycle.
    phase: MarkPhase,
}

// Safety: Marker is only used during stop-the-world GC.
unsafe impl Send for Marker {}

impl Marker {
    pub fn new() -> Self {
        Self {
            worklist: Vec::with_capacity(1024),
            live_set: HashSet::with_capacity(4096),
            is_marking: AtomicBool::new(false),
            phase: MarkPhase::Idle,
        }
    }

    /// Reset the marker for a new GC cycle.
    pub fn reset(&mut self) {
        self.worklist.clear();
        self.live_set.clear();
        self.is_marking.store(false, Ordering::Release);
        self.phase = MarkPhase::Idle;
    }

    // ── Phase management ────────────────────────────────────────────────

    /// Begin a new incremental marking cycle.
    ///
    /// After calling this, the mutator should activate its SATB write barrier
    /// (checking `is_marking()`).
    pub fn start_marking(&mut self) {
        self.worklist.clear();
        self.live_set.clear();
        self.is_marking.store(true, Ordering::Release);
        self.phase = MarkPhase::Marking;
    }

    /// Finish the marking cycle (called after termination + sweep).
    pub fn finish_marking(&mut self) {
        self.is_marking.store(false, Ordering::Release);
        self.phase = MarkPhase::Idle;
    }

    /// Check whether a marking cycle is active.
    ///
    /// This is the fast-path check used by the SATB write barrier.
    #[inline(always)]
    pub fn is_marking(&self) -> bool {
        self.is_marking.load(Ordering::Acquire)
    }

    /// Get the current mark phase.
    pub fn phase(&self) -> MarkPhase {
        self.phase
    }

    // ── Root and child marking ──────────────────────────────────────────

    /// Mark a root pointer: color it gray and add to worklist.
    pub fn mark_root(&mut self, ptr: *mut u8) {
        if ptr.is_null() {
            return;
        }
        let header = Self::header_of(ptr);
        if header.color() == GcColor::White {
            header.set_color(GcColor::Gray);
            self.worklist.push(ptr);
            self.live_set.insert(ptr as usize);
        }
    }

    /// Mark a pointer as gray (add to worklist) if it is currently white.
    ///
    /// Used during SATB termination to re-gray old references.
    pub fn mark_gray(&mut self, ptr: *mut u8) {
        if ptr.is_null() {
            return;
        }
        // Only add to live_set / worklist if not already known-live.
        if self.live_set.insert(ptr as usize) {
            let header = Self::header_of(ptr);
            header.set_color(GcColor::Gray);
            self.worklist.push(ptr);
        } else {
            // Already in live set — but if it was somehow reset to white
            // (shouldn't normally happen), re-gray it.
            let header = Self::header_of(ptr);
            if header.color() == GcColor::White {
                header.set_color(GcColor::Gray);
                self.worklist.push(ptr);
            }
        }
    }

    /// Mark a child pointer discovered during tracing.
    /// If it's white, color it gray and add to the worklist.
    pub fn mark_child(&mut self, ptr: *mut u8) {
        if ptr.is_null() {
            return;
        }
        let header = Self::header_of(ptr);
        if header.color() == GcColor::White {
            header.set_color(GcColor::Gray);
            self.worklist.push(ptr);
            self.live_set.insert(ptr as usize);
        }
    }

    // ── Incremental mark step ───────────────────────────────────────────

    /// Process up to `budget` gray objects.
    ///
    /// Returns `true` if the worklist is empty (marking phase complete for
    /// this step), `false` if more work remains.
    ///
    /// Each processed object is colored black.  In a full implementation the
    /// `trace_object` callback would push the object's children as gray via
    /// `mark_child`; for now objects are treated as opaque blobs and simply
    /// colored black.
    pub fn mark_step(&mut self, budget: usize) -> bool {
        let mut processed = 0;
        while processed < budget {
            let Some(ptr) = self.worklist.pop() else {
                return true;
            };

            let header = Self::header_of(ptr);

            // Already fully scanned
            if header.color() == GcColor::Black {
                continue;
            }

            // Trace children — for now, objects are opaque blobs.
            // The real tracing happens via the Trace trait implementations
            // which call mark_child() for each inner pointer.
            // Here we just color self black.
            header.set_color(GcColor::Black);
            processed += 1;
        }
        self.worklist.is_empty()
    }

    /// Process all remaining gray objects until the worklist is empty.
    pub fn mark_all(&mut self) {
        while !self.mark_step(256) {}
    }

    // ── Mark termination (short STW) ────────────────────────────────────

    /// Terminate the marking cycle by draining the SATB buffer and processing
    /// any remaining gray objects.
    ///
    /// This should be called in a short STW pause after incremental marking
    /// reports the worklist as empty.
    ///
    /// Returns `true` if marking is truly complete (gray set empty AND SATB
    /// buffer empty), `false` if new work was discovered (caller should loop).
    pub fn terminate_marking(&mut self, satb: &mut SatbBuffer) -> bool {
        self.phase = MarkPhase::Terminating;

        // Drain SATB buffer — any old reference that was overwritten must be
        // treated as potentially live.
        for ptr in satb.drain() {
            if !self.is_marked(ptr) {
                self.mark_gray(ptr);
            }
        }

        // Process all remaining grays (those from SATB + any stragglers).
        while let Some(obj) = self.worklist.pop() {
            let header = Self::header_of(obj);
            if header.color() != GcColor::Black {
                header.set_color(GcColor::Black);
            }
            // In a full implementation: trace_object(obj) pushes children.
        }

        let terminated = self.worklist.is_empty() && satb.is_empty();
        if terminated {
            self.phase = MarkPhase::Complete;
        }
        terminated
    }

    // ── Query helpers ───────────────────────────────────────────────────

    /// Check if a pointer was marked live in this cycle.
    pub fn is_marked(&self, ptr: *const u8) -> bool {
        self.live_set.contains(&(ptr as usize))
    }

    /// Alias for `is_marked` — consistent naming with the public API.
    pub fn is_live(&self, ptr: *const u8) -> bool {
        self.is_marked(ptr)
    }

    /// Get the number of objects currently in the gray worklist.
    pub fn gray_count(&self) -> usize {
        self.worklist.len()
    }

    /// Get the number of objects marked live.
    pub fn live_count(&self) -> usize {
        self.live_set.len()
    }

    /// Clear the mark bit (reset to White) for a specific pointer.
    ///
    /// Used during sweep to prepare objects for the next cycle.
    pub fn clear_mark(&self, ptr: *mut u8) {
        let header = Self::header_of(ptr);
        header.set_color(GcColor::White);
    }

    // ── Internal helpers ────────────────────────────────────────────────

    /// Get a mutable reference to the GcHeader before an object pointer.
    ///
    /// # Safety
    /// `ptr` must be a valid GC-managed object pointer (preceded by GcHeader).
    fn header_of(ptr: *mut u8) -> &'static mut GcHeader {
        unsafe {
            let header_ptr = ptr.sub(std::mem::size_of::<GcHeader>()) as *mut GcHeader;
            &mut *header_ptr
        }
    }
}

impl Default for Marker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header::GcHeader;

    fn make_fake_object() -> Vec<u8> {
        let mut buf = vec![0u8; 16]; // 8 header + 8 data
        let header_ptr = buf.as_mut_ptr() as *mut GcHeader;
        unsafe {
            header_ptr.write(GcHeader::new(0, 8));
        }
        buf
    }

    #[test]
    fn test_mark_root_colors_gray() {
        let mut buf = make_fake_object();
        let obj_ptr = unsafe { buf.as_mut_ptr().add(8) };

        let mut marker = Marker::new();
        marker.mark_root(obj_ptr);

        let header = unsafe { &*(buf.as_ptr() as *const GcHeader) };
        assert_eq!(header.color(), GcColor::Gray);
        assert_eq!(marker.gray_count(), 1);
    }

    #[test]
    fn test_mark_step_colors_black() {
        let mut buf = make_fake_object();
        let obj_ptr = unsafe { buf.as_mut_ptr().add(8) };

        let mut marker = Marker::new();
        marker.mark_root(obj_ptr);
        let done = marker.mark_step(10);
        assert!(done);

        let header = unsafe { &*(buf.as_ptr() as *const GcHeader) };
        assert_eq!(header.color(), GcColor::Black);
        assert_eq!(marker.gray_count(), 0);
    }

    #[test]
    fn test_null_root_ignored() {
        let mut marker = Marker::new();
        marker.mark_root(std::ptr::null_mut());
        assert_eq!(marker.gray_count(), 0);
    }

    #[test]
    fn test_is_marking_flag() {
        let mut marker = Marker::new();
        assert!(!marker.is_marking());

        marker.start_marking();
        assert!(marker.is_marking());
        assert_eq!(marker.phase(), MarkPhase::Marking);

        marker.finish_marking();
        assert!(!marker.is_marking());
        assert_eq!(marker.phase(), MarkPhase::Idle);
    }

    #[test]
    fn test_mark_gray_adds_to_worklist() {
        let mut buf = make_fake_object();
        let obj_ptr = unsafe { buf.as_mut_ptr().add(8) };

        let mut marker = Marker::new();
        marker.start_marking();
        marker.mark_gray(obj_ptr);

        assert_eq!(marker.gray_count(), 1);
        assert!(marker.is_marked(obj_ptr));
    }

    #[test]
    fn test_mark_gray_idempotent() {
        let mut buf = make_fake_object();
        let obj_ptr = unsafe { buf.as_mut_ptr().add(8) };

        let mut marker = Marker::new();
        marker.start_marking();
        marker.mark_gray(obj_ptr);
        marker.mark_gray(obj_ptr); // second time — should not add again

        assert_eq!(marker.live_count(), 1);
    }

    #[test]
    fn test_incremental_mark_step_budget() {
        // Create 5 fake objects
        let mut bufs: Vec<Vec<u8>> = (0..5).map(|_| make_fake_object()).collect();
        let ptrs: Vec<*mut u8> = bufs
            .iter_mut()
            .map(|b| unsafe { b.as_mut_ptr().add(8) })
            .collect();

        let mut marker = Marker::new();
        marker.start_marking();
        for &ptr in &ptrs {
            marker.mark_root(ptr);
        }
        assert_eq!(marker.gray_count(), 5);

        // Process 2 out of 5
        let done = marker.mark_step(2);
        assert!(!done);
        // Should have 3 remaining
        assert_eq!(marker.gray_count(), 3);

        // Process remaining
        let done = marker.mark_step(10);
        assert!(done);
        assert_eq!(marker.gray_count(), 0);
    }

    #[test]
    fn test_terminate_marking_empty_satb() {
        let mut buf = make_fake_object();
        let obj_ptr = unsafe { buf.as_mut_ptr().add(8) };

        let mut marker = Marker::new();
        marker.start_marking();
        marker.mark_root(obj_ptr);
        marker.mark_all();

        let mut satb = SatbBuffer::new(64);
        let terminated = marker.terminate_marking(&mut satb);
        assert!(terminated);
        assert_eq!(marker.phase(), MarkPhase::Complete);
    }

    #[test]
    fn test_terminate_marking_with_satb_entries() {
        let mut buf1 = make_fake_object();
        let mut buf2 = make_fake_object();
        let ptr1 = unsafe { buf1.as_mut_ptr().add(8) };
        let ptr2 = unsafe { buf2.as_mut_ptr().add(8) };

        let mut marker = Marker::new();
        marker.start_marking();
        marker.mark_root(ptr1);
        marker.mark_all();

        // Simulate: mutator overwrote a reference to ptr2 during marking
        let mut satb = SatbBuffer::new(64);
        satb.enqueue(ptr2);

        let terminated = marker.terminate_marking(&mut satb);
        assert!(terminated);
        // ptr2 should now be marked live (saved by SATB)
        assert!(marker.is_marked(ptr2));
    }

    #[test]
    fn test_clear_mark() {
        let mut buf = make_fake_object();
        let obj_ptr = unsafe { buf.as_mut_ptr().add(8) };

        let mut marker = Marker::new();
        marker.mark_root(obj_ptr);
        marker.mark_all();

        let header = unsafe { &*(buf.as_ptr() as *const GcHeader) };
        assert_eq!(header.color(), GcColor::Black);

        marker.clear_mark(obj_ptr);
        assert_eq!(header.color(), GcColor::White);
    }

    #[test]
    fn test_full_incremental_cycle() {
        // Simulate: start marking -> incremental steps -> termination -> verify
        let mut bufs: Vec<Vec<u8>> = (0..10).map(|_| make_fake_object()).collect();
        let ptrs: Vec<*mut u8> = bufs
            .iter_mut()
            .map(|b| unsafe { b.as_mut_ptr().add(8) })
            .collect();

        let mut marker = Marker::new();
        marker.start_marking();

        // Root scanning
        for &ptr in &ptrs {
            marker.mark_root(ptr);
        }

        // Incremental steps with budget 3
        let mut steps = 0;
        while !marker.mark_step(3) {
            steps += 1;
        }
        // Should have taken multiple steps
        assert!(steps >= 1);

        // Termination
        let mut satb = SatbBuffer::new(64);
        let terminated = marker.terminate_marking(&mut satb);
        assert!(terminated);
        assert_eq!(marker.phase(), MarkPhase::Complete);

        // All objects should be marked live
        for &ptr in &ptrs {
            assert!(marker.is_marked(ptr));
        }

        marker.finish_marking();
        assert!(!marker.is_marking());
        assert_eq!(marker.phase(), MarkPhase::Idle);
    }
}
