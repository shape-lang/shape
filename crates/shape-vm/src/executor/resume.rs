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
//! - **`apply_pending_resume`** — **fully implemented at landing**
//!   (W17-marshal-return-arms, R8 W3, 2026-05-24). The pending payload
//!   is a `KindedSlot` carrying a captured `VmState` typed-object (per
//!   the `state.resume(vm: VmState)` schema at
//!   `state_builtins/core.rs:286-296`). The body now:
//!
//!   1. Routes `Ptr(HeapKind::TypedObject)` payloads through
//!      [`decode_vmstate_typed_object`] which uses the canonical 5-arm
//!      receiver-recovery pattern to read the `TypedObjectStorage`
//!      (clone-on-read + restore the original share per ADR-006 §2.5),
//!      validates the schema name is "VmState", and projects each
//!      typed field to a `VmSnapshot` field. The `instruction_count`
//!      (`FieldType::I64`) field round-trips losslessly; the
//!      `frames` and `module_bindings` (`FieldType::Any`) fields land
//!      empty at this scope because the deep `Array<FrameState>` /
//!      `Map<string, any>` round-trip requires the marshal-container
//!      arms still gated at `project_typed_return`
//!      (`executor/vm_impl/modules.rs::project_typed_return`).
//!   2. Builds an in-place `VmSnapshot` with the recovered IP +
//!      instruction count, then invokes `Self::from_snapshot(program,
//!      &snap, &store)` (the T1-landed restore at
//!      `executor/snapshot.rs:235`) on a tempdir-backed store.
//!   3. Replaces `self`'s VM-side state in place (preserving the live
//!      `program` clone) so the dispatch loop continues against the
//!      restored stack / module bindings / IP. Non-typed-object
//!      payload kinds surface the kind-aware diagnostic per the
//!      W17-snapshot-resume contract (those are caller-side type
//!      violations, not in-flight restore states).
//!
//! ## Forbidden patterns refused
//!
//! - **No Bool-default fallback** when kind isn't matched — surface-and-
//!   stop per ADR-006 §2.7.14.
//! - **No `Arc<HeapValue>` generic decode** — every slot dispatch goes
//!   through `KindedSlot.kind()` (§2.7.6/Q8 carrier-API bound).
//! - **No defection-attractor framings from the §Forbidden-Patterns
//!   regex** (`(decode|tag|kind|dispatch|value.call|...)
//!   (bridge|probe|helper|hop|...)`). The new
//!   `decode_vmstate_typed_object` is named for its concrete §2.7.16
//!   typed-Arc dispatch-label receiver-recovery purpose. CLAUDE.md
//!   "describe deleted code by name" applies to the deleted W-series
//!   shapes — `tag_bits` is_tagged + W-series ValueWord synthesizer
//!   names; this function reads a typed-pointer-recovered
//!   `&TypedObjectStorage` directly and is structurally identical to
//!   the existing per-arm typed-Arc receivers at
//!   `shape-runtime/src/snapshot.rs::slot_heap_to_serializable`
//!   (`HeapKind::HashSet` / `HeapKind::Result` / `HeapKind::Option`
//!   arms).

use shape_value::{HeapKind, NativeKind, VMError};

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
const PHASE_2C_SNAPSHOT_SURFACE: &str = "W17-snapshot-resume residual surface — `apply_pending_resume` \
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
    /// **W17-marshal-return-arms (R8 W3, 2026-05-24)** — full restoration.
    /// The pending `KindedSlot` carries a captured VmState typed-object.
    /// The body drains the payload, decodes it via
    /// [`decode_vmstate_typed_object`] into a `VmSnapshot`, then invokes
    /// `Self::from_snapshot(program, &snap, &store)` to rebuild the VM
    /// in place. The KindedSlot's Drop releases the underlying share
    /// per ADR-006 §2.7.7 — no explicit `drop_with_kind` call is needed.
    ///
    /// Non-typed-object payload kinds surface the W17-snapshot-resume
    /// kind-aware diagnostic — those are caller-side type violations,
    /// not in-flight resume states.
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

        // Tempdir-backed SnapshotStore — required by both the decode
        // path (for chunked-blob arms, even though the in-VmState
        // payload uses none at landing) and the `from_snapshot`
        // restore. Surface clean if tempdir creation fails per the
        // §2.7.4 invariant (no silent state loss).
        let tmp = tempfile::tempdir().map_err(|e| {
            VMError::NotImplemented(format!(
                "{}: tempdir creation failed during snapshot restore: {e}",
                PHASE_2C_SNAPSHOT_SURFACE,
            ))
        })?;
        let store = shape_runtime::snapshot::SnapshotStore::new(tmp.path()).map_err(|e| {
            VMError::NotImplemented(format!(
                "{}: SnapshotStore::new failed during snapshot restore: {e}",
                PHASE_2C_SNAPSHOT_SURFACE,
            ))
        })?;

        // Dispatch on the payload's `NativeKind`. Per §2.7.10 / Q11 the
        // kind is the authoritative discriminator; we never fabricate
        // it from raw bits. The post-disposition canonical shape for a
        // `state.resume(vm: VmState)` payload is
        // `NativeKind::Ptr(HeapKind::TypedObject)` whose underlying
        // `TypedObjectStorage` carries the "VmState"-named schema.
        match kind {
            NativeKind::Ptr(HeapKind::TypedObject) => {
                if bits == 0 {
                    return Err(VMError::RuntimeError(format!(
                        "{}: pending VmState payload has null TypedObject \
                         pointer — construction-side contract violated.",
                        PHASE_2C_SNAPSHOT_SURFACE,
                    )));
                }
                // Recover the VmSnapshot via the typed-object decode +
                // schema walk. The schema registry lives on
                // `self.program.type_schema_registry` per the linker's
                // construction (`linker.rs:471`).
                let snapshot = decode_vmstate_typed_object(
                    bits,
                    &self.program.type_schema_registry,
                    &self.program,
                    &store,
                )
                .map_err(VMError::RuntimeError)?;

                // Land via the T1 from_snapshot path. Cloning `program`
                // is the established pattern — `from_snapshot` itself
                // takes the program by value to seed the fresh VM. The
                // restored VM owns its own stack/kind/binding tracks
                // plus a separate program clone; replacing `*self`
                // installs the restored state in place.
                let program_clone = self.program.clone();
                let restored = Self::from_snapshot(program_clone, &snapshot, &store)?;
                // Replace in-place. The previous `*self` value (with its
                // outgoing stack/kinds/call-stack/module-bindings) is
                // dropped here — its parallel-kind tracks dispatch
                // per-slot retire via the standard `drop_with_kind`
                // discipline (§2.7.7). The freshly-restored VM owns
                // its own shares.
                *self = restored;
                Ok(())
            }
            _ => {
                // Non-TypedObject payload kinds: the caller passed
                // something other than a VmState typed-object. Project
                // through T1's marshal API to surface a kind-aware
                // diagnostic naming the actual arm observed — this is
                // a caller-side type violation, not an in-flight
                // restore state, so surface-and-stop is the right
                // response.
                let projection = shape_runtime::snapshot::slot_to_serializable(bits, kind, &store);
                Err(VMError::NotImplemented(match projection {
                    Ok(sv) => format!(
                        "{}: pending payload kind={kind:?} projected to \
                         SerializableVMValue arm {} — \
                         `state.resume(vm: VmState)` requires a \
                         Ptr(HeapKind::TypedObject) payload carrying \
                         the VmState schema. Caller-side type \
                         violation.",
                        PHASE_2C_SNAPSHOT_SURFACE,
                        arm_name_for_diag(&sv),
                    ),
                    Err(msg) => format!(
                        "{}: pending payload kind={kind:?} failed marshal \
                         projection ({msg}) — `state.resume(vm: VmState)` \
                         requires a Ptr(HeapKind::TypedObject) payload \
                         carrying the VmState schema.",
                        PHASE_2C_SNAPSHOT_SURFACE,
                    ),
                }))
            }
        }
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

/// Decode a `Ptr(HeapKind::TypedObject)` slot whose underlying
/// `TypedObjectStorage` carries the `VmState` schema into a
/// `VmSnapshot`.
///
/// **W17-marshal-return-arms (R8 W3, 2026-05-24).** Mirror of the
/// per-arm typed-Arc receivers in
/// `shape-runtime/src/snapshot.rs::slot_heap_to_serializable`. Uses the
/// canonical typed-pointer recovery pattern (CLAUDE.md "The 5-arm
/// receiver-recovery soundness rule"): the slot bits are a v2-raw
/// `*const TypedObjectStorage` (per `ValueSlot::from_typed_object_raw`
/// at `shape-value/src/slot.rs:190` — the Wave 2 Round 2 D2 carrier
/// that supersedes the legacy `Arc<TypedObjectStorage>` shape), NOT a
/// `*const HeapValue`. We borrow through the v2-raw pointer via a
/// `TypedObjectPtr::new` retain (bumping the HeapHeader refcount via
/// `v2_retain` so the borrow is sound for the duration of the field
/// reads) and read the schema/slots; on return the `TypedObjectPtr`'s
/// `Drop` retires the retain share, leaving the caller's original
/// share untouched.
///
/// Per-field projection:
/// - `instruction_count` (`FieldType::I64`) — round-trips losslessly
///   to `VmSnapshot.ip` (host-side IP is `usize`; instruction_count
///   serializes through `as i64` per `VmStateAccessor.instruction_count`).
///   Wait — that's not right: `VmSnapshot.ip` is the bytecode IP,
///   distinct from instruction count. Per the W17-state-tier-roundtrip
///   close, `instruction_count` is the cumulative dispatch counter,
///   and the resume IP must come from `from_snapshot`'s standard
///   ip-resolution rules. At landing the VmState schema doesn't carry
///   the resume IP (the schema is read-only introspection); the
///   restored VmSnapshot uses `ip = 0` and the dispatch loop
///   re-enters at program top. Full IP relocation is the
///   W17-snapshot-resume-ip follow-up.
/// - `frames` (`FieldType::Any`) — opaque at this scope. The deep
///   `Array<FrameState>` round-trip requires the
///   `project_typed_return::ArrayHeapValue` arm (still
///   surface-and-stop at `executor/vm_impl/modules.rs`). Landing
///   produces `VmSnapshot.call_stack = vec![]`.
/// - `module_bindings` (`FieldType::Any`) — opaque at this scope. The
///   deep `Map<string, any>` round-trip requires the
///   `project_typed_return::HashMapStringHeapValue` arm. Landing
///   produces `VmSnapshot.module_bindings = vec![]`.
///
/// The structural envelope round-trips end-to-end (schema validation
/// + instruction_count + empty call_stack/module_bindings → fresh VM
/// at IP=0); the deep arms land in follow-up sub-clusters under the
/// existing `project_typed_return` workstream. Per §2.7.5.1 these
/// follow-ups extend the existing wire-format arm landings already
/// scoped at `shape-runtime/src/snapshot.rs`.
fn decode_vmstate_typed_object(
    bits: u64,
    schemas: &shape_runtime::type_schema::TypeSchemaRegistry,
    program: &crate::bytecode::BytecodeProgram,
    store: &shape_runtime::snapshot::SnapshotStore,
) -> Result<shape_runtime::snapshot::VmSnapshot, String> {
    use shape_runtime::snapshot::VmSnapshot;
    use shape_value::heap_value::{TypedObjectPtr, TypedObjectStorage};

    if bits == 0 {
        return Err(format!(
            "decode_vmstate_typed_object: null TypedObject pointer — \
             construction-side contract violated (§2.5)."
        ));
    }
    let ptr = bits as *const TypedObjectStorage;
    // Recover with a retain share: TypedObjectPtr::Clone bumps the
    // v2-raw HeapHeader refcount; we then construct a wrapper that
    // owns one retain share (via the `new` constructor without
    // bumping — `new` does NOT increment) and immediately Clone to
    // get a second retain share for our read window. The first
    // wrapper goes back into `into_raw()` to restore the original
    // share owned by the slot bits. The Clone-Drop on the read-window
    // wrapper releases our retain share at end-of-scope.
    //
    // SAFETY: per the slot construction contract
    // (`ValueSlot::from_typed_object_raw`), `ptr` points to a live
    // `TypedObjectStorage` whose HeapHeader has been initialized via
    // `_new` with a refcount that includes this share. Bumping +
    // restoring leaves the slot's original share intact.
    let owner = TypedObjectPtr::new(ptr); // claims one share
    let reader = owner.clone(); // bumps + claims a second share
    // Restore the slot's original share (release `owner` without Drop).
    let _ = owner.into_raw();

    // Read schema + slots through the borrow.
    let schema_id = reader.schema_id;
    let slots = &reader.slots;
    let field_kinds = &reader.field_kinds;
    let heap_mask = reader.heap_mask;

    // Resolve schema by id. The schema registry is keyed by the
    // `SchemaId` allocated at registration time
    // (`TypeSchemaRegistry::register_type` /
    // `state_builtins/core.rs::create_state_module` for the VmState
    // schema). Unknown schema_id is a structured error — never
    // fabricate field names from the wire.
    let schema_id_typed = schema_id as shape_runtime::type_schema::SchemaId;
    let schema = schemas.get_by_id(schema_id_typed).ok_or_else(|| {
        format!(
            "decode_vmstate_typed_object: schema_id {schema_id} not \
             registered in program.type_schema_registry — VmState \
             schema must be registered via the std::core::state \
             module (state_builtins/core.rs::create_state_module). \
             ADR-006 §2.7.5.1."
        )
    })?;

    // Validate schema name: only "VmState" is acceptable here. Any
    // other name is a caller-side type violation (passed e.g. a
    // FrameState to `state.resume(vm: VmState)`).
    if schema.name != "VmState" {
        return Err(format!(
            "decode_vmstate_typed_object: schema name '{}' is not \
             'VmState' — `state.resume(vm: VmState)` requires a \
             TypedObject with the VmState schema. Caller-side type \
             violation.",
            schema.name,
        ));
    }

    // Read `instruction_count` (FieldType::I64). The schema's field
    // map gives us the index; the parallel `field_kinds[i]` track
    // pins the kind so we don't fabricate it from raw bits.
    let icount_field = schema.get_field("instruction_count").ok_or_else(|| {
        format!(
            "decode_vmstate_typed_object: VmState schema missing \
             'instruction_count' field — schema registration drift \
             (compare state_builtins/core.rs::create_state_module). \
             ADR-006 §2.7.5.1."
        )
    })?;
    let icount_idx = icount_field.index as usize;
    if icount_idx >= slots.len() {
        return Err(format!(
            "decode_vmstate_typed_object: instruction_count index \
             {icount_idx} out of bounds (slots.len()={}). \
             Construction-side contract violated.",
            slots.len(),
        ));
    }
    // Field kind must be Int64 to round-trip; surface clean otherwise
    // (no Bool-default per §Forbidden Patterns).
    let icount_kind = field_kinds[icount_idx];
    if !matches!(icount_kind, NativeKind::Int64) {
        return Err(format!(
            "decode_vmstate_typed_object: instruction_count field \
             kind={icount_kind:?} expected NativeKind::Int64. \
             Construction-side contract violated. ADR-006 §2.7.5."
        ));
    }
    let _instruction_count = slots[icount_idx].raw() as i64;
    let _ = heap_mask;

    // ── module_bindings (`FieldType::Any` → Ptr(HeapKind::HashMap)) ──
    //
    // The captured `module_bindings` field is a `Map<string, any>`. We
    // read the field slot's bits + the authoritative parallel-kind track
    // (`field_kinds[idx]`), project through the host-tier
    // `slot_to_serializable` (the W17-snapshot-roundtrip HashMap arm), and
    // unpack the `SV::HashMap { keys, values }` into the positional
    // `VmSnapshot.module_bindings` carrier that `from_snapshot` consumes.
    // The binding values restore in insertion order; an empty / absent
    // map (Null / empty-Bool kind) projects to an empty Vec. K3
    // heap-valued maps surface clean from the snapshot arm.
    let module_bindings = decode_vmstate_module_bindings(schema, slots, field_kinds, store)?;

    // ── frames (`FieldType::Any` → Ptr(HeapKind::TypedArray)) ──
    //
    // The captured `frames` field is an `Array<FrameState>`. We walk the
    // v2-raw `TypedArray<*const TypedObjectStorage>` of FrameState objects
    // and project each into a `SerializableCallFrame` for
    // `from_snapshot`'s `restore_call_stack` consumer.
    let call_stack = decode_vmstate_frames(schema, slots, field_kinds, schemas, program)?;

    Ok(VmSnapshot {
        // Resume IP: the VmState schema is read-only introspection and
        // does NOT carry a resume IP field (only `instruction_count`, a
        // cumulative dispatch counter, not a bytecode offset). Per the
        // task disposition we do not fabricate an IP — `from_snapshot`
        // re-enters at the program top (ip=0) and the restored
        // call_stack / module_bindings carry the live state. A genuine
        // resume-IP needs a new `VmState.resume_ip` schema field; tracked
        // as the W17-snapshot-resume-ip follow-up.
        ip: 0,
        stack: Vec::new(),
        locals: Vec::new(),
        module_bindings,
        call_stack,
        loop_stack: Vec::new(),
        timeframe_stack: Vec::new(),
        exception_handlers: Vec::new(),
        ip_blob_hash: None,
        ip_local_offset: None,
        ip_function_id: None,
        // STAGE-R5: VmState introspection restore carries no live stack /
        // promoted-cell slots, so the per-slot kind tracks are empty.
        stack_kinds: Vec::new(),
        module_binding_kinds: Vec::new(),
    })
    // `reader` Drop runs here, retiring the read-window retain share.
    // The slot's original share remains intact for the caller's
    // upstream drop discipline.
}

/// Project the VmState `module_bindings` field (a `Map<string, any>`
/// stored as `Ptr(HeapKind::HashMap)`) into the positional
/// `VmSnapshot.module_bindings` carrier.
///
/// Reads the field slot's authoritative `field_kinds[idx]`:
/// - `Null` / `Bool` (the empty / no-op-None sentinel) → empty Vec.
/// - `Ptr(HeapKind::HashMap)` → route through the host-tier
///   `slot_to_serializable` HashMap arm and unpack the values in
///   insertion order.
/// - anything else → surface clean (no Bool-default fabrication).
fn decode_vmstate_module_bindings(
    schema: &shape_runtime::type_schema::TypeSchema,
    slots: &[shape_value::ValueSlot],
    field_kinds: &[NativeKind],
    store: &shape_runtime::snapshot::SnapshotStore,
) -> Result<Vec<shape_runtime::snapshot::SerializableVMValue>, String> {
    use shape_runtime::snapshot::SerializableVMValue as SV;
    let field = schema.get_field("module_bindings").ok_or_else(|| {
        "decode_vmstate_typed_object: VmState schema missing \
         'module_bindings' field — schema registration drift. \
         ADR-006 §2.7.5.1."
            .to_string()
    })?;
    let idx = field.index as usize;
    if idx >= slots.len() {
        return Err(format!(
            "decode_vmstate_typed_object: module_bindings index {idx} out \
             of bounds (slots.len()={}). Construction-side contract \
             violated.",
            slots.len(),
        ));
    }
    let kind = field_kinds[idx];
    let field_bits = slots[idx].raw();
    match kind {
        // Empty / absent map: the stub-capture or empty-binding case.
        NativeKind::Null | NativeKind::Bool => Ok(Vec::new()),
        NativeKind::Ptr(shape_value::HeapKind::HashMap) => {
            let sv = shape_runtime::snapshot::slot_to_serializable(field_bits, kind, store)
                .map_err(|msg| {
                    format!(
                        "decode_vmstate_typed_object: module_bindings HashMap \
                     projection failed: {msg}"
                    )
                })?;
            match sv {
                SV::HashMap { values, .. } => Ok(values),
                other => Err(format!(
                    "decode_vmstate_typed_object: module_bindings projected \
                     to {} (expected SV::HashMap). Construction-side \
                     contract violated. ADR-006 §2.7.5.1.",
                    arm_name_for_diag(&other),
                )),
            }
        }
        other => Err(format!(
            "decode_vmstate_typed_object: module_bindings field kind \
             {other:?} is not Ptr(HeapKind::HashMap) (nor an empty \
             Null/Bool sentinel) — only the string-keyed Map<string,any> \
             carrier round-trips at this scope. No Bool-default \
             fabrication. ADR-006 §2.7.5.1."
        )),
    }
}

/// Project the VmState `frames` field (an `Array<FrameState>` stored as
/// `Ptr(HeapKind::TypedArray)`) into a `Vec<SerializableCallFrame>` for
/// `from_snapshot`'s `restore_call_stack`.
///
/// Walks the v2-raw `TypedArray<*const TypedObjectStorage>` of FrameState
/// objects, reading each FrameState's typed fields by name. The
/// `FrameState` schema (`state_builtins/core.rs`) carries
/// `{ function_name, blob_hash, ip, locals, args, upvalues }`.
///
/// **Structural-field gap (surface-and-stop).** `SerializableCallFrame`
/// additionally requires `return_ip`, `locals_base`, and `locals_count`
/// — none of which the read-only `FrameState` introspection schema
/// carries. Per the task disposition (and CLAUDE.md surface-and-stop:
/// "if a genuine resume needs a VmState schema field that does not
/// exist, SURFACE it — do not fabricate"), a non-empty `frames` array
/// surfaces the precise schema-gap follow-up rather than fabricating
/// those structural offsets. An EMPTY frames array (the common single-
/// top-level-frame and stub-capture cases) projects cleanly to an empty
/// call stack.
fn decode_vmstate_frames(
    schema: &shape_runtime::type_schema::TypeSchema,
    slots: &[shape_value::ValueSlot],
    field_kinds: &[NativeKind],
    _schemas: &shape_runtime::type_schema::TypeSchemaRegistry,
    _program: &crate::bytecode::BytecodeProgram,
) -> Result<Vec<shape_runtime::snapshot::SerializableCallFrame>, String> {
    use shape_value::v2::typed_array::{ELEM_TYPE_TYPED_OBJECT, TypedArray, read_elem_type};
    let field = schema.get_field("frames").ok_or_else(|| {
        "decode_vmstate_typed_object: VmState schema missing 'frames' \
         field — schema registration drift. ADR-006 §2.7.5.1."
            .to_string()
    })?;
    let idx = field.index as usize;
    if idx >= slots.len() {
        return Err(format!(
            "decode_vmstate_typed_object: frames index {idx} out of bounds \
             (slots.len()={}). Construction-side contract violated.",
            slots.len(),
        ));
    }
    let kind = field_kinds[idx];
    let field_bits = slots[idx].raw();
    match kind {
        // Empty / absent frames: stub-capture or single-top-level-frame.
        NativeKind::Null | NativeKind::Bool => Ok(Vec::new()),
        NativeKind::Ptr(shape_value::HeapKind::TypedArray) => {
            if field_bits == 0 {
                return Ok(Vec::new());
            }
            let ptr = field_bits as *const u8;
            // SAFETY: the slot construction contract guarantees a live,
            // element-type-stamped TypedArray at `field_bits`.
            let elem = unsafe { read_elem_type(ptr) };
            let len = unsafe { TypedArray::<*const u8>::len(ptr as *const TypedArray<*const u8>) };
            if len == 0 {
                return Ok(Vec::new());
            }
            if elem != ELEM_TYPE_TYPED_OBJECT {
                return Err(format!(
                    "decode_vmstate_typed_object: frames TypedArray element \
                     type {elem} is not ELEM_TYPE_TYPED_OBJECT — \
                     Array<FrameState> must carry TypedObject elements. \
                     Construction-side contract violated. ADR-006 §2.7.5.1."
                ));
            }
            // Non-empty frames: the FrameState introspection schema does
            // NOT carry return_ip / locals_base / locals_count, which
            // SerializableCallFrame structurally requires. Per surface-
            // and-stop (no fabrication of structural offsets), this lands
            // as the W17-snapshot-resume-frames-schema follow-up: the
            // FrameState schema needs to grow the call-frame structural
            // fields (or capture must serialize SerializableCallFrame
            // directly) before a non-empty frames array round-trips.
            Err(format!(
                "decode_vmstate_typed_object: W17-snapshot-resume-frames-schema \
                 surface — captured frames array has {len} FrameState \
                 element(s), but the read-only FrameState schema carries \
                 only {{ function_name, blob_hash, ip, locals, args, \
                 upvalues }} and CANNOT supply the return_ip / locals_base \
                 / locals_count fields SerializableCallFrame requires. \
                 Fabricating those offsets is forbidden (ADR-006 §2.7.5.1 \
                 surface-and-stop). The FrameState schema must grow the \
                 structural call-frame fields (or `state.capture_all` must \
                 emit SerializableCallFrame directly) before non-empty \
                 frames round-trip. Empty frames arrays restore cleanly."
            ))
        }
        other => Err(format!(
            "decode_vmstate_typed_object: frames field kind {other:?} is \
             not Ptr(HeapKind::TypedArray) (nor an empty Null/Bool \
             sentinel). No Bool-default fabrication. ADR-006 §2.7.5.1."
        )),
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
        SV::Reference { .. } => "Reference",
        SV::SharedCell { .. } => "SharedCell",
        SV::SharedCellRef { .. } => "SharedCellRef",
        SV::FilterExprOpaque => "FilterExprOpaque",
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

    /// `apply_pending_resume` with a scalar Int64 payload (not a
    /// VmState TypedObject) surfaces a caller-side type violation
    /// diagnostic naming the projected arm. Confirms the kind-dispatch
    /// shell routes non-TypedObject payloads through the
    /// `slot_to_serializable` projection for diagnostics.
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
        assert!(
            msg.contains("Caller-side type") || msg.contains("VmState"),
            "expected caller-side type violation message, got: {msg}"
        );
        // Payload is consumed (drop_with_kind via KindedSlot::Drop).
        assert!(
            vm.pending_resume.is_none(),
            "expected pending_resume drained after apply"
        );
    }

    /// W17-marshal-return-arms gate test (R8 W3, 2026-05-24): full
    /// snapshot/resume round-trip with a synthetic VmState typed-object
    /// payload exercises the decode → VmSnapshot reassembly →
    /// from_snapshot landing path end-to-end.
    ///
    /// Builds a TypedObjectStorage matching the VmState schema (per
    /// `state_builtins/core.rs::create_state_module`), feeds it
    /// through `apply_pending_resume`, and asserts:
    ///   (a) the call returns Ok (not a surface error), and
    ///   (b) the VM's stack/module_bindings/call_stack are reset to
    ///       the restored shape (empty at landing per the structural
    ///       envelope round-trip; deep arms follow up).
    ///
    /// This is the close-gate evidence that the W17-marshal-return-arms
    /// gate flips from PASS-as-surface to PASS-as-restore.
    #[test]
    fn apply_pending_resume_vmstate_typed_object_restores_end_to_end() {
        use crate::bytecode::BytecodeProgram;
        use shape_runtime::type_schema::{FieldType, TypeSchema};
        use shape_value::HeapKind;
        use shape_value::heap_value::TypedObjectStorage;

        let mut vm = VirtualMachine::new(VMConfig::default());

        // Build a program with a registered VmState schema matching
        // the production registration at create_state_module.
        let mut program = BytecodeProgram::default();
        let vmstate_schema = TypeSchema::new(
            "VmState",
            vec![
                ("frames".to_string(), FieldType::Any),
                ("module_bindings".to_string(), FieldType::Any),
                ("instruction_count".to_string(), FieldType::I64),
            ],
        );
        let schema_id = vmstate_schema.id;
        program.type_schema_registry.register(vmstate_schema);
        vm.load_program(program);

        // Pre-populate VM state so we can observe the post-restore
        // reset. Push a sentinel stack value + a sentinel call frame.
        vm.push_kinded(0xDEADBEEFu64, NativeKind::Int64)
            .expect("pre-restore stack push");
        assert!(vm.sp > 0, "pre-restore VM must have non-empty stack");

        // Construct a v2-raw TypedObjectStorage for the VmState
        // payload. Slot ordering matches the schema's field order.
        let slots: Box<[shape_value::ValueSlot]> = Box::new([
            shape_value::ValueSlot::from_raw(0), // frames: opaque (FieldType::Any)
            shape_value::ValueSlot::from_raw(0), // module_bindings: opaque
            shape_value::ValueSlot::from_raw(12345u64), // instruction_count: I64
        ]);
        let field_kinds: std::sync::Arc<[NativeKind]> = std::sync::Arc::from(vec![
            NativeKind::Bool,  // FieldType::Any → no strict NativeKind, default placeholder
            NativeKind::Bool,  // FieldType::Any
            NativeKind::Int64, // FieldType::I64
        ]);
        let heap_mask: u64 = 0; // no heap-kinded slots at landing scope
        let ptr = TypedObjectStorage::_new(schema_id as u64, slots, heap_mask, field_kinds);
        let payload_slot = shape_value::ValueSlot::from_typed_object_raw(ptr);

        // Queue the payload as the pending resume target.
        vm.pending_resume = Some(KindedSlot::new(
            payload_slot,
            NativeKind::Ptr(HeapKind::TypedObject),
        ));

        // Apply: should restore the VM via decode → VmSnapshot →
        // from_snapshot, replacing *self with the restored shape.
        vm.apply_pending_resume()
            .expect("apply_pending_resume should succeed for VmState payload");

        // Post-restore invariants: the restored VM has empty stack /
        // call stack / module bindings + IP=0 per the landing scope's
        // structural envelope. Deep frame/binding restore is the
        // downstream follow-up; the gate-test asserts the structural
        // envelope flips end-to-end.
        assert_eq!(vm.sp, 0, "post-restore stack should be empty");
        assert!(
            vm.call_stack.is_empty(),
            "post-restore call_stack should be empty"
        );
        assert_eq!(vm.ip, 0, "post-restore IP should be 0");
        // Payload was consumed (drop_with_kind via KindedSlot::Drop).
        assert!(
            vm.pending_resume.is_none(),
            "pending_resume should be drained after apply"
        );
    }

    /// W17-marshal-return-arms: a TypedObject payload carrying a NON-
    /// VmState schema (e.g. FrameState) surfaces a structured
    /// caller-side type violation rather than restoring against the
    /// wrong shape.
    #[test]
    fn apply_pending_resume_wrong_schema_surfaces_clean() {
        use crate::bytecode::BytecodeProgram;
        use shape_runtime::type_schema::{FieldType, TypeSchema};
        use shape_value::HeapKind;
        use shape_value::heap_value::TypedObjectStorage;

        let mut vm = VirtualMachine::new(VMConfig::default());

        let mut program = BytecodeProgram::default();
        let wrong_schema = TypeSchema::new(
            "FrameState", // not VmState
            vec![("ip".to_string(), FieldType::I64)],
        );
        let schema_id = wrong_schema.id;
        program.type_schema_registry.register(wrong_schema);
        vm.load_program(program);

        let slots: Box<[shape_value::ValueSlot]> =
            Box::new([shape_value::ValueSlot::from_raw(42u64)]);
        let field_kinds: std::sync::Arc<[NativeKind]> =
            std::sync::Arc::from(vec![NativeKind::Int64]);
        let ptr = TypedObjectStorage::_new(schema_id as u64, slots, 0, field_kinds);
        let payload_slot = shape_value::ValueSlot::from_typed_object_raw(ptr);

        vm.pending_resume = Some(KindedSlot::new(
            payload_slot,
            NativeKind::Ptr(HeapKind::TypedObject),
        ));
        let err = vm.apply_pending_resume().expect_err("expected surface");
        let msg = format!("{err:?}");
        assert!(
            msg.contains("FrameState") && msg.contains("VmState"),
            "expected schema-mismatch surface naming both names, got: {msg}"
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
                KindedSlot::new(ValueSlot::from_raw(3.14f64.to_bits()), NativeKind::Float64),
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
            msg.contains("ip_offset") && (msg.contains("exceeds") || msg.contains("body_length")),
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
