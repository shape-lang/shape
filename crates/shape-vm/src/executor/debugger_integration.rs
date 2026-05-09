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
//! `set_module_binding`) needs a parallel-kind track on
//! `VirtualMachine.module_bindings` (currently `Vec<u64>` with no
//! companion). That parallel track is owned by the §2.7.8 cell-storage
//! rebuild — module-binding kinds were not extended in B7/B8/B9 (B7 =
//! ClosureCell, B8 = SharedCell, B9 = CallFrame.closure_heap_kind). The
//! debugger surface therefore uses the §2.7.4 Phase-2c deferral pattern
//! (`todo!`) for those two methods until the parallel-kind track lands
//! for module bindings — handing back an empty Vec or fabricating
//! `NativeKind::Bool` defaults would silently lose data-flow information
//! the debugger is supposed to surface, which §2.7.7 forbids verbatim.

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
    /// **Phase-2c deferral — ADR-006 §2.7.4.** Module bindings need a
    /// parallel-kind track (companion `Vec<NativeKind>` on
    /// `VirtualMachine.module_bindings`) so the debugger can hand back
    /// kinded values without fabricating a discriminator. That track is
    /// owned by the §2.7.8 cell-storage rebuild.
    fn module_binding_values(&self) -> Vec<(u64, shape_value::NativeKind)>;

    /// Set a module-binding variable by index.
    ///
    /// **Phase-2c deferral — ADR-006 §2.7.4.** Symmetric to the read path:
    /// requires the parallel-kind track to thread the kinded write through
    /// the same retain/release discipline `stack_write_kinded` enforces
    /// for stack slots.
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
        // Module bindings inspection is §2.7.4-deferred (no parallel-kind
        // track yet — see trait doc on `module_binding_values`). Display
        // only the raw 8-byte slot bits — kind is unavailable.
        let module_bindings_raw: Vec<String> = self
            .module_bindings
            .iter()
            .map(|&b| format!("{:#x}", b))
            .collect();
        println!(
            "Globals (raw bits — kinds pending §2.7.8 parallel track): {:?}",
            module_bindings_raw
        );
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
        todo!(
            "phase-2c — ADR-006 §2.7.4: module_binding_values requires a \
             parallel-kind track (companion Vec<NativeKind>) on \
             VirtualMachine.module_bindings (§2.7.8 / Q10). The pre-bulldozer \
             return type was Vec<ValueWord> via the deleted binding_read_raw \
             shim; without the parallel track the debugger cannot hand back \
             kinded values without fabricating a discriminator (forbidden by \
             §2.7.7)."
        )
    }

    fn set_module_binding(&mut self, _index: usize, _bits: u64, _kind: shape_value::NativeKind) {
        todo!(
            "phase-2c — ADR-006 §2.7.4: set_module_binding requires the same \
             parallel-kind track as module_binding_values. The pre-bulldozer \
             body called the deleted binding_write_raw shim with a ValueWord; \
             the kinded write site (drop_with_kind on the prior slot, install \
             new (bits, kind) pair) needs the §2.7.8 parallel track to land \
             before this method can re-light."
        )
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
