//! VM snapshot and restore for suspending/resuming execution.
//!
//! # W17-snapshot-resume surface (ADR-006 §2.7.4 + §2.7.5.1)
//!
//! `snapshot()` and `from_snapshot()` previously consumed the slot-(de)
//! serialization helpers from `shape-runtime::snapshot` (and their `enum_*`
//! / `print_result_*` adapters). Those helpers were deleted alongside the
//! pre-bulldozer dynamic value carrier. The replacement — kind-threaded
//! `slot_to_serializable(bits, kind, store)` and its inverse, mirroring the
//! wire-conversion shape — is **deferred to a Phase 2c snapshot rebuild
//! session per ADR-006 §2.7.4**. The deferral is binding: papering over the
//! gap with a placeholder serializer or a hand-rolled byte format would
//! silently corrupt persisted state, which §2.7.4 forbids verbatim. Instead,
//! both methods return `VMError::NotImplemented` carrying a structured
//! W17-snapshot-resume surface string (see [`w17_snapshot_surface`]); the
//! prior `todo!()` macro-driven VM-thread abort is replaced with a
//! recoverable runtime error so callers can detect the missing capability
//! without crashing.
//!
//! `resolve_function_identity` is pure, value-tier-independent logic
//! (operates only on `FunctionHash` / `Function` / IDs) and is kept
//! intact. Its tests pass without exercising the snapshot pipeline.

use std::collections::HashMap;

use shape_value::VMError;

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

/// W17-snapshot-resume surface text for `VirtualMachine::snapshot()` /
/// `VirtualMachine::from_snapshot()`. Both methods return
/// `Result<..., VMError>`, so they return a structured
/// `VMError::NotImplemented` rather than panicking — the strict
/// improvement over the prior `todo!()` macros that aborted the VM
/// thread on first invocation.
fn w17_snapshot_surface(op: &str) -> String {
    format!(
        "VirtualMachine::{op}: W17-snapshot-resume surface — \
         kind-threaded `slot_to_serializable(bits, kind, store)` / \
         inverse `serializable_to_slot(sv, expected_kind, store)` \
         replacement for the deleted `nanboxed_to_serializable` / \
         `serializable_to_nanboxed` pair has not landed. The design \
         must (a) project every `NativeKind::Ptr(HeapKind::*)` slot to a \
         `SerializableVMValue` arm of the right shape via \
         `slot.as_heap_value()` + `HeapValue::*` match (§2.7.6 Q8 \
         carrier-API bound), (b) reconstruct the parallel kind tracks \
         from the persisted discriminator on restore (§2.7.7 / §2.7.8), \
         (c) extend `SerializableVMValue` for the post-W14/W15 \
         HeapKinds that have no current wire-format arm: HashSet, \
         Iterator, Result, Option, Deque, Channel, PriorityQueue, \
         Range, Reference, FilterExpr, SharedCell — the §2.7.5.1 \
         wire-format extension question. Tracked as W17-snapshot-resume \
         per docs/cluster-audits/phase-2d-playbook.md §3. ADR-006 \
         §2.7.4 + §2.7.5.1.",
    )
}

impl super::VirtualMachine {
    /// Create a serializable snapshot of VM state.
    ///
    /// **W17-snapshot-resume surface — see ADR-006 §2.7.4 + §2.7.5.1.**
    /// Returns `VMError::NotImplemented` until the kind-threaded
    /// `slot_to_serializable` lands. See [`w17_snapshot_surface`] for
    /// the structured error string.
    pub fn snapshot(
        &self,
        _store: &shape_runtime::snapshot::SnapshotStore,
    ) -> Result<shape_runtime::snapshot::VmSnapshot, VMError> {
        Err(VMError::NotImplemented(w17_snapshot_surface("snapshot")))
    }

    /// Restore a VM from a snapshot and bytecode program.
    ///
    /// **W17-snapshot-resume surface — see ADR-006 §2.7.4 + §2.7.5.1.**
    /// Symmetric to `snapshot()`: the deleted runtime-side
    /// slot-deserialization helper is replaced by a kind-threaded
    /// `serializable_to_slot(sv, expected_kind, store) -> (u64,
    /// NativeKind)` that reconstructs the parallel kind tracks from the
    /// persisted `SerializableVMValue` discriminator. B9-callframe-kind's
    /// `CallFrame.closure_heap_kind: Option<NativeKind>` companion is
    /// already in place to receive the threaded kind on the restore
    /// side. Returns `VMError::NotImplemented` until that lands.
    pub fn from_snapshot(
        _program: crate::bytecode::BytecodeProgram,
        _snapshot: &shape_runtime::snapshot::VmSnapshot,
        _store: &shape_runtime::snapshot::SnapshotStore,
    ) -> Result<Self, VMError> {
        Err(VMError::NotImplemented(w17_snapshot_surface(
            "from_snapshot",
        )))
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
    //
    // These tests exercise only the `VmSnapshot` data shape (field presence,
    // serde defaults, JSON roundtrip) and never call into `snapshot()` /
    // `from_snapshot()`. They pass even while the snapshot/restore pipeline
    // itself is `todo!()`-deferred per ADR-006 §2.7.4.

    use shape_runtime::snapshot::VmSnapshot;

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

    // -----------------------------------------------------------------
    // W17-snapshot-resume gate tests
    // -----------------------------------------------------------------
    //
    // VM-level `snapshot()` / `from_snapshot()` return a structured
    // `VMError::NotImplemented` carrying the W17 surface text, never
    // the pre-W17 `todo!()` panic that would abort the VM thread.

    use crate::VMConfig;
    use crate::executor::VirtualMachine;
    use shape_runtime::snapshot::SnapshotStore;

    #[test]
    fn test_w17_vm_snapshot_returns_structured_error() {
        let vm = VirtualMachine::new(VMConfig::default());
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = SnapshotStore::new(tmp.path()).expect("snapshot store");

        let result = vm.snapshot(&store);
        let err = match result {
            Ok(_) => panic!("expected Err(VMError::NotImplemented), got Ok"),
            Err(e) => e,
        };
        let msg = format!("{err:?}");
        assert!(
            msg.contains("W17-snapshot-resume surface"),
            "missing W17 marker; got: {msg}"
        );
        assert!(
            msg.contains("§2.7.4"),
            "missing ADR-006 §2.7.4 cite; got: {msg}"
        );
        assert!(
            msg.contains("§2.7.5.1"),
            "missing ADR-006 §2.7.5.1 cite; got: {msg}"
        );
    }
}
