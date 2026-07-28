//! ADR-018 §3 (#190) — dominated retain/release pair cancellation.
//!
//! Every test here is a *differential*: it runs the same source with the
//! elision on and off and compares. That is the shape ADR-018 §3's correctness
//! gate demands, because the property at issue ("the pass changes nothing an
//! observer can see, except how many refcount operations run") is not
//! expressible as a single-sided assertion.
//!
//! The toggle is `compiler::helpers::with_rc_elision_flag`. It exists for
//! these tests; the shipped default is on and no language surface reads it.

use crate::bytecode::{BytecodeProgram, OpCode, Operand};
use crate::compiler::helpers::{with_ownership_moves_flag, with_rc_elision_flag};
use crate::executor::tests::test_utils::{compile, eval};
use shape_value::{KindedSlot, NativeKind};

// ── Helpers ──────────────────────────────────────────────────────────

fn opcode_count(bc: &BytecodeProgram, op: OpCode) -> usize {
    bc.instructions.iter().filter(|i| i.opcode == op).count()
}

/// Every emitted release of a local's own share: the ownership-aware
/// `DropLocal` and the legacy `LoadLocal` + `DropCall` pass. Which of the two
/// a binding gets depends on its storage class, so a test that wants to say
/// "a release was cancelled" has to count both.
fn release_count(bc: &BytecodeProgram) -> usize {
    opcode_count(bc, OpCode::DropLocal) + opcode_count(bc, OpCode::DropCall)
}

fn local_operand_slots(bc: &BytecodeProgram, op: OpCode) -> Vec<u16> {
    bc.instructions
        .iter()
        .filter(|i| i.opcode == op)
        .filter_map(|i| match i.operand {
            Some(Operand::Local(slot)) => Some(slot),
            _ => None,
        })
        .collect()
}

/// Compile `source` with the elision forced on and off, returning both.
fn compile_both(source: &str) -> (BytecodeProgram, BytecodeProgram) {
    let on = with_rc_elision_flag(true, || compile(source));
    let off = with_rc_elision_flag(false, || compile(source));
    (on, off)
}

/// Evaluate `source` with the elision forced on and off, returning both.
fn eval_both(source: &str) -> (KindedSlot, KindedSlot) {
    let on = with_rc_elision_flag(true, || eval(source));
    let off = with_rc_elision_flag(false, || eval(source));
    (on, off)
}

fn assert_same_i64(source: &str) {
    let (on, off) = eval_both(source);
    // A fixture whose program value is not an int would compare `None` to
    // `None` and prove nothing; every caller must yield its result.
    assert!(
        on.as_i64().is_some(),
        "fixture produced no int result, so the comparison is vacuous:\n{source}"
    );
    assert_eq!(
        on.as_i64(),
        off.as_i64(),
        "elision changed the result of:\n{source}"
    );
}

/// Read the refcount out of a returned v2 header carrier. Returns `None` when
/// the value is not a header-carrying heap pointer.
fn header_refcount(slot: &KindedSlot) -> Option<u32> {
    let NativeKind::Ptr(kind) = slot.kind() else {
        return None;
    };
    if !shape_value::gc::is_header_carrier(kind) {
        return None;
    }
    let bits = slot.slot().raw();
    if bits == 0 {
        return None;
    }
    // SAFETY: the slot is the live program result; a header carrier's first
    // eight bytes are its `HeapHeader`.
    Some(unsafe { (*(bits as *const shape_value::V2HeapHeader)).get_refcount() })
}

// ── The cancellation itself ──────────────────────────────────────────

/// A `let`-bound owned heap local carries the full retain/release shape:
/// `CloneLocal` on the read, `DropLocal` at scope exit, and the legacy
/// `LoadLocal` + `DropCall` pass behind it. A single terminal read cancels all
/// three — the read takes the slot's share rather than minting a second one,
/// and nothing downstream may re-read the moved-out slot.
#[test]
fn a_terminal_read_cancels_its_retain_release_pair() {
    let source = r#"
fn take(xs: Array<int>) -> int { xs.len() }
fn f() -> int {
    let xs = [1, 2, 3]
    return take(xs)
}
let r = f()
r
"#;
    let (on, off) = compile_both(source);

    assert!(
        opcode_count(&on, OpCode::LoadLocalMove) > opcode_count(&off, OpCode::LoadLocalMove),
        "the terminal read must become a move with the pass on"
    );
    assert!(
        opcode_count(&on, OpCode::CloneLocal) < opcode_count(&off, OpCode::CloneLocal),
        "the retain must be gone with the pass on"
    );
    assert!(
        opcode_count(&on, OpCode::DropLocal) < opcode_count(&off, OpCode::DropLocal),
        "the paired release must be gone with the pass on"
    );

    // The pair cancels as a pair: the slot that stopped being retained is
    // exactly the slot that stopped being released.
    let moved: Vec<u16> = local_operand_slots(&on, OpCode::LoadLocalMove);
    let dropped_off: Vec<u16> = local_operand_slots(&off, OpCode::DropLocal);
    let dropped_on: Vec<u16> = local_operand_slots(&on, OpCode::DropLocal);
    for slot in &moved {
        assert!(
            dropped_off.contains(slot),
            "slot {slot} moved with the pass on but had no release with it off — \
             that is a cancelled release with no retain to cancel against"
        );
        assert!(
            !dropped_on.contains(slot),
            "slot {slot} was both moved and released with the pass on — double release"
        );
    }

    assert_same_i64(source);
}

/// #181 TRIPWIRE 3, now assertable in its literal form. That ticket had to
/// settle for "the read is ownership-aware and not cell-pinned" because
/// `LoadLocalMove` was unreachable for a heap-carrying binding: the V1.1C
/// shortcut in `emit_load_local_owned` returned `CloneLocal` before the move
/// path was consulted. This ticket landed the move path, so the tripwire can
/// now name the opcode it always meant.
#[test]
fn a_qualifying_var_read_compiles_to_load_local_move() {
    let bc = compile(
        r#"
fn take(xs: Array<int>) -> int { xs.len() }
fn f() -> int {
    var xs = [1, 2, 3]
    take(xs)
}
let r = f()
r
"#,
    );
    let f = bc
        .functions
        .iter()
        .find(|func| func.name.ends_with("f"))
        .expect("function f present");
    let body = &bc.instructions[f.entry_point..f.entry_point + f.body_length];
    let ops: Vec<OpCode> = body.iter().map(|i| i.opcode).collect();

    assert!(
        ops.contains(&OpCode::LoadLocalMove),
        "the `var xs` read must take the slot's share; got {ops:?}"
    );
    assert!(
        !ops.iter()
            .any(|op| matches!(op, OpCode::LoadClosure | OpCode::LoadSharedLocal)),
        "a `var` that is never captured or aliased must not be read through a \
         shared cell — that is the pinning the retired force-SharedCow gate \
         caused; got {ops:?}"
    );
}

/// Nothing may read a slot after the move took its share. This is the
/// invariant that makes the cancellation a cancellation rather than a
/// half-applied edit: the compiler-injected scope-exit passes
/// (`DropLocal`, `LoadLocal` + `DropCall`) are not modelled in MIR, so the
/// "single read" proof does not cover them and they must be suppressed
/// explicitly.
#[test]
fn no_slot_is_read_after_its_move() {
    for source in [
        r#"
fn take(xs: Array<int>) -> int { xs.len() }
fn f() -> int {
    let xs = [1, 2, 3]
    return take(xs)
}
let r = f()
r
"#,
        r#"
fn take(xs: Array<int>) -> int { xs.len() }
fn f() -> int {
    var xs = [1, 2, 3]
    take(xs)
}
let r = f()
r
"#,
        r#"
fn build() -> Array<int> {
    let a = [1, 2, 3, 4]
    return a
}
let r = build()
r
"#,
    ] {
        let bc = with_rc_elision_flag(true, || compile(source));
        for func in &bc.functions {
            let body = &bc.instructions[func.entry_point..func.entry_point + func.body_length];
            let mut moved: Vec<u16> = Vec::new();
            for inst in body {
                let Some(Operand::Local(slot)) = inst.operand else {
                    continue;
                };
                if inst.opcode == OpCode::LoadLocalMove {
                    moved.push(slot);
                    continue;
                }
                assert!(
                    !moved.contains(&slot),
                    "slot {slot} is read by {:?} after its LoadLocalMove in {}:\n{source}",
                    inst.opcode,
                    func.name
                );
            }
        }
    }
}

/// A binding declared *inside* a loop body and consumed in the same iteration
/// elides, even though its move sits on a cycle and dominates no exit. This is
/// the shape allocation-heavy code is made of (`while r < n { let g =
/// build(); use(g) }`), and the reason the legality condition is
/// post-domination of the definition rather than domination of the exits.
#[test]
fn a_binding_declared_inside_a_loop_body_elides() {
    let source = r#"
fn take(xs: Array<int>) -> int { xs.len() }
fn f(rounds: int) -> int {
    var total = 0
    var r = 0
    while r < rounds {
        let g = [1, 2, 3]
        total = total + take(g)
        r = r + 1
    }
    return total
}
let r = f(4)
r
"#;
    let (on, off) = compile_both(source);
    assert!(
        opcode_count(&on, OpCode::LoadLocalMove) > opcode_count(&off, OpCode::LoadLocalMove),
        "the loop-body binding's read must become a move"
    );
    assert!(
        release_count(&on) < release_count(&off),
        "the loop-body binding's per-iteration release must be gone"
    );
    assert_same_i64(source);
    let (on_v, _) = eval_both(source);
    assert_eq!(on_v.as_i64(), Some(12), "4 rounds x len 3");
}

/// The mirror image: a binding declared *outside* a loop and read *inside* it
/// must not elide — one definition, many moves, and the second iteration would
/// read a cleared slot. Liveness carries this: the back edge keeps the slot
/// live after its read.
#[test]
fn a_binding_read_inside_a_loop_but_declared_outside_does_not_elide() {
    let source = r#"
fn take(xs: Array<int>) -> int { xs.len() }
fn f(rounds: int) -> int {
    let g = [1, 2, 3]
    var total = 0
    var r = 0
    while r < rounds {
        total = total + take(g)
        r = r + 1
    }
    return total
}
let r = f(4)
r
"#;
    let (on, off) = compile_both(source);
    let ops_on: Vec<OpCode> = on.instructions.iter().map(|i| i.opcode).collect();
    let ops_off: Vec<OpCode> = off.instructions.iter().map(|i| i.opcode).collect();
    assert_eq!(
        ops_on, ops_off,
        "a many-moves-per-definition read must not elide"
    );
    assert_same_i64(source);
    let (on_v, _) = eval_both(source);
    assert_eq!(on_v.as_i64(), Some(12));
}

/// The elision is a pure emission choice: with the ownership-moves flag off
/// there is no `CloneLocal`/`DropLocal` pair to cancel, and emission must be
/// byte-identical either way.
#[test]
fn with_ownership_moves_off_emission_is_byte_identical() {
    let source = r#"
fn take(xs: Array<int>) -> int { xs.len() }
fn f() -> int {
    var xs = [1, 2, 3]
    take(xs)
}
let r = f()
r
"#;
    let on = with_ownership_moves_flag(false, || with_rc_elision_flag(true, || compile(source)));
    let off = with_ownership_moves_flag(false, || with_rc_elision_flag(false, || compile(source)));
    let ops_on: Vec<OpCode> = on.instructions.iter().map(|i| i.opcode).collect();
    let ops_off: Vec<OpCode> = off.instructions.iter().map(|i| i.opcode).collect();
    assert_eq!(ops_on, ops_off);
}

// ── Refcount balance at the observable boundary ──────────────────────

/// A heap value that survives to the program result carries the same count
/// with the pass on as without it. This is the refcount-balance differential
/// at the one place a test can read a real refcount: the returned carrier.
#[test]
fn the_returned_value_carries_an_identical_refcount() {
    for source in [
        r#"
fn build() -> Array<int> {
    let a = [1, 2, 3, 4]
    return a
}
let r = build()
r
"#,
        r#"
fn wrap(xs: Array<int>) -> Array<int> { xs }
fn build() -> Array<int> {
    var a = [9, 8, 7]
    wrap(a)
}
let r = build()
r
"#,
    ] {
        let (on, off) = eval_both(source);
        assert_eq!(on.kind(), off.kind(), "kind changed for:\n{source}");
        match (header_refcount(&on), header_refcount(&off)) {
            (Some(a), Some(b)) => assert_eq!(
                a, b,
                "elision changed the surviving refcount for:\n{source}"
            ),
            (None, None) => {}
            (a, b) => panic!("carrier shape diverged ({a:?} vs {b:?}) for:\n{source}"),
        }
    }
}

// ── Where the proof does not hold, nothing is cancelled ──────────────

/// Each of these must keep its retain: the MIR plan cannot prove a covering
/// owning reference across the interval. The assertion is that the pass makes
/// no difference at all to the emitted opcodes.
#[test]
fn unprovable_shapes_are_left_alone() {
    let cases: &[(&str, &str)] = &[
        (
            "two reads — the first is not terminal",
            r#"
fn take(xs: Array<int>) -> int { xs.len() }
fn f() -> int {
    var xs = [1, 2, 3]
    take(xs) + take(xs)
}
let r = f()
r
"#,
        ),
        (
            "conditional read — does not dominate the exit",
            r#"
fn take(xs: Array<int>) -> int { xs.len() }
fn f(c: bool) -> int {
    var xs = [1, 2, 3]
    if c {
        return take(xs)
    }
    return 0
}
let r = f(true)
r
"#,
        ),
        (
            "read on a loop back edge — would move twice",
            r#"
fn take(xs: Array<int>) -> int { xs.len() }
fn f() -> int {
    var xs = [1, 2, 3]
    var total = 0
    var i = 0
    while i < 3 {
        total = total + take(xs)
        i = i + 1
    }
    return total
}
let r = f()
r
"#,
        ),
        (
            "borrowed slot — a live reference observes it",
            r#"
fn peek(xs: &Array<int>) -> int { 1 }
fn f() -> int {
    var xs = [1, 2, 3]
    return peek(&xs)
}
let r = f()
r
"#,
        ),
        (
            "captured by a closure — the capture teardown re-reads the slot",
            r#"
fn f() -> int {
    var xs = [1, 2, 3]
    let g = || xs.len()
    return g()
}
let r = f()
r
"#,
        ),
    ];

    for (label, source) in cases {
        let (on, off) = compile_both(source);
        let ops_on: Vec<OpCode> = on.instructions.iter().map(|i| i.opcode).collect();
        let ops_off: Vec<OpCode> = off.instructions.iter().map(|i| i.opcode).collect();
        assert_eq!(
            ops_on, ops_off,
            "the pass changed emission for a shape it cannot prove: {label}"
        );
        assert_same_i64(source);
    }
}

// ── Finalization order is untouched ──────────────────────────────────

/// A type with a user `impl Drop` is released by `DropCall`, which reads the
/// slot to invoke the finalizer. Elision must never reach such a slot: Drop
/// ordering is observable public behaviour, and a moved-out slot would both
/// skip the finalizer and hand it a cleared receiver.
#[test]
fn a_user_drop_type_never_elides_and_keeps_its_order() {
    let source = r#"
type Res { id: int }
impl Drop for Res {
    method drop() { }
}
fn f() -> int {
    let a = Res { id: 1 }
    let b = Res { id: 2 }
    return a.id + b.id
}
let r = f()
r
"#;
    let (on, off) = compile_both(source);
    let ops_on: Vec<OpCode> = on.instructions.iter().map(|i| i.opcode).collect();
    let ops_off: Vec<OpCode> = off.instructions.iter().map(|i| i.opcode).collect();
    assert_eq!(
        ops_on, ops_off,
        "elision must not touch emission around user Drop types"
    );
    assert_eq!(
        opcode_count(&on, OpCode::DropCall),
        opcode_count(&off, OpCode::DropCall),
        "DropCall count must be identical"
    );
    assert_same_i64(source);
}

// ── Execution equivalence across a wider set ─────────────────────────

#[test]
fn elided_programs_produce_identical_results() {
    let cases = [
        r#"
fn sum(xs: Array<int>) -> int {
    var t = 0
    for x in xs { t = t + x }
    return t
}
fn f() -> int {
    let a = [1, 2, 3, 4, 5]
    return sum(a)
}
let r = f()
r
"#,
        r#"
fn f() -> int {
    let s = "hello world"
    return s.length()
}
let r = f()
r
"#,
        r#"
type Point { x: int, y: int }
fn take(p: Point) -> int { p.x + p.y }
fn f() -> int {
    let p = Point { x: 3, y: 4 }
    return take(p)
}
let r = f()
r
"#,
        r#"
fn inner(xs: Array<int>) -> int { xs.len() }
fn middle(xs: Array<int>) -> int {
    let ys = xs
    return inner(ys)
}
fn f() -> int {
    let a = [1, 2, 3]
    return middle(a)
}
let r = f()
r
"#,
    ];
    for source in cases {
        assert_same_i64(source);
    }
}

/// Finalization order, observed directly. Each `Drop` body appends its id to a
/// module-scope log, so the program's value *is* the finalization sequence.
/// ADR-018 §3's gate demands identical order with the pass on and off, and the
/// ticket makes any order change a stop-and-surface rather than a rebaseline —
/// so this compares the sequences literally.
#[test]
fn finalization_order_is_identical_with_the_pass_on_and_off() {
    let cases = [
        // Reverse declaration order within one scope.
        r#"
var log = ""
type R { id: int }
impl Drop for R {
    method drop() { log = log + f"{self.id};" }
}
fn f() -> int {
    let a = R { id: 1 }
    let b = R { id: 2 }
    let c = R { id: 3 }
    return 0
}
let _x = f()
log
"#,
        // Nested scopes: inner finishes before outer.
        r#"
var log = ""
type R { id: int }
impl Drop for R {
    method drop() { log = log + f"{self.id};" }
}
fn f() -> int {
    let outer = R { id: 1 }
    {
        let inner = R { id: 2 }
    }
    return 0
}
let _x = f()
log
"#,
        // Early return from a nested scope.
        r#"
var log = ""
type R { id: int }
impl Drop for R {
    method drop() { log = log + f"{self.id};" }
}
fn f(c: bool) -> int {
    let a = R { id: 1 }
    {
        let b = R { id: 2 }
        if c { return 7 }
    }
    return 0
}
let _x = f(true)
log
"#,
        // A Drop-bearing value alongside plain heap locals that DO elide.
        r#"
var log = ""
type R { id: int }
impl Drop for R {
    method drop() { log = log + f"{self.id};" }
}
fn take(xs: Array<int>) -> int { xs.len() }
fn f() -> int {
    let g = R { id: 1 }
    let xs = [1, 2, 3]
    return take(xs)
}
let _x = f()
log
"#,
    ];

    for source in cases {
        let (on, off) = eval_both(source);
        let on_log = on.as_str().map(String::from).expect("drop log is a string");
        let off_log = off
            .as_str()
            .map(String::from)
            .expect("drop log is a string");
        assert_eq!(
            on_log, off_log,
            "finalization order changed with the pass on:\n{source}"
        );
        assert!(
            !on_log.is_empty(),
            "the fixture produced no finalizations, so it proves nothing:\n{source}"
        );
    }
}

// ── Forced-collection differential (ADR-018 §3 correctness gate) ─────
//
// Built and run only with `--features gc-stress`, which turns every
// instruction boundary in the dispatch loop into a full Bacon-Rajan
// trial-deletion collection. That is the only way to satisfy the ADR's
// requirement that collection be triggered INSIDE each elided interval: the
// production safepoint fires on a 1024-instruction stride and would step over
// intervals a handful of instructions long.
//
// The three cases below are the three interval classes this slice produces —
// the classes are defined by what consumes the moved share, because that is
// what determines where the surviving release lands.
#[cfg(all(feature = "gc", feature = "gc-stress"))]
mod forced_collection {
    use super::*;

    /// Class 1 — the moved share is consumed by a callee frame.
    #[test]
    fn a_share_moved_into_a_call_survives_collection_inside_the_interval() {
        let source = r#"
fn take(xs: Array<int>) -> int { xs.len() }
fn f() -> int {
    let xs = [1, 2, 3, 4, 5]
    return take(xs)
}
let r = f()
r
"#;
        let (on, off) = eval_both(source);
        assert_eq!(on.as_i64(), Some(5));
        assert_eq!(on.as_i64(), off.as_i64());
    }

    /// Class 2 — the moved share becomes the function's return value and
    /// outlives the frame whose slot gave it up.
    #[test]
    fn a_share_moved_into_a_return_survives_collection_inside_the_interval() {
        let source = r#"
fn build() -> Array<int> {
    let a = [10, 20, 30]
    return a
}
fn f() -> int {
    let xs = build()
    return xs[0] + xs[1] + xs[2]
}
let r = f()
r
"#;
        let (on, off) = eval_both(source);
        assert_eq!(on.as_i64(), Some(60));
        assert_eq!(on.as_i64(), off.as_i64());
    }

    /// Class 3 — the moved share is rebound to another local, so the release
    /// that survives is the rebind target's, one scope-exit later.
    #[test]
    fn a_share_moved_into_a_rebind_survives_collection_inside_the_interval() {
        let source = r#"
fn inner(xs: Array<int>) -> int { xs.len() }
fn f() -> int {
    let a = [1, 2, 3, 4]
    let b = a
    return inner(b)
}
let r = f()
r
"#;
        let (on, off) = eval_both(source);
        assert_eq!(on.as_i64(), Some(4));
        assert_eq!(on.as_i64(), off.as_i64());
    }

    /// Cycle-collection completeness with the pass on is asserted by the
    /// `gc_teardown` suite, which builds real closure-capture and
    /// `SharedCell`-rooted cycles with the production allocators and asserts
    /// they are reclaimed. The Shape surface cannot express those topologies
    /// (an untyped `var arr = []` is a strict-typing compile error, which is
    /// what the Finding-#31 fixture needs), so the completeness evidence is
    /// that suite run with the pass on and under this harness — not a
    /// weaker source-level restatement of it here.
    #[allow(dead_code)]
    const CYCLE_COMPLETENESS_EVIDENCE: () = ();
}

// ── Measured dynamic retain/release reduction ────────────────────────

/// The ticket's "no measurement, no close" clause, run against the committed
/// allocation-heavy workloads themselves (#186's charter suite) rather than a
/// hand-written stand-in. Built only with `--features rc-stats`, which is what
/// compiles the counters into `clone_with_kind` / `drop_with_kind`.
///
/// The benchmark files are read, never modified — the workload is whatever
/// `benchmarks/charter/shape/` currently holds.
#[cfg(feature = "rc-stats")]
mod measured {
    use super::*;
    use crate::rc_stats;

    fn run_counted(source: &str, elision: bool) -> (u64, u64, u64) {
        rc_stats::reset();
        let _ = with_rc_elision_flag(elision, || eval(source));
        rc_stats::snapshot()
    }

    #[test]
    fn allocation_heavy_workloads_run_fewer_refcount_operations() {
        let root = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../benchmarks/charter/shape/"
        );
        let mut any_reduction = false;
        for name in ["alloc_object_graph.shape", "alloc_tree.shape"] {
            let path = format!("{root}{name}");
            let source = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("charter workload {path} unreadable: {e}"));

            let (on_retain, on_release, on_addr) = run_counted(&source, true);
            let (off_retain, off_release, off_addr) = run_counted(&source, false);

            let on_total = on_retain + on_release;
            let off_total = off_retain + off_release;
            let removed = off_total.saturating_sub(on_total);
            let pct_all = 100.0 * removed as f64 / off_total.max(1) as f64;
            let pct_addr =
                100.0 * (off_addr.saturating_sub(on_addr)) as f64 / off_addr.max(1) as f64;
            println!(
                "{name}:\n  retains  {off_retain} -> {on_retain}\n  releases {off_release} -> {on_release}\n                   all refcount ops {off_total} -> {on_total} (-{removed}, {pct_all:.5}%)\n                   addressable opcode ops {off_addr} -> {on_addr} (-{}, {pct_addr:.2}%)",
                off_addr.saturating_sub(on_addr)
            );

            assert!(
                on_total <= off_total,
                "{name}: the elision increased refcount traffic"
            );
            any_reduction |= on_total < off_total;
        }
        assert!(
            any_reduction,
            "no allocation-heavy workload lost a single refcount operation — \
             the pass does not reach this suite and the ticket cannot close on it"
        );
    }
}
