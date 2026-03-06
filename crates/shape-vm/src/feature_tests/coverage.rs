//! Coverage analysis for feature tests
//!
//! This module analyzes which grammar rules are covered by tests
//! and reports on coverage gaps.

use std::collections::{BTreeMap, BTreeSet};

use super::PEST_RULES;
use super::all_feature_tests;

// ============================================================================
// Coverage Analysis
// ============================================================================

/// Analyze coverage gaps between registered tests and grammar features
pub fn analyze_coverage() -> CoverageReport {
    let grammar: BTreeSet<&str> = PEST_RULES.iter().copied().collect();

    // Collect all covered features from all test modules
    let mut covered: BTreeSet<&str> = BTreeSet::new();
    let mut test_coverage: BTreeMap<&str, Vec<&str>> = BTreeMap::new();

    for test in all_feature_tests() {
        for &feature in test.covers {
            covered.insert(feature);
            test_coverage.entry(feature).or_default().push(test.name);
        }
    }

    // Find uncovered grammar rules
    let uncovered: Vec<&str> = grammar.difference(&covered).copied().collect();

    // Features in tests but not in grammar (possibly stale or internal)
    let unknown: Vec<&str> = covered.difference(&grammar).copied().collect();

    let coverage_pct = if grammar.is_empty() {
        0.0
    } else {
        (covered.intersection(&grammar).count() as f64 / grammar.len() as f64) * 100.0
    };

    CoverageReport {
        total_grammar_features: grammar.len(),
        covered_features: covered.intersection(&grammar).count(),
        uncovered_features: uncovered,
        unknown_features: unknown,
        test_coverage,
        coverage_pct,
    }
}

#[derive(Debug)]
pub struct CoverageReport {
    pub total_grammar_features: usize,
    pub covered_features: usize,
    pub uncovered_features: Vec<&'static str>,
    pub unknown_features: Vec<&'static str>,
    pub test_coverage: BTreeMap<&'static str, Vec<&'static str>>,
    pub coverage_pct: f64,
}

impl std::fmt::Display for CoverageReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Feature Coverage Report")?;
        writeln!(f, "======================")?;
        writeln!(
            f,
            "Coverage: {:.1}% ({}/{} grammar rules)",
            self.coverage_pct, self.covered_features, self.total_grammar_features
        )?;
        writeln!(f)?;

        if !self.uncovered_features.is_empty() {
            writeln!(
                f,
                "Uncovered Grammar Rules ({}):",
                self.uncovered_features.len()
            )?;
            for feature in &self.uncovered_features {
                writeln!(f, "   - {}", feature)?;
            }
            writeln!(f)?;
        }

        if !self.unknown_features.is_empty() {
            writeln!(
                f,
                "Test features not in grammar ({}):",
                self.unknown_features.len()
            )?;
            for feature in &self.unknown_features {
                writeln!(f, "   - {}", feature)?;
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_coverage_report() {
        let report = analyze_coverage();
        println!("{}", report);
        assert!(
            report.total_grammar_features > 0,
            "Expected grammar features to be extracted from pest"
        );
    }
}
