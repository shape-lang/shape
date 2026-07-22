//! ADR-009 E2 #18 (slice 0) — the pre-analysis materialization spike (E2-D6).
//!
//! # What the spike decides
//!
//! C2's named finding (`checked_body/mod.rs`, pinned by the LSP quarantine
//! `generated_captures::semantic_tests::replace_body_edit_capture_is_observed_but_specialization_quarantined`)
//! is: a `replace body` REPLACEMENT is swapped at PASS-2, AFTER the shared
//! analyzer has already run and handed off its structural inference facts, so
//! the analyzer never sees the replacement's closure and no fact is published →
//! the capture resolves to a `[C0911]` MissingInferenceFact quarantine instead
//! of an exact specialization identity. The fix E2 owns is to materialize the
//! replacement PRE-analysis (the same window a FRESH generated `extend` method
//! is materialized in), so its closures reach the analyzer and get facts.
//!
//! E2-D6's slice-0 obligation is a STOP decision: can a directive-produced
//! REPLACEMENT run through the EXISTING pre-analysis window
//! (`materialize_computed_comptime_extends`, the executed declaration-discovery
//! pre-pass that runs BEFORE `analyze_program_full` in `compile_in_place_inner`)
//! with the existing machinery — or does emitting/applying the directive
//! structurally require pass-2 context (compiled functions, registered types),
//! making pre-analysis execution impossible without analyzer-ordering changes?
//!
//! # The verdict these pins carry: FEASIBLE-WITH-EXISTING-MACHINERY
//!
//! The pre-pass ALREADY executes `targets: [function]` comptime-post handlers
//! pre-analysis. `collect_declaration_discovery_targets`
//! (`declaration_discovery.rs`) collects every annotated non-comptime function
//! as a `DeclarationDiscoveryTarget::Function`, and the fixed-point loop in
//! `materialize_computed_comptime_extends` executes each target's handler and
//! processes its directives — but its directive-materialization arm handles ONLY
//! `Extend` / `ExtendItems` (functions_annotations.rs ~:2200), dropping every
//! other directive (including `ReplaceBody`) at `_ => continue`.
//!
//! So the missing precondition is NOT missing pass-2 context. The handler
//! EXECUTION is already pre-analysis (proven by `t2`, where a function-target
//! `extend` handler's method is materialized by the same pre-pass), and the
//! `ReplaceBody` directive application reads only pre-analysis-available state:
//! the target `func_def` AST, the semantic-freeze handle (installed at
//! `compile_in_place_inner` BEFORE the pre-pass), and the effective pass modes
//! (functions_annotations.rs `ReplaceBody` arm ~:3049). What is absent is purely
//! the WIRING: the pre-pass discards the `ReplaceBody` directive rather than
//! threading the replacement (and its hygienic `ctx.original` shadow) into the
//! analysis program the way it already threads a fresh `extend`. The analyzer
//! ordering is unchanged — `analyze_program_full` still runs at the same point;
//! only WHAT program it sees gains the replacement, exactly as it already gains
//! prepended generated `extend` items.
//!
//! The named E2-D6 machinery — the `InstallTransaction` bracketing
//! `compile_in_place` and the `GeneratedNodeOrigin` provenance issuer — is
//! already present and already used on BOTH sides (the pre-pass stamps generated
//! closures via `stamp_generated_closure_provenance`; the `ReplaceBody` arm
//! stamps the replacement via `stamp_generated_replacement_body`). The wiring
//! point is `materialize_computed_comptime_extends`'s directive loop plus a
//! replacement-edit return channel that `compile_in_place_inner` applies to
//! `analysis_program` before `analyze_program_full`.
//!
//! # These pins are tripwires
//!
//! `t1` and `t3` assert the CURRENT reality: a function-target `replace body`
//! contributes NOTHING to the executed authority's generated items
//! (`generated_analysis_items`, which only the pre-pass populates — never
//! pass-2), while the replacement still SHIPS via the pass-2 swap. When E2's
//! fix materializes the replacement pre-analysis, both empty assertions FLIP
//! (the executed authority will carry the replacement / its shadow), which is
//! the intended signal that the C0911 seam has moved. `t2` is the positive
//! control: the reusable machinery already threads a sibling directive shape.

use super::BytecodeCompiler;
use crate::compiler::executed_generated_items;

fn parse(source: &str) -> shape_ast::ast::Program {
    shape_ast::parse_program(source).expect("E2 slice-0 spike fixture parses")
}

/// Names of every method recorded on a generated `extend <type_name>` block in
/// the executed-discovery output (mirrors the E3 discovery test's helper).
fn discovered_extend_methods(items: &[shape_ast::ast::Item], type_name: &str) -> Vec<String> {
    let mut methods = Vec::new();
    for item in items {
        if let shape_ast::ast::Item::Extend(extend, _) = item {
            let name = match &extend.type_name {
                shape_ast::ast::TypeName::Simple(n) => n.to_string(),
                shape_ast::ast::TypeName::Generic { name, .. } => name.to_string(),
            };
            if name == type_name {
                methods.extend(extend.methods.iter().map(|m| m.name.clone()));
            }
        }
    }
    methods
}

/// A `targets: [function]` annotation whose comptime-post handler replaces the
/// annotated function's body. Capture-free (`return 42`) so it compiles green in
/// Strict — the spike is about materialization TIMING, not capture semantics
/// (the capture case is the C0911 quarantine fixture in the LSP suite). Mirrors
/// the `tests/smokes-jit-closure/c2-replace-body-edit.shape` smoke shape.
const FUNCTION_TARGET_REPLACE_BODY: &str = r#"
annotation edit_answer() on function {
    comptime post(target, ctx) {
        replace body {
            return 42
        }
    }
}

@edit_answer()
fn answer() -> int { 7 }

answer()
"#;

/// A `targets: [function]` annotation whose comptime-post handler emits an
/// `extend` — the sibling directive shape the pre-pass ALREADY materializes.
/// Identical target shape to `FUNCTION_TARGET_REPLACE_BODY`; only the directive
/// kind differs, which is what isolates the gap to directive WIRING.
const FUNCTION_TARGET_EXTEND: &str = r#"
annotation add_number_method() on function {
    comptime post(target, ctx) {
        extend Number {
            method doubled() { self * 2.0 }
        }
    }
}

@add_number_method()
fn marker() { 0 }
"#;

/// THE GAP (E2-D6). A function-target `replace body` handler runs in the
/// executed pre-pass — the same pass that materializes a function-target
/// `extend` in `t2` — but its `ReplaceBody` directive is DROPPED by the
/// pre-pass directive loop (`_ => continue`), so the replacement contributes
/// nothing to the executed authority the analyzer reads. This is the exact
/// missing precondition E2 closes: the handler EXECUTES pre-analysis, but the
/// replacement is never threaded to the analysis program, so no inference fact
/// is published for its closures (the C0911 root).
///
/// SLICE-3 REBASELINE (Fork A landed): this fixture's replacement is
/// closure-FREE (`return 42`), and E2 slice-3 scopes pre-analysis
/// materialization to CLOSURE-BEARING replacements only (a closure-free
/// replacement has no structural inference fact to publish, so materializing it
/// buys nothing) AND threads the edit through a dedicated
/// `pending_replace_body_analysis` channel applied directly to the analysis
/// clone — never through `generated_analysis_items`/`executed_generated_items`.
/// So this authority stays empty on BOTH counts, and the pin is now a CONTROL:
/// a closure-free `replace body` is byte-unchanged pass-2 behavior. The
/// materialization SIGNAL for the closure-bearing case moved to the shadow
/// reservation, pinned in `e2_slice3_replace_body_tests`.
#[test]
fn t1_function_target_replace_body_is_not_materialized_by_the_executed_prepass() {
    let program = parse(FUNCTION_TARGET_REPLACE_BODY);
    let generated = executed_generated_items(&program);
    assert!(
        generated.is_empty(),
        "a closure-free function-target `replace body` is not pre-analysis-materialized \
         (slice-3 scopes materialization to closure-bearing replacements, via a channel \
         separate from the executed-generated-items authority); got {generated:?}"
    );
}

/// THE REUSABLE MACHINERY (positive control). The SAME pre-pass, over a
/// function-target handler that emits `extend Number { doubled }`, DOES
/// materialize the generated method into the executed authority
/// (`generated_analysis_items`) BEFORE the analyzer runs. This proves two of
/// the three feasibility facts directly: (a) `targets: [function]` handlers
/// execute in the pre-analysis window, and (b) a directive they emit is threaded
/// to the analyzer with existing machinery. E2's fix extends this exact loop to
/// also thread `ReplaceBody` (the `t1` gap).
#[test]
fn t2_function_target_extend_is_materialized_by_the_executed_prepass() {
    let program = parse(FUNCTION_TARGET_EXTEND);
    let methods = discovered_extend_methods(&executed_generated_items(&program), "Number");
    assert!(
        methods.iter().any(|m| m == "doubled"),
        "the executed pre-pass materializes a function-target `extend` directive pre-analysis \
         (the machinery E2 reuses for `replace body`); got {methods:?}"
    );
}

/// THE ASYMMETRY (C2's named finding, at the compiler tier). The replacement
/// DOES ship: `compile_in_place` succeeds and the pass-2 swap installs the
/// `replace body` replacement under the user function name. But that same
/// compile leaves the executed authority (`generated_analysis_items`) EMPTY,
/// because the swap lands at pass-2 — after the analyzer already ran over the
/// pre-edit body. Codegen/install is unaffected (the replacement ships); only
/// analysis-time visibility is missing. This is precisely the timing that
/// quarantines the replacement's closure captures to `[C0911]`.
///
/// SLICE-3 REBASELINE (Fork A landed): the closure-free replacement still ships
/// via the pass-2 swap, and `generated_analysis_items` stays empty — but under
/// the landed design that emptiness is now definitional, not the C0911 timing:
/// slice-3 materializes CLOSURE-BEARING replacements through the dedicated
/// `pending_replace_body_analysis` channel (applied straight to the analysis
/// clone), never through `generated_analysis_items`. So this authority is empty
/// for EVERY `replace body` (closure-free or not); the pin holds as a control
/// that the executed-generated-items authority is not the replace-body channel.
/// The closure-bearing materialization is observed via the shadow reservation in
/// `e2_slice3_replace_body_tests`.
#[test]
fn t3_replace_body_replacement_ships_at_pass2_but_is_analyzer_invisible() {
    let program = parse(FUNCTION_TARGET_REPLACE_BODY);
    let mut compiler = BytecodeCompiler::new();

    compiler
        .compile_in_place(&program)
        .expect("the `replace body` replacement ships via the pass-2 swap (default Strict)");

    assert!(
        compiler.generated_analysis_items().is_empty(),
        "the replace-body channel is `pending_replace_body_analysis`, not \
         `generated_analysis_items` — this authority stays empty for a `replace body` edit; \
         got {:?}",
        compiler.generated_analysis_items()
    );
}
