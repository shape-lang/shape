//! Shrink-only baselines for the LSP's dual-authority debt (ADR-017 §4, R23).
//!
//! Two populations here are migration debt, not design:
//!
//! - **message scraping** — code that recovers meaning by probing the text of
//!   a rendered diagnostic message, instead of reading a fact the compiler
//!   proved;
//! - **parallel validators** — hand-written `validate_*` rules the LSP runs
//!   beside the compiler's own checking.
//!
//! Both shrink and never grow. A fix migrated to the structured channel
//! deletes its scraping; a validator's rule either moves into the shared
//! semantic query or is deleted. The constants below are ratchets: growth
//! fails, and a shrink fails until the constant comes down in the same commit
//! that earned it.
//!
//! # Why the whole tree
//!
//! The ADR-017 row text names `analysis.rs:70-81` ("eleven parallel
//! validators") and `code_actions.rs:587-615`. Those line ranges are where
//! the debt is *wired*, not where it lives: the measured tree carries 24
//! `validate_*` definitions across four files, and message scraping is spread
//! over nine. Scoping a ratchet to two files would let the debt grow in the
//! other seven while the number stayed flat, so these walk
//! `tools/shape-lsp/src` at test time. A new scraper in a new file trips the
//! ratchet.
//!
//! This file excludes itself from the scan — its own constants name the
//! patterns it looks for.

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    /// The LSP source tree these baselines measure.
    const LSP_SRC: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/src");

    /// This file, excluded from its own scan.
    const SELF_FILE: &str = "fix_baselines.rs";

    /// Identifiers that hold a rendered diagnostic message. A preceding `.`
    /// counts, so `diagnostic.message` and `d.message` are included.
    const MESSAGE_RECEIVERS: &[&str] = &["message", "msg", "err_msg"];

    /// String methods that read meaning *out of* message text. Ownership and
    /// formatting operations (`clone`, `to_string`, `as_str`, `push_str`) are
    /// not scraping and are deliberately absent.
    const TEXT_PROBES: &[&str] = &[
        "contains",
        "starts_with",
        "ends_with",
        "find",
        "rfind",
        "strip_prefix",
        "strip_suffix",
        "split",
        "splitn",
        "split_whitespace",
        "matches",
        "get",
    ];

    /// Lines that scrape a diagnostic message, across the whole LSP tree.
    ///
    /// The ratchet. Lower this — never raise it.
    ///
    /// 75 → 73 on migrating the non-exhaustive-match fix, which deleted
    /// `parse_non_exhaustive_match`'s `strip_prefix` probe and the
    /// `Non-exhaustive match` dispatch guard. 75 is the measured count at the
    /// wave-1 base (`87f51f61`) and agrees with the #135 inventory.
    const MESSAGE_SCRAPING_BASELINE: usize = 73;

    /// `validate_*` definitions across the whole LSP tree.
    ///
    /// The ratchet. Lower this — never raise it.
    ///
    /// 24 at the wave-1 base and unchanged by this slice: ADR-017 §4 requires
    /// the baseline to exist before the migration that consumes it. The row
    /// text's "eleven" counts only the ones wired into
    /// `analyze_program_semantics`; the other 13 live in `doc_diagnostics.rs`,
    /// `toml_support/diagnostics.rs`, and `analysis/import_registration.rs`.
    const PARALLEL_VALIDATOR_BASELINE: usize = 24;

    /// Scraping helpers already migrated to the structured channel. These
    /// names must not appear anywhere in the tree — not as a definition, not
    /// as a call.
    const MIGRATED_SCRAPERS: &[&str] = &["parse_non_exhaustive_match"];

    /// Every `.rs` file under the LSP source tree, except this one.
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

    /// Whether `line` reads meaning out of a diagnostic message.
    ///
    /// A receiver match requires a non-identifier character before it, so
    /// `some_message.contains(` does not count while `d.message.contains(`
    /// does — the same word boundary the inventory grep used.
    fn scrapes_message(line: &str) -> bool {
        for receiver in MESSAGE_RECEIVERS {
            for probe in TEXT_PROBES {
                let needle = format!("{receiver}.{probe}(");
                let mut from = 0;
                while let Some(rel) = line[from..].find(&needle) {
                    let at = from + rel;
                    let preceded_by_identifier = line[..at]
                        .chars()
                        .next_back()
                        .is_some_and(|c| c.is_alphanumeric() || c == '_');
                    if !preceded_by_identifier {
                        return true;
                    }
                    from = at + needle.len();
                }
            }
        }
        false
    }

    fn count_by_file(matches: impl Fn(&str) -> bool) -> Vec<(String, usize)> {
        let mut counts = Vec::new();
        for path in lsp_sources() {
            let source = std::fs::read_to_string(&path).expect("read source");
            let count = source.lines().filter(|line| matches(line)).count();
            if count > 0 {
                counts.push((short_name(&path), count));
            }
        }
        counts.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        counts
    }

    fn scraping_by_file() -> Vec<(String, usize)> {
        count_by_file(scrapes_message)
    }

    fn validators_by_file() -> Vec<(String, usize)> {
        count_by_file(|line| line.contains("fn validate_"))
    }

    fn short_name(path: &Path) -> String {
        path.strip_prefix(LSP_SRC)
            .unwrap_or(path)
            .display()
            .to_string()
    }

    fn total(counts: &[(String, usize)]) -> usize {
        counts.iter().map(|(_, n)| n).sum()
    }

    #[test]
    fn message_scraping_only_shrinks() {
        let counts = scraping_by_file();
        assert_eq!(
            total(&counts),
            MESSAGE_SCRAPING_BASELINE,
            "message-scraping lines across tools/shape-lsp/src: {counts:?}. \
             Growth is forbidden — a fix that needs message text belongs on \
             the structured channel instead. A shrink means \
             MESSAGE_SCRAPING_BASELINE comes down in this commit."
        );
    }

    #[test]
    fn parallel_validators_only_shrink() {
        let counts = validators_by_file();
        assert_eq!(
            total(&counts),
            PARALLEL_VALIDATOR_BASELINE,
            "validate_* definitions across tools/shape-lsp/src: {counts:?}. \
             Growth is forbidden — a new rule belongs in the shared semantic \
             query. A shrink means PARALLEL_VALIDATOR_BASELINE comes down in \
             this commit."
        );
    }

    /// Tripwire 2: a scraper that has been migrated stays gone. If any code
    /// path reaches for it again — even one that compiles, because the
    /// definition came back with it — this turns red and names the site.
    #[test]
    fn migrated_scrapers_have_no_remaining_call_sites() {
        let mut offenders = Vec::new();
        for path in lsp_sources() {
            let source = std::fs::read_to_string(&path).expect("read source");
            for (index, line) in source.lines().enumerate() {
                if line.trim_start().starts_with("//") {
                    continue;
                }
                for name in MIGRATED_SCRAPERS {
                    if line.contains(name) {
                        offenders.push(format!("{}:{}", short_name(&path), index + 1));
                    }
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "migrated scrapers {MIGRATED_SCRAPERS:?} are still referenced at {offenders:?}"
        );
    }

    /// The measurement finds the code it claims to measure. Without this, a
    /// silently-empty scan would read as a clean shrink.
    #[test]
    fn baselines_measure_the_expected_owners() {
        let scraping = scraping_by_file();
        assert!(
            scraping.iter().any(|(f, _)| f.ends_with("code_actions.rs")),
            "code_actions.rs must appear among scraping owners: {scraping:?}"
        );
        assert!(
            scraping.iter().any(|(f, _)| f.ends_with("diagnostics.rs")),
            "diagnostics.rs must appear among scraping owners: {scraping:?}"
        );

        let validators = validators_by_file();
        assert!(
            validators.len() >= 4,
            "validators span at least four files, found: {validators:?}"
        );
        assert!(
            validators
                .iter()
                .any(|(f, n)| f.ends_with("doc_diagnostics.rs") && *n > 0),
            "doc_diagnostics.rs carries validators the row text omits: {validators:?}"
        );
    }
}
