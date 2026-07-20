//! ADR-009 C3 #14 (slice 4) — the G7-compliant transitional classification
//! (`AnnotationSurfaceClass`) + the declaration-site checks R1 (ConstLift
//! domain, the ONE `const_lift::annotation_within_lift_domain` producer
//! reused) and R2 (mixed typed/untyped config params), all firing AT THE
//! DECLARATION — before any `@application` exists.
//!
//! The classification rule, the zero-param ruling, and the named S6 close
//! are documented at the classifier (`planner.rs`).

use super::*;
use crate::compiler::statements::annotation_declarations::planner::{
    classify_annotation_surface, AnnotationSurfaceClass,
};

fn compile_err(source: &str) -> String {
    let program = parse(source);
    let err = BytecodeCompiler::new()
        .compile(&program)
        .expect_err("fixture must reject");
    err.to_string()
}

fn compile_ok(source: &str) {
    let program = parse(source);
    BytecodeCompiler::new()
        .compile(&program)
        .expect("fixture must compile");
}

// ── the classification rule (sealed single-producer chokepoint) ────────────

#[test]
fn zero_param_definition_classifies_legacy() {
    // ZERO-PARAM RULING: no opt-in marker is minted; zero-param defs stay
    // Legacy until S6 deletes the Legacy arm (then every annotation is
    // new-path for free).
    let program = parse("annotation plain() { targets: [function] }");
    let definition = only_definition(&program);
    assert!(matches!(
        classify_annotation_surface(&definition),
        Ok(AnnotationSurfaceClass::Legacy(_))
    ));
}

#[test]
fn all_untyped_params_classify_legacy() {
    let program = parse("annotation warmup(period, mode) { targets: [function] }");
    let definition = only_definition(&program);
    assert!(matches!(
        classify_annotation_surface(&definition),
        Ok(AnnotationSurfaceClass::Legacy(_))
    ));
}

#[test]
fn all_typed_params_classify_typed_config() {
    let program =
        parse("annotation retry(times: int, label: string) { targets: [function] }");
    let definition = only_definition(&program);
    assert!(matches!(
        classify_annotation_surface(&definition),
        Ok(AnnotationSurfaceClass::TypedConfig(_))
    ));
}

#[test]
fn mixed_params_are_a_classification_error_naming_the_first_untyped() {
    let program = parse("annotation partial(times: int, label) { targets: [function] }");
    let definition = only_definition(&program);
    let mixed = classify_annotation_surface(&definition)
        .err()
        .expect("mixed params must not classify");
    assert_eq!(mixed.first_untyped.simple_name(), Some("label"));
}

// ── R2: the mixed typed/untyped declaration-site rejection ─────────────────

#[test]
fn r2_mixed_config_params_reject_at_declaration_with_the_exact_sentence() {
    // Fires with ZERO applications in the program — declaration-site.
    let message = compile_err(
        "annotation partial(times: int, label) { comptime post(target, ctx) { 1 } }",
    );
    assert!(
        message.contains(
            "annotation `partial` mixes typed and untyped config parameters; \
             a typed-config annotation declares a type on every config parameter \
             — annotate `label`"
        ),
        "R2 sentence must fire verbatim, got: {message}"
    );
}

#[test]
fn r2_positive_twin_all_typed_definition_compiles() {
    compile_ok("annotation retry(times: int, label: string) { comptime post(target, ctx) { 1 } }");
}

// ── R1: the declaration-site ConstLift domain check ────────────────────────
// One domain producer (`const_lift::annotation_within_lift_domain`), reused
// at the declaration — never re-implemented. The sentence embeds the closed
// C3-G5 domain sentence verbatim.

#[test]
fn r1_fn_type_config_param_rejects_naming_functions() {
    let message =
        compile_err("annotation bad(cb: (int) -> int) { comptime post(target, ctx) { 1 } }");
    // KNOWN RENDERING RESIDUAL (matches the S3 finish()-time precedent —
    // same `to_type_string()` producer): `TypeAnnotation::Function` falls to
    // the renderer's `_ => "any"` catch-all, so `{type}` renders `any` here.
    // The CLASS parenthetical ("is a function type ...") carries the real
    // diagnosis; fixing the renderer is a display follow-up with wide
    // blast radius (foreign marshaling / inference read the same string).
    assert!(
        message.contains("annotation `bad` declares config parameter `cb: "),
        "R1 head must fire naming the parameter, got: {message}"
    );
    assert!(
        message.contains("whose type is outside the ConstLift domain"),
        "R1 head must fire, got: {message}"
    );
    assert!(
        message.contains("is a function type, and functions are never liftable (C3-G5 / Dec-95)"),
        "the functions class must be named, got: {message}"
    );
    assert!(
        message.contains("declare the config parameter with a liftable type"),
        "the positive tail must be present, got: {message}"
    );
}

#[test]
fn r1_reference_type_config_param_rejects_naming_references() {
    let message = compile_err("annotation bad(r: &int) { comptime post(target, ctx) { 1 } }");
    assert!(
        message.contains("whose type is outside the ConstLift domain"),
        "R1 head must fire, got: {message}"
    );
    assert!(
        message.contains("is a reference type, and references are never liftable (C3-G5 / Dec-95)"),
        "the references class must be named, got: {message}"
    );
}

#[test]
fn r1_nominal_type_config_param_rejects_as_not_liftable() {
    let message = compile_err(
        "type Widget { id: int }\nannotation bad(w: Widget) { comptime post(target, ctx) { 1 } }",
    );
    assert!(
        message.contains("whose type is outside the ConstLift domain"),
        "R1 head must fire, got: {message}"
    );
    assert!(
        message.contains("`Widget` is not a liftable type"),
        "the nominal rejection must name the type, got: {message}"
    );
}

#[test]
fn r1_positive_twins_every_liftable_spelling_compiles() {
    // int, string, Array<int>, homogeneous bracket tuple, Option<int> —
    // the C3-G5 domain's declared spellings all pass the declaration check.
    compile_ok(
        "annotation cfg(a: int, b: string, c: Array<int>, d: [int, int], e: Option<int>) \
         { comptime post(target, ctx) { 1 } }",
    );
}

// ── the installer fork skeleton (surface-and-stop until the S4c lowering) ──

#[test]
fn typed_config_declarative_before_handler_is_a_surface_and_stop() {
    // A TypedConfig def's declarative before/after handlers must NEVER
    // register on the legacy weave slots. Until S4c lands the sugar
    // lowering onto the public comptime API, this is an internal-error-
    // shaped surface-and-stop — never silent legacy engagement.
    let message =
        compile_err("annotation typedcfg(times: int) { before(args, ctx) { args } }");
    assert!(
        message.contains(
            "typed-config annotation `typedcfg` declares a declarative `before` handler"
        ),
        "the fork must refuse the legacy slots, got: {message}"
    );
    assert!(
        message.contains("the legacy weave slots are refused for typed-config definitions"),
        "the refusal must be named, got: {message}"
    );
}

#[test]
fn typed_config_declarative_after_handler_is_a_surface_and_stop() {
    let message = compile_err(
        "annotation typedcfg(times: int) { after(args, result, ctx) { result } }",
    );
    assert!(
        message.contains(
            "typed-config annotation `typedcfg` declares a declarative `after` handler"
        ),
        "the fork must refuse the legacy slots, got: {message}"
    );
}

#[test]
fn legacy_untyped_declarative_before_handler_still_compiles() {
    // The Legacy class keeps the byte-unchanged legacy weave until S6 —
    // the fork twin.
    compile_ok("annotation once() { before(args, ctx) { args } }");
}
