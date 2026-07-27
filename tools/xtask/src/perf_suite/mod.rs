//! The performance charter comparison suite and harness (PERF-SUITE, ADR-018
//! §1, issue #186).
//!
//! One command measures the committed workload suite against the pinned
//! reference runtime and emits a machine-readable report carrying everything
//! ADR-016 requires of a performance number: revisions, the hash of the binary
//! that ran, environment identity, benchmark-corpus integrity, and every raw
//! sample.
//!
//! Three tripwires are mechanical rather than procedural:
//!
//! 1. two consecutive runs at one revision must agree within the manifest's
//!    declared noise bound (`perf-suite noise-check`);
//! 2. the report refuses to render a comparison when the captured environment
//!    identity differs from the pinned one (or when the corpus has been
//!    modified);
//! 3. every workload source file is hashed, so the benchmark-integrity rule
//!    fails a test instead of relying on a reviewer noticing.
//!
//! What this suite deliberately does not do: claim that a workload executed
//! natively. Observed `[jit-fallback]` lines are recorded, but their absence
//! proves nothing — nativity is R15's `NativeExecutionWitness` (NATIVE-WITNESS,
//! #117).

pub mod commands;
pub mod env;
pub mod integrity;
pub mod report;

pub use commands::{compare_command, integrity_command, noise_check, record_environment_command};

use anyhow::{Context, Result, bail};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use report::{
    BarInputs, ComparisonState, RunOutcome, Timing, WorkloadResult, canonicalize_output,
    decide_comparison, evaluate_bar,
};

pub const MANIFEST_PATH: &str = "benchmarks/charter/manifest.json";

/// Repository root, derived from this crate's location in the workspace.
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("tools/xtask is two levels below the repository root")
        .to_path_buf()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Charter {
    pub authority: String,
    pub calibration_state: String,
    #[serde(default)]
    pub calibrations: Vec<serde_json::Value>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Measurement {
    pub warmup_runs: usize,
    pub iterations: usize,
    pub primary_statistic: String,
    pub noise_bound_pct: f64,
    pub timeout_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reference {
    pub runtime: String,
    pub pinned_version: String,
    pub pinned_v8_version: String,
    pub pinned_binary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PinnedEnvironment {
    pub identity: String,
    #[serde(flatten)]
    pub environment: env::Environment,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workload {
    pub id: String,
    pub category: String,
    pub shape: String,
    pub reference: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntegritySpec {
    pub digest_file: String,
    pub covered: Vec<integrity::CoveredTree>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub schema: String,
    pub suite_dir: String,
    pub charter: Charter,
    pub measurement: Measurement,
    pub reference: Reference,
    pub environment: PinnedEnvironment,
    pub categories: BTreeMap<String, report::CategorySpec>,
    pub workloads: Vec<Workload>,
    pub integrity: IntegritySpec,
}

impl Manifest {
    pub fn load(repo_root: &Path) -> Result<Manifest> {
        let path = repo_root.join(MANIFEST_PATH);
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading suite manifest {}", path.display()))?;
        let manifest: Manifest = serde_json::from_str(&text)
            .with_context(|| format!("parsing suite manifest {}", path.display()))?;
        for workload in &manifest.workloads {
            if !manifest.categories.contains_key(&workload.category) {
                bail!(
                    "workload {} names category {}, which the manifest does not define",
                    workload.id,
                    workload.category
                );
            }
        }
        Ok(manifest)
    }

    pub fn startup_workload(&self) -> Option<&Workload> {
        self.workloads.iter().find(|w| w.category == "startup")
    }
}

/// A comparison is admissible only against an unmodified corpus on the pinned
/// environment. A modified benchmark is not a milder problem than a different
/// machine: both mean the numbers describe something other than the charter
/// baseline.
pub fn decide_comparison_with_integrity(
    captured: &env::Environment,
    pinned: &env::Environment,
    integrity_violations: &[integrity::Violation],
) -> ComparisonState {
    if !integrity_violations.is_empty() {
        return ComparisonState::Refused {
            reason: format!(
                "comparison refused: {} benchmark-integrity violation(s); the committed corpus \
                 is not the corpus that was measured",
                integrity_violations.len()
            ),
            differing_fields: Vec::new(),
        };
    }
    decide_comparison(captured, pinned)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Revision {
    pub git_commit: String,
    pub git_dirty: bool,
    pub shape_binary: String,
    pub shape_binary_sha256: String,
    pub manifest_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntegrityState {
    pub status: String,
    pub covered_files: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub violations: Vec<integrity::Violation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ReferenceState {
    Available {
        runtime: String,
        binary: String,
        version: String,
    },
    /// Recorded as a structured state; the harness never fabricates a number
    /// for a reference it could not run.
    Unavailable { runtime: String, reason: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    pub schema: String,
    pub generated_utc: String,
    pub charter: Charter,
    pub measurement: Measurement,
    pub revision: Revision,
    pub environment: env::Environment,
    pub environment_identity: String,
    pub pinned_environment_identity: String,
    pub integrity: IntegrityState,
    pub reference: ReferenceState,
    pub comparison: ComparisonState,
    /// One-minute load average at the start and end of the run. Context for a
    /// human reading a bad report; the mechanical contention signal is the
    /// reference runtime's stability in `noise-check`.
    pub load_average_1m: (Option<f64>, Option<f64>),
    pub startup_floor_ms: BTreeMap<String, f64>,
    pub workloads: Vec<WorkloadResult>,
}

pub struct RunOptions {
    pub iterations: Option<usize>,
    pub warmup: Option<usize>,
    pub node: Option<PathBuf>,
    pub build: bool,
    pub out: Option<PathBuf>,
    pub quiet: bool,
}

fn git_output(repo_root: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .current_dir(repo_root)
        .args(args)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn build_shape_binary(repo_root: &Path) -> Result<()> {
    let status = Command::new("cargo")
        .current_dir(repo_root)
        .args(["build", "--release", "--bin", "shape"])
        .status()
        .context("spawning cargo build for the shape binary")?;
    if !status.success() {
        bail!("cargo build --release --bin shape failed");
    }
    Ok(())
}

fn measure(
    program: &Path,
    args: &[&str],
    cwd: &Path,
    warmup: usize,
    iterations: usize,
    timeout_secs: u64,
) -> Result<RunOutcome> {
    let mut samples = Vec::new();
    let mut stdout_canonical = String::new();
    let mut exit_ok = true;
    let mut fallbacks: Vec<String> = Vec::new();

    for run_index in 0..(warmup + iterations) {
        let started = Instant::now();
        let output = Command::new("timeout")
            .current_dir(cwd)
            .arg(timeout_secs.to_string())
            .arg(program)
            .args(args)
            .output()
            .with_context(|| format!("running {}", program.display()))?;
        let elapsed = started.elapsed().as_secs_f64() * 1000.0;

        if !output.status.success() {
            exit_ok = false;
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        for line in stderr.lines() {
            if line.contains("[jit-fallback]") {
                let truncated: String = line.chars().take(240).collect();
                if !fallbacks.contains(&truncated) {
                    fallbacks.push(truncated);
                }
            }
        }
        if run_index == 0 {
            stdout_canonical = canonicalize_output(&String::from_utf8_lossy(&output.stdout));
        }
        if run_index >= warmup {
            samples.push(elapsed);
        }
    }

    Ok(RunOutcome {
        timing: Timing::from_samples(samples),
        exit_ok,
        stdout_canonical,
        jit_fallbacks: fallbacks,
    })
}

fn primary(timing: &Timing, statistic: &str) -> f64 {
    match statistic {
        "median" => timing.median_ms,
        _ => timing.min_ms,
    }
}

pub fn run_suite(repo_root: &Path, opts: &RunOptions) -> Result<Report> {
    let manifest = Manifest::load(repo_root)?;
    let iterations = opts.iterations.unwrap_or(manifest.measurement.iterations);
    let warmup = opts.warmup.unwrap_or(manifest.measurement.warmup_runs);
    let statistic = manifest.measurement.primary_statistic.clone();

    if opts.build {
        build_shape_binary(repo_root)?;
    }
    let shape_binary = repo_root.join("target/release/shape");
    if !shape_binary.exists() {
        bail!(
            "{} does not exist; build it with `cargo build --release --bin shape` or drop \
             --no-build",
            shape_binary.display()
        );
    }

    let covered = integrity::collect_covered(repo_root, &manifest.integrity.covered)?;
    let actual = integrity::hash_covered(repo_root, &covered)?;
    let digest_text = std::fs::read_to_string(repo_root.join(&manifest.integrity.digest_file))
        .with_context(|| format!("reading {}", manifest.integrity.digest_file))?;
    let recorded = integrity::parse_digest_file(&digest_text)?;
    let violations = integrity::verify(&recorded, &actual);

    let node = env::resolve_node(opts.node.as_deref());
    let environment = env::capture(node.as_deref())?;
    let comparison = decide_comparison_with_integrity(
        &environment,
        &manifest.environment.environment,
        &violations,
    );

    let reference_state = match &node {
        Some(path) => ReferenceState::Available {
            runtime: manifest.reference.runtime.clone(),
            binary: path.to_string_lossy().to_string(),
            version: environment
                .get("node_version")
                .unwrap_or("unknown")
                .to_string(),
        },
        None => ReferenceState::Unavailable {
            runtime: manifest.reference.runtime.clone(),
            reason: "no `node` binary was found on PATH and none was supplied with --node"
                .to_string(),
        },
    };

    let suite_dir = repo_root.join(&manifest.suite_dir);
    let load_at_start = env::load_average_1m();
    let mut results: Vec<WorkloadResult> = Vec::new();
    let mut startup_floor: BTreeMap<String, f64> = BTreeMap::new();

    for workload in &manifest.workloads {
        if !opts.quiet {
            eprintln!("  measuring {}", workload.id);
        }
        let shape_file = suite_dir.join(&workload.shape);
        let shape_outcome = measure(
            &shape_binary,
            &["run", shape_file.to_string_lossy().as_ref()],
            repo_root,
            warmup,
            iterations,
            manifest.measurement.timeout_secs,
        )?;

        let reference_outcome = match &node {
            Some(node_path) => {
                let ref_file = suite_dir.join(&workload.reference);
                Some(measure(
                    node_path,
                    &[ref_file.to_string_lossy().as_ref()],
                    repo_root,
                    warmup,
                    iterations,
                    manifest.measurement.timeout_secs,
                )?)
            }
            None => None,
        };

        if workload.category == "startup" {
            startup_floor.insert("shape".to_string(), primary(&shape_outcome.timing, &statistic));
            if let Some(reference) = &reference_outcome {
                startup_floor.insert(
                    manifest.reference.runtime.clone(),
                    primary(&reference.timing, &statistic),
                );
            }
        }

        results.push(WorkloadResult {
            id: workload.id.clone(),
            category: workload.category.clone(),
            outputs_agree: reference_outcome
                .as_ref()
                .map(|r| r.stdout_canonical == shape_outcome.stdout_canonical)
                .unwrap_or(false),
            shape: shape_outcome,
            reference: reference_outcome,
            ratio_process: None,
            ratio_startup_adjusted: None,
            adjusted_ratio_confidence: None,
            bar: report::BarStatus::NotEvaluated {
                reason: "not yet evaluated".to_string(),
            },
        });
    }

    let shape_floor = startup_floor.get("shape").copied().unwrap_or(0.0);
    let reference_floor = startup_floor
        .get(&manifest.reference.runtime)
        .copied()
        .unwrap_or(0.0);

    for result in &mut results {
        let shape_ms = primary(&result.shape.timing, &statistic);
        let reference_ms = result
            .reference
            .as_ref()
            .map(|r| primary(&r.timing, &statistic));

        if let Some(reference_ms) = reference_ms {
            result.ratio_process = report::ratio(reference_ms, shape_ms);
            if result.category != "startup" {
                result.ratio_startup_adjusted = report::startup_adjusted_ratio(
                    reference_ms,
                    reference_floor,
                    shape_ms,
                    shape_floor,
                );
                if reference_ms - reference_floor < 50.0 {
                    result.adjusted_ratio_confidence = Some(
                        "low: the reference kernel time left after subtracting its startup \
                         floor is under 50ms, so this adjusted ratio should not be read \
                         precisely"
                            .to_string(),
                    );
                }
            }
        }

        let category = &manifest.categories[&result.category];
        result.bar = evaluate_bar(&BarInputs {
            category,
            ratio: result.ratio_process,
            comparison_rendered: comparison.rendered(),
            reference_available: result.reference.is_some(),
            outputs_agree: result.outputs_agree,
            shape_ok: result.shape.exit_ok,
        });
    }

    let report = Report {
        schema: report::REPORT_SCHEMA.to_string(),
        generated_utc: Utc::now().to_rfc3339(),
        charter: manifest.charter.clone(),
        measurement: Measurement {
            iterations,
            warmup_runs: warmup,
            ..manifest.measurement.clone()
        },
        revision: Revision {
            git_commit: git_output(repo_root, &["rev-parse", "HEAD"]).unwrap_or_default(),
            git_dirty: !git_output(repo_root, &["status", "--porcelain"])
                .unwrap_or_default()
                .is_empty(),
            shape_binary: shape_binary.to_string_lossy().to_string(),
            shape_binary_sha256: env::file_sha256(&shape_binary)?,
            manifest_sha256: env::file_sha256(&repo_root.join(MANIFEST_PATH))?,
        },
        environment_identity: environment.identity(),
        pinned_environment_identity: manifest.environment.identity.clone(),
        environment,
        integrity: IntegrityState {
            status: if violations.is_empty() { "ok" } else { "violated" }.to_string(),
            covered_files: covered.len(),
            violations,
        },
        reference: reference_state,
        comparison,
        load_average_1m: (load_at_start, env::load_average_1m()),
        startup_floor_ms: startup_floor,
        workloads: results,
    };

    if let Some(out) = &opts.out {
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        std::fs::write(out, serde_json::to_string_pretty(&report)? + "\n")
            .with_context(|| format!("writing report to {}", out.display()))?;
    }
    Ok(report)
}

pub fn primary_statistics(report: &Report) -> Vec<(String, String, f64)> {
    let statistic = &report.measurement.primary_statistic;
    let mut out = Vec::new();
    for workload in &report.workloads {
        out.push((
            workload.id.clone(),
            "shape".to_string(),
            primary(&workload.shape.timing, statistic),
        ));
        if let Some(reference) = &workload.reference {
            out.push((
                workload.id.clone(),
                "reference".to_string(),
                primary(&reference.timing, statistic),
            ));
        }
    }
    out
}

pub fn print_summary(report: &Report) {
    println!();
    println!("Shape performance charter suite — {}", report.generated_utc);
    println!(
        "  revision            {}{}",
        report.revision.git_commit,
        if report.revision.git_dirty {
            " (working tree dirty)"
        } else {
            ""
        }
    );
    println!("  shape binary        {}", report.revision.shape_binary_sha256);
    println!("  environment         {}", report.environment_identity);
    println!("  pinned environment  {}", report.pinned_environment_identity);
    println!(
        "  integrity           {} ({} covered files)",
        report.integrity.status, report.integrity.covered_files
    );
    match &report.comparison {
        ComparisonState::Rendered => println!("  comparison          rendered"),
        ComparisonState::Refused {
            reason,
            differing_fields,
        } => {
            println!("  comparison          REFUSED — {reason}");
            for diff in differing_fields {
                println!(
                    "      {}: pinned {:?} / captured {:?}",
                    diff.field, diff.pinned, diff.captured
                );
            }
        }
    }
    println!();
    println!(
        "  {:<22} {:<12} {:>10} {:>10} {:>9} {:>9}  {}",
        "workload", "category", "shape ms", "ref ms", "ratio", "adj", "bar"
    );
    for workload in &report.workloads {
        let statistic = &report.measurement.primary_statistic;
        let shape_ms = primary(&workload.shape.timing, statistic);
        let reference_ms = workload
            .reference
            .as_ref()
            .map(|r| primary(&r.timing, statistic));
        let bar = match &workload.bar {
            report::BarStatus::Meets { bar, .. } => format!("meets ≥{bar:.2}x"),
            report::BarStatus::Below { bar, .. } => format!("BELOW ≥{bar:.2}x"),
            report::BarStatus::MeasuredNotGating { .. } => "measured (not gating)".to_string(),
            report::BarStatus::NotEvaluated { .. } => "not evaluated".to_string(),
        };
        println!(
            "  {:<22} {:<12} {:>10.1} {:>10} {:>9} {:>9}  {}",
            workload.id,
            workload.category,
            shape_ms,
            reference_ms
                .map(|ms| format!("{ms:.1}"))
                .unwrap_or_else(|| "-".to_string()),
            workload
                .ratio_process
                .map(|r| format!("{r:.3}x"))
                .unwrap_or_else(|| "-".to_string()),
            workload
                .ratio_startup_adjusted
                .map(|r| format!("{r:.3}x"))
                .unwrap_or_else(|| "-".to_string()),
            bar
        );
    }
    let fallback_count: usize = report
        .workloads
        .iter()
        .filter(|w| !w.shape.jit_fallbacks.is_empty())
        .count();
    println!();
    println!(
        "  {fallback_count} of {} workloads emitted at least one [jit-fallback] line. \
         Absence of a line is not a nativity claim (see NATIVE-WITNESS #117).",
        report.workloads.len()
    );
    if let ComparisonState::Rendered = report.comparison {
        println!(
            "  Ratios are reference ÷ shape on the {} of {} samples; ≥ 1.0 means Shape is \
             faster. `adj` subtracts each runtime's own measured startup floor.",
            report.measurement.primary_statistic, report.measurement.iterations
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_of(pairs: &[(&str, &str)]) -> env::Environment {
        env::Environment {
            fields: pairs
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect(),
        }
    }

    #[test]
    fn a_modified_benchmark_refuses_the_comparison_even_on_the_pinned_machine() {
        let pinned = env_of(&[("node_version", "v24.14.1")]);
        let violations = vec![integrity::Violation::Modified {
            path: "benchmarks/charter/shape/numeric_matmul.shape".to_string(),
            recorded: "a".repeat(64),
            actual: "b".repeat(64),
        }];
        let state = decide_comparison_with_integrity(&pinned, &pinned, &violations);
        match state {
            ComparisonState::Refused { reason, .. } => assert!(reason.contains("integrity")),
            ComparisonState::Rendered => panic!("a modified corpus must refuse the comparison"),
        }
    }

    #[test]
    fn an_intact_corpus_on_the_pinned_machine_renders() {
        let pinned = env_of(&[("node_version", "v24.14.1")]);
        assert!(decide_comparison_with_integrity(&pinned, &pinned, &[]).rendered());
    }

    #[test]
    fn the_committed_manifest_parses_and_is_self_consistent() {
        let manifest = Manifest::load(&repo_root()).expect("the committed manifest must parse");
        assert_eq!(manifest.schema, "shape.perf-suite.manifest/v1");
        assert!(
            manifest.startup_workload().is_some(),
            "a startup workload is required: it supplies the startup floor other workloads are \
             adjusted by"
        );
        assert!(manifest.measurement.noise_bound_pct > 0.0);
        assert!(manifest.measurement.iterations >= 3);
    }

    #[test]
    fn the_pinned_environment_identity_matches_its_recorded_fields() {
        let manifest = Manifest::load(&repo_root()).unwrap();
        assert_eq!(
            manifest.environment.environment.identity(),
            manifest.environment.identity,
            "the manifest's pinned identity hash disagrees with the fields it pins; \
             re-record with `perf-suite record-environment`"
        );
    }

    #[test]
    fn every_charter_category_is_represented_by_at_least_one_workload() {
        let manifest = Manifest::load(&repo_root()).unwrap();
        for category in manifest.categories.keys() {
            assert!(
                manifest.workloads.iter().any(|w| &w.category == category),
                "category {category} has no workload"
            );
        }
    }

    #[test]
    fn categories_blocked_on_another_lane_declare_the_precondition() {
        let manifest = Manifest::load(&repo_root()).unwrap();
        for (name, spec) in &manifest.categories {
            if !spec.gating {
                assert!(
                    spec.precondition.is_some(),
                    "category {name} is non-gating without naming why"
                );
            }
        }
    }
}
