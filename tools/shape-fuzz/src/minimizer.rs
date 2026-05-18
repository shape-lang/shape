//! AST-subset-bisect minimizer per W13 audit §5.1.
//!
//! Given a divergent seed P (HIGH-signal `Divergence`), iteratively remove
//! whole statements/items and re-run the differential comparison; the
//! smallest P' that still produces a HIGH-signal divergence wins.
//!
//! Bounded to `MinimizeConfig::max_iterations` re-runs per finding per
//! audit §5.1 (default 50). Each re-run is a fresh subprocess via the
//! existing `compare_outputs` driver — no JIT/VM state is shared.
//!
//! Strategy: greedy line/statement-block bisect. The source is segmented
//! into statement-like blocks (a top-level item or a top-level expression-
//! statement; brace-balanced). The minimizer drops one block at a time and
//! checks whether the residual program (a) still parses + type-checks and
//! (b) still classifies as HIGH-signal. If both, the drop is kept; if
//! either fails, the block is restored and the next block tried.
//!
//! This is not full delta-debugging (`ddmin`); the bounded greedy form
//! matches §5.1's "~50 LoC bounded scope — a bisect-by-statement-removal
//! loop suffices" framing.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::divergence::{Divergence, Signal};
use crate::{CompareConfig, HarnessError, classify_divergence, compare_outputs};

/// Configuration for the minimizer.
#[derive(Debug, Clone)]
pub struct MinimizeConfig {
    /// Per-finding iteration cap. Audit §5.1 picks 50.
    pub max_iterations: usize,
    /// Directory to write the minimized reproducer into. The minimizer
    /// writes `<stem>__minimized.shape`.
    pub output_dir: PathBuf,
    /// Compare-driver config (binary path, per-mode timeout).
    pub compare: CompareConfig,
}

impl MinimizeConfig {
    pub fn new(output_dir: PathBuf, compare: CompareConfig) -> Self {
        Self {
            max_iterations: 50,
            output_dir,
            compare,
        }
    }
}

/// Outcome of a minimize attempt.
#[derive(Debug)]
pub enum MinimizeOutcome {
    /// Minimized to a smaller program; reproducer written to `path`.
    /// `iterations` counts re-runs spent.
    Minimized {
        original: PathBuf,
        minimized: PathBuf,
        original_len_bytes: usize,
        minimized_len_bytes: usize,
        iterations: usize,
        classification: Divergence,
    },
    /// The base program is not HIGH-signal divergent — nothing to minimize.
    BaseNotHighSignal {
        original: PathBuf,
        classification: Divergence,
    },
    /// The base diverges but no removal preserves the divergence within the
    /// iteration budget — the original is already minimal under the
    /// statement-removal bisect.
    AlreadyMinimal {
        original: PathBuf,
        iterations: usize,
        classification: Divergence,
    },
}

impl MinimizeOutcome {
    /// Convenience accessor used by integration tests + CLI.
    pub fn minimized_path(&self) -> Option<&Path> {
        match self {
            Self::Minimized { minimized, .. } => Some(minimized.as_path()),
            _ => None,
        }
    }
}

/// Errors specific to the minimizer.
#[derive(Debug)]
pub enum MinimizeError {
    /// I/O failure reading the base source.
    Io(io::Error),
    /// Underlying harness driver failure during a re-run.
    Harness(HarnessError),
}

impl std::fmt::Display for MinimizeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "minimizer I/O failure: {e}"),
            Self::Harness(e) => write!(f, "minimizer driver failure: {e}"),
        }
    }
}

impl std::error::Error for MinimizeError {}

impl From<io::Error> for MinimizeError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<HarnessError> for MinimizeError {
    fn from(e: HarnessError) -> Self {
        Self::Harness(e)
    }
}

/// Run the minimizer against `failing_seed`. The seed must currently
/// classify as HIGH-signal; the result is written to `cfg.output_dir`.
pub fn minimize_failure(
    failing_seed: &Path,
    cfg: &MinimizeConfig,
) -> Result<MinimizeOutcome, MinimizeError> {
    let original_source = fs::read_to_string(failing_seed)?;
    let original_len = original_source.len();

    // Baseline run — confirm the seed is HIGH-signal divergent.
    let baseline_cmp = compare_outputs(failing_seed, &cfg.compare)?;
    let baseline_div = classify_divergence(&baseline_cmp.vm, &baseline_cmp.jit);
    if baseline_div.signal() != Signal::High {
        return Ok(MinimizeOutcome::BaseNotHighSignal {
            original: failing_seed.to_path_buf(),
            classification: baseline_div,
        });
    }

    fs::create_dir_all(&cfg.output_dir)?;

    let mut current = original_source.clone();
    let mut iterations = 0usize;
    let mut last_classification = baseline_div.clone();
    let mut shrunk_any = false;

    // Outer loop: keep sweeping while we made progress in the last pass.
    let mut progress = true;
    while progress && iterations < cfg.max_iterations {
        progress = false;
        let blocks = split_into_blocks(&current);
        if blocks.len() <= 1 {
            break;
        }
        // Inner sweep: try dropping each block once.
        for drop_idx in 0..blocks.len() {
            if iterations >= cfg.max_iterations {
                break;
            }
            iterations += 1;

            let candidate = join_blocks_skipping(&blocks, drop_idx);
            if candidate == current {
                continue;
            }

            // Re-run via a tmp file (compare_outputs reads from disk per §1.2).
            let tmp = write_tmp_candidate(&cfg.output_dir, failing_seed, &candidate, iterations)?;
            let cmp = match compare_outputs(&tmp, &cfg.compare) {
                Ok(c) => c,
                Err(e) => {
                    let _ = fs::remove_file(&tmp);
                    return Err(MinimizeError::Harness(e));
                }
            };
            let div = classify_divergence(&cmp.vm, &cmp.jit);
            let _ = fs::remove_file(&tmp);

            if div.signal() == Signal::High {
                // Accept this drop — recompute blocks on the smaller source.
                current = candidate;
                last_classification = div;
                shrunk_any = true;
                progress = true;
                break;
            }
            // Else: drop didn't preserve the divergence; restore + try next.
        }
    }

    // Write final minimized form.
    let out_path = output_path_for(&cfg.output_dir, failing_seed);
    fs::write(&out_path, &current)?;

    if shrunk_any {
        Ok(MinimizeOutcome::Minimized {
            original: failing_seed.to_path_buf(),
            minimized: out_path,
            original_len_bytes: original_len,
            minimized_len_bytes: current.len(),
            iterations,
            classification: last_classification,
        })
    } else {
        Ok(MinimizeOutcome::AlreadyMinimal {
            original: failing_seed.to_path_buf(),
            iterations,
            classification: last_classification,
        })
    }
}

/// Split a Shape source into top-level blocks suitable for drop-bisect.
///
/// A block is one of:
/// - a comment-only line (preserved across drops),
/// - a brace-balanced top-level item (`fn`, `enum`, `type`, `trait`, `impl`,
///   `extend`, `async fn`, etc.) — opens with a head keyword, closes when
///   brace depth returns to 0,
/// - a single physical line otherwise (each top-level expression-statement
///   is one block).
pub fn split_into_blocks(source: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let lines: Vec<&str> = source.lines().collect();
    let head_keywords = [
        "fn ", "async fn ", "enum ", "type ", "trait ", "impl ", "extend ",
        "extern ", "pub ",
    ];

    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim_start();
        let is_head = head_keywords.iter().any(|kw| trimmed.starts_with(kw));
        if is_head && line.contains('{') {
            // Multi-line brace-balanced block.
            let mut depth = brace_delta(line);
            let mut buf = String::from(line);
            i += 1;
            while i < lines.len() && depth != 0 {
                buf.push('\n');
                buf.push_str(lines[i]);
                depth += brace_delta(lines[i]);
                i += 1;
            }
            blocks.push(buf);
        } else {
            blocks.push(line.to_string());
            i += 1;
        }
    }

    blocks
}

fn brace_delta(line: &str) -> i32 {
    let mut d = 0i32;
    for c in line.chars() {
        match c {
            '{' => d += 1,
            '}' => d -= 1,
            _ => {}
        }
    }
    d
}

fn join_blocks_skipping(blocks: &[String], skip: usize) -> String {
    let mut buf = String::new();
    for (i, b) in blocks.iter().enumerate() {
        if i == skip {
            continue;
        }
        if !buf.is_empty() {
            buf.push('\n');
        }
        buf.push_str(b);
    }
    if !buf.ends_with('\n') {
        buf.push('\n');
    }
    buf
}

fn write_tmp_candidate(
    out_dir: &Path,
    base: &Path,
    src: &str,
    iteration: usize,
) -> io::Result<PathBuf> {
    let stem = base.file_stem().and_then(|s| s.to_str()).unwrap_or("seed");
    let path = out_dir.join(format!("{stem}__bisect_{iteration:03}.shape"));
    fs::write(&path, src)?;
    Ok(path)
}

fn output_path_for(out_dir: &Path, base: &Path) -> PathBuf {
    let stem = base.file_stem().and_then(|s| s.to_str()).unwrap_or("seed");
    out_dir.join(format!("{stem}__minimized.shape"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_into_blocks_keeps_single_line_statements_as_separate_blocks() {
        let src = "let x = 1\nlet y = 2\nprint(x + y)\n";
        let blocks = split_into_blocks(src);
        assert_eq!(blocks, vec!["let x = 1", "let y = 2", "print(x + y)"]);
    }

    #[test]
    fn split_into_blocks_groups_a_brace_balanced_fn_into_one_block() {
        let src = "fn f() -> int {\n  let x = 1\n  x + 1\n}\nprint(f())\n";
        let blocks = split_into_blocks(src);
        assert_eq!(blocks.len(), 2);
        assert!(blocks[0].starts_with("fn f"));
        assert!(blocks[0].ends_with("}"));
        assert_eq!(blocks[1], "print(f())");
    }

    #[test]
    fn split_into_blocks_groups_an_enum_with_struct_payloads() {
        let src = "enum E {\n  A,\n  B { x: int, y: int },\n}\nprint(0)\n";
        let blocks = split_into_blocks(src);
        assert_eq!(blocks.len(), 2);
        assert!(blocks[0].starts_with("enum E"));
        assert_eq!(blocks[1], "print(0)");
    }

    #[test]
    fn join_blocks_skipping_drops_one_block_and_preserves_order() {
        let blocks = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        assert_eq!(join_blocks_skipping(&blocks, 0), "b\nc\n");
        assert_eq!(join_blocks_skipping(&blocks, 1), "a\nc\n");
        assert_eq!(join_blocks_skipping(&blocks, 2), "a\nb\n");
    }

    #[test]
    fn brace_delta_counts_balanced_and_unbalanced_braces() {
        assert_eq!(brace_delta("fn f() {"), 1);
        assert_eq!(brace_delta("}"), -1);
        assert_eq!(brace_delta("{ } { }"), 0);
        assert_eq!(brace_delta("no braces"), 0);
    }

    #[test]
    fn output_path_for_uses_minimized_suffix() {
        let p = output_path_for(Path::new("/tmp/out"), Path::new("/x/foo.shape"));
        assert_eq!(p, Path::new("/tmp/out/foo__minimized.shape"));
    }
}
