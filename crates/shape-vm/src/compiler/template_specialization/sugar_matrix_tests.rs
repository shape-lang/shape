//! ADR-009 C3 #14 (slice 2, S2d) — the C3-G2 SUGAR-TEST COMPLETENESS MATRIX.
//!
//! C3-G2 (user-ratified): the declarative `annotation name(config) {
//! before/after }` block survives as sugar that LOWERS onto the public
//! comptime API with ZERO private side-channels — if desugaring ever needs a
//! capability the API does not expose, the API is incomplete and must GROW
//! (never a side-channel). This module is that gate, pinned one slice before
//! S4 writes the lowering: every capability the S4 declarative block lowers
//! to, expressed as a fixture written as an annotation handler using ONLY
//! public spellings (`before_hook` / `after_hook` / `capture` / `install`,
//! the annotation-config binding, and the existing frozen `target`
//! descriptor). Every behavioral row is EXECUTED with value-distinguishing
//! output (the S0 §2 "compile-proof alone is banned" rule).
//!
//! # The matrix (row → fixture → API calls)
//!
//! | row | capability the sugar lowers to            | fixture (this module)                                        | public API surface exercised |
//! |-----|-------------------------------------------|--------------------------------------------------------------|------------------------------|
//! | r1  | typed config params → template inputs     | `r1_config_params_are_rule6_constlift_identity_two_specializations` | annotation-config binding + `capture(name, value)` + `before_hook` + `install`; S3b: config values are rule-6 specialization identity (distinct config = distinct baked specialization) |
//! | r2  | `before` body                             | `r2_before_body_is_a_module_scope_typed_fn`                  | `before_hook(fn_ident, captures)` + `install` |
//! | r3  | `after` body                              | `r3_after_body_is_a_module_scope_typed_fn`                   | `after_hook(fn_ident, captures)` + `install` |
//! | r4  | application to a target (target implicit) | `r4_application_covers_before_only_after_only_and_both`      | `install(t)` — before-only / after-only / both on three targets |
//! | r5  | stacked annotations                       | `r5_stacked_annotations_compose_as_wrapping`                 | repeated handler runs; before chain in application order, after chain in REVERSE application order (the onion/wrapping semantic — fix-round-1) |
//! | r6  | config-conditional hook selection         | `r6_config_conditional_install_is_ordinary_control_flow`     | ordinary handler `if` around `install` |
//! | r7  | target introspection for composition      | `r7_target_introspection_selects_the_template`               | the EXISTING frozen `target.params[i].type` surface (reused, not duplicated — S2 adds NO new inspection builtin) |
//! | r8  | hover data                                | `r8_registry_row_carries_declaration_and_application_views`  | `hook_install_registry` rows: generic view at declaration + specialized types at application (the S8 hover substrate — sugar gets hover for free because it lowers to `install()`) |
//! | r9  | observer hooks on zero-param / void targets | `r9_observers_cover_zero_param_and_void_targets`           | the OBSERVER template form (fix-round-1 C3-G2 growth): `before_hook`/`after_hook` over a concrete zero-signature-param void body — the entry/exit-logging blocks the declarative surface green-pins on `fn hello()` (before_after.rs `before_hook_with_empty_params`, wrapping.rs `annotation_wrapping_void_function`) |
//!
//! Config enters templates ONLY as `capture(name, value)` (C3-G4's
//! [C0926]-totality premise; the a5 surviving-legit path from slice-0 §4) —
//! r1 pins exactly that path. r7 is the RULED reuse: the handler's first
//! param (`__ComptimeTarget` built from `ComptimeTarget`,
//! `comptime_target.rs` — params with name/type/const/type_ref, return_type,
//! annotations) is the inspection surface; no second descriptor exists.

use crate::compiler::comptime_fragments::checked_template::TemplateHookKind;
use crate::executor::{VMConfig, VirtualMachine};

/// Full production compile (parse → pre-pass → analyzer → pass-2 handler →
/// install → weave); returns BOTH the result and the compiler so pins can
/// assert the post-compile registry state (the S2b/S2c harness shape).
fn compile_source(src: &str) -> (shape_ast::error::Result<()>, crate::compiler::BytecodeCompiler) {
    let program = shape_ast::parse_program(src).expect("fixture parses");
    let mut compiler = crate::compiler::BytecodeCompiler::new();
    let result = compiler.compile_in_place(&program);
    (result, compiler)
}

fn compiled_ok(src: &str) -> crate::compiler::BytecodeCompiler {
    let (result, compiler) = compile_source(src);
    result.expect("fixture must compile");
    compiler
}

/// Execute the compiled program's top level and return the final int.
fn top_level_i64(src: &str) -> (i64, crate::compiler::BytecodeCompiler) {
    let compiler = compiled_ok(src);
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(compiler.program.clone());
    let value = vm
        .execute(None)
        .expect("program executes")
        .as_i64()
        .expect("top-level result is an int");
    (value, compiler)
}

// ── r1: annotation config params → template inputs, ONLY as captures ───────

// The a5 surviving-legit path: config arrives as a handler param (the
// existing annotation-args binding; S4's grammar adds the types) and enters
// the template ONLY through `capture(name, value)` — never ambient scope.
// S3b PIN FLIP (Dec-95 rule 6, ordered by the slice-3 charter): config
// VALUES are now ConstLift'd specialization identity — @scaled(3) and
// @scaled(5) get TWO distinct baked specializations (structurally different
// config = distinct specialization; the S2 "ONE shared value-generic
// handler" posture is superseded). Registry rows still record 3/5; the
// executed output is UNCHANGED.
#[test]
fn r1_config_params_are_rule6_constlift_identity_two_specializations() {
    let src = r#"
fn tmpl<Args>(args: Args, factor: int) -> Args {
    args[0] = args[0] * factor
    return args
}

annotation scaled(factor) {
  targets: [function]
  comptime post(target, ctx) {
    install(before_hook(tmpl, [capture("factor", factor)]))
  }
}

@scaled(3)
fn victim_a(a: int) -> int { return a + 1 }

@scaled(5)
fn victim_b(a: int) -> int { return a + 1 }

victim_a(10) * 1000 + victim_b(10)
"#;
    let (value, compiler) = top_level_i64(src);
    // victim_a: 10*3 = 30 → 31; victim_b: 10*5 = 50 → 51.
    assert_eq!(
        value, 31051,
        "each application's CONFIG value reaches its weave through the capture"
    );
    assert_eq!(compiler.hook_install_registry.len(), 2);
    let (row_a, row_b) = (
        &compiler.hook_install_registry[0],
        &compiler.hook_install_registry[1],
    );
    assert_eq!(
        row_a.captures,
        vec![("factor".to_string(), "3".to_string())],
        "the config ARG value (not the param name) is the recorded capture literal"
    );
    assert_eq!(row_b.captures, vec![("factor".to_string(), "5".to_string())]);
    assert_ne!(
        row_a.function_index, row_b.function_index,
        "rule 6: structurally different config = DISTINCT baked specializations \
         (the S2 shared-handler posture is superseded)"
    );
}

// ── r2 / r3: before / after bodies are module-scope typed fns ──────────────

#[test]
fn r2_before_body_is_a_module_scope_typed_fn() {
    let src = r#"
fn add_one(x: int) -> int { return x + 1 }

annotation with_before() {
  targets: [function]
  comptime post(target, ctx) {
    install(before_hook(add_one, []))
  }
}

@with_before()
fn victim(a: int) -> int { return a * 10 }

victim(4)
"#;
    let (value, compiler) = top_level_i64(src);
    assert_eq!(value, 50, "the before body fires (skip ⇒ 40)");
    let row = &compiler.hook_install_registry[0];
    assert_eq!(row.hook_kind, TemplateHookKind::Before);
    assert!(
        row.template_sig.starts_with("add_one "),
        "the template body IS the module-scope fn: {}",
        row.template_sig
    );
}

#[test]
fn r3_after_body_is_a_module_scope_typed_fn() {
    let src = r#"
fn double(r: int) -> int { return r * 2 }

annotation with_after() {
  targets: [function]
  comptime post(target, ctx) {
    install(after_hook(double, []))
  }
}

@with_after()
fn victim(a: int) -> int { return a * 10 }

victim(4)
"#;
    let (value, compiler) = top_level_i64(src);
    assert_eq!(value, 80, "the after body fires (skip ⇒ 40)");
    assert_eq!(
        compiler.hook_install_registry[0].hook_kind,
        TemplateHookKind::After
    );
}

// ── r4: application — before-only / after-only / both, target implicit ─────

// One fixture, three targets: `install(t)` names NO target — the annotation's
// target is implicit (matching every existing directive), and each shape
// (before-only / after-only / both) is value-distinguishing.
#[test]
fn r4_application_covers_before_only_after_only_and_both() {
    let src = r#"
fn add_one(x: int) -> int { return x + 1 }
fn double(r: int) -> int { return r * 2 }

annotation before_only() {
  targets: [function]
  comptime post(target, ctx) {
    install(before_hook(add_one, []))
  }
}

annotation after_only() {
  targets: [function]
  comptime post(target, ctx) {
    install(after_hook(double, []))
  }
}

annotation both_hooks() {
  targets: [function]
  comptime post(target, ctx) {
    install(before_hook(add_one, []))
    install(after_hook(double, []))
  }
}

@before_only()
fn v1(a: int) -> int { return a * 10 }

@after_only()
fn v2(a: int) -> int { return a * 10 }

@both_hooks()
fn v3(a: int) -> int { return a * 10 }

v1(4) * 10000 + v2(4) * 100 + v3(4)
"#;
    let (value, compiler) = top_level_i64(src);
    // v1: (4+1)*10 = 50; v2: 40*2 = 80; v3: (4+1)*10*2 = 100.
    assert_eq!(
        value, 508100,
        "before-only 50 / after-only 80 / both 100 — each application shape distinct"
    );
    assert_eq!(
        compiler.hook_install_registry.len(),
        4,
        "one row per install: v1 gets 1, v2 gets 1, v3 gets 2"
    );
    assert!(
        compiler
            .hook_install_registry
            .iter()
            .all(|row| ["v1", "v2", "v3"].contains(&row.target_name.as_str())),
        "the install target is IMPLICIT — always the annotation's own target"
    );
}

// ── r5: stacked annotations compose as WRAPPING (onion) ────────────────────

// Repeated handler runs (one per stacked annotation) accumulate installs;
// the weave is the WRAPPING/onion composition (fix-round-1): the
// first-applied annotation is the OUTERMOST wrapper — before chain in
// application order, after chain in REVERSE application order (the surviving
// declarative surface's stacked semantics, wrapping.rs
// stacked_after_hooks_transform_result_in_order). add_ten-then-mul_two:
// before (1+10)*2 = 22 → impl 220; after 220*2 = 440 → +10 = 450. Any order
// flip is value-distinguishing (before flipped ⇒ impl 120; after in
// application order ⇒ 460).
#[test]
fn r5_stacked_annotations_compose_as_wrapping() {
    let src = r#"
fn add_ten(x: int) -> int { return x + 10 }
fn mul_two(x: int) -> int { return x * 2 }

annotation outer_hook() {
  targets: [function]
  comptime post(target, ctx) {
    install(before_hook(add_ten, []))
    install(after_hook(add_ten, []))
  }
}

annotation inner_hook() {
  targets: [function]
  comptime post(target, ctx) {
    install(before_hook(mul_two, []))
    install(after_hook(mul_two, []))
  }
}

@outer_hook()
@inner_hook()
fn victim(a: int) -> int { return a * 10 }

victim(1)
"#;
    let (value, compiler) = top_level_i64(src);
    assert_eq!(
        value, 450,
        "onion composition: before (1+10)*2 → impl 220, after (inner-first) 220*2+10 = 450"
    );
    assert_eq!(compiler.hook_install_registry.len(), 4);
}

// ── r6: config-conditional hook selection is ordinary control flow ─────────

// The sugar's conditional forms need NO dedicated API: the handler is
// ordinary comptime code, so `if config { install(...) }` is the whole
// mechanism. `@maybe_hook(false)` leaves its target UNWOVEN (no row, no
// wrapper) — not a woven no-op.
#[test]
fn r6_config_conditional_install_is_ordinary_control_flow() {
    let src = r#"
fn add_one(x: int) -> int { return x + 1 }

annotation maybe_hook(enabled) {
  targets: [function]
  comptime post(target, ctx) {
    if enabled {
      install(before_hook(add_one, []))
    }
  }
}

@maybe_hook(true)
fn on_target(a: int) -> int { return a * 10 }

@maybe_hook(false)
fn off_target(a: int) -> int { return a * 10 }

on_target(4) * 100 + off_target(4)
"#;
    let (value, compiler) = top_level_i64(src);
    // on: (4+1)*10 = 50; off: 4*10 = 40.
    assert_eq!(value, 5040, "only the enabled application hooks (both-on ⇒ 5050)");
    assert_eq!(
        compiler.hook_install_registry.len(),
        1,
        "the disabled application installs NOTHING"
    );
    assert_eq!(compiler.hook_install_registry[0].target_name, "on_target");
}

// ── r7: target introspection reuses the frozen descriptor surface ──────────

// RULED REUSE: composition decisions read the handler's first param — the
// existing `__ComptimeTarget` descriptor (params with name/type/const/
// type_ref, return_type, annotations). S2 adds NO new inspection builtin.
// The handler picks a TYPE-APPROPRIATE template per target from
// `target.params[0].type`; a mis-selection is not silent (installing the
// int template on the number target is the concrete match-or-error
// rejection), and each selection is value-distinguishing.
#[test]
fn r7_target_introspection_selects_the_template() {
    let src = r#"
fn bump_int(x: int) -> int { return x + 1 }
fn bump_num(x: number) -> number { return x + 0.5 }

annotation adaptive() {
  targets: [function]
  comptime post(target, ctx) {
    if target.params[0].type == "int" {
      install(before_hook(bump_int, []))
    } else {
      install(before_hook(bump_num, []))
    }
  }
}

@adaptive()
fn int_victim(a: int) -> int { return a * 10 }

@adaptive()
fn num_victim(a: number) -> number { return a * 10.0 }

let n = num_victim(2.0)
let bonus = if n > 24.9 { 1000 } else { 0 }
int_victim(4) + bonus
"#;
    let (value, compiler) = top_level_i64(src);
    // int_victim: (4+1)*10 = 50; num_victim: (2.0+0.5)*10.0 = 25.0 → bonus.
    assert_eq!(
        value, 1050,
        "each target got ITS template: int path 50 (skip ⇒ 40), number path 25.0 (skip ⇒ 20.0)"
    );
    assert_eq!(compiler.hook_install_registry.len(), 2);
    let sig_for = |target: &str| {
        compiler
            .hook_install_registry
            .iter()
            .find(|row| row.target_name == target)
            .unwrap_or_else(|| panic!("registry row for {target}"))
            .template_sig
            .clone()
    };
    assert!(
        sig_for("int_victim").starts_with("bump_int "),
        "the introspected int target selected the int template: {}",
        sig_for("int_victim")
    );
    assert!(
        sig_for("num_victim").starts_with("bump_num "),
        "the introspected number target selected the number template: {}",
        sig_for("num_victim")
    );
}

// ── r8: hover data — the install-registry row is the query substrate ───────

// Sugar gets hover FOR FREE because it lowers to `install()`: the S2b
// registry row carries the GENERIC view at declaration (the template's
// declared Sig rendering) and the SPECIALIZED types at application (the
// injective Sig-suffixed symbol — the delimited `(int, number)` rendering),
// plus the capture literals and the `@application` anchor (the S8 hover
// surface reads exactly these fields; the C1 slice-4 query precedent).
#[test]
fn r8_registry_row_carries_declaration_and_application_views() {
    let src = r#"
fn tmpl<Args>(args: Args, factor: int) -> Args {
    args[0] = args[0] * factor
    return args
}

annotation hooked() {
  targets: [function]
  comptime post(target, ctx) {
    install(before_hook(tmpl, [capture("factor", 3)]))
  }
}

@hooked()
fn victim(a: int, b: number) -> int { return a }

victim(1, 2.0)
"#;
    let (_, compiler) = top_level_i64(src);
    assert_eq!(compiler.hook_install_registry.len(), 1);
    let row = &compiler.hook_install_registry[0];
    assert_eq!(row.annotation_name, "hooked");
    assert_eq!(row.target_name, "victim");
    assert_eq!(row.hook_kind, TemplateHookKind::Before);
    assert_eq!(
        row.template_sig, "tmpl <Args>(args: Args) -> Args",
        "the GENERIC view at declaration"
    );
    assert!(
        row.specialized_symbol.contains("(int, number)"),
        "the SPECIALIZED types at application (the injective delimited Sig): {}",
        row.specialized_symbol
    );
    assert_eq!(
        row.captures,
        vec![("factor".to_string(), "3".to_string())],
        "capture names + rendered values, in delivery order"
    );
    let registered = compiler
        .program
        .functions
        .get(row.function_index as usize)
        .expect("the row's function index resolves");
    assert_eq!(
        registered.name, row.specialized_symbol,
        "symbol and index name the SAME registered specialization"
    );
    assert_ne!(
        row.application_span,
        shape_ast::ast::Span::default(),
        "the row anchors at the real @application span (the hover anchor)"
    );
}

// ── r9: observer hooks — zero-param and void targets (fix-round-1 growth) ──

// The G2 hole the round-1 gate named: the canonical declarative entry/exit-
// logging block green-pins on `fn hello()` (0-param void:
// before_after.rs `before_hook_with_empty_params`) and `after` on a void
// 1-param fn (wrapping.rs `annotation_wrapping_void_function`) — shapes that
// previously had NO public-API spelling. The OBSERVER template form (a
// concrete zero-signature-param void body) is that spelling: ONE annotation
// lowers to ONE pair of observer templates and applies to a 0-param void
// target AND a 1-param void target unchanged (target-uniform, so the S4
// lowering needs no per-target branching). Value-distinguishing execution
// proof for observers is error-injection
// (`weave::observer_execution_is_proven_by_an_erroring_observer_body`); this
// row pins the green composition + rows for both target shapes.
#[test]
fn r9_observers_cover_zero_param_and_void_targets() {
    let src = r#"
fn note_in() { let x = 1 }
fn note_out() { let x = 2 }

annotation entry_exit() {
  targets: [function]
  comptime post(target, ctx) {
    install(before_hook(note_in, []))
    install(after_hook(note_out, []))
  }
}

@entry_exit()
fn hello() { let a = 1 }

@entry_exit()
fn log_it(msg: int) { let b = msg }

hello()
log_it(4)
7
"#;
    let (value, compiler) = top_level_i64(src);
    assert_eq!(value, 7, "both observer weaves execute cleanly");
    assert_eq!(
        compiler.hook_install_registry.len(),
        4,
        "one before + one after observer row per target"
    );
    for target in ["hello", "log_it"] {
        let kinds: Vec<_> = compiler
            .hook_install_registry
            .iter()
            .filter(|row| row.target_name == target)
            .map(|row| row.hook_kind)
            .collect();
        assert_eq!(
            kinds,
            vec![TemplateHookKind::Before, TemplateHookKind::After],
            "target {target} carries the before+after observer pair"
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// ADR-009 C3 #14 (slice 4, S4c) — THE SUGAR LOWERING e2e matrix.
//
// The declarative `annotation name(typed config) { before/after }` block now
// LOWERS onto the public API (C3-G2): minted module-scope-shaped body fns
// (C3-G3) + ONE synthesized `comptime post` handler spelling ONLY
// install/before_hook/after_hook/capture, config bound as ConstLift'd
// declared captures (C3-G4/G5) through the S4b typed injection. Every pin
// below EXECUTES the woven program with value-distinguishing output; each
// mirrors an S2d/S4a/S4b public-API pin so the sugar's behavior is the
// API's behavior (the sugar-equivalence discipline; the E1/E2/E3 formal
// equivalence pins land in the S4 close stage).
// ═══════════════════════════════════════════════════════════════════════════

// (a) THE CHARTER PIN — mixed (int, string) config through the sugar:
// the declarative twin of the S4b hand-written
// `typed_config_params_flow_through_the_public_api_end_to_end` (weave.rs),
// same fixture values: two applications with DIFFERENT config prove the
// values flow from each `@application`'s args (140240; swapped configs ⇒
// 240140; skip ⇒ 40).
#[test]
fn s4c_sugar_mixed_int_string_config_executes_end_to_end() {
    let src = r#"
annotation retry(times: int, tag: string) {
  targets: [function]
  before(args) {
    args[0] = args[0] * times + tag.length()
    return args
  }
}

@retry(3, "ab")
fn victim_a(a: int) -> int { return a * 10 }

@retry(5, "wxyz")
fn victim_b(a: int) -> int { return a * 10 }

victim_a(4) * 1000 + victim_b(4)
"#;
    let (value, compiler) = top_level_i64(src);
    assert_eq!(
        value, 140_240,
        "each application's OWN typed config values drive its mutation through the sugar"
    );
    assert_eq!(compiler.hook_install_registry.len(), 2, "both installs land");
    assert_eq!(
        compiler.hook_install_registry[0].captures,
        vec![
            ("times".to_string(), "3".to_string()),
            ("tag".to_string(), "\"ab\"".to_string())
        ],
        "row a renders the first application's config in declared order"
    );
    assert_eq!(
        compiler.hook_install_registry[1].captures,
        vec![
            ("times".to_string(), "5".to_string()),
            ("tag".to_string(), "\"wxyz\"".to_string())
        ]
    );
    assert_ne!(
        compiler.hook_install_registry[0].function_index,
        compiler.hook_install_registry[1].function_index,
        "rule 6: differing typed config splits the baked specializations"
    );
}

// (a2) The second charter shape — int + Array<int> config through the sugar
// (the declarative twin of the S4a
// `mixed_capture_value_types_in_one_handler_execute_end_to_end` pin; same
// arithmetic: 4*3 + 5 = 17, skip ⇒ 4, either constant misread shifts it).
// The `cfg[0]` read inside the hook body exercises the guard-env IndexAccess
// proof (a minted body has no span-keyed inference facts).
#[test]
fn s4c_sugar_int_and_array_config_executes_end_to_end() {
    let src = r#"
annotation boost(bump: int, cfg: Array<int>) {
  targets: [function]
  before(args) {
    args[0] = args[0] * bump + cfg[0]
    return args
  }
}

@boost(3, [5, 6])
fn victim(a: int) -> int { return a }

victim(4)
"#;
    let (value, compiler) = top_level_i64(src);
    assert_eq!(value, 17, "BOTH mixed-typed baked constants drive the mutation");
    assert_eq!(compiler.hook_install_registry.len(), 1);
    let row = &compiler.hook_install_registry[0];
    assert_eq!(
        row.captures,
        vec![
            ("bump".to_string(), "3".to_string()),
            ("cfg".to_string(), "[5, 6]".to_string())
        ],
        "the row renders both capture values in declared order"
    );
    assert!(
        row.specialized_symbol.contains("::cfg#2"),
        "the specialized symbol carries the two-value config arity head: {}",
        row.specialized_symbol
    );
}

// (b) before + after in ONE sugar definition — both hooks lower, both fire:
// before 4+3 = 7 → impl 70 → after 70*3 = 210 (skip before ⇒ 120, skip
// after ⇒ 70).
#[test]
fn s4c_sugar_before_and_after_in_one_definition() {
    let src = r#"
annotation wrapb(bump: int) {
  targets: [function]
  before(args) {
    args[0] = args[0] + bump
    return args
  }
  after(result) {
    return result * bump
  }
}

@wrapb(3)
fn victim(a: int) -> int { return a * 10 }

victim(4)
"#;
    let (value, compiler) = top_level_i64(src);
    assert_eq!(value, 210, "the before AND the after hook both execute");
    assert_eq!(compiler.hook_install_registry.len(), 2);
    assert_eq!(
        compiler.hook_install_registry[0].hook_kind,
        TemplateHookKind::Before,
        "hooks install in declaration order: before first"
    );
    assert_eq!(
        compiler.hook_install_registry[1].hook_kind,
        TemplateHookKind::After
    );
}

// (c) STACKED sugar annotations compose as WRAPPING (the F2 onion — the r5
// value discipline): before chain in application order, after chain in
// REVERSE application order. Non-commutative afters distinguish: onion =
// 632; an application-order after chain would give 623.
#[test]
fn s4c_stacked_sugar_annotations_compose_as_wrapping() {
    let src = r#"
annotation wrap_x(k: int) {
  targets: [function]
  before(args) {
    args[0] = args[0] + k
    return args
  }
  after(result) {
    return result * 10 + k
  }
}

annotation wrap_y(k: int) {
  targets: [function]
  before(args) {
    args[0] = args[0] + k
    return args
  }
  after(result) {
    return result * 10 + k
  }
}

@wrap_x(2)
@wrap_y(3)
fn victim(x: int) -> int { return x }

victim(1)
"#;
    let (value, compiler) = top_level_i64(src);
    // befores (application order): 1+2 = 3 → 3+3 = 6 → impl 6.
    // afters (REVERSE application — onion): 6*10+3 = 63 → 63*10+2 = 632.
    assert_eq!(
        value, 632,
        "first-applied sugar annotation is the OUTERMOST wrapper (623 = broken order)"
    );
    assert_eq!(compiler.hook_install_registry.len(), 4);
}

// (d) OBSERVER forms through the sugar: `before()` / `after()` (zero params)
// lower to the F1 concrete observer form on a zero-param void target. Green
// control here; execution non-vacuity is the error-injection twin below.
#[test]
fn s4c_sugar_observers_on_zero_param_target() {
    let src = r#"
annotation trace_obs(tag: int) {
  targets: [function]
  before() { let x = tag }
  after() { let y = tag }
}

@trace_obs(1)
fn hello() { let a = 1 }

hello()
7
"#;
    let (value, compiler) = top_level_i64(src);
    assert_eq!(value, 7, "both observer weaves execute cleanly");
    assert_eq!(compiler.hook_install_registry.len(), 2);
    assert_eq!(
        compiler.hook_install_registry[0].hook_kind,
        TemplateHookKind::Before
    );
    assert_eq!(
        compiler.hook_install_registry[1].hook_kind,
        TemplateHookKind::After
    );
}

// (d-twin) EXECUTION PROOF for sugar observers (error-injection — observers
// have no data-flow observable): an erroring observer body fails the woven
// program iff the observer actually runs. The green pin above is the
// non-vacuity control.
#[test]
fn s4c_sugar_observer_execution_proven_by_error_injection() {
    for hook in ["before()", "after()"] {
        let src = format!(
            r#"
annotation boom_obs(n: int) {{
  targets: [function]
  {hook} {{
    let xs = [1, 2]
    let mut i = 0
    while i < 9 {{ i = i + 1 }}
    let y = xs[i]
  }}
}}

@boom_obs(1)
fn hello() {{ let a = 1 }}

hello()
7
"#
        );
        let compiler = compiled_ok(&src);
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(compiler.program.clone());
        vm.execute(None).expect_err(&format!(
            "the woven sugar {hook} observer must EXECUTE (its body errors at runtime)"
        ));
    }
}

// (e) Dec-95 rule 6 THROUGH the sugar: two applications with structurally
// EQUAL config SHARE one baked specialization (equal registry
// function_index); differing config SPLITS.
#[test]
fn s4c_sugar_rule6_equal_config_shares_and_differing_splits() {
    let src = r#"
annotation scale6(k: int) {
  targets: [function]
  before(args) {
    args[0] = args[0] * k
    return args
  }
}

@scale6(4)
fn v_a(a: int) -> int { return a + 1 }

@scale6(4)
fn v_b(a: int) -> int { return a + 2 }

@scale6(9)
fn v_c(a: int) -> int { return a + 3 }

(v_a(10) * 1000 + v_b(20)) * 1000 + v_c(30)
"#;
    let (value, compiler) = top_level_i64(src);
    // a: 10*4+1 = 41; b: 20*4+2 = 82; c: 30*9+3 = 273.
    assert_eq!(value, 41082273, "all three targets execute their own config");
    assert_eq!(compiler.hook_install_registry.len(), 3);
    assert_eq!(
        compiler.hook_install_registry[0].function_index,
        compiler.hook_install_registry[1].function_index,
        "rule 6 via sugar: equal config SHARES one specialization"
    );
    assert_ne!(
        compiler.hook_install_registry[0].function_index,
        compiler.hook_install_registry[2].function_index,
        "rule 6 via sugar: differing config SPLITS"
    );
}

// (f) Config-arg TYPE MISMATCH through the sugar: `@retry("x", "y")` feeds a
// string application arg to the declared `times: int` config param — a LOUD
// compile-time rejection (contains-level per the S4 charter; S5 owns exact
// attribution). The green twin is the (a) charter pin.
#[test]
fn s4c_sugar_config_arg_mismatch_is_a_loud_rejection() {
    let src = r#"
annotation retry(times: int, tag: string) {
  targets: [function]
  before(args) {
    args[0] = args[0] * times + tag.length()
    return args
  }
}

@retry("x", "y")
fn victim(a: int) -> int { return a * 10 }

victim(4)
"#;
    let (result, _) = compile_source(src);
    let message = result
        .expect_err("a string application arg against `times: int` must reject loudly")
        .to_string();
    assert!(
        message.contains("int") || message.contains("type"),
        "the rejection names the type mismatch (contains-level; S5 owns attribution): {message}"
    );
}

// (g) COEXISTENCE: a TypedConfig definition with BOTH a user `comptime post`
// handler (hand-written public-API install) AND a declarative hook — BOTH
// fire, user handler first. 2+5 = 7 → impl 70 → sugar after 70+7 = 77
// (skip user ⇒ 27, skip sugar ⇒ 70).
#[test]
fn s4c_sugar_coexists_with_a_user_comptime_post_handler() {
    let src = r#"
fn add_five(x: int) -> int { return x + 5 }

annotation both_ways(bump: int) {
  targets: [function]
  comptime post(target, ctx) {
    install(before_hook(add_five, []))
  }
  after(result) {
    return result + bump
  }
}

@both_ways(7)
fn victim(a: int) -> int { return a * 10 }

victim(2)
"#;
    let (value, compiler) = top_level_i64(src);
    assert_eq!(
        value, 77,
        "the user handler's install AND the declarative hook both fire"
    );
    assert_eq!(compiler.hook_install_registry.len(), 2);
    assert_eq!(
        compiler.hook_install_registry[0].hook_kind,
        TemplateHookKind::Before,
        "the user handler's install lands first (user post runs before the sugar post)"
    );
    assert_eq!(
        compiler.hook_install_registry[1].hook_kind,
        TemplateHookKind::After
    );
}

// C3-G8 THROUGH the sugar: a sugar annotation applied to a GENERIC target
// fires the EXISTING S2b producer at the `@application` site (no new
// producer) and leaves ZERO registry rows.
#[test]
fn s4c_sugar_on_generic_target_fires_the_g8_rejection() {
    let src = r#"
annotation retry_g(times: int) {
  targets: [function]
  before(args) {
    args[0] = args[0] * times
    return args
  }
}

@retry_g(3)
fn victim<T>(x: T) -> T { return x }

victim(1)
"#;
    let (result, compiler) = compile_source(src);
    let message = result
        .expect_err("a sugar install on a generic target must reject (C3-G8)")
        .to_string();
    for fragment in [
        "(via @retry_g) on `victim`",
        "withdrawn until #59 (the monomorphization-origin re-arm)",
        "apply @retry_g to a concrete function",
    ] {
        assert!(
            message.contains(fragment),
            "the G8 sentence must fire through the sugar; missing {fragment:?}: {message}"
        );
    }
    assert!(
        compiler.hook_install_registry.is_empty(),
        "a rejected install leaves no registry row"
    );
}

// C3-G12: a TypedConfig (hook-template) annotation on a fn-local NESTED fn
// is a LOUD named rejection at the application site (the parser desugar
// formerly dropped it SILENTLY — S0 a4/a4c; the annotations now ride
// `Expr::FunctionExpr.annotations`).
#[test]
fn s4c_typed_config_annotation_on_nested_fn_rejects_loudly() {
    let src = r#"
annotation retry_n(times: int) {
  targets: [function]
  before(args) {
    args[0] = args[0] * times
    return args
  }
}

fn outer() -> int {
  @retry_n(3)
  fn inner(x: int) -> int { return x }
  return inner(4)
}

outer()
"#;
    let (result, _) = compile_source(src);
    let message = result
        .expect_err("a hook-template annotation on a nested fn must reject loudly (C3-G12)")
        .to_string();
    assert!(
        message.contains(
            "annotation `@retry_n` on fn-local nested function `inner` is not applied — \
             hook-template annotations on nested functions are not supported yet (#62); \
             apply @retry_n to a module-scope function"
        ),
        "the G12 sentence must fire verbatim, got: {message}"
    );
}

// C3-G12 positive twin: the SAME annotation on a module-scope fn weaves.
#[test]
fn s4c_g12_twin_module_scope_target_weaves() {
    let src = r#"
annotation retry_n(times: int) {
  targets: [function]
  before(args) {
    args[0] = args[0] * times
    return args
  }
}

@retry_n(3)
fn scaled(x: int) -> int { return x }

scaled(4)
"#;
    let (value, _) = top_level_i64(src);
    assert_eq!(value, 12, "the module-scope twin weaves (skip ⇒ 4)");
}

// C3-G12 residual control: a LEGACY-classified annotation on a nested fn
// keeps the PRE-slice-4 behavior BYTE-identical — silently dropped (the
// recorded residual until S5's matrix owns the class). Value-distinguishing:
// an APPLIED legacy before would replace the args with [99] ⇒ 99; the
// silent drop keeps 4.
#[test]
fn s4c_g12_legacy_annotation_on_nested_fn_stays_silently_dropped() {
    let src = r#"
annotation legacy_n() {
  targets: [function]
  before(args, ctx) { [99] }
}

fn outer() -> int {
  @legacy_n()
  fn inner(x: int) -> int { return x }
  return inner(4)
}

outer()
"#;
    let (value, compiler) = top_level_i64(src);
    assert_eq!(
        value, 4,
        "the legacy annotation on the nested fn stays silently dropped (S5 owns the class)"
    );
    assert!(compiler.hook_install_registry.is_empty());
}

// ═══════════════════════════════════════════════════════════════════════════
// ADR-009 C3 #14 (slice 4, S4d) — THE SUGAR-EQUIVALENCE PROOF (C3-G2).
//
// The G2 completeness gate's formal close: where the S4c lowering calls
// internal seams for compile-time efficiency, these pins prove the
// equivalent PUBLIC-API program produces IDENTICAL behavior:
//
//   E1 BEHAVIORAL — one program carries a sugar-declared typed-config
//      annotation AND its hand-written public-API twin (identical hook body
//      code as a module-scope fn + `comptime post` with capture/install;
//      BOTH defs TypedConfig-classified), applied to identical twin
//      targets: identical executed values + registry-row agreement.
//   E2 IDENTITY-SUFFIX — the strongest cheap identity assertion: full
//      symbol equality is IMPOSSIBLE (the sugar's minted hygienic body-fn
//      name differs from the hand-written name by construction), so the
//      pin splits both specialized symbols at `::cfg#` and asserts the
//      rule-6 identity tail (the netstring config segment from the ONE
//      identity-suffix producer, `template_specialization_key_suffix`) is
//      BYTE-IDENTICAL for equal config — the producer provably serves both
//      paths at the observable symbol tier. Differing-config split control
//      included.
//   E3 STRUCTURAL ZERO-SIDE-CHANNEL — a unit pin walking the SYNTHESIZED
//      handler AST: every call is one of install/before_hook/after_hook/
//      capture, every capture value argument is a bare config-param
//      identifier, and no other statement/expression kind is present. The
//      machine check that the lowering cannot use a private capability —
//      its output is literally a public-API program.
// ═══════════════════════════════════════════════════════════════════════════

// The shared E1/E2 fixture: the hand-written API twin (`retry_api` +
// module-scope `api_body`) and the sugar-declared def (`retry_sugar`) carry
// IDENTICAL hook body code and IDENTICAL (int, string) config params; the
// equal-config applications (3, "ab") land on identical twin targets. The
// third application (5, "xy") is E2's differing-config split control.
//
// Values: twin targets both compute before 4*3 + len("ab") = 14 → impl 140;
// the split control computes 4*5 + len("xy") = 22 → 220. Total:
// (140*1000 + 140)*1000 + 220 = 140_140_220 (either twin diverging from the
// other shifts a digit block — the behavioral-equality refuter).
const S4D_EQUIVALENCE_SRC: &str = r#"
fn api_body<Args>(args: Args, times: int, tag: string) -> Args {
    args[0] = args[0] * times + tag.length()
    return args
}

annotation retry_api(times: int, tag: string) {
  targets: [function]
  comptime post(target, ctx) {
    install(before_hook(api_body, [capture("times", times), capture("tag", tag)]))
  }
}

annotation retry_sugar(times: int, tag: string) {
  targets: [function]
  before(args) {
    args[0] = args[0] * times + tag.length()
    return args
  }
}

@retry_api(3, "ab")
fn victim_api(a: int) -> int { return a * 10 }

@retry_sugar(3, "ab")
fn victim_sugar(a: int) -> int { return a * 10 }

@retry_sugar(5, "xy")
fn victim_split(a: int) -> int { return a * 10 }

(victim_api(4) * 1000 + victim_sugar(4)) * 1000 + victim_split(4)
"#;

fn row_for<'a>(
    compiler: &'a crate::compiler::BytecodeCompiler,
    target: &str,
) -> &'a crate::compiler::template_specialization::install_registry::HookInstallRecord {
    compiler
        .hook_install_registry
        .iter()
        .find(|row| row.target_name == target)
        .unwrap_or_else(|| panic!("registry row for {target}"))
}

// E1 — BEHAVIORAL equivalence: identical executed values AND registry-row
// agreement on hook kind, capture names + LiftedConst renderings, the
// `::cfg#{n}` arity head, and the (implicit) application target.
#[test]
fn s4d_e1_sugar_and_handwritten_api_twin_agree_behaviorally() {
    let (value, compiler) = top_level_i64(S4D_EQUIVALENCE_SRC);
    assert_eq!(
        value, 140_140_220,
        "the sugar twin and the hand-written API twin execute IDENTICALLY \
         (both equal-config targets 140); the split control executes its own config (220)"
    );
    assert_eq!(compiler.hook_install_registry.len(), 3, "one row per install");

    let api = row_for(&compiler, "victim_api");
    let sugar = row_for(&compiler, "victim_sugar");
    assert_eq!(
        api.hook_kind, sugar.hook_kind,
        "row agreement: hook kind (both Before)"
    );
    assert_eq!(api.hook_kind, TemplateHookKind::Before);
    assert_eq!(
        api.captures, sugar.captures,
        "row agreement: capture names + LiftedConst renderings, in declared order"
    );
    assert_eq!(
        api.captures,
        vec![
            ("times".to_string(), "3".to_string()),
            ("tag".to_string(), "\"ab\"".to_string())
        ]
    );
    for row in [api, sugar] {
        assert!(
            row.specialized_symbol.contains("::cfg#2"),
            "row agreement: the two-value config arity head: {}",
            row.specialized_symbol
        );
    }
    // Target agreement: both paths install on the IMPLICIT application
    // target (their own annotated fn) — neither path names a target.
    assert_eq!(api.target_name, "victim_api");
    assert_eq!(sugar.target_name, "victim_sugar");
    assert_eq!(api.annotation_name, "retry_api");
    assert_eq!(sugar.annotation_name, "retry_sugar");
}

// E2 — IDENTITY-SUFFIX equivalence: the `::cfg#` tail of both specialized
// symbols is BYTE-IDENTICAL for equal config (the ONE identity-suffix
// producer serves both paths); differing config splits the tail.
#[test]
fn s4d_e2_cfg_identity_suffix_is_byte_identical_across_both_paths() {
    let (_, compiler) = top_level_i64(S4D_EQUIVALENCE_SRC);

    let split = |target: &str| -> (String, String) {
        let symbol = row_for(&compiler, target).specialized_symbol.clone();
        let (head, tail) = symbol
            .split_once("::cfg#")
            .unwrap_or_else(|| panic!("{target}: symbol carries a ::cfg# segment: {symbol}"));
        (head.to_string(), tail.to_string())
    };
    let (api_head, api_tail) = split("victim_api");
    let (sugar_head, sugar_tail) = split("victim_sugar");
    let (_, split_tail) = split("victim_split");

    assert_eq!(
        api_tail, sugar_tail,
        "equal config: the rule-6 identity tail (netstring segments) is BYTE-IDENTICAL \
         across the hand-written and sugar paths"
    );
    assert!(
        api_tail.starts_with("2::"),
        "the tail begins with the two-value arity head: {api_tail}"
    );
    assert_ne!(
        api_head, sugar_head,
        "why full-symbol equality is impossible: the sugar's minted hygienic body-fn \
         name differs from the hand-written `api_body` (heads: {api_head} vs {sugar_head})"
    );
    assert_ne!(
        split_tail, api_tail,
        "differing-config SPLIT control: (5, \"xy\") produces a different identity tail"
    );
}

// E3 — STRUCTURAL ZERO-SIDE-CHANNEL: walk the synthesized handler AST and
// whitelist-check every node. Any statement or expression kind outside the
// exact `install(before_hook|after_hook(<minted>, [capture("p", p), …]))`
// shape panics — the machine proof that the lowering's output is literally
// a public-API program (C3-G2: zero private side-channels BY CONSTRUCTION).
#[test]
fn s4d_e3_synthesized_handler_ast_is_public_api_only() {
    use crate::compiler::statements::annotation_declarations::sugar_lowering::{
        lower_typed_config_declarative_hooks, SugarLowering,
    };
    use shape_ast::ast::{AnnotationHandlerType, BlockItem, Expr, Item, Literal};

    let src = r#"
annotation retry(times: int, tag: string) {
  targets: [function]
  before(args) {
    args[0] = args[0] * times + tag.length()
    return args
  }
  after(result) {
    return result + times
  }
}
"#;
    let program = shape_ast::parse_program(src).expect("fixture parses");
    let definition = program
        .items
        .iter()
        .find_map(|item| match item {
            Item::AnnotationDef(definition, _) => Some(definition.clone()),
            _ => None,
        })
        .expect("fixture has an annotation definition");
    let compiler = crate::compiler::BytecodeCompiler::new();
    let SugarLowering {
        post_handler,
        body_fns,
    } = lower_typed_config_declarative_hooks(&compiler, &definition)
        .expect("the TypedConfig definition lowers")
        .expect("declarative hooks produce a lowering");

    // The handler shell: a zero-param `comptime post` (config arrives via
    // the S4b typed injection, exactly as for a hand-written handler).
    assert!(matches!(
        post_handler.handler_type,
        AnnotationHandlerType::ComptimePost
    ));
    assert!(post_handler.params.is_empty(), "zero declared params");
    assert!(post_handler.return_type.is_none());

    let config_params = ["times", "tag"];
    let minted_names: Vec<&str> = body_fns.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(minted_names.len(), 2, "one minted body fn per declarative hook");

    // Whitelist walk. Every `match` arm below names an ALLOWED node; every
    // else-branch panics — nothing outside the four public spellings can
    // survive the walk.
    let Expr::Block(block, _) = &post_handler.body else {
        panic!("the synthesized body is a block, got {:?}", post_handler.body);
    };
    assert_eq!(
        block.items.len(),
        2,
        "exactly one install statement per declarative hook, nothing else"
    );
    let expected_hooks = ["before_hook", "after_hook"];
    for (index, item) in block.items.iter().enumerate() {
        let BlockItem::Expression(expr) = item else {
            panic!("non-expression item in the synthesized handler: {item:?}");
        };
        // install(<hook_call>)
        let Expr::FunctionCall {
            name: install_name,
            const_args,
            args,
            named_args,
            ..
        } = expr
        else {
            panic!("non-call statement in the synthesized handler: {expr:?}");
        };
        assert_eq!(install_name, "install", "the outer call is `install`");
        assert!(const_args.is_empty() && named_args.is_empty());
        assert_eq!(args.len(), 1, "install takes exactly the hook call");
        // before_hook/after_hook(<minted_ident>, [captures])
        let Expr::FunctionCall {
            name: hook_name,
            const_args,
            args: hook_args,
            named_args,
            ..
        } = &args[0]
        else {
            panic!("install's argument must be a hook-builtin call: {:?}", args[0]);
        };
        assert_eq!(
            hook_name, expected_hooks[index],
            "hooks lower in declaration order"
        );
        assert!(const_args.is_empty() && named_args.is_empty());
        assert_eq!(hook_args.len(), 2, "hook builtin takes (fn_ident, captures)");
        let Expr::Identifier(fn_ident, _) = &hook_args[0] else {
            panic!("the hook's template must be a bare identifier: {:?}", hook_args[0]);
        };
        assert_eq!(
            fn_ident, minted_names[index],
            "the identifier names THIS hook's minted body fn"
        );
        let Expr::Array(captures, _) = &hook_args[1] else {
            panic!("the captures argument must be an array literal: {:?}", hook_args[1]);
        };
        assert_eq!(
            captures.len(),
            config_params.len(),
            "one capture per config param"
        );
        for (capture, expected_param) in captures.iter().zip(config_params) {
            // capture("p", p) — name literal + BARE config-param identifier.
            let Expr::FunctionCall {
                name: capture_name,
                const_args,
                args: capture_args,
                named_args,
                ..
            } = capture
            else {
                panic!("captures array element must be a capture() call: {capture:?}");
            };
            assert_eq!(capture_name, "capture");
            assert!(const_args.is_empty() && named_args.is_empty());
            assert_eq!(capture_args.len(), 2);
            let Expr::Literal(Literal::String(literal_name), _) = &capture_args[0] else {
                panic!("capture's first arg must be a string literal: {:?}", capture_args[0]);
            };
            let Expr::Identifier(value_ident, _) = &capture_args[1] else {
                panic!(
                    "capture's VALUE arg must be a BARE config-param identifier \
                     (zero side-channels): {:?}",
                    capture_args[1]
                );
            };
            assert_eq!(literal_name, expected_param, "captures in declared order");
            assert_eq!(
                value_ident, literal_name,
                "the value identifier IS the named config param"
            );
        }
    }
}
