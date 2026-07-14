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

    /// Allocate every validated scalar SharedCell before compiling the body.
    /// All refusal checks run before the first allocation is emitted.
    pub(crate) fn initialize_shared_local_slots(&mut self) -> Result<(), String> {
        let mut slots = self
            .shared_local_slots
            .iter()
            .map(|(slot, evidence)| evidence.validated(*slot).map(|kind| (*slot, kind)))
            .collect::<Result<Vec<_>, _>>()?;
        slots.sort_by_key(|(slot, _)| slot.0);

        for (slot, kind) in &slots {
            if kind.is_refcounted() {
                return Err(format!(
                    "SURFACE (ADR-006 §2.7.8 / Q10): SharedCell for local slot {slot} has a \
                     REFCOUNTED payload kind ({kind:?}). The JIT shared-cell store lowering \
                     writes raw bits without retaining the new value or releasing the previous \
                     value. Whole-function JIT bail before cell allocation."
                ));
            }
        }

        for (slot, kind) in slots {
            let Some(&variable) = self.locals.get(&slot) else {
                continue;
            };
            let initial = self
                .builder
                .ins()
                .iconst(types::I64, crate::ffi::value_ffi::TAG_NULL as i64);
            let kind_code = crate::ffi::stack_kind_code::encode(kind);
            let kind_value = self.builder.ins().iconst(types::I8, kind_code as i64);
            let call = self
                .builder
                .ins()
                .call(self.ffi.alloc_shared_cell, &[initial, kind_value]);
            let cell_pointer = self.builder.inst_results(call)[0];
            self.builder.def_var(variable, cell_pointer);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_source_disagreement_is_a_codegen_error() {
        let evidence = SharedLocalKindEvidence {
            layout: Some(NativeKind::Bool),
            inferred: Some(NativeKind::Float64),
            layout_conflict: None,
        };

        let error = evidence
            .validated(SlotId(7))
            .expect_err("disagreeing producer evidence must never select a kind");
        assert!(error.contains("kind-source disagreement"));
        assert!(error.contains("before cell allocation"));
    }

    #[test]
    fn agreeing_sources_resolve_to_the_layout_kind() {
        let evidence = SharedLocalKindEvidence {
            layout: Some(NativeKind::Bool),
            inferred: Some(NativeKind::Bool),
            layout_conflict: None,
        };

        assert_eq!(evidence.validated(SlotId(7)), Ok(NativeKind::Bool));
    }
}
