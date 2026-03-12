//! Field-level definite assignment and liveness analysis on MIR.
//!
//! Supplements the AST-level "optimistic hoisting" pre-pass (Phase 1) with a
//! flow-sensitive MIR analysis (Phase 2) that uses the CFG with dominators.
//! Phase 1 runs before compilation to collect fields; Phase 2 validates them
//! per-function after MIR lowering, detecting conditionally-initialized and
//! dead fields. Tracks which TypedObject
//! fields are definitely initialized, conditionally initialized, live (have
//! future reads), or dead (written but never read) at each program point.
//!
//! The analysis has two phases:
//! 1. **Forward**: Definite initialization — which fields are guaranteed to be
//!    assigned on every path from the entry block to a given point.
//! 2. **Backward**: Field liveness — which (slot, field) pairs will be read on
//!    some path from the current point to function exit.

use super::cfg::ControlFlowGraph;
use super::types::*;
use std::collections::{HashMap, HashSet};

/// A (slot, field) pair identifying a specific field on a specific local.
pub type FieldKey = (SlotId, FieldIdx);

/// Results of field-level analysis for a single MIR function.
#[derive(Debug)]
pub struct FieldAnalysis {
    /// Fields that are definitely initialized at the *entry* of each block.
    pub definitely_initialized: HashMap<BasicBlockId, HashSet<FieldKey>>,
    /// Fields that are live (have future reads) at the *entry* of each block.
    pub field_liveness: HashMap<BasicBlockId, HashSet<FieldKey>>,
    /// Fields that are assigned but never read anywhere in the function.
    pub dead_fields: HashSet<FieldKey>,
    /// Fields that are initialized on some but not all paths to a use point.
    pub conditionally_initialized: HashSet<FieldKey>,
    /// Fields eligible for TypedObject schema hoisting: written on any path
    /// and not dead.  Keyed by slot, with the list of field indices to hoist.
    pub hoisted_fields: HashMap<SlotId, Vec<FieldIdx>>,
    /// MIR-authoritative hoisting recommendations: maps each slot to pairs of
    /// (field_index, field_name) for schema construction. Populated when
    /// field names are available from the lowering result.
    pub hoisting_recommendations: HashMap<SlotId, Vec<(FieldIdx, String)>>,
}

/// Input bundle for field analysis.
pub struct FieldAnalysisInput<'a> {
    pub mir: &'a MirFunction,
    pub cfg: &'a ControlFlowGraph,
}

/// Run field-level definite-assignment and liveness analysis.
pub fn analyze_fields(input: &FieldAnalysisInput) -> FieldAnalysis {
    let mir = input.mir;
    let cfg = input.cfg;

    // Step 1: Collect all field writes and reads across the whole function.
    let (block_writes, block_reads, all_writes, all_reads) = collect_field_accesses(mir);

    // Step 2: Forward dataflow — definite initialization.
    let definitely_initialized = compute_definite_initialization(mir, cfg, &block_writes);

    // Step 3: Backward dataflow — field liveness.
    let field_liveness = compute_field_liveness(mir, cfg, &block_writes, &block_reads);

    // Step 4: Dead fields = written but never read anywhere.
    let dead_fields: HashSet<FieldKey> = all_writes.difference(&all_reads).cloned().collect();

    // Step 5: Conditionally initialized = initialized on some paths but not all
    // paths to a use point (i.e., the field is read somewhere, and at the entry
    // of some block that reads it, it is NOT definitely initialized).
    let conditionally_initialized =
        compute_conditionally_initialized(mir, &block_reads, &definitely_initialized, &all_writes);

    // Step 6: Hoisted fields = written on any path and not dead.
    // These are candidates for inclusion in the TypedObject schema at
    // object-creation time so the schema doesn't need runtime migration.
    let mut hoisted_fields: HashMap<SlotId, Vec<FieldIdx>> = HashMap::new();
    for key in &all_writes {
        if !dead_fields.contains(key) {
            hoisted_fields.entry(key.0).or_default().push(key.1);
        }
    }

    FieldAnalysis {
        definitely_initialized,
        field_liveness,
        dead_fields,
        conditionally_initialized,
        hoisted_fields,
        hoisting_recommendations: HashMap::new(), // populated by caller with field names
    }
}

/// Collect per-block field writes and reads, plus global write/read sets.
///
/// Returns `(block_writes, block_reads, all_writes, all_reads)`.
fn collect_field_accesses(
    mir: &MirFunction,
) -> (
    HashMap<BasicBlockId, HashSet<FieldKey>>,
    HashMap<BasicBlockId, HashSet<FieldKey>>,
    HashSet<FieldKey>,
    HashSet<FieldKey>,
) {
    let mut block_writes: HashMap<BasicBlockId, HashSet<FieldKey>> = HashMap::new();
    let mut block_reads: HashMap<BasicBlockId, HashSet<FieldKey>> = HashMap::new();
    let mut all_writes = HashSet::new();
    let mut all_reads = HashSet::new();

    for block in &mir.blocks {
        let writes = block_writes.entry(block.id).or_default();
        let reads = block_reads.entry(block.id).or_default();

        for stmt in &block.statements {
            collect_statement_field_accesses(&stmt.kind, writes, reads);
        }
        // Terminators can also read fields (e.g., a field used as a call arg).
        collect_terminator_field_reads(&block.terminator.kind, reads);

        all_writes.extend(writes.iter().cloned());
        all_reads.extend(reads.iter().cloned());
    }

    (block_writes, block_reads, all_writes, all_reads)
}

/// Extract field writes and reads from a single statement.
fn collect_statement_field_accesses(
    kind: &StatementKind,
    writes: &mut HashSet<FieldKey>,
    reads: &mut HashSet<FieldKey>,
) {
    match kind {
        StatementKind::Assign(place, rvalue) => {
            // Check for field write: `slot.field = ...`
            if let Some(key) = extract_field_key(place) {
                writes.insert(key);
            }
            // The assignment target might also be a deeper read (e.g., `a.x.y = ...`
            // reads `a.x`). We only track one level of field for now, but we should
            // still record reads from the rvalue side.
            collect_rvalue_field_reads(rvalue, reads);
        }
        StatementKind::Drop(place) => {
            // A drop reads the place.
            if let Some(key) = extract_field_key(place) {
                reads.insert(key);
            }
        }
        StatementKind::TaskBoundary(ops, ..)
        | StatementKind::ClosureCapture { operands: ops, .. }
        | StatementKind::ArrayStore { operands: ops, .. }
        | StatementKind::ObjectStore { operands: ops, .. }
        | StatementKind::EnumStore { operands: ops, .. } => {
            for op in ops {
                collect_operand_field_reads(op, reads);
            }
        }
        StatementKind::Nop => {}
    }
}

/// Extract field reads from an rvalue.
fn collect_rvalue_field_reads(rvalue: &Rvalue, reads: &mut HashSet<FieldKey>) {
    match rvalue {
        Rvalue::Use(op) | Rvalue::Clone(op) | Rvalue::UnaryOp(_, op) => {
            collect_operand_field_reads(op, reads);
        }
        Rvalue::Borrow(_, place) => {
            if let Some(key) = extract_field_key(place) {
                reads.insert(key);
            }
        }
        Rvalue::BinaryOp(_, lhs, rhs) => {
            collect_operand_field_reads(lhs, reads);
            collect_operand_field_reads(rhs, reads);
        }
        Rvalue::Aggregate(ops) => {
            for op in ops {
                collect_operand_field_reads(op, reads);
            }
        }
    }
}

/// Extract field reads from an operand.
fn collect_operand_field_reads(op: &Operand, reads: &mut HashSet<FieldKey>) {
    match op {
        Operand::Copy(place) | Operand::Move(place) | Operand::MoveExplicit(place) => {
            if let Some(key) = extract_field_key(place) {
                reads.insert(key);
            }
        }
        Operand::Constant(_) => {}
    }
}

/// Extract field reads from a terminator.
fn collect_terminator_field_reads(kind: &TerminatorKind, reads: &mut HashSet<FieldKey>) {
    match kind {
        TerminatorKind::SwitchBool { operand, .. } => {
            collect_operand_field_reads(operand, reads);
        }
        TerminatorKind::Call { func, args, .. } => {
            collect_operand_field_reads(func, reads);
            for arg in args {
                collect_operand_field_reads(arg, reads);
            }
        }
        TerminatorKind::Goto(_) | TerminatorKind::Return | TerminatorKind::Unreachable => {}
    }
}

/// Extract the `(SlotId, FieldIdx)` from a `Place::Field(Place::Local(slot), idx)`.
/// Returns `None` for non-field places or nested field paths (we only track
/// single-level fields).
fn extract_field_key(place: &Place) -> Option<FieldKey> {
    match place {
        Place::Field(base, idx) => match base.as_ref() {
            Place::Local(slot) => Some((*slot, *idx)),
            _ => None,
        },
        _ => None,
    }
}

// ── Forward dataflow: definite initialization ──────────────────────────

/// Compute which fields are definitely initialized at the entry of each block.
///
/// Lattice: `HashSet<FieldKey>` with intersection as meet.
///   - Entry block: empty set (nothing initialized).
///   - Transfer: `out[B] = in[B] ∪ writes[B]` (once a field is written, it
///     stays initialized on that path).
///   - Merge at join points: `in[B] = ∩ out[P] for all predecessors P`.
///     A field is definitely initialized only if ALL predecessor paths
///     initialize it.
fn compute_definite_initialization(
    mir: &MirFunction,
    cfg: &ControlFlowGraph,
    block_writes: &HashMap<BasicBlockId, HashSet<FieldKey>>,
) -> HashMap<BasicBlockId, HashSet<FieldKey>> {
    let rpo = cfg.reverse_postorder();
    let entry = mir.entry_block();

    // `init_out[B]` = definitely initialized fields at the *exit* of block B.
    let mut init_in: HashMap<BasicBlockId, HashSet<FieldKey>> = HashMap::new();
    let mut init_out: HashMap<BasicBlockId, HashSet<FieldKey>> = HashMap::new();

    // Collect the universe of all field keys for the "TOP" element.
    // For definite init, top = all fields (intersection identity), bottom = empty.
    let universe: HashSet<FieldKey> = block_writes.values().flatten().cloned().collect();

    // Initialize: entry gets empty (nothing initialized); all others get TOP.
    for block in &mir.blocks {
        if block.id == entry {
            init_in.insert(block.id, HashSet::new());
        } else {
            init_in.insert(block.id, universe.clone());
        }
    }

    // Apply transfer for initial out values.
    for block in &mir.blocks {
        let in_set = init_in.get(&block.id).cloned().unwrap_or_default();
        let writes = block_writes.get(&block.id).cloned().unwrap_or_default();
        let out_set: HashSet<FieldKey> = in_set.union(&writes).cloned().collect();
        init_out.insert(block.id, out_set);
    }

    // Iterate until fixpoint.
    let mut changed = true;
    while changed {
        changed = false;

        for &block_id in &rpo {
            // Merge: intersect out-sets of all predecessors.
            let preds = cfg.predecessors(block_id);
            let new_in = if block_id == entry {
                HashSet::new()
            } else if preds.is_empty() {
                // Unreachable block — leave as universe (won't affect results).
                universe.clone()
            } else {
                let mut merged = init_out
                    .get(&preds[0])
                    .cloned()
                    .unwrap_or_else(|| universe.clone());
                for &pred in &preds[1..] {
                    let pred_out = init_out
                        .get(&pred)
                        .cloned()
                        .unwrap_or_else(|| universe.clone());
                    merged = merged.intersection(&pred_out).cloned().collect();
                }
                merged
            };

            // Transfer: out = in ∪ writes
            let writes = block_writes.get(&block_id).cloned().unwrap_or_default();
            let new_out: HashSet<FieldKey> = new_in.union(&writes).cloned().collect();

            if new_in != *init_in.get(&block_id).unwrap_or(&HashSet::new()) {
                changed = true;
                init_in.insert(block_id, new_in);
            }
            if new_out != *init_out.get(&block_id).unwrap_or(&HashSet::new()) {
                changed = true;
                init_out.insert(block_id, new_out);
            }
        }
    }

    init_in
}

// ── Backward dataflow: field liveness ──────────────────────────────────

/// Compute which fields are live (have future reads) at the entry of each block.
///
/// Standard backward liveness:
///   - `live_out[B] = ∪ live_in[S] for all successors S`.
///   - `live_in[B] = (live_out[B] − kill[B]) ∪ use[B]`
///     where `kill[B]` is the set of fields definitely overwritten (we don't
///     kill in this analysis since a write doesn't prevent an earlier read from
///     being live — it's the same as standard variable liveness but for fields).
///
///   Actually for field liveness we use a simpler model:
///   - `use[B]` = fields read in block B.
///   - `def[B]` = fields written in block B (kills liveness for fields defined
///     before being read within the same block).
///   - `live_in[B] = (live_out[B] − def_before_use[B]) ∪ use[B]`
fn compute_field_liveness(
    mir: &MirFunction,
    cfg: &ControlFlowGraph,
    block_writes: &HashMap<BasicBlockId, HashSet<FieldKey>>,
    block_reads: &HashMap<BasicBlockId, HashSet<FieldKey>>,
) -> HashMap<BasicBlockId, HashSet<FieldKey>> {
    let rpo = cfg.reverse_postorder();

    // For each block, compute `use_before_def` and `def_before_use`.
    // A field is "used before def" if it appears as a read before any write
    // in the same block. A field is "def before use" if it's written before
    // any read in the same block.
    let mut use_before_def: HashMap<BasicBlockId, HashSet<FieldKey>> = HashMap::new();
    let mut def_before_use: HashMap<BasicBlockId, HashSet<FieldKey>> = HashMap::new();

    for block in &mir.blocks {
        let (ubd, dbu) = compute_block_use_def_order(block);
        use_before_def.insert(block.id, ubd);
        def_before_use.insert(block.id, dbu);
    }

    let mut live_in: HashMap<BasicBlockId, HashSet<FieldKey>> = HashMap::new();
    let mut live_out: HashMap<BasicBlockId, HashSet<FieldKey>> = HashMap::new();

    for block in &mir.blocks {
        live_in.insert(block.id, HashSet::new());
        live_out.insert(block.id, HashSet::new());
    }

    let mut changed = true;
    while changed {
        changed = false;

        // Process in reverse of RPO for efficient backward analysis.
        for &block_id in rpo.iter().rev() {
            // live_out[B] = ∪ live_in[S] for all successors S
            let mut new_live_out: HashSet<FieldKey> = HashSet::new();
            for &succ in cfg.successors(block_id) {
                if let Some(succ_in) = live_in.get(&succ) {
                    new_live_out.extend(succ_in.iter().cloned());
                }
            }

            // live_in[B] = (live_out[B] − def_before_use[B]) ∪ use_before_def[B]
            let dbu = def_before_use.get(&block_id).cloned().unwrap_or_default();
            let ubd = use_before_def.get(&block_id).cloned().unwrap_or_default();

            let mut new_live_in: HashSet<FieldKey> =
                new_live_out.difference(&dbu).cloned().collect();
            new_live_in.extend(ubd.iter().cloned());

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

    live_in
}

/// For a single block, compute `(use_before_def, def_before_use)`.
///
/// Walk statements in order. For each field key:
/// - If the first access is a read, it goes into `use_before_def`.
/// - If the first access is a write, it goes into `def_before_use`.
fn compute_block_use_def_order(block: &BasicBlock) -> (HashSet<FieldKey>, HashSet<FieldKey>) {
    let mut use_before_def = HashSet::new();
    let mut def_before_use = HashSet::new();
    let mut seen = HashSet::new();

    for stmt in &block.statements {
        // Collect reads from this statement.
        let mut stmt_reads = HashSet::new();
        let mut stmt_writes = HashSet::new();

        match &stmt.kind {
            StatementKind::Assign(place, rvalue) => {
                // Reads from the rvalue come first (executed before the write).
                collect_rvalue_field_reads(rvalue, &mut stmt_reads);
                if let Some(key) = extract_field_key(place) {
                    stmt_writes.insert(key);
                }
            }
            StatementKind::Drop(place) => {
                if let Some(key) = extract_field_key(place) {
                    stmt_reads.insert(key);
                }
            }
            StatementKind::TaskBoundary(ops, ..)
            | StatementKind::ClosureCapture { operands: ops, .. }
            | StatementKind::ArrayStore { operands: ops, .. }
            | StatementKind::ObjectStore { operands: ops, .. }
            | StatementKind::EnumStore { operands: ops, .. } => {
                for op in ops {
                    collect_operand_field_reads(op, &mut stmt_reads);
                }
            }
            StatementKind::Nop => {}
        }

        // Reads before writes within the same statement.
        for key in &stmt_reads {
            if !seen.contains(key) {
                use_before_def.insert(*key);
                seen.insert(*key);
            }
        }
        for key in &stmt_writes {
            if !seen.contains(key) {
                def_before_use.insert(*key);
                seen.insert(*key);
            }
        }
    }

    // Also account for terminator reads.
    let mut term_reads = HashSet::new();
    collect_terminator_field_reads(&block.terminator.kind, &mut term_reads);
    for key in &term_reads {
        if !seen.contains(key) {
            use_before_def.insert(*key);
            // seen.insert not needed — last pass
        }
    }

    (use_before_def, def_before_use)
}

// ── Conditional initialization detection ───────────────────────────────

/// A field is conditionally initialized if it is written on at least one path
/// but at some block where it is read, it is NOT in the definitely-initialized
/// set.
fn compute_conditionally_initialized(
    mir: &MirFunction,
    block_reads: &HashMap<BasicBlockId, HashSet<FieldKey>>,
    definitely_initialized: &HashMap<BasicBlockId, HashSet<FieldKey>>,
    all_writes: &HashSet<FieldKey>,
) -> HashSet<FieldKey> {
    let mut conditionally = HashSet::new();

    for block in &mir.blocks {
        let reads = match block_reads.get(&block.id) {
            Some(r) => r,
            None => continue,
        };
        let init = definitely_initialized
            .get(&block.id)
            .cloned()
            .unwrap_or_default();

        for key in reads {
            // The field is written somewhere (it's in all_writes) but not
            // definitely initialized at this read point.
            if all_writes.contains(key) && !init.contains(key) {
                conditionally.insert(*key);
            }
        }
    }

    conditionally
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir::cfg::ControlFlowGraph;

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

    fn field_place(slot: u16, field: u16) -> Place {
        Place::Field(Box::new(Place::Local(SlotId(slot))), FieldIdx(field))
    }

    // ── Test: unconditional initialization ─────────────────────────────

    #[test]
    fn test_unconditional_field_init() {
        // bb0: _0.0 = 1; _0.1 = 2; return
        // Both fields are definitely initialized at exit of bb0.
        let mir = MirFunction {
            name: "test".to_string(),
            blocks: vec![BasicBlock {
                id: BasicBlockId(0),
                statements: vec![
                    make_stmt(
                        StatementKind::Assign(
                            field_place(0, 0),
                            Rvalue::Use(Operand::Constant(MirConstant::Int(1))),
                        ),
                        0,
                    ),
                    make_stmt(
                        StatementKind::Assign(
                            field_place(0, 1),
                            Rvalue::Use(Operand::Constant(MirConstant::Int(2))),
                        ),
                        1,
                    ),
                ],
                terminator: make_terminator(TerminatorKind::Return),
            }],
            num_locals: 1,
            param_slots: vec![],
            param_reference_kinds: vec![],
            local_types: vec![LocalTypeInfo::NonCopy],
            span: span(),
        };

        let cfg = ControlFlowGraph::build(&mir);
        let result = analyze_fields(&FieldAnalysisInput { mir: &mir, cfg: &cfg });

        // At entry of bb0, nothing is initialized (correct: writes happen inside).
        let init_at_entry = result
            .definitely_initialized
            .get(&BasicBlockId(0))
            .cloned()
            .unwrap_or_default();
        assert!(init_at_entry.is_empty());

        // Both fields were written but never read → dead fields.
        assert!(result.dead_fields.contains(&(SlotId(0), FieldIdx(0))));
        assert!(result.dead_fields.contains(&(SlotId(0), FieldIdx(1))));

        // No conditional initialization (there's only one path).
        assert!(result.conditionally_initialized.is_empty());
    }

    // ── Test: conditional initialization (if/else) ─────────────────────

    #[test]
    fn test_conditional_field_init() {
        // bb0: if cond goto bb1 else bb2
        // bb1: _0.0 = 1; goto bb3
        // bb2: goto bb3
        // bb3: use _0.0; return
        //
        // _0.0 is only initialized in bb1, not bb2 → conditionally initialized.
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
                    statements: vec![make_stmt(
                        StatementKind::Assign(
                            field_place(0, 0),
                            Rvalue::Use(Operand::Constant(MirConstant::Int(1))),
                        ),
                        0,
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
                    statements: vec![make_stmt(
                        StatementKind::Assign(
                            Place::Local(SlotId(1)),
                            Rvalue::Use(Operand::Copy(field_place(0, 0))),
                        ),
                        1,
                    )],
                    terminator: make_terminator(TerminatorKind::Return),
                },
            ],
            num_locals: 2,
            param_slots: vec![],
            param_reference_kinds: vec![],
            local_types: vec![LocalTypeInfo::NonCopy, LocalTypeInfo::Copy],
            span: span(),
        };

        let cfg = ControlFlowGraph::build(&mir);
        let result = analyze_fields(&FieldAnalysisInput { mir: &mir, cfg: &cfg });

        // At bb3 entry, _0.0 should NOT be definitely initialized (missing from bb2 path).
        let init_at_bb3 = result
            .definitely_initialized
            .get(&BasicBlockId(3))
            .cloned()
            .unwrap_or_default();
        assert!(
            !init_at_bb3.contains(&(SlotId(0), FieldIdx(0))),
            "field should not be definitely initialized at join point"
        );

        // _0.0 should be conditionally initialized.
        assert!(
            result
                .conditionally_initialized
                .contains(&(SlotId(0), FieldIdx(0))),
            "field should be conditionally initialized"
        );
    }

    // ── Test: both branches initialize → definitely initialized ────────

    #[test]
    fn test_both_branches_init() {
        // bb0: if cond goto bb1 else bb2
        // bb1: _0.0 = 1; goto bb3
        // bb2: _0.0 = 2; goto bb3
        // bb3: use _0.0; return
        //
        // _0.0 is initialized on ALL paths → definitely initialized at bb3.
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
                    statements: vec![make_stmt(
                        StatementKind::Assign(
                            field_place(0, 0),
                            Rvalue::Use(Operand::Constant(MirConstant::Int(1))),
                        ),
                        0,
                    )],
                    terminator: make_terminator(TerminatorKind::Goto(BasicBlockId(3))),
                },
                BasicBlock {
                    id: BasicBlockId(2),
                    statements: vec![make_stmt(
                        StatementKind::Assign(
                            field_place(0, 0),
                            Rvalue::Use(Operand::Constant(MirConstant::Int(2))),
                        ),
                        1,
                    )],
                    terminator: make_terminator(TerminatorKind::Goto(BasicBlockId(3))),
                },
                BasicBlock {
                    id: BasicBlockId(3),
                    statements: vec![make_stmt(
                        StatementKind::Assign(
                            Place::Local(SlotId(1)),
                            Rvalue::Use(Operand::Copy(field_place(0, 0))),
                        ),
                        2,
                    )],
                    terminator: make_terminator(TerminatorKind::Return),
                },
            ],
            num_locals: 2,
            param_slots: vec![],
            param_reference_kinds: vec![],
            local_types: vec![LocalTypeInfo::NonCopy, LocalTypeInfo::Copy],
            span: span(),
        };

        let cfg = ControlFlowGraph::build(&mir);
        let result = analyze_fields(&FieldAnalysisInput { mir: &mir, cfg: &cfg });

        // At bb3 entry, _0.0 SHOULD be definitely initialized.
        let init_at_bb3 = result
            .definitely_initialized
            .get(&BasicBlockId(3))
            .cloned()
            .unwrap_or_default();
        assert!(
            init_at_bb3.contains(&(SlotId(0), FieldIdx(0))),
            "field should be definitely initialized when both branches write it"
        );

        // Not conditionally initialized (all paths cover it).
        assert!(
            !result
                .conditionally_initialized
                .contains(&(SlotId(0), FieldIdx(0))),
        );

        // Not dead (it's read in bb3).
        assert!(
            !result.dead_fields.contains(&(SlotId(0), FieldIdx(0))),
            "field is read so should not be dead"
        );
    }

    // ── Test: dead field detection ─────────────────────────────────────

    #[test]
    fn test_dead_field() {
        // bb0: _0.0 = 1; _0.1 = 2; use _0.0; return
        // _0.1 is written but never read → dead.
        let mir = MirFunction {
            name: "test".to_string(),
            blocks: vec![BasicBlock {
                id: BasicBlockId(0),
                statements: vec![
                    make_stmt(
                        StatementKind::Assign(
                            field_place(0, 0),
                            Rvalue::Use(Operand::Constant(MirConstant::Int(1))),
                        ),
                        0,
                    ),
                    make_stmt(
                        StatementKind::Assign(
                            field_place(0, 1),
                            Rvalue::Use(Operand::Constant(MirConstant::Int(2))),
                        ),
                        1,
                    ),
                    make_stmt(
                        StatementKind::Assign(
                            Place::Local(SlotId(1)),
                            Rvalue::Use(Operand::Copy(field_place(0, 0))),
                        ),
                        2,
                    ),
                ],
                terminator: make_terminator(TerminatorKind::Return),
            }],
            num_locals: 2,
            param_slots: vec![],
            param_reference_kinds: vec![],
            local_types: vec![LocalTypeInfo::NonCopy, LocalTypeInfo::Copy],
            span: span(),
        };

        let cfg = ControlFlowGraph::build(&mir);
        let result = analyze_fields(&FieldAnalysisInput { mir: &mir, cfg: &cfg });

        // _0.0 is read → not dead.
        assert!(!result.dead_fields.contains(&(SlotId(0), FieldIdx(0))));
        // _0.1 is written but never read → dead.
        assert!(result.dead_fields.contains(&(SlotId(0), FieldIdx(1))));
    }

    // ── Test: field liveness ───────────────────────────────────────────

    #[test]
    fn test_field_liveness() {
        // bb0: _0.0 = 1; goto bb1
        // bb1: _1 = _0.0; return
        //
        // _0.0 should be live at exit of bb0 (read in bb1).
        let mir = MirFunction {
            name: "test".to_string(),
            blocks: vec![
                BasicBlock {
                    id: BasicBlockId(0),
                    statements: vec![make_stmt(
                        StatementKind::Assign(
                            field_place(0, 0),
                            Rvalue::Use(Operand::Constant(MirConstant::Int(1))),
                        ),
                        0,
                    )],
                    terminator: make_terminator(TerminatorKind::Goto(BasicBlockId(1))),
                },
                BasicBlock {
                    id: BasicBlockId(1),
                    statements: vec![make_stmt(
                        StatementKind::Assign(
                            Place::Local(SlotId(1)),
                            Rvalue::Use(Operand::Copy(field_place(0, 0))),
                        ),
                        1,
                    )],
                    terminator: make_terminator(TerminatorKind::Return),
                },
            ],
            num_locals: 2,
            param_slots: vec![],
            param_reference_kinds: vec![],
            local_types: vec![LocalTypeInfo::NonCopy, LocalTypeInfo::Copy],
            span: span(),
        };

        let cfg = ControlFlowGraph::build(&mir);
        let result = analyze_fields(&FieldAnalysisInput { mir: &mir, cfg: &cfg });

        // _0.0 should be live at entry of bb1 (read there).
        let live_bb1 = result
            .field_liveness
            .get(&BasicBlockId(1))
            .cloned()
            .unwrap_or_default();
        assert!(
            live_bb1.contains(&(SlotId(0), FieldIdx(0))),
            "field should be live at entry of block where it is read"
        );

        // _0.0 should NOT be live at entry of bb0 (it is defined there before
        // any read within bb0, and the liveness from bb1 propagates back but
        // is killed by the def in bb0 — but since bb0 only writes, not reads,
        // and the write is a def_before_use, liveness is killed).
        // Actually the field is written in bb0 (def_before_use), so live_in[bb0]
        // should NOT contain it: live_out[bb0] has it, but def_before_use kills it.
        let live_bb0 = result
            .field_liveness
            .get(&BasicBlockId(0))
            .cloned()
            .unwrap_or_default();
        assert!(
            !live_bb0.contains(&(SlotId(0), FieldIdx(0))),
            "field defined before use in bb0 should not be live at bb0 entry"
        );
    }

    // ── Test: loop-based initialization ────────────────────────────────

    #[test]
    fn test_loop_init() {
        // bb0: goto bb1
        // bb1 (loop header): if cond goto bb2 else bb3
        // bb2 (loop body): _0.0 = 1; goto bb1
        // bb3 (exit): use _0.0; return
        //
        // _0.0 is only initialized inside the loop body, so at bb3 entry
        // it is NOT definitely initialized (bb1 can come from bb0 where
        // _0.0 wasn't written).
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
                    statements: vec![make_stmt(
                        StatementKind::Assign(
                            field_place(0, 0),
                            Rvalue::Use(Operand::Constant(MirConstant::Int(1))),
                        ),
                        0,
                    )],
                    terminator: make_terminator(TerminatorKind::Goto(BasicBlockId(1))),
                },
                BasicBlock {
                    id: BasicBlockId(3),
                    statements: vec![make_stmt(
                        StatementKind::Assign(
                            Place::Local(SlotId(1)),
                            Rvalue::Use(Operand::Copy(field_place(0, 0))),
                        ),
                        1,
                    )],
                    terminator: make_terminator(TerminatorKind::Return),
                },
            ],
            num_locals: 2,
            param_slots: vec![],
            param_reference_kinds: vec![],
            local_types: vec![LocalTypeInfo::NonCopy, LocalTypeInfo::Copy],
            span: span(),
        };

        let cfg = ControlFlowGraph::build(&mir);
        let result = analyze_fields(&FieldAnalysisInput { mir: &mir, cfg: &cfg });

        // At bb3 entry (after loop exit), _0.0 is NOT definitely initialized
        // because the path bb0 → bb1 → bb3 never writes _0.0.
        let init_at_bb3 = result
            .definitely_initialized
            .get(&BasicBlockId(3))
            .cloned()
            .unwrap_or_default();
        assert!(
            !init_at_bb3.contains(&(SlotId(0), FieldIdx(0))),
            "field initialized only in loop body should not be definitely initialized at loop exit"
        );

        // It IS conditionally initialized (written in bb2, read in bb3).
        assert!(
            result
                .conditionally_initialized
                .contains(&(SlotId(0), FieldIdx(0))),
        );
    }

    // ── Test: empty function ───────────────────────────────────────────

    #[test]
    fn test_empty_function() {
        let mir = MirFunction {
            name: "empty".to_string(),
            blocks: vec![BasicBlock {
                id: BasicBlockId(0),
                statements: vec![],
                terminator: make_terminator(TerminatorKind::Return),
            }],
            num_locals: 0,
            param_slots: vec![],
            param_reference_kinds: vec![],
            local_types: vec![],
            span: span(),
        };

        let cfg = ControlFlowGraph::build(&mir);
        let result = analyze_fields(&FieldAnalysisInput { mir: &mir, cfg: &cfg });

        assert!(result.dead_fields.is_empty());
        assert!(result.conditionally_initialized.is_empty());
    }

    // ── Test: multiple slots ───────────────────────────────────────────

    #[test]
    fn test_multiple_slots() {
        // bb0: _0.0 = 1; _1.0 = 2; _2 = _0.0 + _1.0; return
        // Both fields are read → not dead.
        let mir = MirFunction {
            name: "test".to_string(),
            blocks: vec![BasicBlock {
                id: BasicBlockId(0),
                statements: vec![
                    make_stmt(
                        StatementKind::Assign(
                            field_place(0, 0),
                            Rvalue::Use(Operand::Constant(MirConstant::Int(1))),
                        ),
                        0,
                    ),
                    make_stmt(
                        StatementKind::Assign(
                            field_place(1, 0),
                            Rvalue::Use(Operand::Constant(MirConstant::Int(2))),
                        ),
                        1,
                    ),
                    make_stmt(
                        StatementKind::Assign(
                            Place::Local(SlotId(2)),
                            Rvalue::BinaryOp(
                                BinOp::Add,
                                Operand::Copy(field_place(0, 0)),
                                Operand::Copy(field_place(1, 0)),
                            ),
                        ),
                        2,
                    ),
                ],
                terminator: make_terminator(TerminatorKind::Return),
            }],
            num_locals: 3,
            param_slots: vec![],
            param_reference_kinds: vec![],
            local_types: vec![
                LocalTypeInfo::NonCopy,
                LocalTypeInfo::NonCopy,
                LocalTypeInfo::Copy,
            ],
            span: span(),
        };

        let cfg = ControlFlowGraph::build(&mir);
        let result = analyze_fields(&FieldAnalysisInput { mir: &mir, cfg: &cfg });

        // Both fields are read → not dead.
        assert!(!result.dead_fields.contains(&(SlotId(0), FieldIdx(0))));
        assert!(!result.dead_fields.contains(&(SlotId(1), FieldIdx(0))));
    }
}
