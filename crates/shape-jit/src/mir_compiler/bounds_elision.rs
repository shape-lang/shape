//! MIR-level bounds-check elision analysis.
//!
//! This module identifies `Place::Index(Local(arr), Operand::*(Local(iv)))`
//! sites where the bounds check inside `inline_array_get`/`inline_array_set`
//! is provably redundant.
//!
//! # Soundness Argument
//!
//! For an indexed access `arr[iv]` to be safe without a runtime bounds
//! check we must show that at the access point both:
//!   1. `iv >= 0` (a non-negative index)
//!   2. `iv < arr.length`
//!
//! The MIR pattern we look for is the standard `for i in 0..n` /
//! `while i < n` shape:
//!
//! ```text
//!   bb_pre:
//!     iv  = Use(Constant(Int(0)))                  // (1) iv starts at 0
//!     bnd = arr.length                              // (2) bound = arr.length
//!     ... goto bb_header ...
//!
//!   bb_header:
//!     cond = iv < bnd                               // (3) loop test
//!     SwitchBool(cond, bb_body, bb_after)
//!
//!   bb_body:
//!     ... use arr[iv] ...                           // (4) elidable access
//!     iv = iv + 1                                   // (5) non-negative step
//!     goto bb_header
//! ```
//!
//! When all five conditions hold, every dynamic access of `arr[iv]` inside
//! `bb_body` is sound without a bounds check:
//!   - From (1) and (5) by induction: `iv >= 0` at every iteration.
//!   - From (3): `iv < bnd`, and since (2) bnd was set from `arr.length`,
//!     `iv < arr.length` (the array can only grow inside the body, never
//!     shrink — the only inline opcodes that grow it are pushes, but those
//!     would invalidate the data pointer; we conservatively also require
//!     that the array slot is not reassigned in the body).
//!
//! The `arr` slot must also not be reassigned between the `bnd = arr.length`
//! statement and the access. We enforce this by checking that `arr` is not
//! the LHS of any `Assign(Local(arr), _)` between the bound capture and the
//! header, nor inside the body.
//!
//! This matches the conservative subset of `optimizer/bounds.rs::analyze_bounds`
//! that the bytecode-side analyzer implements; here we run directly on MIR
//! so the slot-numbering convention is irrelevant.

use std::collections::{HashMap, HashSet};

use shape_vm::mir::types::{
    BasicBlockId, BinOp, MirFunction, Operand, Place, Rvalue, SlotId, StatementKind,
    TerminatorKind,
};

/// Result of bounds-elision analysis: per-MIR-function set of `(arr_slot, iv_slot)`
/// pairs where `arr[iv]` accesses inside the corresponding loop are trusted.
#[derive(Debug, Clone, Default)]
pub struct BoundsElisionPlan {
    /// `(arr_slot, iv_slot)` pairs that may bypass the inline bounds check.
    pub trusted_pairs: HashSet<(SlotId, SlotId)>,
}

impl BoundsElisionPlan {
    pub fn is_trusted(&self, arr: SlotId, iv: SlotId) -> bool {
        self.trusted_pairs.contains(&(arr, iv))
    }
}

/// Analyze a MIR function and return a `BoundsElisionPlan` listing trusted
/// (arr, iv) slot pairs.
pub fn analyze(mir: &MirFunction) -> BoundsElisionPlan {
    let mut plan = BoundsElisionPlan::default();

    // Build CFG predecessors so we can recognise loop headers (a header is
    // a block that has a back-edge predecessor: a predecessor whose
    // BasicBlockId is >= the header's id).
    let mut preds: HashMap<BasicBlockId, Vec<BasicBlockId>> = HashMap::new();
    for block in &mir.blocks {
        for succ in successors(&block.terminator.kind) {
            preds.entry(succ).or_default().push(block.id);
        }
    }

    // Resolve "length" FieldIdx — bound captures look like
    // `Place::Field(arr, length_idx)`. We accept the field by name table
    // lookup so this remains schema-agnostic.
    let length_field_idxs: HashSet<u16> = mir
        .field_name_table
        .iter()
        .filter_map(|(idx, name)| if name == "length" { Some(idx.0) } else { None })
        .collect();

    if length_field_idxs.is_empty() {
        // Without a known "length" field idx we cannot prove the bound came
        // from the array.
        return plan;
    }

    // Identify candidate loop headers: blocks whose terminator is a
    // SwitchBool whose predicate matches `iv < bnd`. For each such block
    // we want to verify that:
    //   (a) bnd was last assigned `arr.length` on a path from entry to
    //       the header, with no subsequent reassignment of arr, bnd, or iv
    //       to non-monotone values.
    //   (b) iv was initialized to a non-negative integer constant before
    //       the header, with no negative reassignment between init and the
    //       loop body.
    //   (c) the body block contains accesses `arr[iv]` and the only
    //       reassignment to iv inside the body is `iv = iv + <constant>`
    //       with non-negative constant, AND arr is not reassigned in the
    //       body, AND bnd is not reassigned in the body.
    //
    // We do all of this in a single pass per candidate header.
    for header in &mir.blocks {
        let TerminatorKind::SwitchBool {
            operand: pred_op,
            true_bb,
            false_bb: _false_bb,
        } = &header.terminator.kind
        else {
            continue;
        };

        // Predicate must be `Local(cond)` produced inside the same header
        // by `Assign(Local(cond), BinaryOp(Lt, Copy/Move(Local(iv)),
        // Copy/Move(Local(bnd))))`.
        let Some(cond_slot) = operand_local(pred_op) else {
            continue;
        };
        let Some((iv, bnd)) = find_lt_definition(header, cond_slot) else {
            continue;
        };

        // Verify the header has at least one back-edge predecessor (to be
        // a real loop header).
        let header_preds = preds.get(&header.id).cloned().unwrap_or_default();
        let has_back_edge = header_preds.iter().any(|p| p.0 >= header.id.0);
        if !has_back_edge {
            // Plain `if` rather than a loop. We still allow elision in this
            // case if the body's accesses are a single straight-line path
            // — but conservatively skip for now.
            continue;
        }

        // Body block. We only inspect the `true` successor for accesses.
        let Some(body) = mir.blocks.iter().find(|b| b.id == *true_bb) else {
            continue;
        };

        // Check that bnd was assigned `arr.length` somewhere reachable
        // before the header and not reassigned in the body. We scan all
        // statements globally and collect every `Assign(Local(bnd),
        // Use(Copy/Move(Place::Field(Place::Local(arr), len_idx))))`.
        let bnd_array_sources: Vec<SlotId> = mir
            .blocks
            .iter()
            .flat_map(|b| b.statements.iter())
            .filter_map(|stmt| {
                let StatementKind::Assign(Place::Local(lhs), rvalue) = &stmt.kind else {
                    return None;
                };
                if *lhs != bnd {
                    return None;
                }
                let arr_slot = rvalue_field_length_source(rvalue, &length_field_idxs)?;
                Some(arr_slot)
            })
            .collect();

        if bnd_array_sources.is_empty() {
            continue;
        }

        // bnd must have a SINGLE consistent array source. If the analyser
        // sees multiple distinct arrays writing to bnd, conservatively skip.
        let arr = bnd_array_sources[0];
        if bnd_array_sources.iter().any(|s| *s != arr) {
            continue;
        }

        // The array slot must be stable across the function: either it is
        // a parameter (no Assign at all is acceptable) with zero
        // reassignments, or a local with at most one initial Assign and no
        // subsequent reassignment. Either way, the maximum allowed count
        // depends on whether the slot is a param.
        let is_param = mir.param_slots.contains(&arr);
        let max_assigns = if is_param { 0 } else { 1 };
        if slot_assignment_count(mir, arr) > max_assigns {
            continue;
        }

        // bnd must not be reassigned inside the body (so the runtime
        // length check that already executes on the loop edge stays valid).
        if block_assigns(body, bnd) {
            continue;
        }

        // iv must be initialized to `0` before the header and only
        // incremented inside the body by a non-negative constant.
        if !iv_starts_non_negative(mir, iv, header.id) {
            continue;
        }
        if !iv_only_monotonic_in_body(body, iv) {
            continue;
        }

        // All conditions hold. Mark `(arr, iv)` as trusted.
        plan.trusted_pairs.insert((arr, iv));
    }

    plan
}

fn successors(term: &TerminatorKind) -> Vec<BasicBlockId> {
    match term {
        TerminatorKind::Goto(b) => vec![*b],
        TerminatorKind::SwitchBool { true_bb, false_bb, .. } => vec![*true_bb, *false_bb],
        TerminatorKind::Call { next, .. } => vec![*next],
        TerminatorKind::Return | TerminatorKind::Unreachable => vec![],
    }
}

fn operand_local(op: &Operand) -> Option<SlotId> {
    match op {
        Operand::Copy(Place::Local(s))
        | Operand::Move(Place::Local(s))
        | Operand::MoveExplicit(Place::Local(s)) => Some(*s),
        _ => None,
    }
}

fn rvalue_field_length_source(
    rvalue: &Rvalue,
    length_field_idxs: &HashSet<u16>,
) -> Option<SlotId> {
    let inner_op = match rvalue {
        Rvalue::Use(op) | Rvalue::Clone(op) => op,
        _ => return None,
    };
    let place = match inner_op {
        Operand::Copy(p) | Operand::Move(p) | Operand::MoveExplicit(p) => p,
        _ => return None,
    };
    let Place::Field(base, field_idx) = place else {
        return None;
    };
    if !length_field_idxs.contains(&field_idx.0) {
        return None;
    }
    if let Place::Local(arr_slot) = base.as_ref() {
        Some(*arr_slot)
    } else {
        None
    }
}

/// Find a statement in `block` of the form
/// `Assign(Local(cond), BinaryOp(Lt, Copy/Move(Local(iv)), Copy/Move(Local(bnd))))`
/// and return `(iv, bnd)`.
fn find_lt_definition(
    block: &shape_vm::mir::types::BasicBlock,
    cond_slot: SlotId,
) -> Option<(SlotId, SlotId)> {
    for stmt in &block.statements {
        let StatementKind::Assign(Place::Local(lhs), Rvalue::BinaryOp(BinOp::Lt, l, r)) =
            &stmt.kind
        else {
            continue;
        };
        if *lhs != cond_slot {
            continue;
        }
        let iv = operand_local(l)?;
        let bnd = operand_local(r)?;
        return Some((iv, bnd));
    }
    None
}

/// Count the number of `Assign(Local(slot), _)` statements in the function.
fn slot_assignment_count(mir: &MirFunction, slot: SlotId) -> usize {
    let mut count = 0usize;
    for block in &mir.blocks {
        for stmt in &block.statements {
            if let StatementKind::Assign(Place::Local(s), _) = &stmt.kind {
                if *s == slot {
                    count += 1;
                }
            }
        }
    }
    count
}

fn block_assigns(block: &shape_vm::mir::types::BasicBlock, slot: SlotId) -> bool {
    for stmt in &block.statements {
        if let StatementKind::Assign(Place::Local(s), _) = &stmt.kind {
            if *s == slot {
                return true;
            }
        }
    }
    false
}

/// True iff `iv` was assigned `Constant(Int(c))` with `c >= 0` before
/// reaching `header`, and not reassigned to a negative constant in between.
fn iv_starts_non_negative(mir: &MirFunction, iv: SlotId, header: BasicBlockId) -> bool {
    // Find the most recent Assign to iv on any block reaching the header
    // (we use a simple block-id-ordering heuristic: scan all blocks with
    // id < header.id, take the latest assignment).
    let mut last_const_init: Option<i64> = None;
    for block in &mir.blocks {
        if block.id.0 >= header.0 {
            continue;
        }
        for stmt in &block.statements {
            let StatementKind::Assign(Place::Local(lhs), rv) = &stmt.kind else {
                continue;
            };
            if *lhs != iv {
                continue;
            }
            // We only accept const Int initializers; anything else is
            // potentially negative.
            let value = match rv {
                Rvalue::Use(Operand::Constant(shape_vm::mir::types::MirConstant::Int(v))) => Some(*v),
                _ => None,
            };
            last_const_init = value;
        }
    }
    matches!(last_const_init, Some(v) if v >= 0)
}

/// True iff every Assign to `iv` in `body` has the shape
/// `Assign(Local(iv), BinaryOp(Add, Copy/Move(Local(iv)), Constant(Int(c))))`
/// with `c >= 0`.
fn iv_only_monotonic_in_body(body: &shape_vm::mir::types::BasicBlock, iv: SlotId) -> bool {
    for stmt in &body.statements {
        let StatementKind::Assign(Place::Local(lhs), rv) = &stmt.kind else {
            continue;
        };
        if *lhs != iv {
            continue;
        }
        let Rvalue::BinaryOp(BinOp::Add, l, r) = rv else {
            return false;
        };
        let l_is_iv = operand_local(l) == Some(iv);
        let r_is_iv = operand_local(r) == Some(iv);
        let const_step = match (l, r) {
            (_, Operand::Constant(shape_vm::mir::types::MirConstant::Int(v))) => Some(*v),
            (Operand::Constant(shape_vm::mir::types::MirConstant::Int(v)), _) => Some(*v),
            _ => None,
        };
        let Some(step) = const_step else {
            return false;
        };
        if !(l_is_iv || r_is_iv) {
            return false;
        }
        if step < 0 {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_vm::mir::types::{
        BasicBlock, BasicBlockId, BinOp, FieldIdx, LocalTypeInfo, MirConstant, MirFunction,
        MirStatement, Place, Point, Rvalue, SlotId, StatementKind, Terminator, TerminatorKind,
    };
    use shape_ast::ast::Span;

    fn s(kind: StatementKind) -> MirStatement {
        MirStatement {
            kind,
            span: Span { start: 0, end: 0 },
            point: Point(0),
        }
    }

    fn term(kind: TerminatorKind) -> Terminator {
        Terminator {
            kind,
            span: Span { start: 0, end: 0 },
        }
    }

    fn mir_for_loop_with_arr_index(arr: SlotId, iv: SlotId, bnd: SlotId, cond: SlotId) -> MirFunction {
        // bb0: arr param implicitly held in slot `arr`
        //   bnd = arr.length
        //   iv  = 0
        //   goto bb1
        // bb1: cond = iv < bnd
        //      SwitchBool(cond, bb2, bb3)
        // bb2: arr[iv] (no MIR statement in pure read; we encode as a
        //      `Use(Copy(Place::Index(Local(arr), Copy(Local(iv)))))` assign
        //      into a sink local).
        //      iv = iv + 1
        //      goto bb1
        // bb3: return
        let length_idx = FieldIdx(7);
        let mut field_name_table = std::collections::HashMap::new();
        field_name_table.insert(length_idx, "length".to_string());

        let bb0 = BasicBlock {
            id: BasicBlockId(0),
            statements: vec![
                s(StatementKind::Assign(
                    Place::Local(bnd),
                    Rvalue::Use(Operand::Copy(Place::Field(
                        Box::new(Place::Local(arr)),
                        length_idx,
                    ))),
                )),
                s(StatementKind::Assign(
                    Place::Local(iv),
                    Rvalue::Use(Operand::Constant(MirConstant::Int(0))),
                )),
            ],
            terminator: term(TerminatorKind::Goto(BasicBlockId(1))),
        };
        let bb1 = BasicBlock {
            id: BasicBlockId(1),
            statements: vec![s(StatementKind::Assign(
                Place::Local(cond),
                Rvalue::BinaryOp(
                    BinOp::Lt,
                    Operand::Copy(Place::Local(iv)),
                    Operand::Copy(Place::Local(bnd)),
                ),
            ))],
            terminator: term(TerminatorKind::SwitchBool {
                operand: Operand::Copy(Place::Local(cond)),
                true_bb: BasicBlockId(2),
                false_bb: BasicBlockId(3),
            }),
        };
        let sink = SlotId(99);
        let bb2 = BasicBlock {
            id: BasicBlockId(2),
            statements: vec![
                s(StatementKind::Assign(
                    Place::Local(sink),
                    Rvalue::Use(Operand::Copy(Place::Index(
                        Box::new(Place::Local(arr)),
                        Box::new(Operand::Copy(Place::Local(iv))),
                    ))),
                )),
                s(StatementKind::Assign(
                    Place::Local(iv),
                    Rvalue::BinaryOp(
                        BinOp::Add,
                        Operand::Copy(Place::Local(iv)),
                        Operand::Constant(MirConstant::Int(1)),
                    ),
                )),
            ],
            terminator: term(TerminatorKind::Goto(BasicBlockId(1))),
        };
        let bb3 = BasicBlock {
            id: BasicBlockId(3),
            statements: vec![],
            terminator: term(TerminatorKind::Return),
        };

        MirFunction {
            name: "test_fn".to_string(),
            blocks: vec![bb0, bb1, bb2, bb3],
            num_locals: 100,
            param_slots: vec![arr],
            param_reference_kinds: vec![None],
            local_types: (0..100).map(|_| LocalTypeInfo::Unknown).collect(),
            span: Span { start: 0, end: 0 },
            field_name_table,
        }
    }

    #[test]
    fn detects_simple_for_loop_index_pattern() {
        let arr = SlotId(1);
        let iv = SlotId(2);
        let bnd = SlotId(3);
        let cond = SlotId(4);

        let mir = mir_for_loop_with_arr_index(arr, iv, bnd, cond);
        let plan = analyze(&mir);
        assert!(
            plan.is_trusted(arr, iv),
            "expected (arr={:?}, iv={:?}) to be trusted; got {:?}",
            arr,
            iv,
            plan.trusted_pairs,
        );
    }

    #[test]
    fn rejects_when_arr_is_reassigned() {
        let arr = SlotId(1);
        let iv = SlotId(2);
        let bnd = SlotId(3);
        let cond = SlotId(4);

        let mut mir = mir_for_loop_with_arr_index(arr, iv, bnd, cond);
        // Inject a second Assign to arr in bb0.
        mir.blocks[0].statements.push(s(StatementKind::Assign(
            Place::Local(arr),
            Rvalue::Use(Operand::Constant(MirConstant::None)),
        )));
        let plan = analyze(&mir);
        assert!(
            !plan.is_trusted(arr, iv),
            "expected access not to be trusted when arr is reassigned",
        );
    }

    #[test]
    fn rejects_when_iv_is_negative_initialized() {
        let arr = SlotId(1);
        let iv = SlotId(2);
        let bnd = SlotId(3);
        let cond = SlotId(4);

        let mut mir = mir_for_loop_with_arr_index(arr, iv, bnd, cond);
        // Replace the iv initialization with -1.
        mir.blocks[0].statements[1] = s(StatementKind::Assign(
            Place::Local(iv),
            Rvalue::Use(Operand::Constant(MirConstant::Int(-1))),
        ));
        let plan = analyze(&mir);
        assert!(
            !plan.is_trusted(arr, iv),
            "expected access not to be trusted when iv starts negative",
        );
    }

    #[test]
    fn rejects_when_no_back_edge() {
        let arr = SlotId(1);
        let iv = SlotId(2);
        let bnd = SlotId(3);
        let cond = SlotId(4);

        let mut mir = mir_for_loop_with_arr_index(arr, iv, bnd, cond);
        // Strip the back-edge: bb2 returns instead of jumping to bb1.
        let bb2 = mir.blocks.iter_mut().find(|b| b.id == BasicBlockId(2)).unwrap();
        bb2.terminator = term(TerminatorKind::Return);
        let plan = analyze(&mir);
        assert!(
            !plan.is_trusted(arr, iv),
            "expected access not to be trusted without a back edge",
        );
    }
}
