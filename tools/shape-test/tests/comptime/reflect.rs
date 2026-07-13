//! ADR-009 B1 S4 — e2e proofs for `reflect(TypeRef<T>) -> FrozenType<T>`
//! (the sealed payload sum, Dec 50/94) plus the full B1 rejection matrix.
//!
//! Every POSITIVE program is proven on both engines via
//! `expect_vm_and_jit_output` (the frozen_type.rs VM+JIT entry-proof
//! pattern). Negative programs assert their NAMED diagnostics.
//!
//! Reachability note (invariant §3.7 — no tests for non-enabled categories'
//! payload structures, and no tests pretending unreachable forms exist):
//! `type_ref` accepts bare identifiers only until ticket A2 lands, so of the
//! 7 non-enabled categories exactly TWO are publicly reachable — Parameter
//! (a generic body's own type parameter) and Nominal (user structs/enums +
//! builtin containers). Both R1 rejections are asserted end-to-end below.
//! The remaining five (Tuple / Record / Callable / Reference / Union) have
//! no `type_ref` spelling yet; their named per-category diagnostics are
//! pinned at the unit level
//! (`type_reflection/tests.rs::non_enabled_categories_reject_with_named_per_category_diagnostics`).

use shape_test::shape_test::ShapeTest;

fn expect_vm_and_jit_output(source: &str, expected: &str) {
    ShapeTest::new(source).expect_output(expected);
    ShapeTest::new(source).with_jit().expect_output(expected);
}

/// The one exhaustive payload-match template every width/domain proof runs
/// through: all 3 enabled `FrozenType` payload variants, all 10 sealed
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

/// The Erased bound set is complete AND empty for `any` — the only
/// reachable erased spelling until A2 lands trait-bound syntax.
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

/// The category layer stays exhaustive at 10 alongside reflect in the SAME
/// program: `type_category` still matches all 10 `FrozenTypeCategory`
/// variants while `reflect` matches the 3-variant payload sum.
#[test]
fn type_category_stays_exhaustive_at_ten_alongside_reflect() {
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
  }
}
let payload = comptime {
  match reflect(type_ref(int)) {
    FrozenType::Primitive(p) => "primitive-payload"
    FrozenType::Never(n) => "never-payload"
    FrozenType::Erased(e) => "erased-payload"
  }
}

print(category)
print(payload)
"#;
    expect_vm_and_jit_output(source, "primitive\nprimitive-payload");
}

// ─────────────────────────────────────────────────────────────────────────
// R1 — reflecting a non-enabled category is the NAMED per-category
// compile-time rejection, never a partial descriptor (sanctioned tracer).
// ─────────────────────────────────────────────────────────────────────────

/// R1 Parameter: a generic body reflecting its own declared type parameter
/// (the scoped `parameter:{owner}:{name}` overlay identity).
#[test]
fn reflect_on_generic_parameter_is_the_named_r1_rejection() {
    ShapeTest::new(
        r#"
fn describe<T>(value: T) -> string {
  let reflected = comptime { reflect(type_ref(T)) }
  "unreachable"
}

print(describe(1))
"#,
    )
    .expect_run_err_contains("reflect: the Parameter payload descriptor has not landed");
}

/// R1 Nominal: user struct, user enum, and builtin container — every
/// Nominal spelling rejects with the SAME named per-category diagnostic
/// pointing at the exhaustive category layer.
#[test]
fn reflect_on_nominal_types_is_the_named_r1_rejection() {
    for source in [
        r#"
type User { id: int }
let reflected = comptime { reflect(type_ref(User)) }
"#,
        r#"
enum Status { Ready, Done }
let reflected = comptime { reflect(type_ref(Status)) }
"#,
        "let reflected = comptime { reflect(type_ref(Array)) }",
    ] {
        ShapeTest::new(source)
            .expect_run_err_contains("reflect: the Nominal payload descriptor has not landed");
    }
}

/// The R1 diagnostic points at the exhaustive category layer by name.
#[test]
fn r1_rejection_points_at_the_exhaustive_category_layer() {
    ShapeTest::new("let reflected = comptime { reflect(type_ref(Array)) }")
        .expect_run_err_contains("use type_category for the exhaustive category");
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
annotation inject_descriptor() {
  targets: [function]
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
