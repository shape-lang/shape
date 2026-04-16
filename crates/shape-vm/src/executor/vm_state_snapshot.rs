//! Snapshot of live VM state for read-only introspection by module functions.
//!
//! `VmStateSnapshot` captures the call stack, locals, and module bindings at a
//! point during execution and implements `VmStateAccessor` so that extension
//! modules (e.g., `std::state`) can inspect the VM without holding a mutable
//! borrow on it.

use shape_runtime::module_exports::{FrameInfo, VmStateAccessor};
use shape_value::ValueWord;

use super::VirtualMachine;

/// Snapshot of VM state captured at a point during execution.
/// Implements `VmStateAccessor` for use in `ModuleContext`.
pub(crate) struct VmStateSnapshot {
    frames: Vec<FrameInfo>,
    current_args: Vec<ValueWord>,
    current_locals: Vec<(String, ValueWord)>,
    module_binding_names: Vec<String>,
    module_binding_values: Vec<ValueWord>,
    instruction_count: usize,
}

/// Construction via `VirtualMachine::capture_vm_state()`.
impl VirtualMachine {
    /// Capture a read-only snapshot of the current VM state.
    ///
    /// Iterates the call stack to build `FrameInfo` entries and copies the
    /// current module bindings. The snapshot is entirely owned data so it can
    /// be passed to `ModuleContext` without borrowing the VM.
    pub(crate) fn capture_vm_state(&self) -> VmStateSnapshot {
        let mut frames = Vec::with_capacity(self.call_stack.len());

        for frame in &self.call_stack {
            let function_name = frame
                .function_id
                .and_then(|fid| self.program.functions.get(fid as usize))
                .map(|f| f.name.clone())
                .unwrap_or_default();

            let blob_hash = frame.blob_hash.map(|fh| fh.0);

            // Compute local_ip relative to the function's entry point.
            let local_ip = if let Some(fid) = frame.function_id {
                let entry = self
                    .function_entry_points
                    .get(fid as usize)
                    .copied()
                    .unwrap_or(0);
                frame.return_ip.saturating_sub(entry)
            } else {
                frame.return_ip
            };

            // Extract locals from the unified stack for this frame.
            let locals: Vec<ValueWord> = if frame.locals_count > 0 {
                let start = frame.base_pointer;
                let end = (start + frame.locals_count).min(self.sp);
                (start..end).map(|i| self.stack_read_raw(i)).collect()
            } else {
                Vec::new()
            };

            // Extract upvalue values if present.
            let upvalues = frame
                .upvalues
                .as_ref()
                .map(|ups| ups.iter().map(|u| u.get()).collect());

            // Extract args: the first `arity` locals in the frame's register
            // window are the arguments.
            let args: Vec<ValueWord> = frame
                .function_id
                .and_then(|fid| self.program.functions.get(fid as usize))
                .map(|func| {
                    let arity = func.arity as usize;
                    let start = frame.base_pointer;
                    let end = start
                        .saturating_add(arity)
                        .min(frame.base_pointer + frame.locals_count)
                        .min(self.sp);
                    (start..end).map(|i| self.stack_read_raw(i)).collect()
                })
                .unwrap_or_default();

            frames.push(FrameInfo {
                function_id: frame.function_id,
                function_name,
                blob_hash,
                local_ip,
                locals,
                upvalues,
                args,
            });
        }

        // Extract current args and locals from the topmost frame.
        let current_args = frames.last().map(|f| f.args.clone()).unwrap_or_default();

        let current_locals = frames
            .last()
            .and_then(|f| {
                f.function_id
                    .and_then(|fid| self.program.functions.get(fid as usize))
                    .map(|func| {
                        func.param_names
                            .iter()
                            .enumerate()
                            .filter_map(|(i, name)| {
                                f.locals.get(i).map(|val| (name.clone(), val.clone()))
                            })
                            .collect::<Vec<_>>()
                    })
            })
            .unwrap_or_default();

        VmStateSnapshot {
            frames,
            current_args,
            current_locals,
            module_binding_names: self.program.module_binding_names.clone(),
            module_binding_values: (0..self.module_bindings.len())
                .map(|i| self.binding_read_raw(i))
                .collect(),
            instruction_count: self.instruction_count,
        }
    }
}

impl VmStateAccessor for VmStateSnapshot {
    fn current_frame(&self) -> Option<FrameInfo> {
        self.frames.last().cloned()
    }

    fn all_frames(&self) -> Vec<FrameInfo> {
        self.frames.clone()
    }

    fn caller_frame(&self) -> Option<FrameInfo> {
        if self.frames.len() >= 2 {
            Some(self.frames[self.frames.len() - 2].clone())
        } else {
            None
        }
    }

    fn current_args(&self) -> Vec<ValueWord> {
        self.current_args.clone()
    }

    fn current_locals(&self) -> Vec<(String, ValueWord)> {
        self.current_locals.clone()
    }

    fn module_bindings(&self) -> Vec<(String, ValueWord)> {
        self.module_binding_names
            .iter()
            .zip(self.module_binding_values.iter())
            .map(|(name, val)| (name.clone(), val.clone()))
            .collect()
    }

    fn instruction_count(&self) -> usize {
        self.instruction_count
    }
}
