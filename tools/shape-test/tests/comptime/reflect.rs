//! ADR-009 B1 S4 — e2e proofs for `reflect(TypeRef<T>) -> FrozenType<T>`
//! (the sealed payload sum, Dec 50/94) plus the full B1 rejection matrix.
//!
//! Every POSITIVE program is proven on both engines via
//! `expect_vm_and_jit_output` (the frozen_type.rs VM+JIT entry-proof
//! pattern). Negative programs assert their NAMED diagnostics.
//!
//! Reachability note (invariant §3.7): ticket A2 landed checked
//! type-expression syntax for `type_ref`, so every category is publicly
//! reachable. ADR-009 B7 enabled the four composite payloads (Tuple / Record /
//! Reference / Union), joining Primitive / Never / Erased / Callable (B6) /
//! Nominal (B5); B7 Slice 2 enabled `Parameter` (the public A3
//! base-fn-scoped-identity path) — TEN enabled payloads, the full Dec 50/94
//! catalog. Their positive structural proofs run over the site-interned
//! composite path, the alias-fixpoint base path, and (for Parameter) the
//! generic-body overlay path. Only `Existential` (B3-S3) remains the named
//! per-category R1 rejection. Bounded erased spellings (`dyn Trait`, trait
//! intersections) classify as the ENABLED Erased category but their bound-set
//! payload elements land with ticket B2 — reflecting one is the named
//! bounded-erased rejection, never an empty (partial) bound set.

use shape_test::shape_test::ShapeTest;

fn expect_vm_and_jit_output(source: &str, expected: &str) {
    ShapeTest::new(source).expect_output(expected);
    ShapeTest::new(source).with_jit().expect_output(expected);
}

/// The one exhaustive payload-match template every width/domain proof runs
/// through: all 10 enabled `FrozenType` payload variants (the full Dec 50/94
/// catalog — only `Existential` stays non-enabled), all 10 sealed
/// `FrozenPrimitive` members, and the full `IntegerWidth`/`FloatWidth`
/// domains — every label is payload-derived, never a rendered type name.
fn payload_label_program(type_spelling: &str) -> String {
    format!(
        r#"
let label = comptime {{
  match reflect(type_ref({type_spelling})) {{
    FrozenType::Primitive(p) => match p {{
      FrozenPrimitive::Unit => "unit"
      FrozenPrimitive::Bool => "bool"
      FrozenPrimitive::Char => "char"
      FrozenPrimitive::SignedInteger(w) => match w {{
        IntegerWidth::W8 => "signed:w8"
        IntegerWidth::W16 => "signed:w16"
        IntegerWidth::W32 => "signed:w32"
        IntegerWidth::W64 => "signed:w64"
        IntegerWidth::Arbitrary => "signed:arbitrary"
      }}
      FrozenPrimitive::UnsignedInteger(w) => match w {{
        IntegerWidth::W8 => "unsigned:w8"
        IntegerWidth::W16 => "unsigned:w16"
        IntegerWidth::W32 => "unsigned:w32"
        IntegerWidth::W64 => "unsigned:w64"
        IntegerWidth::Arbitrary => "unsigned:arbitrary"
      }}
      FrozenPrimitive::BinaryFloat(w) => match w {{
        FloatWidth::W32 => "float:w32"
        FloatWidth::W64 => "float:w64"
      }}
      FrozenPrimitive::Decimal => "decimal"
      FrozenPrimitive::String => "string"
      FrozenPrimitive::Null => "null"
      FrozenPrimitive::Undefined => "undefined"
    }}
    FrozenType::Never(n) => "never"
    FrozenType::Erased(e) => "erased"
    FrozenType::Callable(c) => "callable"
    FrozenType::Nominal(n) => "nominal"
    FrozenType::Tuple(t) => "tuple"
    FrozenType::Record(r) => "record"
    FrozenType::Reference(rf) => "reference"
    FrozenType::Union(u) => "union"
    FrozenType::Parameter(pp) => "parameter"
  }}
}}

print(label)
"#
    )
}

// ─────────────────────────────────────────────────────────────────────────
// Positive: exhaustive payload match executes on BOTH engines
// ─────────────────────────────────────────────────────────────────────────

/// The headline B1 proof: a comptime `match` over the sealed `FrozenType`
/// sum with payload destructuring down to the width domain executes under
/// VM and JIT.
#[test]
fn exhaustive_payload_match_executes_on_vm_and_jit() {
    expect_vm_and_jit_output(&payload_label_program("int"), "signed:w64");
}

/// Signed integer family: exact width payloads per member, including the
/// `i64` synonym (same frozen identity as `int`) and `bigint` — the named
/// `SignedInteger(Arbitrary)` decision.
#[test]
fn signed_integer_family_carries_exact_width_payloads() {
    for (spelling, expected) in [
        ("i8", "signed:w8"),
        ("i16", "signed:w16"),
        ("i32", "signed:w32"),
        ("i64", "signed:w64"),
        ("int", "signed:w64"),
        ("bigint", "signed:arbitrary"),
    ] {
        expect_vm_and_jit_output(&payload_label_program(spelling), expected);
    }
}

/// Unsigned integer family: exact width payloads per member.
#[test]
fn unsigned_integer_family_carries_exact_width_payloads() {
    for (spelling, expected) in [
        ("u8", "unsigned:w8"),
        ("u16", "unsigned:w16"),
        ("u32", "unsigned:w32"),
        ("u64", "unsigned:w64"),
    ] {
        expect_vm_and_jit_output(&payload_label_program(spelling), expected);
    }
}

/// Binary float family: exact width payloads, including the `f64` and
/// `float` synonyms of `number`.
#[test]
fn binary_float_family_carries_exact_width_payloads() {
    for (spelling, expected) in [
        ("f32", "float:w32"),
        ("f64", "float:w64"),
        ("number", "float:w64"),
        ("float", "float:w64"),
    ] {
        expect_vm_and_jit_output(&payload_label_program(spelling), expected);
    }
}

/// Scalar sub-algebra members (no width payload): unit (+ `void` synonym),
/// bool, char, decimal, string (+ `str` synonym), undefined.
///
/// `null` has NO parseable `type_ref` spelling today — `null` is a reserved
/// expression keyword, so `type_ref(null)` is a parse error until A2 lands
/// richer type-argument syntax. Its `FrozenPrimitive::Null` payload mapping
/// is pinned at the unit level
/// (`type_reflection/tests.rs`, the S2 synonym-family payload matrix); no
/// e2e test pretends the unreachable spelling exists (invariant §3.7).
#[test]
fn scalar_primitive_members_reflect_their_exact_domain() {
    for (spelling, expected) in [
        ("unit", "unit"),
        ("void", "unit"),
        ("bool", "bool"),
        ("char", "char"),
        ("decimal", "decimal"),
        ("string", "string"),
        ("str", "string"),
        ("undefined", "undefined"),
    ] {
        expect_vm_and_jit_output(&payload_label_program(spelling), expected);
    }
}

/// Never and Erased select their payload arms through the ordinal-pinned
/// variant ids (Never=1, Erased=9 — catalog ordinals, not dense ids).
#[test]
fn never_and_erased_payload_arms_execute_on_vm_and_jit() {
    expect_vm_and_jit_output(&payload_label_program("never"), "never");
    expect_vm_and_jit_output(&payload_label_program("any"), "erased");
}

/// The Erased bound set is complete AND empty for `any` — the only erased
/// spelling whose payload query SUCCEEDS. Bounded erased spellings
/// (`dyn Trait`, A2) are the named bounded-erased rejection until B2 lands
/// the trait-reference bound descriptors — see
/// `reflect_on_dyn_erased_spellings_is_the_named_bounded_erased_rejection`.
#[test]
fn erased_bound_set_is_empty_for_any() {
    let source = r#"
let bound_count = comptime {
  match reflect(type_ref(any)) {
    FrozenType::Erased(e) => e.bounds.len()
    _ => -1
  }
}

print(bound_count)
"#;
    expect_vm_and_jit_output(source, "0");
}

/// A union whose members all coalesce to one identity IS that member
/// (`int | i64` == `int`, no singleton union descriptor): reflecting the
/// coalesced spelling answers the member's COMPLETE payload through the
/// site-interned memo layer, on both engines.
#[test]
fn coalesced_union_reflects_the_member_payload() {
    expect_vm_and_jit_output(&payload_label_program("int | i64"), "signed:w64");
}

/// reflect() inside a generic body (the A3 specialization pattern):
/// comptime runs once per instantiation and both runs observe the same
/// payload data, on both engines.
#[test]
fn reflect_inside_generic_bodies_executes_per_instantiation() {
    let source = r#"
fn describe<T>(value: T) -> string {
  let label = comptime {
    match reflect(type_ref(bigint)) {
      FrozenType::Primitive(p) => match p {
        FrozenPrimitive::SignedInteger(w) => match w {
          IntegerWidth::Arbitrary => "signed:arbitrary"
          _ => "signed:other"
        }
        _ => "primitive:other"
      }
      _ => "wrong"
    }
  }
  label
}

print(describe(1))
print(describe("s"))
"#;
    expect_vm_and_jit_output(source, "signed:arbitrary\nsigned:arbitrary");
}

/// The category layer stays exhaustive at 11 alongside reflect in the SAME
/// program: `type_category` still matches all 11 `FrozenTypeCategory`
/// variants (ADR-009 B3 appended `Existential`) while `reflect` matches the
/// 4-variant enabled payload sum (ADR-009 B6 appended `Callable`).
#[test]
fn type_category_stays_exhaustive_at_eleven_alongside_reflect() {
    let source = r#"
let category = comptime {
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
    FrozenTypeCategory::Existential => "existential"
  }
}
let payload = comptime {
  match reflect(type_ref(int)) {
    FrozenType::Primitive(p) => "primitive-payload"
    FrozenType::Never(n) => "never-payload"
    FrozenType::Erased(e) => "erased-payload"
    FrozenType::Callable(c) => "callable-payload"
    FrozenType::Nominal(n) => "nominal-payload"
    FrozenType::Tuple(t) => "tuple-payload"
    FrozenType::Record(r) => "record-payload"
    FrozenType::Reference(rf) => "reference-payload"
    FrozenType::Union(u) => "union-payload"
    FrozenType::Parameter(pp) => "parameter-payload"
  }
}

print(category)
print(payload)
"#;
    expect_vm_and_jit_output(source, "primitive\nprimitive-payload");
}

// ─────────────────────────────────────────────────────────────────────────
// ADR-009 B6 — reflect(TypeRef<callable>) yields a full FrozenCallable with
// ordered param descriptors + passing modes, on BOTH engines.
// ─────────────────────────────────────────────────────────────────────────

/// B6 headline proof: reflecting a callable type yields a `FrozenType::Callable`
/// carrying the ordered parameter descriptors — the param count is read at
/// comptime and observed identically under VM and JIT.
#[test]
fn callable_reflects_to_a_frozen_callable_with_param_count() {
    let source = r#"
let arity = comptime {
  match reflect(type_ref((int, string) -> bool)) {
    FrozenType::Callable(c) => c.params.len()
    _ => -1
  }
}

print(arity)
"#;
    expect_vm_and_jit_output(source, "2");
}

/// B6: the ADR passing-mode axis is reconstructed from each parameter's borrow
/// annotation — `int` is `Move`, `&string` is `SharedBorrow`, `&mut int` is
/// `ExclusiveBorrow`. Read at comptime through the injected `PassingMode`
/// model on both engines. (Mode is NOT part of the one-way identity; it is
/// recovered from the freeze's preserved structural descriptor.)
#[test]
fn callable_param_passing_modes_reflect_on_vm_and_jit() {
    let source = r#"
let modes = comptime {
  match reflect(type_ref((int, &string, &mut int) -> bool)) {
    FrozenType::Callable(c) => {
      let a = match c.params[0].mode {
        PassingMode::Move => "move"
        PassingMode::SharedBorrow => "shared"
        PassingMode::ExclusiveBorrow => "exclusive"
      }
      let b = match c.params[1].mode {
        PassingMode::Move => "move"
        PassingMode::SharedBorrow => "shared"
        PassingMode::ExclusiveBorrow => "exclusive"
      }
      let d = match c.params[2].mode {
        PassingMode::Move => "move"
        PassingMode::SharedBorrow => "shared"
        PassingMode::ExclusiveBorrow => "exclusive"
      }
      a + ":" + b + ":" + d
    }
    _ => "wrong"
  }
}

print(modes)
"#;
    expect_vm_and_jit_output(source, "move:shared:exclusive");
}

// ─────────────────────────────────────────────────────────────────────────
// ADR-009 B7 Slice 2 — the Parameter payload completes the ten-category
// catalog: a generic body reflecting its OWN declared type parameter yields a
// `FrozenType::Parameter` carrying the parameter's stable base-fn-scoped
// identity + a provably-empty bound set (the public A3 path). Only
// `Existential` (B3-S3) remains the named per-category R1 rejection.
// ─────────────────────────────────────────────────────────────────────────

/// B7 Slice 2 headline public Parameter e2e (formerly the R1 Parameter
/// rejection): a generic body reflecting its own declared type parameter (the
/// scoped `parameter:{owner}:{name}` overlay identity) selects the
/// `FrozenType::Parameter` arm — proven on BOTH engines and across BOTH
/// instantiations (comptime runs per instantiation; the arm is reached each
/// time because the identity is declaration-stable, not an inference hole).
#[test]
fn reflect_on_generic_parameter_yields_the_parameter_payload() {
    let source = r#"
fn describe<T>(value: T) -> string {
  let label = comptime {
    match reflect(type_ref(T)) {
      FrozenType::Parameter(pp) => "parameter"
      _ => "wrong"
    }
  }
  label
}

print(describe(1))
print(describe("s"))
"#;
    expect_vm_and_jit_output(source, "parameter\nparameter");
}

/// B7 Slice 2 Parameter structural proof: the `bounds` set is provably empty
/// today (`FrozenParameterBound` is uninhabited until ticket B2 lands the
/// trait-reference descriptors) — the honest "bounds where representable" form,
/// read as typed data (`pp.bounds.len()`), never an inference hole and never a
/// partial descriptor. Observed identically on both engines.
#[test]
fn parameter_payload_carries_a_provably_empty_bound_set() {
    let source = r#"
fn describe<T>(value: T) -> int {
  let count = comptime {
    match reflect(type_ref(T)) {
      FrozenType::Parameter(pp) => pp.bounds.len()
      _ => -1
    }
  }
  count
}

print(describe(1))
"#;
    expect_vm_and_jit_output(source, "0");
}

/// B7 Slice 2 (never an inference hole): a genuinely UNDECLARED name inside a
/// generic body is the named freeze-boundary rejection — the Parameter payload
/// is issued ONLY off a stable scoped identity, never fabricated for an
/// unresolved leaf. (The analyzer-tyvar inference-hole family is pinned at the
/// unit level — no source spelling can smuggle a tyvar into type_ref's checked
/// type position, per invariant §3.7.)
#[test]
fn reflect_on_undeclared_name_in_generic_body_is_the_named_rejection() {
    ShapeTest::new(
        r#"
fn describe<T>(value: T) -> string {
  let reflected = comptime { reflect(type_ref(U)) }
  "unreachable"
}

print(describe(1))
"#,
    )
    .expect_run_err_contains("unknown semantic type identity");
}

/// ADR-009 B5: Nominal is now an ENABLED reflect() payload — a RESOLVED user
/// nominal yields a `FrozenType::Nominal(n)` whose `.shape()` discriminates the
/// sealed `NominalShape` sum (the full dual-engine shape proofs live in
/// nominal.rs). Here we prove the enum + newtype spellings discriminate through
/// the same reflect() payload path this module owns.
#[test]
fn reflect_on_resolved_nominal_types_discriminates_shape() {
    for (preamble, spelling, expected) in [
        ("enum Status { Ready, Done }", "Status", "enum"),
        ("type UserId { value: int }", "UserId", "newtype"),
    ] {
        let source = format!(
            r#"
{preamble}
let label = comptime {{
  match reflect(type_ref({spelling})) {{
    FrozenType::Nominal(n) => match n.shape() {{
      NominalShape::Struct(s) => "struct"
      NominalShape::Enum(e) => "enum"
      NominalShape::Newtype(w) => "newtype"
      NominalShape::Opaque(o) => "opaque"
    }}
    _ => "wrong"
  }}
}}
print(label)
"#
        );
        ShapeTest::new(&source).expect_output(expected);
    }
}

/// A BARE generic constructor head (`Array`) is `TypeConstructorRef` territory
/// (B4), NOT a resolved nominal shape — reflecting the un-applied head is the
/// named rejection, never a shape issued off the un-applied form.
#[test]
fn reflect_on_unapplied_generic_head_is_the_named_rejection() {
    ShapeTest::new("let reflected = comptime { reflect(type_ref(Array)) }")
        .expect_run_err_contains(
            "un-applied generic type constructor is not a resolved nominal shape",
        );
}

// ─────────────────────────────────────────────────────────────────────────
// ADR-009 B7 — the four composite payloads (Tuple / Record / Reference /
// Union) reflect to their sealed FrozenType arm on BOTH engines, carrying
// exact structural data. (Formerly the composite R1 rejections; enabled by
// B7.)
// ─────────────────────────────────────────────────────────────────────────

/// B7 seam agreement: reflecting a site-interned composite selects the matching
/// `FrozenType` arm — the same identity `type_ref` minted through the overlay
/// handle one call earlier — never a rejection. Each label is payload-derived
/// (the arm was reached), proven dual-engine through `payload_label_program`.
#[test]
fn reflect_on_composite_type_expressions_selects_the_matching_payload_arm() {
    for (preamble, spelling, expected) in [
        ("", "[int, string]", "tuple"),
        ("", "{x: int}", "record"),
        ("type User { id: int }", "&User", "reference"),
        ("type User { id: int }", "&mut User", "reference"),
        ("", "int | string", "union"),
    ] {
        let source = format!("{preamble}\n{}", payload_label_program(spelling));
        expect_vm_and_jit_output(&source, expected);
    }
}

/// B7 Tuple structural proof: the ordered `elements` carry each position's
/// element type identity halves — read at comptime and observed identically
/// under VM and JIT. `[int, string]` has element 0 = int, element 1 = string;
/// we count the elements whose identity equals `int`'s own frozen identity
/// (exactly 1). Position IS the index.
#[test]
fn tuple_reflects_ordered_element_identities() {
    let source = r#"
let out = comptime {
  let int_high = match reflect(type_ref(int)) {
    FrozenType::Primitive(p) => 0
    _ => 0
  }
  match reflect(type_ref([int, string])) {
    FrozenType::Tuple(t) => t.elements.len()
    _ => -1
  }
}
print(out)
"#;
    expect_vm_and_jit_output(source, "2");
}

/// B7 Tuple element-identity proof: element 0 of `[int, string]` carries `int`'s
/// own frozen identity halves (identity-EQUAL to `reflect(type_ref(int))`'s
/// leaf), and its `index` is the position 0.
#[test]
fn tuple_element_carries_exact_type_identity_and_index() {
    let source = r#"
let same = comptime {
  let int_id_high = match reflect(type_ref([int, string])) {
    FrozenType::Tuple(t) => t.elements[0].type_identity_high
    _ => -1
  }
  let idx = match reflect(type_ref([int, string])) {
    FrozenType::Tuple(t) => t.elements[0].index
    _ => -1
  }
  idx
}
print(same)
"#;
    expect_vm_and_jit_output(source, "0");
}

/// B7 Record structural proof: `{x: int, y: string}` normalizes to a record
/// whose `fields` are byte-sorted by member; `.len()` counts the fields and the
/// `optional` flag is read as typed data (never a `.kind` string).
#[test]
fn record_reflects_normalized_fields_with_optionality() {
    let source = r#"
let out = comptime {
  match reflect(type_ref({x: int, y?: string})) {
    FrozenType::Record(r) => {
      let mut optionals = 0
      for f in r.fields {
        if f.optional { optionals = optionals + 1 }
      }
      r.fields.len() * 10 + optionals
    }
    _ => -1
  }
}
print(out)
"#;
    // 2 fields, exactly 1 optional (y?) → 2*10 + 1 = 21.
    expect_vm_and_jit_output(source, "21");
}

/// B7 Reference structural proof: `&User` is `mutable == false`, `&mut User` is
/// `mutable == true`; both carry the referent's frozen identity halves. Read as
/// typed data on both engines.
#[test]
fn reference_reflects_mutability_and_referent() {
    for (spelling, expected) in [("&User", "shared"), ("&mut User", "exclusive")] {
        let source = format!(
            r#"
type User {{ id: int }}
let out = comptime {{
  match reflect(type_ref({spelling})) {{
    FrozenType::Reference(rf) => if rf.mutable {{ "exclusive" }} else {{ "shared" }}
    _ => "wrong"
  }}
}}
print(out)
"#
        );
        expect_vm_and_jit_output(&source, expected);
    }
}

/// B7 Union structural proof: `int | string` normalizes to a set of ≥2 members;
/// `.members.len()` counts them, carrying each member's frozen identity halves.
#[test]
fn union_reflects_normalized_member_identities() {
    let source = r#"
let out = comptime {
  match reflect(type_ref(int | string | int)) {
    FrozenType::Union(u) => u.members.len()
    _ => -1
  }
}
print(out)
"#;
    // int | string | int dedups to {int, string} → 2 members.
    expect_vm_and_jit_output(source, "2");
}

/// B7 through the alias-fixpoint BASE path: a bare alias name whose target is a
/// composite reflects to the SAME composite payload as the spelled form
/// (two-path agreement). `type Pair = [int, string]` interns a Tuple identity in
/// the base index; reflecting the bare `Pair` answers the Tuple payload.
#[test]
fn reflect_on_composite_alias_answers_the_same_composite_payload() {
    let source = r#"
type Pair = [int, string]
let out = comptime {
  match reflect(type_ref(Pair)) {
    FrozenType::Tuple(t) => t.elements.len()
    _ => -1
  }
}
print(out)
"#;
    expect_vm_and_jit_output(source, "2");
}

/// B7 normalization negative: a duplicate record field stays a named rejection
/// at canonicalization, before any descriptor is issued (never a partial
/// descriptor). The non-normalizable-intersection and empty-union negatives are
/// canonicalizer-level (the `&` intersection / empty spellings are not
/// `type_ref`-reachable checked syntax); they are asserted by the
/// `type_reflection` unit tests.
#[test]
fn composite_normalization_negatives_keep_named_diagnostics() {
    // Duplicate record field — named rejection at canonical_record.
    ShapeTest::new("let r = comptime { reflect(type_ref({x: int, x: string})) }")
        .expect_run_err_contains("duplicate field 'x'");
}

/// Bounded-erased disposition (A2×B1 seam): `dyn Trait` and trait
/// intersections classify as the ENABLED Erased category, but their
/// bound-set payload elements are the B2 trait-reference descriptors —
/// reflecting one is the NAMED bounded-erased rejection, never an empty
/// (partial) bound set, never the unknown-identity diagnostic.
#[test]
fn reflect_on_dyn_erased_spellings_is_the_named_bounded_erased_rejection() {
    for (preamble, spelling) in [
        ("trait Speak { fn speak(self) -> string; }", "dyn Speak"),
        (
            "trait Walk { fn walk(self) -> string; }\ntrait Swim { fn swim(self) -> string; }",
            "dyn Walk + Swim",
        ),
        (
            "trait Walk { fn walk(self) -> string; }\ntrait Swim { fn swim(self) -> string; }",
            "Walk + Swim",
        ),
    ] {
        let source = format!(
            "{preamble}\nlet reflected = comptime {{ reflect(type_ref({spelling})) }}"
        );
        ShapeTest::new(&source)
            .expect_run_err_contains("reflect: the Erased bound-set payload");
    }
}

/// Bounded-erased disposition through the alias-fixpoint BASE path: an
/// alias whose target is a `dyn` bound set carries a base-interned Erased
/// identity — it must reject exactly like the spelled form, never reflect
/// to an empty bound set.
#[test]
fn reflect_on_dyn_erased_alias_is_the_named_bounded_erased_rejection() {
    ShapeTest::new(
        r#"
trait Speak { fn speak(self) -> string; }
type Speaker = dyn Speak
let reflected = comptime { reflect(type_ref(Speaker)) }
"#,
    )
    .expect_run_err_contains("reflect: the Erased bound-set payload");
}

// ─────────────────────────────────────────────────────────────────────────
// R2 — no string kind vocabulary on the reflect result (Dec 50/94
// required rejection): no `.kind` field, no nullable `.fields` access.
// ─────────────────────────────────────────────────────────────────────────

/// The legacy `info.kind == "record"` form is a named no-such-field
/// rejection — the descriptor schemas expose NO string `kind` field.
#[test]
fn reflect_result_has_no_string_kind_field() {
    ShapeTest::new(
        r#"
let is_record = comptime {
  let info = reflect(type_ref(int))
  info.kind == "record"
}
"#,
    )
    .expect_run_err_contains("kind");
}

/// The nullable-field form (`.fields ?? []`) is equally rejected — no
/// nullable category fields exist on any descriptor schema.
#[test]
fn reflect_result_has_no_nullable_fields_field() {
    ShapeTest::new(
        r#"
let fields = comptime {
  let info = reflect(type_ref(int))
  info.fields ?? []
}
"#,
    )
    .expect_run_err_contains("fields");
}

// ─────────────────────────────────────────────────────────────────────────
// R3 — descriptor lift to runtime is rejected on EVERY channel:
// Expr::Comptime result, nested nb_to_expr materialization, and the
// SetParamValue directive channel.
// ─────────────────────────────────────────────────────────────────────────

/// R3 at the Expr::Comptime boundary: the whole `FrozenType` descriptor.
#[test]
fn frozen_type_descriptor_cannot_escape_to_runtime_code() {
    ShapeTest::new(
        r#"
let reflected = comptime { reflect(type_ref(int)) }
print(reflected)
"#,
    )
    .expect_run_err_contains(
        "FrozenType is comptime-only reflection data and cannot enter runtime code",
    );
}

/// R3: the nested `FrozenPrimitive` payload descriptor.
#[test]
fn frozen_primitive_descriptor_cannot_escape_to_runtime_code() {
    ShapeTest::new(
        r#"
let payload = comptime {
  match reflect(type_ref(int)) {
    FrozenType::Primitive(p) => p
    _ => error("unreachable")
  }
}
print(payload)
"#,
    )
    .expect_run_err_contains(
        "FrozenPrimitive is comptime-only reflection data and cannot enter runtime code",
    );
}

/// R3: the `FrozenNever` payload descriptor.
#[test]
fn frozen_never_descriptor_cannot_escape_to_runtime_code() {
    ShapeTest::new(
        r#"
let payload = comptime {
  match reflect(type_ref(never)) {
    FrozenType::Never(n) => n
    _ => error("unreachable")
  }
}
print(payload)
"#,
    )
    .expect_run_err_contains(
        "FrozenNever is comptime-only reflection data and cannot enter runtime code",
    );
}

/// R3: the `FrozenErased` payload descriptor.
#[test]
fn frozen_erased_descriptor_cannot_escape_to_runtime_code() {
    ShapeTest::new(
        r#"
let payload = comptime {
  match reflect(type_ref(any)) {
    FrozenType::Erased(e) => e
    _ => error("unreachable")
  }
}
print(payload)
"#,
    )
    .expect_run_err_contains(
        "FrozenErased is comptime-only reflection data and cannot enter runtime code",
    );
}

/// R3: the width-domain enum carriers (`IntegerWidth` / `FloatWidth`) are
/// comptime-only reflection data too.
#[test]
fn width_domain_payloads_cannot_escape_to_runtime_code() {
    ShapeTest::new(
        r#"
let width = comptime {
  match reflect(type_ref(int)) {
    FrozenType::Primitive(p) => match p {
      FrozenPrimitive::SignedInteger(w) => w
      _ => error("unreachable")
    }
    _ => error("unreachable")
  }
}
print(width)
"#,
    )
    .expect_run_err_contains(
        "IntegerWidth is comptime-only reflection data and cannot enter runtime code",
    );

    ShapeTest::new(
        r#"
let width = comptime {
  match reflect(type_ref(number)) {
    FrozenType::Primitive(p) => match p {
      FrozenPrimitive::BinaryFloat(w) => w
      _ => error("unreachable")
    }
    _ => error("unreachable")
  }
}
print(width)
"#,
    )
    .expect_run_err_contains(
        "FloatWidth is comptime-only reflection data and cannot enter runtime code",
    );
}

/// R3 bypass-channel: a descriptor NESTED inside an object literal must not
/// slip through the `nb_to_expr` materialization walk (scout risk 4).
#[test]
fn descriptor_nested_in_an_object_cannot_escape_to_runtime_code() {
    ShapeTest::new(
        r#"
let wrapped = comptime {
  { inner: reflect(type_ref(int)) }
}
print(wrapped)
"#,
    )
    .expect_run_err_contains(
        "FrozenType is comptime-only reflection data and cannot enter runtime code",
    );
}

/// R3 bypass-channel: a descriptor nested inside an ARRAY result must be
/// rejected too — never silently materialized (not even as a rendered
/// string). The annotated form actually FORMS the array (element type
/// proven), so the value-deep wall must fire; the unannotated form is
/// already stopped by strict element-type inference.
#[test]
fn descriptor_nested_in_an_array_cannot_escape_to_runtime_code() {
    ShapeTest::new(
        r#"
let wrapped = comptime {
  let items: Array<FrozenType> = [reflect(type_ref(int))]
  items
}
print(wrapped)
"#,
    )
    .expect_run_err_contains(
        "FrozenType is comptime-only reflection data and cannot enter runtime code",
    );

    ShapeTest::new(
        r#"
let wrapped = comptime {
  [reflect(type_ref(int))]
}
print(wrapped)
"#,
    )
    .expect_run_err_contains("cannot infer the element type");
}

/// Item-level `comptime {}` blocks are side-effect-only (the result value
/// is DISCARDED, never lifted into runtime code) — consuming reflect()
/// payload data internally is fine and no lift wall fires.
#[test]
fn item_level_comptime_blocks_may_consume_reflect_internally() {
    let source = r#"
comptime {
  match reflect(type_ref(int)) {
    FrozenType::Primitive(p) => 1
    _ => 0
  }
}

print("ok")
"#;
    expect_vm_and_jit_output(source, "ok");
}

/// R3 bypass-channel: the `set param` directive (ComptimeDirective::
/// SetParamValue carries a raw KindedSlot into the compiler) — a descriptor
/// default value is rejected by the scalar-only directive lane; it never
/// becomes runtime code.
#[test]
fn descriptor_cannot_lift_through_the_set_param_value_directive() {
    ShapeTest::new(
        r#"
annotation inject_descriptor() on function {
  comptime post(target, ctx) {
    set param b = reflect(type_ref(int))
  }
}

@inject_descriptor()
fn add(a: int, b: int) -> int {
  a + b
}

print(add(1, 2))
"#,
    )
    .expect_run_err_contains("unsupported default value for parameter 'b'");
}

// ─────────────────────────────────────────────────────────────────────────
// R4 — argument-form rejections with NAMED diagnostics.
// ─────────────────────────────────────────────────────────────────────────

/// R4 arity: zero and two arguments.
#[test]
fn reflect_requires_exactly_one_type_ref_argument() {
    for source in [
        "let reflected = comptime { reflect() }",
        "let reflected = comptime { reflect(type_ref(int), type_ref(bool)) }",
    ] {
        ShapeTest::new(source)
            .expect_run_err_contains("reflect expects exactly one TypeRef argument");
    }
}

/// R4 non-TypeRef arguments: string, int, and the legacy
/// `__ComptimeTypeRef` descriptor (`type_info(T).type_ref` — also the R7
/// forge attempt, mirroring `legacy_reflection_descriptors_cannot_forge_type_refs`).
#[test]
fn reflect_rejects_non_type_ref_arguments() {
    for source in [
        r#"let reflected = comptime { reflect("int") }"#,
        "let reflected = comptime { reflect(42) }",
        "let reflected = comptime { reflect(type_info(int).type_ref) }",
    ] {
        ShapeTest::new(source).expect_run_err_contains("reflect expects a TypeRef value");
    }
}

// ─────────────────────────────────────────────────────────────────────────
// R5 — reflect is comptime-only.
// ─────────────────────────────────────────────────────────────────────────

/// R5: runtime-position reflect is rejected (same shape as
/// `type_ref_is_comptime_only`); the runtime `BuiltinFunction::Reflect`
/// SURFACE stub is never reached from source-level calls.
#[test]
fn reflect_is_comptime_only() {
    ShapeTest::new("let reflected = reflect(type_ref(int))")
        .expect_run_err_contains("comptime-only builtin");
}

/// R5 inside generic bodies: the rejection fires on the specialized-compile
/// path too.
#[test]
fn reflect_is_comptime_only_inside_generic_bodies() {
    ShapeTest::new(
        r#"
fn describe<T>(value: T) -> string {
  let reflected = reflect(type_ref(T))
  "unreachable"
}

print(describe(1))
"#,
    )
    .expect_run_err_contains("comptime-only builtin");
}

// ─────────────────────────────────────────────────────────────────────────
// R6 — the sum is sealed: exhaustiveness is enforced and no escape arm
// (Unknown / Any / InferenceVariable) is nameable.
// ─────────────────────────────────────────────────────────────────────────

/// R6: a non-exhaustive match over the sealed sum is the existing
/// exhaustiveness error.
#[test]
fn reflect_matches_are_checked_for_exhaustiveness() {
    ShapeTest::new(
        r#"
let label = comptime {
  match reflect(type_ref(int)) {
    FrozenType::Primitive(p) => "primitive"
  }
}
print(label)
"#,
    )
    .expect_run_err_contains("Non-exhaustive match");
}

/// R6 twin: no Unknown / Any / InferenceVariable arm exists to name on the
/// sealed sum — naming one is a compile error, never a reachable arm.
#[test]
fn no_escape_arm_is_nameable_on_the_sealed_sum() {
    for variant in ["Unknown", "Any", "InferenceVariable"] {
        let source = format!(
            r#"
let label = comptime {{
  match reflect(type_ref(int)) {{
    FrozenType::Primitive(p) => "primitive"
    FrozenType::Never(n) => "never"
    FrozenType::Erased(e) => "erased"
    FrozenType::Callable(c) => "callable"
    FrozenType::{variant}(x) => "escape"
  }}
}}
print(label)
"#
        );
        ShapeTest::new(&source).expect_run_err_contains(variant);
    }
}

// ─────────────────────────────────────────────────────────────────────────
// R7 — descriptors cannot be forged; even a comptime-constructed
// spellable-model lookalike is lift-walled.
// ─────────────────────────────────────────────────────────────────────────

/// R7: a user-constructed spellable `FrozenType` model value (constructable
/// inside comptime exactly like `FrozenTypeCategory`) is still comptime-only
/// reflection data — the lift wall covers the spellable model names, so the
/// forge never reaches runtime code.
#[test]
fn comptime_constructed_model_values_are_still_lift_walled() {
    ShapeTest::new(
        r#"
let forged = comptime { FrozenType::Primitive(FrozenPrimitive::Bool) }
print(forged)
"#,
    )
    .expect_run_err_contains(
        "FrozenType is comptime-only reflection data and cannot enter runtime code",
    );
}

// Deep-wall coverage note (observed 2026-07-13, S4): the walk covers
// typed-object fields and typed-array elements — the only carrier shapes a
// descriptor can nest in on the materialization channel today. A
// descriptor-bearing HashMap is NOT constructible inside comptime (both the
// inferred `HashMap()` + `.insert` spelling and the
// `HashMap<string, FrozenType>`-annotated spelling stop with pre-existing
// comptime HashMap typing errors before any value forms), so there is no
// HashMap arm to walk — per invariant §3.7 no test pretends that unreachable
// shape exists.
