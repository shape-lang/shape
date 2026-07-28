//! Elision-plan assertions for the JIT bounds-check elision analyzer.
//!
//! Every fixture is **real Shape source**, compiled through the ordinary
//! bytecode pipeline (`shape_vm::stdlib::compile_source`), so the MIR the
//! analyzer sees is the MIR the JIT sees. The pre-widening version of this
//! file asserted against hand-built MIR that the lowering never emits, which
//! is how a matcher that admitted *nothing on any real program* passed its
//! tests for two waves.
//!
//! Each test asserts the plan **per fixture and per access**: which
//! `(block, statement, receiver, index)` sites the analyzer trusts, named by
//! the source-level access they correspond to. Timing is never used to infer
//! elision.
//!
//! The runtime counterpart — an out-of-range access of each widened shape
//! still trapping through the checked path — lives in
//! `crates/shape-jit/tests/bounds_elision_traps.rs`, which executes each
//! fixture on both tiers.

use shape_jit::mir_compiler::bounds_elision::{self, AccessSite, ElisionBase, ElisionIndex};
use shape_vm::mir::types::{MirFunction, Operand, Place, Rvalue, SlotId, StatementKind};

/// Compile `src` and return the MIR of the named function.
fn mir_of(name: &str, src: &str) -> MirFunction {
    let program = shape_vm::stdlib::compile_source("fixture.shape", src)
        .unwrap_or_else(|e| panic!("fixture failed to compile: {e}"));
    program
        .functions
        .iter()
        .find(|f| f.name == name)
        .unwrap_or_else(|| panic!("fixture has no function `{name}`"))
        .mir_data
        .as_ref()
        .unwrap_or_else(|| panic!("function `{name}` carries no MIR"))
        .mir
        .clone()
}

/// Every `Place::Index` site in the function, in MIR order — the denominator
/// the plan is asserted against, so a test can state "2 of these 3 accesses
/// elide".
fn all_index_sites(mir: &MirFunction) -> Vec<AccessSite> {
    let mut out = Vec::new();
    for block in &mir.blocks {
        for (stmt_idx, stmt) in block.statements.iter().enumerate() {
            let mut places: Vec<&Place> = Vec::new();
            collect(&stmt.kind, &mut places);
            for p in places {
                if let Place::Index(base, index) = p {
                    if let Some((b, i)) = bounds_elision::classify_access(base, index) {
                        out.push(AccessSite {
                            block: block.id,
                            stmt: stmt_idx,
                            base: b,
                            index: i,
                        });
                    }
                }
            }
        }
    }
    out
}

fn collect<'a>(kind: &'a StatementKind, out: &mut Vec<&'a Place>) {
    // Only the shapes the fixtures produce: an index access always appears
    // inside an `Assign`, as the destination place or inside the rvalue.
    if let StatementKind::Assign(place, rv) = kind {
        walk_place(place, out);
        match rv {
            Rvalue::Use(op) | Rvalue::Clone(op) | Rvalue::UnaryOp(_, op) => walk_operand(op, out),
            Rvalue::BinaryOp(_, a, b) => {
                walk_operand(a, out);
                walk_operand(b, out);
            }
            _ => {}
        }
    }
}

fn walk_place<'a>(p: &'a Place, out: &mut Vec<&'a Place>) {
    match p {
        Place::Local(_) => {}
        Place::Field(inner, _) | Place::Deref(inner) => walk_place(inner, out),
        Place::Index(base, index) => {
            out.push(p);
            walk_place(base, out);
            walk_operand(index, out);
        }
    }
}

fn walk_operand<'a>(op: &'a Operand, out: &mut Vec<&'a Place>) {
    if let Operand::Copy(p) | Operand::Move(p) | Operand::MoveExplicit(p) = op {
        walk_place(p, out);
    }
}

/// Assert the plan trusts exactly `expected_trusted` of the function's index
/// sites, and report the full site inventory on failure.
#[track_caller]
fn assert_plan(mir: &MirFunction, expected_trusted: usize) -> Vec<AccessSite> {
    let plan = bounds_elision::analyze(mir);
    let sites = all_index_sites(mir);
    let trusted: Vec<AccessSite> = sites
        .iter()
        .copied()
        .filter(|s| plan.is_trusted_site(s))
        .collect();
    assert_eq!(
        trusted.len(),
        expected_trusted,
        "expected {expected_trusted} of {} index sites trusted.\n  all sites: {:#?}\n  plan: {:#?}",
        sites.len(),
        sites,
        plan.sites_sorted(),
    );
    // The plan must not name anything that is not a real index access here.
    assert_eq!(
        plan.len(),
        trusted.len(),
        "plan holds sites that are not index accesses in this function: {:#?}",
        plan.sites_sorted(),
    );
    trusted
}

// ── Shape 0: the pre-existing bare `arr[iv]` matcher ─────────────────────

/// `while i < arr.length { arr[i] }` — the shape the pre-widening matcher was
/// documented to admit and, on real MIR, admitted nothing of: the bound is
/// read inline in the comparison, `i` is initialised through a copy of a
/// constant temp, and `i = i + 1` lowers to `t = i + 1; i = t`. All three
/// defeated the old pattern match.
#[test]
fn while_length_bound_elides_the_single_access() {
    let mir = mir_of(
        "sum",
        r#"
fn sum(arr: Array<int>) -> int {
    let mut acc = 0
    let mut i = 0
    while i < arr.length {
        acc = acc + arr[i]
        i = i + 1
    }
    return acc
}
print(sum([1, 2, 3]))
"#,
    );
    let trusted = assert_plan(&mir, 1);
    assert!(matches!(trusted[0].base, ElisionBase::Local(_)));
    assert!(matches!(trusted[0].index, ElisionIndex::Slot(_)));
}

/// `for i in 0..arr.length` puts the induction step in a separate latch
/// block, so the analyzer must reason over the whole natural loop rather than
/// the header's true successor alone.
#[test]
fn for_range_over_length_elides_the_single_access() {
    let mir = mir_of(
        "sum",
        r#"
fn sum(arr: Array<int>) -> int {
    let mut acc = 0
    for i in 0..arr.length {
        acc = acc + arr[i]
    }
    return acc
}
print(sum([1, 2, 3]))
"#,
    );
    assert_plan(&mir, 1);
}

/// An access after the loop must stay checked even though it uses the same
/// `(array, index)` slot pair — at that point `i == arr.length`. This is the
/// case a slot-pair-keyed plan gets wrong.
#[test]
fn access_after_the_loop_is_not_trusted() {
    let mir = mir_of(
        "tail",
        r#"
fn tail(arr: Array<int>) -> int {
    let mut acc = 0
    let mut i = 0
    while i < arr.length {
        acc = acc + arr[i]
        i = i + 1
    }
    return acc + arr[i]
}
print(tail([1, 2, 3]))
"#,
    );
    let trusted = assert_plan(&mir, 1);
    let sites = all_index_sites(&mir);
    assert_eq!(sites.len(), 2, "fixture should have two index sites");
    assert_ne!(
        trusted[0], sites[1],
        "the post-loop access must not be trusted"
    );
}

// ── Shape (a): constant index with proven bounds ─────────────────────────

/// Inside a body guarded by `i < arr.length` with `i >= 0`, `arr.length >= 1`,
/// so `arr[0]` is proven in range. `arr[1]` is not — the guard gives no
/// second element.
#[test]
fn constant_index_within_proven_length_elides_and_beyond_it_does_not() {
    let mir = mir_of(
        "head",
        r#"
fn head(arr: Array<int>) -> int {
    let mut acc = 0
    let mut i = 0
    while i < arr.length {
        acc = acc + arr[0] + arr[1]
        i = i + 1
    }
    return acc
}
print(head([1, 2, 3]))
"#,
    );
    let sites = all_index_sites(&mir);
    let trusted = assert_plan(&mir, 1);
    assert_eq!(sites.len(), 2, "fixture has arr[0] and arr[1]");
    assert_eq!(
        trusted[0], sites[0],
        "only arr[0] is proven; arr[1] needs a length the guard does not give"
    );
}

/// A loop starting at `i = 2` proves `arr.length >= 3`, so `arr[0]` and
/// `arr[2]` elide but `arr[3]` does not — the constant bound scales with the
/// induction variable's proven lower bound.
#[test]
fn constant_index_bound_scales_with_the_iv_lower_bound() {
    let mir = mir_of(
        "from_two",
        r#"
fn from_two(arr: Array<int>) -> int {
    let mut acc = 0
    let mut i = 2
    while i < arr.length {
        acc = acc + arr[0] + arr[2] + arr[3]
        i = i + 1
    }
    return acc
}
print(from_two([1, 2, 3, 4]))
"#,
    );
    assert_plan(&mir, 2);
}

// ── Shape (b): `iv ± constant` with adjusted range proofs ────────────────

/// `i` starts at 1 and the bound is `arr.length - 2`, so the window
/// `arr[i-1] .. arr[i+2]` is proven: `i - 1 >= 0` from the start value, and
/// `i + 2 < length` from the two elements of slack in the bound.
#[test]
fn iv_offset_window_within_slack_elides_every_access() {
    let mir = mir_of(
        "window",
        r#"
fn window(arr: Array<int>) -> int {
    let mut acc = 0
    let mut i = 1
    let n = arr.length - 2
    while i < n {
        acc = acc + arr[i - 1] + arr[i] + arr[i + 1] + arr[i + 2]
        i = i + 1
    }
    return acc
}
print(window([1, 2, 3, 4, 5, 6]))
"#,
    );
    assert_plan(&mir, 4);
}

/// One element past the slack is not proven, and neither is one element below
/// the start value. Both stay on the checked path.
#[test]
fn iv_offset_outside_the_proven_range_is_not_trusted() {
    let mir = mir_of(
        "over",
        r#"
fn over(arr: Array<int>) -> int {
    let mut acc = 0
    let mut i = 1
    let n = arr.length - 1
    while i < n {
        acc = acc + arr[i + 1] + arr[i + 2] + arr[i - 2]
        i = i + 1
    }
    return acc
}
print(over([1, 2, 3, 4, 5]))
"#,
    );
    // Slack is 1, so only `arr[i + 1]` is admitted; `arr[i + 2]` exceeds the
    // slack and `arr[i - 2]` can be -1.
    assert_plan(&mir, 1);
}

/// `i <= arr.length - 1` admits `i == arr.length - 1`, so an inclusive
/// comparison spends one element of slack. `arr[i]` still elides; `arr[i+1]`
/// does not.
#[test]
fn inclusive_bound_spends_one_element_of_slack() {
    let mir = mir_of(
        "incl",
        r#"
fn incl(arr: Array<int>) -> int {
    let mut acc = 0
    let mut i = 0
    let n = arr.length - 1
    while i <= n {
        acc = acc + arr[i] + arr[i + 1]
        i = i + 1
    }
    return acc
}
print(incl([1, 2, 3]))
"#,
    );
    assert_plan(&mir, 1);
}

/// A bare `while i <= arr.length` proves nothing at all — `arr[i]` with
/// `i == arr.length` is out of range.
#[test]
fn inclusive_bound_on_bare_length_trusts_nothing() {
    let mir = mir_of(
        "bad_incl",
        r#"
fn bad_incl(arr: Array<int>) -> int {
    let mut acc = 0
    let mut i = 0
    while i <= arr.length {
        acc = acc + arr[i]
        i = i + 1
    }
    return acc
}
print(bad_incl([1, 2, 3]))
"#,
    );
    assert_plan(&mir, 0);
}

// ── Nested loops: the outer fact reaches the inner body ─────────────────

/// The bspline-class shape: an outer loop whose induction variable indexes a
/// window, and an inner loop over samples that does the indexing. The outer
/// variable is invariant across the inner loop, so its range fact still holds
/// there. All four windowed accesses elide even though none of them is in the
/// outer loop's immediate body block.
#[test]
fn nested_inner_loop_body_inherits_the_outer_loop_fact() {
    let mir = mir_of(
        "spline",
        r#"
fn spline(cp: Array<number>, samples: int) -> number {
    let mut acc = 0.0
    let mut seg = 1
    let n = cp.length - 2
    while seg < n {
        let mut s = 0
        while s < samples {
            acc = acc + cp[seg - 1] + cp[seg] + cp[seg + 1] + cp[seg + 2]
            s = s + 1
        }
        seg = seg + 1
    }
    return acc
}
print(spline([1.0, 2.0, 3.0, 4.0, 5.0], 2))
"#,
    );
    assert_plan(&mir, 4);
}

/// The inner loop's own induction variable is bounded by an unrelated
/// parameter, so accesses indexed by *it* are not admitted — only the outer
/// variable carries a length-derived fact.
#[test]
fn nested_inner_variable_without_a_length_fact_is_not_trusted() {
    let mir = mir_of(
        "mixed",
        r#"
fn mixed(cp: Array<number>, samples: int) -> number {
    let mut acc = 0.0
    let mut seg = 0
    let n = cp.length
    while seg < n {
        let mut s = 0
        while s < samples {
            acc = acc + cp[seg] + cp[s]
            s = s + 1
        }
        seg = seg + 1
    }
    return acc
}
print(mixed([1.0, 2.0, 3.0], 2))
"#,
    );
    // `cp[seg]` elides; `cp[s]` does not.
    assert_plan(&mir, 1);
}

/// A step of the induction variable on a path that does not go back through
/// the header invalidates the fact for anything downstream. The analyzer must
/// not admit a block merely because *one* clean path reaches it.
#[test]
fn access_after_an_unguarded_step_is_not_trusted() {
    let mir = mir_of(
        "stepped",
        r#"
fn stepped(arr: Array<int>, flag: bool) -> int {
    let mut acc = 0
    let mut i = 0
    while i < arr.length {
        if flag {
            i = i + 1
        }
        acc = acc + arr[i]
        i = i + 1
    }
    return acc
}
print(stepped([1, 2, 3], true))
"#,
    );
    assert_plan(&mir, 0);
}

// ── Shape (c): field-projected receivers ─────────────────────────────────

#[test]
fn field_projected_receiver_elides_when_the_field_is_never_written() {
    let mir = mir_of(
        "total",
        r#"
type Buf { data: Array<int> }
fn total(b: Buf) -> int {
    let mut acc = 0
    let mut i = 0
    while i < b.data.length {
        acc = acc + b.data[i]
        i = i + 1
    }
    return acc
}
print(total(Buf { data: [1, 2, 3] }))
"#,
    );
    let trusted = assert_plan(&mir, 1);
    assert!(matches!(trusted[0].base, ElisionBase::Field(_, _)));
}

#[test]
fn field_projected_receiver_is_not_trusted_when_the_field_is_written() {
    let mir = mir_of(
        "total",
        r#"
type Buf { data: Array<int> }
fn total(b: Buf) -> int {
    let mut acc = 0
    let mut i = 0
    while i < b.data.length {
        acc = acc + b.data[i]
        i = i + 1
    }
    b.data = [9]
    return acc
}
print(total(Buf { data: [1, 2, 3] }))
"#,
    );
    assert_plan(&mir, 0);
}

// ── Receiver-stability and induction negative controls ───────────────────

/// A bound that is not derived from the receiver's length proves nothing,
/// however plausible it looks. This is why the charter's `numeric_spline` and
/// `07b_dot_product` kernels gain no elisions.
#[test]
fn parameter_bound_unrelated_to_the_array_trusts_nothing() {
    let mir = mir_of(
        "dot",
        r#"
fn dot(a: Array<int>, n: int) -> int {
    let mut acc = 0
    let mut k = 0
    while k < n {
        acc = acc + a[k]
        k = k + 1
    }
    return acc
}
print(dot([1, 2, 3], 3))
"#,
    );
    assert_plan(&mir, 0);
}

/// Reassigning the receiver inside the loop invalidates the captured length.
#[test]
fn receiver_reassigned_in_the_loop_trusts_nothing() {
    let mir = mir_of(
        "grow",
        r#"
fn grow(seed: Array<int>) -> int {
    let mut arr = seed
    let mut acc = 0
    let mut i = 0
    let n = arr.length
    while i < n {
        acc = acc + arr[i]
        arr = arr.push(0)
        i = i + 1
    }
    return acc
}
print(grow([1, 2, 3]))
"#,
    );
    assert_plan(&mir, 0);
}

/// A decreasing induction variable has no non-negative lower bound.
#[test]
fn decrementing_induction_variable_trusts_nothing() {
    let mir = mir_of(
        "down",
        r#"
fn down(arr: Array<int>) -> int {
    let mut acc = 0
    let mut i = 0
    while i < arr.length {
        acc = acc + arr[i]
        i = i - 1
    }
    return acc
}
print(down([1, 2, 3]))
"#,
    );
    assert_plan(&mir, 0);
}

/// A negative start value breaks the non-negativity half of the proof even
/// though the upper half still holds.
#[test]
fn negative_start_value_trusts_nothing() {
    let mir = mir_of(
        "neg",
        r#"
fn neg(arr: Array<int>) -> int {
    let mut acc = 0
    let mut i = 0 - 1
    while i < arr.length {
        acc = acc + arr[i]
        i = i + 1
    }
    return acc
}
print(neg([1, 2, 3]))
"#,
    );
    assert_plan(&mir, 0);
}

/// `for x in arr` sources its bound from a `len` **method call**. Trusting it
/// would let a method's spelling select a memory-safety proof, which
/// §Forbidden Patterns refuses; the analyzer declines until `len` carries a
/// resolved intrinsic identity (ADR-011).
#[test]
fn len_call_sourced_bound_is_deliberately_not_trusted() {
    let mir = mir_of(
        "iter",
        r#"
fn iter(arr: Array<int>) -> int {
    let mut acc = 0
    for x in arr {
        acc = acc + x
    }
    return acc
}
print(iter([1, 2, 3]))
"#,
    );
    assert_plan(&mir, 0);
}

// ── Write-side ──────────────────────────────────────────────────────────

/// `arr[i] = i` lowers to a store plus the assignment expression's read-back,
/// so the fixture has two index sites in the body; the proof covers both.
#[test]
fn indexed_writes_elide_under_the_same_proof() {
    let mir = mir_of(
        "fill",
        r#"
fn fill(arr: Array<int>) -> int {
    let mut i = 0
    while i < arr.length {
        arr[i] = i
        i = i + 1
    }
    return arr.length
}
print(fill([1, 2, 3]))
"#,
    );
    assert_plan(&mir, 2);
}

// ── Structural guards on the plan itself ────────────────────────────────

/// The analyzer must never name a site that is not an index access, and a
/// constant access outside the loop carries no loop fact.
#[test]
fn every_trusted_site_is_a_real_in_loop_access() {
    let mir = mir_of(
        "window",
        r#"
fn window(arr: Array<int>) -> int {
    let mut acc = 0
    let mut i = 1
    let n = arr.length - 2
    while i < n {
        acc = acc + arr[i - 1] + arr[i + 2]
        i = i + 1
    }
    return acc + arr[0]
}
print(window([1, 2, 3, 4, 5, 6]))
"#,
    );
    let plan = bounds_elision::analyze(&mir);
    let sites = all_index_sites(&mir);
    for s in plan.sites_sorted() {
        assert!(
            sites.contains(&s),
            "plan named a site that is not an index access: {s:?}"
        );
    }
    let trusted: Vec<&AccessSite> = sites.iter().filter(|s| plan.is_trusted_site(s)).collect();
    assert_eq!(trusted.len(), 2, "the two in-loop window accesses elide");
    assert!(
        !plan.is_trusted_site(sites.last().unwrap()),
        "the post-loop `arr[0]` must stay checked"
    );
}

#[test]
fn empty_plan_trusts_nothing() {
    let plan = bounds_elision::BoundsElisionPlan::default();
    assert!(plan.is_empty());
    assert!(!plan.is_trusted_site(&AccessSite {
        block: shape_vm::mir::types::BasicBlockId(0),
        stmt: 0,
        base: ElisionBase::Local(SlotId(1)),
        index: ElisionIndex::Slot(SlotId(2)),
    }));
}
