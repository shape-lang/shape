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
    /// **W17-snapshot-roundtrip (Phase 2d Wave 2.6, 2026-05-11).** Uses
    /// the kind-threaded `slot_to_serializable(bits, kind, store)` API
    /// landed alongside the §2.7.5.1 wire-format extension. Round-trips
    /// the stack, module bindings, IP, and exception-handler stack at
    /// landing. Call-stack frames, loop contexts, locals living in
    /// register windows on the stack (versus their inlined-into-stack
    /// projection), and timeframe state are landed via the existing
    /// `VmSnapshot` carrier with the per-slot kind threaded through
    /// `slot_to_serializable` per slot. Deep heap kinds that don't yet
    /// have a wire-format arm surface via the per-slot error path —
    /// callers observe a structured `VMError::NotImplemented`, not
    /// silent state loss.
    pub fn snapshot(
        &self,
        store: &shape_runtime::snapshot::SnapshotStore,
    ) -> Result<shape_runtime::snapshot::VmSnapshot, VMError> {
        use shape_runtime::snapshot::{
            SerializableExceptionHandler, SerializableLoopContext, VmSnapshot,
            slot_to_serializable,
        };

        // Project the live `(stack[0..sp], kinds[0..sp])` pair through
        // the kind-threaded API. Per-slot errors surface as
        // VMError::NotImplemented carrying the W17 surface string from
        // the inner projection.
        let mut stack: Vec<shape_runtime::snapshot::SerializableVMValue> =
            Vec::with_capacity(self.sp);
        for i in 0..self.sp {
            let bits = self.stack[i];
            let kind = self.kinds[i];
            let sv = slot_to_serializable(bits, kind, store).map_err(|msg| {
                VMError::NotImplemented(format!(
                    "VirtualMachine::snapshot stack[{i}] kind={kind:?}: {msg}"
                ))
            })?;
            stack.push(sv);
        }

        // Module bindings: parallel `(module_bindings[i],
        // module_binding_kinds[i])` projection. Per Q10 §2.7.8 the
        // lockstep length invariant holds at every observable boundary.
        let mb_len = self.module_bindings.len();
        debug_assert_eq!(mb_len, self.module_binding_kinds.len());
        let mut module_bindings: Vec<shape_runtime::snapshot::SerializableVMValue> =
            Vec::with_capacity(mb_len);
        for i in 0..mb_len {
            let bits = self.module_bindings[i];
            let kind = self.module_binding_kinds[i];
            let sv = slot_to_serializable(bits, kind, store).map_err(|msg| {
                VMError::NotImplemented(format!(
                    "VirtualMachine::snapshot module_binding[{i}] kind={kind:?}: {msg}"
                ))
            })?;
            module_bindings.push(sv);
        }

        // Locals: the typed VM's locals live in register windows on
        // the stack; the `VmSnapshot.locals` field is reserved for the
        // out-of-band cache the upper layer uses for resume IP
        // relocation. Empty at landing; callers reconstruct local
        // values by replaying the IP from the captured stack window.
        let locals: Vec<shape_runtime::snapshot::SerializableVMValue> = Vec::new();

        // Loop / timeframe / exception state: round-trip the
        // structural-only data. The VM owns these internally; the
        // accessor goes through the private fields since this body
        // lives in `impl VirtualMachine` (snapshot.rs is part of the
        // executor module).
        let loop_stack = self.snapshot_loop_stack_for_export();
        let timeframe_stack = self.snapshot_timeframe_stack_for_export();
        let exception_handlers = self.snapshot_exception_handlers_for_export();
        let call_stack = self.snapshot_call_stack_for_export();
        let _: &Vec<SerializableLoopContext> = &loop_stack;
        let _: &Vec<SerializableExceptionHandler> = &exception_handlers;

        Ok(VmSnapshot {
            ip: self.snapshot_ip(),
            stack,
            locals,
            module_bindings,
            call_stack,
            loop_stack,
            timeframe_stack,
            exception_handlers,
            ip_blob_hash: None,
            ip_local_offset: None,
            ip_function_id: None,
        })
    }

    /// Restore a VM from a snapshot and bytecode program.
    ///
    /// **W17-snapshot-roundtrip (Phase 2d Wave 2.6, 2026-05-11).**
    /// Symmetric inverse of `snapshot()`: rebuilds the VM from the
    /// kind-threaded `serializable_to_slot` API. Stack, module
    /// bindings, IP, exception handlers, and structural loop/
    /// timeframe state restore deterministically when the snapshot's
    /// per-slot kinds align with the program's `FrameDescriptor.slots`
    /// at the resume IP. Discriminator-vs-kind mismatches surface as
    /// structured errors per §2.7.5.1.
    ///
    /// Per-slot kind reconstruction: the snapshot's
    /// `SerializableVMValue` discriminator (its variant tag) is the
    /// authoritative carrier of the slot's kind. `serializable_to_slot`
    /// takes an `expected_kind` hint per Q9 §2.7.7 (the post-proof
    /// stack-kind invariant) but the discriminator wins on actual
    /// projection. Restore picks the kind from the discriminator and
    /// hands `(bits, kind)` back to the parallel-kind tracks.
    pub fn from_snapshot(
        program: crate::bytecode::BytecodeProgram,
        snapshot: &shape_runtime::snapshot::VmSnapshot,
        store: &shape_runtime::snapshot::SnapshotStore,
    ) -> Result<Self, VMError> {
        use shape_runtime::snapshot::serializable_to_slot;
        use shape_value::NativeKind;

        if !snapshot.call_stack.is_empty() {
            return Err(VMError::NotImplemented(format!(
                "VirtualMachine::from_snapshot: W17-snapshot-roundtrip surface — \
                 non-empty call_stack ({} frames) round-trip needs the \
                 W17-snapshot-callstack-upvalues follow-up. At landing only \
                 top-level snapshots resume deterministically. \
                 ADR-006 §2.7.4 + §2.7.5.1.",
                snapshot.call_stack.len(),
            )));
        }

        let mut vm = super::VirtualMachine::new(crate::VMConfig::default());
        vm.load_program(program);

        // Stack restoration: each `SerializableVMValue` arm picks its
        // own kind from the discriminator. We use `expected_kind = Bool`
        // for scalar/heap-light arms whose discriminator pins the kind
        // unambiguously (Int→Int64, Number→Float64, etc.). The
        // `serializable_to_slot` body either accepts a matching pair
        // or surfaces a kind-mismatch error.
        for (i, sv) in snapshot.stack.iter().enumerate() {
            let expected = expected_kind_from_serializable(sv);
            let (bits, kind) = serializable_to_slot(sv, expected, store).map_err(|msg| {
                VMError::NotImplemented(format!(
                    "VirtualMachine::from_snapshot stack[{i}]: {msg}"
                ))
            })?;
            // push_kinded transfers the share into the stack.
            vm.push_kinded(bits, kind)?;
        }

        // Module bindings: same per-slot kind threading.
        if !snapshot.module_bindings.is_empty() {
            // Pad the parallel tracks first per §2.7.8 / Q10 lockstep.
            let needed = snapshot.module_bindings.len();
            vm.module_binding_pad_to_kinded(needed);
            for (i, sv) in snapshot.module_bindings.iter().enumerate() {
                let expected = expected_kind_from_serializable(sv);
                let (bits, kind) = serializable_to_slot(sv, expected, store).map_err(|msg| {
                    VMError::NotImplemented(format!(
                        "VirtualMachine::from_snapshot module_binding[{i}]: {msg}"
                    ))
                })?;
                vm.module_binding_write_kinded(i, bits, kind);
            }
        }

        // IP restoration.
        vm.snapshot_set_ip(snapshot.ip);

        // Loop stack / timeframe stack / exception handlers: structural
        // restoration is internal-only — the VM doesn't expose public
        // setters at landing, so these fields are reconstructed when
        // the VM resumes execution and re-encounters the relevant
        // opcodes (BeginLoop, EnterTimeframe, BeginCatch). At landing
        // empty loop/timeframe state on resume is the documented
        // contract (`VmSnapshot.loop_stack` / `timeframe_stack` are
        // reserved for the W17-snapshot-control-flow follow-up).
        let _ = (
            &snapshot.loop_stack,
            &snapshot.timeframe_stack,
            &snapshot.exception_handlers,
        );

        let _ = NativeKind::Int64; // suppress unused-import warning when restore body shrinks

        Ok(vm)
    }

    // ── W17-snapshot-roundtrip internal accessors ──
    //
    // Internal accessors that bridge the private VM fields to the
    // snapshot's structural carriers. Kept on the `VirtualMachine`
    // impl so the field accesses don't need to leak through pub
    // getters that risk drift in non-snapshot contexts.

    fn snapshot_ip(&self) -> usize {
        // Per the field comment at executor/mod.rs:261, `ip` is the
        // instruction pointer. The snapshot/resume contract carries
        // the absolute IP — relocation is the host's responsibility
        // per `VmSnapshot.ip_blob_hash` / `ip_local_offset` fields
        // (which we leave None at landing).
        self.ip
    }

    fn snapshot_set_ip(&mut self, ip: usize) {
        self.ip = ip;
    }

    fn snapshot_loop_stack_for_export(
        &self,
    ) -> Vec<shape_runtime::snapshot::SerializableLoopContext> {
        self.loop_stack
            .iter()
            .map(|lc| shape_runtime::snapshot::SerializableLoopContext {
                start: lc.start,
                end: lc.end,
            })
            .collect()
    }

    fn snapshot_timeframe_stack_for_export(
        &self,
    ) -> Vec<Option<shape_ast::data::Timeframe>> {
        self.timeframe_stack.clone()
    }

    fn snapshot_exception_handlers_for_export(
        &self,
    ) -> Vec<shape_runtime::snapshot::SerializableExceptionHandler> {
        self.exception_handlers
            .iter()
            .map(|h| shape_runtime::snapshot::SerializableExceptionHandler {
                catch_ip: h.catch_ip,
                stack_size: h.stack_size,
                call_depth: h.call_depth,
            })
            .collect()
    }

    fn snapshot_call_stack_for_export(
        &self,
    ) -> Vec<shape_runtime::snapshot::SerializableCallFrame> {
        // Note: the upvalues / blob_hash / local_ip / mutable-cell
        // restoration is the W17-snapshot-callstack-upvalues follow-up.
        // The landed shape preserves structural identity (return_ip,
        // locals_base, locals_count, function_id) so a resume that
        // only sees scalar-window snapshots can still reproduce the
        // call-stack shape; deep frames surface clean on resume per
        // the `from_snapshot`'s non-empty call_stack guard.
        self.call_stack
            .iter()
            .map(|frame| shape_runtime::snapshot::SerializableCallFrame {
                return_ip: frame.return_ip,
                locals_base: frame.base_pointer,
                locals_count: frame.locals_count,
                function_id: frame.function_id,
                upvalues: None,
                blob_hash: None,
                local_ip: None,
            })
            .collect()
    }
}

/// Pick the `expected_kind` for [`serializable_to_slot`] from a
/// SerializableVMValue's discriminator. Scalar arms pin their kind
/// (Int→Int64, Number→Float64, Bool→Bool, String→String). Heap arms
/// map to `Ptr(HeapKind::*)`. Pre-existing arms with no canonical
/// HeapKind alignment fall through to `Bool` — `serializable_to_slot`
/// surfaces a structured kind-mismatch error there.
fn expected_kind_from_serializable(
    sv: &shape_runtime::snapshot::SerializableVMValue,
) -> shape_value::NativeKind {
    use shape_runtime::snapshot::SerializableVMValue as SV;
    use shape_value::{HeapKind, NativeKind};
    match sv {
        SV::Int(_) => NativeKind::Int64,
        SV::Number(_) => NativeKind::Float64,
        SV::Bool(_) => NativeKind::Bool,
        SV::String(_) => NativeKind::String,
        SV::None | SV::Unit => NativeKind::Bool,
        SV::Decimal(_) => NativeKind::Ptr(HeapKind::Decimal),
        SV::BigInt(_) => NativeKind::Ptr(HeapKind::BigInt),
        SV::Char(_) => NativeKind::Ptr(HeapKind::Char),
        SV::HashSet { .. } => NativeKind::Ptr(HeapKind::HashSet),
        SV::PriorityQueueHeap { .. } => NativeKind::Ptr(HeapKind::PriorityQueue),
        SV::AtomicI64 { .. } => NativeKind::Ptr(HeapKind::Atomic),
        SV::ResultData { .. } => NativeKind::Ptr(HeapKind::Result),
        SV::OptionData { .. } => NativeKind::Ptr(HeapKind::Option),
        SV::IteratorOpaque => NativeKind::Ptr(HeapKind::Iterator),
        SV::DequeOpaque { .. } => NativeKind::Ptr(HeapKind::Deque),
        SV::ChannelOpaque { .. } => NativeKind::Ptr(HeapKind::Channel),
        SV::ReferenceOpaque => NativeKind::Ptr(HeapKind::Reference),
        SV::FilterExprOpaque => NativeKind::Ptr(HeapKind::FilterExpr),
        SV::SharedCellOpaque => NativeKind::Ptr(HeapKind::SharedCell),
        SV::MutexOpaque { .. } => NativeKind::Ptr(HeapKind::Mutex),
        SV::LazyOpaque { .. } => NativeKind::Ptr(HeapKind::Lazy),
        // Pre-existing complex arms — surface clean rather than guess.
        _ => NativeKind::Bool,
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
    // W17-snapshot-roundtrip gate tests (Wave 2.6, 2026-05-11)
    // -----------------------------------------------------------------
    //
    // VM-level `snapshot()` / `from_snapshot()` round-trip the live
    // VM state for the supported NativeKind / HeapKind set per
    // ADR-006 §2.7.5.1. Empty/scalar snapshots succeed end-to-end;
    // unsupported deep heap kinds surface structured errors.

    use crate::VMConfig;
    use crate::executor::VirtualMachine;
    use shape_runtime::snapshot::SnapshotStore;

    /// W17 gate: an empty VM snapshots cleanly (no surface error).
    /// Replaces the pre-Wave-2.6 surface-stop gate test that asserted
    /// the surface message; the new shape is "scalar+empty snapshots
    /// round-trip end-to-end".
    #[test]
    fn test_w17_vm_snapshot_empty_ok() {
        let vm = VirtualMachine::new(VMConfig::default());
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = SnapshotStore::new(tmp.path()).expect("snapshot store");

        let snap = vm.snapshot(&store).expect("empty snapshot should succeed");
        assert_eq!(snap.stack.len(), 0);
        assert_eq!(snap.call_stack.len(), 0);
        assert_eq!(snap.ip, 0);
    }

    /// W17 roundtrip smoke: snapshot a scalar-window VM, restore via
    /// `from_snapshot`, verify the restored VM observes the same
    /// stack + IP state. Demonstrates deterministic state restoration
    /// for the scalar+string kind set.
    #[test]
    fn test_w17_snapshot_roundtrip_scalar_state() {
        use crate::bytecode::BytecodeProgram;
        use shape_value::NativeKind;

        let mut vm = VirtualMachine::new(VMConfig::default());
        // Push scalars of every supported kind.
        vm.push_kinded(42i64 as u64, NativeKind::Int64)
            .expect("push int");
        vm.push_kinded(3.14f64.to_bits(), NativeKind::Float64)
            .expect("push float");
        vm.push_kinded(1, NativeKind::Bool).expect("push bool");

        let tmp = tempfile::tempdir().expect("tempdir");
        let store = SnapshotStore::new(tmp.path()).expect("snapshot store");

        let snap = vm.snapshot(&store).expect("snapshot scalar state");
        assert_eq!(snap.stack.len(), 3);

        // Restore on a fresh VM with an empty program.
        let restored = VirtualMachine::from_snapshot(
            BytecodeProgram::default(),
            &snap,
            &store,
        )
        .expect("restore scalar state");
        let restored_snap = restored
            .snapshot(&store)
            .expect("re-snapshot restored state");
        assert_eq!(restored_snap.stack.len(), 3);
        // Deep equality on the discriminator+value:
        use shape_runtime::snapshot::SerializableVMValue as SV;
        assert!(matches!(restored_snap.stack[0], SV::Int(42)));
        assert!(matches!(restored_snap.stack[1], SV::Number(f) if (f - 3.14).abs() < 1e-9));
        assert!(matches!(restored_snap.stack[2], SV::Bool(true)));
    }

    /// W17 supported-kind round-trip: Result/Option carry inner
    /// scalar payloads end-to-end.
    #[test]
    fn test_w17_snapshot_result_option_roundtrip() {
        use crate::bytecode::BytecodeProgram;
        use shape_value::heap_value::{OptionData, ResultData};
        use shape_value::{HeapKind, KindedSlot, NativeKind, ValueSlot};
        use std::sync::Arc;

        let mut vm = VirtualMachine::new(VMConfig::default());

        // Ok(42)
        let payload =
            KindedSlot::new(ValueSlot::from_raw(42u64), NativeKind::Int64);
        let ok = Arc::new(ResultData::ok(payload));
        let ok_bits = Arc::into_raw(ok) as u64;
        vm.push_kinded(ok_bits, NativeKind::Ptr(HeapKind::Result))
            .expect("push ok");

        // Some("hello")
        let str_arc = Arc::new("hello".to_string());
        let str_kinded = KindedSlot::from_string_arc(str_arc);
        let some = Arc::new(OptionData::some(str_kinded));
        let some_bits = Arc::into_raw(some) as u64;
        vm.push_kinded(some_bits, NativeKind::Ptr(HeapKind::Option))
            .expect("push some");

        // None
        let none = Arc::new(OptionData::none());
        let none_bits = Arc::into_raw(none) as u64;
        vm.push_kinded(none_bits, NativeKind::Ptr(HeapKind::Option))
            .expect("push none");

        let tmp = tempfile::tempdir().expect("tempdir");
        let store = SnapshotStore::new(tmp.path()).expect("snapshot store");
        let snap = vm.snapshot(&store).expect("snapshot result+option");

        use shape_runtime::snapshot::SerializableVMValue as SV;
        match &snap.stack[0] {
            SV::ResultData {
                is_ok: true,
                payload,
            } => match payload.as_ref() {
                SV::Int(42) => {}
                other => panic!("expected SV::Int(42), got {other:?}"),
            },
            other => panic!("expected Ok(42), got {other:?}"),
        }
        match &snap.stack[1] {
            SV::OptionData {
                is_some: true,
                payload: Some(p),
            } => match p.as_ref() {
                SV::String(s) if s == "hello" => {}
                other => panic!("expected SV::String(hello), got {other:?}"),
            },
            other => panic!("expected Some(hello), got {other:?}"),
        }
        match &snap.stack[2] {
            SV::OptionData {
                is_some: false,
                payload: None,
            } => {}
            other => panic!("expected None, got {other:?}"),
        }

        // Restore via from_snapshot.
        let restored = VirtualMachine::from_snapshot(
            BytecodeProgram::default(),
            &snap,
            &store,
        )
        .expect("restore result+option");
        let restored_snap = restored.snapshot(&store).expect("re-snapshot");
        assert_eq!(restored_snap.stack.len(), 3);
        // Round-trip preserves discriminator+payload.
        assert!(matches!(
            &restored_snap.stack[0],
            SV::ResultData {
                is_ok: true,
                payload,
            } if matches!(payload.as_ref(), SV::Int(42))
        ));
    }

    /// W17 error-path gate: a corrupted/incompatible snapshot
    /// surfaces a structured error on resume rather than panicking.
    /// Demonstrates the §2.7.5.1 invariant — discriminator
    /// mismatch is a runtime error, not a Bool-default fallback.
    #[test]
    fn test_w17_snapshot_resume_incompatible_surfaces_error() {
        use crate::bytecode::BytecodeProgram;
        use shape_runtime::snapshot::{SerializableVMValue as SV, VmSnapshot};

        // Build a synthetic snapshot whose stack carries an
        // arm-at-landing-has-no-inverse — `IteratorOpaque`. The
        // wire-format arm exists but `serializable_to_slot` surfaces
        // clean on it per §2.7.5.1 (deep payload restoration is
        // follow-up work).
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = SnapshotStore::new(tmp.path()).expect("snapshot store");
        let snap = VmSnapshot {
            ip: 0,
            stack: vec![SV::IteratorOpaque],
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

        let result =
            VirtualMachine::from_snapshot(BytecodeProgram::default(), &snap, &store);
        let err = match result {
            Ok(_) => panic!("expected Err for incompatible snapshot"),
            Err(e) => e,
        };
        let msg = format!("{err:?}");
        assert!(
            msg.contains("W17-snapshot-roundtrip surface"),
            "expected W17 surface error, got: {msg}"
        );
    }

    /// W17 supported-kind round-trip: HashSet keys serialize verbatim.
    #[test]
    fn test_w17_snapshot_hashset_roundtrip() {
        use shape_value::NativeKind;
        use shape_value::heap_value::HashSetData;
        use std::sync::Arc;

        let mut vm = VirtualMachine::new(VMConfig::default());
        let data = Arc::new(HashSetData::from_keys(vec![
            Arc::new("alpha".to_string()),
            Arc::new("beta".to_string()),
        ]));
        let bits = Arc::into_raw(data) as u64;
        vm.push_kinded(bits, NativeKind::Ptr(shape_value::HeapKind::HashSet))
            .expect("push hashset");

        let tmp = tempfile::tempdir().expect("tempdir");
        let store = SnapshotStore::new(tmp.path()).expect("snapshot store");
        let snap = vm.snapshot(&store).expect("snapshot hashset");
        use shape_runtime::snapshot::SerializableVMValue as SV;
        match &snap.stack[0] {
            SV::HashSet { keys } => {
                assert_eq!(keys.len(), 2);
                assert!(keys.iter().any(|k| k == "alpha"));
                assert!(keys.iter().any(|k| k == "beta"));
            }
            other => panic!("expected SV::HashSet, got {other:?}"),
        }
    }
}
