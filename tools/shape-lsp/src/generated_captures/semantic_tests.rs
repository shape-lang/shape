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
