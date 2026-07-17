use super::*;
use crate::compiler::BytecodeCompiler;

#[path = "inferred_tests/fixtures.rs"]
mod fixtures;
use fixtures::{facts_for, legacy_pair};

#[path = "inferred_tests/direct_function_instructions.rs"]
mod direct_function_instructions;

// ───────────────────────────────────────────────────────────────────
// (a) FUSION EQUIVALENCE — the fused plan reproduces the pre-fusion
//     `(mutable_flags[i], capture_kinds[i])` pair across the FULL
//     (tier × ownership × mutated × boxed × witness) cross-product.
//
// `legacy_pair` below is the pre-fusion code transcribed verbatim from
// closures.rs:3236-3256 (mutable_flags) and :3550-3635 (capture_kinds).
// It is the ORACLE, not a paraphrase.
// ───────────────────────────────────────────────────────────────────

#[test]
fn fused_plan_matches_legacy_pair_across_cross_product() {
    let tiers = [
        None,
        Some(CaptureTarget::Local(3)),
        Some(CaptureTarget::ModuleBinding(7)),
    ];
    let ownerships = [
        None,
        Some(BindingOwnershipClass::OwnedImmutable),
        Some(BindingOwnershipClass::OwnedMutable),
        Some(BindingOwnershipClass::Flexible),
    ];
    let bools = [false, true];

    let mut seen_param = false;
    let mut seen_owned_mutable_cell = false;
    let mut seen_shared_cell = false;
    let mut seen_mutable_cell = false;
    let mut cases = 0usize;

    for target in tiers {
        for ownership in ownerships {
            for mutated in bools {
                for boxed in bools {
                    for wsl in bools {
                        for wsm in bools {
                            for woml in bools {
                                let facts =
                                    facts_for(target, ownership, mutated, boxed, wsl, wsm, woml);
                                let (legacy_flag, legacy_kind) = legacy_pair(&facts);
                                let plan = infer_plan(&facts);
                                cases += 1;

                                assert_eq!(
                                    plan.kind(),
                                    legacy_kind,
                                    "kind divergence for {facts:?}"
                                );
                                assert_eq!(
                                    plan.needs_cell(),
                                    legacy_flag,
                                    "mutable-flag divergence for {facts:?}"
                                );

                                // The access refinement must be a faithful
                                // decomposition of the legacy pair.
                                let expected_access = match (legacy_flag, legacy_kind) {
                                    (false, _) => CaptureAccess::Param,
                                    (true, CaptureKind::OwnedMutable) => {
                                        CaptureAccess::OwnedMutableCell
                                    }
                                    (true, CaptureKind::Shared) => CaptureAccess::SharedCell,
                                    // THE RESIDUAL: cell access needed,
                                    // kind stayed Immutable.
                                    (true, CaptureKind::Immutable) => CaptureAccess::MutableCell,
                                };
                                assert_eq!(
                                    plan.access(),
                                    expected_access,
                                    "access divergence for {facts:?}"
                                );

                                match plan.access() {
                                    CaptureAccess::Param => seen_param = true,
                                    CaptureAccess::OwnedMutableCell => {
                                        seen_owned_mutable_cell = true
                                    }
                                    CaptureAccess::SharedCell => seen_shared_cell = true,
                                    CaptureAccess::MutableCell => seen_mutable_cell = true,
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    assert_eq!(
        cases,
        3 * 4 * 2 * 2 * 2 * 2 * 2,
        "full cross-product covered"
    );
    assert!(seen_param, "Param arm exercised");
    assert!(seen_owned_mutable_cell, "OwnedMutableCell arm exercised");
    assert!(seen_shared_cell, "SharedCell arm exercised");
    assert!(
        seen_mutable_cell,
        "the degenerate `mutable_flags==true ∧ kind==Immutable` residual MUST be \
             reachable on the inferred path — if it is not, the fusion silently dropped a \
             live emission arm"
    );
}

#[test]
fn inherited_shared_parameter_evidence_precedes_by_value_param_semantics() {
    let mut facts = facts_for(
        Some(CaptureTarget::Local(0)),
        Some(BindingOwnershipClass::OwnedMutable),
        false,
        false,
        false,
        false,
        false,
    );
    facts.inherited_shared_cell = true;

    let plan = infer_plan(&facts);
    assert_eq!(plan.kind(), CaptureKind::Shared);
    assert_eq!(plan.access(), CaptureAccess::SharedCell);
}

/// The residual arm, pinned by name so a future edit that "cleans it up"
/// has to delete an assertion rather than a comment: an `OwnedImmutable`
/// (a `let`) binding that a previous pass boxed needs cell access but the
/// classifier still calls it `Immutable`.
#[test]
fn boxed_let_capture_is_the_mutable_cell_residual() {
    let facts = facts_for(
        Some(CaptureTarget::Local(1)),
        Some(BindingOwnershipClass::OwnedImmutable),
        false,
        true, // boxed
        false,
        false,
        false,
    );
    let plan = infer_plan(&facts);
    assert!(plan.needs_cell(), "boxed local needs cell access");
    assert_eq!(plan.access(), CaptureAccess::MutableCell);
    assert_eq!(
        plan.kind(),
        CaptureKind::Immutable,
        "the residual keeps the Immutable kind — the layout masks stay clear"
    );
}

// ───────────────────────────────────────────────────────────────────
// (b) MODEL-vs-EMISSION — read from the EMITTED artifact
//     (`program.closure_function_layouts[fid]`), never from the model's
//     own table. This is the R2 assertion: if a future declared-mode path
//     writes the pack but leaves emission on a second inference vector,
//     THIS test fails.
// ───────────────────────────────────────────────────────────────────

pub(super) fn compile(src: &str) -> BytecodeCompiler {
    let program = shape_ast::parse_program(src).expect("fixture parses");
    let mut compiler = BytecodeCompiler::new();
    compiler
        .compile_in_place(&program)
        .expect("fixture compiles");
    compiler
}

/// For every closure the compiler planned, the EMITTED `ClosureLayout`'s
/// per-capture storage kind equals the pack's `lowered`, and the three
/// capture masks agree with the plan bit-for-bit and stay disjoint.
fn assert_model_equals_emission(compiler: &BytecodeCompiler) {
    assert!(
        !compiler.closure_capture_packs.is_empty(),
        "fixture must produce at least one closure pack"
    );
    for pack in &compiler.closure_capture_packs {
        let layout = compiler
            .program
            .closure_function_layouts
            .get(pack.closure as usize)
            .and_then(|l| l.as_ref())
            .unwrap_or_else(|| panic!("closure {} has no emitted layout", pack.closure));
        assert_eq!(
            layout.capture_kinds.len(),
            pack.len(),
            "closure {}: emitted capture count",
            pack.closure
        );
        for d in &pack.descriptors {
            let i = d.index as usize;
            // THE artifact read — not `pack.kinds()`.
            assert_eq!(
                layout.capture_storage_kind(i),
                d.lowered,
                "closure {} capture {}: emitted kind != planned kind",
                pack.closure,
                i
            );
            let bit = 1u64 << i;
            let heap = layout.heap_capture_mask & bit != 0;
            let owned = layout.owned_mutable_capture_mask & bit != 0;
            let shared = layout.shared_capture_mask & bit != 0;
            assert!(
                !(heap && owned) && !(heap && shared) && !(owned && shared),
                "closure {} capture {}: masks overlap",
                pack.closure,
                i
            );
            match d.access {
                CaptureAccess::OwnedMutableCell => {
                    assert!(
                        owned,
                        "OwnedMutableCell must set owned_mutable_capture_mask"
                    );
                    assert!(!shared && !heap);
                }
                CaptureAccess::SharedCell => {
                    assert!(shared, "SharedCell must set shared_capture_mask");
                    assert!(!owned && !heap);
                }
                // Param and MutableCell both carry the Immutable kind, so
                // the mask is TYPE-derived: heap bit iff the capture's
                // ConcreteType is a pointer.
                CaptureAccess::Param | CaptureAccess::MutableCell => {
                    assert!(!owned && !shared);
                    assert_eq!(
                        heap,
                        layout.captures[i].kind == shape_value::v2::struct_layout::FieldKind::Ptr,
                        "closure {} capture {}: heap mask must follow the capture TYPE",
                        pack.closure,
                        i
                    );
                }
            }
        }
    }
}

#[test]
fn emitted_layout_matches_plan_immutable_scalar() {
    let c = compile(
        r#"
fn run() -> int {
  let base = 10
  let f = |x: int| x + base
  f(1)
}
run()
"#,
    );
    assert_model_equals_emission(&c);
    let pack = &c.closure_capture_packs[0];
    assert_eq!(pack.descriptors[0].access, CaptureAccess::Param);
}

#[test]
fn emitted_layout_matches_plan_owned_mutable() {
    let c = compile(
        r#"
fn run() -> int {
  let mut total = 0
  let f = |x: int| { total = total + x
    total }
  f(1)
}
run()
"#,
    );
    assert_model_equals_emission(&c);
    let pack = &c.closure_capture_packs[0];
    assert_eq!(pack.descriptors[0].access, CaptureAccess::OwnedMutableCell);
    assert!(matches!(
        pack.descriptors[0].target,
        Some(CaptureTarget::Local(_))
    ));
}

#[test]
fn emitted_layout_matches_plan_shared_local_var() {
    let c = compile(
        r#"
fn run() -> int {
  var counter = 0
  let bump = |x: int| { counter = counter + x
    counter }
  let peek = |y: int| y + counter
  bump(2) + peek(1)
}
run()
"#,
    );
    assert_model_equals_emission(&c);
    // BOTH siblings must see the same shared cell — the read-only sibling
    // is Shared too, not a snapshot.
    assert_eq!(c.closure_capture_packs.len(), 2);
    for pack in &c.closure_capture_packs {
        assert_eq!(pack.descriptors[0].access, CaptureAccess::SharedCell);
    }
}

#[test]
fn emitted_layout_matches_plan_shared_module_binding() {
    let c = compile(
        r#"
var hits = 0
let f = |x: int| { hits = hits + x
  hits }
f(3)
"#,
    );
    assert_model_equals_emission(&c);
    let pack = &c.closure_capture_packs[0];
    assert_eq!(pack.descriptors[0].access, CaptureAccess::SharedCell);
    assert!(matches!(
        pack.descriptors[0].target,
        Some(CaptureTarget::ModuleBinding(_))
    ));
    assert!(!c.shared_module_bindings.is_empty());
    assert!(c
        .program
        .instructions
        .iter()
        .any(|instruction| {
            instruction.opcode == crate::bytecode::OpCode::AllocSharedModuleBinding
        }));
}

#[test]
fn emitted_layout_matches_plan_heap_immutable_capture() {
    let c = compile(
        r#"
fn run() -> int {
  let xs = [1, 2, 3]
  let f = |i: int| xs[i]
  f(1)
}
run()
"#,
    );
    assert_model_equals_emission(&c);
    let pack = &c.closure_capture_packs[0];
    assert_eq!(pack.descriptors[0].access, CaptureAccess::Param);
    // heap mask follows the TYPE, not the mode.
    let layout = c.program.closure_function_layouts[pack.closure as usize]
        .as_ref()
        .unwrap();
    assert_eq!(layout.heap_capture_mask & 1, 1);
}

#[test]
fn emitted_layout_matches_plan_nested_closures() {
    let c = compile(
        r#"
fn run() -> int {
  let outer = 7
  let f = |x: int| {
    let g = |y: int| y + outer
    g(x)
  }
  f(1)
}
run()
"#,
    );
    assert_model_equals_emission(&c);
    assert_eq!(c.closure_capture_packs.len(), 2);
}

/// R3: the pack is keyed by `func_idx`, and distinct closures get distinct
/// keys. A `Span`-keyed table collides here the moment generated AST
/// (which parses from offset 0) is in play — that was rejection finding (2).
#[test]
fn packs_are_keyed_by_func_idx_and_are_distinct_per_closure() {
    let c = compile(
        r#"
fn run() -> int {
  var counter = 0
  let bump = |x: int| { counter = counter + x
    counter }
  let peek = |y: int| y + counter
  bump(2) + peek(1)
}
run()
"#,
    );
    let keys: Vec<u16> = c.closure_capture_packs.iter().map(|p| p.closure).collect();
    let mut sorted = keys.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(keys.len(), sorted.len(), "func_idx keys must be distinct");
}

#[test]
fn rejected_closure_body_clears_pending_capture_evidence() {
    let bad = shape_ast::parse_program("let broken = || { let x = 1\n x = 2\n x }\nbroken()")
        .expect("bad fixture parses");
    let good = shape_ast::parse_program("let ok = || 42\nok()").expect("follow-up fixture parses");
    let mut compiler = BytecodeCompiler::new();

    let error = compiler
        .compile_in_place(&bad)
        .expect_err("immutable assignment must be a named compile error, not a panic");
    assert!(
        error
            .to_string()
            .contains("cannot assign to immutable binding 'x'"),
        "unexpected diagnostic: {error}"
    );
    assert!(
        compiler
            .pending_closure_capture_parameter_evidence
            .is_none()
    );
    assert!(compiler.mutable_closure_captures.is_empty());
    assert!(compiler.shared_closure_captures.is_empty());
    assert!(compiler.owned_mutable_closure_captures.is_empty());

    compiler
        .compile_in_place(&good)
        .expect("the same compiler remains clean after the rejected closure");
    assert!(
        compiler
            .pending_closure_capture_parameter_evidence
            .is_none()
    );
}

#[path = "inferred_tests/invariants.rs"]
mod invariants;

#[path = "inferred_tests/module_capture_preflight.rs"]
mod module_capture_preflight;
