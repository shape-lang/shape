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
/// install → weave), returning both the result and the compiler. The VM-native
/// `std::core::remote` module is registered in the extension registry (exactly
/// as `configuration.rs::register_stdlib_modules` does for the real binary) so
/// the `@remote` short-circuit's `__call_raising` emission resolves the native
/// export — the compiler needs this for `is_native_module_export`.
fn compile_source(
    src: &str,
) -> (
    shape_ast::error::Result<()>,
    crate::compiler::BytecodeCompiler,
) {
    let program = shape_ast::parse_program(src).expect("fixture parses");
    let mut compiler = crate::compiler::BytecodeCompiler::new()
        .with_extensions(vec![crate::executor::create_remote_module_exports()]);
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

// ── CP2 — config captures on a decision hook ───────────────────────────────

/// A declarative decision hook carrying a `capture(...)` config value resolves
/// the config in the short-circuit. Pre-S5 (S4 first cut) this REJECTED loudly
/// ("capturing values into a decision `before` hook … is not yet supported");
/// CP2 lifts the bound by baking the config as a ConstLift prologue constant
/// (`bake_captures_into_def`). This is the exact path @remote's `addr` rides.
#[test]
fn cp2_decision_hook_config_capture_resolves_in_short_circuit() {
    let src = r#"
annotation offset(delta: int) on function {
  before(args) -> HookDecision<Args> {
    return HookDecision::Return(args[0] + delta)
  }
}

@offset(500)
fn t(x: int) -> int { return x + 1000 }

t(7)
"#;
    // The hook short-circuits with `args[0] + delta` = 7 + 500 = 507 (not 1007).
    assert_eq!(
        top_level_i64(src),
        507,
        "the config capture `delta` resolves in the decision short-circuit"
    );
}

/// Two applications with DIFFERENT config values bake DISTINCT constants (the
/// Dec-95 rule-6 specialization identity, preserved on the decision path).
#[test]
fn cp2_decision_hook_distinct_config_bakes_distinct_short_circuits() {
    let src = r#"
annotation offset(delta: int) on function {
  before(args) -> HookDecision<Args> {
    return HookDecision::Return(args[0] + delta)
  }
}

@offset(3)
fn a(x: int) -> int { return x + 1000 }

@offset(5)
fn b(x: int) -> int { return x + 1000 }

a(10) * 100000 + b(10)
"#;
    // a: 10+3 = 13; b: 10+5 = 15.
    assert_eq!(top_level_i64(src), 13 * 100000 + 15);
}

// ── CP3 + CP4 — the @remote markers + short-circuit R-elaboration ───────────
//
// These fixtures exercise the SAME markers + `__call_raising` short-circuit the
// stdlib `@remote` annotation uses (CP6 lands the stdlib block); a synthetic
// `myremote` annotation carries them so the compiler weave is exercised without
// a live `shape serve` transport. The end-to-end EXECUTED no-recursion proof
// (dispatch to the shadow, not the wrapper) rides the CP6 book/CLI fixtures
// against a loopback server; here the no-recursion invariant is proven
// STRUCTURALLY on the woven bytecode.

use crate::bytecode::{Constant, Function};

/// A synthetic `@remote`-shaped annotation carrying the two weave markers and
/// the raising short-circuit, plus a `target` declaration + a trivial non-remote
/// top level (the fixtures COMPILE — running would attempt a real wire call).
fn remote_shaped_fixture(target: &str) -> String {
    format!(
        r#"
annotation myremote(addr: string) on function {{
  before(args) -> HookDecision<Args> {{
    return HookDecision::Return(__call_raising(addr, __remote_impl_ref(), __remote_arg_pack()))
  }}
}}

{target}

1
"#
    )
}

/// The `Constant::Function` ids a registered function's body pushes as values.
fn pushed_function_ids(
    compiler: &crate::compiler::BytecodeCompiler,
    function: &Function,
) -> Vec<u16> {
    use crate::bytecode::{Instruction, OpCode, Operand};
    let end = function.entry_point + function.body_length;
    compiler.program.instructions[function.entry_point..end]
        .iter()
        .filter_map(|instr: &Instruction| match (instr.opcode, instr.operand) {
            (OpCode::PushConst, Some(Operand::Const(idx))) => {
                match compiler.program.constants.get(idx as usize) {
                    Some(Constant::Function(fid)) => Some(*fid),
                    _ => None,
                }
            }
            _ => None,
        })
        .collect()
}

/// Hazard #1 (no self-recursion): the woven decision HELPER's short-circuit
/// pushes the impl SHADOW's fn-ref as the `__call_raising` callee, NEVER the
/// wrapper's — a bytecode-level proof that a `@remote`'d fn dispatches to its
/// hygienic impl copy, not to itself.
#[test]
fn cp3_cp4_remote_short_circuit_targets_shadow_not_wrapper() {
    let src = remote_shaped_fixture(
        r#"@myremote("127.0.0.1:9")
fn compute(x: int) -> int { return x * 2 }"#,
    );
    let compiler = compiled_ok(&src);

    let wrapper_id = compiler
        .find_function("compute")
        .expect("wrapper registered") as u16;
    let shadow_name = compiler.template_weave_impl_name("compute");
    let shadow_id = compiler
        .find_function(&shadow_name)
        .expect("shadow registered") as u16;
    let helper_name = compiler.template_weave_decision_helper_name("compute");
    let helper_index = compiler
        .find_function(&helper_name)
        .expect("helper registered");
    let helper = &compiler.program.functions[helper_index];

    let pushed = pushed_function_ids(&compiler, helper);
    assert!(
        pushed.contains(&shadow_id),
        "the short-circuit callee is the impl shadow's fn-ref (E4-D3); pushed ids = {pushed:?}, \
         shadow id = {shadow_id}"
    );
    assert!(
        !pushed.contains(&wrapper_id),
        "the short-circuit callee is NEVER the wrapper's fn-ref (no self-recursion); pushed \
         ids = {pushed:?}, wrapper id = {wrapper_id}"
    );
}

/// CP4: `@remote fn f(x: int) -> int` type-checks — the synthesized
/// `__call_raising` short-circuit is elaborated at the shadow's BARE R (`int`),
/// not the builtin's declared `_`. (If it stayed `_`, the helper's `-> int`
/// return could not prove and this would be a compile error.)
#[test]
fn cp4_remote_short_circuit_types_at_bare_r() {
    let src = remote_shaped_fixture(
        r#"@myremote("127.0.0.1:9")
fn compute(x: int) -> int { return x * 2 }"#,
    );
    let (result, _) = compile_source(&src);
    result.expect("@remote fn f(x:int) -> int type-checks (bare-R short-circuit)");
}

/// CP4: `@remote fn g(x: int) -> Result<int, string>` (R is itself a Result)
/// type-checks — the payload delivers `Result<int, string>` (== R) and transport
/// failure still raises; composes with the propagate path at zero extra work.
#[test]
fn cp4_remote_short_circuit_result_r_composes() {
    let src = remote_shaped_fixture(
        r#"@myremote("127.0.0.1:9")
fn compute(x: int) -> Result<int, string> { return Ok(x * 2) }"#,
    );
    let (result, _) = compile_source(&src);
    result.expect("@remote fn g(x:int) -> Result<int,string> type-checks (Result-R short-circuit)");
}

/// CP4: a homogeneous MULTI-int signature fits the OUTER-TypedArray pack arm.
#[test]
fn cp4_remote_homogeneous_multi_int_signature_compiles() {
    let src = remote_shaped_fixture(
        r#"@myremote("127.0.0.1:9")
fn multiply(a: int, b: int) -> int { return a * b }"#,
    );
    let (result, _) = compile_source(&src);
    result.expect("@remote fn multiply(a:int,b:int) -> int type-checks (homogeneous pack)");
}

/// CP4: an `Array<int>`-argument signature fits the pack arm (the arg pack is a
/// TypedArray of TypedArrays — `[data]`).
#[test]
fn cp4_remote_array_argument_signature_compiles() {
    let src = remote_shaped_fixture(
        r#"@myremote("127.0.0.1:9")
fn transform(data: Array<int>) -> int { return data.length() }"#,
    );
    let (result, _) = compile_source(&src);
    result.expect("@remote fn transform(data:Array<int>) -> int type-checks");
}

// ── CP3 — fail-loud discipline ─────────────────────────────────────────────

/// Hazard #2 (fail LOUD): a stray `__remote_impl_ref()` reference outside a
/// decision-hook weave (where the weave substitutes it) is a LOUD compile error
/// — never a silent misdispatch, never a fabricated value.
#[test]
fn cp3_stray_impl_ref_marker_out_of_weave_fails_loud() {
    let src = r#"
fn use_marker() -> int {
    return __remote_impl_ref()
}

use_marker()
"#;
    let message = compile_error(src);
    // Fail LOUD, naming the marker. (A truly-undefined bare marker is caught by
    // the analysis pass's undefined-function reject before the compile-tier
    // marker backstop; both are loud and name the marker.)
    assert!(
        message.contains("__remote_impl_ref"),
        "a stray @remote impl-ref marker fails loud, naming the marker; got: {message}"
    );
}

/// Hazard #2: a stray `__remote_arg_pack()` reference is equally loud.
#[test]
fn cp3_stray_arg_pack_marker_out_of_weave_fails_loud() {
    let src = r#"
fn use_marker() -> Array<int> {
    return __remote_arg_pack()
}

use_marker()
"#;
    let message = compile_error(src);
    assert!(
        message.contains("__remote_arg_pack"),
        "a stray @remote arg-pack marker fails loud, naming the marker; got: {message}"
    );
}

/// CP3 homogeneous-signature guard: a HETEROGENEOUS multi-argument `@remote`
/// signature gets the CLEAN named-defer (the positional pack is one homogeneous
/// `Array<T>`), NOT a cryptic downstream array-element-type mismatch.
#[test]
fn cp3_heterogeneous_multiarg_remote_is_clean_named_defer() {
    let src = remote_shaped_fixture(
        r#"@myremote("127.0.0.1:9")
fn mixed(a: int, b: string) -> int { return a }"#,
    );
    let message = compile_error(&src);
    assert!(
        message.contains("heterogeneous multi-argument signature") && message.contains("#83"),
        "a heterogeneous @remote signature gets the clean named-defer + issue pointer; got: {message}"
    );
}

// ── CP5 — async + 0-ary loud defers ────────────────────────────────────────

/// async `@remote` is a LOUD named-defer: the sync `__call_raising`
/// short-circuit cannot run on an async executor thread (no
/// `__call_async_raising` sibling yet). Never a silent blocking call.
#[test]
fn cp5_async_remote_is_loud_named_defer() {
    let src = remote_shaped_fixture(
        r#"@myremote("127.0.0.1:9")
async fn compute(x: int) -> int { return x * 2 }"#,
    );
    let message = compile_error(&src);
    assert!(
        message.contains("async fn") && message.contains("#83"),
        "async @remote is a loud named-defer with an issue pointer; got: {message}"
    );
}

/// 0-ary `@remote` is a LOUD reject (the decision-hook machinery has no arguments
/// to short-circuit over a 0-parameter target). Never a silent no-op.
#[test]
fn cp5_zero_ary_remote_is_loud_reject() {
    let src = remote_shaped_fixture(
        r#"@myremote("127.0.0.1:9")
fn snapshot_id() -> int { return 7 }"#,
    );
    let message = compile_error(&src);
    assert!(
        message.contains("no parameters") || message.contains("no arguments"),
        "0-ary @remote is loudly rejected; got: {message}"
    );
}
