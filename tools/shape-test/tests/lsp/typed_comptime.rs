use shape_test::shape_test::{ShapeTest, pos};

#[test]
fn comptime_completion_offers_typed_reflection_builtins() {
    ShapeTest::new("comptime {\n    \n}\n")
        .at(pos(1, 4))
        .expect_completion("type_ref")
        .expect_completion("type_category");
}

/// ADR-009 B3 (Dec 51): hover over a binding whose type is an existential
/// descriptor package renders the exact `exists<W...> Descriptor<W...>`
/// witness spelling — the shared freeze/render surface reaches the LSP, so a
/// `some`-bound witness type is shown, not erased to `any`/`?`.
#[test]
fn hover_renders_existential_witness_package_type() {
    // `pkg` is annotated with the existential package; hover renders the
    // witness list + inner descriptor via the shared annotation renderer.
    ShapeTest::new("fn describe(pkg: exists<T> FrozenType<T>) -> string {\n  return \"x\"\n}\n")
        .at(pos(0, 13))
        .expect_hover_contains("exists<T>");
}

#[test]
fn runtime_completion_hides_typed_reflection_builtins() {
    ShapeTest::new("")
        .at(pos(0, 0))
        .expect_no_completion("type_ref")
        .expect_no_completion("type_category");
}

#[test]
fn annotation_comptime_hook_offers_typed_reflection_builtins() {
    ShapeTest::new(
        "annotation inspect() {\n  targets: [type]\n  comptime post(target, ctx) {\n    \n  }\n}\n",
    )
    .at(pos(3, 4))
    .expect_completion("type_ref")
    .expect_completion("type_category");
}

#[test]
fn type_ref_hover_explains_opaque_identity() {
    ShapeTest::new("let reflected = comptime { type_ref(int) }\n")
        .at(pos(0, 29))
        .expect_hover_contains("opaque compiler-issued identity");
}

#[test]
fn type_category_hover_exposes_exhaustive_result() {
    ShapeTest::new("let category = comptime { type_category(type_ref(int)) }\n")
        .at(pos(0, 32))
        .expect_hover_contains("exhaustive semantic category");
}

#[test]
fn type_ref_hover_exposes_typed_signature() {
    ShapeTest::new("let reflected = comptime { type_ref(int) }\n")
        .at(pos(0, 29))
        .expect_hover_contains("type_ref(T) -> TypeRef<T>");
}

#[test]
fn typed_reflection_offers_signature_help() {
    ShapeTest::new("let reflected = comptime { type_ref( ) }\n")
        .at(pos(0, 36))
        .expect_signature_help();
}

// =====================================================================
// ADR-009 A2 (S6): completion inside the `type_ref(` TYPE position.
// The argument of type_ref is checked type syntax, so completion routes
// to the type-annotation provider: type names (primitives, user types,
// in-scope generic parameters) — never value bindings.
// =====================================================================

#[test]
fn type_ref_argument_completion_offers_type_names_not_value_bindings() {
    ShapeTest::new(
        "type Point {\n  x: int\n}\nlet count = 1\nlet reflected = comptime { type_ref( ) }\n",
    )
    .at(pos(4, 36))
    .expect_completion("int")
    .expect_completion("Point")
    .expect_completion("Option")
    .expect_no_completion("count")
    .expect_no_completion("reflected");
}

#[test]
fn generic_body_type_ref_argument_completion_offers_type_parameters() {
    ShapeTest::new(
        "fn describe<T>(value: T) -> string {\n  let label = comptime { type_ref( ) }\n  label\n}\n",
    )
    .at(pos(1, 34))
    .expect_completion("T")
    .expect_completion("int")
    .expect_no_completion("value")
    .expect_no_completion("label");
}

#[test]
fn type_ref_hover_documents_the_checked_type_expression_surface() {
    // The catalog-owned builtin row (comptime_reflection.rs) documents the
    // A2 checked type-expression forms; hover renders that row.
    ShapeTest::new("let reflected = comptime { type_ref(int) }\n")
        .at(pos(0, 29))
        .expect_hover_contains("applied generics")
        .expect_hover_contains("opaque compiler-issued identity");
}

#[test]
fn string_type_ref_construction_has_semantic_diagnostic() {
    ShapeTest::new("let reflected = comptime { type_ref(\"int\") }\n")
        .expect_semantic_diagnostic_contains("strings cannot construct TypeRef");
}

#[test]
fn unresolved_type_ref_has_semantic_diagnostic() {
    ShapeTest::new("let reflected = comptime { type_ref(DoesNotExist) }\n")
        .expect_semantic_diagnostic_contains("unknown semantic type identity");
}

/// ADR-009 A2 (S5) LSP mirror: a STRING spelling a composite type is still a
/// string — the named rejection reaches LSP semantic diagnostics through the
/// S3 expression fallback.
#[test]
fn composite_string_type_ref_has_semantic_diagnostic() {
    ShapeTest::new("let reflected = comptime { type_ref(\"Option<int>\") }\n")
        .expect_semantic_diagnostic_contains("strings cannot construct TypeRef");
}

/// ADR-009 A2 (S5) LSP mirror: an unresolved leaf NESTED inside a checked
/// type expression surfaces the named freeze rejection as a semantic
/// diagnostic (compile-time, before user comptime executes — Dec 52).
#[test]
fn nested_unresolved_type_ref_has_semantic_diagnostic() {
    ShapeTest::new("let reflected = comptime { type_ref(Option<Bogus>) }\n")
        .expect_semantic_diagnostic_contains("unknown semantic type identity");
}

#[test]
fn legacy_type_descriptor_has_semantic_diagnostic() {
    ShapeTest::new("let category = comptime { type_category(type_info(int).type_ref) }\n")
        .expect_semantic_diagnostic_contains("TypeRef");
}

#[test]
fn raw_type_ref_escape_has_semantic_diagnostic() {
    ShapeTest::new("let reflected = comptime { type_ref(int) }\nprint(reflected)\n")
        .expect_semantic_diagnostic_contains("comptime-only compiler capability");
}

#[test]
fn frozen_category_completion_is_closed_and_has_no_unknown_arm() {
    ShapeTest::new("let category = comptime { FrozenTypeCategory:: }\n")
        .at(pos(0, 46))
        .expect_completion("Primitive")
        .expect_completion("Erased")
        .expect_no_completion("Unknown");
}

#[test]
fn enum_variant_completion_path_is_not_specific_to_comptime() {
    ShapeTest::new("enum Color { Red, Green }\nlet color = Color::\n")
        .at(pos(1, 19))
        .expect_completion("Red")
        .expect_completion("Green");
}

#[test]
fn frozen_category_completion_filters_partial_variant_prefix() {
    ShapeTest::new("let category = comptime { FrozenTypeCategory::Pr }\n")
        .at(pos(0, 48))
        .expect_completion("Primitive")
        .expect_no_completion("Erased");
}

// =====================================================================
// ADR-009 B1 (S5): reflect + the payload descriptor types. Hover flows
// from the catalog-owned REFLECT_BUILTIN_ROW; completion visibility is
// metadata-driven (is_comptime_builtin_function); descriptor-enum variant
// completion is a catalog-keyed lookup over the shared runtime constants
// (closed lists, no Unknown arm, no hand-written parallel list); semantic
// diagnostics surface the R1/R2/R4/R5 rejection forms through the same
// compile-error plumbing as the A1 rows.
// =====================================================================

#[test]
fn comptime_completion_offers_reflect() {
    ShapeTest::new("comptime {\n    \n}\n")
        .at(pos(1, 4))
        .expect_completion("reflect");
}

#[test]
fn runtime_completion_hides_reflect() {
    ShapeTest::new("")
        .at(pos(0, 0))
        .expect_no_completion("reflect");
}

#[test]
fn annotation_comptime_hook_offers_reflect() {
    ShapeTest::new(
        "annotation inspect() {\n  targets: [type]\n  comptime post(target, ctx) {\n    \n  }\n}\n",
    )
    .at(pos(3, 4))
    .expect_completion("reflect");
}

#[test]
fn reflect_hover_exposes_typed_signature() {
    ShapeTest::new("let reflected = comptime { reflect(type_ref(int)) }\n")
        .at(pos(0, 29))
        .expect_hover_contains("reflect(type_ref: TypeRef<T>) -> FrozenType<T>");
}

#[test]
fn reflect_hover_notes_the_enabled_payload_stage() {
    ShapeTest::new("let reflected = comptime { reflect(type_ref(int)) }\n")
        .at(pos(0, 29))
        .expect_hover_contains("named compile-time rejection");
}

#[test]
fn frozen_type_completion_is_closed_to_enabled_payload_variants() {
    ShapeTest::new("let payload = comptime { FrozenType:: }\n")
        .at(pos(0, 37))
        .expect_completion("Primitive")
        .expect_completion("Never")
        .expect_completion("Erased")
        .expect_no_completion("Unknown")
        .expect_no_completion("Nominal");
}

#[test]
fn frozen_primitive_completion_is_closed_and_has_no_unknown_arm() {
    ShapeTest::new("let payload = comptime { FrozenPrimitive:: }\n")
        .at(pos(0, 42))
        .expect_completion("SignedInteger")
        .expect_completion("BinaryFloat")
        .expect_completion("Decimal")
        .expect_no_completion("Unknown");
}

#[test]
fn frozen_primitive_completion_filters_partial_variant_prefix() {
    ShapeTest::new("let payload = comptime { FrozenPrimitive::Si }\n")
        .at(pos(0, 44))
        .expect_completion("SignedInteger")
        .expect_no_completion("Decimal");
}

#[test]
fn generic_body_comptime_completion_offers_reflect() {
    ShapeTest::new(
        "fn describe<T>(value: T) -> string {\n  let label = comptime {\n    \n  }\n  label\n}\n",
    )
    .at(pos(2, 4))
    .expect_completion("reflect");
}

#[test]
fn generic_body_runtime_position_after_comptime_block_hides_reflect() {
    ShapeTest::new(
        "fn describe<T>(value: T) -> string {\n  let label = comptime {\n    type_ref(T)\n  }\n  \n  label\n}\n",
    )
    .at(pos(4, 2))
    .expect_no_completion("reflect");
}

/// R1 (representative category): reflecting a non-enabled category surfaces
/// the named per-category rejection through the LSP diagnostics path.
#[test]
fn reflect_non_enabled_category_has_semantic_diagnostic() {
    ShapeTest::new("let reflected = comptime { reflect(type_ref(Array)) }\n")
        .expect_semantic_diagnostic_contains(
            "reflect: the Nominal payload descriptor has not landed",
        );
}

/// R2: the legacy string-kind form (`info.kind == "record"`) is a named
/// no-such-field rejection — descriptor schemas expose no `kind` field.
#[test]
fn reflect_string_kind_access_has_semantic_diagnostic() {
    ShapeTest::new(
        "let is_record = comptime {\n  let info = reflect(type_ref(int))\n  info.kind == \"record\"\n}\n",
    )
    .expect_semantic_diagnostic_contains("kind");
}

/// R4: a string argument is the named non-TypeRef rejection.
#[test]
fn reflect_string_argument_has_semantic_diagnostic() {
    ShapeTest::new("let reflected = comptime { reflect(\"int\") }\n")
        .expect_semantic_diagnostic_contains("reflect expects a TypeRef value");
}

/// R5: runtime-position reflect is comptime-only.
#[test]
fn runtime_position_reflect_has_semantic_diagnostic() {
    ShapeTest::new("let reflected = reflect(type_ref(int))\n")
        .expect_semantic_diagnostic_contains("comptime-only builtin");
}

// =====================================================================
// ADR-009 A3 (S5): LSP surface inside comptime blocks in GENERIC fn
// bodies. Generic template bodies are never compiled at definition
// (functions.rs:770-776 skip), so completion/hover MUST come from the
// shared query surface (builtin metadata + comptime-context detection),
// never from a compiled body. Semantic diagnostics come from the
// RecoverAll compile in analyze_program_semantics: specialized-body
// freeze errors surface through the S1 parameter overlay + S2 hard
// propagation when a call site instantiates the generic.
// =====================================================================

#[test]
fn generic_body_comptime_completion_offers_typed_reflection_builtins() {
    ShapeTest::new(
        "fn describe<T>(value: T) -> string {\n  let label = comptime {\n    \n  }\n  label\n}\n",
    )
    .at(pos(2, 4))
    .expect_completion("type_ref")
    .expect_completion("type_category");
}

#[test]
fn generic_body_runtime_position_after_comptime_block_hides_typed_reflection_builtins() {
    // Cursor is INSIDE the generic fn body but AFTER the comptime block has
    // closed — a runtime position. Comptime-only builtins must not leak here.
    ShapeTest::new(
        "fn describe<T>(value: T) -> string {\n  let label = comptime {\n    type_ref(T)\n  }\n  \n  label\n}\n",
    )
    .at(pos(4, 2))
    .expect_no_completion("type_ref")
    .expect_no_completion("type_category");
}

#[test]
fn generic_body_frozen_category_completion_is_closed_and_has_no_unknown_arm() {
    ShapeTest::new(
        "fn describe<T>(value: T) -> string {\n  let label = comptime {\n    match type_category(type_ref(T)) {\n      FrozenTypeCategory::\n    }\n  }\n  label\n}\n",
    )
    .at(pos(3, 26))
    .expect_completion("Parameter")
    .expect_completion("Primitive")
    .expect_no_completion("Unknown");
}

#[test]
fn generic_body_type_ref_hover_explains_opaque_identity() {
    ShapeTest::new(
        "fn describe<T>(value: T) -> string {\n  let label = comptime { type_ref(T) }\n  label\n}\n",
    )
    .at(pos(1, 26))
    .expect_hover_contains("opaque compiler-issued identity");
}

#[test]
fn generic_body_type_category_hover_exposes_exhaustive_result() {
    ShapeTest::new(
        "fn describe<T>(value: T) -> string {\n  let label = comptime { type_category(type_ref(T)) }\n  label\n}\n",
    )
    .at(pos(1, 28))
    .expect_hover_contains("exhaustive semantic category");
}

#[test]
fn generic_body_typed_reflection_offers_signature_help() {
    ShapeTest::new(
        "fn describe<T>(value: T) -> string {\n  let label = comptime { type_ref( ) }\n  label\n}\n",
    )
    .at(pos(1, 34))
    .expect_signature_help();
}

#[test]
fn generic_body_string_type_ref_has_semantic_diagnostic() {
    // The specialized-body compile (triggered by the call site) must surface
    // the named rejection through the LSP diagnostic path.
    ShapeTest::new(
        "fn describe<T>(value: T) -> string {\n  let label = comptime { type_ref(\"T\") }\n  \"ok\"\n}\nprint(describe(1))\n",
    )
    .expect_semantic_diagnostic_contains("strings cannot construct TypeRef");
}

#[test]
fn generic_body_undeclared_param_has_unknown_identity_diagnostic() {
    // type_ref(U) inside fn describe<T> — the freeze failure must reach LSP
    // diagnostics as the NAMED error, not the masked inference diagnostic.
    ShapeTest::new(
        "fn describe<T>(value: T) -> string {\n  let label = comptime { type_ref(U) }\n  \"ok\"\n}\nprint(describe(1))\n",
    )
    .expect_semantic_diagnostic_contains("unknown semantic type identity");
}

#[test]
fn generic_body_runtime_type_ref_use_has_comptime_only_diagnostic() {
    // Runtime use inside a generic body is rejected by the AST-walk validator
    // even though the template body never compiles (no call site needed).
    ShapeTest::new(
        "fn describe<T>(value: T) -> string {\n  let label = type_ref(T)\n  \"ok\"\n}\n",
    )
    .expect_semantic_diagnostic_contains("comptime-only builtin");
}

// =====================================================================
// ADR-009 ticket B2 (S6): LSP surface for `trait_ref` / `find_impl` and
// the TraitRef/ImplRef evidence carriers. Every behavior below is driven
// by the S3 shared-catalog rows (`comptime_reflection.rs::
// {TRAIT_REF_BUILTIN_ROW, FIND_IMPL_BUILTIN_ROW}` spliced verbatim into
// `builtin_metadata::CORE_BUILTINS`) — a hand-written parallel LSP row is
// a defect; a red test here is fixed by enriching the shared rows.
// Semantic-diagnostic twins mirror the S5 rejection matrix rows R1-R4/R8
// plus the R6 stage-boundary escape (see the row → test-name table in
// tests/comptime/trait_evidence.rs and the wave46 B2 addendum).
// =====================================================================

#[test]
fn comptime_completion_offers_trait_evidence_builtins() {
    ShapeTest::new("comptime {\n    \n}\n")
        .at(pos(1, 4))
        .expect_completion("trait_ref")
        .expect_completion("find_impl");
}

#[test]
fn runtime_completion_hides_trait_evidence_builtins() {
    ShapeTest::new("")
        .at(pos(0, 0))
        .expect_no_completion("trait_ref")
        .expect_no_completion("find_impl");
}

#[test]
fn trait_ref_hover_explains_distinct_trait_identity() {
    ShapeTest::new(
        "trait Greetable {\n  method greet() -> string\n}\n\nlet t = comptime { trait_ref(Greetable) }\n",
    )
    .at(pos(4, 22))
    .expect_hover_contains("opaque compiler-issued identity for a declared trait")
    .expect_hover_contains("a trait is not a value type")
    .expect_hover_contains("trait_ref(Tr) -> TraitRef<Tr>")
    .expect_hover_contains("Only valid inside comptime blocks");
}

#[test]
fn find_impl_hover_exposes_optional_evidence_signature() {
    ShapeTest::new(
        "trait Greetable {\n  method greet() -> string\n}\n\ntype User { id: int }\n\nlet r = comptime { find_impl(type_ref(User), trait_ref(Greetable)) }\n",
    )
    .at(pos(6, 22))
    .expect_hover_contains("implementation evidence")
    .expect_hover_contains("Option<ImplRef<T, Tr>>")
    .expect_hover_contains("None — never an error and never partial evidence");
}

#[test]
fn trait_ref_offers_signature_help() {
    ShapeTest::new("let t = comptime { trait_ref( ) }\n")
        .at(pos(0, 29))
        .expect_signature_help();
}

#[test]
fn find_impl_offers_signature_help() {
    ShapeTest::new("let r = comptime { find_impl( ) }\n")
        .at(pos(0, 29))
        .expect_signature_help();
}

/// R1 twin: `type_ref(TraitName)` is the named traits-are-not-value-types
/// semantic diagnostic (upgraded from A1's generic unknown-identity row).
#[test]
fn trait_as_type_ref_has_semantic_diagnostic() {
    ShapeTest::new(
        "trait Greetable {\n  method greet() -> string\n}\n\nlet reflected = comptime { type_ref(Greetable) }\n",
    )
    .expect_semantic_diagnostic_contains("traits are not value types");
}

/// R1 guard twin (A1 row 2 must not regress): a genuinely-unknown name keeps
/// the generic unknown-semantic-type-identity diagnostic even with a trait
/// declared in the same unit.
#[test]
fn unknown_name_keeps_generic_diagnostic_with_traits_declared() {
    ShapeTest::new(
        "trait Greetable {\n  method greet() -> string\n}\n\nlet reflected = comptime { type_ref(DoesNotExist) }\n",
    )
    .expect_semantic_diagnostic_contains("unknown semantic type identity");
}

/// R2 twin: only a declared trait forms a TraitRef — a value type is the
/// named semantic diagnostic.
#[test]
fn type_as_trait_ref_has_semantic_diagnostic() {
    ShapeTest::new("type User { id: int }\n\nlet t = comptime { trait_ref(User) }\n")
        .expect_semantic_diagnostic_contains("only a declared trait forms a TraitRef");
}

/// R3 twin (trait half): strings cannot construct TraitRef.
#[test]
fn string_trait_ref_construction_has_semantic_diagnostic() {
    ShapeTest::new(
        "trait Greetable {\n  method greet() -> string\n}\n\nlet t = comptime { trait_ref(\"Greetable\") }\n",
    )
    .expect_semantic_diagnostic_contains("strings cannot construct TraitRef");
}

/// R3 twin (lookup half): trait lookup cannot use text — a name string in
/// the `find_impl` trait position is the named semantic diagnostic.
#[test]
fn find_impl_string_lookup_has_semantic_diagnostic() {
    ShapeTest::new(
        "trait Greetable {\n  method greet() -> string\n}\n\ntype User { id: int }\n\nlet r = comptime {\n  match find_impl(type_ref(User), \"Greetable\") {\n    Some(proof) => \"some\"\n    None => \"none\"\n  }\n}\n",
    )
    .expect_semantic_diagnostic_contains("trait lookup cannot use text");
}

/// R4 twin: a boolean (including the legacy `implements(...)` result) can
/// never authorize an operation that requires implementation evidence.
#[test]
fn boolean_authorized_generation_has_semantic_diagnostic() {
    ShapeTest::new(
        "trait Greetable {\n  method greet() -> string\n}\n\ntype User { id: int }\n\nimpl Greetable for User {\n  method greet() -> string {\n    return \"hi\"\n  }\n}\n\nlet r = comptime {\n  match find_impl(type_ref(User), implements(User, Greetable)) {\n    Some(proof) => \"some\"\n    None => \"none\"\n  }\n}\n",
    )
    .expect_semantic_diagnostic_contains(
        "a boolean cannot authorize an operation that requires implementation evidence",
    );
}

/// R8 twin (trait_ref): wrong arity is the named semantic diagnostic.
#[test]
fn trait_ref_arity_has_semantic_diagnostic() {
    ShapeTest::new("let t = comptime { trait_ref() }\n")
        .expect_semantic_diagnostic_contains("expects exactly one trait argument");
}

/// R8 twin (find_impl): wrong arity is the named semantic diagnostic.
#[test]
fn find_impl_arity_has_semantic_diagnostic() {
    ShapeTest::new("let r = comptime { find_impl(type_ref(int)) }\n")
        .expect_semantic_diagnostic_contains("find_impl expects exactly two arguments");
}

/// R6 twin: a raw TraitRef escaping to runtime code is the named
/// comptime-only semantic diagnostic (stage-boundary lift rejection).
#[test]
fn raw_trait_ref_escape_has_semantic_diagnostic() {
    ShapeTest::new(
        "trait Greetable {\n  method greet() -> string\n}\n\nlet t = comptime { trait_ref(Greetable) }\nprint(t)\n",
    )
    .expect_semantic_diagnostic_contains("TraitRef is a comptime-only compiler capability");
}

/// R6 twin (evidence half): ImplRef evidence bound in the Some arm cannot
/// cross the stage boundary — named comptime-only semantic diagnostic.
#[test]
fn raw_impl_ref_escape_has_semantic_diagnostic() {
    ShapeTest::new(
        "trait Greetable {\n  method greet() -> string\n}\n\ntype User { id: int }\n\nimpl Greetable for User {\n  method greet() -> string {\n    return \"hi\"\n  }\n}\n\nlet proof = comptime {\n  match find_impl(type_ref(User), trait_ref(Greetable)) {\n    Some(proof) => proof\n    None => error(\"pair is implemented\")\n  }\n}\nprint(proof)\n",
    )
    .expect_semantic_diagnostic_contains("ImplRef is comptime-only implementation evidence");
}

#[test]
fn generic_body_declared_param_reflection_is_diagnostic_clean() {
    // The S1 parameter overlay must reach the LSP compile path too: a
    // declared type param observed via type_ref(T)/type_category must
    // produce neither the freeze failure nor the masked inference error.
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
      FrozenTypeCategory::Existential => "existential"
    }
  }
  label
}

print(describe(1))
"#;
    ShapeTest::new(source)
        .expect_no_semantic_diagnostic_contains("unknown semantic type identity")
        .expect_no_semantic_diagnostic_contains("cannot infer type argument");
}

// =====================================================================
// ADR-009 B3 (S3): LSP hover + inlay over a `some`-bound witness binding.
//
// Hovering the loop variable of a `comptime for some<W...> x in coll`
// renders the OPENED descriptor type (the existential's inner descriptor
// with the hidden witness bound), the stage, and the escape rule.
// Hovering a `some`-clause witness name renders it as an opened hidden
// witness. The inlay hint over the iteration renders the same opened
// descriptor. All three are driven by the shared LSP type-annotation
// surface (`open_comptime_some_descriptor`) — the same `TypeAnnotation`
// carrier the compiler's canonicalizer consumes, not a hand-written row.
// The B1 substrate opens to `FrozenType<T>` (witness erased at the reflect
// boundary; recovering a witness-typed projection is B5, not landed).
// =====================================================================

const SOME_BOUND_ITERATION: &str = "let out = comptime {\n  \
let coll: Array<exists<T> FrozenType<T>> = []\n  \
comptime for some<T> ft in coll {\n    \
let x = ft\n  }\n  \"\"\n}\n";

#[test]
fn hover_on_some_bound_loop_var_shows_opened_descriptor_and_stage() {
    ShapeTest::new(SOME_BOUND_ITERATION)
        .at(pos(2, 23))
        .expect_hover_contains("FrozenType<T>")
        .expect_hover_contains("witness")
        .expect_hover_contains("comptime");
}

#[test]
fn hover_on_some_witness_name_shows_opened_hidden_witness() {
    ShapeTest::new(SOME_BOUND_ITERATION)
        .at(pos(2, 20))
        .expect_hover_contains("hidden witness")
        .expect_hover_contains("comptime");
}

#[test]
fn inlay_on_some_bound_iteration_shows_opened_descriptor() {
    ShapeTest::new(SOME_BOUND_ITERATION).expect_type_hint_label("FrozenType<T>");
}
