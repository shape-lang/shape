use std::collections::BTreeSet;

use shape_ast::parser::parse_program;
use shape_runtime::comptime_reflection::FrozenTypeCategory;
use shape_vm::compiler::{GeneratedCaptureDescriptorView, GeneratedCaptureQuery};

use super::query;

const CALLABLE_CAPTURES: &str = r#"
annotation add_inspector() {
  targets: [type]
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
annotation add_apply() {
  targets: [type]
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
annotation add_runner() {
  targets: [type]
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

/// ADR-009 C2 #13 slice 6 — the shared C1 capture query surface OBSERVES a
/// replace-body EDIT's closure capture (spec §7 "LSP driven by the shared query
/// surface"), pinning the honest current truth: the edit's `move base` capture
/// (present only in the REPLACEMENT body — the pre-edit `7` has no closure) is
/// seen and provenance-chained to the replacement's node path, but it is
/// QUARANTINED on specialization identity with a `[C0911]` MissingInferenceFact,
/// NOT resolved to an exact Active capture.
///
/// NAMED FINDING (E2 candidate — reported for the C2 close, not a bounded C2
/// patch): the structural inference facts that back a capture's specialization
/// identity are recorded by the type-inference engine at ANALYSIS time
/// (`enter_generated_function_fact_scope`, keyed by closure origin + ordinal) and
/// handed off once, immutably, to the compiler. A FRESH generated `extend`
/// method is materialized PRE-analysis (the D2 pre-pass + `infer_extend_method_bodies`),
/// so its closures get facts and resolve exactly (the `compile_query` fixtures
/// above). A `replace body` REPLACEMENT is swapped at PASS-2, after analysis and
/// after the fact handoff, so the analyzer never sees its closure and no fact is
/// published (NOT mis-keyed — not published). CODEGEN/install is UNAFFECTED: the
/// `CaptureKind` lowering is declared-mode-driven, proven by the slice-4 install
/// pin + the slice-5 native proof; only the semantic specialization identity is
/// quarantined, pending pre-analysis materialization of directive-edited bodies
/// (E2, blocked by C2/D1). The shared-surface FLOW-THROUGH still holds — the edit
/// is observed on the same surface — which is what this test guards.
const REPLACE_BODY_EDIT_CAPTURE: &str = r#"
annotation edit_worker() {
  targets: [function]
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
fn replace_body_edit_capture_is_observed_but_specialization_quarantined() {
    let program = parse_program(REPLACE_BODY_EDIT_CAPTURE).expect("replace-body fixture parses");
    let captures = query(&program, REPLACE_BODY_EDIT_CAPTURE);

    // The shared C1 capture query surface OBSERVES the edit's closure capture: a
    // provenance-chained `[C0911]` for `base` (only the REPLACEMENT body has a
    // closure) — proving C2's edited-body captures flow through the same surface.
    let quarantine = captures
        .issues()
        .iter()
        .find(|issue| issue.code() == "C0911" && issue.message().contains("base"))
        .expect(
            "the shared surface must observe the replace-body edit's `base` capture as a \
             provenance-chained [C0911]",
        );
    assert!(
        quarantine.message().contains("structural")
            && quarantine.message().contains("specialization identity"),
        "the [C0911] must name the missing structural specialization identity for `base`: {}",
        quarantine.message(),
    );

    // Named finding: the capture is quarantined, NOT resolved to an exact Active
    // capture, because the structural inference fact is recorded at analysis time
    // over the pre-edit body and the replacement is swapped at pass-2. Codegen is
    // unaffected (declared-mode lowering); publishing the fact needs pre-analysis
    // materialization of directive-edited bodies (E2), beyond a bounded C2 seam.
    assert!(
        !captures
            .captures()
            .iter()
            .any(|capture| capture.display_name() == "base"),
        "the quarantined capture is the [C0911] above, not an exact Active capture",
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
