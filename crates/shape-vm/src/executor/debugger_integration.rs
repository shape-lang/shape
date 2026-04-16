//! Debugger integration for the VM
//!
//! This module handles debugger support, tracing, and debugging operations
//! for the virtual machine.
//!
//! Display-only methods (stack_top, stack_values_vec, local_values_vec)
//! return `ExternalValue` for safe serialization and display.
//! Data-flow methods (module_binding_values, set_global) retain `ValueWord` since
//! they feed values back into the VM.

use crate::debugger::VMDebugger;
use shape_value::{ExternalValue, ValueWord};

/// Debugger integration for VirtualMachine
pub trait DebuggerIntegration {
    /// Trace VM state (for debugging)
    fn trace_state(&self);

    /// Trigger a debug break
    fn debug_break(&self);

    /// Get current instruction pointer
    fn instruction_pointer(&self) -> usize;

    /// Get stack size
    fn stack_size(&self) -> usize;

    /// Get top of stack as ExternalValue (display only)
    fn stack_top(&self) -> Option<ExternalValue>;

    /// Get all stack values as ExternalValues (display only)
    fn stack_values_vec(&self) -> Vec<ExternalValue>;

    /// Get call stack depth
    fn call_stack_depth(&self) -> usize;

    /// Get call frames (for debugging)
    fn call_frames(&self) -> &[super::CallFrame];

    /// Get local variables as ExternalValues (display only)
    fn local_values_vec(&self) -> Vec<ExternalValue>;

    /// Get module_binding variables (data-flow: values are stored back into VM)
    fn module_binding_values(&self) -> Vec<ValueWord>;

    /// Set a module_binding variable by index
    fn set_module_binding(&mut self, index: usize, value: ValueWord);

    /// Set trace mode
    fn set_trace_mode(&mut self, enabled: bool);

    /// Get mutable reference to debugger
    fn debugger_mut(&mut self) -> Option<&mut VMDebugger>;

    /// Check if debugger is enabled
    fn has_debugger(&self) -> bool;
}

impl DebuggerIntegration for super::VirtualMachine {
    fn trace_state(&self) {
        let schemas = &self.program.type_schema_registry;
        let stack_vals: Vec<_> = self.stack_slice_raw(0..self.sp)
            .iter()
            .map(|nb| shape_value::nb_to_external(nb, schemas))
            .collect();
        println!("IP: {}, Stack: {:?}", self.ip, stack_vals);
        if self.ip < self.program.instructions.len() {
            println!("Next: {:?}", self.program.instructions[self.ip]);
        }
    }

    fn debug_break(&self) {
        let schemas = &self.program.type_schema_registry;
        println!("=== DEBUG BREAK ===");
        println!("IP: {}, SP: {}", self.ip, self.sp);
        let stack_vals: Vec<_> = self.stack_slice_raw(0..self.sp)
            .iter()
            .map(|nb| shape_value::nb_to_external(nb, schemas))
            .collect();
        println!("Stack: {:?}", stack_vals);
        if let Some(frame) = self.call_stack.last() {
            let bp = frame.base_pointer;
            let end = (bp + frame.locals_count).min(self.sp);
            let locals: Vec<_> = self.stack_slice_raw(bp..end)
                .iter()
                .map(|nb| shape_value::nb_to_external(nb, schemas))
                .collect();
            println!("Locals (bp={}): {:?}", bp, locals);
        }
        let module_bindings: Vec<_> = self
            .bindings_slice_raw()
            .iter()
            .map(|nb| shape_value::nb_to_external(nb, schemas))
            .collect();
        println!("Globals: {:?}", module_bindings);
        println!("Call stack depth: {}", self.call_stack.len());
    }

    // ===== Debugger Interface Methods =====

    fn instruction_pointer(&self) -> usize {
        self.ip
    }

    fn stack_size(&self) -> usize {
        self.sp
    }

    fn stack_top(&self) -> Option<ExternalValue> {
        if self.sp > 0 {
            let schemas = &self.program.type_schema_registry;
            let vw_slice = self.stack_slice_raw((self.sp - 1)..self.sp);
            Some(shape_value::nb_to_external(&vw_slice[0], schemas))
        } else {
            None
        }
    }

    fn stack_values_vec(&self) -> Vec<ExternalValue> {
        let schemas = &self.program.type_schema_registry;
        self.stack_slice_raw(0..self.sp)
            .iter()
            .map(|nb| shape_value::nb_to_external(nb, schemas))
            .collect()
    }

    fn call_stack_depth(&self) -> usize {
        self.call_stack.len()
    }

    fn call_frames(&self) -> &[super::CallFrame] {
        &self.call_stack
    }

    fn local_values_vec(&self) -> Vec<ExternalValue> {
        let schemas = &self.program.type_schema_registry;
        if let Some(frame) = self.call_stack.last() {
            let bp = frame.base_pointer;
            let end = (bp + frame.locals_count).min(self.sp);
            self.stack_slice_raw(bp..end)
                .iter()
                .map(|nb| shape_value::nb_to_external(nb, schemas))
                .collect()
        } else {
            vec![]
        }
    }

    fn module_binding_values(&self) -> Vec<ValueWord> {
        (0..self.module_bindings.len())
            .map(|i| self.binding_read_raw(i))
            .collect()
    }

    fn set_module_binding(&mut self, index: usize, value: ValueWord) {
        if index < self.module_bindings.len() {
            // BARRIER: heap write site — debugger overwrites module binding slot
            self.binding_write_raw(index, value);
        } else {
            self.module_bindings.resize_with(index + 1, || Self::NONE_BITS);
            // BARRIER: heap write site — debugger overwrites module binding slot (after resize)
            self.binding_write_raw(index, value);
        }
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
