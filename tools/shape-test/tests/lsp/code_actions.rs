//! LSP code action tests: quickfix, organize imports, refactor suggestions.

use shape_test::shape_test::{ShapeTest, range};

// == LSP-N §D regression flow #6: codeAction returns 0 on real diagnostic ====
//
// Audit `v0.3-lsp-parity-audit.md` executive summary item #6: returns 0
// actions on a real `Undefined variable: zzz_undefined` diagnostic — 8 sites
// in `code_actions.rs:77-265` use `.contains("...")` keying that has drifted
// from the diagnostic message format. Currently red; LSP-G closes.

#[test]
fn lsp_n_code_actions_for_undefined_variable_diagnostic() {
    // §D #6: a file with a known `Undefined variable: foo` diagnostic must
    // surface at least one quickfix action. PASSES today at HEAD 7813a652
    // — single-file characterization does not reproduce the §D regression
    // (which used `broken.shape` with `SEMANTIC: Undefined variable: zzz`
    // diagnostic code keying). Regression-prevention coverage; LSP-G
    // close-gate is the manual editor exercise on a real
    // module-cache-bound diagnostic.
    let code = "let x = zzz_undefined + 1\n";
    ShapeTest::new(code)
        .in_range(range(0, 8, 0, 21))
        .expect_code_actions_min(1);
}

// == Quickfix actions =========================================================

#[test]
fn quickfix_actions_on_code_range() {
    // TDD: quickfix actions may not be fully implemented for all diagnostics
    let code = "let x = 42;\nlet y = x + 1;\n";
    ShapeTest::new(code)
        .in_range(range(0, 0, 1, 14))
        .expect_code_actions_ok();
}

#[test]
fn quickfix_on_single_expression() {
    let code = "let result = 10 + 20;\n";
    ShapeTest::new(code)
        .in_range(range(0, 13, 0, 20))
        .expect_code_actions_ok();
}

// == Organize imports =========================================================

#[test]
fn organize_imports_does_not_crash() {
    // TDD: organize imports not fully implemented; verifies no crash on range with imports
    let code = "let x = 1;\nlet y = 2;\nlet z = x + y;\n";
    ShapeTest::new(code)
        .in_range(range(0, 0, 2, 14))
        .expect_code_actions_ok();
}

// == Refactor extract =========================================================

#[test]
fn refactor_extract_on_block_does_not_crash() {
    // TDD: extract-function refactoring not yet implemented
    let code = "fn main() {\n    let a = 1;\n    let b = 2;\n    let c = a + b;\n}\n";
    ShapeTest::new(code)
        .in_range(range(1, 4, 3, 18))
        .expect_code_actions_ok();
}

#[test]
fn code_actions_on_function_definition() {
    let code = "function helper() {\n    return 42;\n}\n";
    ShapeTest::new(code)
        .in_range(range(0, 0, 2, 1))
        .expect_code_actions_ok();
}

// == R8 W10 LSP-G refactor-assists (Decision 4 v0.3 scope) ====================
//
// Decision 4 ratified extract-fn + extract-var for v0.3. The skeletons in
// `code_actions.rs::get_refactor_actions` ship working edits today; these
// E2E flows lock in the contract so the LSP-G+ follow-up that hardens them
// (parameter capture, statement-boundary insertion, multi-line indent)
// stays behavior-compatible.

#[test]
fn lsp_n_refactor_extract_variable_on_expression() {
    // Selecting a single expression should surface an "Extract to
    // variable" action via REFACTOR_EXTRACT.
    let code = "let result = 10 + 20;\n";
    ShapeTest::new(code)
        .in_range(range(0, 13, 0, 20))
        .expect_refactor_extract_action("Extract to variable");
}

#[test]
fn lsp_n_refactor_extract_function_on_multiline() {
    // Multi-line selection should surface an "Extract to function"
    // action via REFACTOR_EXTRACT.
    let code = "fn main() {\n    let a = 1;\n    let b = 2;\n    let c = a + b;\n}\n";
    ShapeTest::new(code)
        .in_range(range(1, 4, 3, 18))
        .expect_refactor_extract_action("Extract to function");
}
