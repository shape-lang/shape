//! Divergence classification per W13 audit §2.
//!
//! The harness compares `(vm_out, vm_ec)` against `(jit_out, jit_ec)` from
//! independent subprocess runs of `shape run --mode {vm,jit} <file>` and
//! classifies the result into one of the eight rows from §2's table.
//!
//! Per §2.1 the `[jit-fallback]` info-level stderr emission from
//! `crates/shape-jit/src/executor.rs:151` is NOT a divergence — fall-through
//! produces identical stdout to VM by construction. The harness pipes stderr
//! to `/dev/null` so the diagnostic is dropped from the compare and the case
//! lands in `Convergent` like any other matching pair.

use std::fmt;

/// Outcome of one mode's run captured by `compare_outputs`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModeOutcome {
    /// Last line of stdout, mirroring the corrected smoke harness shape
    /// (`... | tail -1`) per W12 close §3.1.
    pub stdout_tail: String,
    /// Process exit code; `None` indicates timeout (SIGKILL after the
    /// harness wall-clock budget elapsed).
    pub exit_code: Option<i32>,
    /// `true` if the run timed out at the harness budget.
    pub timed_out: bool,
}

impl ModeOutcome {
    pub fn new(stdout_tail: String, exit_code: i32) -> Self {
        Self {
            stdout_tail,
            exit_code: Some(exit_code),
            timed_out: false,
        }
    }

    pub fn timeout() -> Self {
        Self {
            stdout_tail: String::new(),
            exit_code: None,
            timed_out: true,
        }
    }

    /// `true` iff the mode exited successfully (`ec == 0`) without timing out.
    pub fn is_clean_success(&self) -> bool {
        !self.timed_out && self.exit_code == Some(0)
    }
}

/// Signal strength of a divergence classification, used by the harness
/// exit-code policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    /// Convergent run (VM == JIT) OR fall-through (per §2.1).
    Convergent,
    /// HIGH-SIGNAL divergence; harness MUST surface and exit non-zero.
    High,
    /// MEDIUM-SIGNAL divergence; harness records but distinguishes from HIGH.
    Medium,
    /// LOW-SIGNAL divergence; optional record.
    Low,
    /// NOISE class (e.g. both modes timeout); harness drops.
    Noise,
}

/// Eight-class divergence taxonomy per W13 audit §2 table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Divergence {
    /// VM == JIT on both stdout-tail AND exit code. NOT a divergence; this
    /// also subsumes the §2.1 `[jit-fallback]` fall-through case because
    /// stderr is dropped from the compare.
    Convergent,
    /// Row 1: `out_vm != out_jit` AND `ec_vm == ec_jit == 0`.
    StdoutTailDivergence,
    /// Row 2: `ec_vm != ec_jit` (e.g. `0` vs `1`).
    ExitCodeDivergence,
    /// Row 3: one mode `ec=1` with Error; the other `ec=0` with clean output.
    RuntimeErrorAsymmetry,
    /// Row 4: both `ec=1`, stderr Error messages differ (LOW-SIGNAL).
    DualRuntimeError,
    /// Row 5: both modes timeout at the harness budget (NOISE).
    DualTimeout,
    /// Row 6: one mode timed out; the other completed.
    SingleTimeout,
    /// Row 7: same program produced different outputs across a tier boundary.
    /// Only emitted by future tier-sweep helpers; the scaffold compares one
    /// program at a time, so this variant is reserved for W13.3 / W13.4 wiring.
    TierBoundaryDivergence,
}

impl Divergence {
    pub fn signal(&self) -> Signal {
        match self {
            Self::Convergent => Signal::Convergent,
            Self::StdoutTailDivergence
            | Self::ExitCodeDivergence
            | Self::RuntimeErrorAsymmetry
            | Self::TierBoundaryDivergence => Signal::High,
            Self::SingleTimeout => Signal::Medium,
            Self::DualRuntimeError => Signal::Low,
            Self::DualTimeout => Signal::Noise,
        }
    }

    /// Stable short name suitable for findings filenames + AGENTS.md citations.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Convergent => "convergent",
            Self::StdoutTailDivergence => "stdout-tail-divergence",
            Self::ExitCodeDivergence => "exit-code-divergence",
            Self::RuntimeErrorAsymmetry => "runtime-error-asymmetry",
            Self::DualRuntimeError => "dual-runtime-error",
            Self::DualTimeout => "dual-timeout",
            Self::SingleTimeout => "single-timeout",
            Self::TierBoundaryDivergence => "tier-boundary-divergence",
        }
    }
}

impl fmt::Display for Divergence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({:?})", self.name(), self.signal())
    }
}

/// Apply the §2 table to a `(vm, jit)` outcome pair.
pub fn classify_divergence(vm: &ModeOutcome, jit: &ModeOutcome) -> Divergence {
    match (vm.timed_out, jit.timed_out) {
        (true, true) => return Divergence::DualTimeout,
        (true, false) | (false, true) => return Divergence::SingleTimeout,
        (false, false) => {}
    }

    match (vm.exit_code, jit.exit_code) {
        (Some(0), Some(0)) => {
            if vm.stdout_tail == jit.stdout_tail {
                Divergence::Convergent
            } else {
                Divergence::StdoutTailDivergence
            }
        }
        (Some(ev), Some(ej)) if ev != 0 && ej != 0 => Divergence::DualRuntimeError,
        (Some(ev), Some(ej)) if ev != ej => {
            // One side ec=0 and the other side ec!=0 is the asymmetric
            // crash case the audit calls out separately from a pure
            // numeric ec difference.
            if (ev == 0) ^ (ej == 0) {
                Divergence::RuntimeErrorAsymmetry
            } else {
                Divergence::ExitCodeDivergence
            }
        }
        // Both timed_out branches handled above; remaining patterns shouldn't
        // be reachable but fall back to ExitCodeDivergence as a safe default
        // rather than panicking on a future variant.
        _ => Divergence::ExitCodeDivergence,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(s: &str) -> ModeOutcome {
        ModeOutcome::new(s.to_string(), 0)
    }
    fn err(s: &str, code: i32) -> ModeOutcome {
        ModeOutcome::new(s.to_string(), code)
    }

    // §2 row 0 — equal outputs + ec=0 in both modes.
    #[test]
    fn convergent_matches_when_outputs_and_exit_codes_equal() {
        let vm = ok("4950");
        let jit = ok("4950");
        let d = classify_divergence(&vm, &jit);
        assert_eq!(d, Divergence::Convergent);
        assert_eq!(d.signal(), Signal::Convergent);
    }

    // §2 row 1 — stdout-tail divergence with both ec=0.
    #[test]
    fn stdout_tail_divergence_when_outputs_differ_under_equal_ec() {
        let vm = ok("-62500");
        let jit = ok("9223372036854775558");
        let d = classify_divergence(&vm, &jit);
        assert_eq!(d, Divergence::StdoutTailDivergence);
        assert_eq!(d.signal(), Signal::High);
    }

    // §2 row 4 — both ec non-zero (parser/type-check rejected). The §2
    // table calls this DualRuntimeError (LOW-SIGNAL) regardless of whether
    // the codes are equal, because the asymmetric-crash case is captured
    // separately by row 3 (RuntimeErrorAsymmetry).
    #[test]
    fn dual_runtime_error_covers_both_nonzero_even_with_unequal_codes() {
        let vm = err("e", 2);
        let jit = err("e", 3);
        let d = classify_divergence(&vm, &jit);
        assert_eq!(d, Divergence::DualRuntimeError);
        assert_eq!(d.signal(), Signal::Low);
    }

    // §2 row 3 — asymmetric crash: one mode clean, the other ec=1.
    #[test]
    fn runtime_error_asymmetry_when_one_mode_crashes_clean_other_succeeds() {
        let vm = ok("ok");
        let jit = err("Error: ...", 1);
        let d = classify_divergence(&vm, &jit);
        assert_eq!(d, Divergence::RuntimeErrorAsymmetry);
        assert_eq!(d.signal(), Signal::High);
    }

    // §2 row 4 — both crashed (parser-rejects-both); LOW-SIGNAL.
    #[test]
    fn dual_runtime_error_when_both_modes_crash() {
        let vm = err("Error: parse", 1);
        let jit = err("Error: parse", 1);
        let d = classify_divergence(&vm, &jit);
        assert_eq!(d, Divergence::DualRuntimeError);
        assert_eq!(d.signal(), Signal::Low);
    }

    // §2 row 5 — both timed out; harness drops as NOISE.
    #[test]
    fn dual_timeout_when_both_modes_time_out() {
        let vm = ModeOutcome::timeout();
        let jit = ModeOutcome::timeout();
        let d = classify_divergence(&vm, &jit);
        assert_eq!(d, Divergence::DualTimeout);
        assert_eq!(d.signal(), Signal::Noise);
    }

    // §2 row 6 — one mode timed out, the other completed.
    #[test]
    fn single_timeout_when_only_one_mode_times_out() {
        let vm = ModeOutcome::timeout();
        let jit = ok("done");
        let d = classify_divergence(&vm, &jit);
        assert_eq!(d, Divergence::SingleTimeout);
        assert_eq!(d.signal(), Signal::Medium);
    }

    // §2.1 — `[jit-fallback]` info is dropped via 2>/dev/null; stdout matches
    // VM by construction, so the harness classifies as Convergent.
    #[test]
    fn jit_fallback_is_not_a_divergence_when_stdout_tails_match() {
        // Both modes ran clean; the only difference would be stderr lines,
        // which the harness pipes to /dev/null and never feeds into the
        // ModeOutcome. The classifier sees matching `(stdout_tail, ec)`.
        let vm = ok("100");
        let jit = ok("100");
        assert_eq!(classify_divergence(&vm, &jit), Divergence::Convergent);
    }

    #[test]
    fn divergence_names_are_stable_for_filenames() {
        for d in [
            Divergence::Convergent,
            Divergence::StdoutTailDivergence,
            Divergence::ExitCodeDivergence,
            Divergence::RuntimeErrorAsymmetry,
            Divergence::DualRuntimeError,
            Divergence::DualTimeout,
            Divergence::SingleTimeout,
            Divergence::TierBoundaryDivergence,
        ] {
            let n = d.name();
            assert!(!n.is_empty());
            assert!(n.chars().all(|c| c.is_ascii_lowercase() || c == '-'));
        }
    }
}
