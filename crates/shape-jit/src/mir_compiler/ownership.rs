//! Ownership-aware codegen: Move, Copy, Drop.
//!
//! This is the core of what makes MirToIR correct where BytecodeToIR isn't:
//! - Move: read value, null source slot (prevents double-drop)
//! - Copy: read value, arc_retain if heap type (Arc::clone)
//! - Drop: arc_release for heap types, no-op for primitives
//!
//! ## Refcount discrimination (W11-jit-new-array, ADR-006 §2.7.5 / §2.7.6 / Q8)
//!
//! Post-strict-typing the kind IS the discriminator that decides refcount
//! semantics; there is no tag-bit probing. The discrimination here uses
//! [`shape_value::NativeKind::is_refcounted`] which returns `true` for the
//! two heap-pointer kinds (`String`, `Ptr(HeapKind::*)`) and `false` for
//! every numeric / bool / nullable-scalar kind — including `NativeKind::Int64`,
//! which the legacy `types::is_native_slot` predicate excluded (the legacy
//! exclusion was correct under the deleted ValueWord ABI where an `Int64`
//! slot might carry NaN-boxed pointer bits; under strict typing an `Int64`
//! slot stores a raw native `i64`, period).
//!
//! When the slot's `NativeKind` is not proven by either source (the
//! bytecode compiler's seed, the MIR-level forward/backward inference in
//! `infer_slot_kinds`), the response is **surface-and-stop** — never a
//! kind-blind fall-through to `arc_retain` / `arc_release`. Defaulting
//! "unknown kind → assume heap and retain" is the W-series Bool-default
//! defection-attractor (CLAUDE.md "Forbidden rationalizations": *"Soft-fail
//! counter for now, harden later."*) applied to a different surface; the
//! prior W11-jit-new-array close attempted the symmetric variant
//! ("unknown kind → silently skip retain") via a no-op FFI body, which
//! refcount-leaks every heap value the JIT routes through. Both are refused
//! on sight per §2.7.7 #9 / W10 jit-playbook §5.

use cranelift::prelude::*;
use std::collections::{HashMap, HashSet};

use super::MirToIR;
use shape_vm::mir::ControlFlowGraph;
use shape_vm::mir::analysis::OwnershipDecision;
use shape_vm::mir::types::*;
use shape_vm::type_tracking::NativeKind;

/// Refcount disposition for an ownership-aware codegen site.
///
/// Computed from the slot's proven `NativeKind` (per ADR-006 §2.7.5
/// stamp-at-compile-time). The variants encode every legitimate answer the
/// emitter can give without falling back to a kind-blind default.
#[derive(Debug, Clone, Copy)]
enum RefcountDisposition {
    /// The slot is a raw scalar / bool / nullable-scalar — emit no
    /// retain/release call.
    Skip,
    /// The slot is a heap-pointer kind (`String` / `Ptr(HeapKind::*)`) —
    /// emit the matching retain/release call.
    Refcounted,
    /// The slot is one of the "raw pointer to a typed cell" carriers the
    /// MIR uses for closure capture cells / shared cells / stack closures.
    /// These have their own dedicated retain/release path (the matching
    /// per-FieldKind FFI in `ffi/object/closure.rs`, or no retain at all
    /// for stack closures) — emit nothing here.
    SkipTypedCellCarrier,
}

impl<'a, 'b> MirToIR<'a, 'b> {
    pub(crate) fn mir_move_then_read_divergence_reason(&self) -> Option<String> {
        let cfg = ControlFlowGraph::build(&self.mir_data.mir);
        let block_by_id: HashMap<BasicBlockId, usize> = self
            .mir
            .blocks
            .iter()
            .enumerate()
            .map(|(idx, block)| (block.id, idx))
            .collect();
        let mut in_sets: HashMap<BasicBlockId, HashSet<SlotId>> = HashMap::new();
        let mut out_sets: HashMap<BasicBlockId, HashSet<SlotId>> = HashMap::new();
        let rpo = cfg.reverse_postorder();

        let mut changed = true;
        while changed {
            changed = false;
            for block_id in &rpo {
                let Some(&block_idx) = block_by_id.get(block_id) else {
                    continue;
                };
                let block = &self.mir.blocks[block_idx];
                let mut state = HashSet::new();
                for pred in cfg.predecessors(*block_id) {
                    if let Some(pred_out) = out_sets.get(pred) {
                        state.extend(pred_out.iter().copied());
                    }
                }
                if in_sets.get(block_id) != Some(&state) {
                    in_sets.insert(*block_id, state.clone());
                    changed = true;
                }

                for (stmt_idx, stmt) in block.statements.iter().enumerate() {
                    if let Some(reason) =
                        self.apply_move_read_statement_effects(block.id, stmt_idx, stmt, &mut state)
                    {
                        return Some(reason);
                    }
                }

                if let Some(reason) =
                    self.apply_move_read_terminator_effects(block.id, &block.terminator, &mut state)
                {
                    return Some(reason);
                }

                if out_sets.get(block_id) != Some(&state) {
                    out_sets.insert(*block_id, state);
                    changed = true;
                }
            }
        }

        None
    }

    fn apply_move_read_statement_effects(
        &self,
        block: BasicBlockId,
        stmt_idx: usize,
        stmt: &MirStatement,
        moved: &mut HashSet<SlotId>,
    ) -> Option<String> {
        match &stmt.kind {
            StatementKind::Assign(dest, rvalue) => {
                // Mirror `compile_statement`: typed-object literals lower as a
                // dead scratch `Assign(Aggregate(...))` followed by the real
                // `ObjectStore` / `EnumStore`. The JIT skips this aggregate
                // entirely, so its operands are not read or destructively moved
                // at this program point. Counting them here falsely marks the
                // field payload temp as moved, then the following real store
                // looks like a read-after-move.
                if matches!(rvalue, Rvalue::Aggregate(_)) && self.is_typed_object_slot(dest) {
                    return None;
                }
                let ownership_src = match rvalue {
                    Rvalue::Use(
                        Operand::Move(Place::Local(src)) | Operand::Copy(Place::Local(src)),
                    ) => Some((*src, self.mir_data.borrow_analysis.ownership_at(stmt.point))),
                    _ => None,
                };
                if let Some(reason) = self.apply_move_read_rvalue_effects(
                    block,
                    stmt_idx,
                    Some(stmt.point),
                    ownership_src,
                    rvalue,
                    moved,
                ) {
                    return Some(reason);
                }
                if let Place::Local(slot) = dest {
                    moved.remove(slot);
                }
                None
            }
            StatementKind::ArrayStore {
                container_slot,
                operands,
            } => {
                if let Some(reason) = self.apply_move_read_operands_effects(
                    block,
                    stmt_idx,
                    Some(stmt.point),
                    None,
                    operands,
                    moved,
                ) {
                    return Some(reason);
                }
                moved.remove(container_slot);
                None
            }
            StatementKind::ObjectStore {
                container_slot,
                operands,
                ..
            }
            | StatementKind::EnumStore {
                container_slot,
                operands,
                ..
            } => {
                if let Some(reason) = self.apply_move_read_operands_effects(
                    block,
                    stmt_idx,
                    Some(stmt.point),
                    None,
                    operands,
                    moved,
                ) {
                    return Some(reason);
                }
                moved.remove(container_slot);
                None
            }
            StatementKind::ClosureCapture {
                closure_slot,
                operands,
                ..
            } => {
                if let Some(reason) = self.apply_move_read_operands_effects(
                    block,
                    stmt_idx,
                    Some(stmt.point),
                    None,
                    operands,
                    moved,
                ) {
                    return Some(reason);
                }
                moved.remove(closure_slot);
                None
            }
            StatementKind::ModuleBindingStore { operands, .. }
            | StatementKind::TaskBoundary(operands, _) => self.apply_move_read_operands_effects(
                block,
                stmt_idx,
                Some(stmt.point),
                None,
                operands,
                moved,
            ),
            StatementKind::Drop(_) | StatementKind::Nop => None,
        }
    }

    fn apply_move_read_terminator_effects(
        &self,
        block: BasicBlockId,
        terminator: &Terminator,
        moved: &mut HashSet<SlotId>,
    ) -> Option<String> {
        match &terminator.kind {
            TerminatorKind::Call {
                func,
                args,
                destination,
                ..
            } => {
                if let Some(reason) =
                    self.apply_move_read_operand_effect(block, usize::MAX, None, None, func, moved)
                {
                    return Some(reason);
                }
                if let Some(reason) = self.apply_move_read_operands_effects(
                    block,
                    usize::MAX,
                    None,
                    None,
                    args,
                    moved,
                ) {
                    return Some(reason);
                }
                if let Place::Local(slot) = destination {
                    moved.remove(slot);
                }
                None
            }
            TerminatorKind::SwitchBool { operand, .. } => {
                self.apply_move_read_operand_effect(block, usize::MAX, None, None, operand, moved)
            }
            TerminatorKind::Goto(_) | TerminatorKind::Return | TerminatorKind::Unreachable => None,
        }
    }

    fn apply_move_read_rvalue_effects(
        &self,
        block: BasicBlockId,
        stmt_idx: usize,
        point: Option<Point>,
        ownership_src: Option<(SlotId, OwnershipDecision)>,
        rvalue: &Rvalue,
        moved: &mut HashSet<SlotId>,
    ) -> Option<String> {
        match rvalue {
            Rvalue::Use(op) | Rvalue::Clone(op) | Rvalue::UnaryOp(_, op) => self
                .apply_move_read_operand_effect(block, stmt_idx, point, ownership_src, op, moved),
            Rvalue::BinaryOp(_, lhs, rhs) | Rvalue::FuzzyComparison { lhs, rhs, .. } => {
                if let Some(reason) = self.apply_move_read_operand_effect(
                    block,
                    stmt_idx,
                    point,
                    ownership_src,
                    lhs,
                    moved,
                ) {
                    return Some(reason);
                }
                self.apply_move_read_operand_effect(
                    block,
                    stmt_idx,
                    point,
                    ownership_src,
                    rhs,
                    moved,
                )
            }
            Rvalue::Aggregate(operands) => self.apply_move_read_operands_effects(
                block,
                stmt_idx,
                point,
                ownership_src,
                operands,
                moved,
            ),
            Rvalue::Borrow(_, place) => {
                let root = place.root_local();
                if moved.contains(&root) {
                    Some(self.move_read_reason(block, stmt_idx, root, "borrow"))
                } else {
                    None
                }
            }
            Rvalue::EnumTest { operand, .. }
            | Rvalue::EnumPayload { operand, .. }
            | Rvalue::TypePatternTest { operand, .. }
            | Rvalue::EnumDiscriminantTest { operand, .. }
            | Rvalue::PrimitiveCast { operand, .. }
            | Rvalue::FormatValue { operand, .. } => self.apply_move_read_operand_effect(
                block,
                stmt_idx,
                point,
                ownership_src,
                operand,
                moved,
            ),
        }
    }

    fn apply_move_read_operands_effects(
        &self,
        block: BasicBlockId,
        stmt_idx: usize,
        point: Option<Point>,
        ownership_src: Option<(SlotId, OwnershipDecision)>,
        operands: &[Operand],
        moved: &mut HashSet<SlotId>,
    ) -> Option<String> {
        for operand in operands {
            if let Some(reason) = self.apply_move_read_operand_effect(
                block,
                stmt_idx,
                point,
                ownership_src,
                operand,
                moved,
            ) {
                return Some(reason);
            }
        }
        None
    }

    fn apply_move_read_operand_effect(
        &self,
        block: BasicBlockId,
        stmt_idx: usize,
        _point: Option<Point>,
        ownership_src: Option<(SlotId, OwnershipDecision)>,
        operand: &Operand,
        moved: &mut HashSet<SlotId>,
    ) -> Option<String> {
        let (place, destructive) = match operand {
            Operand::Copy(place) => {
                let destructive = matches!(
                    (place, ownership_src),
                    (Place::Local(slot), Some((src, OwnershipDecision::Move))) if *slot == src
                );
                (place, destructive)
            }
            Operand::Move(place) => {
                let destructive = !self.place_is_known_copy_value(place)
                    && !matches!(
                        (place, ownership_src),
                        (
                            Place::Local(slot),
                            Some((src, OwnershipDecision::Copy | OwnershipDecision::Clone))
                        ) if *slot == src
                    );
                (place, destructive)
            }
            Operand::MoveExplicit(place) => {
                let destructive = !self.place_is_known_copy_value(place);
                (place, destructive)
            }
            Operand::Constant(_) => return None,
        };

        let root = place.root_local();
        if moved.contains(&root) {
            return Some(self.move_read_reason(block, stmt_idx, root, "operand read"));
        }
        if destructive {
            if let Place::Local(slot) = place {
                moved.insert(*slot);
            }
        }
        None
    }

    fn move_read_reason(
        &self,
        block: BasicBlockId,
        stmt_idx: usize,
        slot: SlotId,
        site: &str,
    ) -> String {
        let stmt = if stmt_idx == usize::MAX {
            "terminator".to_string()
        } else {
            format!("statement {stmt_idx}")
        };
        format!(
            "slot {} was read after a destructive MIR move at block {} {stmt} ({site}); \
             JIT operand lowering would read a nulled source slot.",
            slot.0, block.0
        )
    }

    pub(crate) fn place_is_known_copy_value(&self, place: &Place) -> bool {
        if matches!(place, Place::Local(_)) {
            if let Some(kind) = self.place_native_kind(place) {
                return !kind.is_refcounted();
            }
        }
        matches!(
            self.local_types.get(place.root_local().0 as usize),
            Some(LocalTypeInfo::Copy)
        )
    }

    fn compile_read_by_ownership_decision(
        &mut self,
        place: &Place,
        decision: OwnershipDecision,
    ) -> Result<Value, String> {
        match decision {
            OwnershipDecision::Move => {
                let val = self.read_place(place)?;
                self.null_place(place)?;
                Ok(val)
            }
            OwnershipDecision::Copy | OwnershipDecision::Clone => {
                let val = self.read_place(place)?;
                if matches!(
                    self.refcount_disposition(place)?,
                    RefcountDisposition::Refcounted
                ) {
                    let retain_func = self.retain_func_for_place(place);
                    self.builder.ins().call(retain_func, &[val]);
                }
                Ok(val)
            }
            OwnershipDecision::DeepClone => Err("MirToIR ownership: SURFACE — statement-local \
                 OwnershipDecision::DeepClone is not yet lowered by the JIT \
                 MIR path. Whole-program/function deopt to the bytecode \
                 interpreter preserves VM == JIT until the kinded deep-clone \
                 helpers are threaded into MirToIR."
                .to_string()),
        }
    }

    /// Compute the refcount disposition for a place's root local.
    ///
    /// Returns the disposition or a surface-and-stop error when the slot's
    /// `NativeKind` cannot be resolved from either the bytecode-compiler
    /// seed or the MIR-level inference. The error path is the §2.7.7 #9
    /// principled response; no Bool-default fall-through.
    fn refcount_disposition(&self, place: &Place) -> Result<RefcountDisposition, String> {
        let slot = place.root_local();

        // Stack closures: the slot value is a raw Cranelift stack-slot
        // address, not a refcounted handle. (Phase E.)
        if let Place::Local(slot_id) = place {
            if self.stack_closure_slots.contains_key(slot_id) {
                return Ok(RefcountDisposition::SkipTypedCellCarrier);
            }
            // Track A.1D.2 / A.1E: OwnedMutable / Shared capture slots
            // hold raw `*mut ValueWord` / `*const SharedCell` pointers
            // whose lifecycle is owned by `release_typed_closure`
            // (`ClosureLayout`'s owned-mutable / shared masks). Frame-
            // exit retain/release on these slots would mis-interpret
            // the pointer as a NaN-boxed heap handle.
            if self.owned_mutable_capture_slots.contains_key(slot_id)
                || self.shared_capture_slots.contains_key(slot_id)
            {
                return Ok(RefcountDisposition::SkipTypedCellCarrier);
            }
            // Session 1 Commit 3: SharedCow outer-scope local slots
            // hold a `*const SharedCell` Arc pointer; their lifecycle
            // is `jit_arc_shared_release` (not the generic
            // `jit_arc_release`) at `Drop`. Skip here.
            if self.shared_local_slots.contains_key(slot_id) {
                return Ok(RefcountDisposition::SkipTypedCellCarrier);
            }
        }

        // v2 typed-array slots: the value is a raw `*mut TypedArray<T>`
        // pointer with inline `HeapHeader` refcount. The kinded `v2`
        // retain/release surface is the right path (a §2.7.14 follow-up);
        // skip the generic arc_retain/release here.
        if matches!(place, Place::Local(_)) && self.v2_typed_array_elem_kind(place).is_some() {
            return Ok(RefcountDisposition::SkipTypedCellCarrier);
        }

        // W12-jit-binop-after-heap-read-kind-tracker: for projection
        // places (`Place::Field` / `Place::Index`), the value being
        // copied is the field's / element's value, NOT the base struct/
        // array's heap handle. Refcount disposition must follow the
        // PROJECTED kind. `place_native_kind` does the project lookup
        // through the producer-side `field_native_kinds` map (§2.7.5
        // producer classification) for fields and through
        // `concrete_types`'s `Array<scalar>` shape for indexes — the
        // same kind sources the BinaryOp lowering picker uses.
        //
        // Without this projection, `Copy(Field(p_TypedObject, x_Int64))`
        // routed `refcount_disposition` to the base's `Ptr(TypedObject)`
        // kind (refcounted), then `compile_operand`'s Copy arm called
        // `arc_retain(i64_3_field_value)` — segfaulting in
        // `Arc::increment_strong_count` interpreting the integer 3 as
        // a pointer.
        match place {
            Place::Field(_, _) | Place::Index(_, _) => {
                match self.place_native_kind(place) {
                    Some(k) if k.is_refcounted() => {
                        return Ok(RefcountDisposition::Refcounted);
                    }
                    Some(_) => return Ok(RefcountDisposition::Skip),
                    None => {
                        // Projection kind genuinely unproven at this
                        // consumer site (e.g. the field name isn't in
                        // `field_native_kinds` because the producer-side
                        // walk didn't see the ObjectStore that stamps
                        // it, or the array's `ConcreteType` isn't
                        // `Array<scalar>`). Fall through to the
                        // root-local-kind dispatch below — that arm
                        // already has the surface-and-stop discipline
                        // for genuinely-unproven kinds via the
                        // `LocalTypeInfo` arms.
                    }
                }
            }
            _ => {}
        }

        // Authoritative kind source: the slot's proven `NativeKind` from
        // bytecode-compiler seed + MIR-level inference. Under §2.7.5
        // stamp-at-compile-time this is the canonical refcount
        // discriminator.
        let slot_kind = super::types::slot_kind_for_local(&self.slot_kinds, slot.0);
        match slot_kind {
            Some(k) if k.is_refcounted() => Ok(RefcountDisposition::Refcounted),
            Some(_) => Ok(RefcountDisposition::Skip),
            None => {
                // Kind genuinely unproven by both inference passes. Per
                // §2.7.7 #9 / CLAUDE.md "Forbidden rationalizations" the
                // emitter does NOT default to "assume heap and retain"
                // (the W-series Bool-default attractor); surface-and-stop
                // at JIT compile time so the program falls back to the
                // interpreter rather than refcount-leak / segfault.
                //
                // Practical fallback for the implicit-return slot 0 +
                // unused-tail-slot cases the MIR-inference pass leaves
                // unproven: those slots are never written via an Assign
                // the inference can see, so they never carry a live value
                // — emit no retain/release. We discriminate this from a
                // genuine kind-source gap via `LocalTypeInfo`: `Copy`
                // and `NonCopy` are bytecode-compiler-authoritative
                // (primitive / heap), `Unknown` is the "no annotation
                // and no Assign" path — that's the unused / implicit
                // slot, safe to skip.
                let type_info = self
                    .local_types
                    .get(slot.0 as usize)
                    .cloned()
                    .unwrap_or(LocalTypeInfo::Unknown);
                match type_info {
                    LocalTypeInfo::Copy => Ok(RefcountDisposition::Skip),
                    LocalTypeInfo::NonCopy => {
                        // The bytecode compiler classified this as heap,
                        // but MIR inference couldn't prove the kind.
                        // Surface-and-stop: a `NonCopy` slot needs a
                        // proven heap `NativeKind` to dispatch the
                        // correct retain (per-kind §2.7.6 / Q8). Falling
                        // through to a kind-blind retain on `String` /
                        // `Ptr(HeapKind::*)` ambiguity is the W-series
                        // attractor.
                        Err(format!(
                            "MirToIR ownership: SURFACE — slot {} has \
                             LocalTypeInfo::NonCopy but MIR inference did \
                             not prove its NativeKind. Refcount dispatch \
                             requires a proven `NativeKind::String` or \
                             `NativeKind::Ptr(HeapKind::*)` per ADR-006 \
                             §2.7.5 / §2.7.6 / Q8. Tracked as a \
                             W11-jit-new-array kind-source-gap follow-up. \
                             ADR-006 §2.7.7 #9 (no Bool-default fallback).",
                            slot.0
                        ))
                    }
                    LocalTypeInfo::Unknown => {
                        // Implicit-return / unused / dead-store slot —
                        // no live value, no refcount work. This is the
                        // structurally-safe arm: the slot has neither a
                        // proven kind nor a bytecode-compiler heap
                        // classification, so no Assign(_, _) flows a
                        // refcounted value into it.
                        Ok(RefcountDisposition::Skip)
                    }
                }
            }
        }
    }

    /// Public wrapper for `refcount_disposition` — returns `true` when the
    /// place's slot is heap-kinded (a refcount call is required). Used by
    /// `Rvalue::Clone` in `rvalues.rs` to share the same discrimination
    /// path as `compile_operand`'s `Copy` arm.
    pub(crate) fn refcount_disposition_for_place(&self, place: &Place) -> Result<bool, String> {
        Ok(matches!(
            self.refcount_disposition(place)?,
            RefcountDisposition::Refcounted
        ))
    }

    /// Compile an Operand, respecting Move/Copy ownership semantics.
    pub(crate) fn compile_operand(&mut self, operand: &Operand) -> Result<Value, String> {
        match operand {
            Operand::Move(place) | Operand::MoveExplicit(place) => {
                if self.place_is_known_copy_value(place) {
                    return self.read_place(place);
                }
                if matches!(operand, Operand::Move(_)) {
                    if let (Place::Local(slot), Some((_point, src, decision))) =
                        (place, self.current_stmt_local_ownership)
                    {
                        if *slot == src {
                            return self.compile_read_by_ownership_decision(place, decision);
                        }
                    }
                }
                // Move: read the value, then null the source to prevent double-drop.
                let val = self.read_place(place)?;
                self.null_place(place)?;
                Ok(val)
            }
            Operand::Copy(place) => {
                if let (Place::Local(slot), Some((_point, src, decision))) =
                    (place, self.current_stmt_local_ownership)
                {
                    if *slot == src {
                        return self.compile_read_by_ownership_decision(place, decision);
                    }
                }
                // Copy: read the value. For heap-kind slots, increment the refcount.
                let val = self.read_place(place)?;
                if matches!(
                    self.refcount_disposition(place)?,
                    RefcountDisposition::Refcounted
                ) {
                    let retain_func = self.retain_func_for_place(place);
                    self.builder.ins().call(retain_func, &[val]);
                }
                Ok(val)
            }
            Operand::Constant(constant) => self.compile_constant(constant),
        }
    }

    /// Pick the kind-appropriate retain FFI for a place. ADR-006 §2.7.17
    /// adopted `Arc<ResultData>` / `Arc<OptionData>` as the strict-typed
    /// Result/Option carriers; their refcount lives at offset -16 per
    /// Rust Arc contract, NOT at offset 4 like the legacy
    /// `UnifiedValue<T>` shape. The legacy `jit_arc_retain` would write
    /// to the wrong offset and corrupt the inner payload.
    ///
    /// Round 7A added `arc_result_retain` / `arc_option_retain` for
    /// Result/Option. W12-jit-collection-arc-ffi-ctors-and-refcount
    /// (Phase 3 cluster-0 Round 9 / 8B.1, 2026-05-13) extends the
    /// dispatch with 8 more typed-Arc collection carriers — HashSet,
    /// HashMap, Deque, PriorityQueue, Channel, Mutex, Atomic, Lazy.
    /// All 10 dispatch arms operate on `Arc::into_raw(Arc<XData>) as
    /// u64` carriers (refcount at offset -16); the legacy `arc_retain`
    /// fallback stays for kinds NOT in the typed-Arc family
    /// (Array / TypedObject / Closure / etc. — still on
    /// `UnifiedValue<T>` HeapHeader at offset 4).
    pub(crate) fn retain_func_for_place(&self, place: &Place) -> cranelift::codegen::ir::FuncRef {
        use shape_value::heap_value::HeapKind;
        use shape_vm::type_tracking::NativeKind;
        let kind = self.place_native_kind(place);
        match kind {
            Some(NativeKind::Ptr(HeapKind::Result)) => self.ffi.arc_result_retain,
            Some(NativeKind::Ptr(HeapKind::Option)) => self.ffi.arc_option_retain,
            Some(NativeKind::Ptr(HeapKind::HashSet)) => self.ffi.arc_hashset_retain,
            Some(NativeKind::Ptr(HeapKind::HashMap)) => self.ffi.arc_hashmap_retain,
            Some(NativeKind::Ptr(HeapKind::Deque)) => self.ffi.arc_deque_retain,
            Some(NativeKind::Ptr(HeapKind::PriorityQueue)) => self.ffi.arc_priorityqueue_retain,
            Some(NativeKind::Ptr(HeapKind::Channel)) => self.ffi.arc_channel_retain,
            Some(NativeKind::Ptr(HeapKind::Mutex)) => self.ffi.arc_mutex_retain,
            Some(NativeKind::Ptr(HeapKind::Atomic)) => self.ffi.arc_atomic_retain,
            Some(NativeKind::Ptr(HeapKind::Lazy)) => self.ffi.arc_lazy_retain,
            // ADR-019 §3 / #200 — the opaque foreign-reference carrier.
            Some(NativeKind::Ptr(HeapKind::ForeignRef)) => self.ffi.arc_foreign_ref_retain,
            // W12-jit-string-carrier-unification (Phase 3 cluster-0 Round 12
            // T2/T3, 2026-05-13). ADR-006 §2.7.5 `NativeKind::String` slots
            // carry `Arc::into_raw(Arc<String>) as u64`; retain bumps the
            // Rust Arc control-block refcount at offset -16 via
            // `Arc::increment_strong_count::<String>`. The legacy
            // `arc_retain` would write a `fetch_add` at offset +4 — inside
            // the `String` payload, scribbling on `ptr/cap/len`.
            Some(NativeKind::String) => self.ffi.arc_string_retain,
            // W15.2-LANG-4 jit-filter-predicate close (2026-05-18).
            // ADR-006 §2.7.11/Q12 `Ptr(HeapKind::Closure)` slots carry
            // `Arc::into_raw(Arc<HeapValue>) as u64` per
            // `jit_finalize_heap_closure`; retain dispatch goes through
            // the typed-Arc-shape `jit_arc_closure_retain`
            // (`Arc::increment_strong_count::<HeapValue>`). The legacy
            // `arc_retain` would write a `fetch_add` at offset +4 of
            // the HeapValue payload — corrupting the discriminant.
            Some(NativeKind::Ptr(HeapKind::Closure)) => self.ffi.arc_closure_retain,
            // r5c-2-β-δ-(α): v2-raw `*mut TypedArray<T>` carrier. The
            // legacy `arc_retain` writes a `fetch_add` at offset +4 (inside
            // the `UnifiedValue<T>` HeapHeader) — wrong for a `TypedArray<T>`
            // whose HeapHeader is at offset 0. Route to the dedicated v2
            // helper (`v2_retain` against the on-header refcount).
            Some(NativeKind::Ptr(HeapKind::TypedArray)) => self.ffi.v2_typed_array_retain,
            // Wave-7 jit-typed-pointer-migration: v2-raw `*mut TypedObjectStorage`
            // carrier (HeapHeader at offset 0). The legacy `arc_retain` writes a
            // `fetch_add` at offset +4 — inside the header's kind|flags region,
            // scribbling the object and misdirecting the GC barrier. Route to the
            // dedicated v2 helper (`v2_retain` against the on-header refcount),
            // exactly mirroring the TypedArray arm above.
            Some(NativeKind::Ptr(HeapKind::TypedObject)) => self.ffi.v2_typed_object_retain,
            _ => self.ffi.arc_retain,
        }
    }

    /// Mirror of `retain_func_for_place` for release.
    pub(crate) fn release_func_for_place(&self, place: &Place) -> cranelift::codegen::ir::FuncRef {
        use shape_value::heap_value::HeapKind;
        use shape_vm::type_tracking::NativeKind;
        let kind = self.place_native_kind(place);
        match kind {
            Some(NativeKind::Ptr(HeapKind::Result)) => self.ffi.arc_result_release,
            Some(NativeKind::Ptr(HeapKind::Option)) => self.ffi.arc_option_release,
            Some(NativeKind::Ptr(HeapKind::HashSet)) => self.ffi.arc_hashset_release,
            Some(NativeKind::Ptr(HeapKind::HashMap)) => self.ffi.arc_hashmap_release,
            Some(NativeKind::Ptr(HeapKind::Deque)) => self.ffi.arc_deque_release,
            Some(NativeKind::Ptr(HeapKind::PriorityQueue)) => self.ffi.arc_priorityqueue_release,
            Some(NativeKind::Ptr(HeapKind::Channel)) => self.ffi.arc_channel_release,
            Some(NativeKind::Ptr(HeapKind::Mutex)) => self.ffi.arc_mutex_release,
            Some(NativeKind::Ptr(HeapKind::Atomic)) => self.ffi.arc_atomic_release,
            Some(NativeKind::Ptr(HeapKind::Lazy)) => self.ffi.arc_lazy_release,
            // Mirror of the `Ptr(HeapKind::ForeignRef)` retain arm above.
            Some(NativeKind::Ptr(HeapKind::ForeignRef)) => self.ffi.arc_foreign_ref_release,
            // W12-jit-string-carrier-unification: mirror of the
            // `retain_func_for_place` String arm.
            Some(NativeKind::String) => self.ffi.arc_string_release,
            // W15.2-LANG-4 jit-filter-predicate close (2026-05-18).
            // Mirror of the `Ptr(HeapKind::Closure)` retain arm above.
            Some(NativeKind::Ptr(HeapKind::Closure)) => self.ffi.arc_closure_release,
            // r5c-2-β-δ-(α): v2-raw `*mut TypedArray<T>` carrier. The
            // legacy `arc_release` deallocs the wrong size + leaks the
            // element buffer (heap corruption). Route to the dedicated v2
            // helper (`v2_release` + stamped-element-type `drop_array` /
            // `drop_array_heap` on the last share).
            Some(NativeKind::Ptr(HeapKind::TypedArray)) => self.ffi.v2_typed_array_release,
            // Wave-7 jit-typed-pointer-migration: mirror of the TypedObject retain
            // arm. `v2_release` on the offset-0 header runs the heap-mask field
            // walk + frees on the last share; the legacy `arc_release` would
            // dealloc the wrong shape and corrupt the heap.
            Some(NativeKind::Ptr(HeapKind::TypedObject)) => self.ffi.v2_typed_object_release,
            _ => self.ffi.arc_release,
        }
    }

    /// Compile an operand without ownership tracking (raw value access).
    /// Used for index operands in Place::Index where we just need the value.
    pub(crate) fn compile_operand_raw(&mut self, operand: &Operand) -> Result<Value, String> {
        match operand {
            Operand::Move(place) | Operand::MoveExplicit(place) | Operand::Copy(place) => {
                self.read_place(place)
            }
            Operand::Constant(constant) => self.compile_constant(constant),
        }
    }

    /// Session 1 Commit 3: compile an operand for a `ClosureCapture`
    /// slot whose capture kind is `Shared`.
    ///
    /// Semantics: whether the source is a declaring-frame `var` promoted to
    /// SharedCow storage or an inherited Shared capture parameter, the next
    /// closure needs the RAW `*const SharedCell` pointer bits — never the
    /// locked payload. This matches the interpreter's
    /// `expressions/closures.rs` path, which emits
    /// `LoadLocal(outer_var_slot)` immediately after `AllocSharedLocal`
    /// to push the pointer bits that `op_make_closure` then feeds
    /// through `Arc::increment_strong_count`.
    ///
    /// For all other operand shapes, defer to the standard `compile_operand`.
    /// This keeps the Immutable / OwnedMutable capture paths untouched.
    pub(crate) fn compile_operand_for_shared_capture(
        &mut self,
        operand: &Operand,
    ) -> Result<Value, String> {
        if let super::shared_cells::SharedCaptureOperandLowering::RawCarrier { slot, .. } =
            super::shared_cells::classify_shared_capture_operand(
                operand,
                &self.shared_local_slots,
                &self.shared_capture_slots,
            )
        {
            // Bypass the lock-gated read in `read_place` and produce the raw
            // pointer bits held in either carrier origin. The Shared branch in
            // `emit_heap_closure` immediately retains this exact cell pointer.
            let var = self.local_var(slot)?;
            return Ok(self.builder.use_var(var));
        }
        self.compile_operand(operand)
    }

    /// Compile a MIR constant to a Cranelift value.
    ///
    /// Returns native types when possible (F64 for floats, I64 for ints, I8 for bools).
    /// Consumers that need an I64 slot (e.g. for a dynamic local) rely on
    /// `ensure_kind` in `conversions.rs` to do the width extension.
    /// Per ADR-006 §2.7.5 the JIT FFI carrier is `(u64, NativeKind)` — the
    /// constant's `NativeKind` is stamped at the call signature; the bits
    /// emitted here are raw native u64 with no NaN-box / `tag_bits` wrap.
    pub(crate) fn compile_constant(&mut self, constant: &MirConstant) -> Result<Value, String> {
        match constant {
            MirConstant::Int(n) => {
                // Raw native i64 bits; kind companion is `NativeKind::Int64`
                // stamped at the JIT-FFI carrier site.
                Ok(self.builder.ins().iconst(types::I64, *n))
            }
            MirConstant::Float(bits) => {
                // Native F64 — direct float constant. ~100x faster than FFI path.
                Ok(self.builder.ins().f64const(f64::from_bits(*bits)))
            }
            MirConstant::Decimal(_) => {
                // WS-8 (2026-05-22): surface-and-stop. The MIR producer now
                // preserves the decimal lexeme (`MirConstant::Decimal(s)`)
                // verbatim instead of the pre-WS-8 silent collapse to
                // `MirConstant::Float(0)` ("decimal not yet modeled") that
                // caused `print(1.5D)` to JIT-print `0.0` while VM printed
                // `1.5D` — a v0.3-gating silent wrong-answer divergence per
                // WS-8 audit §1.D. Native JIT decimal codegen would require
                // a `*const DecimalObj` carrier producer + a
                // `jit_print_decimal` FFI body + the print-dispatch arm
                // (terminators.rs `Some(NativeKind::DecimalV2) =>` arm).
                // Surface-and-stop instead routes the program through the
                // W12 fall-through to the bytecode interpreter (which
                // materializes via the VM's `NewDecimalV2` opcode and
                // prints correctly). VM == JIT, both run the interpreter
                // path. Native JIT decimal codegen is a follow-up.
                Err("Route A surface-and-stop: NotImplemented(SURFACE) — \
                     `MirConstant::Decimal` carries the decimal lexeme \
                     through MIR but native JIT decimal codegen is not yet \
                     wired (the `*const DecimalObj` producer + \
                     `jit_print_decimal` FFI + `terminators.rs` DecimalV2 \
                     print-dispatch arm together are a follow-up). The W12 \
                     fall-through to the bytecode interpreter runs the \
                     program under VM, which materializes decimals via \
                     `NewDecimalV2` and prints correctly. WS-8 audit §1.D / \
                     ADR-006 §2.7.5."
                    .to_string())
            }
            MirConstant::Bool(b) => {
                // Native I8 bool — 0 or 1.
                Ok(self.builder.ins().iconst(types::I8, *b as i64))
            }
            MirConstant::Char(c) => {
                // Phase 3 cluster-2 Round 4 cw-D-fam12 follow-up (instance 57,
                // 2026-05-16). ADR-006 §2.7.5 amendment Round 19 S1.5: Char is
                // a 4-byte scalar (codepoint in low 32 bits of `ValueSlot`).
                // Emit as Cranelift I32 — the `print` dispatch's
                // `NativeKind::Char` arm at `terminators.rs` ~679 narrows
                // I32/I64/I8 to I32 before calling `jit_print_char(u32)`
                // which takes the codepoint directly (mirror of scalar
                // `jit_print_i64` / `_f64` / `_bool` shape, scalar-by-value
                // FFI). Cranelift handles I32 in the parallel-kind track
                // without NaN-box / `tag_bits` wrap (§2.7.7 #4 / #7 forbidden).
                Ok(self.builder.ins().iconst(types::I32, *c as i64))
            }
            MirConstant::None => Ok(self.builder.ins().iconst(types::I64, 0i64)),
            MirConstant::StringId(id) => {
                // W12-jit-string-carrier-unification (Phase 3 cluster-0 Round
                // 12 T2/T3, 2026-05-13). ADR-006 §2.7.5: a `NativeKind::String`
                // slot carries `Arc::into_raw(Arc<String>) as u64`, refcount
                // at offset -16 per the standard Rust Arc layout. The VM-side
                // consumer (`set_methods.rs::result_slot_to_string_arc` and
                // `KindedSlot::Drop` for `NativeKind::String`) decodes via
                // `Arc::from_raw(bits as *const String)` / `Arc::decrement_
                // strong_count::<String>(bits)`. Pre-Round-12 this site
                // emitted `box_string(s)` returning `Box::into_raw(Box::new(
                // UnifiedValue<Arc<String>>))` — wrong carrier shape; the
                // VM consumer's `Arc::from_raw` read the UnifiedValue header
                // bytes as `String` pointer/cap/len, segfaulting on access.
                //
                // `arc_string_constant` boosts the initial refcount to keep
                // the constant alive across the JIT-compiled function's
                // full lifetime — see the helper's docstring for the
                // permanent-share discipline.
                //
                // cluster-2-jit-string-const-loop-retain-gap (Phase 3
                // cluster-2 Round 2, 2026-05-16). The per-consumption
                // `jit_arc_string_retain` call below produces a fresh
                // active share each time the constant is consumed by a
                // downstream `StatementKind::Assign(Local(slot), Use(
                // Constant(Str(...))))` (or any operand-consuming context).
                // Without it, the second consumption of the same
                // `MirConstant::StringId`/`Str`/`Method` in a loop body
                // would have its slot's `release_old_value_if_heap` (or
                // `emit_drop`) call `jit_arc_string_release` on the
                // permanent share, freeing the constant — the cw-E
                // empirical finding at `/tmp/cw-E-prog4-string-in-loop.
                // shape` (STRING_RETAIN_CALLS=0 / STRING_RELEASE_CALLS=99
                // / STRING_RELEASE_FREES=1 across 100 iterations).
                // Producer-side retain mirrors the same retain-on-produce
                // discipline the W11-jit-new-array refcount audit
                // established for `jit_new_array` / `jit_v2_collection_*`
                // ctor sites and the §2.7.5 carrier-shape boundary rule.
                let idx = *id as usize;
                if idx < self.strings.len() {
                    let s = self.strings[idx].clone();
                    let boxed = crate::ffi::string::arc_string_constant(s);
                    let bits = self.builder.ins().iconst(types::I64, boxed as i64);
                    self.builder.ins().call(self.ffi.arc_string_retain, &[bits]);
                    Ok(bits)
                } else {
                    Ok(self.builder.ins().iconst(types::I64, 0i64))
                }
            }
            MirConstant::Str(s) => {
                // String literal carried in MIR. Same §2.7.5 producer
                // discipline as `MirConstant::StringId` above —
                // `Arc::into_raw(Arc<String>) as u64` with refcount boosted
                // for constant-lifetime stability + per-consumption retain
                // for the cluster-2-jit-string-const-loop-retain-gap fix
                // (see `StringId` arm docstring for the producer-side
                // retain rationale).
                let boxed = crate::ffi::string::arc_string_constant(s.clone());
                let bits = self.builder.ins().iconst(types::I64, boxed as i64);
                self.builder.ins().call(self.ffi.arc_string_retain, &[bits]);
                Ok(bits)
            }
            MirConstant::Function(name) => {
                // ADR-020 §3.4 / #239 §6.2 — THE CARRIER FLIP. A named
                // function reference is an immortal zero-capture
                // `Arc<HeapValue::ClosureRaw>` record, the VM's carrier,
                // stamped `Ptr(HeapKind::Closure)` and true.
                //
                // This arm used to emit `box_function(idx)` — a NaN-boxed
                // tag word `0xfffd_0000_0000_00<fid>` — under a `UInt64`
                // stamp, while the sibling `ClosurePlaceholder` arm emitted
                // bit-identical values under `Ptr(HeapKind::Closure)`. Two
                // carriers selected by which `MirConstant` variant the MIR
                // happened to use is source-shape-selected semantics, and
                // the second stamp was a lie: it told every consumer that
                // trusts the kind that the slot held a refcounted heap
                // pointer, so capturing such a closure retained a tag word
                // (#254 variant A, SIGSEGV in the ALLOCATION of the
                // capturing closure, not in any dispatch).
                //
                // Per-consumption retain (§6.4 verdict 2), same discipline
                // as the string-constant arms above: the pool's share is
                // per-emit-site, but this `iconst` is evaluated once per
                // EXECUTION and `jit_call_value` retires a share per call.
                // Without the retain the second dispatch is a use-after-
                // free on a constant.
                if let Some(&idx) = self.function_indices.get(name.as_str()) {
                    let boxed = crate::ffi::object::closure::arc_closure_constant(idx);
                    let bits = self.builder.ins().iconst(types::I64, boxed as i64);
                    self.builder.ins().call(self.ffi.arc_closure_retain, &[bits]);
                    Ok(bits)
                } else {
                    Err(format!(
                        "MirToIR: SURFACE — `{}` is used as a function value but is not in the \
                         JIT function table, so there is no fid to build its closure record \
                         from. The pre-#239 arm emitted a bare `iconst 0` here, which is a \
                         null in a slot stamped `Ptr(HeapKind::Closure)` (ADR-020 §3.4).",
                        name
                    ))
                }
            }
            MirConstant::Method(name) => {
                // Method name. Per `rvalues.rs:310` the operand-kind stamp
                // for `MirConstant::Method` is `NativeKind::String` — the
                // §2.7.5 String carrier shape. W12-jit-string-carrier-
                // unification migrates this arm to match the stamp.
                //
                // Note: `MirConstant::Method` is principally used as the
                // `func` field of a Call terminator (see `terminators.rs`
                // method-call path); the method-name push at line 235 of
                // that file uses `crate::ffi::value_ffi::box_string`
                // directly (JIT-internal NaN-box; dispatch shell decodes
                // via the same NaN-box `unbox_string`). This
                // `compile_constant` arm covers the residual case where
                // `MirConstant::Method` flows as a value operand — its
                // stamp on the parallel-kind track says `String`, so the
                // §2.7.5 Arc-shape carrier is the correct producer.
                //
                // cluster-2-jit-string-const-loop-retain-gap (Phase 3
                // cluster-2 Round 2, 2026-05-16): per-consumption retain
                // mirrors the `StringId` / `Str` arms above — the same
                // §2.7.5 carrier-shape retain-on-produce discipline
                // applies whenever the consumer's slot may run a
                // matching `release_old_value_if_heap` / `emit_drop`.
                let boxed = crate::ffi::string::arc_string_constant(name.clone());
                let bits = self.builder.ins().iconst(types::I64, boxed as i64);
                self.builder.ins().call(self.ffi.arc_string_retain, &[bits]);
                Ok(bits)
            }
            MirConstant::ClosurePlaceholder => {
                // Canonical path: the bytecode compiler's back-patcher rewrites
                // this to `Function(name)` during final MIR assembly
                // (`shape-vm/src/compiler/functions.rs` + `compiler_impl_reference_model.rs`).
                //
                // JIT-side fallback: monomorphization-triggered
                // `compile_function` clears `closure_function_ids` before the
                // top-level MIR patching runs, so unpatched placeholders leak
                // into the MIR we receive for top-level code. `scan_closure_placeholder_fids`
                // (called at MirToIR construction time) replays the same scan the
                // bytecode patcher would have run and resolves the N-th unpaired
                // placeholder to `__closure_<N>` via `function_indices`. We
                // consume that pairing here in statement-visit order.
                let idx = self.next_closure_placeholder_idx.get();
                self.next_closure_placeholder_idx.set(idx + 1);
                let fid_opt = self.closure_placeholder_fids.get(idx).copied();
                if let Some(fid) = fid_opt {
                    if fid != u16::MAX {
                        // ADR-020 §3.4 / #239 §6.2 — same carrier as the
                        // `Function` arm above, and now the same stamp.
                        let boxed = crate::ffi::object::closure::arc_closure_constant(fid);
                        let bits = self.builder.ins().iconst(types::I64, boxed as i64);
                        self.builder.ins().call(self.ffi.arc_closure_retain, &[bits]);
                        return Ok(bits);
                    }
                }
                // Exhausted side-table or the `u16::MAX` sentinel — a
                // capture-paired placeholder whose closure allocation is
                // handled by `emit_heap_closure` / `emit_stack_closure`, so
                // this Assign is a dead store the caller discards.
                //
                // #239: the pre-flip arm emitted `iconst 0` here, which is a
                // bare null in a slot stamped `Ptr(HeapKind::Closure)` —
                // §6.2 item 2 names it. It is kept as a null ONLY because
                // the value is provably discarded; a consumer that read it
                // would be reading a null closure pointer, which
                // `jit_call_value`'s `Ptr(Closure)` arm already refuses
                // fail-closed (`callee_bits == 0` → `pending_call_error`).
                Ok(self.builder.ins().iconst(types::I64, 0i64))
            }
        }
    }

    /// Emit Drop for a local: release refcount if it's a heap type.
    pub(crate) fn emit_drop(&mut self, place: &Place) -> Result<(), String> {
        // Session 1 Commit 3 SharedCow path: outer-scope local slots
        // holding a `*const SharedCell` Arc pointer use the dedicated
        // `jit_arc_shared_release` (not the generic `arc_release`).
        // Handled here BEFORE the generic disposition because the
        // disposition's `SkipTypedCellCarrier` arm would otherwise
        // suppress this required release.
        if let Place::Local(slot_id) = place {
            if self.shared_local_slots.contains_key(slot_id) {
                let var = self.local_var(*slot_id)?;
                let cell_ptr = self.builder.use_var(var);
                self.builder
                    .ins()
                    .call(self.ffi.arc_shared_release, &[cell_ptr]);
                // Mark the slot spent. 0 is a genuine null pointer,
                // distinct from NONE_BITS; matches the interpreter's
                // `self.stack[slot] = 0u64` step in
                // `op_drop_shared_local`.
                let zero = self.builder.ins().iconst(types::I64, 0);
                self.builder.def_var(var, zero);
                return Ok(());
            }
        }

        let disposition = self.refcount_disposition(place)?;
        match disposition {
            RefcountDisposition::Refcounted => {
                let val = self.read_place(place)?;
                let release_func = self.release_func_for_place(place);
                self.builder.ins().call(release_func, &[val]);
                self.null_place(place)?;
            }
            RefcountDisposition::Skip => {
                // Raw scalar / unused-tail-slot — no refcount work.
                // Still null the slot per the use-after-drop contract:
                // scalar slots get clobbered to 0, which is the
                // default-init value the runtime expects on re-read.
                self.null_place(place)?;
            }
            RefcountDisposition::SkipTypedCellCarrier => {
                // OwnedMutable / Shared capture slots: lifecycle is
                // owned by `release_typed_closure`; per the
                // Track A.1D.2 / A.1E SAFETY notes the slot must NOT
                // be nulled here (the cell pointer is reclaimed by
                // the closure-drop, not by frame-exit).
                //
                // v2 typed-array / stack-closure slots: null the slot
                // to prevent use-after-drop reads from picking up the
                // raw pointer bits. (Match the prior behaviour.)
                let null_slot = match place {
                    Place::Local(slot_id) => {
                        !(self.owned_mutable_capture_slots.contains_key(slot_id)
                            || self.shared_capture_slots.contains_key(slot_id))
                    }
                    _ => true,
                };
                if null_slot {
                    self.null_place(place)?;
                }
            }
        }
        Ok(())
    }

    /// Release the old value of a local before overwriting it.
    /// This prevents Arc leaks when a heap local is reassigned.
    pub(crate) fn release_old_value_if_heap(&mut self, place: &Place) -> Result<(), String> {
        // Skip non-local places — only Place::Local supplies the
        // discrimination plumbing.
        if !matches!(place, Place::Local(_)) {
            return Ok(());
        }
        let disposition = self.refcount_disposition(place)?;
        match disposition {
            RefcountDisposition::Refcounted => {
                let old_val = self.read_place(place)?;
                let release_func = self.release_func_for_place(place);
                self.builder.ins().call(release_func, &[old_val]);
            }
            RefcountDisposition::Skip | RefcountDisposition::SkipTypedCellCarrier => {
                // Scalar / typed-cell-carrier slots: no refcount work.
                // (TypedCellCarrier: the dedicated reclaim path runs at
                // Drop / closure-drop, not at reassign.)
            }
        }
        Ok(())
    }
}

// Silence unused-import warnings — `NativeKind` is imported for the
// `RefcountDisposition` deductions in `refcount_disposition`; if the
// reader uses no `NativeKind` directly, this stays a documentation
// anchor for the kind-discriminator import.
const _: fn() = || {
    let _ = NativeKind::Int64;
};
