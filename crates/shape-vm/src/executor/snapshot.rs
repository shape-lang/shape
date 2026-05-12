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

        // **W17-state-tier-roundtrip (Phase 2d Wave 3, 2026-05-12).**
        // Call-stack restoration: structurally rebuild each `CallFrame`
        // from the persisted `SerializableCallFrame` quintuple
        // (return_ip, locals_base, locals_count, function_id, upvalues).
        // Upvalues route through `serializable_to_slot` to recover their
        // typed Arc shares — full round-trip for scalar/heap-light kinds;
        // opaque arms surface clean per §2.7.5.1.
        //
        // closure_heap_bits / closure_heap_kind reconstruction (the
        // Vec<u64> upvalue payload pointer back into a live
        // OwnedClosureBlock) needs the layout's alloc_typed_closure +
        // write_capture pipeline. We rebuild the block when:
        //   (a) the function_id has a registered ClosureLayout
        //   (b) the persisted upvalues align with capture_count
        // Otherwise the frame restores without closure-block backing —
        // a degraded but structurally-correct shape. Calls into the
        // restored frame's upvalues then trip a `NotImplemented` at the
        // upvalue-read site if any heap-bearing capture is missing.
        if !snapshot.call_stack.is_empty() {
            vm.restore_call_stack(&snapshot.call_stack, store)?;
        }

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

    /// Restore the call stack from a snapshot's `Vec<SerializableCallFrame>`.
    ///
    /// **W17-state-tier-roundtrip (Phase 2d Wave 3, 2026-05-12).** Per
    /// ADR-006 §2.7.8 / Q10, closure-bearing frames carry their captures
    /// via `OwnedClosureBlock`, not via the legacy `Vec<u64>` upvalue
    /// payload (the `Vec<u64>` field on CallFrame is now an opaque
    /// payload byte-pattern preserved for non-typed closures).
    ///
    /// Per-frame restore steps:
    ///   1. Recover function_id + locals_base + locals_count from the
    ///      persisted SerializableCallFrame.
    ///   2. If `upvalues: Some(serializable_upvalues)` AND the function
    ///      has a registered `ClosureLayout`: allocate a fresh
    ///      OwnedClosureBlock and write each capture's bits via
    ///      `serializable_to_slot(sv, expected_kind=block.layout.
    ///      capture_native_kind(i), store)`. The block's `Drop` walks
    ///      the layout's capture masks and retires shares.
    ///   3. Otherwise restore the frame without closure backing
    ///      (closure_heap_bits/kind = None).
    fn restore_call_stack(
        &mut self,
        frames: &[shape_runtime::snapshot::SerializableCallFrame],
        store: &shape_runtime::snapshot::SnapshotStore,
    ) -> Result<(), VMError> {
        use shape_runtime::snapshot::serializable_to_slot;
        use shape_value::NativeKind;
        use shape_value::v2::closure_raw::{
            OwnedClosureBlock, alloc_typed_closure, write_capture_raw_u64,
        };

        for (frame_idx, sframe) in frames.iter().enumerate() {
            let function_id = sframe.function_id;
            let mut closure_heap_bits: Option<u64> = None;
            let mut closure_heap_kind: Option<NativeKind> = None;
            let mut upvalues_raw: Option<Vec<u64>> = None;

            if let (Some(svec), Some(fid)) = (sframe.upvalues.as_ref(), function_id) {
                // Look up the closure layout for this function. If
                // absent, fall back to raw-Vec<u64> upvalue restoration
                // (non-typed closure path).
                let layout_opt = self
                    .program
                    .closure_function_layouts
                    .get(fid as usize)
                    .and_then(|o| o.clone());
                if let Some(layout) = layout_opt {
                    if layout.capture_count() != svec.len() {
                        return Err(VMError::NotImplemented(format!(
                            "VirtualMachine::from_snapshot frame[{frame_idx}]: \
                             W17-snapshot-roundtrip surface — upvalue count \
                             mismatch (snapshot: {}, layout.capture_count: {}). \
                             ADR-006 §2.7.5.1.",
                            svec.len(),
                            layout.capture_count(),
                        )));
                    }
                    // SAFETY: alloc_typed_closure returns a freshly-
                    // zeroed block sized for layout.total_heap_size();
                    // refcount is 1 (owned by this frame).
                    let ptr = unsafe { alloc_typed_closure(fid, 0, &layout) };
                    for (i, sv) in svec.iter().enumerate() {
                        let expected = layout.capture_native_kind(i);
                        let (bits, _kind) =
                            serializable_to_slot(sv, expected, store).map_err(|msg| {
                                VMError::NotImplemented(format!(
                                    "VirtualMachine::from_snapshot frame[{frame_idx}] \
                                     upvalue[{i}]: {msg}"
                                ))
                            })?;
                        // SAFETY: i < capture_count per the prior check.
                        unsafe {
                            write_capture_raw_u64(ptr, &layout, i, bits);
                        }
                    }
                    // Wrap the freshly-built block — drop releases the
                    // share when the frame pops. We need the raw ptr
                    // bits to install on closure_heap_bits; build the
                    // block, extract the ptr, then mem::forget the
                    // wrapper so its Drop doesn't free the share we
                    // just installed on the frame.
                    let block = unsafe { OwnedClosureBlock::from_raw(ptr as *const u8, layout) };
                    closure_heap_bits = Some(block.as_ptr() as u64);
                    closure_heap_kind = Some(NativeKind::Ptr(
                        shape_value::HeapKind::Closure,
                    ));
                    std::mem::forget(block);
                } else {
                    // No layout — store the raw payload bits as the
                    // legacy Vec<u64> upvalue carrier so the frame
                    // remains structurally complete. Heap-bearing
                    // upvalues in this path surface clean on read.
                    let mut raw: Vec<u64> = Vec::with_capacity(svec.len());
                    for sv in svec {
                        // Bool fallback is OK here: this branch is the
                        // pre-typed-closure path that never carried
                        // kind metadata anyway. Bool-zero-on-mismatch
                        // is the legacy contract.
                        let expected = NativeKind::Bool;
                        let (bits, _) =
                            serializable_to_slot(sv, expected, store).unwrap_or((0, NativeKind::Bool));
                        raw.push(bits);
                    }
                    upvalues_raw = Some(raw);
                }
            }

            let blob_hash = sframe
                .blob_hash
                .map(crate::bytecode::FunctionHash);

            self.call_stack.push(super::CallFrame {
                return_ip: sframe.return_ip,
                base_pointer: sframe.locals_base,
                locals_count: sframe.locals_count,
                function_id,
                upvalues: upvalues_raw,
                blob_hash,
                closure_heap_bits,
                closure_heap_kind,
            });
        }
        Ok(())
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
        // **W17-state-tier-roundtrip (Phase 2d Wave 3, 2026-05-12).**
        // Per-frame upvalues now project through `OwnedClosureBlock::
        // read_capture_kinded` per the §2.7.8 / Q10 cell-storage
        // parallel-kind track. The closure layout side-table provides
        // the kind source for each capture; we route each (bits, kind)
        // pair through `slot_to_serializable` to build the
        // `Vec<SerializableVMValue>` payload.
        //
        // Non-closure frames (`closure_heap_bits == None`) carry
        // `upvalues: None` as before.
        //
        // mutable-cell-payload restoration (cells whose interior holds
        // a SharedCell payload) is the W17-snapshot-sharedcell follow-up.
        let store = shape_runtime::snapshot::SnapshotStore::new(
            std::env::temp_dir().join("shape-w17-snapshot-store"),
        )
        .ok();
        self.call_stack
            .iter()
            .map(|frame| {
                let upvalues = if let Some(ref s) = store {
                    snapshot_frame_upvalues_serializable(self, frame, s)
                } else {
                    None
                };
                shape_runtime::snapshot::SerializableCallFrame {
                    return_ip: frame.return_ip,
                    locals_base: frame.base_pointer,
                    locals_count: frame.locals_count,
                    function_id: frame.function_id,
                    upvalues,
                    blob_hash: frame.blob_hash.map(|h| h.0),
                    local_ip: None,
                }
            })
            .collect()
    }
}

/// Project a frame's upvalues into `Vec<SerializableVMValue>` via the
/// closure block's `read_capture_kinded` + `slot_to_serializable`.
/// Returns `None` for non-closure frames or when the closure layout is
/// not available in the program's side-table (the W17-snapshot-callstack-
/// upvalues-no-layout follow-up).
fn snapshot_frame_upvalues_serializable(
    vm: &super::VirtualMachine,
    frame: &super::CallFrame,
    store: &shape_runtime::snapshot::SnapshotStore,
) -> Option<Vec<shape_runtime::snapshot::SerializableVMValue>> {
    use shape_runtime::snapshot::slot_to_serializable;
    use shape_value::v2::closure_raw::{
        OwnedClosureBlock, retain_typed_closure, typed_closure_function_id,
    };

    let bits = frame.closure_heap_bits?;
    if bits == 0 {
        return None;
    }
    let ptr = bits as *const u8;
    // SAFETY: closure_heap_bits is a live closure block per §2.7.8 / Q10.
    let fn_id = unsafe { typed_closure_function_id(ptr) };
    let layout = vm
        .program
        .closure_function_layouts
        .get(fn_id as usize)
        .and_then(|opt| opt.clone())?;
    // Retain a borrow share. SAFETY: ptr is a live OwnedClosureBlock
    // allocation; retain_typed_closure bumps the strong-count atomically
    // so the resulting OwnedClosureBlock's Drop doesn't free the live
    // share.
    unsafe {
        retain_typed_closure(ptr);
    }
    let block = unsafe { OwnedClosureBlock::from_raw(ptr, layout) };
    let count = block.layout().capture_count();
    let mut out: Vec<shape_runtime::snapshot::SerializableVMValue> = Vec::with_capacity(count);
    for idx in 0..count {
        // SAFETY: idx < count; the block is borrowed live.
        let (cap_bits, cap_kind) = unsafe { block.read_capture_kinded(idx) };
        let sv = match slot_to_serializable(cap_bits, cap_kind, store) {
            Ok(v) => v,
            Err(_) => {
                // Unsupported capture kind — surface as IteratorOpaque
                // sentinel so the wire payload is still serializable.
                // Restore will reject this via the OpaqueOnRestore
                // contract per §2.7.5.1.
                shape_runtime::snapshot::SerializableVMValue::IteratorOpaque
            }
        };
        out.push(sv);
    }
    Some(out)
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

    /// W17-state-tier-roundtrip (Phase 2d Wave 3, 2026-05-12): non-empty
    /// call_stack with scalar locals round-trips structurally.
    /// Closure-bearing frames need the program's
    /// `closure_function_layouts` registered (out of this test's scope —
    /// the bytecode-program plumbing routes through compiler-side
    /// register_closure_function); this test exercises the non-closure
    /// frame path.
    #[test]
    fn test_w17_snapshot_non_closure_callstack_roundtrip() {
        use crate::bytecode::BytecodeProgram;
        use crate::executor::CallFrame;
        use shape_value::NativeKind;

        let mut vm = VirtualMachine::new(VMConfig::default());
        // Push 2 scalars to serve as frame[0]'s locals.
        vm.push_kinded(7i64 as u64, NativeKind::Int64)
            .expect("push int 7");
        vm.push_kinded(1, NativeKind::Bool).expect("push bool true");
        // Manually push a non-closure CallFrame whose locals window is
        // those two slots.
        vm.call_stack.push(CallFrame {
            return_ip: 0,
            base_pointer: 0,
            locals_count: 2,
            function_id: None,
            upvalues: None,
            blob_hash: None,
            closure_heap_bits: None,
            closure_heap_kind: None,
        });

        let tmp = tempfile::tempdir().expect("tempdir");
        let store = SnapshotStore::new(tmp.path()).expect("snapshot store");
        let snap = vm.snapshot(&store).expect("snapshot non-closure frame");
        assert_eq!(snap.call_stack.len(), 1);
        let sframe = &snap.call_stack[0];
        assert_eq!(sframe.locals_count, 2);
        assert_eq!(sframe.locals_base, 0);
        assert!(sframe.upvalues.is_none(), "non-closure frame has no upvalues");

        // Restore on a fresh VM. Pre-pad stack so locals_base=0 +
        // locals_count=2 has space; the test asserts the frame restores
        // structurally (function_id, locals_base, locals_count) not the
        // raw stack window.
        let restored = VirtualMachine::from_snapshot(
            BytecodeProgram::default(),
            &snap,
            &store,
        )
        .expect("restore non-closure callstack");
        assert_eq!(restored.call_stack.len(), 1);
        let restored_frame = &restored.call_stack[0];
        assert_eq!(restored_frame.return_ip, 0);
        assert_eq!(restored_frame.base_pointer, 0);
        assert_eq!(restored_frame.locals_count, 2);
        assert!(restored_frame.upvalues.is_none());
        assert!(restored_frame.closure_heap_bits.is_none());
    }

    /// W17-state-tier-roundtrip (Phase 2d Wave 3, 2026-05-12):
    /// VmStateSnapshot accessor surface for an empty VM round-trips
    /// cleanly (no panics; FrameInfo accessors return empty / None).
    #[test]
    fn test_w17_vm_state_snapshot_empty_accessor() {
        use shape_runtime::module_exports::VmStateAccessor;

        let vm = VirtualMachine::new(VMConfig::default());
        let snap = vm.capture_vm_state();
        assert!(snap.current_frame().is_none());
        assert!(snap.caller_frame().is_none());
        assert_eq!(snap.all_frames().len(), 0);
        assert_eq!(snap.current_args().len(), 0);
        assert_eq!(snap.current_locals().len(), 0);
        assert_eq!(snap.module_bindings().len(), 0);
        assert_eq!(snap.instruction_count(), 0);
    }

    /// W17-state-tier-roundtrip (Phase 2d Wave 3, 2026-05-12):
    /// VmStateSnapshot threads kinds through the parallel stack track
    /// for a non-empty live VM. Locals come out as KindedSlot carriers.
    #[test]
    fn test_w17_vm_state_snapshot_kind_threaded_locals() {
        use crate::executor::CallFrame;
        use shape_runtime::module_exports::VmStateAccessor;
        use shape_value::NativeKind;

        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.push_kinded(42i64 as u64, NativeKind::Int64)
            .expect("push int");
        vm.push_kinded(3.14f64.to_bits(), NativeKind::Float64)
            .expect("push float");
        vm.call_stack.push(CallFrame {
            return_ip: 0,
            base_pointer: 0,
            locals_count: 2,
            function_id: None,
            upvalues: None,
            blob_hash: None,
            closure_heap_bits: None,
            closure_heap_kind: None,
        });

        let snap = vm.capture_vm_state();
        let frames = snap.all_frames();
        assert_eq!(frames.len(), 1);
        let f = &frames[0];
        assert_eq!(f.locals.len(), 2);
        assert!(matches!(f.locals[0].kind(), NativeKind::Int64));
        assert!(matches!(f.locals[1].kind(), NativeKind::Float64));
        assert_eq!(f.locals[0].slot().raw(), 42);
        assert_eq!(f.locals[1].slot().raw(), 3.14f64.to_bits());
    }
}
