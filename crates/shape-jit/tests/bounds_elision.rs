//! Integration-level tests for the JIT bounds-check elision wireup.
//!
//! These tests validate the analyzer's structural detection of trusted
//! `arr[iv]` accesses against synthetic MIR shapes. They live at the
//! integration-test level so they exercise the public `bounds_elision`
//! API exactly as it appears in `compile_strategy` / `program.rs` /
//! `strategy.rs`, providing a guard against accidental loss of detection
//! during future MIR codegen refactors.
//!
//! End-to-end JIT-execution tests of programs that exercise the elision
//! path are pending. They are blocked on a pre-existing JIT issue
//! unrelated to the elision work: bytecode verification rejects
//! `JumpIfFalseTrusted` opcodes whose operand slot is `Unknown` for some
//! stdlib functions (`std::core::math::clamp`, `sign`,
//! `coefficient_of_variation`). Those failures reproduce on the
//! `jit-v2-phase1` baseline before this change. Once that JIT issue
//! lands a fix, the `JITExecutor::execute_program`-based tests can be
//! added here without further wireup.
//!
//! IR text inspection (asserting `brif` is absent in the loop body) is
//! intentionally NOT done here. The current MirToIR compile API does not
//! expose post-compile Cranelift IR text in a way that can be filtered
//! to a single function body, and the v2_array_tests "Out-of-bounds
//! access" test relies on the bounds-checked path remaining reachable
//! for non-trusted accesses. The behavioural unit tests inside
//! `mir_compiler::bounds_elision::tests` pin both the trusted and
//! rejected branches of the analyzer.

use shape_ast::ast::Span;
use shape_jit::mir_compiler::bounds_elision;
use shape_vm::mir::types::{
    BasicBlock, BasicBlockId, BinOp, FieldIdx, LocalTypeInfo, MirConstant, MirFunction,
    MirStatement, Operand, Place, Point, Rvalue, SlotId, StatementKind, Terminator,
    TerminatorKind,
};

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

/// Build a MIR function shaped like
///
/// ```text
///   bb0:
///     bnd = arr.length        // arr is param slot
///     iv  = 0
///     goto bb1
///   bb1:
///     cond = iv < bnd
///     SwitchBool(cond, bb2, bb3)
///   bb2:
///     sink = arr[iv]
///     iv = iv + 1
///     goto bb1
///   bb3:
///     return
/// ```
fn build_canonical_for_loop_mir(arr: SlotId, iv: SlotId, bnd: SlotId, cond: SlotId) -> MirFunction {
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
        name: "synthetic_test".to_string(),
        blocks: vec![bb0, bb1, bb2, bb3],
        num_locals: 100,
        param_slots: vec![arr],
        param_reference_kinds: vec![None],
        local_types: (0..100).map(|_| LocalTypeInfo::Unknown).collect(),
        span: Span { start: 0, end: 0 },
        field_name_table,
        local_struct_type_names: std::collections::HashMap::new(),
    }
}

/// Test 1: the simplest possible canonical pattern.
#[test]
fn canonical_for_loop_marks_pair_trusted() {
    let arr = SlotId(1);
    let iv = SlotId(2);
    let bnd = SlotId(3);
    let cond = SlotId(4);

    let mir = build_canonical_for_loop_mir(arr, iv, bnd, cond);
    let plan = bounds_elision::analyze(&mir);
    assert!(
        plan.is_trusted(arr, iv),
        "expected (arr={:?}, iv={:?}) trusted; got {:?}",
        arr,
        iv,
        plan.trusted_pairs,
    );
}

/// Test 2: array reassignment in the body invalidates the trust pair.
/// Models the `a = a.push(...)` pattern that appears throughout Shape
/// stdlib code: each push produces a freshly-typed array, and the slot's
/// pointer changes. Without rejecting this case, the cached length bound
/// would no longer correspond to the current array.
#[test]
fn array_reassignment_in_body_rejects_pair() {
    let arr = SlotId(1);
    let iv = SlotId(2);
    let bnd = SlotId(3);
    let cond = SlotId(4);

    let mut mir = build_canonical_for_loop_mir(arr, iv, bnd, cond);
    // Inject `arr = sink` inside the body block (bb2). With `arr` being
    // a param slot the allowed assign count is 0; one inline reassignment
    // pushes the count over the threshold.
    let sink = SlotId(99);
    let bb2 = mir
        .blocks
        .iter_mut()
        .find(|b| b.id == BasicBlockId(2))
        .unwrap();
    bb2.statements.insert(
        0,
        s(StatementKind::Assign(
            Place::Local(arr),
            Rvalue::Use(Operand::Copy(Place::Local(sink))),
        )),
    );
    let plan = bounds_elision::analyze(&mir);
    assert!(
        !plan.is_trusted(arr, iv),
        "expected access not to be trusted when arr is reassigned in the body",
    );
}

/// Test 3: when the bound source is a different array than the one
/// indexed in the body, the analyzer must NOT trust the access.
#[test]
fn bound_from_different_array_rejects_pair() {
    let arr = SlotId(1);
    let other_arr = SlotId(5);
    let iv = SlotId(2);
    let bnd = SlotId(3);
    let cond = SlotId(4);

    let mut mir = build_canonical_for_loop_mir(arr, iv, bnd, cond);
    // Rewrite bb0 so bnd captures `other_arr.length` rather than
    // `arr.length`. Push a stand-in Assign for other_arr to keep the
    // (other_arr) slot count consistent — though the analyzer keys on
    // the (arr_slot, iv_slot) tuple regardless.
    mir.param_slots.push(other_arr); // other_arr is also a param
    mir.param_reference_kinds.push(None);
    let length_idx = FieldIdx(7);
    let bb0 = mir.blocks.iter_mut().find(|b| b.id == BasicBlockId(0)).unwrap();
    bb0.statements[0] = s(StatementKind::Assign(
        Place::Local(bnd),
        Rvalue::Use(Operand::Copy(Place::Field(
            Box::new(Place::Local(other_arr)),
            length_idx,
        ))),
    ));
    let plan = bounds_elision::analyze(&mir);
    assert!(
        !plan.is_trusted(arr, iv),
        "expected access through arr to be rejected when bound came from other_arr",
    );
    // The (other_arr, iv) pair is, however, valid — the bound came from it,
    // and there are no accesses against it in the body. Either outcome is
    // acceptable here; the analyzer's correctness only requires that
    // (arr, iv) NOT be trusted.
}
