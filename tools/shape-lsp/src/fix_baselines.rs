//! Shrink-only baselines for the LSP's dual-authority debt (ADR-017 §4, R23).
//!
//! Two populations here are migration debt, not design:
//!
//! - **message-scraping fix extractors** in `code_actions.rs`, which recover
//!   a fix's operands by pulling substrings out of a rendered diagnostic
//!   message;
//! - **parallel validators** in `analysis.rs`, which re-implement compiler
//!   rules LSP-side.
//!
//! Both shrink and never grow. A fix migrated to the structured channel
//! deletes its extractor; a validator's rule either moves into the shared
//! semantic query or is deleted. The constants below are ratchets: growth
//! fails, and a shrink fails until the constant comes down in the same
//! commit that earned it.
//!
//! Counts are measured from the source itself — `include_str!` reads the
//! files these tests are about — so a new scraper trips the ratchet whether
//! or not anyone remembered to list it. The name lists exist to keep the
//! documented inventory honest against what the measurement finds, and the
//! tombstone list turns red if a migrated extractor is mentioned again.

#[cfg(test)]
const CODE_ACTIONS_SRC: &str = include_str!("code_actions.rs");

#[cfg(test)]
const ANALYSIS_SRC: &str = include_str!("analysis.rs");

/// Message-scraping fix extractors still living in `code_actions.rs`.
///
/// Shrink-only. Do not add entries: a new fix belongs on the structured
/// channel (`shape_diagnostics::SuggestedFix::edit_plan`), emitted by the
/// compiler that proved it.
#[cfg(test)]
const MESSAGE_SCRAPING_EXTRACTORS: &[&str] = &[
    "extract_quoted_name",
    "extract_undefined_name",
    "extract_unused_name",
];

/// The ratchet. Lower this — never raise it.
///
/// 4 → 3 on migrating the non-exhaustive-match fix, whose extractor was
/// `parse_non_exhaustive_match`.
#[cfg(test)]
const MESSAGE_SCRAPING_BASELINE: usize = 3;

/// Extractors already migrated to the structured channel. Their names must
/// not appear anywhere in `code_actions.rs` — not as a definition, not as a
/// call.
#[cfg(test)]
const MIGRATED_MESSAGE_SCRAPERS: &[&str] = &["parse_non_exhaustive_match"];

/// Hand-written validators `analysis.rs` runs in parallel with the
/// compiler's own checking.
///
/// Shrink-only, and untouched by this slice: ADR-017 §4 requires the
/// baseline to exist before the migration that consumes it.
#[cfg(test)]
const PARALLEL_VALIDATORS: &[&str] = &[
    "validate_annotations",
    "validate_async_join",
    "validate_async_structured_concurrency",
    "validate_interpolation_format_specs",
    "validate_comptime_overrides",
    "validate_comptime_side_effects",
    "validate_comptime_builtins_context",
    "validate_trait_bounds",
    "validate_color_rgb_range",
    "validate_foreign_function_types",
    "validate_unused_imports",
];

/// The ratchet. Lower this — never raise it.
#[cfg(test)]
const PARALLEL_VALIDATOR_BASELINE: usize = 11;

#[cfg(test)]
mod tests {
    use super::*;

    /// A message-scraping extractor is a free function whose only input is
    /// the rendered diagnostic text. That signature is the population this
    /// baseline governs, and it is read out of the source rather than taken
    /// on trust from the list above.
    const SCRAPER_SIGNATURE: &str = "(message: &str)";

    fn scrapers_in_source() -> Vec<String> {
        CODE_ACTIONS_SRC
            .lines()
            .filter_map(|line| {
                let rest = line.trim_start().strip_prefix("fn ")?;
                let name = rest.split('(').next()?;
                rest.contains(SCRAPER_SIGNATURE)
                    .then(|| name.trim().to_string())
            })
            .collect()
    }

    fn validators_in_source() -> Vec<String> {
        ANALYSIS_SRC
            .lines()
            .filter_map(|line| {
                let rest = line.trim().strip_prefix("diagnostics.extend(")?;
                let name = rest.split('(').next()?;
                name.starts_with("validate_")
                    .then(|| name.trim().to_string())
            })
            .collect()
    }

    #[test]
    fn message_scraping_extractors_only_shrink() {
        let present = scrapers_in_source();
        assert_eq!(
            present.len(),
            MESSAGE_SCRAPING_BASELINE,
            "message-scraping extractors found in code_actions.rs: {present:?}. \
             Growth is forbidden — a new fix belongs on the structured channel. \
             A shrink means MESSAGE_SCRAPING_BASELINE comes down in this commit."
        );
    }

    /// The documented inventory must name exactly what the measurement
    /// finds, so the list cannot rot into fiction while the ratchet holds.
    #[test]
    fn listed_extractors_match_the_source() {
        let mut present = scrapers_in_source();
        present.sort();
        let mut listed: Vec<String> = MESSAGE_SCRAPING_EXTRACTORS
            .iter()
            .map(|n| (*n).to_string())
            .collect();
        listed.sort();
        assert_eq!(
            present, listed,
            "MESSAGE_SCRAPING_EXTRACTORS must name exactly the extractors \
             that still exist in code_actions.rs"
        );
    }

    /// Tripwire 2: an extractor that has been migrated stays gone. If any
    /// code path reaches for it again — even one that compiles, because the
    /// definition came back with it — this turns red and names the lines.
    #[test]
    fn migrated_extractors_have_no_remaining_call_sites() {
        for name in MIGRATED_MESSAGE_SCRAPERS {
            let mentions: Vec<usize> = CODE_ACTIONS_SRC
                .lines()
                .enumerate()
                .filter(|(_, line)| line.contains(name) && !line.trim_start().starts_with("//"))
                .map(|(i, _)| i + 1)
                .collect();
            assert!(
                mentions.is_empty(),
                "`{name}` migrated to the structured channel but code_actions.rs \
                 still references it at line(s) {mentions:?}"
            );
            assert!(
                !MESSAGE_SCRAPING_EXTRACTORS.contains(name),
                "`{name}` cannot be both migrated and remaining debt"
            );
        }
    }

    #[test]
    fn parallel_validators_only_shrink() {
        let wired = validators_in_source();
        assert_eq!(
            wired.len(),
            PARALLEL_VALIDATOR_BASELINE,
            "validators wired into analyze_program_semantics: {wired:?}. \
             Growth is forbidden — a new rule belongs in the shared semantic \
             query. A shrink means PARALLEL_VALIDATOR_BASELINE comes down in \
             this commit."
        );
    }

    #[test]
    fn listed_validators_match_the_source() {
        let mut wired = validators_in_source();
        wired.sort();
        let mut listed: Vec<String> = PARALLEL_VALIDATORS
            .iter()
            .map(|n| (*n).to_string())
            .collect();
        listed.sort();
        assert_eq!(wired, listed);
    }

    /// The baselines describe files that exist. A rename that moves either
    /// population elsewhere must move its baseline too.
    #[test]
    fn baselines_measure_non_empty_sources() {
        assert!(CODE_ACTIONS_SRC.contains("fn get_quick_fixes("));
        assert!(ANALYSIS_SRC.contains("fn analyze_program_semantics_for_document("));
    }
}
