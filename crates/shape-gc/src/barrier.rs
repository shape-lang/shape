//! SATB (Snapshot At The Beginning) write barrier for incremental marking.
//!
//! During an incremental marking cycle the mutator may overwrite reference fields
//! before the marker has scanned them.  The SATB invariant says: "any object that
//! was reachable at the start of marking must be considered live."  To enforce
//! this, every time a reference field is overwritten we enqueue the **old**
//! reference into a per-thread SATB buffer.  At mark termination the GC drains
//! these buffers and marks any enqueued (white) objects, preventing them from
//! being falsely collected.

/// Thread-local SATB buffer that accumulates old reference values overwritten
/// during an active marking phase.
pub struct SatbBuffer {
    /// Accumulated old references that were overwritten while marking was active.
    buffer: Vec<*mut u8>,
    /// Maximum entries before the buffer should be flushed to the marker.
    capacity: usize,
}

// Safety: SatbBuffer is owned by a single mutator thread and only handed to
// the marker during STW termination.
unsafe impl Send for SatbBuffer {}

impl SatbBuffer {
    /// Create a new SATB buffer with the given flush capacity.
    pub fn new(capacity: usize) -> Self {
        Self {
            buffer: Vec::with_capacity(capacity),
            capacity,
        }
    }

    /// Enqueue an old reference that was just overwritten.
    ///
    /// This is the core write-barrier operation.  It must be called **before**
    /// the store overwrites the old pointer value.
    #[inline(always)]
    pub fn enqueue(&mut self, old_ref: *mut u8) {
        if !old_ref.is_null() {
            self.buffer.push(old_ref);
        }
    }

    /// Drain all enqueued references, returning them as a `Vec`.
    ///
    /// After draining, the buffer is empty and ready for new entries.
    pub fn drain(&mut self) -> Vec<*mut u8> {
        std::mem::take(&mut self.buffer)
    }

    /// Check whether the buffer is empty.
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// Number of enqueued entries.
    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    /// Whether the buffer has reached its flush capacity.
    #[inline(always)]
    pub fn should_flush(&self) -> bool {
        self.buffer.len() >= self.capacity
    }
}

impl Default for SatbBuffer {
    fn default() -> Self {
        Self::new(256)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_buffer_is_empty() {
        let buf = SatbBuffer::new(64);
        assert!(buf.is_empty());
        assert_eq!(buf.len(), 0);
    }

    #[test]
    fn test_enqueue_and_drain() {
        let mut buf = SatbBuffer::new(64);
        let a = 0x1000 as *mut u8;
        let b = 0x2000 as *mut u8;
        buf.enqueue(a);
        buf.enqueue(b);
        assert_eq!(buf.len(), 2);
        assert!(!buf.is_empty());

        let drained = buf.drain();
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0], a);
        assert_eq!(drained[1], b);
        assert!(buf.is_empty());
    }

    #[test]
    fn test_null_enqueue_ignored() {
        let mut buf = SatbBuffer::new(64);
        buf.enqueue(std::ptr::null_mut());
        assert!(buf.is_empty());
    }

    #[test]
    fn test_should_flush() {
        let mut buf = SatbBuffer::new(2);
        assert!(!buf.should_flush());
        buf.enqueue(0x1000 as *mut u8);
        assert!(!buf.should_flush());
        buf.enqueue(0x2000 as *mut u8);
        assert!(buf.should_flush());
    }

    #[test]
    fn test_drain_resets_buffer() {
        let mut buf = SatbBuffer::new(64);
        buf.enqueue(0x1000 as *mut u8);
        buf.enqueue(0x2000 as *mut u8);
        let _ = buf.drain();
        assert!(buf.is_empty());
        assert_eq!(buf.len(), 0);
        // Can enqueue again after drain
        buf.enqueue(0x3000 as *mut u8);
        assert_eq!(buf.len(), 1);
    }
}
