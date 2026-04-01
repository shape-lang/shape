//! SIGSEGV crash handler for JIT debugging.
//!
//! Installs a signal handler that captures the faulting address and instruction
//! pointer (RIP) when JIT-compiled code crashes. Compares the RIP against
//! registered JIT code regions to determine if the crash is in generated code
//! or in Rust FFI.

use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};

/// Whether the crash handler has been installed.
static HANDLER_INSTALLED: AtomicBool = AtomicBool::new(false);

thread_local! {
    /// Re-entrancy guard: true while we're borrowing JIT_REGIONS via cell.take().
    /// If the signal handler fires while this is set, skip region lookup to
    /// avoid double-borrow (cell.take() on an already-taken Cell returns empty Vec).
    static JIT_REGIONS_BUSY: Cell<bool> = const { Cell::new(false) };
}

/// JIT code region: (start_addr, end_addr, function_name)
#[derive(Clone, Debug)]
pub struct JitCodeRegion {
    pub start: usize,
    pub end: usize,
    pub name: String,
}

thread_local! {
    /// Registered JIT code regions for the current thread.
    static JIT_REGIONS: Cell<Vec<JitCodeRegion>> = const { Cell::new(Vec::new()) };
}

/// Register a JIT code region for crash diagnosis.
pub fn register_jit_region(name: &str, code_ptr: *const u8, code_size: usize) {
    let start = code_ptr as usize;
    let end = start + code_size;
    JIT_REGIONS.with(|cell| {
        let mut regions = cell.take();
        regions.push(JitCodeRegion {
            start,
            end,
            name: name.to_string(),
        });
        cell.set(regions);
    });
}

/// Register all compiled functions from a JITCompiler's function table.
pub fn register_compiled_functions(compiled_functions: &std::collections::HashMap<String, *const u8>) {
    for (name, &ptr) in compiled_functions {
        if !ptr.is_null() {
            // We don't know exact sizes, so use a generous estimate.
            // The handler checks min/max range of all registered regions.
            register_jit_region(name, ptr, 64 * 1024);
        }
    }
}

/// Clear all registered JIT code regions.
pub fn clear_jit_regions() {
    JIT_REGIONS.with(|cell| {
        cell.set(Vec::new());
    });
}

/// Find which JIT function (if any) contains the given address.
/// Returns None if regions are currently borrowed (re-entrancy).
fn find_jit_function(addr: usize) -> Option<String> {
    JIT_REGIONS_BUSY.with(|busy| {
        if busy.get() {
            return None;
        }
        busy.set(true);
        let result = JIT_REGIONS.with(|cell| {
            let regions = cell.take();
            let found = regions
                .iter()
                .find(|r| addr >= r.start && addr < r.end)
                .map(|r| r.name.clone());
            cell.set(regions);
            found
        });
        busy.set(false);
        result
    })
}

/// Check if an address falls within any registered JIT code region.
/// Returns false if regions are currently borrowed (re-entrancy).
fn is_in_jit_code(addr: usize) -> bool {
    JIT_REGIONS_BUSY.with(|busy| {
        if busy.get() {
            return false;
        }
        busy.set(true);
        let result = JIT_REGIONS.with(|cell| {
            let regions = cell.take();
            let found = regions.iter().any(|r| addr >= r.start && addr < r.end);
            cell.set(regions);
            found
        });
        busy.set(false);
        result
    })
}

/// Check if an address falls within any registered JIT code region (signal-safe).
/// Unlike `is_in_jit_code`, this does not use the re-entrancy guard and
/// operates directly on the cell. Only safe to call from the signal handler
/// after confirming JIT_REGIONS_BUSY is not set.
fn is_rip_in_jit_region_unchecked(addr: usize) -> bool {
    JIT_REGIONS.with(|cell| {
        let regions = cell.take();
        let found = regions.iter().any(|r| addr >= r.start && addr < r.end);
        cell.set(regions);
        found
    })
}

/// Install the SIGSEGV signal handler.
///
/// Only installs once. The handler prints diagnostic info about the crash
/// (faulting address, RIP, whether RIP is in JIT code) and then re-raises
/// the signal with the default handler.
///
/// # Safety
/// Signal handler installation is inherently unsafe. This must only be
/// called before JIT execution begins on the current thread.
pub unsafe fn install_crash_handler() {
    if HANDLER_INSTALLED.swap(true, Ordering::SeqCst) {
        return; // Already installed
    }

    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_flags = libc::SA_SIGINFO | libc::SA_ONSTACK;
        sa.sa_sigaction = sigsegv_handler as usize;
        libc::sigemptyset(&mut sa.sa_mask);
        libc::sigaction(libc::SIGSEGV, &sa, std::ptr::null_mut());
        libc::sigaction(libc::SIGBUS, &sa, std::ptr::null_mut());
    }
}

/// Uninstall the crash handler, restoring default signal behavior.
///
/// # Safety
/// Must be called after JIT execution completes.
pub unsafe fn uninstall_crash_handler() {
    if !HANDLER_INSTALLED.swap(false, Ordering::SeqCst) {
        return; // Not installed
    }

    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = libc::SIG_DFL;
        libc::sigemptyset(&mut sa.sa_mask);
        libc::sigaction(libc::SIGSEGV, &sa, std::ptr::null_mut());
        libc::sigaction(libc::SIGBUS, &sa, std::ptr::null_mut());
    }
}

/// Signal-safe write to stderr. Uses raw `write` syscall — no allocations,
/// no locks, no heap — safe inside a signal handler.
unsafe fn write_stderr(msg: &[u8]) {
    unsafe {
        libc::write(2, msg.as_ptr() as *const libc::c_void, msg.len());
    }
}

/// Convert a u64 to hex string in a fixed buffer (signal-safe, no allocation).
fn u64_to_hex(val: u64, buf: &mut [u8; 18]) -> usize {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    buf[0] = b'0';
    buf[1] = b'x';
    if val == 0 {
        buf[2] = b'0';
        return 3;
    }
    // Find the highest nibble
    let mut shift = 60;
    while shift > 0 && (val >> shift) & 0xF == 0 {
        shift -= 4;
    }
    let mut pos = 2;
    while shift > 0 || pos == 2 {
        buf[pos] = HEX[((val >> shift) & 0xF) as usize];
        pos += 1;
        if shift == 0 {
            break;
        }
        shift -= 4;
    }
    pos
}

/// The actual SIGSEGV/SIGBUS handler.
///
/// Prints crash diagnostics and re-raises the signal.
extern "C" fn sigsegv_handler(
    sig: libc::c_int,
    info: *mut libc::siginfo_t,
    context: *mut libc::c_void,
) {
    unsafe {
        let sig_name = if sig == libc::SIGSEGV {
            b"\n=== JIT CRASH: SIGSEGV ===\n" as &[u8]
        } else {
            b"\n=== JIT CRASH: SIGBUS ===\n" as &[u8]
        };
        write_stderr(sig_name);

        // Extract faulting address from siginfo_t
        if !info.is_null() {
            let fault_addr = (*info).si_addr() as u64;
            let mut buf = [0u8; 18];
            write_stderr(b"  fault_addr = ");
            let len = u64_to_hex(fault_addr, &mut buf);
            write_stderr(&buf[..len]);
            write_stderr(b"\n");
        }

        // Extract RIP from ucontext_t (x86_64 Linux)
        #[cfg(target_arch = "x86_64")]
        {
            if !context.is_null() {
                let uc = context as *const libc::ucontext_t;
                let rip = (*uc).uc_mcontext.gregs[libc::REG_RIP as usize] as u64;
                let rsp = (*uc).uc_mcontext.gregs[libc::REG_RSP as usize] as u64;
                let rax = (*uc).uc_mcontext.gregs[libc::REG_RAX as usize] as u64;
                let rbx = (*uc).uc_mcontext.gregs[libc::REG_RBX as usize] as u64;
                let rcx = (*uc).uc_mcontext.gregs[libc::REG_RCX as usize] as u64;
                let rdx = (*uc).uc_mcontext.gregs[libc::REG_RDX as usize] as u64;
                let rdi = (*uc).uc_mcontext.gregs[libc::REG_RDI as usize] as u64;
                let rsi = (*uc).uc_mcontext.gregs[libc::REG_RSI as usize] as u64;
                let r8  = (*uc).uc_mcontext.gregs[libc::REG_R8 as usize] as u64;
                let r9  = (*uc).uc_mcontext.gregs[libc::REG_R9 as usize] as u64;
                let r10 = (*uc).uc_mcontext.gregs[libc::REG_R10 as usize] as u64;
                let r11 = (*uc).uc_mcontext.gregs[libc::REG_R11 as usize] as u64;
                let r12 = (*uc).uc_mcontext.gregs[libc::REG_R12 as usize] as u64;
                let r13 = (*uc).uc_mcontext.gregs[libc::REG_R13 as usize] as u64;
                let r14 = (*uc).uc_mcontext.gregs[libc::REG_R14 as usize] as u64;
                let r15 = (*uc).uc_mcontext.gregs[libc::REG_R15 as usize] as u64;

                let mut buf = [0u8; 18];

                write_stderr(b"  RIP = ");
                let len = u64_to_hex(rip, &mut buf);
                write_stderr(&buf[..len]);

                // Check if RIP is in JIT code
                if is_in_jit_code(rip as usize) {
                    write_stderr(b"  [IN JIT CODE]");
                    if let Some(func_name) = find_jit_function(rip as usize) {
                        write_stderr(b" fn=");
                        write_stderr(func_name.as_bytes());
                    }
                } else {
                    write_stderr(b"  [NOT in JIT code - possibly Rust FFI]");
                }
                write_stderr(b"\n");

                write_stderr(b"  RSP = ");
                let len = u64_to_hex(rsp, &mut buf);
                write_stderr(&buf[..len]);
                write_stderr(b"\n");

                write_stderr(b"  RAX = ");
                let len = u64_to_hex(rax, &mut buf);
                write_stderr(&buf[..len]);
                write_stderr(b"\n");

                write_stderr(b"  RBX = ");
                let len = u64_to_hex(rbx, &mut buf);
                write_stderr(&buf[..len]);
                write_stderr(b"\n");

                write_stderr(b"  RCX = ");
                let len = u64_to_hex(rcx, &mut buf);
                write_stderr(&buf[..len]);
                write_stderr(b"\n");

                write_stderr(b"  RDX = ");
                let len = u64_to_hex(rdx, &mut buf);
                write_stderr(&buf[..len]);
                write_stderr(b"\n");

                write_stderr(b"  RDI = ");
                let len = u64_to_hex(rdi, &mut buf);
                write_stderr(&buf[..len]);
                write_stderr(b"\n");

                write_stderr(b"  RSI = ");
                let len = u64_to_hex(rsi, &mut buf);
                write_stderr(&buf[..len]);
                write_stderr(b"\n");

                write_stderr(b"  R8  = ");
                let len = u64_to_hex(r8, &mut buf);
                write_stderr(&buf[..len]);
                write_stderr(b"\n");

                write_stderr(b"  R9  = ");
                let len = u64_to_hex(r9, &mut buf);
                write_stderr(&buf[..len]);
                write_stderr(b"\n");

                write_stderr(b"  R10 = ");
                let len = u64_to_hex(r10, &mut buf);
                write_stderr(&buf[..len]);
                write_stderr(b"\n");

                write_stderr(b"  R11 = ");
                let len = u64_to_hex(r11, &mut buf);
                write_stderr(&buf[..len]);
                write_stderr(b"\n");

                write_stderr(b"  R12 = ");
                let len = u64_to_hex(r12, &mut buf);
                write_stderr(&buf[..len]);
                write_stderr(b"\n");

                write_stderr(b"  R13 = ");
                let len = u64_to_hex(r13, &mut buf);
                write_stderr(&buf[..len]);
                write_stderr(b"\n");

                write_stderr(b"  R14 = ");
                let len = u64_to_hex(r14, &mut buf);
                write_stderr(&buf[..len]);
                write_stderr(b"\n");

                write_stderr(b"  R15 = ");
                let len = u64_to_hex(r15, &mut buf);
                write_stderr(&buf[..len]);
                write_stderr(b"\n");

                // Dump a few bytes around RIP to identify the faulting instruction.
                // Only read if RIP falls within a registered JIT region (known-mapped memory).
                // Dereferencing an unmapped RIP would cause a double fault.
                if is_rip_in_jit_region_unchecked(rip as usize) {
                    write_stderr(b"  code @ RIP: ");
                    let rip_ptr = rip as *const u8;
                    for i in 0..16isize {
                        let byte = *rip_ptr.offset(i);
                        let hi = HEX_CHARS[(byte >> 4) as usize];
                        let lo = HEX_CHARS[(byte & 0xF) as usize];
                        let byte_str = [hi, lo, b' '];
                        write_stderr(&byte_str);
                    }
                    write_stderr(b"\n");
                } else {
                    write_stderr(b"  code @ RIP: [skipped - RIP not in registered JIT region]\n");
                }

                // Print JIT region info (with re-entrancy guard)
                write_stderr(b"  JIT regions:\n");
                let regions_available = JIT_REGIONS_BUSY.with(|busy| !busy.get());
                if regions_available {
                    JIT_REGIONS.with(|cell| {
                        let regions = cell.take();
                        for region in &regions {
                            write_stderr(b"    ");
                            write_stderr(region.name.as_bytes());
                            write_stderr(b" @ ");
                            let len = u64_to_hex(region.start as u64, &mut buf);
                            write_stderr(&buf[..len]);
                            write_stderr(b"\n");
                        }
                        cell.set(regions);
                    });
                } else {
                    write_stderr(b"    [unavailable - regions busy]\n");
                }
            }
        }

        write_stderr(b"  [backtrace unavailable in signal handler - use RUST_BACKTRACE=1 or core dump]\n");
        write_stderr(b"=== END JIT CRASH ===\n");

        // Re-raise with default handler to get core dump / process termination
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = libc::SIG_DFL;
        libc::sigemptyset(&mut sa.sa_mask);
        libc::sigaction(sig, &sa, std::ptr::null_mut());
        libc::raise(sig);
    }
}

const HEX_CHARS: &[u8; 16] = b"0123456789abcdef";
