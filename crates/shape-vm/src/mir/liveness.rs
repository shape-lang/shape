//! Liveness analysis on MIR.
//!
//! Determines which variables are live (will be used later) at each program point.
//! This is the foundation for smart move/clone inference:
//! - If a variable is NOT live after an assignment, it can be moved (zero cost).
//! - If a variable IS live after an assignment, it must be cloned (requires Clone).

use super::cfg::ControlFlowGraph;
use super::types::*;
use std::collections::{HashMap, HashSet};

/// Result of liveness analysis for a MIR function.
#[derive(Debug)]
pub struct LivenessResult {
    /// Variables live at the entry of each block.
    pub live_in: HashMap<BasicBlockId, HashSet<SlotId>>,
    /// Variables live at the exit of each block.
    pub live_out: HashMap<BasicBlockId, HashSet<SlotId>>,
}

impl LivenessResult {
    /// Check if a variable is live at a given point within a block.
    /// Walks backwards from the block exit to the statement index.
    pub fn is_live_after(
        &self,
        block: BasicBlockId,
        stmt_idx: usize,
        slot: SlotId,
        mir: &MirFunction,
    ) -> bool {
        let bb = mir.block(block);

        // Start with live_out for the block
        let mut live = self.live_out.get(&block).cloned().unwrap_or_default();

        // Walk backwards from the end of the block to stmt_idx + 1
        // (we want liveness AFTER stmt_idx, so we stop before processing stmt_idx)
        for i in (stmt_idx + 1..bb.statements.len()).rev() {
            let stmt = &bb.statements[i];
            update_liveness_for_statement(&mut live, &stmt.kind);
        }

        // Also account for the terminator's uses
        add_terminator_uses(&mut live, &bb.terminator.kind);

        live.contains(&slot)
    }

    /// Check if a variable is live at the entry of a block.
    pub fn is_live_at_entry(&self, block: BasicBlockId, slot: SlotId) -> bool {
        self.live_in
            .get(&block)
            .map_or(false, |set| set.contains(&slot))
    }
}

/// Run liveness analysis on a MIR function.
/// Uses the standard backward dataflow algorithm:
///   live_out[B] = ∪ live_in[S] for all successors S of B
///   live_in[B] = (live_out[B] - def[B]) ∪ use[B]
pub fn compute_liveness(mir: &MirFunction, cfg: &ControlFlowGraph) -> LivenessResult {
    let mut live_in: HashMap<BasicBlockId, HashSet<SlotId>> = HashMap::new();
    let mut live_out: HashMap<BasicBlockId, HashSet<SlotId>> = HashMap::new();

    // Initialize all blocks with empty sets
    for block in &mir.blocks {
        live_in.insert(block.id, HashSet::new());
        live_out.insert(block.id, HashSet::new());
    }

    // Iterate until fixpoint
    let mut changed = true;
    while changed {
        changed = false;

        // Process blocks in reverse postorder (for backward analysis,
        // processing in reverse of the forward order is efficient)
        let rpo = cfg.reverse_postorder();
        for &block_id in rpo.iter().rev() {
            let block = mir.block(block_id);

            // live_out[B] = ∪ live_in[S] for successors S
            let mut new_live_out = HashSet::new();
            for &succ in cfg.successors(block_id) {
                if let Some(succ_in) = live_in.get(&succ) {
                    new_live_out.extend(succ_in);
                }
            }

            // Compute live_in from live_out
            let mut new_live_in = new_live_out.clone();

            // Process terminator (uses)
            add_terminator_uses(&mut new_live_in, &block.terminator.kind);

            // Process statements in reverse order
            for stmt in block.statements.iter().rev() {
                update_liveness_for_statement(&mut new_live_in, &stmt.kind);
            }

            // Check for changes
            if new_live_in != *live_in.get(&block_id).unwrap_or(&HashSet::new()) {
                changed = true;
                live_in.insert(block_id, new_live_in);
            }
            if new_live_out != *live_out.get(&block_id).unwrap_or(&HashSet::new()) {
                changed = true;
                live_out.insert(block_id, new_live_out);
            }
        }
    }

    LivenessResult { live_in, live_out }
}

/// Update liveness for a single statement (backward: remove defs, add uses).
fn update_liveness_for_statement(live: &mut HashSet<SlotId>, kind: &StatementKind) {
    match kind {
        StatementKind::Assign(place, rvalue) => {
            // Definition: remove the assigned-to slot
            if let Place::Local(slot) = place {
                live.remove(slot);
            }
            // Uses: add all used slots
            add_rvalue_uses(live, rvalue);
        }
        StatementKind::Drop(place) => {
            // Drop uses the place
            live.insert(place.root_local());
        }
        StatementKind::TaskBoundary(operands) => {
            for operand in operands {
                add_operand_uses(live, operand);
            }
        }
        StatementKind::ClosureCapture(operands) => {
            for operand in operands {
                add_operand_uses(live, operand);
            }
        }
        StatementKind::ArrayStore(operands) => {
            for operand in operands {
                add_operand_uses(live, operand);
            }
        }
        StatementKind::Nop => {}
    }
}

/// Add uses from an rvalue to the live set.
fn add_rvalue_uses(live: &mut HashSet<SlotId>, rvalue: &Rvalue) {
    match rvalue {
        Rvalue::Use(op) | Rvalue::Clone(op) | Rvalue::UnaryOp(_, op) => {
            add_operand_uses(live, op);
        }
        Rvalue::Borrow(_, place) => {
            live.insert(place.root_local());
        }
        Rvalue::BinaryOp(_, lhs, rhs) => {
            add_operand_uses(live, lhs);
            add_operand_uses(live, rhs);
        }
        Rvalue::Aggregate(ops) => {
            for op in ops {
                add_operand_uses(live, op);
            }
        }
    }
}

/// Add uses from an operand to the live set.
fn add_operand_uses(live: &mut HashSet<SlotId>, op: &Operand) {
    match op {
        Operand::Copy(place) | Operand::Move(place) | Operand::MoveExplicit(place) => {
            live.insert(place.root_local());
        }
        Operand::Constant(_) => {}
    }
}

/// Add uses from a terminator to the live set.
fn add_terminator_uses(live: &mut HashSet<SlotId>, kind: &TerminatorKind) {
    match kind {
        TerminatorKind::SwitchBool { operand, .. } => {
            add_operand_uses(live, operand);
        }
        TerminatorKind::Call { func, args, .. } => {
            add_operand_uses(live, func);
            for arg in args {
                add_operand_uses(live, arg);
            }
        }
        TerminatorKind::Goto(_) | TerminatorKind::Return | TerminatorKind::Unreachable => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span() -> shape_ast::ast::Span {
        shape_ast::ast::Span { start: 0, end: 1 }
    }

    fn make_stmt(kind: StatementKind, point: u32) -> MirStatement {
        MirStatement {
            kind,
            span: span(),
            point: Point(point),
        }
    }

    fn make_terminator(kind: TerminatorKind) -> Terminator {
        Terminator { kind, span: span() }
    }

    #[test]
    fn test_simple_liveness() {
        // bb0: x = 1; y = x; return
        let mir = MirFunction {
            name: "test".to_string(),
            blocks: vec![BasicBlock {
                id: BasicBlockId(0),
                statements: vec![
                    make_stmt(
                        StatementKind::Assign(
                            Place::Local(SlotId(0)),
                            Rvalue::Use(Operand::Constant(MirConstant::Int(1))),
                        ),
                        0,
                    ),
                    make_stmt(
                        StatementKind::Assign(
                            Place::Local(SlotId(1)),
                            Rvalue::Use(Operand::Copy(Place::Local(SlotId(0)))),
                        ),
                        1,
                    ),
                ],
                terminator: make_terminator(TerminatorKind::Return),
            }],
            num_locals: 2,
            param_slots: vec![],
            local_types: vec![LocalTypeInfo::Copy, LocalTypeInfo::Copy],
            span: span(),
        };

        let cfg = ControlFlowGraph::build(&mir);
        let liveness = compute_liveness(&mir, &cfg);

        // x (slot 0) should be live after stmt 0 (used in stmt 1)
        assert!(liveness.is_live_after(BasicBlockId(0), 0, SlotId(0), &mir));
        // x (slot 0) should NOT be live after stmt 1 (never used again)
        assert!(!liveness.is_live_after(BasicBlockId(0), 1, SlotId(0), &mir));
    }

    #[test]
    fn test_branch_liveness() {
        // bb0: x = 1; if cond goto bb1 else bb2
        // bb1: y = x; goto bb3
        // bb2: goto bb3
        // bb3: return
        let mir = MirFunction {
            name: "test".to_string(),
            blocks: vec![
                BasicBlock {
                    id: BasicBlockId(0),
                    statements: vec![make_stmt(
                        StatementKind::Assign(
                            Place::Local(SlotId(0)),
                            Rvalue::Use(Operand::Constant(MirConstant::Int(1))),
                        ),
                        0,
                    )],
                    terminator: make_terminator(TerminatorKind::SwitchBool {
                        operand: Operand::Copy(Place::Local(SlotId(2))),
                        true_bb: BasicBlockId(1),
                        false_bb: BasicBlockId(2),
                    }),
                },
                BasicBlock {
                    id: BasicBlockId(1),
                    statements: vec![make_stmt(
                        StatementKind::Assign(
                            Place::Local(SlotId(1)),
                            Rvalue::Use(Operand::Copy(Place::Local(SlotId(0)))),
                        ),
                        1,
                    )],
                    terminator: make_terminator(TerminatorKind::Goto(BasicBlockId(3))),
                },
                BasicBlock {
                    id: BasicBlockId(2),
                    statements: vec![],
                    terminator: make_terminator(TerminatorKind::Goto(BasicBlockId(3))),
                },
                BasicBlock {
                    id: BasicBlockId(3),
                    statements: vec![],
                    terminator: make_terminator(TerminatorKind::Return),
                },
            ],
            num_locals: 3,
            param_slots: vec![],
            local_types: vec![
                LocalTypeInfo::Copy,
                LocalTypeInfo::Copy,
                LocalTypeInfo::Copy,
            ],
            span: span(),
        };

        let cfg = ControlFlowGraph::build(&mir);
        let liveness = compute_liveness(&mir, &cfg);

        // x (slot 0) should be live at entry of bb0 (used in bb1 via some path)
        // Actually, x is defined in bb0, so it's live at exit of bb0
        assert!(
            liveness
                .live_out
                .get(&BasicBlockId(0))
                .map_or(false, |s| s.contains(&SlotId(0)))
        );
    }
}
