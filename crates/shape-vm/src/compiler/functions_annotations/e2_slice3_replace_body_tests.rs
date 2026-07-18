//! ADR-009 E2 #18 (slice 3) — pre-analysis materialization of a closure-bearing
//! `replace body` edit (the C0911 flip at the compiler tier).
//!
//! # What these pins fix and how they observe it
//!
//! A `replace body` swaps a function's body at PASS-2, after the shared analyzer
//! has already handed off its closure inference facts. So the analyzer never saw
//! the replacement's closures and no structural fact was published — an edited
//! closure's capture resolved to a `[C0911]` MissingInferenceFact quarantine
//! (the C2 #13 named finding). Slice 3 materializes a const-free, top-level,
//! CLOSURE-BEARING replacement through the executed declaration-discovery
//! pre-pass into the analysis-program clone, so the analyzer infers the stamped
//! replacement closures and publishes their facts.
//!
//! The materialization's one persistent, observable side effect is the hygienic
//! `ctx.original` shadow's reservation in `generated_symbols`, journaled through
//! the already-open C2 `InstallTransaction`. So `generated_symbols.contains_name`
//! of the shadow is the compiler-tier witness that the edit was materialized:
//!
//! - a CLOSURE-BEARING replacement is materialized → the shadow IS reserved;
//! - a CLOSURE-FREE replacement is NOT (nothing to publish; stays pass-2-only,
//!   byte-unchanged legacy behavior) → the shadow is NOT reserved (pass-2
//!   registers the shadow in the function TABLE, but never in `generated_symbols`);
//! - a FAILED closure-bearing install rolls the reservation back atomically.
//!
//! The end-to-end capture-resolution flip (C0911 no longer fires, `base` resolves
//! to an exact Active capture) is pinned in the LSP suite
//! (`generated_captures::semantic_tests::replace_body_edit_capture_is_observed_and_resolved`).

use super::BytecodeCompiler;

/// The edited function every fixture here targets.
const EDIT_TARGET_NAME: &str = "answer";

/// A function-target `replace body` program over a no-arg `fn answer() -> int`.
/// `replacement_body` is literal Shape (not an f-string), so no brace escaping.
fn replace_body_program(replacement_body: &str) -> String {
    format!(
        r#"
annotation edit_answer() {{
  targets: [function]
  comptime post(target, ctx) {{
    replace body {{ {replacement_body} }}
  }}
}}

@edit_answer()
fn answer() -> int {{ 7 }}

answer()
"#
    )
}

/// A CLOSURE-BEARING replacement (mirrors the C0911 LSP fixture): an explicit
/// `move`-capture closure whose call yields 42. This is the case slice-3
/// materializes pre-analysis.
const CLOSURE_BEARING_REPLACEMENT: &str = "let base = 40\n\
     let worker = |; move base| base + 2\n\
     return worker()";

/// RESERVE / PROVENANCE PIN — a closure-bearing replace-body edit is
/// pre-analysis-materialized: the hygienic `ctx.original` shadow is reserved in
/// `generated_symbols` (journaled through the open C2 InstallTransaction) by the
/// pre-pass, and the reservation survives a successful compile's commit.
#[test]
fn closure_bearing_replace_body_reserves_the_hygienic_shadow() {
    let program = shape_ast::parse_program(&replace_body_program(CLOSURE_BEARING_REPLACEMENT))
        .expect("slice-3 fixture parses");
    let mut compiler = BytecodeCompiler::new();
    let shadow = compiler.original_body_shadow_name(EDIT_TARGET_NAME);

    compiler
        .compile_in_place(&program)
        .expect("a valid closure-bearing replace-body edit installs (default Strict)");

    assert!(
        compiler.generated_symbols.contains_name(&shadow),
        "a closure-bearing replacement is pre-analysis-materialized, so its hygienic \
         ctx.original shadow is reserved in generated_symbols (journaled through the C2 \
         InstallTransaction)"
    );
    // The edit still ships via the pass-2 swap byte-unchanged.
    assert!(
        compiler
            .program
            .functions
            .iter()
            .any(|f| f.name == EDIT_TARGET_NAME),
        "the edited function still installs through the authoritative pass-2 swap"
    );
}

/// CONTROL — a CLOSURE-FREE replace-body edit is NOT pre-analysis-materialized
/// (it has no closure fact to publish): it stays the byte-unchanged pass-2 path,
/// so no shadow reservation appears in `generated_symbols` (pass-2 registers the
/// shadow in the function table, never in the D1 reservation table). Pairs with
/// the reserve pin above — the contrast is what proves the scope is closure-
/// bearing, not "every replace body". The edit still installs.
#[test]
fn closure_free_replace_body_stays_pass2_and_reserves_no_shadow() {
    let program =
        shape_ast::parse_program(&replace_body_program("return 42")).expect("control parses");
    let mut compiler = BytecodeCompiler::new();
    let shadow = compiler.original_body_shadow_name(EDIT_TARGET_NAME);

    compiler
        .compile_in_place(&program)
        .expect("a closure-free replace-body edit installs (default Strict)");

    assert!(
        !compiler.generated_symbols.contains_name(&shadow),
        "a closure-free replacement is NOT pre-analysis-materialized — it stays pass-2-only \
         (byte-unchanged legacy behavior), so no shadow is reserved in generated_symbols"
    );
    assert!(
        compiler
            .program
            .functions
            .iter()
            .any(|f| f.name == EDIT_TARGET_NAME),
        "the closure-free edit still installs through the pass-2 swap"
    );
}

/// ROLLBACK / ATOMICITY PIN (pairs with the reserve pin above — same
/// closure-bearing shape: on success the shadow IS reserved, on failure it is
/// NOT, so the reservation is transaction-scoped). A closure-bearing replace-body
/// edit whose replacement REJECTS leaves NOTHING behind: the open C2
/// InstallTransaction restores to compile-start, so neither the journaled shadow
/// reservation nor the edited function survives ("replacement body AND shadow
/// both gone"). The failure lever is an immutable reassignment — a PASS-2
/// rejection (the same lever c2_slice4 uses), so it fires AFTER the pre-pass
/// reserved the shadow AND after the analyzer already saw the materialized
/// closure: the rollback is exercised over a live, published reservation.
#[test]
fn failed_closure_bearing_replace_body_rolls_back_the_shadow_reservation() {
    let replacement = "let base = 40\n\
         let worker = |; move base| base + 2\n\
         let z = 1\n\
         z = 2\n\
         return worker() + z";
    let program =
        shape_ast::parse_program(&replace_body_program(replacement)).expect("fixture parses");
    let mut compiler = BytecodeCompiler::new();
    let shadow = compiler.original_body_shadow_name(EDIT_TARGET_NAME);

    compiler.compile_in_place(&program).expect_err(
        "the type-mismatched binding in the replacement must reject the compile; an Ok means \
         the edit was not policed by the install transaction",
    );

    assert!(
        !compiler.generated_symbols.contains_name(&shadow),
        "atomicity: a failed closure-bearing edit rolls the journaled shadow reservation back \
         to compile-start (nothing half-materialized survives)"
    );
    assert!(
        !compiler
            .program
            .functions
            .iter()
            .any(|f| f.name == EDIT_TARGET_NAME),
        "atomicity: a failed edit leaves no function-table entry for the edited function"
    );
}
