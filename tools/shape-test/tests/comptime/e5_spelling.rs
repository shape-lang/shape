//! ADR-009 E5 CKPT-1 — end-to-end round-trip proof for applied-generic + bare
//! nominal SPELLING reconstruction through the REAL compiler path (not the unit
//! overlay), on BOTH the VM and JIT engines.
//!
//! What is (and is not) observable here. Under CKPT-1 an applied generic now
//! SPELLS off the frozen memo and therefore STAMPS (the stamp-gate
//! `stamp_for = reconstruct(...).is_ok()` auto-widens); pre-CKPT-1 it fell
//! UNSTAMPED to the `__ComptimeTypeRef.source` reparse. For a VALID `.source`,
//! the stamped identity route and the `.source` reparse produce the SAME
//! spelling, so the two are observationally equivalent at the Shape level — the
//! definitive stamp-vs-reparse witness lives at the unit tier (the
//! `e1_s5_route_proof` pins stamp an UNPARSEABLE `.source`, so a green result
//! can only have come from the identity route). This e2e proves the
//! complementary properties the unit tier cannot: the CKPT-1 forms
//! canonicalize + stamp + are CONSUMABLE through the full comptime handler path
//! on both engines, and the `Array<Option<int>>` NESTING TERMINATES (the
//! identity-indirected recursion + the projection-side recursive sub-expression
//! memoization neither hang nor crash the real compile).

use shape_test::shape_test::ShapeTest;

fn expect_vm_and_jit_output(source: &str, expected: &str) {
    ShapeTest::new(source).expect_output(expected);
    ShapeTest::new(source).with_jit().expect_output(expected);
}

/// Consume a `type_ref({spelling})` through the exhaustive `type_category`
/// match; every CKPT-1 applied form is `Nominal`. Green proves the form
/// canonicalizes + stamps + is consumable end-to-end on VM and JIT.
fn expect_applied_is_nominal(spelling: &str) {
    let source = format!(
        r#"
let label = comptime {{
  match type_category(type_ref({spelling})) {{
    FrozenTypeCategory::Nominal => "nominal"
    _ => "wrong"
  }}
}}

print(label)
"#
    );
    expect_vm_and_jit_output(&source, "nominal");
}

#[test]
fn ckpt1_applied_array_type_ref_round_trips_on_both_engines() {
    expect_applied_is_nominal("Array<int>");
}

#[test]
fn ckpt1_applied_option_type_ref_round_trips_on_both_engines() {
    expect_applied_is_nominal("Option<int>");
}

#[test]
fn ckpt1_applied_hashmap_type_ref_round_trips_on_both_engines() {
    expect_applied_is_nominal("HashMap<string, int>");
}

#[test]
fn ckpt1_applied_result_type_ref_round_trips_on_both_engines() {
    expect_applied_is_nominal("Result<int, string>");
}

/// The NESTING canary (ADR-009 E5 A2 identity-indirected-recursion invariant):
/// `Array<Option<int>>` compiles + runs + is consumable on both engines. Green
/// proves the recursive sub-expression memoization (projection.rs) and the
/// identity-indirected reconstruction recursion TERMINATE on the real compile
/// path — no hang, no stack blow-up, no crash.
#[test]
fn ckpt1_nested_applied_array_of_option_type_ref_terminates_on_both_engines() {
    expect_applied_is_nominal("Array<Option<int>>");
}

/// A bare RESOLVED user nominal is `Nominal` and consumable — the CKPT-1
/// `bare_nominal_name_of` spelling arm's end-to-end companion (the un-applied
/// generic HEAD stays a named rejection, A3, pinned at the unit tier).
#[test]
fn ckpt1_bare_user_nominal_type_ref_round_trips_on_both_engines() {
    let source = r#"
type User { id: int }

let label = comptime {
  match type_category(type_ref(User)) {
    FrozenTypeCategory::Nominal => "nominal"
    _ => "wrong"
  }
}

print(label)
"#;
    expect_vm_and_jit_output(source, "nominal");
}

/// Applied generic over a USER struct head (`Box<int>`) — the applied-user-struct
/// arm of `applied_nominal_of` — round-trips on both engines.
#[test]
fn ckpt1_applied_user_generic_type_ref_round_trips_on_both_engines() {
    let source = r#"
type Box<T> { value: T }

let label = comptime {
  match type_category(type_ref(Box<int>)) {
    FrozenTypeCategory::Nominal => "nominal"
    _ => "wrong"
  }
}

print(label)
"#;
    expect_vm_and_jit_output(source, "nominal");
}
