//! ADR-009 C3 #14 (S6 completion) — the COLLAPSED declaration surface.
//!
//! The S4 transitional TypedConfig/Legacy classification is deleted: there
//! is ONE annotation surface. These pins cover the declaration-site checks,
//! all firing BEFORE any `@application` exists:
//!
//!   - THE untyped-config rejection (the collapse's named producer in
//!     `planner::plan_definition`; the former R2 mixed-params refinement
//!     folds into it) + its positive twins.
//!   - R1 (ConstLift domain, the ONE `const_lift::annotation_within_lift_
//!     domain` producer reused).
//!   - R3 (hook shape — now fires for EVERY definition, zero-param
//!     included; the "stay on the legacy surface" escape is deleted).
//!   - The lifecycle handlers' post-collapse contract: `on_define`/
//!     `metadata` register for zero-param AND typed-config definitions
//!     (the S6-completion Risk-1 disposition — the former R3-family
//!     rejection guarded the deleted legacy surface).
//!   - The S5b non-function-target declaration rejection (unchanged
//!     producer, now also reachable from zero-param hook definitions).

use super::*;

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

// ── THE untyped-config declaration-site rejection (the collapse) ───────────

#[test]
fn untyped_config_param_rejects_at_declaration_with_the_exact_sentence() {
    // Fires with ZERO applications in the program — declaration-site.
    let message =
        compile_err("annotation warmup(period) { comptime post(target, ctx) { 1 } }");
    assert!(
        message.contains(
            "annotation `warmup` declares config parameter `period` without a type; \
             every annotation config parameter declares its type (the untyped config \
             surface is deleted, C3-G7/S6) — annotate `period` with a ConstLift-liftable \
             type"
        ),
        "the untyped-config sentence must fire verbatim, got: {message}"
    );
}

#[test]
fn untyped_config_rejection_names_the_first_untyped_of_a_mixed_definition() {
    // The former R2 "mixed typed/untyped" refinement folds into the ONE
    // untyped-config rejection: `label` (the first untyped param) is named.
    let message = compile_err(
        "annotation partial(times: int, label) { comptime post(target, ctx) { 1 } }",
    );
    assert!(
        message.contains(
            "annotation `partial` declares config parameter `label` without a type"
        ),
        "the first untyped param must be named, got: {message}"
    );
}

#[test]
fn untyped_config_rejection_fires_for_declarative_hook_definitions_too() {
    let message = compile_err(
        "annotation traced(tag) {\n\
         \x20 targets: [function]\n\
         \x20 before(args) { return args }\n\
         }",
    );
    assert!(
        message.contains("annotation `traced` declares config parameter `tag` without a type"),
        "hook-bearing untyped definitions reject identically, got: {message}"
    );
}

#[test]
fn untyped_config_positive_twin_all_typed_definition_compiles() {
    compile_ok("annotation retry(times: int, label: string) { comptime post(target, ctx) { 1 } }");
}

#[test]
fn untyped_config_positive_twin_zero_param_definition_compiles() {
    // Zero config params = nothing to type — the definition routes the ONE
    // (new) path for free; there is no opt-in marker and no legacy arm.
    compile_ok("annotation plain() { targets: [function] }");
    compile_ok(
        "annotation observer() {\n\
         \x20 targets: [function]\n\
         \x20 before() { 1 }\n\
         }",
    );
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

// ── R3: the hook-shape declaration-site rejection (S6-collapsed sentence) ──
// Fires for EVERY definition — the former "typed config parameters select
// the typed hook surface" head and the "remove the parameter types" escape
// are deleted with the classification fork.

#[test]
fn r3_before_with_legacy_params_rejects_with_the_exact_sentence() {
    let message =
        compile_err("annotation typedcfg(times: int) { before(args, ctx) { args } }");
    assert!(
        message.contains(
            "annotation `typedcfg`'s `before` handler declares (args, ctx); \
             declarative hooks are before(args) / after(result) / zero-param observers \
             before() / after()"
        ),
        "R3 must fire verbatim, got: {message}"
    );
}

#[test]
fn r3_after_with_legacy_params_rejects_with_the_exact_sentence() {
    let message = compile_err(
        "annotation typedcfg(times: int) { after(args, result, ctx) { result } }",
    );
    assert!(
        message.contains(
            "`after` handler declares (args, result, ctx); declarative hooks are"
        ),
        "R3 must fire naming the after shape, got: {message}"
    );
}

#[test]
fn r3_zero_param_definition_with_legacy_hook_shape_rejects() {
    // The collapse's behavior flip: a zero-param definition's declarative
    // hooks route the ONE path, so the former legacy `before(args, ctx)`
    // spelling now rejects at the declaration instead of engaging the
    // deleted weave (pre-collapse twin: this exact fixture compiled Legacy).
    let message = compile_err("annotation once() { before(args, ctx) { args } }");
    assert!(
        message.contains(
            "annotation `once`'s `before` handler declares (args, ctx); \
             declarative hooks are before(args) / after(result)"
        ),
        "the zero-param R3 rejection must fire, got: {message}"
    );
}

#[test]
fn r3_single_param_magic_spellings_reject() {
    // A SINGLE param named `fn` or `ctx` is a deleted-legacy magic spelling,
    // not a pseudo-tuple binder — R3, so the legacy meaning can never
    // silently change on the collapsed surface.
    for source in [
        "annotation typedcfg(times: int) { before(ctx) { ctx } }",
        "annotation typedcfg(times: int) { after(fn) { 1 } }",
    ] {
        let message = compile_err(source);
        assert!(
            message.contains("declarative hooks are before(args) / after(result)"),
            "R3 must fire on the magic single param, got: {message}"
        );
    }
}

#[test]
fn r3_positive_twins_all_four_hook_forms_compile() {
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

// ── Lifecycle handlers on the collapsed surface (Risk-1 disposition) ───────
// `on_define`/`metadata` are compile-time-fired definition hooks registered
// through the installer's lifecycle arm — NOT part of the deleted runtime
// before/after weave. They register for zero-param AND typed-config
// definitions; the former R3-family rejection ("no typed-surface form yet
// … remove the parameter types") died with the legacy surface it guarded.

#[test]
fn lifecycle_zero_param_definition_compiles_and_registers_handlers() {
    let program = parse(
        "annotation traced() {\n\
         \x20 targets: [function]\n\
         \x20 on_define(target) { 1 }\n\
         \x20 metadata(target) { { version: 1 } }\n\
         }",
    );
    let bytecode = BytecodeCompiler::new()
        .compile(&program)
        .expect("zero-param lifecycle definition compiles");
    let compiled = bytecode
        .compiled_annotations
        .get("traced")
        .expect("compiled annotation registered");
    assert!(compiled.on_define_handler.is_some());
    assert!(compiled.metadata_handler.is_some());
}

#[test]
fn lifecycle_typed_config_definition_now_compiles_with_typed_params() {
    // The Risk-1 flip pin (former `r3_family_lifecycle_hooks_reject_citing_
    // the_e4_s6_fence` / `s5b_r3_family_mixed_def_rejects_in_both_handler_
    // orders`): typed config params + lifecycle handlers COMPILE on the
    // collapsed surface, in both handler orders and mixed with hooks.
    let program = parse(
        "annotation typedcfg(times: int) {\n\
         \x20 targets: [function]\n\
         \x20 on_define(target) { 1 }\n\
         }",
    );
    let bytecode = BytecodeCompiler::new()
        .compile(&program)
        .expect("typed-config lifecycle definition compiles");
    assert!(
        bytecode
            .compiled_annotations
            .get("typedcfg")
            .expect("compiled annotation registered")
            .on_define_handler
            .is_some()
    );
    compile_ok(
        "annotation typedcfg2(times: int) { metadata(target) { { version: times } } }",
    );
    // Mixed hook + lifecycle, both handler orders.
    compile_ok(
        "annotation typedcfg3(times: int) {\n\
         \x20 targets: [function]\n\
         \x20 before(args) { return args }\n\
         \x20 on_define(target) { 1 }\n\
         }",
    );
    compile_ok(
        "annotation typedcfg4(times: int) {\n\
         \x20 targets: [function]\n\
         \x20 on_define(target) { 1 }\n\
         \x20 before(args) { return args }\n\
         }",
    );
}

// ── S5b: the DECLARATION-tier non-function-target rejection (S4 residual 7)
// A hooks-bearing definition whose EXPLICIT targets exclude `function` can
// never fire its hooks (the non-function consumer seams run only comptime
// pre/post handlers — measured silent no-op, probe P-NFT) — rejected at the
// declaration, ZERO applications needed. The application tier (mixed
// targets) is pinned in sugar_matrix_tests. Post-collapse the producer also
// fires for ZERO-PARAM hook definitions (they carry sugar now).

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
fn s5b_nonfn_zero_param_hook_definition_rejects_at_declaration_too() {
    // The collapse's reachability extension: pre-collapse this zero-param
    // fixture classified Legacy (no sugar, no check) and ran the legacy
    // weave; now it carries sugar and the SAME producer fires.
    let message = compile_err(
        "annotation deco0() {\n\
         \x20 targets: [expression]\n\
         \x20 before(args) { return args }\n\
         }",
    );
    assert!(
        message.contains(
            "annotation `deco0` declares declarative before/after hooks, but its targets \
             ([expression]) do not include function"
        ),
        "the zero-param declaration-tier rejection must fire, got: {message}"
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
    // Twin 2: a HOOK-FREE typed-config def with non-function targets stays
    // legal — comptime handlers run on type targets; only declarative
    // hooks demand a function seam.
    compile_ok(
        "annotation info(times: int) {\n\
         \x20 targets: [type]\n\
         \x20 comptime post(target, ctx) { 1 }\n\
         }",
    );
    // Twin 3: a zero-param comptime-only def with non-function targets is
    // hook-free — no sugar, no rejection.
    compile_ok(
        "annotation zero_deco() {\n\
         \x20 targets: [type]\n\
         \x20 comptime post(target, ctx) { 1 }\n\
         }",
    );
}
