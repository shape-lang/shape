use std::collections::BTreeSet;

use shape_ast::parser::parse_program;
use shape_runtime::comptime_reflection::FrozenTypeCategory;
use shape_vm::compiler::{GeneratedCaptureDescriptorView, GeneratedCaptureQuery};

use super::query;

const CALLABLE_CAPTURES: &str = r#"
annotation add_inspector() on type {
  comptime post(target, ctx) {
    extend target {
      method inspect() -> int {
        let keep_int = |value: int| value
        let keep_text = |value: string| value
        let int_worker = |; move keep_int| keep_int(1)
        let text_worker = |; move keep_text| keep_text("shape")
        text_worker()
        int_worker()
      }
    }
  }
}

@add_inspector()
type Job { id: int }

let job = Job { id: 1 }
job.inspect()
"#;

const CALLABLE_ONLY_GENERIC: &str = r#"
annotation add_apply() on type {
  comptime post(target, ctx) {
    extend target {
      method apply<T>(value: T) -> int {
        let base = 1
        let worker = |item: T; move base| base
        worker(value)
      }
    }
  }
}

@add_apply()
type Job { id: int }

let job = Job { id: 1 }
let number = job.apply(7)
let text = job.apply("shape")
"#;

const ORDER_BASE: &str = r#"
annotation add_runner() on type {
  comptime post(target, ctx) {
    extend target {
      method run(x: int) -> int {
        let scale = |value: int| value
        let worker = |y: int; move scale| scale(y)
        worker(x)
      }
    }
  }
}

@add_runner()
type Job { id: int }

let job = Job { id: 1 }
job.run(2)
"#;

#[test]
fn callable_capture_publishes_full_semantics_without_abi_or_registry_ids() {
    let captures = compile_query(CALLABLE_CAPTURES);
    let keep_int = capture(&captures, "keep_int");
    let specialization = &keep_int.specializations()[0];
    let captured_callable = specialization.capture_type();
    let whole_callable = specialization.identity().callable_type();

    assert_eq!(captured_callable.category(), FrozenTypeCategory::Callable);
    assert_eq!(whole_callable.category(), FrozenTypeCategory::Callable);
    assert!(captured_callable.presentation().contains("int"));
    assert!(captured_callable.presentation().contains("->"));
    assert!(whole_callable.presentation().contains("->"));
    assert_has_no_abi_identity(captured_callable.presentation());
    assert_has_no_abi_identity(&specialization.identity().canonical_descriptor());
}

#[test]
fn callable_capture_signatures_do_not_collapse_to_their_common_pointer_abi() {
    let captures = compile_query(CALLABLE_CAPTURES);
    let keep_int = capture(&captures, "keep_int").specializations()[0].capture_type();
    let keep_text = capture(&captures, "keep_text").specializations()[0].capture_type();

    assert_eq!(keep_int.category(), FrozenTypeCategory::Callable);
    assert_eq!(keep_text.category(), FrozenTypeCategory::Callable);
    assert_ne!(keep_int, keep_text);
    assert_ne!(
        keep_int.identity_components(),
        keep_text.identity_components()
    );
    assert!(keep_int.presentation().contains("int"));
    assert!(keep_text.presentation().contains("string"));
}

#[test]
fn callable_only_generic_monomorphizations_remain_distinct_specializations() {
    let captures = compile_query(CALLABLE_ONLY_GENERIC);
    let base = capture(&captures, "base");
    let capture_types: BTreeSet<_> = base
        .specializations()
        .iter()
        .map(|specialization| specialization.capture_type().clone())
        .collect();
    let callable_types: BTreeSet<_> = base
        .specializations()
        .iter()
        .map(|specialization| specialization.identity().callable_type().clone())
        .collect();
    let callable_presentations: BTreeSet<_> = callable_types
        .iter()
        .map(|ty| ty.presentation().to_string())
        .collect();

    assert_eq!(capture_types.len(), 1, "only the callable type varies");
    assert_eq!(callable_types.len(), 2);
    assert!(callable_presentations.iter().any(|ty| ty.contains("int")));
    assert!(
        callable_presentations
            .iter()
            .any(|ty| ty.contains("string"))
    );
    assert_eq!(base.specializations().len(), 2);
}

#[test]
fn unrelated_earlier_callable_registration_cannot_change_specialization_identity() {
    let with_earlier_callable = format!(
        "fn unrelated() -> int {{ let earlier = |value: int| value; earlier(1) }}\n{ORDER_BASE}",
    );
    let base = compile_query(ORDER_BASE);
    let reordered = compile_query(&with_earlier_callable);
    let base_identity = capture(&base, "scale").specializations()[0].identity();
    let reordered_identity = capture(&reordered, "scale").specializations()[0].identity();

    assert_eq!(base_identity, reordered_identity);
    assert_eq!(
        base_identity.canonical_descriptor(),
        reordered_identity.canonical_descriptor(),
    );
    assert_has_no_abi_identity(&base_identity.canonical_descriptor());
}

/// ADR-009 E2 #18 slice 3 — the C0911 quarantine FLIP. A `replace body` edit's
/// closure capture is now not merely OBSERVED on the shared C1 query surface but
/// RESOLVED to an exact specialization identity, exactly like the fresh-generated
/// `extend` fixtures above.
///
/// The C2 #13 named finding was: the structural inference facts that back a
/// capture's specialization identity are recorded by the type-inference engine at
/// ANALYSIS time (`enter_generated_function_fact_scope`, keyed by closure origin +
/// ordinal) and handed off once, immutably, to the compiler. A FRESH generated
/// `extend` method is materialized PRE-analysis, so its closures get facts and
/// resolve exactly. A `replace body` REPLACEMENT used to be swapped at PASS-2,
/// after that handoff — so the analyzer never saw its closure, no fact was
/// published, and the capture resolved to a `[C0911]` MissingInferenceFact
/// quarantine.
///
/// E2 slice 3 closes that seam: it materializes a const-free, top-level,
/// closure-bearing replacement through the SAME pre-analysis window (the executed
/// declaration-discovery pre-pass), stamping the replacement closures with the
/// SAME `ExpansionSite` pass-2 uses. So the analyzer now infers the replacement's
/// `move base` closure and publishes its structural fact, keyed identically to the
/// capture descriptor pass-2 builds — the key equality that IS the flip. The
/// `move base` capture (present only in the REPLACEMENT — the pre-edit `7` has no
/// closure) therefore resolves to an exact Active capture, no longer a `[C0911]`.
///
/// This test was `replace_body_edit_capture_is_observed_but_specialization_quarantined`
/// (a tripwire pinning the pre-E2 quarantine); slice-3 rebases it — a documented
/// true-positive rebaseline — to the observed-AND-resolved truth. The
/// still-meaningful inverse companion is the EXTEND-method quarantine in
/// `generation_reachability_tests` (a distinct directive path E2 does not touch),
/// which must keep firing `[C0911]`.
const REPLACE_BODY_EDIT_CAPTURE: &str = r#"
annotation edit_worker() on function {
  comptime post(target, ctx) {
    replace body {
      let base = 40
      let worker = |; move base| base + 2
      return worker()
    }
  }
}

@edit_worker()
fn answer() -> int { 7 }

answer()
"#;

#[test]
fn replace_body_edit_capture_is_observed_and_resolved() {
    // `compile_query` asserts NO `[C0911]` quarantine appears — that assertion IS
    // the flip (pre-E2 this fixture quarantined `base`; post-E2 it resolves).
    let captures = compile_query(REPLACE_BODY_EDIT_CAPTURE);

    // And the resolved capture is an exact Active capture with a real
    // specialization identity — observed AND resolved, like the extend fixtures.
    let base = capture(&captures, "base");
    assert!(
        !base.specializations().is_empty(),
        "the replace-body edit's `move base` capture now resolves to an exact specialization \
         identity (pre-analysis materialization published its structural fact), not a [C0911] \
         MissingInferenceFact quarantine",
    );
}

fn compile_query(source: &str) -> GeneratedCaptureQuery {
    let program = parse_program(source).expect("semantic capture fixture parses");
    let captures = query(&program, source);
    assert!(
        captures
            .issues()
            .iter()
            .all(|issue| issue.code() != "C0911"),
        "exact semantic fixtures must not be quarantined: {:?}",
        captures.issues(),
    );
    captures
}

fn capture<'query>(
    query: &'query GeneratedCaptureQuery,
    name: &str,
) -> &'query GeneratedCaptureDescriptorView {
    query
        .captures()
        .iter()
        .find(|capture| capture.display_name() == name)
        .unwrap_or_else(|| panic!("fixture has generated capture '{name}'"))
}

fn assert_has_no_abi_identity(rendered: &str) {
    for forbidden in [
        "ClosureTypeId",
        "FunctionTypeId",
        "ConcreteType",
        "Pointer",
        "func_idx",
        "registry",
    ] {
        assert!(
            !rendered.contains(forbidden),
            "semantic projection leaked ABI/registry identity '{forbidden}': {rendered}",
        );
    }
}
