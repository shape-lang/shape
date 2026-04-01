//! VM snapshot and restore for suspending/resuming execution.

use std::collections::HashMap;

use shape_runtime::snapshot::{
    SerializableCallFrame, SerializableExceptionHandler, SerializableLoopContext, SnapshotStore,
    VmSnapshot, nanboxed_to_serializable, serializable_to_nanboxed,
};
use shape_value::{Upvalue, VMError, ValueWord};

use super::{CallFrame, ExceptionHandler, LoopContext, VMConfig, VirtualMachine};
use crate::bytecode::{Function, FunctionHash};

/// Resolve a function's runtime ID from content-addressed identity.
///
/// Priority: `blob_hash` → `function_id` → `function_name`.
/// Cross-validates when multiple identifiers are present.
pub(crate) fn resolve_function_identity(
    function_id_by_hash: &HashMap<FunctionHash, u16>,
    functions: &[Function],
    blob_hash: Option<FunctionHash>,
    function_id: Option<u16>,
    function_name: Option<&str>,
) -> Result<u16, VMError> {
    // 1. Hash-first resolution
    if let Some(hash) = blob_hash {
        let resolved = function_id_by_hash.get(&hash).copied().ok_or_else(|| {
            VMError::RuntimeError(format!("unknown function blob hash: {}", hash))
        })?;
        // Cross-validate: if function_id is also present, they must agree
        if let Some(fid) = function_id {
            if fid != resolved {
                return Err(VMError::RuntimeError(format!(
                    "function_id/hash mismatch: frame id {} does not match hash {} (resolved id {})",
                    fid, hash, resolved
                )));
            }
        }
        return Ok(resolved);
    }

    // 2. Direct function_id (no hash available)
    if let Some(fid) = function_id {
        if (fid as usize) < functions.len() {
            return Ok(fid);
        }
        return Err(VMError::RuntimeError(format!(
            "function_id {} out of range (program has {} functions)",
            fid,
            functions.len()
        )));
    }

    // 3. Name-based fallback — require exactly one match
    if let Some(name) = function_name {
        let matches: Vec<usize> = functions
            .iter()
            .enumerate()
            .filter_map(|(idx, f)| if f.name == name { Some(idx) } else { None })
            .collect();
        return match matches.len() {
            1 => Ok(matches[0] as u16),
            0 => Err(VMError::RuntimeError(format!(
                "no function named '{}'",
                name
            ))),
            n => Err(VMError::RuntimeError(format!(
                "ambiguous function name '{}' ({} matches)",
                name, n
            ))),
        };
    }

    // 4. No identifiers at all
    Err(VMError::RuntimeError(
        "cannot resolve function identity: no hash, id, or name provided".into(),
    ))
}

impl VirtualMachine {
    /// Create a serializable snapshot of VM state.
    pub fn snapshot(&self, store: &SnapshotStore) -> Result<VmSnapshot, VMError> {
        let mut stack = Vec::with_capacity(self.sp);
        for nb in self.stack[..self.sp].iter() {
            stack.push(
                nanboxed_to_serializable(nb, store)
                    .map_err(|e| VMError::RuntimeError(e.to_string()))?,
            );
        }
        // Locals are now part of the unified stack; serialize empty vec for backward compat
        let locals = Vec::new();
        let mut module_bindings = Vec::with_capacity(self.module_bindings.len());
        for nb in self.module_bindings.iter() {
            module_bindings.push(
                nanboxed_to_serializable(nb, store)
                    .map_err(|e| VMError::RuntimeError(e.to_string()))?,
            );
        }

        let mut call_stack = Vec::with_capacity(self.call_stack.len());
        for frame in self.call_stack.iter() {
            let upvalues = match &frame.upvalues {
                Some(values) => {
                    let mut out = Vec::new();
                    for up in values.iter() {
                        let nb = up.get();
                        out.push(
                            nanboxed_to_serializable(&nb, store)
                                .map_err(|e| VMError::RuntimeError(e.to_string()))?,
                        );
                    }
                    Some(out)
                }
                None => None,
            };
            // Compute content-addressed snapshot fields when blob_hash is available
            let (blob_hash, local_ip) =
                if let (Some(hash), Some(fid)) = (frame.blob_hash, frame.function_id) {
                    let entry_point = self
                        .function_entry_points
                        .get(fid as usize)
                        .copied()
                        .unwrap_or(0);
                    let lip = frame.return_ip.saturating_sub(entry_point);
                    (Some(hash.0), Some(lip))
                } else {
                    (None, None)
                };

            call_stack.push(SerializableCallFrame {
                return_ip: frame.return_ip,
                locals_base: frame.base_pointer,
                locals_count: frame.locals_count,
                function_id: frame.function_id,
                upvalues,
                blob_hash,
                local_ip,
            });
        }

        let loop_stack = self
            .loop_stack
            .iter()
            .map(|l| SerializableLoopContext {
                start: l.start,
                end: l.end,
            })
            .collect();
        let exception_handlers = self
            .exception_handlers
            .iter()
            .map(|h| SerializableExceptionHandler {
                catch_ip: h.catch_ip,
                stack_size: h.stack_size,
                call_depth: h.call_depth,
            })
            .collect();

        // Compute relocatable top-level IP from the current call frame.
        // The top-level `ip` corresponds to the innermost frame's function.
        let (ip_blob_hash, ip_local_offset, ip_function_id) =
            if let Some(frame) = self.call_stack.last() {
                let fid = frame.function_id;
                let blob_hash = fid.and_then(|id| self.blob_hash_for_function(id));
                let entry_point = fid
                    .and_then(|id| self.function_entry_points.get(id as usize).copied())
                    .unwrap_or(0);
                let local_offset = self.ip.saturating_sub(entry_point);
                (blob_hash.map(|h| h.0), Some(local_offset), fid)
            } else {
                (None, None, None)
            };

        Ok(VmSnapshot {
            ip: self.ip,
            stack,
            locals,
            module_bindings,
            call_stack,
            loop_stack,
            timeframe_stack: self.timeframe_stack.clone(),
            exception_handlers,
            ip_blob_hash,
            ip_local_offset,
            ip_function_id,
        })
    }

    /// Restore a VM from a snapshot and bytecode program.
    pub fn from_snapshot(
        program: crate::bytecode::BytecodeProgram,
        snapshot: &VmSnapshot,
        store: &SnapshotStore,
    ) -> Result<Self, VMError> {
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(program);

        // Relocate the top-level IP using content-addressed identity when
        // available. This handles the case where the program was recompiled
        // and instruction positions changed.
        vm.ip = if let (Some(hash_bytes), Some(local_offset)) =
            (&snapshot.ip_blob_hash, snapshot.ip_local_offset)
        {
            let hash = FunctionHash(*hash_bytes);
            // Look up the function by blob hash in the new program
            let func_id = resolve_function_identity(
                &vm.function_id_by_hash,
                &vm.program.functions,
                Some(hash),
                snapshot.ip_function_id,
                None,
            )?;
            let entry_point = vm
                .function_entry_points
                .get(func_id as usize)
                .copied()
                .unwrap_or(0);
            entry_point + local_offset
        } else if let Some(fid) = snapshot.ip_function_id {
            // Fallback: use function_id to relocate (same program, stable IDs)
            let entry_point = vm
                .function_entry_points
                .get(fid as usize)
                .copied()
                .unwrap_or(0);
            let local_offset = snapshot.ip_local_offset.unwrap_or(0);
            entry_point + local_offset
        } else {
            // Legacy snapshots without relocation info: use absolute IP
            snapshot.ip
        };

        let restored_stack: Vec<ValueWord> = snapshot
            .stack
            .iter()
            .map(|v| {
                serializable_to_nanboxed(v, store).map_err(|e| VMError::RuntimeError(e.to_string()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let restored_sp = restored_stack.len();
        // Pre-allocate and copy into the unified stack
        vm.stack = (0..restored_sp.max(crate::constants::DEFAULT_STACK_CAPACITY))
            .map(|_| ValueWord::none())
            .collect();
        for (i, nb) in restored_stack.into_iter().enumerate() {
            vm.stack[i] = nb;
        }
        vm.sp = restored_sp;
        // Locals snapshot is ignored — locals now live on the unified stack
        vm.module_bindings = snapshot
            .module_bindings
            .iter()
            .map(|v| {
                serializable_to_nanboxed(v, store).map_err(|e| VMError::RuntimeError(e.to_string()))
            })
            .collect::<Result<Vec<_>, _>>()?;

        vm.call_stack = snapshot
            .call_stack
            .iter()
            .map(|f| {
                let upvalues = match &f.upvalues {
                    Some(values) => {
                        let mut out = Vec::new();
                        for v in values.iter() {
                            out.push(Upvalue::new(
                                serializable_to_nanboxed(v, store)
                                    .map_err(|e| VMError::RuntimeError(e.to_string()))?,
                            ));
                        }
                        Some(out)
                    }
                    None => None,
                };
                // Restore blob_hash from the snapshot frame. Use the shared
                // hash-first resolution helper for strict validation.
                let blob_hash = f.blob_hash.map(FunctionHash);
                let resolved_function_id = if blob_hash.is_some() || f.function_id.is_some() {
                    Some(resolve_function_identity(
                        &vm.function_id_by_hash,
                        &vm.program.functions,
                        blob_hash,
                        f.function_id,
                        None,
                    )?)
                } else {
                    None
                };

                let return_ip = if let (Some(hash), Some(local_ip), Some(fid)) =
                    (&blob_hash, f.local_ip, resolved_function_id)
                {
                    // Validate the blob hash matches the loaded program
                    let current_hash = vm.blob_hash_for_function(fid);
                    if let Some(current) = current_hash
                        && current != *hash
                    {
                        return Err(VMError::RuntimeError(format!(
                            "Snapshot blob hash mismatch for function {}: \
                             snapshot has {}, program has {}",
                            fid, hash, current
                        )));
                    }
                    // Reconstruct absolute IP from local_ip + entry_point
                    let entry_point = vm
                        .function_entry_points
                        .get(fid as usize)
                        .copied()
                        .unwrap_or(0);
                    local_ip + entry_point
                } else {
                    f.return_ip
                };

                Ok(CallFrame {
                    return_ip,
                    base_pointer: f.locals_base,
                    locals_count: f.locals_count,
                    function_id: resolved_function_id,
                    upvalues,
                    blob_hash,
                })
            })
            .collect::<Result<Vec<_>, VMError>>()?;

        vm.loop_stack = snapshot
            .loop_stack
            .iter()
            .map(|l| LoopContext {
                start: l.start,
                end: l.end,
            })
            .collect();
        vm.timeframe_stack = snapshot.timeframe_stack.clone();
        vm.exception_handlers = snapshot
            .exception_handlers
            .iter()
            .map(|h| ExceptionHandler {
                catch_ip: h.catch_ip,
                stack_size: h.stack_size,
                call_depth: h.call_depth,
            })
            .collect();

        Ok(vm)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Create a minimal Function with just a name (other fields defaulted).
    fn make_function(name: &str) -> Function {
        Function {
            name: name.to_string(),
            arity: 0,
            param_names: Vec::new(),
            locals_count: 0,
            entry_point: 0,
            body_length: 0,
            is_closure: false,
            captures_count: 0,
            is_async: false,
            ref_params: Vec::new(),
            ref_mutates: Vec::new(),
            mutable_captures: Vec::new(),
            frame_descriptor: None,
            osr_entry_points: Vec::new(),
            mir_data: None,
        }
    }

    fn make_hash(seed: u8) -> FunctionHash {
        FunctionHash([seed; 32])
    }

    #[test]
    fn test_resolve_by_hash() {
        let hash = make_hash(0xAB);
        let mut by_hash = HashMap::new();
        by_hash.insert(hash, 3u16);
        let funcs = vec![
            make_function("a"),
            make_function("b"),
            make_function("c"),
            make_function("d"),
        ];

        let result = resolve_function_identity(&by_hash, &funcs, Some(hash), None, None);
        assert_eq!(result.unwrap(), 3);
    }

    #[test]
    fn test_resolve_hash_not_found_is_error() {
        let hash = make_hash(0xAB);
        let by_hash = HashMap::new(); // empty — hash not registered
        let funcs = vec![make_function("a")];

        let result = resolve_function_identity(&by_hash, &funcs, Some(hash), None, None);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("unknown function blob hash"), "got: {}", msg);
    }

    #[test]
    fn test_resolve_hash_function_id_mismatch_is_error() {
        let hash = make_hash(0xCD);
        let mut by_hash = HashMap::new();
        by_hash.insert(hash, 2u16); // hash resolves to 2
        let funcs = vec![make_function("a"), make_function("b"), make_function("c")];

        // Pass function_id=5 which disagrees with hash-resolved id=2
        let result = resolve_function_identity(&by_hash, &funcs, Some(hash), Some(5), None);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("mismatch"), "got: {}", msg);
    }

    #[test]
    fn test_resolve_hash_function_id_agree() {
        let hash = make_hash(0xEF);
        let mut by_hash = HashMap::new();
        by_hash.insert(hash, 1u16);
        let funcs = vec![make_function("a"), make_function("b")];

        // Both agree on id=1
        let result = resolve_function_identity(&by_hash, &funcs, Some(hash), Some(1), None);
        assert_eq!(result.unwrap(), 1);
    }

    #[test]
    fn test_resolve_by_function_id() {
        let by_hash = HashMap::new();
        let funcs = vec![make_function("a"), make_function("b"), make_function("c")];

        let result = resolve_function_identity(&by_hash, &funcs, None, Some(2), None);
        assert_eq!(result.unwrap(), 2);
    }

    #[test]
    fn test_resolve_function_id_out_of_range() {
        let by_hash = HashMap::new();
        let funcs = vec![make_function("a")];

        let result = resolve_function_identity(&by_hash, &funcs, None, Some(99), None);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("out of range"), "got: {}", msg);
    }

    #[test]
    fn test_resolve_unique_name_fallback() {
        let by_hash = HashMap::new();
        let funcs = vec![
            make_function("alpha"),
            make_function("beta"),
            make_function("gamma"),
        ];

        let result = resolve_function_identity(&by_hash, &funcs, None, None, Some("beta"));
        assert_eq!(result.unwrap(), 1);
    }

    #[test]
    fn test_resolve_ambiguous_name_is_error() {
        let by_hash = HashMap::new();
        let funcs = vec![
            make_function("dup"),
            make_function("other"),
            make_function("dup"),
        ];

        let result = resolve_function_identity(&by_hash, &funcs, None, None, Some("dup"));
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("ambiguous"), "got: {}", msg);
    }

    #[test]
    fn test_resolve_name_not_found() {
        let by_hash = HashMap::new();
        let funcs = vec![make_function("a")];

        let result = resolve_function_identity(&by_hash, &funcs, None, None, Some("missing"));
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("no function named"), "got: {}", msg);
    }

    #[test]
    fn test_resolve_no_identifiers_is_error() {
        let by_hash = HashMap::new();
        let funcs = vec![make_function("a")];

        let result = resolve_function_identity(&by_hash, &funcs, None, None, None);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("no hash, id, or name"), "got: {}", msg);
    }

    // --- VmSnapshot IP relocation tests ---

    #[test]
    fn test_snapshot_ip_relocation_fields_present() {
        // Verify that VmSnapshot has the new relocation fields
        let snapshot = VmSnapshot {
            ip: 42,
            stack: vec![],
            locals: vec![],
            module_bindings: vec![],
            call_stack: vec![],
            loop_stack: vec![],
            timeframe_stack: vec![],
            exception_handlers: vec![],
            ip_blob_hash: Some([0xAB; 32]),
            ip_local_offset: Some(10),
            ip_function_id: Some(1),
        };
        assert_eq!(snapshot.ip, 42);
        assert_eq!(snapshot.ip_blob_hash, Some([0xAB; 32]));
        assert_eq!(snapshot.ip_local_offset, Some(10));
        assert_eq!(snapshot.ip_function_id, Some(1));
    }

    #[test]
    fn test_snapshot_legacy_without_relocation_fields() {
        // Legacy snapshots that don't have the new fields should still deserialize
        // (serde default kicks in)
        let snapshot = VmSnapshot {
            ip: 100,
            stack: vec![],
            locals: vec![],
            module_bindings: vec![],
            call_stack: vec![],
            loop_stack: vec![],
            timeframe_stack: vec![],
            exception_handlers: vec![],
            ip_blob_hash: None,
            ip_local_offset: None,
            ip_function_id: None,
        };
        // Without relocation info, from_snapshot should fall back to absolute IP
        assert!(snapshot.ip_blob_hash.is_none());
        assert!(snapshot.ip_local_offset.is_none());
        assert!(snapshot.ip_function_id.is_none());
    }

    #[test]
    fn test_snapshot_serialization_roundtrip_with_relocation() {
        let snapshot = VmSnapshot {
            ip: 42,
            stack: vec![],
            locals: vec![],
            module_bindings: vec![],
            call_stack: vec![],
            loop_stack: vec![],
            timeframe_stack: vec![],
            exception_handlers: vec![],
            ip_blob_hash: Some([0xCD; 32]),
            ip_local_offset: Some(7),
            ip_function_id: Some(2),
        };
        let json = serde_json::to_string(&snapshot).unwrap();
        let restored: VmSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.ip_blob_hash, Some([0xCD; 32]));
        assert_eq!(restored.ip_local_offset, Some(7));
        assert_eq!(restored.ip_function_id, Some(2));
    }

    #[test]
    fn test_snapshot_deserialization_without_relocation_fields() {
        // Simulate a JSON snapshot from before the relocation fields were added
        let json = r#"{
            "ip": 50,
            "stack": [],
            "locals": [],
            "module_bindings": [],
            "call_stack": [],
            "loop_stack": [],
            "timeframe_stack": [],
            "exception_handlers": []
        }"#;
        let snapshot: VmSnapshot = serde_json::from_str(json).unwrap();
        assert_eq!(snapshot.ip, 50);
        assert!(snapshot.ip_blob_hash.is_none());
        assert!(snapshot.ip_local_offset.is_none());
        assert!(snapshot.ip_function_id.is_none());
    }
}
