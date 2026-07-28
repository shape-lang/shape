//! Benchmark-file integrity (PERF-SUITE tripwire 3).
//!
//! CLAUDE.md's Benchmark Integrity rule — "benchmarks measure the compiler; the
//! compiler does not get to rewrite the benchmarks" — is a rule a reviewer has
//! to notice being broken. This module makes it mechanical: every workload
//! source file is hashed into a committed digest file, and a modification, a
//! deletion, or an unrecorded addition is a violation with a name.
//!
//! Result and tracking files (`RESULTS.md`, `tracking/*.tsv`, compiled
//! artefacts) are deliberately outside the covered set: those are outputs,
//! which are meant to change.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::Path;
use walkdir::WalkDir;

/// One covered directory tree, filtered to a single source extension.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoveredTree {
    pub dir: String,
    pub ext: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Violation {
    /// A covered file's contents changed. The benchmark-integrity rule.
    Modified {
        path: String,
        recorded: String,
        actual: String,
    },
    /// A recorded file is gone.
    Missing { path: String },
    /// A covered file exists that the digest file does not record. Catches a
    /// workload added without going through `integrity --record`.
    Unrecorded { path: String },
}

impl Violation {
    pub fn describe(&self) -> String {
        match self {
            Violation::Modified {
                path,
                recorded,
                actual,
            } => format!("modified: {path} (recorded {recorded}, actual {actual})"),
            Violation::Missing { path } => format!("missing: {path}"),
            Violation::Unrecorded { path } => format!("unrecorded: {path}"),
        }
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

/// Parse the `<sha256>  <relative path>` digest format (the `sha256sum` shape,
/// so the file is checkable with standard tools too).
pub fn parse_digest_file(text: &str) -> Result<BTreeMap<String, String>> {
    let mut out = BTreeMap::new();
    for (lineno, line) in text.lines().enumerate() {
        let line = line.trim_end();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (hash, path) = line
            .split_once("  ")
            .with_context(|| format!("line {}: expected '<sha256>  <path>'", lineno + 1))?;
        if hash.len() != 64 || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
            bail!("line {}: '{hash}' is not a sha256 digest", lineno + 1);
        }
        if out.insert(path.to_string(), hash.to_string()).is_some() {
            bail!("line {}: duplicate entry for {path}", lineno + 1);
        }
    }
    Ok(out)
}

pub fn render_digest_file(records: &BTreeMap<String, String>) -> String {
    let mut out = String::from(
        "# Benchmark-file integrity digest -- PERF-SUITE tripwire 3 (ADR-018 §1).\n\
         # Workload sources are immutable: a change here without a recorded reason\n\
         # is the benchmark-integrity rule being broken. Regenerate deliberately\n\
         # with `cargo run -p xtask -- perf-suite integrity --record`.\n",
    );
    for (path, hash) in records {
        out.push_str(&format!("{hash}  {path}\n"));
    }
    out
}

/// Every covered file, as repo-root-relative paths, deterministically ordered.
pub fn collect_covered(repo_root: &Path, trees: &[CoveredTree]) -> Result<Vec<String>> {
    let mut paths = Vec::new();
    for tree in trees {
        let root = repo_root.join(&tree.dir);
        if !root.exists() {
            bail!(
                "covered tree {} does not exist (manifest and repository disagree)",
                tree.dir
            );
        }
        for entry in WalkDir::new(&root).sort_by_file_name() {
            let entry = entry.with_context(|| format!("walking {}", tree.dir))?;
            if !entry.file_type().is_file() {
                continue;
            }
            if entry.path().extension().and_then(|e| e.to_str()) != Some(tree.ext.as_str()) {
                continue;
            }
            let rel = entry
                .path()
                .strip_prefix(repo_root)
                .context("covered file outside the repository root")?;
            paths.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}

pub fn hash_covered(repo_root: &Path, paths: &[String]) -> Result<BTreeMap<String, String>> {
    let mut out = BTreeMap::new();
    for path in paths {
        let bytes = std::fs::read(repo_root.join(path))
            .with_context(|| format!("reading covered file {path}"))?;
        out.insert(path.clone(), sha256_hex(&bytes));
    }
    Ok(out)
}

/// Compare recorded digests against what is on disk. Pure over its inputs so
/// the decision table is unit-testable without touching a repository.
pub fn verify(
    recorded: &BTreeMap<String, String>,
    actual: &BTreeMap<String, String>,
) -> Vec<Violation> {
    let mut violations = Vec::new();
    for (path, recorded_hash) in recorded {
        match actual.get(path) {
            None => violations.push(Violation::Missing { path: path.clone() }),
            Some(actual_hash) if actual_hash != recorded_hash => {
                violations.push(Violation::Modified {
                    path: path.clone(),
                    recorded: recorded_hash.clone(),
                    actual: actual_hash.clone(),
                })
            }
            Some(_) => {}
        }
    }
    for path in actual.keys() {
        if !recorded.contains_key(path) {
            violations.push(Violation::Unrecorded { path: path.clone() });
        }
    }
    violations.sort_by_key(|v| match v {
        Violation::Modified { path, .. } => (0, path.clone()),
        Violation::Missing { path } => (1, path.clone()),
        Violation::Unrecorded { path } => (2, path.clone()),
    });
    violations
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    const H1: &str = "aa11223344556677889900aabbccddeeff00112233445566778899aabbccddee";
    const H2: &str = "bb11223344556677889900aabbccddeeff00112233445566778899aabbccddee";

    #[test]
    fn unchanged_tree_has_no_violations() {
        let recorded = map(&[("benchmarks/charter/shape/a.shape", H1)]);
        assert!(verify(&recorded, &recorded).is_empty());
    }

    #[test]
    fn a_modified_benchmark_is_a_violation() {
        let recorded = map(&[("benchmarks/charter/shape/a.shape", H1)]);
        let actual = map(&[("benchmarks/charter/shape/a.shape", H2)]);
        let violations = verify(&recorded, &actual);
        assert_eq!(
            violations,
            vec![Violation::Modified {
                path: "benchmarks/charter/shape/a.shape".to_string(),
                recorded: H1.to_string(),
                actual: H2.to_string(),
            }]
        );
    }

    #[test]
    fn a_deleted_benchmark_is_a_violation() {
        let recorded = map(&[("benchmarks/charter/shape/a.shape", H1)]);
        let violations = verify(&recorded, &map(&[]));
        assert_eq!(
            violations,
            vec![Violation::Missing {
                path: "benchmarks/charter/shape/a.shape".to_string()
            }]
        );
    }

    #[test]
    fn an_added_benchmark_is_a_violation_until_recorded() {
        let actual = map(&[("benchmarks/charter/shape/new.shape", H1)]);
        let violations = verify(&map(&[]), &actual);
        assert_eq!(
            violations,
            vec![Violation::Unrecorded {
                path: "benchmarks/charter/shape/new.shape".to_string()
            }]
        );
    }

    #[test]
    fn digest_file_round_trips() {
        let records = map(&[("benchmarks/charter/shape/a.shape", H1), ("b.mjs", H2)]);
        let rendered = render_digest_file(&records);
        assert_eq!(parse_digest_file(&rendered).unwrap(), records);
    }

    #[test]
    fn digest_file_rejects_a_non_digest_line() {
        assert!(parse_digest_file("notahash  some/path").is_err());
    }

    #[test]
    fn digest_file_rejects_duplicate_paths() {
        let text = format!("{H1}  a.shape\n{H2}  a.shape\n");
        assert!(parse_digest_file(&text).is_err());
    }

    /// The committed suite must verify against its committed digest file.
    /// This is the tripwire in its CI-runnable form: `just test-fast` fails if
    /// any covered benchmark source has been edited without re-recording.
    #[test]
    fn committed_benchmark_corpus_matches_its_recorded_digests() {
        let repo_root = super::super::repo_root();
        let manifest = super::super::Manifest::load(&repo_root).expect("load charter manifest");
        let paths = collect_covered(&repo_root, &manifest.integrity.covered).expect("collect");
        let actual = hash_covered(&repo_root, &paths).expect("hash");
        let digest_path = repo_root.join(&manifest.integrity.digest_file);
        let text = std::fs::read_to_string(&digest_path).expect("read digest file");
        let recorded = parse_digest_file(&text).expect("parse digest file");
        let violations = verify(&recorded, &actual);
        assert!(
            violations.is_empty(),
            "benchmark integrity violated:\n{}",
            violations
                .iter()
                .map(Violation::describe)
                .collect::<Vec<_>>()
                .join("\n")
        );
    }

    /// Every workload the manifest declares must exist on disk and be covered
    /// by the digest file — a workload cannot be listed but unhashed.
    #[test]
    fn every_declared_workload_is_covered_by_the_digest_file() {
        let repo_root = super::super::repo_root();
        let manifest = super::super::Manifest::load(&repo_root).expect("load charter manifest");
        let text = std::fs::read_to_string(repo_root.join(&manifest.integrity.digest_file))
            .expect("read digest file");
        let recorded = parse_digest_file(&text).expect("parse digest file");
        for workload in &manifest.workloads {
            for rel in [&workload.shape, &workload.reference] {
                let full = format!("{}/{}", manifest.suite_dir, rel);
                assert!(
                    repo_root.join(&full).exists(),
                    "declared workload file {full} does not exist"
                );
                assert!(
                    recorded.contains_key(&full),
                    "declared workload file {full} is not in the integrity digest"
                );
            }
        }
    }
}
