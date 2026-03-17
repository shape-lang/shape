//! Control Flow Graph construction and traversal for MIR.

use super::types::{BasicBlockId, MirFunction, TerminatorKind};
use std::collections::{HashMap, HashSet, VecDeque};

/// A control flow graph for a MIR function.
/// Provides predecessor/successor queries and traversal orders.
#[derive(Debug)]
pub struct ControlFlowGraph {
    /// Successors of each block.
    successors: HashMap<BasicBlockId, Vec<BasicBlockId>>,
    /// Predecessors of each block.
    predecessors: HashMap<BasicBlockId, Vec<BasicBlockId>>,
}

impl ControlFlowGraph {
    /// Build a CFG from a MIR function.
    pub fn build(mir: &MirFunction) -> Self {
        let mut successors: HashMap<BasicBlockId, Vec<BasicBlockId>> = HashMap::new();
        let mut predecessors: HashMap<BasicBlockId, Vec<BasicBlockId>> = HashMap::new();

        for block in &mir.blocks {
            let succs = Self::terminator_successors(&block.terminator.kind);
            for &succ in &succs {
                predecessors.entry(succ).or_default().push(block.id);
            }
            successors.insert(block.id, succs);
        }

        ControlFlowGraph {
            successors,
            predecessors,
        }
    }

    /// Get the successors of a block.
    pub fn successors(&self, block: BasicBlockId) -> &[BasicBlockId] {
        self.successors.get(&block).map_or(&[], |v| v.as_slice())
    }

    /// Get the predecessors of a block.
    pub fn predecessors(&self, block: BasicBlockId) -> &[BasicBlockId] {
        self.predecessors.get(&block).map_or(&[], |v| v.as_slice())
    }

    /// Reverse postorder traversal (useful for forward dataflow analysis).
    pub fn reverse_postorder(&self) -> Vec<BasicBlockId> {
        let mut visited = HashSet::new();
        let mut postorder = Vec::new();
        let entry = BasicBlockId(0);

        self.dfs_postorder(entry, &mut visited, &mut postorder);
        postorder.reverse();
        postorder
    }

    fn dfs_postorder(
        &self,
        block: BasicBlockId,
        visited: &mut HashSet<BasicBlockId>,
        postorder: &mut Vec<BasicBlockId>,
    ) {
        if !visited.insert(block) {
            return;
        }
        for &succ in self.successors(block) {
            self.dfs_postorder(succ, visited, postorder);
        }
        postorder.push(block);
    }

    /// Compute dominators using the iterative dataflow algorithm.
    pub fn dominators(&self) -> HashMap<BasicBlockId, BasicBlockId> {
        let rpo = self.reverse_postorder();
        let entry = BasicBlockId(0);
        let mut doms: HashMap<BasicBlockId, BasicBlockId> = HashMap::new();
        doms.insert(entry, entry);

        let mut changed = true;
        while changed {
            changed = false;
            for &b in &rpo {
                if b == entry {
                    continue;
                }
                let preds = self.predecessors(b);
                let mut new_idom = None;
                for &p in preds {
                    if doms.contains_key(&p) {
                        new_idom = Some(match new_idom {
                            None => p,
                            Some(current) => self.intersect(current, p, &doms, &rpo),
                        });
                    }
                }
                if let Some(new_idom) = new_idom {
                    if doms.get(&b) != Some(&new_idom) {
                        doms.insert(b, new_idom);
                        changed = true;
                    }
                }
            }
        }

        doms
    }

    fn intersect(
        &self,
        mut a: BasicBlockId,
        mut b: BasicBlockId,
        doms: &HashMap<BasicBlockId, BasicBlockId>,
        rpo: &[BasicBlockId],
    ) -> BasicBlockId {
        let rpo_index: HashMap<BasicBlockId, usize> =
            rpo.iter().enumerate().map(|(i, &bb)| (bb, i)).collect();
        while a != b {
            while rpo_index.get(&a).copied().unwrap_or(0) > rpo_index.get(&b).copied().unwrap_or(0)
            {
                a = *doms.get(&a).unwrap_or(&a);
            }
            while rpo_index.get(&b).copied().unwrap_or(0) > rpo_index.get(&a).copied().unwrap_or(0)
            {
                b = *doms.get(&b).unwrap_or(&b);
            }
        }
        a
    }

    /// Check if a block is reachable from the entry.
    pub fn is_reachable(&self, target: BasicBlockId) -> bool {
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        queue.push_back(BasicBlockId(0));
        visited.insert(BasicBlockId(0));

        while let Some(block) = queue.pop_front() {
            if block == target {
                return true;
            }
            for &succ in self.successors(block) {
                if visited.insert(succ) {
                    queue.push_back(succ);
                }
            }
        }
        false
    }

    fn terminator_successors(kind: &TerminatorKind) -> Vec<BasicBlockId> {
        match kind {
            TerminatorKind::Goto(target) => vec![*target],
            TerminatorKind::SwitchBool {
                true_bb, false_bb, ..
            } => vec![*true_bb, *false_bb],
            TerminatorKind::Call { next, .. } => vec![*next],
            TerminatorKind::Return | TerminatorKind::Unreachable => vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir::types::*;

    fn span() -> shape_ast::ast::Span {
        shape_ast::ast::Span { start: 0, end: 1 }
    }

    fn make_terminator(kind: TerminatorKind) -> super::super::types::Terminator {
        super::super::types::Terminator { kind, span: span() }
    }

    #[test]
    fn test_linear_cfg() {
        let mir = MirFunction {
            name: "test".to_string(),
            blocks: vec![
                BasicBlock {
                    id: BasicBlockId(0),
                    statements: vec![],
                    terminator: make_terminator(TerminatorKind::Goto(BasicBlockId(1))),
                },
                BasicBlock {
                    id: BasicBlockId(1),
                    statements: vec![],
                    terminator: make_terminator(TerminatorKind::Return),
                },
            ],
            num_locals: 0,
            param_slots: vec![],
            param_reference_kinds: vec![],
            local_types: vec![],
            span: span(),
        };
        let cfg = ControlFlowGraph::build(&mir);
        assert_eq!(cfg.successors(BasicBlockId(0)), &[BasicBlockId(1)]);
        assert_eq!(cfg.predecessors(BasicBlockId(1)), &[BasicBlockId(0)]);
    }

    #[test]
    fn test_branch_cfg() {
        let mir = MirFunction {
            name: "test".to_string(),
            blocks: vec![
                BasicBlock {
                    id: BasicBlockId(0),
                    statements: vec![],
                    terminator: make_terminator(TerminatorKind::SwitchBool {
                        operand: Operand::Constant(MirConstant::Bool(true)),
                        true_bb: BasicBlockId(1),
                        false_bb: BasicBlockId(2),
                    }),
                },
                BasicBlock {
                    id: BasicBlockId(1),
                    statements: vec![],
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
            num_locals: 0,
            param_slots: vec![],
            param_reference_kinds: vec![],
            local_types: vec![],
            span: span(),
        };
        let cfg = ControlFlowGraph::build(&mir);
        let rpo = cfg.reverse_postorder();
        assert_eq!(rpo[0], BasicBlockId(0)); // entry first
        assert!(cfg.is_reachable(BasicBlockId(3)));
    }

    #[test]
    fn test_loop_cfg() {
        let mir = MirFunction {
            name: "test".to_string(),
            blocks: vec![
                BasicBlock {
                    id: BasicBlockId(0),
                    statements: vec![],
                    terminator: make_terminator(TerminatorKind::Goto(BasicBlockId(1))),
                },
                BasicBlock {
                    id: BasicBlockId(1),
                    statements: vec![],
                    terminator: make_terminator(TerminatorKind::SwitchBool {
                        operand: Operand::Constant(MirConstant::Bool(true)),
                        true_bb: BasicBlockId(2),
                        false_bb: BasicBlockId(3),
                    }),
                },
                BasicBlock {
                    id: BasicBlockId(2),
                    statements: vec![],
                    terminator: make_terminator(TerminatorKind::Goto(BasicBlockId(1))),
                },
                BasicBlock {
                    id: BasicBlockId(3),
                    statements: vec![],
                    terminator: make_terminator(TerminatorKind::Return),
                },
            ],
            num_locals: 0,
            param_slots: vec![],
            param_reference_kinds: vec![],
            local_types: vec![],
            span: span(),
        };
        let cfg = ControlFlowGraph::build(&mir);
        // Block 1 should have two predecessors: 0 (entry) and 2 (back edge)
        let preds = cfg.predecessors(BasicBlockId(1));
        assert_eq!(preds.len(), 2);
    }
}
