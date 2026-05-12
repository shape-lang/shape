//! Return-ownership inference (Phase 5.A of the ownership-aware runtime).
//!
//! Classifies each function's return value into a `ReturnOwnershipMode` so
//! callers can make interprocedural decisions — in particular, skip the
//! Arc→Box promotion that Phase 3 emits at let-binding sites when the callee
//! has already returned a uniquely-owned value.
//!
//! The inference is intentionally conservative: if any return path disagrees
//! with another, or we cannot prove what the return slot is derived from, the
//! result is `Unknown` and the caller falls back to today's Arc behavior.
//!
//! # Algorithm
//!
//! 1. For every basic block terminating in `Return`, find the most recent
//!    assignment to `SlotId(0)` (the dedicated return slot, established by
//!    `lower_return_control_flow`).
//! 2. Classify that assignment's source:
//!    - `Rvalue::Aggregate(..)` — array/struct/tuple literal — newly allocated
//!      in this frame: `NewlyOwned`.
//!    - `Rvalue::Use(Move(Place::Local(temp)))` where `temp` is the
//!      destination of an earlier Aggregate, ArrayStore, ObjectStore,
//!      EnumStore, ClosureCapture, or Call — trace one level deeper.
//!    - `Rvalue::Use(Copy(Place::Local(param)))` where `param` is a parameter
//!      slot — `BorrowedFromParam(idx)`.
//!    - `Rvalue::Use(Constant(Str | StringId | Function | Method))` —
//!      statically-lifetime immutable data: `Static`.
//!    - `Rvalue::Use(Constant(Int | Bool | Float | None | ..))` — a primitive
//!      / immediate, no heap allocation; treated as `NewlyOwned` since the
//!      caller's storage handles primitives directly without Arc wrapping.
//!    - `Rvalue::Borrow(..)` — explicit reference return.
//!    - A `Call` terminator whose callee summary says the callee returns
//!      `Shared` / `Static` — the mode propagates through.
//!    - Anything else — `Unknown`.
//! 3. If the function has multiple `Return` terminators, combine the modes
//!    via `meet`: identical modes survive, mismatches collapse to `Unknown`.
//! 4. A function with no `Return` terminators (diverging) is `Unknown`.
//!
//! # Call destinations
//!
//! Functions like `fn wrap() -> Array<int> { make() }` lower to a `Call`
//! terminator whose `destination` is a temp, followed by `Assign(_0,
//! Use(Move(temp)))` and then a `Return`. When we see such a pattern we look
//! up the callee's own return mode (threaded in via `callee_modes`). If the
//! callee is not in the map (e.g. first pass before fixed-point, or a
//! cross-module call), we fall back to `Unknown`.
//!
//! The pass runs in two contexts:
//! - Standalone (no callee info): used for the self-summary when extracting a
//!   `FunctionBorrowSummary` for the function under compilation. `Call`
//!   returns collapse to `Unknown` in this context.
//! - With callee info: via `extract_borrow_summary_with_callees`, threaded
//!   from `function_borrow_summaries` at callsite compilation time.

use std::collections::HashMap;

use super::analysis::ReturnOwnershipMode;
use super::types::*;

/// Infer the return-ownership mode for a single MIR function.
///
/// `callee_modes` carries previously-computed return modes for other
/// functions in the module. When a function's return value flows through a
/// `Call` terminator, the caller consults this map; an absent callee
/// collapses to `Unknown` (safe fallback — current Arc behavior).
///
/// The algorithm walks every reachable assignment to the return slot
/// (`SlotId(0)`) across the CFG and meets their modes. This is more forgiving
/// than keying off blocks that terminate in `Return` directly — the lowering
/// emits phantom exit blocks (a bare `Return` with no preceding slot-0
/// assignment) that would otherwise collapse the inference to `Unknown`.
pub fn infer_return_ownership_mode(
    mir: &MirFunction,
    callee_modes: &HashMap<String, ReturnOwnershipMode>,
) -> ReturnOwnershipMode {
    let return_slot = SlotId(0);
    let reachable = reachable_blocks(mir);
    let mut combined: Option<ReturnOwnershipMode> = None;

    for block in mir.iter_blocks() {
        if !reachable.contains(&block.id) {
            continue;
        }

        for stmt in &block.statements {
            let StatementKind::Assign(Place::Local(dest), rvalue) = &stmt.kind else {
                continue;
            };
            if *dest != return_slot {
                continue;
            }
            let mode = classify_rvalue(rvalue, mir, callee_modes);
            combined = Some(match combined {
                None => mode,
                Some(existing) => existing.meet(mode),
            });
        }
    }

    combined.unwrap_or(ReturnOwnershipMode::Unknown)
}

/// Compute the set of CFG-reachable basic blocks, starting from `bb0`.
fn reachable_blocks(mir: &MirFunction) -> std::collections::HashSet<BasicBlockId> {
    use std::collections::HashSet;
    use std::collections::VecDeque;

    let mut visited: HashSet<BasicBlockId> = HashSet::new();
    let mut queue: VecDeque<BasicBlockId> = VecDeque::new();

    if !mir.blocks.is_empty() {
        queue.push_back(mir.entry_block());
    }

    while let Some(id) = queue.pop_front() {
        if !visited.insert(id) {
            continue;
        }
        let block = mir.block(id);
        match &block.terminator.kind {
            TerminatorKind::Goto(target) => queue.push_back(*target),
            TerminatorKind::SwitchBool {
                true_bb, false_bb, ..
            } => {
                queue.push_back(*true_bb);
                queue.push_back(*false_bb);
            }
            TerminatorKind::Call { next, .. } => queue.push_back(*next),
            TerminatorKind::Return | TerminatorKind::Unreachable => {}
        }
    }

    visited
}

/// Classify an rvalue that's been assigned to the return slot.
fn classify_rvalue(
    rvalue: &Rvalue,
    mir: &MirFunction,
    callee_modes: &HashMap<String, ReturnOwnershipMode>,
) -> ReturnOwnershipMode {
    match rvalue {
        Rvalue::Aggregate(_) => ReturnOwnershipMode::NewlyOwned,
        Rvalue::Clone(_) => ReturnOwnershipMode::NewlyOwned,
        Rvalue::Borrow(kind, place) => classify_borrow_rvalue(*kind, place, mir),
        Rvalue::Use(operand) => classify_operand(operand, mir, callee_modes),
        // Binary/unary ops produce primitives (int/bool/float) — treat as NewlyOwned:
        // no Arc wrap is needed for primitives, so the caller can consume directly.
        Rvalue::BinaryOp(_, _, _) | Rvalue::UnaryOp(_, _) => ReturnOwnershipMode::NewlyOwned,
        // EnumTest produces a fresh native Bool — NewlyOwned by construction.
        // EnumPayload extracts an owned share from the wrapped Result/Option
        // payload per §2.7.17 receiver-recovery soundness; also NewlyOwned.
        Rvalue::EnumTest { .. } | Rvalue::EnumPayload { .. } => ReturnOwnershipMode::NewlyOwned,
    }
}

/// Classify a reference/borrow return: if the borrowed place roots in a
/// parameter slot, the return is `BorrowedFromParam`. Otherwise conservatively
/// `Unknown` (we don't yet propagate references to locals outward — that's
/// handled as an escape error elsewhere).
fn classify_borrow_rvalue(
    _kind: BorrowKind,
    place: &Place,
    mir: &MirFunction,
) -> ReturnOwnershipMode {
    let root = place.root_local();
    if let Some(param_idx) = mir.param_slots.iter().position(|s| *s == root) {
        ReturnOwnershipMode::BorrowedFromParam(param_idx)
    } else {
        ReturnOwnershipMode::Unknown
    }
}

/// Classify an operand that was used to populate the return slot.
fn classify_operand(
    operand: &Operand,
    mir: &MirFunction,
    callee_modes: &HashMap<String, ReturnOwnershipMode>,
) -> ReturnOwnershipMode {
    match operand {
        Operand::Constant(c) => classify_constant(c),
        Operand::Copy(place) | Operand::Move(place) | Operand::MoveExplicit(place) => {
            classify_place(place, mir, callee_modes)
        }
    }
}

/// Classify a `MirConstant` used as a return value.
///
/// String literals and function/method references live in the program's
/// static data segment — their lifetime is the whole program, so they're
/// safely classified as `Static`. Primitive immediates (`int`, `bool`, `f64`,
/// `None`) allocate nothing and are returned by value; `NewlyOwned` preserves
/// the pre-existing semantics for those.
///
/// `ClosurePlaceholder` is a bytecode-resolution marker and should never
/// appear in a completed MIR — treat as `Unknown` for safety.
fn classify_constant(c: &MirConstant) -> ReturnOwnershipMode {
    match c {
        MirConstant::Str(_)
        | MirConstant::StringId(_)
        | MirConstant::Function(_)
        | MirConstant::Method(_) => ReturnOwnershipMode::Static,
        MirConstant::Int(_)
        | MirConstant::Bool(_)
        | MirConstant::Float(_)
        | MirConstant::None => ReturnOwnershipMode::NewlyOwned,
        MirConstant::ClosurePlaceholder => ReturnOwnershipMode::Unknown,
    }
}

/// Classify a place used as the return source. If it's a parameter, that's a
/// borrowed return. If it's a local, trace its defining statement chain.
fn classify_place(
    place: &Place,
    mir: &MirFunction,
    callee_modes: &HashMap<String, ReturnOwnershipMode>,
) -> ReturnOwnershipMode {
    let root = place.root_local();

    if let Some(param_idx) = mir.param_slots.iter().position(|s| *s == root) {
        return ReturnOwnershipMode::BorrowedFromParam(param_idx);
    }

    // Local slot — find all defining statements / terminators and meet.
    let mut visited = std::collections::HashSet::new();
    trace_local_defining_mode(root, mir, callee_modes, &mut visited)
}

/// Find every defining statement / terminator for `slot` and meet their
/// classifications. Returns `Unknown` if inference can't decide.
///
/// Chains of temp copies (`_5 = _6; _4 = _5; ...`) are followed recursively
/// via `visited` to prevent loops. If a cycle is detected the mode is
/// `Unknown` — the safe fallback.
fn trace_local_defining_mode(
    slot: SlotId,
    mir: &MirFunction,
    callee_modes: &HashMap<String, ReturnOwnershipMode>,
    visited: &mut std::collections::HashSet<SlotId>,
) -> ReturnOwnershipMode {
    if !visited.insert(slot) {
        // Cycle or revisit — degrade to Unknown.
        return ReturnOwnershipMode::Unknown;
    }

    let mut combined: Option<ReturnOwnershipMode> = None;

    for block in mir.iter_blocks() {
        for stmt in &block.statements {
            if let StatementKind::Assign(Place::Local(dest), rvalue) = &stmt.kind {
                if *dest != slot {
                    continue;
                }
                let mode = classify_defining_rvalue(rvalue, mir, callee_modes, visited);
                combined = Some(match combined {
                    None => mode,
                    Some(existing) => existing.meet(mode),
                });
            }
        }

        if let TerminatorKind::Call {
            destination,
            func,
            ..
        } = &block.terminator.kind
        {
            if destination.root_local() == slot {
                let mode = match func {
                    Operand::Constant(MirConstant::Function(name)) => callee_modes
                        .get(name)
                        .copied()
                        .unwrap_or(ReturnOwnershipMode::Unknown),
                    _ => ReturnOwnershipMode::Unknown,
                };
                combined = Some(match combined {
                    None => mode,
                    Some(existing) => existing.meet(mode),
                });
            }
        }
    }

    combined.unwrap_or(ReturnOwnershipMode::Unknown)
}

/// Classify an Rvalue that defines a local slot. Unlike `classify_rvalue`
/// (which runs at the return slot), this one is called when we're tracing a
/// local's definition and so also needs to recurse through operand place
/// chains.
fn classify_defining_rvalue(
    rvalue: &Rvalue,
    mir: &MirFunction,
    callee_modes: &HashMap<String, ReturnOwnershipMode>,
    visited: &mut std::collections::HashSet<SlotId>,
) -> ReturnOwnershipMode {
    match rvalue {
        Rvalue::Aggregate(_) | Rvalue::Clone(_) => ReturnOwnershipMode::NewlyOwned,
        Rvalue::BinaryOp(_, _, _) | Rvalue::UnaryOp(_, _) => ReturnOwnershipMode::NewlyOwned,
        // EnumTest emits a Bool; EnumPayload emits an owned-share payload.
        Rvalue::EnumTest { .. } | Rvalue::EnumPayload { .. } => ReturnOwnershipMode::NewlyOwned,
        Rvalue::Borrow(kind, p) => classify_borrow_rvalue(*kind, p, mir),
        Rvalue::Use(op) => match op {
            Operand::Constant(c) => classify_constant(c),
            Operand::Copy(p) | Operand::Move(p) | Operand::MoveExplicit(p) => {
                let root = p.root_local();
                if let Some(idx) = mir.param_slots.iter().position(|s| *s == root) {
                    ReturnOwnershipMode::BorrowedFromParam(idx)
                } else {
                    trace_local_defining_mode(root, mir, callee_modes, visited)
                }
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_ast::ast::Span;
    use std::collections::HashMap as StdHashMap;

    fn dummy_span() -> Span {
        Span::DUMMY
    }

    fn empty_mir(name: &str) -> MirFunction {
        MirFunction {
            name: name.to_string(),
            blocks: Vec::new(),
            num_locals: 1,
            param_slots: Vec::new(),
            param_reference_kinds: Vec::new(),
            local_types: vec![LocalTypeInfo::Unknown],
            span: dummy_span(),
            field_name_table: StdHashMap::new(),
        }
    }

    fn push_stmt(block: &mut BasicBlock, kind: StatementKind, point: u32) {
        block.statements.push(MirStatement {
            kind,
            span: dummy_span(),
            point: Point(point),
        });
    }

    fn return_terminator() -> Terminator {
        Terminator {
            kind: TerminatorKind::Return,
            span: dummy_span(),
        }
    }

    #[test]
    fn test_aggregate_return_is_newly_owned() {
        // fn make() -> Array<int> { [1,2,3] } simplified MIR:
        //   bb0:
        //     _1 = Aggregate([1,2,3])
        //     _0 = move _1
        //     return
        let mut mir = empty_mir("make");
        mir.num_locals = 2;
        mir.local_types = vec![LocalTypeInfo::NonCopy, LocalTypeInfo::NonCopy];

        let mut bb0 = BasicBlock {
            id: BasicBlockId(0),
            statements: Vec::new(),
            terminator: return_terminator(),
        };

        push_stmt(
            &mut bb0,
            StatementKind::Assign(
                Place::Local(SlotId(1)),
                Rvalue::Aggregate(vec![
                    Operand::Constant(MirConstant::Int(1)),
                    Operand::Constant(MirConstant::Int(2)),
                    Operand::Constant(MirConstant::Int(3)),
                ]),
            ),
            0,
        );
        push_stmt(
            &mut bb0,
            StatementKind::Assign(
                Place::Local(SlotId(0)),
                Rvalue::Use(Operand::Move(Place::Local(SlotId(1)))),
            ),
            1,
        );

        mir.blocks.push(bb0);

        let callee_modes = HashMap::new();
        assert_eq!(
            infer_return_ownership_mode(&mir, &callee_modes),
            ReturnOwnershipMode::NewlyOwned
        );
    }

    #[test]
    fn test_param_copy_return_is_borrowed_from_param() {
        // fn first(arr: Array<int>) -> Array<int> { arr } simplified MIR:
        //   parameters: _1 = arr
        //   bb0:
        //     _0 = copy _1
        //     return
        let mut mir = empty_mir("first");
        mir.num_locals = 2;
        mir.param_slots = vec![SlotId(1)];
        mir.param_reference_kinds = vec![None];
        mir.local_types = vec![LocalTypeInfo::NonCopy, LocalTypeInfo::NonCopy];

        let mut bb0 = BasicBlock {
            id: BasicBlockId(0),
            statements: Vec::new(),
            terminator: return_terminator(),
        };
        push_stmt(
            &mut bb0,
            StatementKind::Assign(
                Place::Local(SlotId(0)),
                Rvalue::Use(Operand::Copy(Place::Local(SlotId(1)))),
            ),
            0,
        );

        mir.blocks.push(bb0);

        let callee_modes = HashMap::new();
        assert_eq!(
            infer_return_ownership_mode(&mir, &callee_modes),
            ReturnOwnershipMode::BorrowedFromParam(0)
        );
    }

    #[test]
    fn test_constant_return_is_newly_owned() {
        // fn answer() -> int { 42 }
        let mut mir = empty_mir("answer");
        mir.local_types = vec![LocalTypeInfo::Copy];
        let mut bb0 = BasicBlock {
            id: BasicBlockId(0),
            statements: Vec::new(),
            terminator: return_terminator(),
        };
        push_stmt(
            &mut bb0,
            StatementKind::Assign(
                Place::Local(SlotId(0)),
                Rvalue::Use(Operand::Constant(MirConstant::Int(42))),
            ),
            0,
        );
        mir.blocks.push(bb0);
        assert_eq!(
            infer_return_ownership_mode(&mir, &HashMap::new()),
            ReturnOwnershipMode::NewlyOwned
        );
    }

    #[test]
    fn test_binary_op_return_is_newly_owned() {
        let mut mir = empty_mir("add");
        mir.num_locals = 3;
        mir.param_slots = vec![SlotId(1), SlotId(2)];
        mir.param_reference_kinds = vec![None, None];
        mir.local_types = vec![LocalTypeInfo::Copy, LocalTypeInfo::Copy, LocalTypeInfo::Copy];
        let mut bb0 = BasicBlock {
            id: BasicBlockId(0),
            statements: Vec::new(),
            terminator: return_terminator(),
        };
        push_stmt(
            &mut bb0,
            StatementKind::Assign(
                Place::Local(SlotId(0)),
                Rvalue::BinaryOp(
                    BinOp::Add,
                    Operand::Copy(Place::Local(SlotId(1))),
                    Operand::Copy(Place::Local(SlotId(2))),
                ),
            ),
            0,
        );
        mir.blocks.push(bb0);
        assert_eq!(
            infer_return_ownership_mode(&mir, &HashMap::new()),
            ReturnOwnershipMode::NewlyOwned
        );
    }

    #[test]
    fn test_two_branches_same_newly_owned_meets_newly_owned() {
        // fn make(cond: bool) -> Array<int> {
        //     if cond { [1] } else { [2] }
        // }
        // Each branch allocates independently but both produce NewlyOwned.
        // Simplified MIR:
        //   bb0: switchBool(cond, bb1, bb2)
        //   bb1: _1 = Aggregate([1]); _0 = move _1; return
        //   bb2: _2 = Aggregate([2]); _0 = move _2; return
        let mut mir = empty_mir("make");
        mir.num_locals = 4;
        mir.param_slots = vec![SlotId(3)];
        mir.param_reference_kinds = vec![None];
        mir.local_types = vec![
            LocalTypeInfo::NonCopy,
            LocalTypeInfo::NonCopy,
            LocalTypeInfo::NonCopy,
            LocalTypeInfo::Copy,
        ];

        let bb0 = BasicBlock {
            id: BasicBlockId(0),
            statements: Vec::new(),
            terminator: Terminator {
                kind: TerminatorKind::SwitchBool {
                    operand: Operand::Copy(Place::Local(SlotId(3))),
                    true_bb: BasicBlockId(1),
                    false_bb: BasicBlockId(2),
                },
                span: dummy_span(),
            },
        };

        let mut bb1 = BasicBlock {
            id: BasicBlockId(1),
            statements: Vec::new(),
            terminator: return_terminator(),
        };
        push_stmt(
            &mut bb1,
            StatementKind::Assign(
                Place::Local(SlotId(1)),
                Rvalue::Aggregate(vec![Operand::Constant(MirConstant::Int(1))]),
            ),
            0,
        );
        push_stmt(
            &mut bb1,
            StatementKind::Assign(
                Place::Local(SlotId(0)),
                Rvalue::Use(Operand::Move(Place::Local(SlotId(1)))),
            ),
            1,
        );

        let mut bb2 = BasicBlock {
            id: BasicBlockId(2),
            statements: Vec::new(),
            terminator: return_terminator(),
        };
        push_stmt(
            &mut bb2,
            StatementKind::Assign(
                Place::Local(SlotId(2)),
                Rvalue::Aggregate(vec![Operand::Constant(MirConstant::Int(2))]),
            ),
            2,
        );
        push_stmt(
            &mut bb2,
            StatementKind::Assign(
                Place::Local(SlotId(0)),
                Rvalue::Use(Operand::Move(Place::Local(SlotId(2)))),
            ),
            3,
        );

        mir.blocks = vec![bb0, bb1, bb2];

        assert_eq!(
            infer_return_ownership_mode(&mir, &HashMap::new()),
            ReturnOwnershipMode::NewlyOwned
        );
    }

    #[test]
    fn test_route_between_params_meets_to_unknown() {
        // fn route(cond: bool, a: Array<int>, b: Array<int>) -> Array<int> {
        //     if cond { a } else { b }
        // }
        // One branch returns param 1, the other returns param 2 — meet is Unknown.
        let mut mir = empty_mir("route");
        mir.num_locals = 3;
        mir.param_slots = vec![SlotId(0) /* unused placeholder */, SlotId(1), SlotId(2)];
        // Actually, let's not mess with SlotId(0) — use distinct param slots.
        mir.param_slots = vec![SlotId(1), SlotId(2)];
        mir.param_reference_kinds = vec![None, None];
        mir.local_types = vec![
            LocalTypeInfo::NonCopy,
            LocalTypeInfo::NonCopy,
            LocalTypeInfo::NonCopy,
        ];

        let bb0 = BasicBlock {
            id: BasicBlockId(0),
            statements: Vec::new(),
            terminator: Terminator {
                kind: TerminatorKind::SwitchBool {
                    operand: Operand::Constant(MirConstant::Bool(true)),
                    true_bb: BasicBlockId(1),
                    false_bb: BasicBlockId(2),
                },
                span: dummy_span(),
            },
        };

        let mut bb1 = BasicBlock {
            id: BasicBlockId(1),
            statements: Vec::new(),
            terminator: return_terminator(),
        };
        push_stmt(
            &mut bb1,
            StatementKind::Assign(
                Place::Local(SlotId(0)),
                Rvalue::Use(Operand::Copy(Place::Local(SlotId(1)))),
            ),
            0,
        );

        let mut bb2 = BasicBlock {
            id: BasicBlockId(2),
            statements: Vec::new(),
            terminator: return_terminator(),
        };
        push_stmt(
            &mut bb2,
            StatementKind::Assign(
                Place::Local(SlotId(0)),
                Rvalue::Use(Operand::Copy(Place::Local(SlotId(2)))),
            ),
            1,
        );

        mir.blocks = vec![bb0, bb1, bb2];

        assert_eq!(
            infer_return_ownership_mode(&mir, &HashMap::new()),
            ReturnOwnershipMode::Unknown
        );
    }

    #[test]
    fn test_call_return_uses_callee_mode() {
        // fn wrap() -> Array<int> { make() }
        //   bb0: Call(func=make, destination=_1, next=bb1)
        //   bb1: _0 = move _1; return
        let mut mir = empty_mir("wrap");
        mir.num_locals = 2;
        mir.local_types = vec![LocalTypeInfo::NonCopy, LocalTypeInfo::NonCopy];

        let bb0 = BasicBlock {
            id: BasicBlockId(0),
            statements: Vec::new(),
            terminator: Terminator {
                kind: TerminatorKind::Call {
                    func: Operand::Constant(MirConstant::Function("make".into())),
                    args: Vec::new(),
                    destination: Place::Local(SlotId(1)),
                    next: BasicBlockId(1),
                },
                span: dummy_span(),
            },
        };

        let mut bb1 = BasicBlock {
            id: BasicBlockId(1),
            statements: Vec::new(),
            terminator: return_terminator(),
        };
        push_stmt(
            &mut bb1,
            StatementKind::Assign(
                Place::Local(SlotId(0)),
                Rvalue::Use(Operand::Move(Place::Local(SlotId(1)))),
            ),
            0,
        );

        mir.blocks = vec![bb0, bb1];

        let mut callee_modes = HashMap::new();
        callee_modes.insert("make".to_string(), ReturnOwnershipMode::NewlyOwned);

        assert_eq!(
            infer_return_ownership_mode(&mir, &callee_modes),
            ReturnOwnershipMode::NewlyOwned
        );

        // Without the callee in the map, the result degrades to Unknown.
        assert_eq!(
            infer_return_ownership_mode(&mir, &HashMap::new()),
            ReturnOwnershipMode::Unknown
        );
    }

    #[test]
    fn test_no_return_is_unknown() {
        // Diverging function with no Return terminator.
        let mut mir = empty_mir("diverge");
        mir.blocks.push(BasicBlock {
            id: BasicBlockId(0),
            statements: Vec::new(),
            terminator: Terminator {
                kind: TerminatorKind::Unreachable,
                span: dummy_span(),
            },
        });
        assert_eq!(
            infer_return_ownership_mode(&mir, &HashMap::new()),
            ReturnOwnershipMode::Unknown
        );
    }

    #[test]
    fn test_borrow_from_param_is_borrowed_from_param() {
        let mut mir = empty_mir("first");
        mir.num_locals = 2;
        mir.param_slots = vec![SlotId(1)];
        mir.param_reference_kinds = vec![Some(BorrowKind::Shared)];
        mir.local_types = vec![LocalTypeInfo::NonCopy, LocalTypeInfo::NonCopy];

        let mut bb0 = BasicBlock {
            id: BasicBlockId(0),
            statements: Vec::new(),
            terminator: return_terminator(),
        };
        push_stmt(
            &mut bb0,
            StatementKind::Assign(
                Place::Local(SlotId(0)),
                Rvalue::Borrow(BorrowKind::Shared, Place::Local(SlotId(1))),
            ),
            0,
        );
        mir.blocks.push(bb0);

        assert_eq!(
            infer_return_ownership_mode(&mir, &HashMap::new()),
            ReturnOwnershipMode::BorrowedFromParam(0)
        );
    }

    #[test]
    fn test_meet_identical_modes() {
        assert_eq!(
            ReturnOwnershipMode::NewlyOwned.meet(ReturnOwnershipMode::NewlyOwned),
            ReturnOwnershipMode::NewlyOwned
        );
        assert_eq!(
            ReturnOwnershipMode::BorrowedFromParam(1).meet(ReturnOwnershipMode::BorrowedFromParam(1)),
            ReturnOwnershipMode::BorrowedFromParam(1)
        );
    }

    #[test]
    fn test_meet_mismatch_collapses_to_unknown() {
        assert_eq!(
            ReturnOwnershipMode::NewlyOwned.meet(ReturnOwnershipMode::BorrowedFromParam(0)),
            ReturnOwnershipMode::Unknown
        );
        assert_eq!(
            ReturnOwnershipMode::BorrowedFromParam(0).meet(ReturnOwnershipMode::BorrowedFromParam(1)),
            ReturnOwnershipMode::Unknown
        );
    }

    #[test]
    fn test_default_is_unknown() {
        assert_eq!(ReturnOwnershipMode::default(), ReturnOwnershipMode::Unknown);
    }

    // ---------------------------------------------------------------------
    // V0.b: Shared / Static emission (diagnostic only).
    // ---------------------------------------------------------------------

    #[test]
    fn test_string_literal_return_is_static() {
        // fn static_string() -> string { "hello" }
        //   bb0:
        //     _0 = use const Str("hello")
        //     return
        let mut mir = empty_mir("static_string");
        mir.local_types = vec![LocalTypeInfo::NonCopy];
        let mut bb0 = BasicBlock {
            id: BasicBlockId(0),
            statements: Vec::new(),
            terminator: return_terminator(),
        };
        push_stmt(
            &mut bb0,
            StatementKind::Assign(
                Place::Local(SlotId(0)),
                Rvalue::Use(Operand::Constant(MirConstant::Str("hello".into()))),
            ),
            0,
        );
        mir.blocks.push(bb0);
        assert_eq!(
            infer_return_ownership_mode(&mir, &HashMap::new()),
            ReturnOwnershipMode::Static
        );
    }

    #[test]
    fn test_interned_string_id_return_is_static() {
        // Same as above but via StringId (legacy interned form).
        let mut mir = empty_mir("interned");
        mir.local_types = vec![LocalTypeInfo::NonCopy];
        let mut bb0 = BasicBlock {
            id: BasicBlockId(0),
            statements: Vec::new(),
            terminator: return_terminator(),
        };
        push_stmt(
            &mut bb0,
            StatementKind::Assign(
                Place::Local(SlotId(0)),
                Rvalue::Use(Operand::Constant(MirConstant::StringId(7))),
            ),
            0,
        );
        mir.blocks.push(bb0);
        assert_eq!(
            infer_return_ownership_mode(&mir, &HashMap::new()),
            ReturnOwnershipMode::Static
        );
    }

    #[test]
    fn test_function_ref_return_is_static() {
        // A bare `fn foo` reference has static lifetime.
        let mut mir = empty_mir("returns_fn");
        mir.local_types = vec![LocalTypeInfo::NonCopy];
        let mut bb0 = BasicBlock {
            id: BasicBlockId(0),
            statements: Vec::new(),
            terminator: return_terminator(),
        };
        push_stmt(
            &mut bb0,
            StatementKind::Assign(
                Place::Local(SlotId(0)),
                Rvalue::Use(Operand::Constant(MirConstant::Function("foo".into()))),
            ),
            0,
        );
        mir.blocks.push(bb0);
        assert_eq!(
            infer_return_ownership_mode(&mir, &HashMap::new()),
            ReturnOwnershipMode::Static
        );
    }

    #[test]
    fn test_static_string_via_temp_is_static() {
        // Tracing a temp all the way back to a Str constant still yields Static.
        //   bb0:
        //     _1 = use const Str("greet")
        //     _0 = move _1
        //     return
        let mut mir = empty_mir("static_via_temp");
        mir.num_locals = 2;
        mir.local_types = vec![LocalTypeInfo::NonCopy, LocalTypeInfo::NonCopy];
        let mut bb0 = BasicBlock {
            id: BasicBlockId(0),
            statements: Vec::new(),
            terminator: return_terminator(),
        };
        push_stmt(
            &mut bb0,
            StatementKind::Assign(
                Place::Local(SlotId(1)),
                Rvalue::Use(Operand::Constant(MirConstant::Str("greet".into()))),
            ),
            0,
        );
        push_stmt(
            &mut bb0,
            StatementKind::Assign(
                Place::Local(SlotId(0)),
                Rvalue::Use(Operand::Move(Place::Local(SlotId(1)))),
            ),
            1,
        );
        mir.blocks.push(bb0);
        assert_eq!(
            infer_return_ownership_mode(&mir, &HashMap::new()),
            ReturnOwnershipMode::Static
        );
    }

    #[test]
    fn test_call_returning_shared_propagates_shared() {
        // fn wrap_arc() -> Arc<T> { arc_new() }   // conceptually
        //   bb0: Call(func=arc_new, dest=_1, next=bb1)
        //   bb1: _0 = move _1; return
        //
        // With `arc_new` registered as Shared in callee_modes, `wrap_arc`
        // inherits Shared.
        let mut mir = empty_mir("wrap_arc");
        mir.num_locals = 2;
        mir.local_types = vec![LocalTypeInfo::NonCopy, LocalTypeInfo::NonCopy];

        let bb0 = BasicBlock {
            id: BasicBlockId(0),
            statements: Vec::new(),
            terminator: Terminator {
                kind: TerminatorKind::Call {
                    func: Operand::Constant(MirConstant::Function("arc_new".into())),
                    args: Vec::new(),
                    destination: Place::Local(SlotId(1)),
                    next: BasicBlockId(1),
                },
                span: dummy_span(),
            },
        };
        let mut bb1 = BasicBlock {
            id: BasicBlockId(1),
            statements: Vec::new(),
            terminator: return_terminator(),
        };
        push_stmt(
            &mut bb1,
            StatementKind::Assign(
                Place::Local(SlotId(0)),
                Rvalue::Use(Operand::Move(Place::Local(SlotId(1)))),
            ),
            0,
        );
        mir.blocks = vec![bb0, bb1];

        let mut callee_modes = HashMap::new();
        callee_modes.insert("arc_new".to_string(), ReturnOwnershipMode::Shared);

        assert_eq!(
            infer_return_ownership_mode(&mir, &callee_modes),
            ReturnOwnershipMode::Shared
        );
    }

    #[test]
    fn test_call_returning_static_propagates_static() {
        // A callee classified as Static flows through a simple wrapper.
        let mut mir = empty_mir("wrap_singleton");
        mir.num_locals = 2;
        mir.local_types = vec![LocalTypeInfo::NonCopy, LocalTypeInfo::NonCopy];

        let bb0 = BasicBlock {
            id: BasicBlockId(0),
            statements: Vec::new(),
            terminator: Terminator {
                kind: TerminatorKind::Call {
                    func: Operand::Constant(MirConstant::Function("get_singleton".into())),
                    args: Vec::new(),
                    destination: Place::Local(SlotId(1)),
                    next: BasicBlockId(1),
                },
                span: dummy_span(),
            },
        };
        let mut bb1 = BasicBlock {
            id: BasicBlockId(1),
            statements: Vec::new(),
            terminator: return_terminator(),
        };
        push_stmt(
            &mut bb1,
            StatementKind::Assign(
                Place::Local(SlotId(0)),
                Rvalue::Use(Operand::Move(Place::Local(SlotId(1)))),
            ),
            0,
        );
        mir.blocks = vec![bb0, bb1];

        let mut callee_modes = HashMap::new();
        callee_modes.insert(
            "get_singleton".to_string(),
            ReturnOwnershipMode::Static,
        );

        assert_eq!(
            infer_return_ownership_mode(&mir, &callee_modes),
            ReturnOwnershipMode::Static
        );
    }

    #[test]
    fn test_arc_param_passthrough_stays_borrowed_not_shared() {
        // fn make_shared(x: Arc<T>) -> Arc<T> { x }
        // Returning an `Arc`-typed parameter is still BorrowedFromParam —
        // the caller already owns the Arc, and the "borrowed-from-param"
        // semantics take precedence over the Shared classification.
        let mut mir = empty_mir("passthrough_arc");
        mir.num_locals = 2;
        mir.param_slots = vec![SlotId(1)];
        mir.param_reference_kinds = vec![None];
        mir.local_types = vec![LocalTypeInfo::NonCopy, LocalTypeInfo::NonCopy];

        let mut bb0 = BasicBlock {
            id: BasicBlockId(0),
            statements: Vec::new(),
            terminator: return_terminator(),
        };
        push_stmt(
            &mut bb0,
            StatementKind::Assign(
                Place::Local(SlotId(0)),
                Rvalue::Use(Operand::Copy(Place::Local(SlotId(1)))),
            ),
            0,
        );
        mir.blocks.push(bb0);

        assert_eq!(
            infer_return_ownership_mode(&mir, &HashMap::new()),
            ReturnOwnershipMode::BorrowedFromParam(0)
        );
    }

    #[test]
    fn test_static_and_newly_owned_branches_meet_to_unknown() {
        // Two branches: one returns a string literal (Static), the other
        // returns an aggregate (NewlyOwned). The meet must collapse to
        // Unknown — the conservative fallback.
        //   bb0: switchBool(cond, bb1, bb2)
        //   bb1: _0 = const Str("hi"); return            -> Static
        //   bb2: _1 = [1]; _0 = move _1; return          -> NewlyOwned
        let mut mir = empty_mir("mixed");
        mir.num_locals = 3;
        mir.param_slots = vec![SlotId(2)];
        mir.param_reference_kinds = vec![None];
        mir.local_types = vec![
            LocalTypeInfo::NonCopy,
            LocalTypeInfo::NonCopy,
            LocalTypeInfo::Copy,
        ];

        let bb0 = BasicBlock {
            id: BasicBlockId(0),
            statements: Vec::new(),
            terminator: Terminator {
                kind: TerminatorKind::SwitchBool {
                    operand: Operand::Copy(Place::Local(SlotId(2))),
                    true_bb: BasicBlockId(1),
                    false_bb: BasicBlockId(2),
                },
                span: dummy_span(),
            },
        };

        let mut bb1 = BasicBlock {
            id: BasicBlockId(1),
            statements: Vec::new(),
            terminator: return_terminator(),
        };
        push_stmt(
            &mut bb1,
            StatementKind::Assign(
                Place::Local(SlotId(0)),
                Rvalue::Use(Operand::Constant(MirConstant::Str("hi".into()))),
            ),
            0,
        );

        let mut bb2 = BasicBlock {
            id: BasicBlockId(2),
            statements: Vec::new(),
            terminator: return_terminator(),
        };
        push_stmt(
            &mut bb2,
            StatementKind::Assign(
                Place::Local(SlotId(1)),
                Rvalue::Aggregate(vec![Operand::Constant(MirConstant::Int(1))]),
            ),
            1,
        );
        push_stmt(
            &mut bb2,
            StatementKind::Assign(
                Place::Local(SlotId(0)),
                Rvalue::Use(Operand::Move(Place::Local(SlotId(1)))),
            ),
            2,
        );

        mir.blocks = vec![bb0, bb1, bb2];

        assert_eq!(
            infer_return_ownership_mode(&mir, &HashMap::new()),
            ReturnOwnershipMode::Unknown
        );
    }

    #[test]
    fn test_two_static_branches_meet_to_static() {
        // Both branches return string literals — the meet stays Static.
        let mut mir = empty_mir("pick_word");
        mir.num_locals = 2;
        mir.param_slots = vec![SlotId(1)];
        mir.param_reference_kinds = vec![None];
        mir.local_types = vec![LocalTypeInfo::NonCopy, LocalTypeInfo::Copy];

        let bb0 = BasicBlock {
            id: BasicBlockId(0),
            statements: Vec::new(),
            terminator: Terminator {
                kind: TerminatorKind::SwitchBool {
                    operand: Operand::Copy(Place::Local(SlotId(1))),
                    true_bb: BasicBlockId(1),
                    false_bb: BasicBlockId(2),
                },
                span: dummy_span(),
            },
        };
        let mut bb1 = BasicBlock {
            id: BasicBlockId(1),
            statements: Vec::new(),
            terminator: return_terminator(),
        };
        push_stmt(
            &mut bb1,
            StatementKind::Assign(
                Place::Local(SlotId(0)),
                Rvalue::Use(Operand::Constant(MirConstant::Str("yes".into()))),
            ),
            0,
        );
        let mut bb2 = BasicBlock {
            id: BasicBlockId(2),
            statements: Vec::new(),
            terminator: return_terminator(),
        };
        push_stmt(
            &mut bb2,
            StatementKind::Assign(
                Place::Local(SlotId(0)),
                Rvalue::Use(Operand::Constant(MirConstant::Str("no".into()))),
            ),
            1,
        );
        mir.blocks = vec![bb0, bb1, bb2];

        assert_eq!(
            infer_return_ownership_mode(&mir, &HashMap::new()),
            ReturnOwnershipMode::Static
        );
    }

    #[test]
    fn test_meet_shared_and_static_collapses_to_unknown() {
        // Lattice spot-checks for the new variants.
        assert_eq!(
            ReturnOwnershipMode::Shared.meet(ReturnOwnershipMode::Shared),
            ReturnOwnershipMode::Shared
        );
        assert_eq!(
            ReturnOwnershipMode::Static.meet(ReturnOwnershipMode::Static),
            ReturnOwnershipMode::Static
        );
        assert_eq!(
            ReturnOwnershipMode::Shared.meet(ReturnOwnershipMode::Static),
            ReturnOwnershipMode::Unknown
        );
        assert_eq!(
            ReturnOwnershipMode::Static.meet(ReturnOwnershipMode::NewlyOwned),
            ReturnOwnershipMode::Unknown
        );
        assert_eq!(
            ReturnOwnershipMode::Shared.meet(ReturnOwnershipMode::NewlyOwned),
            ReturnOwnershipMode::Unknown
        );
    }

    #[test]
    fn test_int_conditional_return_stays_newly_owned() {
        // fn return_unknown(x: bool) -> int { if x { 1 } else { 2 } }
        // Two branches, both int constants, both NewlyOwned under the new
        // classification: the meet stays NewlyOwned — per the task's
        // "classification per existing rules (likely NewlyOwned for int)".
        let mut mir = empty_mir("return_unknown");
        mir.num_locals = 2;
        mir.param_slots = vec![SlotId(1)];
        mir.param_reference_kinds = vec![None];
        mir.local_types = vec![LocalTypeInfo::Copy, LocalTypeInfo::Copy];

        let bb0 = BasicBlock {
            id: BasicBlockId(0),
            statements: Vec::new(),
            terminator: Terminator {
                kind: TerminatorKind::SwitchBool {
                    operand: Operand::Copy(Place::Local(SlotId(1))),
                    true_bb: BasicBlockId(1),
                    false_bb: BasicBlockId(2),
                },
                span: dummy_span(),
            },
        };
        let mut bb1 = BasicBlock {
            id: BasicBlockId(1),
            statements: Vec::new(),
            terminator: return_terminator(),
        };
        push_stmt(
            &mut bb1,
            StatementKind::Assign(
                Place::Local(SlotId(0)),
                Rvalue::Use(Operand::Constant(MirConstant::Int(1))),
            ),
            0,
        );
        let mut bb2 = BasicBlock {
            id: BasicBlockId(2),
            statements: Vec::new(),
            terminator: return_terminator(),
        };
        push_stmt(
            &mut bb2,
            StatementKind::Assign(
                Place::Local(SlotId(0)),
                Rvalue::Use(Operand::Constant(MirConstant::Int(2))),
            ),
            1,
        );
        mir.blocks = vec![bb0, bb1, bb2];

        assert_eq!(
            infer_return_ownership_mode(&mir, &HashMap::new()),
            ReturnOwnershipMode::NewlyOwned
        );
    }

    // ---------------------------------------------------------------------
    // End-to-end: parse Shape source, lower to MIR, run inference.
    // These cover the real lowering shapes (ArrayStore, implicit return,
    // etc.) rather than hand-built MIR.
    // ---------------------------------------------------------------------

    fn infer_from_source(
        code: &str,
    ) -> std::collections::HashMap<String, ReturnOwnershipMode> {
        use shape_ast::ast::Item;
        let program = shape_ast::parser::parse_program(code).expect("parse failed");
        let mut modes = std::collections::HashMap::new();
        for item in &program.items {
            if let Item::Function(def, _) = item {
                let lowering = crate::mir::lowering::lower_function_detailed(
                    &def.name,
                    &def.params,
                    &def.body,
                    def.name_span,
                );
                let mode = infer_return_ownership_mode(&lowering.mir, &modes);
                modes.insert(def.name.clone(), mode);
            }
        }
        modes
    }

    fn mode_of(
        modes: &std::collections::HashMap<String, ReturnOwnershipMode>,
        name: &str,
    ) -> ReturnOwnershipMode {
        modes.get(name).copied().unwrap_or(ReturnOwnershipMode::Unknown)
    }

    #[test]
    fn test_source_array_literal_return_is_newly_owned() {
        let modes = infer_from_source("fn make() -> Array<int> { [1, 2, 3] }");
        assert_eq!(mode_of(&modes, "make"), ReturnOwnershipMode::NewlyOwned);
    }

    #[test]
    fn test_source_int_constant_return_is_newly_owned() {
        let modes = infer_from_source("fn answer() -> int { 42 }");
        assert_eq!(mode_of(&modes, "answer"), ReturnOwnershipMode::NewlyOwned);
    }

    #[test]
    fn test_source_arith_return_is_newly_owned() {
        let modes = infer_from_source("fn add(a: int, b: int) -> int { a + b }");
        assert_eq!(mode_of(&modes, "add"), ReturnOwnershipMode::NewlyOwned);
    }

    #[test]
    fn test_source_passthrough_is_borrowed_from_param() {
        let modes = infer_from_source("fn pass(x: Array<int>) -> Array<int> { x }");
        assert_eq!(
            mode_of(&modes, "pass"),
            ReturnOwnershipMode::BorrowedFromParam(0)
        );
    }

    #[test]
    fn test_source_if_both_branches_allocate_is_newly_owned() {
        let modes = infer_from_source(
            "fn choose(cond: bool) -> Array<int> { if cond { [1] } else { [2] } }",
        );
        assert_eq!(mode_of(&modes, "choose"), ReturnOwnershipMode::NewlyOwned);
    }

    #[test]
    fn test_source_route_between_params_meets_to_unknown() {
        let modes = infer_from_source(
            "fn route(cond: bool, a: Array<int>, b: Array<int>) -> Array<int> { if cond { a } else { b } }",
        );
        assert_eq!(mode_of(&modes, "route"), ReturnOwnershipMode::Unknown);
    }

    #[test]
    fn test_source_call_through_inherits_callee_mode() {
        let modes = infer_from_source(
            r#"
            fn make() -> Array<int> { [1, 2, 3] }
            fn wrap() -> Array<int> { make() }
            "#,
        );
        assert_eq!(mode_of(&modes, "make"), ReturnOwnershipMode::NewlyOwned);
        assert_eq!(mode_of(&modes, "wrap"), ReturnOwnershipMode::NewlyOwned);
    }

    #[test]
    fn test_source_three_stage_pipeline_propagates() {
        let modes = infer_from_source(
            r#"
            fn a() -> Array<int> { [1, 2, 3] }
            fn b() -> Array<int> { a() }
            fn c() -> Array<int> { b() }
            "#,
        );
        assert_eq!(mode_of(&modes, "a"), ReturnOwnershipMode::NewlyOwned);
        assert_eq!(mode_of(&modes, "b"), ReturnOwnershipMode::NewlyOwned);
        assert_eq!(mode_of(&modes, "c"), ReturnOwnershipMode::NewlyOwned);
    }

    #[test]
    fn test_source_call_without_callee_info_is_unknown() {
        // Without `make` seen first, `wrap`'s Call resolves to Unknown.
        // This asserts the conservative fallback — safe to return Arc.
        let modes = infer_from_source("fn wrap() -> Array<int> { external() }");
        // `external` is never defined here; Call has no callee info.
        assert_eq!(mode_of(&modes, "wrap"), ReturnOwnershipMode::Unknown);
    }

    #[test]
    fn test_source_string_literal_return_is_static() {
        let modes = infer_from_source(r#"fn greet() -> string { "hello" }"#);
        assert_eq!(mode_of(&modes, "greet"), ReturnOwnershipMode::Static);
    }

    #[test]
    fn test_source_object_literal_return_is_newly_owned() {
        let modes = infer_from_source(
            r#"
            type Pair { a: int, b: int }
            fn make_pair() -> Pair { Pair { a: 1, b: 2 } }
            "#,
        );
        assert_eq!(
            mode_of(&modes, "make_pair"),
            ReturnOwnershipMode::NewlyOwned
        );
    }
}
