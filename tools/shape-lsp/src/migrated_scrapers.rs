//! Tombstones for LSP fix derivations that have moved to the structured
//! channel (ADR-017 §4, R23).
//!
//! # Where the counts live
//!
//! The shrink-only baselines for the LSP's dual-authority debt are **not**
//! here. They are the `lsp-parallel-validators` and `lsp-message-scraping`
//! sets of the #135 legacy census,
//! `docs/program/adr011-012/baselines/tooling-evidence-inventory.json`,
//! checked by `just check-legacy-baselines` and by verify-merge CHECK 15.
//! That census owns the patterns, the scope, the owner lists, and the
//! monotonic-non-increasing rule.
//!
//! An earlier revision of this file carried its own scan with its own
//! pattern. Two counts of one population is the dual-authority defect this
//! ticket exists to remove, and the two had already diverged (74 against 77)
//! because they defined the population differently. The census wins; this
//! file keeps only what the census cannot express.
//!
//! # What the census cannot express
//!
//! The census ratchets a total per set. A commit that deletes one derivation
//! and revives another leaves the total unchanged, and a revival inside a
//! file that already owns sites adds no new owner, so a swap can pass. The
//! tombstone below names the specific derivations that have migrated and
//! fails if any of them is referenced again, at any count, in any file.

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    /// The LSP source tree scanned for revived derivations.
    const LSP_SRC: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/src");

    /// This file, excluded from its own scan — it names the tombstoned
    /// symbols by definition.
    const SELF_FILE: &str = "migrated_scrapers.rs";

    /// Fix derivations that now arrive from the compiler as structured
    /// edits. These names must not appear anywhere in the tree again:
    /// not as a definition, not as a call.
    ///
    /// `parse_non_exhaustive_match` recovered an enum name and its missing
    /// variants from rendered prose. The checker proves both
    /// (`shape_runtime::type_system::fixes::non_exhaustive_match_fix`), so
    /// the derivation is deleted rather than duplicated.
    const MIGRATED: &[&str] = &["parse_non_exhaustive_match"];

    fn lsp_sources() -> Vec<PathBuf> {
        fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
            let entries =
                std::fs::read_dir(dir).unwrap_or_else(|e| panic!("reading {}: {e}", dir.display()));
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, out);
                } else if path.extension().is_some_and(|e| e == "rs")
                    && path.file_name().is_some_and(|n| n != SELF_FILE)
                {
                    out.push(path);
                }
            }
        }

        let mut out = Vec::new();
        walk(Path::new(LSP_SRC), &mut out);
        assert!(
            out.len() > 20,
            "expected the LSP source tree at {LSP_SRC}, found {} files",
            out.len()
        );
        out.sort();
        out
    }

    /// Tripwire 2: a migrated derivation stays gone. If any code path reaches
    /// for it again — even one that compiles, because the definition came
    /// back alongside the call — this turns red and names the site.
    #[test]
    fn migrated_derivations_are_never_revived() {
        let mut offenders = Vec::new();
        for path in lsp_sources() {
            let source = std::fs::read_to_string(&path).expect("read source");
            for (index, line) in source.lines().enumerate() {
                if line.trim_start().starts_with("//") {
                    continue;
                }
                for name in MIGRATED {
                    if line.contains(name) {
                        let short = path.strip_prefix(LSP_SRC).unwrap_or(&path);
                        offenders.push(format!("{}:{}", short.display(), index + 1));
                    }
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "migrated derivations {MIGRATED:?} are referenced again at {offenders:?} — \
             the compiler proves these facts now; do not re-derive them"
        );
    }

    /// The scan reaches the files it claims to. Without this, a silently
    /// empty walk would read as a clean tombstone.
    #[test]
    fn scan_reaches_the_fix_surface() {
        let names: Vec<String> = lsp_sources()
            .iter()
            .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .collect();
        for expected in ["code_actions.rs", "diagnostics.rs", "structured_fixes.rs"] {
            assert!(
                names.iter().any(|n| n == expected),
                "{expected} must be in the scanned set, got {} files",
                names.len()
            );
        }
    }
}
