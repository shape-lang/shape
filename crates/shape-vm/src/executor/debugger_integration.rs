//! Debugger integration for the VM
//!
//! This module handles debugger support, tracing, and debugging operations
//! for the virtual machine.
//!
//! ## Wave 6.5 R-async-time migration (ADR-006 §2.7.4 / §2.7.7)
//!
//! Pre-bulldozer the display-side trait methods returned `ExternalValue`
//! (a runtime-tier display carrier reading `ValueWord` tag bits). Both
//! `ExternalValue` and `shape_value::nb_to_external` are deleted along
//! with the rest of the dynamic-dispatch pipeline (CLAUDE.md "Forbidden
//! Patterns"). The post-§2.7.4 display path is `KindedSlot::Debug` —
//! `format!("{:?}", kinded_slot)` produces a runtime-only display string
//! without a value-tier dependency. The trait surface here returns
//! `Vec<String>` (debug-formatted) at every display-only site:
//!
//! - `stack_top` / `stack_values_vec` / `local_values_vec` use
//!   `read_owned_kinded(idx)` per playbook §3 (WB2.4 retain-on-read for
//!   the runtime carrier) and format the resulting `KindedSlot` via its
//!   `Debug` impl. The temporary `KindedSlot` owns one share which its
//!   own `Drop` releases when the formatted string is built.
//!
//! Module-binding inspection (`module_binding_values`,
//! `set_module_binding`) reads through the §2.7.8 / Q10 parallel-kind
//! track on `VirtualMachine.module_bindings` /
//! `module_binding_kinds`. Both methods are live: the read returns
//! each binding as a `(bits, NativeKind)` pair via
//! `module_binding_read_kinded_raw`, and the write threads through
//! `module_binding_write_kinded` (drop-prior + install-new with the
//! same retain/release discipline `stack_write_kinded` enforces). No
//! discriminator fabrication, no §2.7.7 #9 Bool-default fallback —
//! the kind comes from the parallel track populated lockstep by every
//! producer site.

use crate::debugger::VMDebugger;

/// Debugger integration for VirtualMachine.
///
/// Display-only methods (`stack_top`, `stack_values_vec`, `local_values_vec`)
/// return debug-formatted strings (post-§2.7.4: `KindedSlot::Debug`).
/// Data-flow methods (`module_binding_values`, `set_module_binding`) are
/// §2.7.4-deferred pending the parallel-kind track for module bindings.
pub trait DebuggerIntegration {
    /// Trace VM state (for debugging)
    fn trace_state(&self);

    /// Trigger a debug break
    fn debug_break(&self);

    /// Get current instruction pointer
    fn instruction_pointer(&self) -> usize;

    /// Get stack size
    fn stack_size(&self) -> usize;

    /// Get top of stack as a debug-formatted string (display only).
    fn stack_top(&self) -> Option<String>;

    /// Get all stack values as debug-formatted strings (display only).
    fn stack_values_vec(&self) -> Vec<String>;

    /// Get call stack depth
    fn call_stack_depth(&self) -> usize;

    /// Get call frames (for debugging)
    fn call_frames(&self) -> &[super::CallFrame];

    /// Get local variables as debug-formatted strings (display only).
    fn local_values_vec(&self) -> Vec<String>;

    /// Get module-binding values for data-flow inspection.
    ///
    /// Returns each binding as a `(bits, NativeKind)` pair via the
    /// §2.7.8 parallel-kind track on `VirtualMachine.module_bindings` /
    /// `module_binding_kinds`. Read-side; no refcount change.
    fn module_binding_values(&self) -> Vec<(u64, shape_value::NativeKind)>;

    /// Set a module-binding variable by index.
    ///
    /// Threads the kinded write through `module_binding_write_kinded`,
    /// which releases the prior occupant via `drop_with_kind` and
    /// installs the new `(bits, kind)` pair — same retain/release
    /// discipline `stack_write_kinded` enforces for stack slots
    /// (ADR-006 §2.7.7 / §2.7.8). Caller transfers in one
    /// strong-count share for heap-bearing kinds.
    fn set_module_binding(&mut self, index: usize, bits: u64, kind: shape_value::NativeKind);

    /// Set trace mode
    fn set_trace_mode(&mut self, enabled: bool);

    /// Get mutable reference to debugger
    fn debugger_mut(&mut self) -> Option<&mut VMDebugger>;

    /// Check if debugger is enabled
    fn has_debugger(&self) -> bool;
}

impl DebuggerIntegration for super::VirtualMachine {
    fn trace_state(&self) {
        let stack_strs: Vec<String> = (0..self.sp)
            .map(|i| {
                // Borrow read — no refcount change, no temporary KindedSlot
                // ownership transfer. Format the (bits, kind) pair directly.
                let (bits, kind) = self.stack_read_kinded_raw(i);
                format!("{{ bits: {:#x}, kind: {:?} }}", bits, kind)
            })
            .collect();
        println!("IP: {}, Stack: {:?}", self.ip, stack_strs);
        if self.ip < self.program.instructions.len() {
            println!("Next: {:?}", self.program.instructions[self.ip]);
        }
    }

    fn debug_break(&self) {
        println!("=== DEBUG BREAK ===");
        println!("IP: {}, SP: {}", self.ip, self.sp);
        let stack_strs: Vec<String> = (0..self.sp)
            .map(|i| {
                let (bits, kind) = self.stack_read_kinded_raw(i);
                format!("{{ bits: {:#x}, kind: {:?} }}", bits, kind)
            })
            .collect();
        println!("Stack: {:?}", stack_strs);
        if let Some(frame) = self.call_stack.last() {
            let bp = frame.base_pointer;
            let end = (bp + frame.locals_count).min(self.sp);
            let locals: Vec<String> = (bp..end)
                .map(|i| {
                    let (bits, kind) = self.stack_read_kinded_raw(i);
                    format!("{{ bits: {:#x}, kind: {:?} }}", bits, kind)
                })
                .collect();
            println!("Locals (bp={}): {:?}", bp, locals);
        }
        // ADR-006 §2.7.8 / Q10: walk both vecs lockstep via the
        // kinded accessor, displaying `(bits, kind)` per slot.
        let module_bindings_kinded: Vec<String> = (0..self.module_bindings_len())
            .map(|i| {
                let (bits, kind) = self.module_binding_read_kinded_raw(i);
                format!("{{ bits: {:#x}, kind: {:?} }}", bits, kind)
            })
            .collect();
        println!("Globals: {:?}", module_bindings_kinded);
        println!("Call stack depth: {}", self.call_stack.len());
    }

    // ===== Debugger Interface Methods =====

    fn instruction_pointer(&self) -> usize {
        self.ip
    }

    fn stack_size(&self) -> usize {
        self.sp
    }

    fn stack_top(&self) -> Option<String> {
        if self.sp > 0 {
            let (bits, kind) = self.stack_read_kinded_raw(self.sp - 1);
            Some(format!("{{ bits: {:#x}, kind: {:?} }}", bits, kind))
        } else {
            None
        }
    }

    fn stack_values_vec(&self) -> Vec<String> {
        (0..self.sp)
            .map(|i| {
                let (bits, kind) = self.stack_read_kinded_raw(i);
                format!("{{ bits: {:#x}, kind: {:?} }}", bits, kind)
            })
            .collect()
    }

    fn call_stack_depth(&self) -> usize {
        self.call_stack.len()
    }

    fn call_frames(&self) -> &[super::CallFrame] {
        &self.call_stack
    }

    fn local_values_vec(&self) -> Vec<String> {
        if let Some(frame) = self.call_stack.last() {
            let bp = frame.base_pointer;
            let end = (bp + frame.locals_count).min(self.sp);
            (bp..end)
                .map(|i| {
                    let (bits, kind) = self.stack_read_kinded_raw(i);
                    format!("{{ bits: {:#x}, kind: {:?} }}", bits, kind)
                })
                .collect()
        } else {
            vec![]
        }
    }

    fn module_binding_values(&self) -> Vec<(u64, shape_value::NativeKind)> {
        // ADR-006 §2.7.8 / Q10: module-binding storage now carries a
        // parallel `NativeKind` track. The debugger walks both vecs in
        // lockstep via the kinded accessor; no discriminator
        // fabrication, no §2.7.7 #9 Bool-default fallback for live
        // heap-bearing slots.
        let len = self.module_bindings_len();
        (0..len)
            .map(|i| self.module_binding_read_kinded_raw(i))
            .collect()
    }

    fn set_module_binding(&mut self, index: usize, bits: u64, kind: shape_value::NativeKind) {
        // ADR-006 §2.7.8 / Q10: kinded write through the parallel
        // track. `module_binding_write_kinded` releases the prior
        // occupant via `drop_with_kind` and installs the new
        // `(bits, kind)` pair atomically — same retain/release
        // discipline `stack_write_kinded` enforces for stack slots.
        // The caller is responsible for having retained the new
        // share before this call (matching the §2.7.7 ownership
        // contract for stack writes).
        self.module_binding_write_kinded(index, bits, kind);
    }

    fn set_trace_mode(&mut self, enabled: bool) {
        self.config.trace_execution = enabled;
        if let Some(ref mut debugger) = self.debugger {
            debugger.set_trace_mode(enabled);
        }
    }

    fn debugger_mut(&mut self) -> Option<&mut VMDebugger> {
        self.debugger.as_mut()
    }

    fn has_debugger(&self) -> bool {
        self.debugger.is_some()
    }
}
