//! Book doctest runner — verifies all book snippets execute correctly
//! and produce the expected output.

use std::path::PathBuf;

use shape_test::book_snippets::collect_book_snippets;
use shape_test::shape_test::ShapeTest;

fn snippets_dir() -> PathBuf {
    // shape/tools/shape-test/tests/book_doctests.rs → shape-web/book/snippets/
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest.join("../../../shape-web/book/snippets")
}

#[test]
fn book_snippets_run_ok() {
    let snippets = collect_book_snippets(&snippets_dir());
    if snippets.is_empty() {
        // Book snippets dir doesn't exist yet — nothing to test
        return;
    }

    let mut failures = Vec::new();
    for snippet in &snippets {
        let result = std::panic::catch_unwind(|| {
            ShapeTest::new(&snippet.code).expect_run_ok();
        });
        if result.is_err() {
            failures.push(format!("  {}", snippet.file.display()));
        }
    }
    assert!(
        failures.is_empty(),
        "Snippets failed to run:\n{}",
        failures.join("\n")
    );
}

#[test]
fn book_snippets_expected_output() {
    let snippets = collect_book_snippets(&snippets_dir());
    if snippets.is_empty() {
        return;
    }

    let with_expected: Vec<_> = snippets
        .iter()
        .filter(|s| s.expected_output.is_some())
        .collect();
    if with_expected.is_empty() {
        return;
    }

    let mut failures = Vec::new();
    for snippet in &with_expected {
        let expected = snippet.expected_output.as_ref().unwrap();
        let expected = expected.trim_end_matches('\n');
        let result = std::panic::catch_unwind(|| {
            ShapeTest::new(&snippet.code).expect_output(expected);
        });
        if let Err(e) = result {
            let msg = if let Some(s) = e.downcast_ref::<String>() {
                s.clone()
            } else if let Some(s) = e.downcast_ref::<&str>() {
                s.to_string()
            } else {
                "unknown panic".to_string()
            };
            failures.push(format!("  {} — {}", snippet.file.display(), msg));
        }
    }
    assert!(
        failures.is_empty(),
        "Snippets with wrong output:\n{}",
        failures.join("\n")
    );
}

#[test]
fn book_snippets_lsp_ok() {
    let snippets = collect_book_snippets(&snippets_dir());
    if snippets.is_empty() {
        return;
    }

    let mut failures = Vec::new();
    for snippet in &snippets {
        let result = std::panic::catch_unwind(|| {
            ShapeTest::new(&snippet.code).expect_semantic_tokens();
        });
        if result.is_err() {
            failures.push(format!("  {}", snippet.file.display()));
        }
    }
    assert!(
        failures.is_empty(),
        "Snippets failed LSP semantic tokens:\n{}",
        failures.join("\n")
    );
}
