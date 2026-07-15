//! Ownership-aware JIT access to refcounted `SharedCell` payloads.
//!
//! The closure/local carrier is always the raw pointer of a live
//! `Arc<SharedCell>`. The cell's immutable `NativeKind` companion is the sole
//! authority for payload ownership. Reads mint one typed payload share while
//! holding the cell lock; replacements transfer the incoming share into the
//! same cell and retire the previous share after unlocking.

use shape_value::NativeKind;
use shape_value::v2::closure_layout::SharedCell;
use shape_value::v2::closure_raw::{clone_with_kind, drop_with_kind};

/// Mint one owning share for a borrowed `(bits, kind)` pair.
///
/// `clone_with_kind` is the canonical per-`NativeKind` retain dispatch,
/// including the optional cycle-GC increment barrier. Ownership of the
/// original remains in the cell and the clone is returned as raw bits. Zero is
/// the canonical empty payload and requires no dispatch.
///
/// # Safety
///
/// Nonzero `bits` must be a valid live carrier for `kind`.
unsafe fn retain_payload(bits: u64, kind: NativeKind) {
    unsafe { clone_with_kind(bits, kind) };
}

/// Retire one owning share through the canonical typed drop dispatch.
///
/// # Safety
///
/// Nonzero `bits` must be one live owning carrier share for `kind`.
unsafe fn retire_payload(bits: u64, kind: NativeKind) {
    unsafe { drop_with_kind(bits, kind) };
}

/// Clone the refcounted payload owned by a live `SharedCell`.
///
/// This is the JIT counterpart of VM `LoadSharedCapturePtr` /
/// `LoadSharedLocal`: the cell stays the owner of its original payload share,
/// while the returned raw bits own one newly retained share. The retain occurs
/// while the lock is held, so a concurrent replacement cannot retire the
/// payload between load and retain.
///
/// # Safety
///
/// - `cell_ptr` must be a non-null pointer from a live `Arc<SharedCell>` share.
/// - The cell payload must satisfy its construction invariant: nonzero bits are
///   a valid carrier for `SharedCell::kind()`.
/// - The caller becomes responsible for exactly one typed release of a nonzero
///   return value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_read_shared_cell_ptr(cell_ptr: i64) -> i64 {
    let cell = unsafe { &*(cell_ptr as *const SharedCell) };
    let kind = cell.kind();
    let guard = cell.lock();
    let bits = *guard;
    unsafe { retain_payload(bits, kind) };
    drop(guard);
    bits as i64
}

/// Replace the refcounted payload of a live `SharedCell`.
///
/// The caller transfers exactly one owning share in `value`. The swap is
/// serialized by the cell lock. The displaced share is retired only after the
/// lock is released, matching the VM path and avoiding destructor work or a
/// recursive cell edge while the cell is locked.
///
/// # Safety
///
/// - `cell_ptr` must be a non-null pointer from a live `Arc<SharedCell>` share.
/// - Nonzero `value` must be exactly one owning carrier share matching the
///   cell's immutable `NativeKind`.
/// - Ownership of `value` is consumed even when its raw address equals the
///   previous payload address; callers must have retained or transferred a
///   distinct strong-count share in that case.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_write_shared_cell_ptr(cell_ptr: i64, value: i64) {
    let cell = unsafe { &*(cell_ptr as *const SharedCell) };
    let kind = cell.kind();
    let previous = {
        let mut guard = cell.lock();
        std::mem::replace(&mut *guard, value as u64)
    };
    unsafe { retire_payload(previous, kind) };
}
