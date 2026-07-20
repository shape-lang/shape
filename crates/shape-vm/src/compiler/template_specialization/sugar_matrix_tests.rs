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
//! | r1  | typed config params → template inputs     | `r1_config_param_enters_the_template_only_as_a_capture`      | annotation-config binding + `capture(name, value)` + `before_hook` + `install` |
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
// Two applications with different config values share ONE value-generic
// specialized handler (the capture stays out of the cache key) while each
// weave delivers its own config value.
#[test]
fn r1_config_param_enters_the_template_only_as_a_capture() {
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
    assert_eq!(
        row_a.function_index, row_b.function_index,
        "config values stay OUT of the specialization cache key (one shared handler)"
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
