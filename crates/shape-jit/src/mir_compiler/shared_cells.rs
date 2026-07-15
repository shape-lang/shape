//! Shared-local discovery, kind proof, and allocation.
//!
//! A captured `var` has two physical consumers: the declaring frame and the
//! closure body. Both must interpret the cell payload through one validated
//! `NativeKind`; a debug-only comparison or last-writer-wins choice would make
//! release builds capable of stamping one kind and loading another.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use cranelift::prelude::*;
use shape_value::NativeKind;
use shape_value::v2::closure_layout::{CaptureKind, ClosureLayout};
use shape_vm::bytecode::MirFunctionData;
use shape_vm::mir::types::{Operand, Place, SlotId, StatementKind};
use shape_vm::type_tracking::{BindingStorageClass, EscapeStatus};

use super::{MirToIR, types as slot_types};

/// Candidate kind sources for one declaring-frame SharedCell.
///
/// The closure layout is authoritative when present because the closure body
/// consumes that same layout. Inferred slot evidence is the fallback for the
/// storage-plan-only path and an unconditional cross-check when both exist.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct SharedLocalKindEvidence {
    layout: Option<NativeKind>,
    inferred: Option<NativeKind>,
    layout_conflict: Option<(NativeKind, NativeKind)>,
}

/// Which lowering a Shared `ClosureCapture` operand requires.
///
/// Both declaring-frame SharedCow locals and inherited Shared capture params
/// store raw `*const SharedCell` carrier bits in their Cranelift variable. A
/// normal place read projects the locked payload, so these two origins must
/// select the raw-carrier path before `emit_heap_closure` retains the cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SharedCaptureOperandLowering {
    RawCarrier {
        slot: SlotId,
        origin: SharedCellCarrierOrigin,
    },
    ProjectedPayload,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SharedCellCarrierOrigin {
    DeclaringFrameLocal,
    InheritedCapture,
}

impl SharedCellCarrierOrigin {
    fn diagnostic_name(self) -> &'static str {
        match self {
            Self::DeclaringFrameLocal => "declaring-frame Shared local",
            Self::InheritedCapture => "inherited Shared capture parameter",
        }
    }
}

pub(crate) fn classify_shared_capture_operand(
    operand: &Operand,
    shared_local_slots: &HashMap<SlotId, SharedLocalKindEvidence>,
    shared_capture_slots: &HashMap<SlotId, NativeKind>,
) -> SharedCaptureOperandLowering {
    let slot = match operand {
        Operand::Move(Place::Local(slot))
        | Operand::MoveExplicit(Place::Local(slot))
        | Operand::Copy(Place::Local(slot)) => *slot,
        _ => return SharedCaptureOperandLowering::ProjectedPayload,
    };

    let origin = if shared_local_slots.contains_key(&slot) {
        SharedCellCarrierOrigin::DeclaringFrameLocal
    } else if shared_capture_slots.contains_key(&slot) {
        SharedCellCarrierOrigin::InheritedCapture
    } else {
        return SharedCaptureOperandLowering::ProjectedPayload;
    };

    SharedCaptureOperandLowering::RawCarrier { slot, origin }
}

impl SharedLocalKindEvidence {
    fn record_inferred(&mut self, kind: Option<NativeKind>) {
        if self.inferred.is_none() {
            self.inferred = kind;
        }
    }

    fn record_layout(&mut self, kind: NativeKind) {
        match self.layout {
            Some(previous) if previous != kind => {
                self.layout_conflict.get_or_insert((previous, kind));
            }
            Some(_) => {}
            None => self.layout = Some(kind),
        }
    }

    /// Resolve exactly one kind or return a clean codegen refusal.
    pub(crate) fn validated(self, slot: SlotId) -> Result<NativeKind, String> {
        if let Some((first, second)) = self.layout_conflict {
            return Err(format!(
                "SURFACE (ADR-006 §2.7.8 / Q10): SharedCell for local slot {slot} has \
                 conflicting closure-layout NativeKinds ({first:?} versus {second:?}). \
                 A declaring frame and its closure bodies must consume one compiler-issued \
                 payload discriminator. Whole-function JIT bail before cell allocation."
            ));
        }
        match (self.layout, self.inferred) {
            (Some(layout), Some(inferred)) if layout != inferred => Err(format!(
                "SURFACE (ADR-006 §2.7.8 / Q10): SharedCell kind-source disagreement on \
                 local slot {slot}: ClosureLayout says {layout:?}, while inferred slot \
                 evidence says {inferred:?}. The closure layout is authoritative, but the \
                 declaring-frame evidence must agree before either side may emit a load, \
                 store, or Drop. Whole-function JIT bail before cell allocation."
            )),
            (Some(layout), _) => Ok(layout),
            (None, Some(inferred)) => Ok(inferred),
            (None, None) => Err(format!(
                "SURFACE (ADR-006 §2.7.8 / Q10): SharedCell for local slot {slot} has no \
                 derivable NativeKind companion. Neither ClosureLayout capture evidence nor \
                 inferred slot evidence proves the payload discriminator, and defaulting it \
                 is forbidden. Whole-function JIT bail before cell allocation."
            )),
        }
    }
}

/// Prove that the compact JIT ABI can recover this exact SharedCell kind.
///
/// This check runs for every declaring-frame Shared local and inherited Shared
/// capture parameter before the first body instruction or allocation is
/// emitted. An encoding/catalog defect therefore produces a whole-function JIT
/// refusal instead of calling the defensive FFI branch that returns null.
fn validated_shared_cell_kind_code(
    slot: SlotId,
    kind: NativeKind,
    origin: SharedCellCarrierOrigin,
) -> Result<u8, String> {
    let origin = origin.diagnostic_name();
    if matches!(kind, NativeKind::Ptr(heap_kind) if !heap_kind.has_kinded_slot_carrier()) {
        return Err(format!(
            "INTERNAL SharedCell kind invariant: {origin} slot {slot} resolved to \
             unsupported {kind:?}. The exhaustive ConcreteType capture-kind issuer cannot \
             produce a carrier-less kind; forged or external layout metadata must be \
             rejected before JIT emission or allocation."
        ));
    }
    let code = crate::ffi::stack_kind_code::encode(kind);
    match crate::ffi::stack_kind_code::decode(code) {
        Some(decoded) if decoded == kind => Ok(code),
        Some(decoded) => Err(format!(
            "SURFACE (ADR-006 §2.7.7 / Q9 + §2.7.8 / Q10): SharedCell for {origin} \
             slot {slot} encodes payload kind {kind:?} as byte {code}, but the canonical \
             JIT kind catalog decodes it as {decoded:?}. Whole-function JIT bail before \
             cell allocation or code emission."
        )),
        None => Err(format!(
            "SURFACE (ADR-006 §2.7.7 / Q9 + §2.7.8 / Q10): SharedCell for {origin} \
             slot {slot} encodes payload kind {kind:?} as byte {code}, but that byte is \
             absent from the canonical JIT kind catalog. Whole-function JIT bail before \
             cell allocation or code emission."
        )),
    }
}

/// Discover declaring-frame slots promoted to SharedCell storage and retain
/// both producer-side kind proofs for validation before code emission.
pub(crate) fn discover_shared_local_slots(
    mir_data: &MirFunctionData,
    slot_kinds: &[Option<NativeKind>],
    closure_layouts: &HashMap<u16, Arc<ClosureLayout>>,
) -> HashMap<SlotId, SharedLocalKindEvidence> {
    let param_slots: HashSet<SlotId> = mir_data.mir.param_slots.iter().copied().collect();
    let mut result = HashMap::<SlotId, SharedLocalKindEvidence>::new();

    // Storage-plan evidence covers the normal SharedCow path. Only captured
    // non-parameter locals receive the bytecode AllocSharedLocal lifecycle.
    for (slot, class) in &mir_data.storage_plan.slot_classes {
        if !matches!(class, BindingStorageClass::SharedCow) || param_slots.contains(slot) {
            continue;
        }
        let captured = mir_data
            .storage_plan
            .slot_semantics
            .get(slot)
            .map(|sem| matches!(sem.escape_status, EscapeStatus::Captured))
            .unwrap_or(false);
        if !captured {
            continue;
        }
        result
            .entry(*slot)
            .or_default()
            .record_inferred(slot_types::slot_kind_for_local(slot_kinds, slot.0));
    }

    // Some pipelines classify a source `var` as LocalMutablePtr rather than
    // SharedCow even though bytecode emits the Shared lifecycle. A Shared
    // ClosureLayout capture is the authoritative second discovery route.
    for block in &mir_data.mir.blocks {
        for statement in &block.statements {
            let StatementKind::ClosureCapture {
                operands,
                function_id,
                ..
            } = &statement.kind
            else {
                continue;
            };
            let Some(layout) = (*function_id).and_then(|id| closure_layouts.get(&id)) else {
                continue;
            };

            for (index, operand) in operands.iter().enumerate().take(layout.capture_count()) {
                if !matches!(layout.capture_storage_kind(index), CaptureKind::Shared) {
                    continue;
                }
                let slot = match operand {
                    Operand::Copy(Place::Local(slot))
                    | Operand::Move(Place::Local(slot))
                    | Operand::MoveExplicit(Place::Local(slot)) => *slot,
                    _ => continue,
                };
                if param_slots.contains(&slot) {
                    continue;
                }

                let inferred = slot_types::slot_kind_for_local(slot_kinds, slot.0);
                let layout_kind = layout.capture_native_kind(index);
                let evidence = result.entry(slot).or_default();
                evidence.record_inferred(inferred);
                evidence.record_layout(layout_kind);
            }
        }
    }

    result
}

impl<'a, 'b> MirToIR<'a, 'b> {
    /// Reject forged Shared payload metadata before any Cranelift instruction
    /// or allocation is emitted. Semantic capture issuers cannot hit this.
    pub(crate) fn validate_shared_cell_kinds(&self) -> Result<(), String> {
        for (slot, evidence) in &self.shared_local_slots {
            let kind = evidence.validated(*slot)?;
            validated_shared_cell_kind_code(
                *slot,
                kind,
                SharedCellCarrierOrigin::DeclaringFrameLocal,
            )?;
        }
        for (slot, kind) in &self.shared_capture_slots {
            validated_shared_cell_kind_code(
                *slot,
                *kind,
                SharedCellCarrierOrigin::InheritedCapture,
            )?;
        }
        Ok(())
    }

    /// Return the same validated payload kind used for allocation, reads,
    /// writes, and eventual SharedCell drop.
    pub(crate) fn validated_shared_local_kind(&self, slot: SlotId) -> Result<NativeKind, String> {
        self.shared_local_slots
            .get(&slot)
            .copied()
            .ok_or_else(|| {
                format!(
                    "SURFACE (ADR-006 §2.7.8 / Q10): local slot {slot} reached SharedCell \
                     lowering without discovery evidence. Whole-function JIT bail."
                )
            })?
            .validated(slot)
    }

    /// Read one projected value while preserving the cell's payload share.
    ///
    /// Scalar payloads retain the existing inline lock/CAS hot path. A
    /// refcounted payload must mint a new typed share while the cell is locked,
    /// because the cell continues to own its original share. The Ptr FFI reads
    /// the immutable `SharedCell::kind()` companion and clones through the
    /// canonical GC-aware kinded dispatch; it never treats payload bits as the
    /// cell pointer or reconstructs ownership from `FieldKind::Ptr`.
    pub(crate) fn emit_shared_payload_read(
        &mut self,
        cell_pointer: Value,
        kind: NativeKind,
    ) -> Value {
        if kind.is_refcounted() {
            let call = self
                .builder
                .ins()
                .call(self.ffi.read_shared_cell_ptr, &[cell_pointer]);
            let payload = self.builder.inst_results(call)[0];
            return self.ensure_kind(payload, kind);
        }

        use shape_value::v2::closure_layout::SHARED_CELL_VALUE_OFFSET;
        self.emit_shared_lock(cell_pointer);
        let payload = self.builder.ins().load(
            types::I64,
            MemFlags::trusted(),
            cell_pointer,
            SHARED_CELL_VALUE_OFFSET,
        );
        self.emit_shared_unlock(cell_pointer);
        self.ensure_kind(payload, kind)
    }

    /// Replace one projected value, transferring the incoming share to the cell.
    ///
    /// The refcounted Ptr FFI swaps under the cell lock and retires the previous
    /// `(bits, kind)` through the canonical GC-aware kinded drop dispatch.
    /// Scalar payloads keep the inline lock/store/unlock sequence.
    pub(crate) fn emit_shared_payload_write(
        &mut self,
        cell_pointer: Value,
        value: Value,
        kind: NativeKind,
    ) {
        let typed_value = self.ensure_kind(value, kind);
        let bits = self.coerce_value_to_i64_bits(typed_value);
        if kind.is_refcounted() {
            self.builder
                .ins()
                .call(self.ffi.write_shared_cell_ptr, &[cell_pointer, bits]);
            return;
        }

        use shape_value::v2::closure_layout::SHARED_CELL_VALUE_OFFSET;
        self.emit_shared_lock(cell_pointer);
        self.builder.ins().store(
            MemFlags::trusted(),
            bits,
            cell_pointer,
            SHARED_CELL_VALUE_OFFSET,
        );
        self.emit_shared_unlock(cell_pointer);
    }

    /// Allocate every validated SharedCell before compiling the body.
    /// All kind-source checks run before the first allocation is emitted.
    pub(crate) fn initialize_shared_local_slots(&mut self) -> Result<(), String> {
        let mut slots = self
            .shared_local_slots
            .iter()
            .map(|(slot, evidence)| {
                let kind = evidence.validated(*slot)?;
                let kind_code = validated_shared_cell_kind_code(
                    *slot,
                    kind,
                    SharedCellCarrierOrigin::DeclaringFrameLocal,
                )?;
                Ok((*slot, kind_code))
            })
            .collect::<Result<Vec<_>, String>>()?;
        slots.sort_by_key(|(slot, _)| slot.0);

        for (slot, kind_code) in slots {
            let Some(&variable) = self.locals.get(&slot) else {
                continue;
            };
            // Zero is the canonical empty payload for every typed carrier:
            // `SharedCell::Drop` and the canonical
            // `clone_with_kind`/`drop_with_kind` helpers return before dispatch
            // when bits == 0. In particular, this is a valid empty value for
            // every `NativeKind::is_refcounted()` arm, unlike the old non-zero
            // TAG_NULL bit pattern (which would be an invalid raw pointer under
            // a refcounted kind). The first source assignment transfers the
            // real payload share into the cell.
            let initial = self.builder.ins().iconst(types::I64, 0);
            let kind_value = self.builder.ins().iconst(types::I8, kind_code as i64);
            let call = self
                .builder
                .ins()
                .call(self.ffi.alloc_shared_cell, &[initial, kind_value]);
            let cell_pointer = self.builder.inst_results(call)[0];
            // `kind_code` was proven decodable before this loop emitted any
            // allocation. The accepted FFI branch is exactly
            // `Arc::new(SharedCell) -> Arc::into_raw`: it either aborts on
            // allocation failure or returns a non-null pointer. Its null
            // sentinel is reserved for malformed external codes and cannot
            // reach this generated store.
            self.builder.def_var(variable, cell_pointer);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
