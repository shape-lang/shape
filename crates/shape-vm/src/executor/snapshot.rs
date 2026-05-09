//! VM snapshot and restore for suspending/resuming execution.
//!
//! # Phase-2c deferral (ADR-006 §2.7.4)
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
//! both methods panic loudly via `todo!()` so the broken capability surfaces
//! rather than masquerading as a working roundtrip.
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

impl super::VirtualMachine {
    /// Create a serializable snapshot of VM state.
    ///
    /// **Phase-2c rebuild pending — see ADR-006 §2.7.4.** The pre-bulldozer
    /// implementation walked the unified stack and module bindings via
    /// deleted Wave-6.5-substep-1 raw-bits readers and passed each result
    /// to the now-deleted runtime-side slot-serialization helpers. The
    /// post-§2.7.7 replacement reads `(bits, kind)` from the parallel kind
    /// tracks and dispatches a kind-threaded
    /// `slot_to_serializable(bits, kind, store)`. That helper is part of
    /// the §2.7.4-deferred Phase-2c snapshot rebuild — its design must
    /// account for `HeapKind` payload variants (TypedArray, TypedObject,
    /// HashMap, Decimal, BigInt, ...) that each need their own serialized
    /// shape, and for upvalue capture state that depends on §2.7.8
    /// cell-storage kind-awareness. Until that lands, this method panics
    /// rather than emitting a placeholder serialization that would silently
    /// corrupt persisted state on round-trip.
    pub fn snapshot(
        &self,
        _store: &shape_runtime::snapshot::SnapshotStore,
    ) -> Result<shape_runtime::snapshot::VmSnapshot, VMError> {
        todo!("phase-2c snapshot rebuild — see ADR-006 §2.7.4")
    }

    /// Restore a VM from a snapshot and bytecode program.
    ///
    /// **Phase-2c rebuild pending — see ADR-006 §2.7.4.** Symmetric to
    /// `snapshot()`: the deleted runtime-side slot-deserialization helper
    /// is replaced by a kind-threaded `serializable_to_slot(sv,
    /// expected_kind, store) -> (u64, NativeKind)` that reconstructs the
    /// parallel kind tracks from the persisted `SerializableVMValue`
    /// discriminator. The design and consumer migration are Phase-2c scope
    /// per ADR-006 §2.7.4. This method panics until the rebuild lands.
    pub fn from_snapshot(
        _program: crate::bytecode::BytecodeProgram,
        _snapshot: &shape_runtime::snapshot::VmSnapshot,
        _store: &shape_runtime::snapshot::SnapshotStore,
    ) -> Result<Self, VMError> {
        // The snapshot/restore body is deferred to Phase 2c per ADR-006
        // §2.7.4. B9-callframe-kind's CallFrame.closure_heap_kind: Option<NativeKind>
        // companion (added in this same merge round) will be threaded
        // through the rebuild path when Phase 2c lands; for now the entire
        // body panics until the snapshot rebuild is reimplemented against
        // the kinded VM state.
        todo!("phase-2c snapshot rebuild — see ADR-006 §2.7.4")
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
}
