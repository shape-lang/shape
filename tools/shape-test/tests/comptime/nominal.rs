//! ADR-009 B5 (Stage 2, Dec 55-58) — the public `FrozenNominal<T>.shape()`
//! discrimination surface.
//!
//! Built on the S1 compiler model: `reflect(type_ref(<nominal>))` yields a
//! `FrozenType::Nominal(n)` carrying a `FrozenNominal` whose `.shape()`
//! projects the sealed `NominalShape` sum
//! (`Struct` / `Enum` / `Newtype` / `Opaque`). Nominal-shape selection is
//! exhaustive and TYPED — a `match` over `NominalShape`, never a `.kind`
//! string (Dec 55 required rejection).
//!
//! Every POSITIVE program runs under the plain interpreter AND the JIT tier
//! (`expect_vm_and_jit_output`): shape discrimination is read at comptime on
//! the comptime VM (comptime never tiers up), and the enclosing program then
//! lowers and runs identically on both engines.
//!
//! S2 adds the member-reflection surface on top of the shape() axis: generic
//! substitution BEFORE descriptor issuance (reflecting an applied user struct
//! `Pair<User>` substitutes field type `T` → `User` before issuing the
//! `FieldDescriptor`s, R10) and derive-style iteration over `record.fields` /
//! `enum.variants` reading the typed descriptor rows — both proven dual-engine.
//! Representation authority (`reflect_repr` + `RepresentationAccess<T>`), the
//! `comptime for some<F,T>` existential field-selection vehicle, and hygienic
//! `#name` member selection are later B5 slices (see `docs/defections.md`).

use shape_test::shape_test::ShapeTest;

fn expect_vm_and_jit_output(source: &str, expected: &str) {
    ShapeTest::new(source).expect_output(expected);
    ShapeTest::new(source).with_jit().expect_output(expected);
}

/// The one exhaustive `NominalShape` match template every discrimination
/// proof runs through: all four sealed shapes, every label projected from the
/// typed shape sum (never a rendered `.kind` string).
fn shape_label_program(preamble: &str, spelling: &str) -> String {
    format!(
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
    )
}

/// A multi-field user `type` is a `NominalShape::Struct`. Proven dual-engine.
#[test]
fn struct_shape_discriminates_on_vm_and_jit() {
    let source = shape_label_program("type User { id: int, name: string }", "User");
    expect_vm_and_jit_output(&source, "struct");
}

/// A user `enum` is a `NominalShape::Enum`. Proven dual-engine.
#[test]
fn enum_shape_discriminates_on_vm_and_jit() {
    let source = shape_label_program("enum Status { Ready, Done }", "Status");
    expect_vm_and_jit_output(&source, "enum");
}

/// A single-field user `type` is a `NominalShape::Newtype` — a nominal wrapper
/// over one inner type (the S1 CURRENT classification absent dedicated newtype
/// syntax; see docs/defections.md). Proven dual-engine.
#[test]
fn newtype_shape_discriminates_on_vm_and_jit() {
    let source = shape_label_program("type UserId { value: int }", "UserId");
    expect_vm_and_jit_output(&source, "newtype");
}

/// A zero-field user `type` has no decomposable representation — it is a
/// `NominalShape::Opaque` (semantically non-decomposable; the S1 CURRENT
/// classification, see docs/defections.md). Proven dual-engine.
#[test]
fn opaque_shape_discriminates_on_vm_and_jit() {
    let source = shape_label_program("type Handle { }", "Handle");
    expect_vm_and_jit_output(&source, "opaque");
}

/// R4 (Dec 55): nominal shape selection is a typed exhaustive `match`, never a
/// `.kind` string comparison. `n.shape` carries no string kind field — a
/// `.kind` access does not resolve, so a `.kind`-string program cannot even be
/// spelled against the sealed descriptor.
#[test]
fn nominal_shape_read_is_typed_not_a_kind_string_on_vm_and_jit() {
    // A struct-shape program that reads a typed inner descriptor (the owner
    // identity halves are ints) — proving the descriptor is real typed data,
    // never a rendered string.
    let source = r#"
type Point { x: int, y: int }
let ok = comptime {
  match reflect(type_ref(Point)) {
    FrozenType::Nominal(n) => match n.shape() {
      NominalShape::Struct(s) => if s.field_count == 2 { "two-fields" } else { "wrong-count" }
      _ => "wrong-shape"
    }
    _ => "wrong"
  }
}

print(ok)
"#;
    expect_vm_and_jit_output(source, "two-fields");
}

/// R10/R11 (drift note): a BARE generic constructor head (`Array`, arity 1) is
/// `TypeConstructorRef` territory, NOT a resolved nominal shape — reflecting an
/// un-applied head is a NAMED rejection (apply its arguments first), never a
/// `StructDescriptor` issued off the un-applied form.
#[test]
fn reflect_on_unapplied_generic_head_is_the_named_rejection() {
    ShapeTest::new("let reflected = comptime { reflect(type_ref(Array)) }")
        .expect_run_err_contains(
            "un-applied generic type constructor is not a resolved nominal shape",
        );
}

/// R10 (Dec 55): reflecting an applied BUILTIN/enum generic (`Option<int>`)
/// stays the named applied-substitution-pending rejection: `Option` is a
/// builtin enum, and the enum freeze projection carries variant name+arity but
/// NOT per-variant payload field TYPES, so there is nothing to substitute into
/// — never a descriptor issued off the un-substituted form. The S2 substitution
/// path (proven below) resolves applied USER STRUCT forms, whose field type
/// annotations ARE frozen.
#[test]
fn reflect_on_applied_enum_generic_is_the_named_pending_rejection() {
    ShapeTest::new("let reflected = comptime { reflect(type_ref(Option<int>)) }")
        .expect_run_err_contains(
            "reflecting an applied generic nominal requires generic substitution",
        );
}

// ===========================================================================
// S2 — member reflection: field / variant iteration + generic substitution.
// ===========================================================================

/// Derive-style field iteration: the `StructDescriptor` a `NominalShape::Struct`
/// carries exposes its ordered `fields` array, iterated with an ordinary
/// comptime `for` over the typed `FieldDescriptor` rows (each row is real typed
/// descriptor data — no `.kind` string, no name string). Proven dual-engine.
#[test]
fn struct_field_iteration_counts_fields_on_vm_and_jit() {
    let source = r#"
type Vec3 { x: int, y: int, z: int }
let out = comptime {
  match reflect(type_ref(Vec3)) {
    FrozenType::Nominal(n) => match n.shape() {
      NominalShape::Struct(s) => {
        let mut c = 0
        for field in s.fields {
          c = c + 1
        }
        c
      }
      _ => 0
    }
    _ => 0
  }
}
print(out)
"#;
    expect_vm_and_jit_output(source, "3");
}

/// Derive-style variant iteration: a `NominalShape::Enum`'s `EnumDescriptor`
/// exposes its ordered `variants` array of typed `VariantDescriptor` rows; each
/// row carries the owner-bound member identity + the payload arity (Unit=0,
/// Tuple(n)=n). Summing `variant_count` + the per-variant arities proves the
/// iteration reads real typed rows. Proven dual-engine.
#[test]
fn enum_variant_iteration_reads_arities_on_vm_and_jit() {
    let source = r#"
enum Color { Red, Green(int), Blue }
let out = comptime {
  match reflect(type_ref(Color)) {
    FrozenType::Nominal(n) => match n.shape() {
      NominalShape::Enum(e) => {
        let mut total = e.variant_count
        for v in e.variants {
          total = total + v.payload_arity
        }
        total
      }
      _ => 0
    }
    _ => 0
  }
}
print(out)
"#;
    // variant_count = 3 (Red, Green, Blue); arities 0 + 1 + 0 = 1; total = 4.
    expect_vm_and_jit_output(source, "4");
}

/// R10 headline (Dec 55): generic substitution PRECEDES descriptor issuance.
/// Reflecting the applied user struct `Pair<User>` substitutes the field type
/// parameter `T` with `User` BEFORE issuing the `FieldDescriptor`s — so the
/// `first: T` field's frozen VALUE-type identity becomes `User`'s frozen
/// identity, never the un-substituted `T` parameter identity. Derive-style
/// iteration counts exactly the fields whose substituted type identity equals
/// `User`'s own frozen identity (read off `reflect(type_ref(User))`). `second:
/// int` does not match, so the count is exactly 1. Proven dual-engine.
#[test]
fn generic_substitution_precedes_field_issuance_on_vm_and_jit() {
    let source = r#"
type User { id: int, name: string }
type Pair<T> { first: T, second: int }
let out = comptime {
  let user_high = match reflect(type_ref(User)) {
    FrozenType::Nominal(n) => match n.shape() {
      NominalShape::Struct(s) => s.owner_identity_high
      _ => 0
    }
    _ => 0
  }
  let user_low = match reflect(type_ref(User)) {
    FrozenType::Nominal(n) => match n.shape() {
      NominalShape::Struct(s) => s.owner_identity_low
      _ => 0
    }
    _ => 0
  }
  match reflect(type_ref(Pair<User>)) {
    FrozenType::Nominal(n) => match n.shape() {
      NominalShape::Struct(s) => {
        let mut count = 0
        for field in s.fields {
          if field.type_identity_high == user_high {
            if field.type_identity_low == user_low {
              count = count + 1
            }
          }
        }
        count
      }
      _ => 0
    }
    _ => 0
  }
}
print(out)
"#;
    expect_vm_and_jit_output(source, "1");
}

// ===========================================================================
// S3 — RepresentationAccess<T> authority + reflect_repr + rejection matrix.
// ===========================================================================

fn expect_vm_and_jit_number(source: &str, expected: f64) {
    ShapeTest::new(source).expect_number(expected);
    ShapeTest::new(source).with_jit().expect_number(expected);
}

/// Dec 56 headline: complete nominal-shape reflection requires a compiler-issued
/// `RepresentationAccess<T>`. Applying an annotation at a `type` declaration is
/// explicit author consent — the compiler mints the authority bound to that
/// type's frozen identity and delivers it to the handler's third positional
/// `access` parameter. The handler reflects the COMPLETE representation with
/// `reflect_repr(type_ref(User), access)` and generates a function the user body
/// calls. Proven dual-engine (the generated fn compiles + runs identically on VM
/// and JIT; the comptime handler itself runs on the never-tiered comptime VM).
#[test]
fn reflect_repr_with_authority_exposes_complete_shape_on_vm_and_jit() {
    let source = r#"
annotation derive_field_count() {
  targets: [type]
  comptime post(target, ctx, access) {
    let count = match reflect_repr(type_ref(User), access) {
      FrozenType::Nominal(n) => match n.shape() {
        NominalShape::Struct(s) => s.field_count
        _ => 0
      }
      _ => 0
    }
    extend (item_fn("user_field_count", "int", count))
  }
}

@derive_field_count()
type User { id: int, name: string }

fn show() -> int { user_field_count() }
show()
"#;
    expect_vm_and_jit_number(source, 2.0);
}

/// R6 (Dec 56): `reflect_repr` without a genuine `RepresentationAccess<T>` is a
/// NAMED authority rejection — representation reflection is never ambient. An
/// ordinary comptime block has no authority in scope, so passing any ordinary
/// value where the capability is required fails at compile time with the named
/// diagnostic (a pre-execution rejection, identical under both engines).
#[test]
fn reflect_repr_without_authority_is_the_named_r6_rejection() {
    ShapeTest::new(
        r#"
type User { id: int }
let reflected = comptime { reflect_repr(type_ref(User), 0) }
"#,
    )
    .expect_run_err_contains(
        "representation reflection requires explicit RepresentationAccess<T> authority",
    );
}

// Cross-type authority (a `RepresentationAccess<User>` cannot decompose
// `Other`, Dec 56) is proven at the intrinsic level by the
// `representation_access_is_bound_to_its_own_type` unit test in
// type_reflection/tests.rs: the annotation pre-pass swallows handler errors as a
// pre-existing limitation (masking them as a later "Undefined function"), so the
// specific named diagnostic is asserted where it fires — the decoder.

/// R6 arity: `reflect_repr` requires exactly the TypeRef and the authority — a
/// one-argument call (the `reflect` spelling) is a named rejection, never a
/// silent decay to a generic arity error.
#[test]
fn reflect_repr_missing_authority_argument_is_the_named_rejection() {
    ShapeTest::new(
        r#"
type User { id: int }
let reflected = comptime { reflect_repr(type_ref(User)) }
"#,
    )
    .expect_run_err_contains(
        "reflect_repr expects exactly two arguments",
    );
}

/// R7 (Dec 56): ordinary `reflect()` never substitutes a filtered or partial
/// field list — a struct's `reflect().shape().field_count` is the honest full
/// count, exactly what the authority-gated `reflect_repr` reports. Proven inside
/// an annotation hook (the only scope holding authority) by generating a
/// function that returns `1` iff the two counts agree. Proven dual-engine.
#[test]
fn ordinary_reflect_is_not_a_filtered_representation_on_vm_and_jit() {
    let source = r#"
annotation check_agreement() {
  targets: [type]
  comptime post(target, ctx, access) {
    let public_count = match reflect(type_ref(Account)) {
      FrozenType::Nominal(n) => match n.shape() {
        NominalShape::Struct(s) => s.field_count
        _ => -1
      }
      _ => -2
    }
    let complete_count = match reflect_repr(type_ref(Account), access) {
      FrozenType::Nominal(n) => match n.shape() {
        NominalShape::Struct(s) => s.field_count
        _ => -3
      }
      _ => -4
    }
    let agree = if public_count == complete_count { 1 } else { 0 }
    extend (item_fn("reflect_agreement", "int", agree))
  }
}

@check_agreement()
type Account { id: int, name: string, balance: int }

fn show() -> int { reflect_agreement() }
show()
"#;
    expect_vm_and_jit_number(source, 1.0);
}

/// Helper: a comptime program that binds a `StructDescriptor` as `s` and then
/// runs `selection` against it — the shape every member-selection rejection
/// (R1-R5) shares. The forbidden spelling is rejected at comptime prep, so the
/// engine never differs.
fn descriptor_selection_program(selection: &str) -> String {
    format!(
        r#"
type User {{ id: int, name: string }}
let out = comptime {{
  match reflect(type_ref(User)) {{
    FrozenType::Nominal(n) => match n.shape() {{
      NominalShape::Struct(s) => {selection}
      _ => 0
    }}
    _ => 0
  }}
}}
print(out)
"#
    )
}

/// R1 (Dec 57): `record.field("name")` — a source-name STRING is not an
/// owner-bound member identity. Named rejection.
#[test]
fn r1_string_field_selection_is_the_named_rejection() {
    ShapeTest::new(&descriptor_selection_program(r#"{ s.field("id") }"#))
        .expect_run_err_contains("requires an owner-bound member identity");
}

/// R2 (Dec 57): `record.field(0)` — a declaration ORDINAL is not a member
/// identity. Named rejection (same wall as R1).
#[test]
fn r2_ordinal_field_selection_is_the_named_rejection() {
    ShapeTest::new(&descriptor_selection_program("{ s.field(0) }"))
        .expect_run_err_contains("a declaration ordinal");
}

/// The DoD-named `record.field(#name)` hygienic-token selection surface shares
/// the grammar-pending `#ident` selection token with B6's `param(#name)`. Until
/// the token lands, a `#name` selector is a NAMED grammar-pending rejection (the
/// sanctioned tracer, not a raw parse error); iteration stays the CURRENT member
/// vehicle. See docs/defections.md (ADR009-B6 residual).
#[test]
fn hash_token_field_selection_is_the_named_grammar_pending_rejection() {
    ShapeTest::new(&descriptor_selection_program("{ s.field(#id) }"))
        .expect_run_err_contains("not yet a spellable surface");
}

/// R4 (Dec 55): `record.kind` — nominal shape selection is a typed exhaustive
/// `match`, never a `.kind` string read off the descriptor. Named rejection.
#[test]
fn r4_kind_string_read_is_the_named_rejection() {
    ShapeTest::new(&descriptor_selection_program("{ s.kind }"))
        .expect_run_err_contains("nominal shape selection is exhaustive and typed");
}

/// R5 (Dec 55): `record.is_builtin` — a runtime representation class is not a
/// reflection category. R9 shares this wall: `record.is_comptime` (a
/// comptime-field disposition, Dec 58) is likewise not exposed on a shape
/// descriptor. Named rejection.
#[test]
fn r5_r9_runtime_representation_class_read_is_the_named_rejection() {
    ShapeTest::new(&descriptor_selection_program("{ s.is_builtin }"))
        .expect_run_err_contains("are not nominal reflection categories");
    ShapeTest::new(&descriptor_selection_program("{ s.is_comptime }"))
        .expect_run_err_contains("are not nominal reflection categories");
}

/// R9 (Dec 58): a `comptime` field is a const-generic/associated-const member,
/// not a runtime struct field — it is NEVER surfaced in a `StructDescriptor`'s
/// fields. `Money` has one runtime field (`amount`) plus one comptime field
/// (`code`); it reflects as a single-runtime-field `Newtype`, proving the
/// comptime field is excluded (no zero-slot special case, Dec 58 removal is
/// E-track). Proven dual-engine.
#[test]
fn r9_comptime_field_is_not_a_struct_field_on_vm_and_jit() {
    let source = r#"
type Money { amount: int, comptime code: string }
let label = comptime {
  match reflect(type_ref(Money)) {
    FrozenType::Nominal(n) => match n.shape() {
      NominalShape::Struct(s) => "struct"
      NominalShape::Newtype(w) => "newtype"
      NominalShape::Opaque(o) => "opaque"
      _ => "wrong"
    }
    _ => "wrong"
  }
}
print(label)
"#;
    // One runtime field (comptime `code` excluded) → Newtype classification.
    expect_vm_and_jit_output(source, "newtype");
}

/// R11 (§3.1/§3.4, Dec 52): a descriptor is issued only for a nominal the
/// freeze actually froze — reflecting a name the freeze never issued fails at
/// the freeze boundary before user comptime runs, never a runtime fallback. An
/// unknown type name cannot mint a TypeRef in the first place.
#[test]
fn r11_unfrozen_nominal_is_the_freeze_boundary_rejection() {
    ShapeTest::new("let x = comptime { reflect(type_ref(NoSuchTypeXYZ)) }")
        .expect_run_err_contains("unknown semantic type identity");
}
