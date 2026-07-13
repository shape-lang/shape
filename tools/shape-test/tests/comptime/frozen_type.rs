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

// =========================================================================
// ADR-009 A2 (slice S4): checked type-expression syntax — every composite
// form lowers to a canonical frozen identity at COMPILE time (the rewrite
// canonicalizes through the shared freeze overlay before user comptime
// executes) and its TypeRef is consumable through the EXHAUSTIVE
// FrozenTypeCategory match under both VM and JIT execution.
// =========================================================================

/// Positive per-form proof template: `type_ref({spelling})` consumed via the
/// full 10-arm exhaustive `type_category` match, asserted on VM and JIT.
fn expect_type_ref_category(preamble: &str, spelling: &str, expected: &str) {
    let source = format!(
        r#"
{preamble}
let label = comptime {{
  match type_category(type_ref({spelling})) {{
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
  }}
}}

print(label)
"#
    );
    expect_vm_and_jit_output(&source, expected);
}

#[test]
fn tuple_type_expression_category_is_tuple() {
    expect_type_ref_category("", "[int, string]", "tuple");
}

#[test]
fn record_type_expression_category_is_record() {
    expect_type_ref_category("", "{x: int}", "record");
}

#[test]
fn record_optional_field_type_expression_category_is_record() {
    expect_type_ref_category("", "{x?: int}", "record");
}

#[test]
fn callable_type_expression_category_is_callable() {
    expect_type_ref_category("", "(int) -> bool", "callable");
}

#[test]
fn shared_reference_type_expression_category_is_reference() {
    expect_type_ref_category("type User { id: int }", "&User", "reference");
}

#[test]
fn mutable_reference_type_expression_category_is_reference() {
    expect_type_ref_category("type User { id: int }", "&mut User", "reference");
}

#[test]
fn union_type_expression_category_is_union() {
    expect_type_ref_category("", "int | string", "union");
}

#[test]
fn erased_dyn_trait_type_expression_category_is_erased() {
    expect_type_ref_category(
        "trait Speak { fn speak(self) -> string; }",
        "dyn Speak",
        "erased",
    );
}

#[test]
fn erased_dyn_trait_bound_set_category_is_erased() {
    expect_type_ref_category(
        "trait Walk { fn walk(self) -> string; }\ntrait Swim { fn swim(self) -> string; }",
        "dyn Walk + Swim",
        "erased",
    );
}

#[test]
fn applied_builtin_generic_type_expression_category_is_nominal() {
    expect_type_ref_category("", "Option<int>", "nominal");
}

#[test]
fn applied_array_over_user_type_category_is_nominal() {
    expect_type_ref_category("type User { id: int }", "Array<User>", "nominal");
}

#[test]
fn applied_user_generic_type_expression_category_is_nominal() {
    expect_type_ref_category("type Box<T> { value: T }", "Box<int>", "nominal");
}

#[test]
fn nested_applied_generic_type_expression_category_is_nominal() {
    expect_type_ref_category("", "Option<Array<int>>", "nominal");
}

// ADR-009 A2 (S4): an applied form over the enclosing generic fn's own type
// parameter composes the A3 specialization overlay (pre-substitution
// `parameter:{owner}:{name}` identity embedded in the applied descriptor) —
// stable across instantiations, mirroring the A3 *_inside_generic_bodies
// pattern.
#[test]
fn applied_generic_over_parameter_is_stable_across_instantiations() {
    let source = r#"
fn describe<T>(value: T) -> string {
  let label = comptime {
    match type_category(type_ref(Array<T>)) {
      FrozenTypeCategory::Nominal => "nominal"
      _ => "wrong"
    }
  }
  label
}

print(describe(1))
print(describe("s"))
"#;
    expect_vm_and_jit_output(source, "nominal\nnominal");
}

// ADR-009 A2 (S4, two-path agreement through the S1 alias fixpoint graft):
// a bare alias NAME whose target is a composite (`type Pair = [int, string]`)
// resolves through the bare-Identifier arm to the SAME structural category
// the spelled composite reaches through the type-syntax arm.
#[test]
fn composite_alias_bare_name_agrees_with_spelled_composite() {
    let source = r#"
type Pair = [int, string]

let alias_label = comptime {
  match type_category(type_ref(Pair)) {
    FrozenTypeCategory::Tuple => "tuple"
    _ => "wrong"
  }
}
let spelled_label = comptime {
  match type_category(type_ref([int, string])) {
    FrozenTypeCategory::Tuple => "tuple"
    _ => "wrong"
  }
}

print(alias_label)
print(spelled_label)
"#;
    expect_vm_and_jit_output(source, "tuple\ntuple");
}

// ADR-009 A2 (S4, Dec 52 rejection placement): an unresolved leaf at depth
// inside a composite form rejects at COMPILE time with the named
// unknown-identity diagnostic (the Result-ified rewrite fires before user
// comptime executes; the leaf is named). Full rejection matrix is slice S5.
#[test]
fn unresolved_leaf_inside_composite_cannot_cross_freeze_boundary() {
    ShapeTest::new("let reflected = comptime { type_ref(Option<Bogus>) }")
        .expect_run_err_contains("unknown semantic type identity");
}

// =========================================================================
// ADR-009 A2 (slice S5): the full rejection matrix re-fired over composite
// type-expression forms, plus positive normalization proofs. Every rejection
// asserts its NAMED diagnostic; identity-stability proofs live at the unit
// level (compiler/comptime.rs + type_reflection/tests.rs) where identity
// literals are observable.
// =========================================================================

/// S5 R1: a string spelling a composite type is still a string — the A1
/// row-1 rejection re-fires through the S3 expression fallback.
#[test]
fn composite_spelling_strings_cannot_construct_type_refs() {
    ShapeTest::new(r#"let reflected = comptime { type_ref("Option<int>") }"#)
        .expect_run_err_contains("strings cannot construct TypeRef");
}

/// S5 R2: an unresolved leaf at ANY depth inside a composite form is a
/// compile-time freeze rejection in the unknown-identity family, naming the
/// leaf — tuple, callable, and applied positions alike.
#[test]
fn unresolved_leaf_at_depth_is_rejected_naming_the_leaf_across_composite_forms() {
    for spelling in ["Option<Bogus>", "[int, Bogus]", "(Bogus) -> int"] {
        ShapeTest::new(&format!(
            "let reflected = comptime {{ type_ref({spelling}) }}"
        ))
        .expect_run_err_contains("unknown semantic type identity")
        .expect_run_err_contains("Bogus");
    }
}

/// S5 R2 (dyn case): erased trait objects resolve their bounds against the
/// frozen trait-name set — an undeclared trait is the same named rejection.
#[test]
fn unknown_trait_in_dyn_bound_cannot_cross_freeze_boundary() {
    ShapeTest::new("let reflected = comptime { type_ref(dyn NoSuchTrait) }")
        .expect_run_err_contains("unknown semantic type identity")
        .expect_run_err_contains("NoSuchTrait");
}

/// S5 R3: `_` is NOT inference-hole syntax in Shape — it parses as an
/// ordinary (never-frozen) type name, so both the bare and the nested
/// spelling reject in the unknown-identity family naming `_`. The Dec-52
/// unresolved-inference-variable family is pinned at the unit level
/// (type_reflection/tests.rs::inference_holes_reject_with_freeze_boundary_
/// diagnostic) because no source spelling can smuggle an analyzer tyvar
/// into type_ref's checked type position.
#[test]
fn underscore_is_an_unresolved_type_name_not_an_inference_hole() {
    ShapeTest::new("let reflected = comptime { type_ref(Option<_>) }")
        .expect_run_err_contains("unknown semantic type identity")
        .expect_run_err_contains("_");
    ShapeTest::new("let reflected = comptime { type_ref(_) }")
        .expect_run_err_contains("unknown semantic type identity");
}

/// S5 R5: applied-generic arity mismatches are named compile-time
/// rejections — builtin heads from the freeze's arity table, user-generic
/// heads from their declared type parameters.
#[test]
fn applied_generic_arity_mismatch_is_a_named_rejection() {
    ShapeTest::new("let reflected = comptime { type_ref(Option<int, string>) }")
        .expect_run_err_contains("expects 1 type argument(s), but 2 were provided");
    ShapeTest::new("let reflected = comptime { type_ref(HashMap<int>) }")
        .expect_run_err_contains("expects 2 type argument(s), but 1 were provided");
    ShapeTest::new(
        "type Box<T> { value: T }\nlet reflected = comptime { type_ref(Box<int, string>) }",
    )
    .expect_run_err_contains("expects 1 type argument(s), but 2 were provided");
    ShapeTest::new("type User { id: int }\nlet reflected = comptime { type_ref(User<int>) }")
        .expect_run_err_contains("expects 0 type argument(s), but 1 were provided");
}

/// S5 R6 (per the S3 prove-or-reject decision): const-generic type
/// applications are a named parse-time rejection — no descriptor bytes are
/// minted for a carrier the language cannot yet prove.
#[test]
fn const_generic_application_is_a_named_rejection() {
    ShapeTest::new("let reflected = comptime { type_ref(Array<3>) }")
        .expect_run_err_contains("const-generic type applications are not yet supported in type_ref");
}

/// S5 R8 (Dec 50/94 rule 3): a trait intersection in type position erases to
/// a bound set — same closed category as the dyn spelling.
#[test]
fn trait_intersection_type_expression_category_is_erased() {
    expect_type_ref_category(
        "trait Walk { fn walk(self) -> string; }\ntrait Swim { fn swim(self) -> string; }",
        "Walk + Swim",
        "erased",
    );
}

/// S5 R8: a structural object intersection normalizes to a Record.
#[test]
fn object_intersection_type_expression_category_is_record() {
    expect_type_ref_category("", "{a: int} + {b: string}", "record");
}

/// S5 R8: a mixed object/trait intersection is a named rejection.
#[test]
fn mixed_intersection_is_a_named_rejection() {
    ShapeTest::new(
        "trait Walk { fn walk(self) -> string; }\nlet reflected = comptime { type_ref({a: int} + Walk) }",
    )
    .expect_run_err_contains("intersection");
}

/// S5 R9: non-type expressions re-fire the A1 row-7 rejections through the
/// S3 expression fallback (arithmetic), and a VALUE binding's name is not a
/// frozen TYPE name — it rejects at the freeze boundary.
#[test]
fn arithmetic_and_value_bindings_cannot_construct_type_refs() {
    ShapeTest::new("let reflected = comptime { type_ref(1 + 2) }")
        .expect_run_err_contains("expects compiler-resolved type syntax");
    ShapeTest::new("let x = 5\nlet reflected = comptime { type_ref(x) }")
        .expect_run_err_contains("unknown semantic type identity");
}

/// S5 R12: a TypeRef minted from a composite form is still comptime-only —
/// the escape guards re-fire for the new forms.
#[test]
fn raw_composite_type_refs_cannot_escape_to_runtime_code() {
    ShapeTest::new("let reflected = comptime { type_ref([int, string]) }\nprint(reflected)")
        .expect_run_err_contains("TypeRef is a comptime-only compiler capability");
}

/// S5 R12: same for the category carrier over a composite-formed TypeRef.
#[test]
fn raw_composite_frozen_categories_cannot_escape_to_runtime_code() {
    ShapeTest::new(
        "let category = comptime { type_category(type_ref({x: int})) }\nprint(category)",
    )
    .expect_run_err_contains("FrozenTypeCategory is comptime-only reflection data");
}

/// S5 R14 (legacy confinement): the E5-confined `type_info` path learned
/// NOTHING from A2 — the checked type-argument grammar exists ONLY for
/// `type_ref(`, so a composite spelling under `type_info` parses as a VALUE
/// array literal whose elements are undefined value names (named rejection,
/// never a classified legacy descriptor). The unit-level confinement
/// sentinel (type_reflection/tests.rs) pins the source-level vocabulary.
#[test]
fn legacy_type_info_does_not_learn_composite_forms() {
    ShapeTest::new("let info = comptime { type_info([int, string]) }")
        .expect_run_err_contains("Undefined variable");
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
