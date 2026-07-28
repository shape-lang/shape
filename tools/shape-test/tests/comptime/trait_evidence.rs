//! ADR-009 ticket B2 (slice S4): public `trait_ref` / `find_impl` surface.
//!
//! Dec 49 positive form: comptime `match find_impl(type_ref(T), trait_ref(Tr))`
//! with the evidence bound in the `Some(proof)` arm, proven under VM AND JIT
//! via the `frozen_type.rs` dual-run convention, plus the `None` arm (R9:
//! an unimplemented pair is `None` — never an error, never partial evidence).
//!
//! S4 pre-flight (plan-mandated): comptime `match` with an Option-payload
//! binding must execute end-to-end in the comptime mini-VM using ONLY
//! pre-B2 surface. If that capability is absent the slice stops and
//! surfaces — no boolean-shaped workaround.
//!
//! Spelling note (S6 defections log): the landed surface is the positional
//! bare-identifier form `trait_ref(Serializable)`, consistent with the landed
//! `type_ref(int)` (ticket A2's checked turbofish syntax is not in this
//! base); Dec 49 spells `trait_ref<Serializable>()`.
//!
//! ## B2 rejection matrix (slice S5) — row → test-name mapping
//!
//! Mirrors the A1 addendum table in
//! `docs/cluster-audits/wave46-typed-comptime-first-tracers.md` (S6 records
//! the final table there):
//!
//! | Row | Forbidden form | Asserting tests |
//! |---|---|---|
//! | R1 | Trait-as-type: `type_ref(TraitName)` | `r1_trait_as_type_is_the_named_traits_are_not_value_types_rejection`; A1-row-2 guard `r1_upgrade_keeps_the_generic_diagnostic_for_unknown_names`; unit `trait_evidence::tests::type_ref_carrier_rejects_a_frozen_trait_identity_with_the_named_r1_text` |
//! | R2 | Type-as-trait: `trait_ref(User)` / `trait_ref(int)` | `r2_type_as_trait_only_a_declared_trait_forms_a_trait_ref` |
//! | R3 | Name-string lookup | `r3_strings_cannot_construct_trait_refs`; `r3_find_impl_rejects_name_string_lookup` |
//! | R4 | Boolean-authorized generation | `r4_boolean_cannot_authorize_implementation_evidence` |
//! | R5 | Evidence escaping its branch (minimal sound: stage boundary + Some-arm-only issuance) | `r5_impl_ref_evidence_cannot_escape_the_comptime_stage_boundary`; Some-arm-only issuance is structural (only `find_impl` builds `ImplRef`, R7 blocks forgery) |
//! | R6 | TraitRef/ImplRef escaping to runtime | `r6_trait_ref_cannot_escape_to_runtime_code`; `r5_impl_ref_evidence_cannot_escape_the_comptime_stage_boundary` |
//! | R7 | Forged evidence (arbitrary values / legacy descriptors) | `r7_arbitrary_values_and_legacy_descriptors_cannot_forge_evidence`; unit `trait_evidence::tests::{trait_ref_carrier_rejects_identities_the_freeze_never_issued, impl_ref_decode_rejects_forged_or_lookalike_evidence}` (structural, S3) |
//! | R8 | Wrong arity / wrong forms | `r8_trait_ref_arity_and_wrong_forms_are_named_rejections`; `r8_find_impl_arity_is_a_named_rejection` |
//! | +  | Post-barrier (comptime-generated) impl queried as evidence | `post_barrier_comptime_generated_impls_are_a_named_stop_not_none`; unit `trait_evidence::tests::post_barrier_registered_pair_is_the_named_ordering_stop_never_a_silent_none` |
//!
//! R9 (unimplemented pair = None) is asserted by
//! `find_impl_unimplemented_pair_is_none_never_an_error` above the matrix.

use shape_test::shape_test::ShapeTest;

fn expect_vm_and_jit_output(source: &str, expected: &str) {
    ShapeTest::new(source).expect_output(expected);
    ShapeTest::new(source).with_jit().expect_output(expected);
}

/// S4 pre-flight: `comptime match` with an Option-payload binding
/// (`Some(v) => …` consuming the bound payload) executes end-to-end in the
/// comptime mini-VM. Exercises only pre-B2 surface.
#[test]
fn preflight_comptime_match_binds_option_payloads() {
    let source = r#"
let n = comptime {
  let x = Some(41)
  match x {
    Some(v) => v + 1
    None => 0
  }
}

print(n)
"#;
    expect_vm_and_jit_output(source, "42");
}

/// Dec 49 canonical positive example (S4 shape): `find_impl` over frozen
/// barrier truth returns compiler-issued evidence; the successful fact is
/// consumed inside the `Some(proof)` match arm, which authorizes the
/// generation branch. Green under VM AND JIT.
#[test]
fn find_impl_some_arm_consumes_evidence_under_vm_and_jit() {
    let source = r#"
trait Greetable {
  method greet() -> string
}

type User { id: int }

impl Greetable for User {
  method greet() -> string {
    return "hi"
  }
}

let generated = comptime {
  match find_impl(type_ref(User), trait_ref(Greetable)) {
    Some(proof) => "serializer:User:Greetable"
    None => "missing-impl"
  }
}

print(generated)
"#;
    expect_vm_and_jit_output(source, "serializer:User:Greetable");
}

/// R9: an unimplemented `(type, trait)` pair is `None` — never an error and
/// never a default/partial `ImplRef`. The `None` arm is an ordinary branch,
/// green under VM AND JIT.
#[test]
fn find_impl_unimplemented_pair_is_none_never_an_error() {
    let source = r#"
trait Greetable {
  method greet() -> string
}

type Ghost { id: int }

let generated = comptime {
  match find_impl(type_ref(Ghost), trait_ref(Greetable)) {
    Some(proof) => "unexpected-evidence"
    None => "no-impl"
  }
}

print(generated)
"#;
    expect_vm_and_jit_output(source, "no-impl");
}

/// The two directions compose in one program: the same comptime surface
/// distinguishes an implemented pair from an unimplemented one — evidence is
/// per-pair, not a global flag.
#[test]
fn find_impl_distinguishes_pairs_within_one_program() {
    let source = r#"
trait Greetable {
  method greet() -> string
}

type User { id: int }
type Ghost { id: int }

impl Greetable for User {
  method greet() -> string {
    return "hi"
  }
}

let user_arm = comptime {
  match find_impl(type_ref(User), trait_ref(Greetable)) {
    Some(proof) => "user:some"
    None => "user:none"
  }
}
let ghost_arm = comptime {
  match find_impl(type_ref(Ghost), trait_ref(Greetable)) {
    Some(proof) => "ghost:some"
    None => "ghost:none"
  }
}

print(user_arm)
print(ghost_arm)
"#;
    expect_vm_and_jit_output(source, "user:some\nghost:none");
}

// ===== S5 rejection matrix R1-R8 (named diagnostics) =====

/// R1: a trait is not a value type — `type_ref(TraitName)` is the NAMED
/// traits-are-not-value-types rejection (upgraded from A1's generic
/// "unknown semantic type identity", which the freeze answered before it
/// knew trait names).
#[test]
fn r1_trait_as_type_is_the_named_traits_are_not_value_types_rejection() {
    ShapeTest::new(
        r#"
trait Greetable {
  method greet() -> string
}

let reflected = comptime { type_ref(Greetable) }
print(reflected)
"#,
    )
    .expect_run_err_contains("traits are not value types");
}

/// R1 guard (A1 row 2 must not regress): a genuinely-unknown name — even
/// with a trait declared in the same unit — keeps the generic
/// unknown-semantic-type-identity diagnostic, NOT the trait diagnostic.
#[test]
fn r1_upgrade_keeps_the_generic_diagnostic_for_unknown_names() {
    ShapeTest::new(
        r#"
trait Greetable {
  method greet() -> string
}

let reflected = comptime { type_ref(DoesNotExist) }
print(reflected)
"#,
    )
    .expect_run_err_contains("unknown semantic type identity");
}

/// R2: only a declared trait forms a TraitRef — a value type (`User`) and a
/// primitive (`int`) are both the named rejection.
#[test]
fn r2_type_as_trait_only_a_declared_trait_forms_a_trait_ref() {
    for source in [
        r#"
type User { id: int }

let t = comptime { trait_ref(User) }
"#,
        "let t = comptime { trait_ref(int) }",
    ] {
        ShapeTest::new(source).expect_run_err_contains("only a declared trait forms a TraitRef");
    }
}

/// R3 (trait half): trait lookup cannot use text — a string literal never
/// constructs a TraitRef (checker-side, mirror of the A1 TypeRef row).
#[test]
fn r3_strings_cannot_construct_trait_refs() {
    ShapeTest::new(
        r#"
trait Greetable {
  method greet() -> string
}

let t = comptime { trait_ref("Greetable") }
"#,
    )
    .expect_run_err_contains("strings cannot construct TraitRef");
}

/// R3 (lookup half): `find_impl` rejects a name string where compiler-issued
/// TypeRef/TraitRef values are required — trait lookup cannot use text.
#[test]
fn r3_find_impl_rejects_name_string_lookup() {
    ShapeTest::new(
        r#"
trait Greetable {
  method greet() -> string
}

type User { id: int }

let r = comptime {
  match find_impl(type_ref(User), "Greetable") {
    Some(proof) => "some"
    None => "none"
  }
}
"#,
    )
    .expect_run_err_contains("trait lookup cannot use text");
}

/// R4: a boolean cannot authorize an operation that requires implementation
/// evidence — a bool in the `find_impl` trait position never unifies with
/// the evidence surface.
#[test]
fn r4_boolean_cannot_authorize_implementation_evidence() {
    ShapeTest::new(
        r#"
trait Greetable {
  method greet() -> string
}

type User { id: int }

impl Greetable for User {
  method greet() -> string {
    return "hi"
  }
}

let r = comptime {
  match find_impl(type_ref(User), true) {
    Some(proof) => "some"
    None => "none"
  }
}
"#,
    )
    .expect_run_err_contains(
        "a boolean cannot authorize an operation that requires implementation evidence",
    );
}

/// R5/R6 (evidence half, minimal sound enforcement): ImplRef evidence bound
/// in the Some arm cannot cross the comptime stage boundary into runtime
/// code — the compiler-issued evidence is comptime-only.
#[test]
fn r5_impl_ref_evidence_cannot_escape_the_comptime_stage_boundary() {
    ShapeTest::new(
        r#"
trait Greetable {
  method greet() -> string
}

type User { id: int }

impl Greetable for User {
  method greet() -> string {
    return "hi"
  }
}

let proof = comptime {
  match find_impl(type_ref(User), trait_ref(Greetable)) {
    Some(proof) => proof
    None => error("pair is implemented")
  }
}
print(proof)
"#,
    )
    .expect_run_err_contains("ImplRef is comptime-only implementation evidence");
}

/// R6 (trait half): a raw TraitRef cannot escape to runtime code.
#[test]
fn r6_trait_ref_cannot_escape_to_runtime_code() {
    ShapeTest::new(
        r#"
trait Greetable {
  method greet() -> string
}

let t = comptime { trait_ref(Greetable) }
print(t)
"#,
    )
    .expect_run_err_contains("TraitRef is a comptime-only compiler capability");
}

/// R7: arbitrary values cannot forge TraitRef/ImplRef evidence — the opaque
/// carriers are distinct types that never unify with scalars (structural, S3;
/// the decode tier is pinned by the trait_evidence.rs unit tests).
#[test]
fn r7_arbitrary_values_and_legacy_descriptors_cannot_forge_evidence() {
    let preamble = r#"
trait Greetable {
  method greet() -> string
}

type User { id: int }
"#;
    for (tail, needle) in [
        (
            r#"let r = comptime {
  match find_impl(type_ref(User), 42) {
    Some(proof) => "some"
    None => "none"
  }
}"#,
            "TraitRef",
        ),
        (
            r#"let r = comptime {
  match find_impl(42, trait_ref(Greetable)) {
    Some(proof) => "some"
    None => "none"
  }
}"#,
            "TypeRef",
        ),
    ] {
        ShapeTest::new(&format!("{preamble}\n{tail}")).expect_run_err_contains_any(&[
            needle,
            "not compatible",
            "do not unify",
        ]);
    }
}

/// R8 (trait_ref): wrong arity and non-trait argument forms are named
/// rejections.
#[test]
fn r8_trait_ref_arity_and_wrong_forms_are_named_rejections() {
    for source in [
        "let t = comptime { trait_ref() }",
        "let t = comptime { trait_ref(Greetable, Greetable) }",
    ] {
        ShapeTest::new(source).expect_run_err_contains("expects exactly one trait argument");
    }
    ShapeTest::new("let t = comptime { trait_ref(42) }")
        .expect_run_err_contains("expects a declared trait named directly");
}

/// R8 (find_impl): wrong arity is the named rejection.
#[test]
fn r8_find_impl_arity_is_a_named_rejection() {
    for source in [
        "let r = comptime { find_impl() }",
        "let r = comptime { find_impl(type_ref(int)) }",
        r#"
trait Greetable {
  method greet() -> string
}
let r = comptime { find_impl(type_ref(int), trait_ref(Greetable), trait_ref(Greetable)) }
"#,
    ] {
        ShapeTest::new(source).expect_run_err_contains("find_impl expects exactly two arguments");
    }
}

/// S1-ruled stance, S5 named diagnostic: an implementation registered AFTER
/// the semantic-freeze barrier (here the J-CT.2 `comptime impl` family) is
/// NOT frozen evidence, and querying its pair is a named surface-and-stop —
/// it must never masquerade as `find_impl -> None`.
#[test]
fn post_barrier_comptime_generated_impls_are_a_named_stop_not_none() {
    ShapeTest::new(
        r#"
comptime trait Greetable {
  method greet() -> string
}

type User { id: int }

comptime impl Greetable for User {
  method greet() -> string {
    return "hi"
  }
}

let r = comptime {
  match find_impl(type_ref(User), trait_ref(Greetable)) {
    Some(proof) => "some"
    None => "none"
  }
}
print(r)
"#,
    )
    .expect_run_err_contains("registered after the semantic-freeze barrier");
}
