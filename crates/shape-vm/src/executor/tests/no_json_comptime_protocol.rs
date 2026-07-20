//! Absence sentinel for the ADR-009 E1 #17 slice-6 pure deletion.
//!
//! Slice 6 deleted the last three fully-dead remnants of the JSON/string
//! comptime-directive protocol that typed carriers replaced across slices 0–5
//! (deletion commit on `adr009/e1`):
//!
//!   (a) the directive-payload JSON serializer fn (name: `serialize` +
//!       `_directive_payload`) — caller-less since slice 4;
//!   (b) the extend-payload builtin registration (name: two underscores +
//!       `emit_extend`) plus its inline consumer and
//!       `serde_json::from_str` of an `ExtendStatement` — registered but never
//!       emitted since slice 4; the emit side uses the typed-index variant
//!       (that same name with a `_checked` suffix);
//!   (c) the `serde_json::from_str` of a `TypeAnnotation` first branch of
//!       `parse_type_annotation_payload` (branch only; the `__type_probe`
//!       source-reparse remainder SURVIVES for unstamped refs — E1-D8 residual).
//!
//! This guard fails the workspace build if the two named deleted symbols are
//! reintroduced. It mirrors `no_dynamic.rs`: it scans the source trees
//! (`crates/`, `bin/`, `tools/`, `extensions/`) at the Rust-test layer so the
//! prohibition survives independently of any shell gate. Documentation trees
//! (`docs/`, `CLAUDE.md`, `AGENTS.md`) intentionally discuss the deleted
//! protocol by name and are NOT scanned.
//!
//! The two needles are precise, not prefix matches:
//!   * the serializer needle requires the `fn ` definition keyword, so surviving
//!     prose comments that name the symbol in backticks do NOT trip it;
//!   * the extend-builtin needle requires the exact double-quoted string
//!     literal, so the survivors (the `_checked` / `_items` typed-index carriers,
//!     which continue the token past the closing quote) do NOT trip it.
//!
//! NOTE: like `no_dynamic.rs`, this file must not itself contain either needle
//! contiguously (the scan reads every `.rs`, including this one). Each needle is
//! assembled from fragments at runtime; every comment and assert message below
//! describes the needles by part only — never spelled contiguously.

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

fn files_containing(needle: &str) -> Vec<PathBuf> {
    source_rs_contents()
        .into_iter()
        .filter(|(_, src)| src.contains(needle))
        .map(|(p, _)| p)
        .collect()
}

/// (a) The directive-payload JSON serializer fn (deleted in ADR-009 E1 #17
/// slice 6) must not reappear. The needle carries the `fn ` definition keyword,
/// so surviving comments that mention the symbol in backticks are not matched.
#[test]
fn no_serialize_directive_payload_fn() {
    // Reassembled at runtime: "fn serialize_directive" ++ "_payload".
    let needle = ["fn serialize_directive", "_payload"].concat();
    let hits = files_containing(&needle);
    assert!(
        hits.is_empty(),
        "The directive-payload JSON serializer fn (assembled name shows in the \
         needle) was deleted in ADR-009 E1 #17 slice 6 (typed carriers replaced \
         the JSON protocol). It must not be reintroduced. Found in: {hits:#?}"
    );
}

/// (b) The extend-payload JSON builtin registration (deleted in ADR-009 E1 #17
/// slice 6) must not reappear. The needle is the EXACT double-quoted literal, so
/// the typed-index survivors (the `_checked` / `_items` carriers) are not matched.
#[test]
fn no_emit_extend_json_builtin_registration() {
    // Reassembled at runtime: '"__emit' ++ '_extend"'  →  the 15-char literal
    // (double-quote, two underscores, emit_extend, double-quote) INCLUDING both
    // surrounding double-quotes.
    let needle = ["\"__emit", "_extend\""].concat();
    let hits = files_containing(&needle);
    assert!(
        hits.is_empty(),
        "The extend-payload JSON builtin (ExtendStatement reparse; the exact \
         double-quoted name shows in the needle) was deleted in ADR-009 E1 #17 \
         slice 6; the emit side uses the typed-index `_checked` variant. It must \
         not be reintroduced. Found in: {hits:#?}"
    );
}
