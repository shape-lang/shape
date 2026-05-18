//! Differential-fuzz harness for the Shape language per W13 audit
//! (`docs/cluster-audits/v0.3-w13-differential-fuzz-audit.md`).
//!
//! Runs the same `.shape` program under `shape run --mode vm` and
//! `shape run --mode jit` as independent subprocesses, captures
//! `(stdout_tail, exit_code)` per the corrected W12 smoke-harness shape,
//! and classifies the result against the eight-class taxonomy in
//! `divergence`.
//!
//! **W13.2 scope (this crate at scaffold time):**
//! - `compare_outputs` — subprocess driver.
//! - `classify_divergence` — pure §2 table.
//! - `record_finding` — write divergence record to a findings directory.
//! - `minimize_reproducer` — placeholder; full bisect lands in W13.3.
//!
//! W13.3 lands the corpus + bounded mutation engine + AST-subset bisect
//! minimizer. W13.4 wires the nightly GitHub Actions job. The scaffold
//! itself does not run a corpus beyond a single self-test seed.

use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub mod divergence;

pub use divergence::{Divergence, ModeOutcome, Signal, classify_divergence};

/// Default wall-clock budget per subprocess invocation, mirroring the W13
/// audit §2 corrected-harness shape (`timeout 30 ...`).
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// Errors surfaced from the harness driver layer.
///
/// Distinct from `Divergence`: a `HarnessError` means the harness itself
/// failed to obtain a comparison (e.g. binary missing, snippet unreadable,
/// subprocess spawn failed). A `Divergence` is a successful comparison
/// whose result happens to be a divergence.
#[derive(Debug)]
pub enum HarnessError {
    /// Failed to read the snippet at the requested path.
    SnippetRead { path: PathBuf, source: io::Error },
    /// Failed to spawn the `shape` binary subprocess.
    SpawnFailed {
        binary: PathBuf,
        source: io::Error,
    },
    /// Failed while waiting on or killing the subprocess.
    WaitFailed(io::Error),
    /// I/O failure while writing a findings record.
    FindingsWrite(io::Error),
}

impl std::fmt::Display for HarnessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SnippetRead { path, source } => {
                write!(f, "failed to read snippet {}: {}", path.display(), source)
            }
            Self::SpawnFailed { binary, source } => write!(
                f,
                "failed to spawn shape binary {}: {}",
                binary.display(),
                source
            ),
            Self::WaitFailed(e) => write!(f, "subprocess wait failed: {}", e),
            Self::FindingsWrite(e) => write!(f, "findings write failed: {}", e),
        }
    }
}

impl std::error::Error for HarnessError {}

/// Configuration for `compare_outputs`.
///
/// Defaults pick a stable shape that matches the W13 audit §2 invocation
/// (`timeout 30 ./target/release/shape run --mode {vm,jit} <file>
/// 2>/dev/null | tail -1`). Callers (CLI / future corpus runner) override
/// the binary path + timeout as needed.
#[derive(Debug, Clone)]
pub struct CompareConfig {
    /// Path to the `shape` binary. Defaults to `target/release/shape`
    /// relative to the workspace root, but the CLI exposes
    /// `--shape-bin=<path>` so external callers can target a built artifact
    /// from a different worktree (e.g. CI).
    pub shape_binary: PathBuf,
    /// Per-mode wall-clock budget; SIGKILL on overrun.
    pub timeout: Duration,
}

impl Default for CompareConfig {
    fn default() -> Self {
        Self {
            shape_binary: PathBuf::from("target/release/shape"),
            timeout: DEFAULT_TIMEOUT,
        }
    }
}

/// Result returned by `compare_outputs`.
#[derive(Debug, Clone)]
pub struct CompareResult {
    pub snippet: PathBuf,
    pub vm: ModeOutcome,
    pub jit: ModeOutcome,
}

/// Run `<shape_binary> run --mode vm <snippet>` and `--mode jit <snippet>`
/// as independent subprocesses, returning a `CompareResult` containing both
/// `ModeOutcome`s.
///
/// Mirrors the W13 audit §2 invocation:
/// - stderr is piped to `/dev/null` (drops `[jit-fallback]` info per §2.1).
/// - stdout is captured + reduced to its last non-empty line (`tail -1`
///   shape).
/// - the per-mode subprocess is bounded by `cfg.timeout`; overrun marks
///   that mode's outcome with `timed_out = true`.
pub fn compare_outputs(snippet: &Path, cfg: &CompareConfig) -> Result<CompareResult, HarnessError> {
    // Validate snippet exists + is readable up front so a missing path is a
    // HarnessError, not a `shape run` parse error masking real harness misuse.
    fs::metadata(snippet).map_err(|source| HarnessError::SnippetRead {
        path: snippet.to_path_buf(),
        source,
    })?;

    let vm = run_mode(&cfg.shape_binary, "vm", snippet, cfg.timeout)?;
    let jit = run_mode(&cfg.shape_binary, "jit", snippet, cfg.timeout)?;

    Ok(CompareResult {
        snippet: snippet.to_path_buf(),
        vm,
        jit,
    })
}

fn run_mode(
    binary: &Path,
    mode: &str,
    snippet: &Path,
    timeout: Duration,
) -> Result<ModeOutcome, HarnessError> {
    let mut cmd = Command::new(binary);
    cmd.arg("run")
        .arg("--mode")
        .arg(mode)
        .arg(snippet.as_os_str())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .stdin(Stdio::null());

    let mut child = cmd.spawn().map_err(|source| HarnessError::SpawnFailed {
        binary: binary.to_path_buf(),
        source,
    })?;

    let start = std::time::Instant::now();
    loop {
        match child.try_wait().map_err(HarnessError::WaitFailed)? {
            Some(status) => {
                let mut stdout = Vec::new();
                if let Some(mut out) = child.stdout.take() {
                    let _ = io::Read::read_to_end(&mut out, &mut stdout);
                }
                let stdout_str = String::from_utf8_lossy(&stdout).into_owned();
                let tail = stdout_tail(&stdout_str);
                let code = status.code().unwrap_or(-1);
                return Ok(ModeOutcome::new(tail, code));
            }
            None => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Ok(ModeOutcome::timeout());
                }
                std::thread::sleep(Duration::from_millis(25));
            }
        }
    }
}

/// Extract the final non-empty line of captured stdout, mirroring `tail -1`
/// from the corrected W12 smoke-harness invocation.
fn stdout_tail(out: &str) -> String {
    out.lines()
        .filter(|l| !l.is_empty())
        .next_back()
        .unwrap_or("")
        .to_string()
}

/// Persist a divergence finding to disk so triage (manual or CI artifact
/// upload per W13 audit §6.4) has a self-contained record.
///
/// Format: a single text file containing the snippet source verbatim,
/// followed by the two `ModeOutcome`s and the divergence classification.
/// Filename uses a timestamp + divergence name to keep findings ordered.
pub fn record_finding(
    cmp: &CompareResult,
    divergence: &Divergence,
    output_dir: &Path,
) -> Result<PathBuf, HarnessError> {
    fs::create_dir_all(output_dir).map_err(HarnessError::FindingsWrite)?;

    let snippet_source = fs::read_to_string(&cmp.snippet).map_err(|source| {
        HarnessError::SnippetRead {
            path: cmp.snippet.clone(),
            source,
        }
    })?;

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let snippet_stem = cmp
        .snippet
        .file_stem()
        .and_then(OsStr::to_str)
        .unwrap_or("snippet");
    let filename = format!("{nanos}-{}-{snippet_stem}.txt", divergence.name());
    let path = output_dir.join(filename);

    let body = format!(
        "snippet: {}\n\
         classification: {}\n\
         signal: {:?}\n\
         \n\
         === source ===\n{snippet_source}\n\
         === vm ===\n\
         stdout_tail: {:?}\n\
         exit_code: {:?}\n\
         timed_out: {}\n\
         \n\
         === jit ===\n\
         stdout_tail: {:?}\n\
         exit_code: {:?}\n\
         timed_out: {}\n",
        cmp.snippet.display(),
        divergence.name(),
        divergence.signal(),
        cmp.vm.stdout_tail,
        cmp.vm.exit_code,
        cmp.vm.timed_out,
        cmp.jit.stdout_tail,
        cmp.jit.exit_code,
        cmp.jit.timed_out,
    );

    fs::write(&path, body).map_err(HarnessError::FindingsWrite)?;
    Ok(path)
}

/// Placeholder for the AST-subset-bisect minimizer described in W13 audit
/// §5.1. The scaffold returns the original snippet unchanged + a
/// `Unimplemented` marker so callers can wire the API now and the W13.3
/// closure-wave fills in the bisect body without re-shaping the call site.
///
/// W13.3 will replace this with a statement-removal binary search bounded
/// to 50 iterations per finding per §5.1.
#[derive(Debug)]
pub enum MinimizeOutcome {
    /// W13.3 has not yet landed the bisect engine; the input is returned
    /// untouched.
    Unimplemented { original: PathBuf },
}

pub fn minimize_reproducer(
    snippet: &Path,
    _cfg: &CompareConfig,
) -> Result<MinimizeOutcome, HarnessError> {
    Ok(MinimizeOutcome::Unimplemented {
        original: snippet.to_path_buf(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stdout_tail_returns_last_non_empty_line() {
        assert_eq!(stdout_tail("4950\n"), "4950");
        assert_eq!(stdout_tail("a\nb\nc\n"), "c");
        assert_eq!(stdout_tail("only-line"), "only-line");
        assert_eq!(stdout_tail(""), "");
        // Empty trailing lines must not eat the real tail.
        assert_eq!(stdout_tail("real\n\n\n"), "real");
    }

    #[test]
    fn compare_outputs_surfaces_missing_snippet_as_harness_error() {
        let cfg = CompareConfig::default();
        let result = compare_outputs(Path::new("/this/path/does/not/exist.shape"), &cfg);
        match result {
            Err(HarnessError::SnippetRead { .. }) => {}
            other => panic!("expected SnippetRead error, got {:?}", other),
        }
    }

    #[test]
    fn minimize_reproducer_is_unimplemented_at_scaffold_time() {
        let tmp = std::env::temp_dir().join("shape-fuzz-minimize-stub.shape");
        let _ = fs::write(&tmp, "print(1)\n");
        let cfg = CompareConfig::default();
        let outcome = minimize_reproducer(&tmp, &cfg).expect("placeholder must succeed");
        match outcome {
            MinimizeOutcome::Unimplemented { original } => assert_eq!(original, tmp),
        }
    }

    #[test]
    fn record_finding_writes_a_text_file_describing_the_divergence() {
        let dir = std::env::temp_dir().join(format!(
            "shape-fuzz-record-test-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let snippet = dir.join("seed.shape");
        fs::write(&snippet, "print(1)\n").unwrap();

        let cmp = CompareResult {
            snippet: snippet.clone(),
            vm: ModeOutcome::new("1".into(), 0),
            jit: ModeOutcome::new("2".into(), 0),
        };
        let div = Divergence::StdoutTailDivergence;
        let written = record_finding(&cmp, &div, &dir).expect("write should succeed");
        let body = fs::read_to_string(&written).unwrap();
        assert!(body.contains("classification: stdout-tail-divergence"));
        assert!(body.contains("=== source ==="));
        assert!(body.contains("print(1)"));
        assert!(body.contains("stdout_tail: \"1\""));
        assert!(body.contains("stdout_tail: \"2\""));
        let _ = fs::remove_dir_all(&dir);
    }
}
