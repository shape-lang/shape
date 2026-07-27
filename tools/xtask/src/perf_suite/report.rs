//! Report model for the performance charter suite (ADR-018 §1).
//!
//! Everything a charter number needs in order to be evidence lives in one JSON
//! document: the revision it was taken at, the hash of the binary that was
//! actually executed, the environment identity, the integrity state of the
//! benchmark corpus, and every raw sample. The decision logic (which bar
//! applies, whether a comparison may be rendered at all) is pure so it can be
//! tested without running a benchmark.

use serde::{Deserialize, Serialize};

pub const REPORT_SCHEMA: &str = "shape.perf-suite.report/v1";

/// Charter bar outcome for one workload.
///
/// The distinction that matters: a category whose prerequisite lane has not
/// landed is *measured*, never *failed*. The suite reports the number and names
/// the precondition instead of claiming a gate status the charter does not yet
/// assert.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum BarStatus {
    Meets { ratio: f64, bar: f64 },
    Below { ratio: f64, bar: f64 },
    MeasuredNotGating { ratio: Option<f64>, reason: String },
    NotEvaluated { reason: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategorySpec {
    /// Ratio of reference time to Shape time; ≥ 1.0 means Shape is faster.
    pub bar_ratio: Option<f64>,
    /// Whether a pass/fail claim is admissible today.
    pub gating: bool,
    /// Why it is not admissible, when `gating` is false.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub precondition: Option<String>,
}

/// Inputs to the bar decision, kept separate from the measurement plumbing.
pub struct BarInputs<'a> {
    pub category: &'a CategorySpec,
    pub ratio: Option<f64>,
    pub comparison_rendered: bool,
    pub reference_available: bool,
    pub outputs_agree: bool,
    pub shape_ok: bool,
}

pub fn evaluate_bar(input: &BarInputs<'_>) -> BarStatus {
    if !input.shape_ok {
        return BarStatus::NotEvaluated {
            reason: "the Shape workload did not run to completion".to_string(),
        };
    }
    if !input.reference_available {
        return BarStatus::NotEvaluated {
            reason: "the pinned reference runtime is unavailable".to_string(),
        };
    }
    if !input.comparison_rendered {
        return BarStatus::NotEvaluated {
            reason: "comparison refused: the captured environment identity differs from the \
                     identity pinned in the suite manifest"
                .to_string(),
        };
    }
    if !input.outputs_agree {
        return BarStatus::NotEvaluated {
            reason: "Shape and the reference produced different results; a workload that \
                     computes a different answer is not a comparison"
                .to_string(),
        };
    }
    let Some(ratio) = input.ratio else {
        return BarStatus::NotEvaluated {
            reason: "no ratio was measured".to_string(),
        };
    };
    if !input.category.gating {
        return BarStatus::MeasuredNotGating {
            ratio: Some(ratio),
            reason: input.category.precondition.clone().unwrap_or_else(|| {
                "this category carries no charter bar; the measurement is informational"
                    .to_string()
            }),
        };
    }
    match input.category.bar_ratio {
        None => BarStatus::MeasuredNotGating {
            ratio: Some(ratio),
            reason: "this category carries no charter bar; the measurement is informational"
                .to_string(),
        },
        Some(bar) if ratio >= bar => BarStatus::Meets { ratio, bar },
        Some(bar) => BarStatus::Below { ratio, bar },
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ComparisonState {
    Rendered,
    Refused {
        reason: String,
        differing_fields: Vec<super::env::FieldDiff>,
    },
}

impl ComparisonState {
    pub fn rendered(&self) -> bool {
        matches!(self, ComparisonState::Rendered)
    }
}

/// A comparison may be rendered only against the pinned environment identity.
pub fn decide_comparison(
    captured: &super::env::Environment,
    pinned: &super::env::Environment,
) -> ComparisonState {
    let diffs = captured.diff(pinned);
    if diffs.is_empty() {
        ComparisonState::Rendered
    } else {
        ComparisonState::Refused {
            reason: format!(
                "{} environment identity field(s) differ from the pinned manifest identity; \
                 measurements taken here are not comparable to the charter baseline",
                diffs.len()
            ),
            differing_fields: diffs,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Timing {
    pub samples_ms: Vec<f64>,
    pub min_ms: f64,
    pub median_ms: f64,
    pub max_ms: f64,
}

impl Timing {
    pub fn from_samples(mut samples: Vec<f64>) -> Self {
        let raw = samples.clone();
        samples.sort_by(|a, b| a.partial_cmp(b).expect("timings are never NaN"));
        let median = if samples.is_empty() {
            0.0
        } else if samples.len() % 2 == 1 {
            samples[samples.len() / 2]
        } else {
            (samples[samples.len() / 2 - 1] + samples[samples.len() / 2]) / 2.0
        };
        Timing {
            min_ms: samples.first().copied().unwrap_or(0.0),
            max_ms: samples.last().copied().unwrap_or(0.0),
            median_ms: median,
            samples_ms: raw,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunOutcome {
    pub timing: Timing,
    pub exit_ok: bool,
    pub stdout_canonical: String,
    /// `[jit-fallback]` lines observed on stderr, deduplicated and truncated.
    ///
    /// Recorded as observation, not as a nativity claim: the absence of a
    /// fallback line is NOT proof that a workload executed natively. Proving
    /// nativity is R15's `NativeExecutionWitness`, which is NATIVE-WITNESS
    /// (#117), not this suite.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub jit_fallbacks: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkloadResult {
    pub id: String,
    pub category: String,
    pub shape: RunOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<RunOutcome>,
    pub outputs_agree: bool,
    /// Reference wall time ÷ Shape wall time, both whole-process. ≥ 1.0 means
    /// Shape is faster. This is the primary, least-adjusted number.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ratio_process: Option<f64>,
    /// The same ratio after subtracting each runtime's own measured
    /// hello-world floor. Disclosed separately because it is an adjustment;
    /// it is the harsher number whenever Shape's floor is the larger one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ratio_startup_adjusted: Option<f64>,
    /// Set when the startup-adjusted reference time is small enough that the
    /// adjusted ratio should not be read precisely.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adjusted_ratio_confidence: Option<String>,
    pub bar: BarStatus,
}

/// Reference time ÷ Shape time on the primary statistic (minimum of samples;
/// timing noise is one-sided, so the minimum is the most robust estimator).
pub fn ratio(reference_ms: f64, shape_ms: f64) -> Option<f64> {
    (shape_ms > 0.0 && reference_ms > 0.0).then_some(reference_ms / shape_ms)
}

/// The same ratio with each runtime's own startup floor removed. Returns
/// `None` when either side's kernel time does not survive the subtraction.
pub fn startup_adjusted_ratio(
    reference_ms: f64,
    reference_floor_ms: f64,
    shape_ms: f64,
    shape_floor_ms: f64,
) -> Option<f64> {
    let reference_kernel = reference_ms - reference_floor_ms;
    let shape_kernel = shape_ms - shape_floor_ms;
    (reference_kernel > 0.0 && shape_kernel > 0.0).then_some(reference_kernel / shape_kernel)
}

/// Canonicalise program output so that `3` and `3.0` compare equal while a
/// genuinely different result still shows up as a mismatch. Numeric tokens are
/// re-rendered at 9 significant digits; everything else is compared verbatim.
pub fn canonicalize_output(text: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    for line in text.trim().lines() {
        let tokens: Vec<String> = line
            .split_whitespace()
            .map(|token| match token.parse::<f64>() {
                Ok(v) if v.is_finite() => format!("{v:.9e}"),
                _ => token.to_string(),
            })
            .collect();
        lines.push(tokens.join(" "));
    }
    lines.join("\n")
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NoiseViolation {
    pub workload: String,
    pub runtime: String,
    pub first_ms: f64,
    pub second_ms: f64,
    pub deviation_pct: f64,
    pub bound_pct: f64,
}

/// What a failed noise check means.
///
/// The reference runtime is a fixed quantity: its own run-to-run deviation is a
/// direct measurement of how quiet the machine was. When Shape and the
/// reference move together, the machine moved — reporting that as Shape
/// instability would be the wrong finding, and quietly widening the bound until
/// it passes would be worse.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoiseVerdict {
    /// Every measurement agreed within the bound.
    Reproducible,
    /// Shape moved on at least one workload where the reference held steady.
    ShapeUnstable,
    /// Every exceedance is shadowed by the reference moving too.
    MachineContended,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NoiseOutcome {
    pub verdict: NoiseVerdict,
    pub violations: Vec<NoiseViolation>,
    pub bound_pct: f64,
}

/// Tripwire 1 with its verdict attached.
///
/// Attributing an exceedance to Shape requires the control to have held for the
/// *entire* run, not merely on the same workload: interference lands wherever
/// it lands, so a burst that hits Shape on one workload and the reference on
/// another is unattributable, not a Shape finding. Both non-reproducible
/// verdicts fail the check — the distinction is what the failure means, not
/// whether it counts.
pub fn classify_noise(
    first: &[(String, String, f64)],
    second: &[(String, String, f64)],
    bound_pct: f64,
) -> NoiseOutcome {
    let violations = check_noise(first, second, bound_pct);
    let shape_moved = violations.iter().any(|v| v.runtime == "shape");
    let control_moved = violations.iter().any(|v| v.runtime == "reference");

    let verdict = match (violations.is_empty(), shape_moved, control_moved) {
        (true, _, _) => NoiseVerdict::Reproducible,
        (false, false, _) => NoiseVerdict::MachineContended,
        (false, true, true) => NoiseVerdict::MachineContended,
        (false, true, false) => NoiseVerdict::ShapeUnstable,
    };

    NoiseOutcome {
        verdict,
        violations,
        bound_pct,
    }
}

/// Relative deviation between two measurements of the same quantity, as a
/// percentage of the smaller one.
pub fn deviation_pct(a: f64, b: f64) -> f64 {
    let smaller = a.min(b);
    if smaller <= 0.0 {
        return f64::INFINITY;
    }
    (a - b).abs() / smaller * 100.0
}

/// Differences below this many milliseconds are not violations whatever their
/// percentage. Scheduler and timer granularity live on this scale: calling a
/// 3ms wobble on a 35ms measurement a 9% regression would be reading noise as
/// signal, and would make the tripwire fire constantly on the fastest
/// workloads while saying nothing about the slowest.
pub const NOISE_ABSOLUTE_FLOOR_MS: f64 = 10.0;

/// Tripwire 1: two consecutive runs at the same revision must agree within the
/// manifest's declared noise bound. Compares the primary statistic per
/// (workload, runtime) pair; a workload present in only one run is itself a
/// violation, reported with an infinite deviation.
pub fn check_noise(first: &[(String, String, f64)], second: &[(String, String, f64)], bound_pct: f64) -> Vec<NoiseViolation> {
    let mut violations = Vec::new();
    for (workload, runtime, first_ms) in first {
        let matched = second
            .iter()
            .find(|(w, r, _)| w == workload && r == runtime)
            .map(|(_, _, ms)| *ms);
        match matched {
            Some(second_ms) => {
                let dev = deviation_pct(*first_ms, second_ms);
                if dev > bound_pct && (first_ms - second_ms).abs() >= NOISE_ABSOLUTE_FLOOR_MS {
                    violations.push(NoiseViolation {
                        workload: workload.clone(),
                        runtime: runtime.clone(),
                        first_ms: *first_ms,
                        second_ms,
                        deviation_pct: dev,
                        bound_pct,
                    });
                }
            }
            None => violations.push(NoiseViolation {
                workload: workload.clone(),
                runtime: runtime.clone(),
                first_ms: *first_ms,
                second_ms: f64::NAN,
                deviation_pct: f64::INFINITY,
                bound_pct,
            }),
        }
    }
    violations
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gating(bar: f64) -> CategorySpec {
        CategorySpec {
            bar_ratio: Some(bar),
            gating: true,
            precondition: None,
        }
    }

    fn blocked(bar: f64, why: &str) -> CategorySpec {
        CategorySpec {
            bar_ratio: Some(bar),
            gating: false,
            precondition: Some(why.to_string()),
        }
    }

    fn inputs<'a>(category: &'a CategorySpec, ratio: f64) -> BarInputs<'a> {
        BarInputs {
            category,
            ratio: Some(ratio),
            comparison_rendered: true,
            reference_available: true,
            outputs_agree: true,
            shape_ok: true,
        }
    }

    #[test]
    fn a_gating_category_above_its_bar_meets_it() {
        let cat = gating(1.5);
        assert_eq!(
            evaluate_bar(&inputs(&cat, 1.6)),
            BarStatus::Meets {
                ratio: 1.6,
                bar: 1.5
            }
        );
    }

    #[test]
    fn a_gating_category_below_its_bar_is_below() {
        let cat = gating(1.5);
        assert_eq!(
            evaluate_bar(&inputs(&cat, 0.02)),
            BarStatus::Below {
                ratio: 0.02,
                bar: 1.5
            }
        );
    }

    #[test]
    fn exactly_on_the_bar_meets_it() {
        let cat = gating(1.0);
        assert!(matches!(
            evaluate_bar(&inputs(&cat, 1.0)),
            BarStatus::Meets { .. }
        ));
    }

    /// The rule the ticket is explicit about: a category whose prerequisite
    /// lane has not landed never produces a pass/fail claim, in either
    /// direction, however good the number looks.
    #[test]
    fn a_non_gating_category_never_claims_pass_or_fail() {
        let cat = blocked(1.0, "PERF-CLOSURE-NATIVE has not landed");
        for ratio in [0.01, 1.0, 99.0] {
            match evaluate_bar(&inputs(&cat, ratio)) {
                BarStatus::MeasuredNotGating { ratio: r, reason } => {
                    assert_eq!(r, Some(ratio));
                    assert!(reason.contains("PERF-CLOSURE-NATIVE"));
                }
                other => panic!("expected a measured-not-gating status, got {other:?}"),
            }
        }
    }

    #[test]
    fn a_category_without_a_bar_is_informational() {
        let cat = CategorySpec {
            bar_ratio: None,
            gating: true,
            precondition: None,
        };
        assert!(matches!(
            evaluate_bar(&inputs(&cat, 3.0)),
            BarStatus::MeasuredNotGating { .. }
        ));
    }

    #[test]
    fn a_refused_comparison_blocks_bar_evaluation() {
        let cat = gating(1.5);
        let mut input = inputs(&cat, 9.0);
        input.comparison_rendered = false;
        assert!(matches!(
            evaluate_bar(&input),
            BarStatus::NotEvaluated { .. }
        ));
    }

    #[test]
    fn a_missing_reference_blocks_bar_evaluation() {
        let cat = gating(1.5);
        let mut input = inputs(&cat, 9.0);
        input.reference_available = false;
        assert!(matches!(
            evaluate_bar(&input),
            BarStatus::NotEvaluated { .. }
        ));
    }

    #[test]
    fn disagreeing_outputs_block_bar_evaluation() {
        let cat = gating(1.5);
        let mut input = inputs(&cat, 9.0);
        input.outputs_agree = false;
        assert!(matches!(
            evaluate_bar(&input),
            BarStatus::NotEvaluated { .. }
        ));
    }

    #[test]
    fn a_failed_shape_run_blocks_bar_evaluation() {
        let cat = gating(1.5);
        let mut input = inputs(&cat, 9.0);
        input.shape_ok = false;
        assert!(matches!(
            evaluate_bar(&input),
            BarStatus::NotEvaluated { .. }
        ));
    }

    fn env(pairs: &[(&str, &str)]) -> super::super::env::Environment {
        super::super::env::Environment {
            fields: pairs
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect(),
        }
    }

    /// Tripwire 2.
    #[test]
    fn comparison_is_refused_when_environment_identities_differ() {
        let pinned = env(&[("node_version", "v24.14.1")]);
        let captured = env(&[("node_version", "v22.0.0")]);
        match decide_comparison(&captured, &pinned) {
            ComparisonState::Refused {
                differing_fields, ..
            } => {
                assert_eq!(differing_fields.len(), 1);
                assert_eq!(differing_fields[0].field, "node_version");
            }
            ComparisonState::Rendered => panic!("expected the comparison to be refused"),
        }
    }

    #[test]
    fn comparison_is_rendered_on_the_pinned_environment() {
        let pinned = env(&[("node_version", "v24.14.1")]);
        assert!(decide_comparison(&pinned, &pinned).rendered());
    }

    #[test]
    fn timing_statistics_use_the_sorted_samples() {
        let t = Timing::from_samples(vec![30.0, 10.0, 20.0]);
        assert_eq!(t.min_ms, 10.0);
        assert_eq!(t.median_ms, 20.0);
        assert_eq!(t.max_ms, 30.0);
        assert_eq!(t.samples_ms, vec![30.0, 10.0, 20.0], "raw order is kept");
    }

    #[test]
    fn even_sample_counts_take_the_midpoint() {
        let t = Timing::from_samples(vec![10.0, 20.0, 30.0, 40.0]);
        assert_eq!(t.median_ms, 25.0);
    }

    #[test]
    fn ratio_is_reference_over_shape() {
        assert_eq!(ratio(200.0, 100.0), Some(2.0));
        assert_eq!(ratio(50.0, 100.0), Some(0.5));
        assert_eq!(ratio(100.0, 0.0), None);
    }

    #[test]
    fn startup_adjustment_removes_each_runtime_floor() {
        // Shape 4000ms with a 200ms floor vs reference 100ms with a 25ms floor:
        // the process ratio flatters Shape, the adjusted one does not.
        let process = ratio(100.0, 4000.0).unwrap();
        let adjusted = startup_adjusted_ratio(100.0, 25.0, 4000.0, 200.0).unwrap();
        assert!(adjusted < process);
        assert!((adjusted - (75.0 / 3800.0)).abs() < 1e-12);
    }

    #[test]
    fn startup_adjustment_declines_when_a_kernel_does_not_survive_it() {
        assert_eq!(startup_adjusted_ratio(20.0, 25.0, 4000.0, 200.0), None);
    }

    #[test]
    fn integer_and_float_spellings_of_one_number_compare_equal() {
        assert_eq!(canonicalize_output("3"), canonicalize_output("3.0"));
        assert_eq!(canonicalize_output(" 486165 \n"), canonicalize_output("486165.0"));
    }

    #[test]
    fn genuinely_different_results_do_not_compare_equal() {
        assert_ne!(canonicalize_output("486165"), canonicalize_output("486166"));
    }

    #[test]
    fn non_numeric_output_is_compared_verbatim() {
        assert_eq!(canonicalize_output("hello"), "hello");
        assert_ne!(canonicalize_output("hello"), canonicalize_output("world"));
    }

    /// Tripwire 1.
    #[test]
    fn runs_within_the_noise_bound_pass() {
        let a = vec![("k".to_string(), "shape".to_string(), 1000.0)];
        let b = vec![("k".to_string(), "shape".to_string(), 1050.0)];
        assert!(check_noise(&a, &b, 10.0).is_empty());
    }

    #[test]
    fn runs_outside_the_noise_bound_are_violations() {
        let a = vec![("k".to_string(), "shape".to_string(), 1000.0)];
        let b = vec![("k".to_string(), "shape".to_string(), 1300.0)];
        let violations = check_noise(&a, &b, 10.0);
        assert_eq!(violations.len(), 1);
        assert!((violations[0].deviation_pct - 30.0).abs() < 1e-9);
    }

    #[test]
    fn a_millisecond_wobble_on_a_fast_workload_is_not_a_violation() {
        // 30% by percentage, 3ms in absolute terms: below the floor.
        let a = vec![("k".to_string(), "reference".to_string(), 10.0)];
        let b = vec![("k".to_string(), "reference".to_string(), 13.0)];
        assert!(check_noise(&a, &b, 10.0).is_empty());
    }

    #[test]
    fn the_absolute_floor_does_not_excuse_a_real_move() {
        let a = vec![("k".to_string(), "reference".to_string(), 90.0)];
        let b = vec![("k".to_string(), "reference".to_string(), 124.0)];
        assert_eq!(check_noise(&a, &b, 15.0).len(), 1);
    }

    #[test]
    fn a_workload_missing_from_the_second_run_is_a_violation() {
        let a = vec![("k".to_string(), "shape".to_string(), 1000.0)];
        let violations = check_noise(&a, &[], 10.0);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].deviation_pct.is_infinite());
    }

    #[test]
    fn deviation_is_relative_to_the_smaller_measurement() {
        assert!((deviation_pct(100.0, 110.0) - 10.0).abs() < 1e-9);
        assert!((deviation_pct(110.0, 100.0) - 10.0).abs() < 1e-9);
    }

    fn pair(workload: &str, runtime: &str, ms: f64) -> (String, String, f64) {
        (workload.to_string(), runtime.to_string(), ms)
    }

    #[test]
    fn agreeing_runs_are_reproducible() {
        let a = vec![pair("k", "shape", 1000.0), pair("k", "reference", 50.0)];
        let b = vec![pair("k", "shape", 1020.0), pair("k", "reference", 51.0)];
        assert_eq!(
            classify_noise(&a, &b, 12.0).verdict,
            NoiseVerdict::Reproducible
        );
    }

    /// Shape moved while the control held: that is a finding about Shape.
    #[test]
    fn shape_moving_alone_is_shape_instability() {
        let a = vec![pair("k", "shape", 1000.0), pair("k", "reference", 50.0)];
        let b = vec![pair("k", "shape", 2000.0), pair("k", "reference", 51.0)];
        assert_eq!(
            classify_noise(&a, &b, 12.0).verdict,
            NoiseVerdict::ShapeUnstable
        );
    }

    /// Shape and the control moved together: the machine moved. The check
    /// still fails — it just does not blame the wrong thing.
    #[test]
    fn shape_and_reference_moving_together_is_machine_contention() {
        let a = vec![pair("k", "shape", 1000.0), pair("k", "reference", 50.0)];
        let b = vec![pair("k", "shape", 2000.0), pair("k", "reference", 100.0)];
        let outcome = classify_noise(&a, &b, 12.0);
        assert_eq!(outcome.verdict, NoiseVerdict::MachineContended);
        assert_eq!(outcome.violations.len(), 2);
    }

    #[test]
    fn one_stable_workload_does_not_excuse_another_moving_alone() {
        let a = vec![
            pair("quiet", "shape", 100.0),
            pair("quiet", "reference", 10.0),
            pair("loud", "shape", 1000.0),
            pair("loud", "reference", 50.0),
        ];
        let b = vec![
            pair("quiet", "shape", 101.0),
            pair("quiet", "reference", 10.1),
            pair("loud", "shape", 2000.0),
            pair("loud", "reference", 50.5),
        ];
        assert_eq!(
            classify_noise(&a, &b, 12.0).verdict,
            NoiseVerdict::ShapeUnstable
        );
    }

    /// A burst that lands on Shape in one workload and on the control in
    /// another is unattributable — the machine moved during the run.
    #[test]
    fn exceedances_on_both_sides_of_different_workloads_are_unattributable() {
        let a = vec![
            pair("a", "shape", 1000.0),
            pair("a", "reference", 100.0),
            pair("b", "shape", 1000.0),
            pair("b", "reference", 100.0),
        ];
        let b = vec![
            pair("a", "shape", 2000.0),
            pair("a", "reference", 101.0),
            pair("b", "shape", 1010.0),
            pair("b", "reference", 200.0),
        ];
        assert_eq!(
            classify_noise(&a, &b, 15.0).verdict,
            NoiseVerdict::MachineContended
        );
    }

    #[test]
    fn the_control_moving_alone_still_fails_the_check() {
        let a = vec![pair("k", "shape", 1000.0), pair("k", "reference", 50.0)];
        let b = vec![pair("k", "shape", 1010.0), pair("k", "reference", 100.0)];
        let outcome = classify_noise(&a, &b, 12.0);
        assert_eq!(outcome.verdict, NoiseVerdict::MachineContended);
        assert!(!outcome.violations.is_empty());
    }
}
