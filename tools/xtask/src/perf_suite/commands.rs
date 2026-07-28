//! Command surfaces over the suite: the three tripwires plus the two
//! deliberate re-pinning acts (`integrity --record`, `record-environment`).
//!
//! Re-pinning is never a way to silence a failing check: both commands rewrite
//! a committed file, so the change shows up as a reviewable diff with an author
//! and a date rather than as a flag someone passed once in CI.

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};

use super::report::{ComparisonState, NoiseVerdict, classify_noise};
use super::{Manifest, Report, RunOptions, env, integrity, primary_statistics, print_summary};

/// Tripwire 1, in its runnable form.
pub fn noise_check(
    repo_root: &Path,
    out_dir: &Path,
    iterations: Option<usize>,
    node: Option<PathBuf>,
    build: bool,
    bound_override: Option<f64>,
) -> Result<()> {
    let manifest = Manifest::load(repo_root)?;
    let bound = bound_override.unwrap_or(manifest.measurement.noise_bound_pct);

    eprintln!("noise check: first run");
    let first = super::run_suite(
        repo_root,
        &RunOptions {
            iterations,
            warmup: None,
            node: node.clone(),
            build,
            out: Some(out_dir.join("noise-run-1.json")),
            quiet: false,
        },
    )?;
    eprintln!("noise check: second run (same revision, same binary)");
    let second = super::run_suite(
        repo_root,
        &RunOptions {
            iterations,
            warmup: None,
            node,
            // The binary is already built and hashed; rebuilding between the
            // two runs would put a different artefact under the second one.
            build: false,
            out: Some(out_dir.join("noise-run-2.json")),
            quiet: false,
        },
    )?;

    if first.revision.shape_binary_sha256 != second.revision.shape_binary_sha256 {
        bail!(
            "the shape binary changed between the two runs ({} then {}); a noise check is only \
             meaningful at one revision",
            first.revision.shape_binary_sha256,
            second.revision.shape_binary_sha256
        );
    }

    let outcome = classify_noise(
        &primary_statistics(&first),
        &primary_statistics(&second),
        bound,
    );

    println!();
    println!(
        "Noise check — two consecutive runs at {} (bound {bound:.1}%)",
        &first.revision.git_commit
    );
    println!(
        "  {:<24} {:<10} {:>10} {:>10} {:>10}",
        "workload", "runtime", "run 1 ms", "run 2 ms", "deviation"
    );
    let second_stats = primary_statistics(&second);
    for (workload, runtime, first_ms) in primary_statistics(&first) {
        let second_ms = second_stats
            .iter()
            .find(|(w, r, _)| *w == workload && *r == runtime)
            .map(|(_, _, ms)| *ms);
        println!(
            "  {:<24} {:<10} {:>10.1} {:>10} {:>10}",
            workload,
            runtime,
            first_ms,
            second_ms
                .map(|ms| format!("{ms:.1}"))
                .unwrap_or_else(|| "-".to_string()),
            second_ms
                .map(|ms| format!("{:.1}%", super::report::deviation_pct(first_ms, ms)))
                .unwrap_or_else(|| "-".to_string()),
        );
    }

    std::fs::create_dir_all(out_dir).ok();
    std::fs::write(
        out_dir.join("noise-outcome.json"),
        serde_json::to_string_pretty(&outcome)? + "\n",
    )
    .ok();

    match outcome.verdict {
        NoiseVerdict::Reproducible => {
            println!(
                "\n  PASS — every measurement agreed within {bound:.1}% across two consecutive \
                 runs at one revision."
            );
            return Ok(());
        }
        NoiseVerdict::MachineContended => {
            println!();
            for violation in &outcome.violations {
                println!(
                    "  EXCEEDED: {} [{}] {:.1}ms vs {:.1}ms = {:.1}% > {:.1}%",
                    violation.workload,
                    violation.runtime,
                    violation.first_ms,
                    violation.second_ms,
                    violation.deviation_pct,
                    violation.bound_pct
                );
            }
            let shape_held = outcome
                .violations
                .iter()
                .all(|violation| violation.runtime != "shape");
            bail!(
                "INCONCLUSIVE (machine contention): {}. This is not a finding about Shape. \
                 Re-run on an idle machine; do not widen the bound to make it pass.",
                if shape_held {
                    "the pinned reference runtime — the control — moved beyond the bound while \
                     every Shape measurement held, so the machine was not quiet"
                } else {
                    "every Shape exceedance is shadowed by the reference moving by the same \
                     order on the same workload, so the machine was not quiet"
                }
            );
        }
        NoiseVerdict::ShapeUnstable => {
            println!();
            for violation in &outcome.violations {
                println!(
                    "  EXCEEDED: {} [{}] {:.1}ms vs {:.1}ms = {:.1}% > {:.1}%",
                    violation.workload,
                    violation.runtime,
                    violation.first_ms,
                    violation.second_ms,
                    violation.deviation_pct,
                    violation.bound_pct
                );
            }
            bail!(
                "{} measurement(s) exceeded the declared {bound:.1}% noise bound with the \
                 reference runtime holding steady — the instability is on the Shape side",
                outcome.violations.len()
            );
        }
    }
}

/// Tripwire 3, in its runnable form.
pub fn integrity_command(repo_root: &Path, record: bool) -> Result<()> {
    let manifest = Manifest::load(repo_root)?;
    let digest_path = repo_root.join(&manifest.integrity.digest_file);
    let covered = integrity::collect_covered(repo_root, &manifest.integrity.covered)?;
    let actual = integrity::hash_covered(repo_root, &covered)?;

    if record {
        std::fs::write(&digest_path, integrity::render_digest_file(&actual))
            .with_context(|| format!("writing {}", digest_path.display()))?;
        println!(
            "recorded {} benchmark file hashes into {}",
            actual.len(),
            manifest.integrity.digest_file
        );
        return Ok(());
    }

    let text = std::fs::read_to_string(&digest_path)
        .with_context(|| format!("reading {}", digest_path.display()))?;
    let recorded = integrity::parse_digest_file(&text)?;
    let violations = integrity::verify(&recorded, &actual);
    if violations.is_empty() {
        println!(
            "benchmark integrity ok — {} covered files match {}",
            actual.len(),
            manifest.integrity.digest_file
        );
        return Ok(());
    }
    for violation in &violations {
        println!("  {}", violation.describe());
    }
    bail!(
        "{} benchmark-integrity violation(s). Benchmarks measure the compiler; the compiler does \
         not get to rewrite the benchmarks. If a change is genuinely intended, re-record it as its \
         own reviewable commit.",
        violations.len()
    );
}

/// Re-pin the manifest environment to this machine.
pub fn record_environment_command(repo_root: &Path, node: Option<&Path>) -> Result<()> {
    let manifest_path = repo_root.join(super::MANIFEST_PATH);
    let text = std::fs::read_to_string(&manifest_path)?;
    let mut value: serde_json::Value = serde_json::from_str(&text)?;

    let resolved = env::resolve_node(node);
    let environment = env::capture(resolved.as_deref())?;
    let identity = environment.identity();

    let mut object = serde_json::Map::new();
    object.insert("identity".to_string(), serde_json::json!(identity));
    for (key, field_value) in &environment.fields {
        object.insert(key.clone(), serde_json::json!(field_value));
    }
    value["environment"] = serde_json::Value::Object(object);

    if let Some(node_version) = environment.get("node_version") {
        value["reference"]["pinned_version"] = serde_json::json!(node_version);
    }
    if let Some(v8) = environment.get("node_v8_version") {
        value["reference"]["pinned_v8_version"] = serde_json::json!(v8);
    }
    if let Some(binary) = environment.get("node_binary") {
        value["reference"]["pinned_binary"] = serde_json::json!(binary);
    }

    std::fs::write(&manifest_path, serde_json::to_string_pretty(&value)? + "\n")?;
    println!(
        "pinned environment identity {identity} into {}",
        super::MANIFEST_PATH
    );
    Ok(())
}

/// Tripwire 2 across two report files.
pub fn compare_command(first: &Path, second: &Path) -> Result<()> {
    let load = |path: &Path| -> Result<Report> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading report {}", path.display()))?;
        serde_json::from_str(&text).with_context(|| format!("parsing report {}", path.display()))
    };
    let a = load(first)?;
    let b = load(second)?;

    if a.environment_identity != b.environment_identity {
        let diffs = b.environment.diff(&a.environment);
        println!("comparison REFUSED — the two reports were taken on different environments:");
        for diff in &diffs {
            println!("  {}: {:?} vs {:?}", diff.field, diff.pinned, diff.captured);
        }
        bail!(
            "environment identities differ ({} vs {}); these reports are not comparable",
            a.environment_identity,
            b.environment_identity
        );
    }

    println!(
        "Comparing {} → {} (environment {})",
        a.revision.git_commit, b.revision.git_commit, a.environment_identity
    );
    println!(
        "  {:<24} {:>12} {:>12} {:>10}",
        "workload", "before ms", "after ms", "change"
    );
    let before = primary_statistics(&a);
    let after = primary_statistics(&b);
    for (workload, runtime, before_ms) in &before {
        if runtime != "shape" {
            continue;
        }
        let after_ms = after
            .iter()
            .find(|(w, r, _)| w == workload && r == "shape")
            .map(|(_, _, ms)| *ms);
        let change = after_ms
            .map(|ms| format!("{:+.1}%", (ms - before_ms) / before_ms * 100.0))
            .unwrap_or_else(|| "-".to_string());
        println!(
            "  {:<24} {:>12.1} {:>12} {:>10}",
            workload,
            before_ms,
            after_ms
                .map(|ms| format!("{ms:.1}"))
                .unwrap_or_else(|| "-".to_string()),
            change
        );
    }
    Ok(())
}

/// Print a report that was produced elsewhere (used by `run` and the tests).
pub fn summarize(report: &Report) {
    print_summary(report);
    if let ComparisonState::Refused { .. } = report.comparison {
        eprintln!(
            "note: no ratios were rendered — see the `comparison` block in the report for the \
             fields that differ."
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_report_round_trips_through_json() {
        // The report is the evidence artefact; if it cannot be re-read, the
        // `compare` tripwire cannot work.
        let manifest = Manifest::load(&super::super::repo_root()).unwrap();
        let json = serde_json::to_string(&manifest).unwrap();
        let back: Manifest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.workloads.len(), manifest.workloads.len());
    }
}
