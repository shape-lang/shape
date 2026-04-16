//! Resume logic for `state.resume()` and `state.resume_frame()`.
//!
//! Extracted from `dispatch.rs` for modularity. Both methods remain
//! `impl VirtualMachine` so the dispatch loop can call them directly.

use shape_value::{VMError, ValueWord, ValueWordExt};

use super::snapshot::resolve_function_identity;
use super::{CallFrame, VirtualMachine};

impl VirtualMachine {
    /// Apply a pending full VM state resume from `state.resume()`.
    ///
    /// Deserializes the snapshot (a TypedObject containing frame data) and
    /// replaces the VM's call stack, instruction pointer, stack pointer,
    /// locals, and module bindings.
    pub(crate) fn apply_pending_resume(&mut self) -> Result<(), VMError> {
        let snapshot = self.pending_resume.take().ok_or_else(|| {
            VMError::RuntimeError("apply_pending_resume: no pending snapshot".into())
        })?;

        // The snapshot is a VmState TypedObject with fields:
        //   slot 0 = frames array
        //   slot 1 = module_bindings array of [name, value] pairs
        //   slot 2 = instruction_count
        // Extract all needed data before consuming.
        let (frames_arr, module_bindings_nb, instruction_count_opt) =
            if let Some((_schema_id, slots, heap_mask)) = snapshot.as_typed_object() {
                let frames = if heap_mask & 1 != 0 {
                    slots[0].as_heap_nb()
                } else {
                    return Err(VMError::RuntimeError(
                        "state.resume: snapshot missing frames array".into(),
                    ));
                };
                let bindings = if slots.len() > 1 && heap_mask & (1 << 1) != 0 {
                    Some(slots[1].as_heap_nb())
                } else {
                    None
                };
                let ic = if slots.len() > 2 {
                    Some(slots[2].as_i64() as usize)
                } else {
                    None
                };
                (frames, bindings, ic)
            } else {
                return Err(VMError::RuntimeError(
                    "state.resume: snapshot must be a VmState TypedObject".into(),
                ));
            };

        let frames = frames_arr
            .as_any_array()
            .ok_or_else(|| {
                VMError::RuntimeError("state.resume: frames field must be an array".into())
            })?
            .to_generic();

        if frames.is_empty() {
            return Err(VMError::RuntimeError(
                "state.resume: empty frames array".into(),
            ));
        }

        // Clear existing call stack and reset SP
        self.call_stack.clear();
        let base_sp = self.program.top_level_locals_count as usize;
        self.sp = base_sp;

        // Restore each frame from the snapshot
        for frame_nb in frames.iter() {
            if let Some((_schema_id, slots, heap_mask)) = frame_nb.as_typed_object() {
                // FrameState slots:
                //   0=function_name (legacy / debug)
                //   1=blob_hash (canonical identity)
                //   2=ip
                //   3=locals
                //   4=args
                //   5=upvalues (optional, v2)
                if slots.len() < 4 {
                    continue;
                }

                // Extract function name
                let func_name = if heap_mask & 1 != 0 {
                    slots[0]
                        .as_heap_nb()
                        .as_str()
                        .map(|s| s.to_string())
                        .unwrap_or_default()
                } else {
                    String::new()
                };

                // Extract blob hash if present
                let blob_hash = if slots.len() > 1 && heap_mask & (1 << 1) != 0 {
                    let hash_nb = slots[1].as_heap_nb();
                    hash_nb.as_str().and_then(|hex_str| {
                        let bytes = hex::decode(hex_str.as_bytes()).ok()?;
                        if bytes.len() == 32 {
                            let mut arr = [0u8; 32];
                            arr.copy_from_slice(&bytes);
                            Some(crate::bytecode::FunctionHash(arr))
                        } else {
                            None
                        }
                    })
                } else {
                    None
                };

                // Resolve function identity via the shared hash-first helper.
                let name_ref = if func_name.is_empty() {
                    None
                } else {
                    Some(func_name.as_str())
                };
                let func_id = resolve_function_identity(
                    &self.function_id_by_hash,
                    &self.program.functions,
                    blob_hash,
                    None, // no numeric function_id in resume frames
                    name_ref,
                )?;

                // Cross-validate blob hash against current program (WS3)
                if let Some(hash) = blob_hash {
                    if let Some(current) = self.blob_hash_for_function(func_id) {
                        if current != hash {
                            return Err(VMError::RuntimeError(format!(
                                "state.resume: blob hash mismatch for function {}: \
                                 frame has {}, program has {}",
                                func_id, hash, current
                            )));
                        }
                    }
                }

                let entry = self
                    .function_entry_points
                    .get(func_id as usize)
                    .copied()
                    .ok_or_else(|| {
                        VMError::RuntimeError("state.resume: function entry point not found".into())
                    })?;

                // Extract IP offset
                let local_ip = slots[2].as_i64() as usize;

                // Extract locals
                let locals: Vec<ValueWord> = if heap_mask & (1 << 3) != 0 {
                    slots[3]
                        .as_heap_nb()
                        .as_any_array()
                        .map(|v| v.to_generic().to_vec())
                        .unwrap_or_default()
                } else {
                    Vec::new()
                };

                // Extract upvalues if present in FrameState v2.
                let upvalues: Option<Vec<shape_value::Upvalue>> =
                    if slots.len() > 5 && heap_mask & (1 << 5) != 0 {
                        let up_nb = slots[5].as_heap_nb();
                        let vals = up_nb
                            .as_any_array()
                            .map(|v| v.to_generic().to_vec())
                            .unwrap_or_default();
                        Some(vals.into_iter().map(shape_value::Upvalue::new).collect())
                    } else {
                        None
                    };

                // Set up frame on the stack
                let bp = self.sp;
                let locals_count = locals.len();
                let needed = bp + locals_count + 1;
                if self.stack.len() < needed {
                    self.stack.resize_with(needed * 2 + 1, || Self::NONE_BITS);
                }
                for (i, local) in locals.iter().enumerate() {
                    self.stack_write_raw(bp + i, local.clone());
                }
                self.sp = bp + locals_count;

                self.call_stack.push(CallFrame {
                    return_ip: entry + local_ip,
                    base_pointer: bp,
                    locals_count,
                    function_id: Some(func_id),
                    upvalues,
                    blob_hash,
                });
            }
        }

        // Set IP to the last frame's target position
        if let Some(last_frame) = self.call_stack.last() {
            self.ip = last_frame.return_ip;
        }

        // Restore module bindings (slot 1 = module_bindings array of [name, value] pairs)
        if let Some(bindings_nb) = module_bindings_nb {
            if let Some(bindings_view) = bindings_nb.as_any_array() {
                let bindings = bindings_view.to_generic();
                for pair_nb in bindings.iter() {
                    if let Some(pair_view) = pair_nb.as_any_array() {
                        let pair = pair_view.to_generic();
                        if pair.len() == 2 {
                            if let Some(name) = pair[0].as_str() {
                                if let Some(idx) = self
                                    .program
                                    .module_binding_names
                                    .iter()
                                    .position(|n| n.as_str() == name)
                                {
                                    if idx < self.module_bindings.len() {
                                        // BARRIER: heap write site — restores module binding from snapshot
                                        self.binding_write_raw(idx, pair[1].clone());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Restore instruction count if present in snapshot
        if let Some(ic) = instruction_count_opt {
            self.instruction_count = ic;
        }

        Ok(())
    }

    /// Apply a pending single-frame resume from `state.resume_frame()`.
    ///
    /// Overrides the IP and locals in the topmost call frame to resume
    /// mid-function execution rather than restarting from the beginning.
    pub(crate) fn apply_pending_frame_resume(&mut self) -> Result<(), VMError> {
        let resume_data = self.pending_frame_resume.take().ok_or_else(|| {
            VMError::RuntimeError("apply_pending_frame_resume: no pending data".into())
        })?;

        let frame = self.call_stack.last().ok_or_else(|| {
            VMError::RuntimeError("apply_pending_frame_resume: no call frame".into())
        })?;

        let func_id = frame.function_id;
        let bp = frame.base_pointer;

        // Compute the entry point for this function
        let entry = func_id
            .and_then(|fid| self.function_entry_points.get(fid as usize).copied())
            .unwrap_or(0);

        // Override IP to the captured position within the function
        self.ip = entry + resume_data.ip_offset;

        // Restore locals from the captured state
        let needed = bp + resume_data.locals.len();
        if self.stack.len() < needed {
            self.stack.resize_with(needed * 2 + 1, || Self::NONE_BITS);
        }
        for (i, local) in resume_data.locals.iter().enumerate() {
            self.stack_write_raw(bp + i, local.clone());
        }

        Ok(())
    }
}
