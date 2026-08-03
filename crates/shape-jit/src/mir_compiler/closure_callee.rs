//! Forward resolution of statically-determined closure callees (ADR-020 /
//! #239, design doc §6.7 "the honest route").
//!
//! # What this is for
//!
//! `TerminatorKind::Call` has two callee shapes. A DIRECT call names its
//! callee — `Operand::Constant(MirConstant::Function(name))` — and
//! `types::named_function_return_kind` stamps the destination slot from the
//! callee's entry in the `harvest_return_abi` return table. An INDIRECT call
//! reads its callee out of a slot (`Operand::Copy(Place::Local(_))`) and has
//! no such lookup, so the destination stays unproven and
//! `call_return_abi_class` surfaces: there is no return-ABI class to select
//! and no sound way to bring the value back.
//!
//! Some of those indirect callees are nonetheless statically determined. This
//! module computes which, as a dataflow fact, so the existing forward
//! call-destination stamping can reach them. It is a producing-site
//! classification per ADR-006 §2.7.5 — no inference-policy change, no
//! backward propagation, no default.
//!
//! # Why this is forward, and why that matters
//!
//! §6.7 REFUTED propagating a proven DESTINATION kind backwards into the
//! SOURCE of a bare `Rvalue::Use` move. `Use` is the one `Rvalue` variant
//! that can change kind without a distinct spelling (`let v = fallible()?`
//! and `let v = await f()` both lower to a bare `Use` of a call
//! destination), so no rule keyed on that shape can tell an unwrap from a
//! move. Nothing here reads a destination kind. Every kind flows callee →
//! destination, which is the same direction the direct-call stamp already
//! flows.
//!
//! # The two resolution rules
//!
//! **R1 — intra-function.** A slot whose SOLE writer in this MIR function is
//! a closure or function constant with a known function id holds that
//! callable at every execution. Three spellings are recognised:
//!
//!  - `StatementKind::ClosureCapture { function_id: Some(fid), .. }` — a
//!    closure literal WITH captures;
//!  - `Assign(slot, Use(Constant(MirConstant::Function(name))))` — a closure
//!    literal WITHOUT captures, after `compiler/functions.rs` back-patches
//!    the placeholder into a named function constant (this is what most MIR
//!    reaching the JIT carries), and equally a reference to a named `fn`;
//!  - `Assign(slot, Use(Constant(ClosurePlaceholder)))` — the same
//!    capture-less literal when the back-patch did NOT run, whose fid comes
//!    from the `scan_closure_placeholder_fids` pairing the rest of the JIT
//!    already uses.
//!
//! Single-writer `Use` re-bindings (`let j = inc`) propagate the fact.
//!
//! Single-writer is checked over the whole function, not per block, so a
//! slot reassigned on one arm of a branch is never resolved.
//!
//! **R2 — capture-bound.** Captures are the closure body's LEADING
//! parameters (`compiler/function_abi.rs` asserts
//! `mir.param_slots.len() == arity` and that params `[0..captures_count)`
//! are the captures), so a callee read from `mir.param_slots[i]` is capture
//! `i`. What that capture holds is decided at the closure's construction
//! site, in another function's MIR.
//!
//! ## The correspondence problem, and why R2 does not depend on it
//!
//! `StatementKind::ClosureCapture`'s `operands` and `ClosureLayout`'s
//! capture indices are produced by two independent passes that each sort the
//! captured names alphabetically. They are *intended* to agree positionally
//! — `emit_heap_closure` already zips them, writing `operands[i]` at
//! `layout.heap_capture_offset(i)` — but nothing checks it beyond a count
//! comparison, and the two passes apply different filters to different
//! outer-scope sets.
//!
//! Rather than inherit that assumption, R2 only resolves when the
//! correspondence is FORCED by the layout's own kinds:
//!
//!  - exactly one capture index of the closure carries
//!    `NativeKind::Ptr(HeapKind::Closure)`, and
//!  - exactly one construction-site operand resolves under R1 to a function
//!    id,
//!
//! with the operand count equal to the capture count, so the operands really
//! are a permutation of the captures rather than a subset.
//!
//! Everything R1 resolves is callable-valued, and
//! `capture_native_kind::native_kind_from_concrete_type` maps BOTH
//! `ConcreteType::Closure(_)` and `ConcreteType::Function(_)` to
//! `Ptr(HeapKind::Closure)` — so every R1-resolvable operand is counted by
//! the first condition. A lone callable operand can therefore only be the
//! lone callable capture, whatever order the two producers emitted them in.
//! That is a stronger footing than the positional zip `emit_heap_closure`
//! already relies on. A closure capturing two closures is simply not
//! resolved.
//!
//! Two further conditions:
//!
//!  - the closure must have exactly ONE construction site in the whole
//!    program (`operands` at a second site could bind a different callee);
//!  - the capture's `CaptureKind` must be `Immutable`. `OwnedMutable` and
//!    `Shared` captures hold a *cell* whose contents can be rebound at
//!    runtime, so "the construction site decides the value" is false for
//!    them.
//!
//! # Failure mode if this were wrong
//!
//! A wrong stamp is a wrong `NativeKind` on the destination, which selects
//! the wrong return-ABI monomorph and routes `retain`/`release` through
//! `ownership.rs::retain_func_for_kind` for the wrong kind — silent heap
//! corruption, the §6.6 discriminant class. That is why every condition
//! above is a refusal rather than a preference: an unresolved callee keeps
//! the existing §4.1 surface-and-stop, which is a clean deopt.

use shape_value::heap_value::HeapKind;
use shape_value::v2::closure_layout::{CaptureKind, ClosureLayout};
use shape_vm::mir::types::{
    MirConstant, MirFunction, Operand, Place, Rvalue, SlotId, StatementKind,
};
use shape_vm::type_tracking::NativeKind;
use std::collections::HashMap;
use std::sync::Arc;

/// Which closure a given closure body's capture slot is known to hold.
///
/// Keyed by the closure body's own function id. The value names the capture
/// index and the function id of the closure bound there. At most one entry
/// per closure body — the resolution rule (see the module docs) only fires
/// when exactly one capture is closure-typed.
pub(crate) type ClosureCaptureCallees = HashMap<u16, (usize, u16)>;

/// R1: slots whose sole writer in `mir` is a closure construction, mapped to
/// the constructed closure's function id.
///
/// `placeholder_fids` is the output of
/// `mir_compiler::scan_closure_placeholder_fids` for this same MIR — the
/// capture-less construction spelling carries no id of its own, and this is
/// the pairing the rest of the JIT uses to recover it.
pub(crate) fn local_closure_fids(
    mir: &MirFunction,
    placeholder_fids: &[u16],
    function_indices: &HashMap<String, u16>,
) -> HashMap<SlotId, u16> {
    // Count every write to every slot first. A slot written twice is not a
    // dataflow fact about the callee no matter what the writes say.
    let mut writes: HashMap<SlotId, usize> = HashMap::new();
    for block in &mir.blocks {
        for stmt in &block.statements {
            match &stmt.kind {
                StatementKind::Assign(Place::Local(slot), _) => {
                    *writes.entry(*slot).or_default() += 1;
                }
                StatementKind::ClosureCapture { closure_slot, .. } => {
                    *writes.entry(*closure_slot).or_default() += 1;
                }
                _ => {}
            }
        }
        if let shape_vm::mir::types::TerminatorKind::Call {
            destination: Place::Local(slot),
            ..
        } = &block.terminator.kind
        {
            *writes.entry(*slot).or_default() += 1;
        }
    }

    // The capture-less construction spelling is
    // `Assign(slot, Use(Constant(ClosurePlaceholder)))`; its fid comes from
    // the placeholder scan, consumed in the same MIR traversal order the
    // scan produced. `u16::MAX` is the scan's "unresolved" sentinel and is
    // never a real function id.
    let mut placeholder_cursor = 0usize;
    let mut resolved: HashMap<SlotId, u16> = HashMap::new();
    for block in &mir.blocks {
        for stmt in &block.statements {
            match &stmt.kind {
                StatementKind::Assign(
                    Place::Local(slot),
                    Rvalue::Use(Operand::Constant(MirConstant::ClosurePlaceholder)),
                ) => {
                    let fid = placeholder_fids.get(placeholder_cursor).copied();
                    placeholder_cursor += 1;
                    if let Some(fid) = fid {
                        if fid != u16::MAX && writes.get(slot) == Some(&1) {
                            resolved.insert(*slot, fid);
                        }
                    }
                }
                // The capture-less closure spelling AFTER back-patching:
                // `compiler/functions.rs` rewrites the placeholder into a
                // named `MirConstant::Function` once the closure's index is
                // known, and that is what most MIR the JIT sees carries. The
                // placeholder arm above only fires when the back-patch did
                // not run (monomorphization clears `closure_function_ids`
                // before top-level patching).
                StatementKind::Assign(
                    Place::Local(slot),
                    Rvalue::Use(Operand::Constant(MirConstant::Function(name))),
                ) => {
                    if writes.get(slot) == Some(&1) {
                        if let Some(fid) = crate::mir_compiler::types::resolve_named_function_index(
                            name,
                            function_indices,
                        ) {
                            resolved.insert(*slot, fid);
                        }
                    }
                }
                StatementKind::ClosureCapture {
                    closure_slot,
                    function_id: Some(fid),
                    ..
                } => {
                    if writes.get(closure_slot) == Some(&1) {
                        resolved.insert(*closure_slot, *fid);
                    }
                }
                _ => {}
            }
        }
    }

    // Propagate through single-writer `Use` re-bindings (`let j = inc`) to a
    // fixpoint. The source must itself be resolved, so this only ever moves
    // an already-proven closure identity along a chain of copies.
    loop {
        let mut changed = false;
        for block in &mir.blocks {
            for stmt in &block.statements {
                let StatementKind::Assign(Place::Local(dst), Rvalue::Use(operand)) = &stmt.kind
                else {
                    continue;
                };
                let src = match operand {
                    Operand::Copy(Place::Local(s))
                    | Operand::Move(Place::Local(s))
                    | Operand::MoveExplicit(Place::Local(s)) => *s,
                    _ => continue,
                };
                if writes.get(dst) != Some(&1) {
                    continue;
                }
                let Some(fid) = resolved.get(&src).copied() else {
                    continue;
                };
                if resolved.insert(*dst, fid) != Some(fid) {
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }

    resolved
}

/// The lone capture index of `layout` that carries a closure, if there is
/// exactly one and it is an `Immutable` capture.
fn sole_immutable_closure_capture(layout: &ClosureLayout) -> Option<usize> {
    let mut found = None;
    for i in 0..layout.capture_count() {
        if layout.capture_native_kind(i) == NativeKind::Ptr(HeapKind::Closure) {
            if found.is_some() {
                return None;
            }
            found = Some(i);
        }
    }
    let i = found?;
    (layout.capture_storage_kind(i) == CaptureKind::Immutable).then_some(i)
}

/// R2: resolve, for each closure body in the program, which closure its lone
/// closure-typed capture is bound to.
///
/// `mirs` is every MIR function in the program (top-level included) paired
/// with the placeholder-fid scan for that same MIR. Scanning all of them is
/// what makes "exactly one construction site" checkable; a closure
/// constructed in two places is deliberately left unresolved.
pub(crate) fn resolve_closure_capture_callees(
    mirs: &[(&MirFunction, Vec<u16>)],
    closure_function_layouts: &HashMap<u16, Arc<ClosureLayout>>,
    function_indices: &HashMap<String, u16>,
) -> ClosureCaptureCallees {
    // Per-function R1 results, computed once and shared by both passes below.
    let per_function: Vec<HashMap<SlotId, u16>> = mirs
        .iter()
        .map(|(mir, placeholders)| local_closure_fids(mir, placeholders, function_indices))
        .collect();

    // Every construction site of every closure, with the operand list. A
    // closure appearing twice is poisoned by inserting `None`.
    let mut sites: HashMap<u16, Option<(usize, &[Operand])>> = HashMap::new();
    for (fn_idx, (mir, _)) in mirs.iter().enumerate() {
        for block in &mir.blocks {
            for stmt in &block.statements {
                let StatementKind::ClosureCapture {
                    operands,
                    function_id: Some(fid),
                    ..
                } = &stmt.kind
                else {
                    continue;
                };
                sites
                    .entry(*fid)
                    .and_modify(|e| *e = None)
                    .or_insert(Some((fn_idx, operands.as_slice())));
            }
        }
    }

    let mut out: ClosureCaptureCallees = HashMap::new();
    for (fid, site) in sites {
        let Some((fn_idx, operands)) = site else {
            continue;
        };
        let Some(layout) = closure_function_layouts.get(&fid) else {
            continue;
        };
        // The count check is what lets the kind argument treat `operands` as
        // a permutation of the captures. Without it a short operand list
        // could hide the second closure that would have made the mapping
        // ambiguous.
        if operands.len() != layout.capture_count() {
            continue;
        }
        let Some(capture_idx) = sole_immutable_closure_capture(layout) else {
            continue;
        };
        // Exactly one operand must resolve to a closure. Combined with
        // "exactly one capture is closure-typed", the two must correspond —
        // no positional assumption needed.
        let resolved = &per_function[fn_idx];
        let mut callee = None;
        for operand in operands {
            let slot = match operand {
                Operand::Copy(Place::Local(s))
                | Operand::Move(Place::Local(s))
                | Operand::MoveExplicit(Place::Local(s)) => *s,
                _ => continue,
            };
            let Some(fid) = resolved.get(&slot).copied() else {
                continue;
            };
            if callee.is_some() {
                callee = None;
                break;
            }
            callee = Some(fid);
        }
        if let Some(callee_fid) = callee {
            out.insert(fid, (capture_idx, callee_fid));
        }
    }
    out
}

/// Build the capture-callee map for a whole program.
///
/// Collects every MIR function the program carries — `top_level_mir` plus
/// each `functions[i].mir_data`, which includes every `__closure_N` body —
/// and runs [`resolve_closure_capture_callees`] over the lot. Seeing all of
/// them is what makes "exactly one construction site" a checkable claim
/// rather than an assumption about the function currently being compiled.
///
/// Called from the two JIT orchestration paths that hold a `&BytecodeProgram`
/// (`compiler/program.rs` per-function, `compiler/strategy.rs` top-level),
/// alongside the existing `closure_function_layouts` construction.
pub(crate) fn resolve_program_closure_capture_callees(
    program: &shape_vm::bytecode::BytecodeProgram,
    closure_function_layouts: &HashMap<u16, Arc<ClosureLayout>>,
) -> ClosureCaptureCallees {
    let mirs: Vec<&MirFunction> = program
        .top_level_mir
        .as_ref()
        .map(|d| &d.mir)
        .into_iter()
        .chain(
            program
                .functions
                .iter()
                .filter_map(|f| f.mir_data.as_ref().map(|d| &d.mir)),
        )
        .collect();
    let function_indices: HashMap<String, u16> = program
        .functions
        .iter()
        .enumerate()
        .map(|(i, f)| (f.name.clone(), i as u16))
        .collect();
    let with_placeholders: Vec<(&MirFunction, Vec<u16>)> = mirs
        .into_iter()
        .map(|mir| {
            let fids = super::scan_closure_placeholder_fids(mir, &function_indices);
            (mir, fids)
        })
        .collect();
    resolve_closure_capture_callees(
        &with_placeholders,
        closure_function_layouts,
        &function_indices,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_ast::Span;
    use shape_value::v2::ConcreteType;
    use shape_vm::mir::types::{
        BasicBlock, BasicBlockId, MirStatement, Terminator, TerminatorKind,
    };

    fn mir(name: &str, stmts: Vec<MirStatement>, params: Vec<SlotId>) -> MirFunction {
        MirFunction {
            name: name.to_string(),
            blocks: vec![BasicBlock {
                id: BasicBlockId(0),
                statements: stmts,
                terminator: Terminator {
                    kind: TerminatorKind::Return,
                    span: Span::default(),
                },
            }],
            num_locals: 8,
            param_slots: params,
            param_reference_kinds: vec![],
            local_types: vec![],
            span: Span::default(),
            field_name_table: Default::default(),
            local_struct_type_names: Default::default(),
            local_typed_array_element_types: Default::default(),
            local_declared_scalar_types: Default::default(),
            binding_slots: Default::default(),
            local_names: Vec::new(),
            var_binding_slots: Default::default(),
        }
    }

    fn stmt(kind: StatementKind) -> MirStatement {
        MirStatement {
            kind,
            span: Span::default(),
            point: shape_vm::mir::types::Point(0),
        }
    }

    fn assign_fn_const(slot: u16, name: &str) -> MirStatement {
        stmt(StatementKind::Assign(
            Place::Local(SlotId(slot)),
            Rvalue::Use(Operand::Constant(MirConstant::Function(name.to_string()))),
        ))
    }

    fn capture(closure_slot: u16, fid: u16, operands: Vec<u16>) -> MirStatement {
        stmt(StatementKind::ClosureCapture {
            closure_slot: SlotId(closure_slot),
            operands: operands
                .into_iter()
                .map(|s| Operand::Copy(Place::Local(SlotId(s))))
                .collect(),
            function_id: Some(fid),
        })
    }

    fn layout(caps: &[ConcreteType]) -> Arc<ClosureLayout> {
        let kinds = vec![CaptureKind::Immutable; caps.len()];
        Arc::new(ClosureLayout::from_capture_types(caps, &kinds))
    }

    fn indices(pairs: &[(&str, u16)]) -> HashMap<String, u16> {
        pairs.iter().map(|(n, i)| (n.to_string(), *i)).collect()
    }

    /// `let g = |x| ...` (capture-less, back-patched to a function constant)
    /// then `let h = |y| g(y)`: `h`'s only capture is `g`, so calling that
    /// capture is a call to `g`.
    #[test]
    fn sole_closure_capture_resolves_to_its_construction_site_operand() {
        let top = mir(
            "__main__",
            vec![assign_fn_const(1, "__closure_0"), capture(2, 11, vec![1])],
            vec![],
        );
        let idx = indices(&[("__closure_0", 10), ("__closure_1", 11)]);
        let layouts: HashMap<u16, Arc<ClosureLayout>> = [(
            11u16,
            layout(&[ConcreteType::Closure(
                shape_value::v2::concrete_type::ClosureTypeId(0),
            )]),
        )]
        .into_iter()
        .collect();
        let out = resolve_closure_capture_callees(&[(&top, vec![])], &layouts, &idx);
        assert_eq!(
            out.get(&11),
            Some(&(0usize, 10u16)),
            "capture 0 of __closure_1 is bound to __closure_0"
        );
    }

    /// The whole point of scanning every MIR function: a closure built in two
    /// places can have a different callable at each, so neither is a fact.
    #[test]
    fn two_construction_sites_refuse_to_resolve() {
        let a = mir(
            "__main__",
            vec![assign_fn_const(1, "__closure_0"), capture(2, 11, vec![1])],
            vec![],
        );
        let b = mir(
            "other",
            vec![assign_fn_const(1, "__closure_2"), capture(2, 11, vec![1])],
            vec![],
        );
        let idx = indices(&[
            ("__closure_0", 10),
            ("__closure_1", 11),
            ("__closure_2", 12),
        ]);
        let layouts: HashMap<u16, Arc<ClosureLayout>> = [(
            11u16,
            layout(&[ConcreteType::Closure(
                shape_value::v2::concrete_type::ClosureTypeId(0),
            )]),
        )]
        .into_iter()
        .collect();
        let out = resolve_closure_capture_callees(&[(&a, vec![]), (&b, vec![])], &layouts, &idx);
        assert!(
            out.get(&11).is_none(),
            "a closure constructed twice must not resolve"
        );
    }

    /// The condition that removes the dependency on operand↔capture-index
    /// ordering: with two callable captures there is no kind that forces the
    /// correspondence, so nothing is resolved.
    #[test]
    fn two_closure_captures_refuse_to_resolve() {
        let top = mir(
            "__main__",
            vec![
                assign_fn_const(1, "__closure_0"),
                assign_fn_const(2, "__closure_2"),
                capture(3, 11, vec![1, 2]),
            ],
            vec![],
        );
        let idx = indices(&[
            ("__closure_0", 10),
            ("__closure_1", 11),
            ("__closure_2", 12),
        ]);
        let layouts: HashMap<u16, Arc<ClosureLayout>> = [(
            11u16,
            layout(&[
                ConcreteType::Closure(shape_value::v2::concrete_type::ClosureTypeId(0)),
                ConcreteType::Closure(shape_value::v2::concrete_type::ClosureTypeId(0)),
            ]),
        )]
        .into_iter()
        .collect();
        let out = resolve_closure_capture_callees(&[(&top, vec![])], &layouts, &idx);
        assert!(
            out.get(&11).is_none(),
            "two callable captures leave the correspondence unforced"
        );
    }

    /// A non-callable capture beside the callable one does not block
    /// resolution — the kinds still single out one index.
    #[test]
    fn scalar_capture_beside_the_closure_still_resolves() {
        let top = mir(
            "__main__",
            vec![
                assign_fn_const(1, "__closure_0"),
                capture(3, 11, vec![2, 1]),
            ],
            vec![],
        );
        let idx = indices(&[("__closure_0", 10), ("__closure_1", 11)]);
        let layouts: HashMap<u16, Arc<ClosureLayout>> = [(
            11u16,
            layout(&[
                ConcreteType::I64,
                ConcreteType::Closure(shape_value::v2::concrete_type::ClosureTypeId(0)),
            ]),
        )]
        .into_iter()
        .collect();
        let out = resolve_closure_capture_callees(&[(&top, vec![])], &layouts, &idx);
        assert_eq!(out.get(&11), Some(&(1usize, 10u16)));
    }

    /// A `var`/`let mut` capture holds a CELL whose contents can be rebound
    /// after construction, so the construction site does not decide it.
    #[test]
    fn mutable_closure_capture_refuses_to_resolve() {
        let top = mir(
            "__main__",
            vec![assign_fn_const(1, "__closure_0"), capture(2, 11, vec![1])],
            vec![],
        );
        let idx = indices(&[("__closure_0", 10), ("__closure_1", 11)]);
        let shared = Arc::new(ClosureLayout::from_capture_types(
            &[ConcreteType::Closure(
                shape_value::v2::concrete_type::ClosureTypeId(0),
            )],
            &[CaptureKind::Shared],
        ));
        let layouts: HashMap<u16, Arc<ClosureLayout>> = [(11u16, shared)].into_iter().collect();
        let out = resolve_closure_capture_callees(&[(&top, vec![])], &layouts, &idx);
        assert!(
            out.get(&11).is_none(),
            "a Shared capture is a rebindable cell, not a fixed callable"
        );
    }

    /// R1 is single-writer over the WHOLE function, not per block: a callee
    /// slot reassigned anywhere is not a dataflow fact.
    #[test]
    fn reassigned_callee_slot_is_not_single_writer() {
        let top = mir(
            "__main__",
            vec![
                assign_fn_const(1, "__closure_0"),
                assign_fn_const(1, "__closure_2"),
            ],
            vec![],
        );
        let idx = indices(&[("__closure_0", 10), ("__closure_2", 12)]);
        let out = local_closure_fids(&top, &[], &idx);
        assert!(
            out.get(&SlotId(1)).is_none(),
            "two writers means no fact about what the slot holds"
        );
    }

    /// `let j = inc` carries the identity along; the re-binding must itself
    /// be single-writer.
    #[test]
    fn single_writer_use_rebinding_propagates() {
        let top = mir(
            "__main__",
            vec![
                assign_fn_const(1, "__closure_0"),
                stmt(StatementKind::Assign(
                    Place::Local(SlotId(2)),
                    Rvalue::Use(Operand::Copy(Place::Local(SlotId(1)))),
                )),
            ],
            vec![],
        );
        let idx = indices(&[("__closure_0", 10)]);
        let out = local_closure_fids(&top, &[], &idx);
        assert_eq!(out.get(&SlotId(2)), Some(&10u16));
    }
}
