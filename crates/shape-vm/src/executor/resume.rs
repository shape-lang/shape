//! Resume logic for `state.resume()` and `state.resume_frame()`.
//!
//! ## W17-snapshot-resume rebuild (Phase-2c, R8 W2)
//!
//! T1 (R8 W1, `c125aeb3`) landed the kind-threaded host-tier marshal API
//! (`shape_runtime::snapshot::slot_to_serializable` /
//! `serializable_to_slot`), unblocking the snapshot-resume territory
//! tracked at `docs/cluster-audits/v0.3-phase-2c-audit.md` §2.1 / §3.
//!
//! Disposition by entry point:
//!
//! - **`apply_pending_frame_resume`** — fully implemented. The
//!   `FrameResumeData { ip_offset, locals: Vec<KindedSlot> }` carrier
//!   already threads `NativeKind` per local; the body installs each
//!   `KindedSlot` into the current frame's stack window via
//!   `stack_write_kinded` (transferring share ownership per ADR-006
//!   §2.7.7 retain-on-write) and relocates IP to
//!   `function.entry_point + ip_offset`. Surfaces structured errors when
//!   pre-conditions fail (no active frame, missing function metadata,
//!   ip_offset out of range, locals overflow).
//!
//! - **`apply_pending_resume`** — partial implementation. The pending
//!   payload is a `KindedSlot` carrying a captured `VmState` (per the
//!   `state.resume(vm: VmState)` schema at
//!   `state_builtins/core.rs:286-296`). Full restoration requires the
//!   typed-object field-decode path that's gated on the
//!   W17-marshal-return-arms follow-up (see
//!   `state_builtins/introspection.rs:35-64`). Until that lands the body
//!   projects the payload through `slot_to_serializable` for diagnostic
//!   reporting and surfaces a precise, kind-aware structured error
//!   rather than the previous catch-all. Calling
//!   `VirtualMachine::from_snapshot(BytecodeProgram, &VmSnapshot, &SnapshotStore)`
//!   (already T1-available at `executor/snapshot.rs:235`) is the
//!   downstream landing path once the VmState typed-object decode
//!   wires up.
//!
//! ## Forbidden patterns refused
//!
//! - **No Bool-default fallback** when kind isn't matched — surface-and-
//!   stop per ADR-006 §2.7.14.
//! - **No `Arc<HeapValue>` generic decode** — every slot dispatch goes
//!   through `KindedSlot.kind()` (§2.7.6/Q8 carrier-API bound).
//! - **No `(decode|tag|kind|dispatch) (bridge|probe|helper|hop|...)
//!   shim`** — calling out the actual deleted names per CLAUDE.md
//!   "describe deleted code by name".

use shape_value::VMError;

use super::VirtualMachine;

/// Surface message for the residual W17-snapshot-resume gate on
/// `apply_pending_resume` — the VmState typed-object decode that the
/// W17-marshal-return-arms follow-up has to land before
/// `from_snapshot()` (already T1-available) can consume it.
///
/// Retains the legacy `PHASE_2C_SNAPSHOT_SURFACE` constant name (per the
/// playbook §3 W17-snapshot-resume entry tracking it by name) but the
/// body now cites the precise downstream follow-up rather than the
/// broad "deleted carriers" list — T1 already replaced those.
const PHASE_2C_SNAPSHOT_SURFACE: &str =
    "W17-snapshot-resume residual surface — `apply_pending_resume` \
     requires the VmState typed-object field-decode path that lands \
     with W17-marshal-return-arms. T1 (R8 W1, c125aeb3) made \
     `slot_to_serializable` / `serializable_to_slot` / \
     `VirtualMachine::from_snapshot` available; only the \
     typed-object-field projection into a `VmSnapshot` is gated. \
     Tracked as W17-snapshot-resume per \
     docs/cluster-audits/v0.3-phase-2c-audit.md §2.1 / §3 and \
     docs/cluster-audits/phase-2d-playbook.md §3. ADR-006 §2.7.4 + \
     §2.7.5.1.";

impl VirtualMachine {
    /// Apply a pending full VM state resume from `state.resume()`.
    ///
    /// **W17-snapshot-resume (R8 W2, 2026-05-23)** — partial implementation
    /// per the module doc comment. The body drains the pending payload
    /// (releasing its share via `KindedSlot::Drop` dispatch), then
    /// projects the slot through T1's kind-threaded `slot_to_serializable`
    /// API to produce a diagnostic surface that names the actual kind
    /// observed (rather than a catch-all message).
    ///
    /// Full restoration via `VirtualMachine::from_snapshot` lands once
    /// the VmState typed-object decode wires up — that's the
    /// W17-marshal-return-arms follow-up. Today the typed-object's
    /// fields can be projected per-slot via `slot_to_serializable`, but
    /// the VmState schema-walk + `VmSnapshot` reassembly is not yet
    /// part of this module's territory.
    pub(crate) fn apply_pending_resume(&mut self) -> Result<(), VMError> {
        // Drain the queued payload. The KindedSlot's Drop impl releases
        // the underlying share via `drop_with_kind` per ADR-006 §2.7.7
        // — we don't need an explicit `drop_with_kind` call here.
        let payload = match self.pending_resume.take() {
            Some(p) => p,
            None => {
                // No pending payload. The dispatch shell only invokes
                // `apply_pending_resume` after observing
                // `VMError::ResumeRequested`, which is itself raised
                // only when the introspection body sets the queue.
                // A None-payload path means a caller called
                // `apply_pending_resume` directly without the surface;
                // surface-and-stop per the §2.7.4 invariant rather
                // than a no-op.
                return Err(VMError::NotImplemented(format!(
                    "{}: apply_pending_resume invoked with no pending \
                     payload — caller is bypassing the \
                     VMError::ResumeRequested surface contract.",
                    PHASE_2C_SNAPSHOT_SURFACE,
                )));
            }
        };

        let kind = payload.kind();
        let bits = payload.slot().raw();

        // Project the slot through T1's marshal API to diagnose the
        // arriving payload's wire-format arm. We use an ephemeral
        // tempdir-backed SnapshotStore for projection — same pattern as
        // `state_builtins/core.rs::ephemeral_store` (the chunked-blob
        // arms need it; scalar arms ignore it). On store failure we
        // surface clean per the §2.7.4 invariant.
        let tmp = tempfile::tempdir().map_err(|e| {
            VMError::NotImplemented(format!(
                "{}: tempdir creation failed during diagnostic \
                 projection: {e}",
                PHASE_2C_SNAPSHOT_SURFACE,
            ))
        })?;
        let store = shape_runtime::snapshot::SnapshotStore::new(tmp.path()).map_err(|e| {
            VMError::NotImplemented(format!(
                "{}: SnapshotStore::new failed during diagnostic \
                 projection: {e}",
                PHASE_2C_SNAPSHOT_SURFACE,
            ))
        })?;

        let projection = shape_runtime::snapshot::slot_to_serializable(bits, kind, &store);

        // Surface a kind-aware error so callers (and the downstream
        // W17-marshal-return-arms agent) can observe exactly what
        // shape arrived. The actual restore is deferred to that
        // follow-up — see module doc comment for the wiring.
        Err(VMError::NotImplemented(match projection {
            Ok(sv) => format!(
                "{}: pending payload kind={kind:?} projected to \
                 SerializableVMValue arm {} — VmState typed-object \
                 field-decode + VmSnapshot reassembly + \
                 `VirtualMachine::from_snapshot` invocation is the \
                 W17-marshal-return-arms follow-up.",
                PHASE_2C_SNAPSHOT_SURFACE,
                arm_name_for_diag(&sv),
            ),
            Err(msg) => format!(
                "{}: pending payload kind={kind:?} failed marshal \
                 projection ({msg}) — VmState round-trip blocked at \
                 the inner kind. The typed-object field decode lands \
                 with W17-marshal-return-arms.",
                PHASE_2C_SNAPSHOT_SURFACE,
            ),
        }))
    }

    /// Apply a pending single-frame resume from `state.resume_frame()`.
    ///
    /// **W17-snapshot-resume (R8 W2, 2026-05-23)** — fully implemented.
    /// The `FrameResumeData` carrier already threads `NativeKind` per
    /// local via `Vec<KindedSlot>`; the body:
    ///
    /// 1. Drains the pending data (releasing any prior queue share).
    /// 2. Resolves the current frame from `self.call_stack.last()`.
    /// 3. Resolves the frame's `Function` metadata to recover
    ///    `entry_point` (the absolute IP base for the function body).
    /// 4. Validates the locals count against `locals_count` on the
    ///    frame and the `body_length` against `ip_offset`.
    /// 5. Installs each local into the frame's stack window via
    ///    `stack_write_kinded` — transferring share ownership from the
    ///    `KindedSlot` into the stack slot per ADR-006 §2.7.7 retain-
    ///    on-write. `mem::forget` on the KindedSlot prevents its Drop
    ///    from releasing the share we just installed.
    /// 6. Sets `self.ip = function.entry_point + ip_offset` so the
    ///    dispatch loop resumes inside the function body.
    pub(crate) fn apply_pending_frame_resume(&mut self) -> Result<(), VMError> {
        let data = match self.pending_frame_resume.take() {
            Some(d) => d,
            None => {
                // The dispatch loop checks `pending_frame_resume.is_some()`
                // before calling, so this branch is defensive — a direct
                // caller bypassing the surface contract.
                return Err(VMError::NotImplemented(format!(
                    "{}: apply_pending_frame_resume invoked with no \
                     pending payload.",
                    PHASE_2C_SNAPSHOT_SURFACE,
                )));
            }
        };

        // 1. Locate the current frame. resume_frame is intended to be
        // called immediately after `invoke_callable` sets up the call
        // frame; in that contract the topmost frame on `call_stack` is
        // the one we override.
        let frame = self.call_stack.last().ok_or_else(|| {
            VMError::RuntimeError(format!(
                "apply_pending_frame_resume: no active call frame to \
                 resume — `state.resume_frame()` must be called from a \
                 function set up by `invoke_callable`. ADR-006 §2.7.4."
            ))
        })?;
        let base_pointer = frame.base_pointer;
        let frame_locals_count = frame.locals_count;
        let function_id = frame.function_id;

        // 2. Resolve function metadata.
        let function = function_id
            .and_then(|fid| self.program.functions.get(fid as usize))
            .ok_or_else(|| {
                VMError::RuntimeError(format!(
                    "apply_pending_frame_resume: active frame has no \
                     function_id or function_id is out of range — \
                     resume_frame requires a typed function context."
                ))
            })?;
        let entry_point = function.entry_point;
        let body_length = function.body_length;

        // 3. Validate locals count + ip_offset.
        if data.locals.len() > frame_locals_count {
            return Err(VMError::RuntimeError(format!(
                "apply_pending_frame_resume: persisted locals count \
                 ({}) exceeds frame.locals_count ({}). ADR-006 §2.7.4.",
                data.locals.len(),
                frame_locals_count,
            )));
        }
        if data.ip_offset > body_length {
            return Err(VMError::RuntimeError(format!(
                "apply_pending_frame_resume: ip_offset ({}) exceeds \
                 function body_length ({}) for function '{}'. \
                 ADR-006 §2.7.4.",
                data.ip_offset, body_length, function.name,
            )));
        }
        let abs_locals_end = base_pointer.saturating_add(data.locals.len());
        if abs_locals_end > self.stack.len() {
            return Err(VMError::RuntimeError(format!(
                "apply_pending_frame_resume: locals window \
                 [{base_pointer}..{abs_locals_end}) overflows stack \
                 length ({}). ADR-006 §2.7.7.",
                self.stack.len(),
            )));
        }

        // 4. Install locals. Each KindedSlot carries a strong-count
        // share; `stack_write_kinded` drops the existing slot occupant
        // and transfers the incoming share via raw bits + kind. We
        // disassemble the KindedSlot with `mem::forget` so its Drop
        // doesn't release the share we just installed.
        for (i, slot) in data.locals.into_iter().enumerate() {
            let bits = slot.slot().raw();
            let kind = slot.kind();
            std::mem::forget(slot);
            self.stack_write_kinded(base_pointer + i, bits, kind);
        }
        // The remaining frame-local slots (data.locals.len() .. frame_locals_count)
        // keep their prior occupants — the caller-side contract is that
        // resume_frame replaces a prefix of the locals window (the
        // ones the `state.capture()` body captured).

        // 5. Relocate IP. The bytecode dispatch loop's `self.ip`
        // already includes the post-increment from the last
        // pre-resume instruction; we override it to the function-
        // relative offset added to entry_point.
        self.ip = entry_point.saturating_add(data.ip_offset);

        Ok(())
    }
}

/// Brief discriminator name for `slot_to_serializable` diagnostic
/// messages. Mirrors the private `serializable_arm_name` in
/// `shape-runtime/src/snapshot.rs` but lives here so we don't need to
/// re-export it from the runtime crate just for diagnostic strings.
fn arm_name_for_diag(sv: &shape_runtime::snapshot::SerializableVMValue) -> &'static str {
    use shape_runtime::snapshot::SerializableVMValue as SV;
    match sv {
        SV::Int(_) => "Int",
        SV::Number(_) => "Number",
        SV::Decimal(_) => "Decimal",
        SV::String(_) => "String",
        SV::Bool(_) => "Bool",
        SV::None => "None",
        SV::Unit => "Unit",
        SV::Char(_) => "Char",
        SV::BigInt(_) => "BigInt",
        SV::HashSet { .. } => "HashSet",
        SV::AtomicI64 { .. } => "AtomicI64",
        SV::PriorityQueueHeap { .. } => "PriorityQueueHeap",
        SV::OptionData { .. } => "OptionData",
        SV::ResultData { .. } => "ResultData",
        SV::IteratorOpaque => "IteratorOpaque",
        SV::DequeOpaque { .. } => "DequeOpaque",
        SV::ChannelOpaque { .. } => "ChannelOpaque",
        SV::ReferenceOpaque => "ReferenceOpaque",
        SV::FilterExprOpaque => "FilterExprOpaque",
        SV::SharedCellOpaque => "SharedCellOpaque",
        SV::MutexOpaque { .. } => "MutexOpaque",
        SV::LazyOpaque { .. } => "LazyOpaque",
        SV::TypedObject { .. } => "TypedObject",
        SV::Closure { .. } => "Closure",
        SV::Array(_) => "Array",
        SV::HashMap { .. } => "HashMap",
        SV::TypedArray { .. } => "TypedArray",
        SV::Matrix { .. } => "Matrix",
        SV::DataTable(_) => "DataTable",
        SV::TypedTable { .. } => "TypedTable",
        SV::Future(_) => "Future",
        SV::Function(_) => "Function",
        SV::FunctionRef { .. } => "FunctionRef",
        SV::ModuleFunction(_) => "ModuleFunction",
        SV::Some(_) => "Some",
        SV::Ok(_) => "Ok",
        SV::Err(_) => "Err",
        _ => "Other",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::VMConfig;
    use crate::executor::{CallFrame, VirtualMachine};
    use shape_value::{KindedSlot, NativeKind, ValueSlot};

    /// `apply_pending_resume` with no pending payload returns a
    /// structured error (defensive surface — the dispatch shell never
    /// invokes this without a queued payload, but the surface contract
    /// is preserved per ADR-006 §2.7.4).
    #[test]
    fn apply_pending_resume_no_payload_surfaces_clean() {
        let mut vm = VirtualMachine::new(VMConfig::default());
        let err = vm.apply_pending_resume().expect_err("expected surface");
        let msg = format!("{err:?}");
        assert!(
            msg.contains("W17-snapshot-resume"),
            "expected W17-snapshot-resume surface, got: {msg}"
        );
    }

    /// `apply_pending_resume` with a scalar Int64 payload routes through
    /// T1's `slot_to_serializable` and surfaces a kind-aware diagnostic
    /// naming the projected arm. Confirms the T1 marshal API is wired
    /// into the resume path.
    #[test]
    fn apply_pending_resume_int_payload_projects_diagnostic() {
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.pending_resume = Some(KindedSlot::new(
            ValueSlot::from_raw(42u64),
            NativeKind::Int64,
        ));
        let err = vm.apply_pending_resume().expect_err("expected surface");
        let msg = format!("{err:?}");
        assert!(
            msg.contains("Int") && msg.contains("Int64"),
            "expected diagnostic to name kind=Int64 + arm=Int, got: {msg}"
        );
        // Payload is consumed (drop_with_kind via KindedSlot::Drop).
        assert!(
            vm.pending_resume.is_none(),
            "expected pending_resume drained after apply"
        );
    }

    /// `apply_pending_frame_resume` with no payload surfaces clean.
    #[test]
    fn apply_pending_frame_resume_no_payload_surfaces_clean() {
        let mut vm = VirtualMachine::new(VMConfig::default());
        let err = vm
            .apply_pending_frame_resume()
            .expect_err("expected surface");
        let msg = format!("{err:?}");
        assert!(
            msg.contains("W17-snapshot-resume"),
            "expected W17-snapshot-resume surface, got: {msg}"
        );
    }

    /// `apply_pending_frame_resume` with no active call frame surfaces
    /// as `RuntimeError` per the resume_frame contract.
    #[test]
    fn apply_pending_frame_resume_no_active_frame_surfaces_clean() {
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.pending_frame_resume = Some(crate::executor::FrameResumeData {
            ip_offset: 0,
            locals: vec![],
        });
        let err = vm
            .apply_pending_frame_resume()
            .expect_err("expected surface");
        let msg = format!("{err:?}");
        assert!(
            msg.contains("no active call frame"),
            "expected no-frame surface, got: {msg}"
        );
    }

    /// `apply_pending_frame_resume` happy path with a synthetic frame:
    /// installs scalar locals into the frame's stack window and
    /// relocates IP to function.entry_point + ip_offset.
    ///
    /// This test exercises the kinded share-transfer discipline at the
    /// frame-resume boundary: each KindedSlot's share is installed in
    /// place via `stack_write_kinded`, releasing the prior occupant
    /// (which is Bool/0 here, a no-op release).
    #[test]
    fn apply_pending_frame_resume_installs_locals_and_relocates_ip() {
        use crate::bytecode::{BytecodeProgram, Function};

        let mut vm = VirtualMachine::new(VMConfig::default());
        // Build a single-function program where function 0 has
        // entry_point=100, body_length=50, locals_count=3.
        let program = BytecodeProgram {
            functions: vec![Function {
                name: "test_resume_frame".into(),
                arity: 0,
                param_names: vec![],
                locals_count: 3,
                entry_point: 100,
                body_length: 50,
                is_closure: false,
                captures_count: 0,
                is_async: false,
                ref_params: vec![],
                ref_mutates: vec![],
                mutable_captures: vec![],
                frame_descriptor: None,
                osr_entry_points: vec![],
                mir_data: None,
            }],
            ..Default::default()
        };
        vm.load_program(program);

        // Pre-allocate 3 stack slots (the function's locals window).
        // Use push_kinded so the kind track stays in sync.
        for _ in 0..3 {
            vm.push_kinded(0u64, NativeKind::Bool)
                .expect("pad stack with sentinel");
        }

        // Manually push a CallFrame whose locals window is [0..3) on
        // the stack.
        vm.call_stack.push(CallFrame {
            return_ip: 0,
            base_pointer: 0,
            locals_count: 3,
            function_id: Some(0),
            upvalues: None,
            blob_hash: None,
            closure_heap_bits: None,
            closure_heap_kind: None,
        });

        // Queue a frame resume: ip_offset=10, locals = [Int(42),
        // Float(3.14), Bool(true)].
        vm.pending_frame_resume = Some(crate::executor::FrameResumeData {
            ip_offset: 10,
            locals: vec![
                KindedSlot::new(ValueSlot::from_raw(42u64), NativeKind::Int64),
                KindedSlot::new(
                    ValueSlot::from_raw(3.14f64.to_bits()),
                    NativeKind::Float64,
                ),
                KindedSlot::new(ValueSlot::from_raw(1u64), NativeKind::Bool),
            ],
        });

        // Apply.
        vm.apply_pending_frame_resume()
            .expect("apply_pending_frame_resume should succeed");

        // Verify IP relocated to function.entry_point (100) + ip_offset (10).
        assert_eq!(vm.ip, 110, "IP should relocate to entry_point + ip_offset");

        // Verify locals window was overwritten with the queued values.
        // The kind track tracks per slot per ADR-006 §2.7.7.
        assert_eq!(vm.stack[0], 42u64, "local[0] bits");
        assert_eq!(vm.kinds[0], NativeKind::Int64);
        assert_eq!(vm.stack[1], 3.14f64.to_bits(), "local[1] bits");
        assert_eq!(vm.kinds[1], NativeKind::Float64);
        assert_eq!(vm.stack[2], 1u64, "local[2] bits");
        assert_eq!(vm.kinds[2], NativeKind::Bool);

        // Payload was drained.
        assert!(vm.pending_frame_resume.is_none());
    }

    /// `apply_pending_frame_resume` rejects an ip_offset past the
    /// function body's length.
    #[test]
    fn apply_pending_frame_resume_ip_offset_oob_surfaces_clean() {
        use crate::bytecode::{BytecodeProgram, Function};

        let mut vm = VirtualMachine::new(VMConfig::default());
        let program = BytecodeProgram {
            functions: vec![Function {
                name: "test".into(),
                arity: 0,
                param_names: vec![],
                locals_count: 0,
                entry_point: 100,
                body_length: 50,
                is_closure: false,
                captures_count: 0,
                is_async: false,
                ref_params: vec![],
                ref_mutates: vec![],
                mutable_captures: vec![],
                frame_descriptor: None,
                osr_entry_points: vec![],
                mir_data: None,
            }],
            ..Default::default()
        };
        vm.load_program(program);
        vm.call_stack.push(CallFrame {
            return_ip: 0,
            base_pointer: 0,
            locals_count: 0,
            function_id: Some(0),
            upvalues: None,
            blob_hash: None,
            closure_heap_bits: None,
            closure_heap_kind: None,
        });
        vm.pending_frame_resume = Some(crate::executor::FrameResumeData {
            ip_offset: 999, // way past body_length=50
            locals: vec![],
        });
        let err = vm
            .apply_pending_frame_resume()
            .expect_err("expected ip_offset OOB surface");
        let msg = format!("{err:?}");
        assert!(
            msg.contains("ip_offset")
                && (msg.contains("exceeds") || msg.contains("body_length")),
            "expected ip_offset/body_length surface, got: {msg}"
        );
    }

    /// `apply_pending_frame_resume` rejects a locals vec longer than
    /// the frame's `locals_count`.
    #[test]
    fn apply_pending_frame_resume_too_many_locals_surfaces_clean() {
        use crate::bytecode::{BytecodeProgram, Function};

        let mut vm = VirtualMachine::new(VMConfig::default());
        let program = BytecodeProgram {
            functions: vec![Function {
                name: "test".into(),
                arity: 0,
                param_names: vec![],
                locals_count: 1, // frame holds 1 local
                entry_point: 100,
                body_length: 50,
                is_closure: false,
                captures_count: 0,
                is_async: false,
                ref_params: vec![],
                ref_mutates: vec![],
                mutable_captures: vec![],
                frame_descriptor: None,
                osr_entry_points: vec![],
                mir_data: None,
            }],
            ..Default::default()
        };
        vm.load_program(program);
        vm.push_kinded(0u64, NativeKind::Bool).expect("pad");
        vm.call_stack.push(CallFrame {
            return_ip: 0,
            base_pointer: 0,
            locals_count: 1,
            function_id: Some(0),
            upvalues: None,
            blob_hash: None,
            closure_heap_bits: None,
            closure_heap_kind: None,
        });
        vm.pending_frame_resume = Some(crate::executor::FrameResumeData {
            ip_offset: 0,
            locals: vec![
                KindedSlot::new(ValueSlot::from_raw(1u64), NativeKind::Int64),
                KindedSlot::new(ValueSlot::from_raw(2u64), NativeKind::Int64),
            ],
        });
        let err = vm
            .apply_pending_frame_resume()
            .expect_err("expected too-many-locals surface");
        let msg = format!("{err:?}");
        assert!(
            msg.contains("locals count") && msg.contains("exceeds"),
            "expected locals-count surface, got: {msg}"
        );
    }
}
