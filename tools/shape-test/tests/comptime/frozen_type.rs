use shape_test::shape_test::ShapeTest;

fn expect_vm_and_jit_output(source: &str, expected: &str) {
    ShapeTest::new(source).expect_output(expected);
    ShapeTest::new(source).with_jit().expect_output(expected);
}

#[test]
fn type_ref_category_is_a_typed_exhaustive_enum() {
    let source = r#"
let label = comptime {
  match type_category(type_ref(int)) {
    FrozenTypeCategory::Primitive => "primitive"
    FrozenTypeCategory::Never => "never"
    FrozenTypeCategory::Parameter => "parameter"
    FrozenTypeCategory::Nominal => "nominal"
    FrozenTypeCategory::Tuple => "tuple"
    FrozenTypeCategory::Record => "record"
    FrozenTypeCategory::Callable => "callable"
    FrozenTypeCategory::Reference => "reference"
    FrozenTypeCategory::Union => "union"
    FrozenTypeCategory::Erased => "erased"
  }
}

print(label)
"#;
    expect_vm_and_jit_output(source, "primitive");
}

#[test]
fn user_type_ref_category_is_nominal() {
    ShapeTest::new(
        r#"
type User { id: int }

let label = comptime {
  match type_category(type_ref(User)) {
    FrozenTypeCategory::Nominal => "nominal"
    _ => "wrong"
  }
}

print(label)
"#,
    )
    .expect_output("nominal");
}

#[test]
fn enum_and_builtin_container_types_are_nominal() {
    ShapeTest::new(
        r#"
enum Status { Ready, Done }

let enum_label = comptime {
  match type_category(type_ref(Status)) {
    FrozenTypeCategory::Nominal => "nominal"
    _ => "wrong"
  }
}
let array_label = comptime {
  match type_category(type_ref(Array)) {
    FrozenTypeCategory::Nominal => "nominal"
    _ => "wrong"
  }
}

print(enum_label)
print(array_label)
"#,
    )
    .expect_output("nominal\nnominal");
}

#[test]
fn never_and_explicit_any_have_closed_categories() {
    ShapeTest::new(
        r#"
let never_label = comptime {
  match type_category(type_ref(never)) {
    FrozenTypeCategory::Never => "never"
    _ => "wrong"
  }
}
let erased_label = comptime {
  match type_category(type_ref(any)) {
    FrozenTypeCategory::Erased => "erased"
    _ => "wrong"
  }
}

print(never_label)
print(erased_label)
"#,
    )
    .expect_output("never\nerased");
}

// ADR-009 A3 (Wave-46 gap #4): a generic function whose body reflects on its
// own declared type parameter observes FrozenTypeCategory::Parameter — with
// identity scoped to the BASE generic function name (pre-substitution,
// declaration-stable per ADR-009 / Decision 52), even though comptime executes
// once per instantiation when the specialized body compiles.
#[test]
fn generic_body_observes_parameter_category_for_its_own_type_param() {
    let source = r#"
fn describe<T>(value: T) -> string {
  let label = comptime {
    match type_category(type_ref(T)) {
      FrozenTypeCategory::Primitive => "primitive"
      FrozenTypeCategory::Never => "never"
      FrozenTypeCategory::Parameter => "parameter"
      FrozenTypeCategory::Nominal => "nominal"
      FrozenTypeCategory::Tuple => "tuple"
      FrozenTypeCategory::Record => "record"
      FrozenTypeCategory::Callable => "callable"
      FrozenTypeCategory::Reference => "reference"
      FrozenTypeCategory::Union => "union"
      FrozenTypeCategory::Erased => "erased"
    }
  }
  label
}

print(describe(1))
"#;
    expect_vm_and_jit_output(source, "parameter");
}

// ADR-009 A3 (S2 negative proof): an UNDECLARED name inside a generic body
// must still fail the semantic freeze with the NAMED diagnostic. The
// specialized-body compile error must propagate out of monomorphization
// (surface-and-stop) — before S2 it was swallowed by the specialization
// call site's soft fallback and re-reported as the unrelated "cannot infer
// type argument(s)" inference failure.
#[test]
fn undeclared_name_in_generic_body_still_fails_the_freeze() {
    ShapeTest::new(
        r#"
fn describe<T>(value: T) -> string {
  let label = comptime {
    match type_category(type_ref(U)) {
      FrozenTypeCategory::Parameter => "parameter"
      _ => "wrong"
    }
  }
  label
}

print(describe(1))
"#,
    )
    .expect_run_err_contains("unknown semantic type identity");
}

// ADR-009 A3 (S3, identity stability): comptime executes once PER
// instantiation — identity(1) and identity("s") trigger two separate
// specialized-body compiles, hence two separate comptime runs. Both observe
// the SAME declaration-scoped Parameter identity (owner = BASE fn name, never
// the mono key), so both print "parameter". Public mirror of the unit pins in
// type_reflection/tests.rs (parameter_identity_is_scoped_by_owning_function,
// specialization_overlay_identity_is_stable_across_instantiations).
// NOTE: because comptime runs per instantiation, side-effectful comptime
// (e.g. warning()) in a generic body duplicates by design — do not dedupe.
#[test]
fn parameter_category_is_stable_across_instantiations_of_one_generic_fn() {
    let source = r#"
fn identity<T>(value: T) -> string {
  let label = comptime {
    match type_category(type_ref(T)) {
      FrozenTypeCategory::Parameter => "parameter"
      _ => "wrong"
    }
  }
  label
}

print(identity(1))
print(identity("s"))
"#;
    expect_vm_and_jit_output(source, "parameter\nparameter");
}

// ADR-009 A3 (S3, distinct owners): two different generic functions each
// observe Parameter for their own declared type parameter. The per-owner
// identity DISTINCTNESS (first's T != second's U as frozen identities) is
// pinned at the unit level (type_reflection/tests.rs) — categories alone
// cannot distinguish identities publicly, and TypeParamDescriptor payloads
// are ticket B7 scope.
#[test]
fn distinct_generic_fns_each_observe_parameter_for_their_own_type_param() {
    let source = r#"
fn first<T>(value: T) -> string {
  let label = comptime {
    match type_category(type_ref(T)) {
      FrozenTypeCategory::Parameter => "first:parameter"
      _ => "first:wrong"
    }
  }
  label
}

fn second<U>(value: U) -> string {
  let label = comptime {
    match type_category(type_ref(U)) {
      FrozenTypeCategory::Parameter => "second:parameter"
      _ => "second:wrong"
    }
  }
  label
}

print(first(1))
print(second(true))
"#;
    expect_vm_and_jit_output(source, "first:parameter\nsecond:parameter");
}

// ADR-009 A3 (review round 1, finding 2): a generic extend method resolves
// through the CALL-SITE specialization path (`__w24_method_*` in
// function_calls.rs), NOT through `monomorphization/cache.rs` — the overlay
// wired only around the cache.rs compile sites left this path hard-failing
// the freeze ("unknown semantic type identity") for `type_ref(N)` on the
// method's own declared type parameter. Worse, the failed compile left a
// registered-but-never-compiled specialization behind, and the D-γ probe's
// swallowed Err let the second resolution attempt short-circuit onto that
// empty body (silent wrong output). Two call sites below exercise both the
// fresh-compile path and the specialization-reuse short-circuit.
#[test]
fn generic_extend_method_body_observes_parameter_category_for_its_own_type_param() {
    let source = r#"
extend Number {
  method describe<N>(other: N) -> string {
    let label = comptime {
      match type_category(type_ref(N)) {
        FrozenTypeCategory::Parameter => "parameter"
        _ => "wrong"
      }
    }
    label
  }
}

print((1.5).describe(2.5))
print((3.5).describe(4.5))
"#;
    expect_vm_and_jit_output(source, "parameter\nparameter");
}

// ADR-009 A3 (review round 1, finding 1): a call-site-nested specialization
// compiled INSIDE a generic body must not resolve the ENCLOSING function's
// type parameters. Pre-fix, the enclosing specialization's overlay stayed
// active across the nested `compile_function`, so `type_ref(T)` inside the
// extend method's body falsely resolved to `outer`'s Parameter identity —
// a false accept violating the negative-proof guarantee (spec §3.1
// surface-and-stop). The freeze must reject with the NAMED diagnostic.
#[test]
fn nested_specialization_cannot_resolve_enclosing_generic_type_param() {
    ShapeTest::new(
        r#"
extend Number {
  method leak<N>(other: N) -> string {
    let label = comptime {
      match type_category(type_ref(T)) {
        FrozenTypeCategory::Parameter => "parameter"
        _ => "wrong"
      }
    }
    label
  }
}

fn outer<T>(value: T) -> string {
  (1.5).leak(2.5)
}

print(outer(1))
"#,
    )
    .expect_run_err_contains("unknown semantic type identity");
}

// ADR-009 A3 (review round 1, finding 2 regression pin): a genuinely
// UNDECLARED name inside a generic extend method body surfaces the real
// freeze diagnostic through the call-site specialization path — never a
// silent dispatch of the registered-but-never-compiled specialized body.
#[test]
fn undeclared_name_in_generic_extend_method_body_fails_the_freeze() {
    ShapeTest::new(
        r#"
extend Number {
  method describe<N>(other: N) -> string {
    let label = comptime {
      match type_category(type_ref(U)) {
        FrozenTypeCategory::Parameter => "parameter"
        _ => "wrong"
      }
    }
    label
  }
}

print((1.5).describe(2.5))
"#,
    )
    .expect_run_err_contains("unknown semantic type identity");
}

// ADR-009 A3 (S3, rejection matrix inside generic bodies): every named
// rejection must keep firing on the specialized-compile path — generic bodies
// only compile per instantiation (functions.rs generic-def skip), so these
// diagnostics surface through S2's hard-error propagation out of
// monomorphization, not at definition compile.

#[test]
fn strings_cannot_construct_type_refs_inside_generic_bodies() {
    ShapeTest::new(
        r#"
fn describe<T>(value: T) -> string {
  let reflected = comptime { type_ref("T") }
  "unreachable"
}

print(describe(1))
"#,
    )
    .expect_run_err_contains("strings cannot construct TypeRef");
}

#[test]
fn type_ref_arity_is_enforced_inside_generic_bodies() {
    for body in ["type_ref()", "type_ref(T, T)"] {
        let source = format!(
            r#"
fn describe<T>(value: T) -> string {{
  let reflected = comptime {{ {body} }}
  "unreachable"
}}

print(describe(1))
"#
        );
        ShapeTest::new(&source).expect_run_err_contains("expects exactly one type argument");
    }
}

#[test]
fn type_ref_is_comptime_only_inside_generic_bodies() {
    ShapeTest::new(
        r#"
fn describe<T>(value: T) -> string {
  let reflected = type_ref(T)
  "unreachable"
}

print(describe(1))
"#,
    )
    .expect_run_err_contains("comptime-only builtin");
}

#[test]
fn raw_type_refs_cannot_escape_generic_bodies_to_runtime_code() {
    ShapeTest::new(
        r#"
fn describe<T>(value: T) -> string {
  let reflected = comptime { type_ref(T) }
  print(reflected)
  "unreachable"
}

print(describe(1))
"#,
    )
    .expect_run_err_contains("TypeRef is a comptime-only compiler capability");
}

#[test]
fn raw_frozen_categories_cannot_escape_generic_bodies_to_runtime_code() {
    ShapeTest::new(
        r#"
fn describe<T>(value: T) -> string {
  let category = comptime { type_category(type_ref(T)) }
  print(category)
  "unreachable"
}

print(describe(1))
"#,
    )
    .expect_run_err_contains("FrozenTypeCategory is comptime-only reflection data");
}

#[test]
fn category_matches_inside_generic_bodies_are_checked_for_exhaustiveness() {
    ShapeTest::new(
        r#"
fn describe<T>(value: T) -> string {
  let label = comptime {
    match type_category(type_ref(T)) {
      FrozenTypeCategory::Parameter => "parameter"
    }
  }
  label
}

print(describe(1))
"#,
    )
    .expect_run_err_contains("Non-exhaustive match");
}

/// ADR-009 §4.1 (ticket A1, slice S2) post-freeze semantics: the semantic
/// freeze is built ONCE per compilation unit at the registration-complete
/// barrier, so a comptime block sees every type declared anywhere in the
/// unit — including aliases and enums declared textually AFTER the block.
/// (Structs were already order-independent via the schema predeclare
/// prepass; aliases and enums previously registered only when their item
/// compiled in pass 2, so the old per-site snapshot rebuild missed them.)
#[test]
fn later_declared_alias_and_enum_are_visible_to_earlier_comptime_blocks() {
    let source = r#"
let alias_label = comptime {
  match type_category(type_ref(LaterUserId)) {
    FrozenTypeCategory::Primitive => "primitive"
    _ => "wrong"
  }
}
let enum_label = comptime {
  match type_category(type_ref(LaterStatus)) {
    FrozenTypeCategory::Nominal => "nominal"
    _ => "wrong"
  }
}

type LaterUserId = int
enum LaterStatus { Ready, Done }

print(alias_label)
print(enum_label)
"#;
    expect_vm_and_jit_output(source, "primitive\nnominal");
}

#[test]
fn strings_cannot_construct_type_refs() {
    ShapeTest::new(
        r#"
let reflected = comptime { type_ref("int") }
print(reflected)
"#,
    )
    .expect_run_err_contains("strings cannot construct TypeRef");
}

#[test]
fn transparent_alias_uses_underlying_type_identity() {
    ShapeTest::new(
        r#"
type UserId = int

let label = comptime {
  match type_category(type_ref(UserId)) {
    FrozenTypeCategory::Primitive => "primitive"
    _ => "wrong"
  }
}

print(label)
"#,
    )
    .expect_output("primitive");
}

#[test]
fn transparent_alias_chains_preserve_nominal_category() {
    ShapeTest::new(
        r#"
type User { id: int }
type Account = User
type CurrentAccount = Account

let label = comptime {
  match type_category(type_ref(CurrentAccount)) {
    FrozenTypeCategory::Nominal => "nominal"
    _ => "wrong"
  }
}

print(label)
"#,
    )
    .expect_output("nominal");
}

#[test]
fn unresolved_type_cannot_cross_freeze_boundary() {
    ShapeTest::new(
        r#"
let reflected = comptime { type_ref(DoesNotExist) }
print(reflected)
"#,
    )
    .expect_run_err_contains("unknown semantic type identity");
}

#[test]
fn non_type_expressions_cannot_construct_type_refs() {
    ShapeTest::new("let reflected = comptime { type_ref(42) }")
        .expect_run_err_contains("expects compiler-resolved type syntax");
}

#[test]
fn type_ref_requires_exactly_one_type_argument() {
    for source in [
        "let reflected = comptime { type_ref() }",
        "let reflected = comptime { type_ref(int, bool) }",
    ] {
        ShapeTest::new(source).expect_run_err_contains("expects exactly one type argument");
    }
}

#[test]
fn type_ref_is_comptime_only() {
    ShapeTest::new("let reflected = type_ref(int)")
        .expect_run_err_contains("comptime-only builtin");
}

#[test]
fn legacy_reflection_descriptors_cannot_forge_type_refs() {
    ShapeTest::new("let category = comptime { type_category(type_info(int).type_ref) }")
        .expect_run_err_contains_any(&["TypeRef", "not compatible", "do not unify"]);
}

#[test]
fn arbitrary_values_cannot_be_used_as_type_refs() {
    ShapeTest::new("let category = comptime { type_category(42) }").expect_run_err_contains_any(&[
        "TypeRef",
        "not compatible",
        "do not unify",
    ]);
}

#[test]
fn raw_type_refs_cannot_escape_to_runtime_code() {
    ShapeTest::new("let reflected = comptime { type_ref(int) }\nprint(reflected)")
        .expect_run_err_contains("TypeRef is a comptime-only compiler capability");
}

#[test]
fn raw_frozen_categories_cannot_escape_to_runtime_code() {
    ShapeTest::new("let category = comptime { type_category(type_ref(int)) }\nprint(category)")
        .expect_run_err_contains("FrozenTypeCategory is comptime-only reflection data");
}

#[test]
fn category_matches_are_checked_for_exhaustiveness() {
    ShapeTest::new(
        r#"
let label = comptime {
  match type_category(type_ref(int)) {
    FrozenTypeCategory::Primitive => "primitive"
  }
}
print(label)
"#,
    )
    .expect_run_err_contains("Non-exhaustive match");
}
