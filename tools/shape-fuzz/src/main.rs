//! `shape-fuzz` CLI — differential-fuzz harness driver per W13 audit.
//!
//! W13.2 (this scaffold) exposes a single `run` subcommand that walks a
//! corpus directory of `.shape` files, executes each under `--mode vm` and
//! `--mode jit`, and prints a per-seed classification. There is no
//! mutation engine, no minimizer engine, no CI integration — those land
//! in W13.3 and W13.4 respectively.
//!
//! Exit codes:
//! - 0 if every seed classifies as `Convergent`.
//! - 1 if at least one seed produced a HIGH/MEDIUM signal divergence and
//!   the `--allow-low-signal` flag was not passed.
//! - 2 on harness driver failure (binary missing, snippet unreadable, ...).

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use shape_fuzz::divergence::Signal;
use shape_fuzz::{CompareConfig, DEFAULT_TIMEOUT, classify_divergence, compare_outputs, record_finding};

#[derive(Parser, Debug)]
#[command(
    name = "shape-fuzz",
    about = "Differential-fuzz harness for the Shape language (W13)",
    long_about = "Subprocess-level differential execution of .shape programs comparing \
                  `shape run --mode vm` against `shape run --mode jit` per the W13 audit \
                  (docs/cluster-audits/v0.3-w13-differential-fuzz-audit.md)."
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Walk a corpus directory and classify each `.shape` seed.
    Run {
        /// Directory containing `.shape` seeds (recursed).
        #[arg(long, value_name = "DIR")]
        corpus: PathBuf,

        /// Path to the `shape` binary. Defaults to `target/release/shape`
        /// relative to the workspace root.
        #[arg(long, value_name = "PATH", default_value = "target/release/shape")]
        shape_bin: PathBuf,

        /// Per-mode wall-clock budget in seconds.
        #[arg(long, value_name = "SECS", default_value_t = DEFAULT_TIMEOUT.as_secs())]
        timeout_secs: u64,

        /// Directory to write findings into when a divergence fires.
        /// Defaults to `tools/shape-fuzz/findings/` relative to cwd.
        #[arg(long, value_name = "DIR", default_value = "tools/shape-fuzz/findings")]
        findings_dir: PathBuf,

        /// Reserved for future mutation seeding. Phase W13.2 ignores the
        /// value (no mutation engine yet); accepting the flag now keeps
        /// the W13.3 CLI surface stable.
        #[arg(long, value_name = "U64")]
        seed: Option<u64>,

        /// Allow LOW-signal classifications (dual runtime error) without
        /// failing the harness exit code. HIGH and MEDIUM always fail.
        #[arg(long, default_value_t = false)]
        allow_low_signal: bool,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Run {
            corpus,
            shape_bin,
            timeout_secs,
            findings_dir,
            seed: _,
            allow_low_signal,
        } => run_corpus(
            &corpus,
            shape_bin,
            timeout_secs,
            findings_dir,
            allow_low_signal,
        ),
    }
}

fn run_corpus(
    corpus: &std::path::Path,
    shape_bin: PathBuf,
    timeout_secs: u64,
    findings_dir: PathBuf,
    allow_low_signal: bool,
) -> ExitCode {
    let seeds = match collect_seeds(corpus) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("shape-fuzz: failed to enumerate corpus {}: {}", corpus.display(), e);
            return ExitCode::from(2);
        }
    };

    if seeds.is_empty() {
        eprintln!(
            "shape-fuzz: no .shape seeds found under {} (W13.3 lands the hand-seeded corpus)",
            corpus.display()
        );
        return ExitCode::from(2);
    }

    let cfg = CompareConfig {
        shape_binary: shape_bin,
        timeout: std::time::Duration::from_secs(timeout_secs),
    };

    let mut hard_fail = false;
    for seed in &seeds {
        let cmp = match compare_outputs(seed, &cfg) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("shape-fuzz: driver error on {}: {}", seed.display(), e);
                return ExitCode::from(2);
            }
        };

        let div = classify_divergence(&cmp.vm, &cmp.jit);
        println!(
            "{} :: {} (vm={:?} ec={:?} timed_out={} | jit={:?} ec={:?} timed_out={})",
            seed.display(),
            div,
            cmp.vm.stdout_tail,
            cmp.vm.exit_code,
            cmp.vm.timed_out,
            cmp.jit.stdout_tail,
            cmp.jit.exit_code,
            cmp.jit.timed_out,
        );

        match div.signal() {
            Signal::Convergent | Signal::Noise => {}
            Signal::Low => {
                if !allow_low_signal {
                    hard_fail = true;
                }
                if let Err(e) = record_finding(&cmp, &div, &findings_dir) {
                    eprintln!("shape-fuzz: findings write failed: {}", e);
                }
            }
            Signal::Medium | Signal::High => {
                hard_fail = true;
                if let Err(e) = record_finding(&cmp, &div, &findings_dir) {
                    eprintln!("shape-fuzz: findings write failed: {}", e);
                }
            }
        }
    }

    if hard_fail {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

fn collect_seeds(root: &std::path::Path) -> std::io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    visit(root, &mut out)?;
    out.sort();
    Ok(out)
}

fn visit(dir: &std::path::Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    let md = std::fs::metadata(dir)?;
    if md.is_file() {
        if dir.extension().and_then(|s| s.to_str()) == Some("shape") {
            out.push(dir.to_path_buf());
        }
        return Ok(());
    }
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let ft = entry.file_type()?;
        if ft.is_dir() {
            visit(&path, out)?;
        } else if ft.is_file() && path.extension().and_then(|s| s.to_str()) == Some("shape") {
            out.push(path);
        }
    }
    Ok(())
}
