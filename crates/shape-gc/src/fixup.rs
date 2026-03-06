//! Pointer fixup after relocation.
//!
//! After objects are relocated and the forwarding table is populated,
//! we must update all pointers (in live objects and root set) to point
//! to the new locations.

use crate::region::Region;
use crate::relocator::ForwardingTable;

/// Fix up all pointers in the root set using the forwarding table.
///
/// `trace_roots` should iterate all root pointers. For each pointer that
/// has a forwarding entry, update it to the new address.
pub fn fixup_roots(
    forwarding: &ForwardingTable,
    trace_roots: &mut dyn FnMut(&mut dyn FnMut(*mut u8, &mut *mut u8)),
) {
    trace_roots(&mut |old_ptr, slot| {
        if let Some(new_ptr) = forwarding.lookup(old_ptr) {
            *slot = new_ptr;
        }
    });
}

/// Fix up pointers within all live objects in a region.
///
/// For each live object, trace its inner pointers and update any that
/// appear in the forwarding table.
pub fn fixup_region(region: &mut Region, _forwarding: &ForwardingTable) {
    region.for_each_object_mut(|header, _obj_ptr| {
        // Only process live objects
        if header.size == 0 {
            return;
        }

        // Walk the object's pointer fields and update forwarded ones.
        // For now, this is a placeholder — real implementation needs type-specific
        // tracing via HeapKind to know which fields are pointers.
        // The actual fixup happens via the Trace trait in the VM integration.
    });
}

/// Fix up a single raw pointer if it has a forwarding entry.
#[inline]
pub fn fixup_ptr(ptr: *mut u8, forwarding: &ForwardingTable) -> *mut u8 {
    if let Some(new) = forwarding.lookup(ptr) {
        new
    } else {
        ptr
    }
}

/// Fix up a NaN-boxed u64 value in-place if it contains a forwarded heap pointer.
#[inline]
pub fn fixup_nanboxed_bits(bits: &mut u64, forwarding: &ForwardingTable) {
    const TAG_BASE: u64 = 0xFFF8_0000_0000_0000;
    const PAYLOAD_MASK: u64 = 0x0000_FFFF_FFFF_FFFF;
    const TAG_MASK: u64 = 0x0007_0000_0000_0000;
    const TAG_SHIFT: u32 = 48;
    const TAG_HEAP: u64 = 0b000;

    let is_tagged = (*bits & TAG_BASE) == TAG_BASE;
    if !is_tagged {
        return;
    }

    let tag = (*bits & TAG_MASK) >> TAG_SHIFT;
    if tag != TAG_HEAP {
        return;
    }

    let old_ptr = (*bits & PAYLOAD_MASK) as *mut u8;
    if old_ptr.is_null() {
        return;
    }

    if let Some(new_ptr) = forwarding.lookup(old_ptr) {
        let new_payload = (new_ptr as u64) & PAYLOAD_MASK;
        *bits = (*bits & !PAYLOAD_MASK) | new_payload;
    }
}
