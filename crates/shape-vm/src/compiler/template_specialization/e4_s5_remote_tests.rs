//! ADR-009 E4 S5 — `@remote` reborn on the HookDecision protocol.
//!
//! These are the compiler-machinery unit tests for the five S5 checkpoints
//! (CP1..CP5) that re-implement `@remote` on the final HookDecision protocol:
//!
//! - **CP1** — a DECLARATIVE `before(args) -> HookDecision<Args>` sugar hook
//!   routes to `TemplateSig::PolymorphicDecision` and short-circuits (today only
//!   the explicit `install(before_hook(...))` API could reach the decision plan).
//! - **CP2** — a declarative decision hook may carry a `capture(...)` config
//!   value (the S4 first-cut "no captures" bound is lifted); the config value
//!   resolves in the short-circuit.
//! - **CP3** — the two weave-substituted `@remote` markers (`__remote_impl_ref`
//!   → the impl-shadow fn-ref, `__remote_arg_pack` → the `[p0..pN-1]` pack) lower
//!   to a shadow-fn-ref `__call_raising` call over the positional pack; a stray
//!   marker outside a decision weave FAILS LOUD; a heterogeneous multi-arg
//!   signature gets the CLEAN named-defer (not an array-type mismatch).
//! - **CP4** — the synthesized `__call_raising(addr, shadow, pack)` types at the
//!   shadow's BARE R; `@remote fn f(...) -> int` and `-> Result<int, E>` both
//!   type-check.
//! - **CP5** — async `@remote` and 0-ary `@remote` are LOUD named-defers.
//!
//! The end-to-end `@remote` no-recursion + fail-loud proofs (executed against a
//! loopback `shape serve`) ride the book truth-gate + the CLI fixtures at CP6;
//! these unit tests exercise the compiler weave without a live transport.

use crate::executor::{VMConfig, VirtualMachine};

/// Full production compile (parse → pre-pass → analyzer → pass-2 handler →
/// install → weave), returning both the result and the compiler.
fn compile_source(src: &str) -> (shape_ast::error::Result<()>, crate::compiler::BytecodeCompiler) {
    let program = shape_ast::parse_program(src).expect("fixture parses");
    let mut compiler = crate::compiler::BytecodeCompiler::new();
    compiler.source_text = Some(src.to_string());
    let result = compiler.compile_in_place(&program);
    (result, compiler)
}

fn compiled_ok(src: &str) -> crate::compiler::BytecodeCompiler {
    let (result, compiler) = compile_source(src);
    result.expect("fixture must compile");
    compiler
}

/// Execute the compiled program's top level and return the final int.
fn top_level_i64(src: &str) -> i64 {
    let compiler = compiled_ok(src);
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(compiler.program.clone());
    vm.execute(None)
        .expect("program executes")
        .as_i64()
        .expect("top-level result is an int")
}

/// The compile-error message for a fixture that must be REJECTED.
#[allow(dead_code)] // used by the CP3/CP5 reject fixtures added downstream
fn compile_error(src: &str) -> String {
    let (result, _) = compile_source(src);
    match result {
        Ok(()) => panic!("fixture must be rejected, but it compiled"),
        Err(e) => e.to_string(),
    }
}

// ── CP1 — declarative decision sugar routes + short-circuits ───────────────

/// A declarative `before(args) -> HookDecision<Args>` hook (no config, no
/// markers) short-circuits the target via the SUGAR surface. Pre-S5 the sugar
/// minted `-> Args` unconditionally and this program did not compile (a
/// `HookDecision::Return` in an always-Proceed body is a non-conforming exit);
/// after CP1 it classifies as `PolymorphicDecision`, weaves, and runs.
#[test]
fn cp1_declarative_decision_sugar_short_circuits() {
    let src = r#"
annotation shortcut() on function {
  before(args) -> HookDecision<Args> {
    return HookDecision::Return(args[0])
  }
}

@shortcut
fn t(x: int) -> int { return x + 1000 }

t(7)
"#;
    // The hook returns `args[0]` (= 7), short-circuiting the `+ 1000`.
    assert_eq!(
        top_level_i64(src),
        7,
        "the declarative decision hook short-circuits via the sugar surface (7, not 1007)"
    );
}

/// The Proceed exit of a declarative decision sugar hook still runs the impl.
#[test]
fn cp1_declarative_decision_sugar_proceed_runs_impl() {
    let src = r#"
annotation guard() on function {
  before(args) -> HookDecision<Args> {
    if args[0] < 0 {
      return HookDecision::Return(args[0])
    }
    return HookDecision::Proceed(args)
  }
}

@guard
fn t(x: int) -> int { return x + 1000 }

t(7) * 100000 + (t(-5) + 100000)
"#;
    // t(7): Proceed → 1007. t(-5): Return(-5) → -5 (short-circuit).
    assert_eq!(top_level_i64(src), 1007 * 100000 + (-5 + 100000));
}
