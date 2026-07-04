//! Sentinel test for the strict-typing defection guard.
//!
//! Mirrors `scripts/check-no-dynamic.sh` at the Rust-test layer: scans the
//! source trees (`crates/`, `bin/`, `tools/`, `extensions/`) for a forbidden
//! pattern that the gate's frozen baseline pins to 0. The shell recipe runs
//! in CI / pre-commit; this test fails the workspace build if the pattern is
//! reintroduced, so the prohibition survives even when the recipe is skipped.
//!
//! See CLAUDE.md "Forbidden Patterns" + ADR-006 §2.7.7. Documentation trees
//! (`docs/`, `CLAUDE.md`, plans) intentionally discuss the patterns by name
//! and are NOT scanned — same scope rule as the shell recipe.
//!
//! NOTE: this file must not itself contain the forbidden literal contiguously
//! (the shell gate is a plain text grep with no comment-awareness). The needle
//! is assembled from fragments at runtime; comments describe it by part only.

use std::path::{Path, PathBuf};

/// Walk the repo's source scope and collect every `.rs` file's contents.
fn source_rs_contents() -> Vec<(PathBuf, String)> {
    // CARGO_MANIFEST_DIR = <repo>/crates/shape-vm — repo root is two up.
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest
        .parent()
        .and_then(Path::parent)
        .expect("repo root (two levels above crates/shape-vm)");

    let scope = ["crates", "bin", "tools", "extensions"];
    let mut out = Vec::new();
    for dir in scope {
        collect_rs(&repo_root.join(dir), &mut out);
    }
    out
}

fn collect_rs(dir: &Path, out: &mut Vec<(PathBuf, String)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // Skip build artifacts.
            if path.file_name().is_some_and(|n| n == "target") {
                continue;
            }
            collect_rs(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            if let Ok(src) = std::fs::read_to_string(&path) {
                out.push((path, src));
            }
        }
    }
}

/// ADR-006 §2.7.7 forbidden: Bool-default slot fabrication. A restore that
/// swallows `serializable_to_slot`'s failure and fabricates a Bool-zero slot
/// (the `.unwrap_or(...)`-with-a-Bool-zero-default shape) is the canonical
/// defection this guard exists to prevent. Mirrors the baseline row whose
/// PCRE pins the same shape to a count of 0.
#[test]
fn no_bool_default_slot_fabrication() {
    let matcher = bool_default_matcher();
    let hits: Vec<PathBuf> = source_rs_contents()
        .into_iter()
        .filter(|(_, src)| matcher(src))
        .map(|(p, _)| p)
        .collect();

    assert!(
        hits.is_empty(),
        "Bool-default slot fabrication (ADR-006 §2.7.7 forbidden) found in: {hits:#?}. \
         A restore must `?`-propagate (surface-and-stop), never default the slot \
         to a Bool-zero on failure."
    );
}

/// Returns a matcher for the forbidden shape, allowing any whitespace after
/// the comma — equivalent to the baseline PCRE. The two fragments are kept
/// separate so this very file never contains the forbidden literal
/// contiguously (which would trip the plain-text shell gate on itself).
fn bool_default_matcher() -> impl Fn(&str) -> bool {
    // Reassembled at runtime: "unwrap_or((0," ++ <whitespace> ++ "NativeKind::Bool))".
    let prefix = ["unwrap", "_or((0,"].concat();
    let suffix = ["NativeKind", "::Bool))"].concat();
    move |src: &str| {
        let mut from = 0;
        while let Some(idx) = src[from..].find(&prefix) {
            let start = from + idx + prefix.len();
            // Consume whitespace between the comma and the kind.
            let rest = src[start..].trim_start();
            if rest.starts_with(&suffix) {
                return true;
            }
            from = start;
        }
        false
    }
}
