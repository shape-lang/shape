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

// ── R3: the typed-surface hook-shape declaration-site rejection (S4c) ──────
// The S4b installer surface-and-stop is REPLACED by the real lowering; its
// two pins re-target here onto R3 (the same fixtures now reject EARLIER, at
// planning, with the typed-surface shape sentence — never silent legacy
// engagement, and never the legacy weave slots).

#[test]
fn r3_typed_config_before_with_legacy_params_rejects_with_the_exact_sentence() {
    let message =
        compile_err("annotation typedcfg(times: int) { before(args, ctx) { args } }");
    assert!(
        message.contains(
            "annotation `typedcfg` declares typed config parameters, which selects the \
             typed hook surface, but its `before` handler declares (args, ctx); \
             typed-surface hooks are before(args) / after(result) / zero-param observers \
             before() / after() — or remove the parameter types to stay on the legacy \
             surface until it is deleted (C3-G7/S6)"
        ),
        "R3 must fire verbatim, got: {message}"
    );
}

#[test]
fn r3_typed_config_after_with_legacy_params_rejects_with_the_exact_sentence() {
    let message = compile_err(
        "annotation typedcfg(times: int) { after(args, result, ctx) { result } }",
    );
    assert!(
        message.contains(
            "but its `after` handler declares (args, result, ctx); typed-surface hooks are"
        ),
        "R3 must fire naming the after shape, got: {message}"
    );
}

#[test]
fn r3_single_param_magic_spellings_reject() {
    // A SINGLE param named `fn` or `ctx` is a legacy magic spelling, not a
    // pseudo-tuple binder — R3, so the legacy meaning can never silently
    // change under the typed surface.
    for source in [
        "annotation typedcfg(times: int) { before(ctx) { ctx } }",
        "annotation typedcfg(times: int) { after(fn) { 1 } }",
    ] {
        let message = compile_err(source);
        assert!(
            message.contains("typed-surface hooks are before(args) / after(result)"),
            "R3 must fire on the magic single param, got: {message}"
        );
    }
}

#[test]
fn r3_family_lifecycle_hooks_reject_citing_the_e4_s6_fence() {
    for (source, kind) in [
        (
            "annotation typedcfg(times: int) { on_define(target) { 1 } }",
            "`on_define`",
        ),
        (
            "annotation typedcfg(times: int) { metadata(target) { 1 } }",
            "`metadata`",
        ),
    ] {
        let message = compile_err(source);
        assert!(
            message.contains("is a runtime-lifecycle hook with no typed-surface form yet"),
            "the R3-family {kind} rejection must fire, got: {message}"
        );
        assert!(
            message.contains("the runtime-hook context family is E4's charter")
                && message.contains("deleted at C3-S6"),
            "the E4/S6 fence must be cited, got: {message}"
        );
    }
}

#[test]
fn r3_positive_twins_all_four_typed_surface_forms_compile() {
    // before(args) / after(result) / before() / after() — with a body that
    // satisfies each form's shape — all pass the declaration checks.
    compile_ok(
        "annotation typedcfg(times: int) {\n\
         \x20 targets: [function]\n\
         \x20 before(args) { return args }\n\
         }",
    );
    compile_ok(
        "annotation typedcfg2(times: int) {\n\
         \x20 targets: [function]\n\
         \x20 after(result) { return result }\n\
         }",
    );
    compile_ok(
        "annotation typedcfg3(times: int) {\n\
         \x20 targets: [function]\n\
         \x20 before() { let x = times }\n\
         \x20 after() { let y = times }\n\
         }",
    );
}

#[test]
fn legacy_untyped_declarative_before_handler_still_compiles() {
    // The Legacy class keeps the byte-unchanged legacy weave until S6 —
    // the fork twin.
    compile_ok("annotation once() { before(args, ctx) { args } }");
}

// ── S5b: the DECLARATION-tier non-function-target rejection (S4 residual 7)
// A TypedConfig-with-hooks definition whose EXPLICIT targets exclude
// `function` can never fire its hooks (the non-function consumer seams run
// only comptime pre/post handlers — measured silent no-op, probe P-NFT) —
// rejected at the declaration, ZERO applications needed. The application
// tier (mixed targets) is pinned in sugar_matrix_tests.

#[test]
fn s5b_nonfn_type_only_targets_with_hooks_reject_at_declaration() {
    let message = compile_err(
        "annotation deco(times: int) {\n\
         \x20 targets: [type]\n\
         \x20 before(args) { return args }\n\
         }",
    );
    assert!(
        message.contains(
            "annotation `deco` declares declarative before/after hooks, but its targets \
             ([type]) do not include function; hook templates attach to a function's call \
             seam and can never fire — add function to targets or remove the hooks"
        ),
        "the declaration-tier sentence must fire verbatim, got: {message}"
    );
}

#[test]
fn s5b_nonfn_multi_nonfn_targets_render_the_full_list() {
    let message = compile_err(
        "annotation deco(times: int) {\n\
         \x20 targets: [type, module]\n\
         \x20 after(result) { return result }\n\
         }",
    );
    assert!(
        message.contains("its targets ([type, module]) do not include function"),
        "the rendered target list must name every declared kind, got: {message}"
    );
}

#[test]
fn s5b_nonfn_declaration_twins_compile() {
    // Twin 1: mixed targets INCLUDING function — legal at the declaration
    // (the fn application weaves; the type application is the
    // application-tier rejection, pinned in sugar_matrix_tests).
    compile_ok(
        "annotation deco(times: int) {\n\
         \x20 targets: [function, type]\n\
         \x20 before(args) { return args }\n\
         }",
    );
    // Twin 2: a HOOK-FREE TypedConfig def with non-function targets stays
    // legal — comptime handlers run on type targets; only declarative
    // hooks demand a function seam.
    compile_ok(
        "annotation info(times: int) {\n\
         \x20 targets: [type]\n\
         \x20 comptime post(target, ctx) { 1 }\n\
         }",
    );
    // Twin 3: a LEGACY (untyped) def with non-function targets is outside
    // the sugar surface entirely — untouched.
    compile_ok(
        "annotation legacy_deco() {\n\
         \x20 targets: [type]\n\
         \x20 comptime post(target, ctx) { 1 }\n\
         }",
    );
}

// ── S5b lifecycle angle (charter item 4-i): a MIXED TypedConfig definition
// (typed config + declarative hook + lifecycle handler) rejects with the
// R3-family sentence REGARDLESS of handler order — the lowering loop
// rejects OnDefine/Metadata before its empty-hooks early return, so the
// R3-family declaration rejection is TOTAL: TypedConfig lifecycle handlers
// can NEVER receive typed config params.

#[test]
fn s5b_r3_family_mixed_def_rejects_in_both_handler_orders() {
    for source in [
        // lifecycle handler AFTER the declarative hook
        "annotation typedcfg(times: int) {\n\
         \x20 targets: [function]\n\
         \x20 before(args) { return args }\n\
         \x20 on_define(target) { 1 }\n\
         }",
        // lifecycle handler BEFORE the declarative hook
        "annotation typedcfg(times: int) {\n\
         \x20 targets: [function]\n\
         \x20 on_define(target) { 1 }\n\
         \x20 before(args) { return args }\n\
         }",
    ] {
        let message = compile_err(source);
        assert!(
            message.contains("is a runtime-lifecycle hook with no typed-surface form yet"),
            "the R3-family rejection must fire regardless of handler order, got: {message}"
        );
        assert!(
            message.contains("the runtime-hook context family is E4's charter")
                && message.contains("deleted at C3-S6"),
            "the E4/S6 fence must be cited, got: {message}"
        );
    }
}

