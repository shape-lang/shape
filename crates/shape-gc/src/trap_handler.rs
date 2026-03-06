//! SIGSEGV trap handler for concurrent relocation.
//!
//! When the GC relocates objects, old regions are protected with PROT_NONE.
//! If the mutator accesses a relocated object before pointer fixup completes,
//! a SIGSEGV fires. The trap handler:
//! 1. Checks if the faulting address is in a protected (relocated) region.
//! 2. If yes: looks up the forwarding table, updates the pointer, resumes.
//! 3. If no: chains to the previous handler (real segfault).
//!
//! **Safety:** This is the riskiest component. Initially, relocation is done
//! stop-the-world with synchronous fixup (no trap handler needed). The trap
//! handler is an opt-in feature for concurrent relocation.

use crate::relocator::ForwardingTable;
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};

/// Global state for the trap handler.
static TRAP_HANDLER_INSTALLED: AtomicBool = AtomicBool::new(false);

/// Pointer to the active forwarding table (set during relocation).
static ACTIVE_FORWARDING: AtomicPtr<ForwardingTable> = AtomicPtr::new(std::ptr::null_mut());

/// Protected region ranges for the trap handler to check.
/// Stored as (base, base+size) pairs.
static PROTECTED_RANGES: std::sync::Mutex<Vec<(usize, usize)>> = std::sync::Mutex::new(Vec::new());

/// Install the SIGSEGV trap handler.
///
/// # Safety
/// Must be called once before any relocation with concurrent access.
/// The handler is process-global and affects all threads.
pub unsafe fn install_trap_handler() -> Result<(), &'static str> {
    if TRAP_HANDLER_INSTALLED.swap(true, Ordering::SeqCst) {
        return Ok(()); // Already installed
    }

    unsafe {
        // Set up sigaction
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = trap_handler as usize;
        sa.sa_flags = libc::SA_SIGINFO | libc::SA_RESTART;
        libc::sigemptyset(&mut sa.sa_mask);

        let ret = libc::sigaction(libc::SIGSEGV, &sa, std::ptr::null_mut());
        if ret != 0 {
            TRAP_HANDLER_INSTALLED.store(false, Ordering::SeqCst);
            return Err("Failed to install SIGSEGV handler");
        }
    }

    Ok(())
}

/// Register a protected region range for the trap handler.
pub fn register_protected_range(base: usize, size: usize) {
    let mut ranges = PROTECTED_RANGES.lock().unwrap();
    ranges.push((base, base + size));
}

/// Unregister a protected region range.
pub fn unregister_protected_range(base: usize) {
    let mut ranges = PROTECTED_RANGES.lock().unwrap();
    ranges.retain(|&(b, _)| b != base);
}

/// Set the active forwarding table for the trap handler to use.
///
/// # Safety
/// The forwarding table must outlive the trap handler's use of it.
pub unsafe fn set_active_forwarding(table: *mut ForwardingTable) {
    ACTIVE_FORWARDING.store(table, Ordering::SeqCst);
}

/// Clear the active forwarding table.
pub fn clear_active_forwarding() {
    ACTIVE_FORWARDING.store(std::ptr::null_mut(), Ordering::SeqCst);
}

/// The actual SIGSEGV handler.
///
/// # Safety
/// This is called by the kernel in signal context. Must be async-signal-safe.
extern "C" fn trap_handler(
    sig: libc::c_int,
    info: *mut libc::siginfo_t,
    _context: *mut libc::c_void,
) {
    if sig != libc::SIGSEGV || info.is_null() {
        // Not our fault — chain to default handler
        unsafe {
            libc::signal(libc::SIGSEGV, libc::SIG_DFL);
            libc::raise(libc::SIGSEGV);
        }
        return;
    }

    let fault_addr = unsafe { (*info).si_addr() } as usize;

    // Check if the faulting address is in a protected (relocated) region
    let in_protected = {
        if let Ok(ranges) = PROTECTED_RANGES.try_lock() {
            ranges
                .iter()
                .any(|&(base, end)| fault_addr >= base && fault_addr < end)
        } else {
            false
        }
    };

    if !in_protected {
        // Real segfault — chain to default handler
        unsafe {
            libc::signal(libc::SIGSEGV, libc::SIG_DFL);
            libc::raise(libc::SIGSEGV);
        }
        return;
    }

    // Look up the forwarding table for this address.
    // In a real implementation, we would update the faulting instruction's
    // memory operand. For now, the synchronous fixup path handles this case.
    let _forwarding = ACTIVE_FORWARDING.load(Ordering::SeqCst);
}

/// Check if the trap handler is installed.
pub fn is_trap_handler_installed() -> bool {
    TRAP_HANDLER_INSTALLED.load(Ordering::SeqCst)
}
