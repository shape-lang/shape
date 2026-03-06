//! GC object header — 8 bytes prepended to every GC-managed allocation.
//!
//! Layout (8 bytes total):
//! ```text
//! Byte 0: [color:2][gen:1][forwarded:1][unused:4]
//! Byte 1: kind (HeapKind discriminant)
//! Bytes 2-3: unused (reserved)
//! Bytes 4-7: size (u32, object size in bytes excluding header)
//! ```

/// GC tri-color for mark phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum GcColor {
    /// Unmarked — reclaimable if still white after marking.
    White = 0,
    /// Reachable but children not yet scanned.
    Gray = 1,
    /// Reachable and all children scanned.
    Black = 2,
}

/// Generation identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Generation {
    Young = 0,
    Old = 1,
}

/// 8-byte header prepended to every GC-managed object.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct GcHeader {
    /// Packed flags: [color:2][gen:1][forwarded:1][unused:4]
    flags: u8,
    /// HeapKind discriminant for type identification during tracing.
    pub kind: u8,
    /// Reserved for future use.
    _reserved: u16,
    /// Object size in bytes (excluding this header).
    pub size: u32,
}

impl GcHeader {
    /// Create a new header for a young-generation white object.
    pub fn new(kind: u8, size: u32) -> Self {
        Self {
            flags: 0, // white, young, not forwarded
            kind,
            _reserved: 0,
            size,
        }
    }

    /// Get the object's color.
    #[inline(always)]
    pub fn color(&self) -> GcColor {
        match self.flags & 0b11 {
            0 => GcColor::White,
            1 => GcColor::Gray,
            2 => GcColor::Black,
            _ => GcColor::White, // shouldn't happen
        }
    }

    /// Set the object's color.
    #[inline(always)]
    pub fn set_color(&mut self, color: GcColor) {
        self.flags = (self.flags & !0b11) | (color as u8);
    }

    /// Get the generation.
    #[inline(always)]
    pub fn generation(&self) -> Generation {
        if (self.flags >> 2) & 1 == 0 {
            Generation::Young
        } else {
            Generation::Old
        }
    }

    /// Set the generation.
    #[inline(always)]
    pub fn set_generation(&mut self, generation: Generation) {
        self.flags = (self.flags & !0b100) | ((generation as u8) << 2);
    }

    /// Check if this object has been forwarded (relocated).
    #[inline(always)]
    pub fn is_forwarded(&self) -> bool {
        (self.flags >> 3) & 1 != 0
    }

    /// Mark this object as forwarded.
    #[inline(always)]
    pub fn set_forwarded(&mut self, forwarded: bool) {
        if forwarded {
            self.flags |= 0b1000;
        } else {
            self.flags &= !0b1000;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_header_size() {
        assert_eq!(std::mem::size_of::<GcHeader>(), 8);
    }

    #[test]
    fn test_color_roundtrip() {
        let mut h = GcHeader::new(0, 64);
        assert_eq!(h.color(), GcColor::White);

        h.set_color(GcColor::Gray);
        assert_eq!(h.color(), GcColor::Gray);

        h.set_color(GcColor::Black);
        assert_eq!(h.color(), GcColor::Black);

        h.set_color(GcColor::White);
        assert_eq!(h.color(), GcColor::White);
    }

    #[test]
    fn test_generation_roundtrip() {
        let mut h = GcHeader::new(0, 64);
        assert_eq!(h.generation(), Generation::Young);

        h.set_generation(Generation::Old);
        assert_eq!(h.generation(), Generation::Old);

        // Color should be preserved
        h.set_color(GcColor::Gray);
        assert_eq!(h.generation(), Generation::Old);
        assert_eq!(h.color(), GcColor::Gray);
    }

    #[test]
    fn test_forwarded_flag() {
        let mut h = GcHeader::new(0, 64);
        assert!(!h.is_forwarded());

        h.set_forwarded(true);
        assert!(h.is_forwarded());
        // Other flags preserved
        assert_eq!(h.color(), GcColor::White);
        assert_eq!(h.generation(), Generation::Young);

        h.set_forwarded(false);
        assert!(!h.is_forwarded());
    }
}
