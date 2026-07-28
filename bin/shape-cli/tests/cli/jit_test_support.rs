//! Shared subprocess support for the CLI JIT integration matrices.

use assert_cmd::Command;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

pub(super) struct CapturedRun {
    pub(super) exit_code: Option<i32>,
    pub(super) stdout: String,
    pub(super) stderr: String,
}

fn cli_process_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

pub(super) fn workspace_fixture_path(suite: &str, name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("bin parent")
        .parent()
        .expect("workspace root")
        .join("tests")
        .join(suite)
        .join(name)
}

pub(super) fn run_workspace_fixture(mode: &str, suite: &str, fixture: &str) -> CapturedRun {
    run_shape_path(mode, &workspace_fixture_path(suite, fixture))
}

/// In-harness vacuity guard: a top-level `comptime` block silently excludes a
/// fixture from native JIT, so a zero-fallback assertion would pass vacuously.
/// (An annotation `comptime post` handler is NOT a top-level comptime block —
/// the C2 generation fixtures rely on exactly that distinction.)
pub(super) fn assert_fixture_has_no_top_level_comptime(fixture: &str) {
    let path = workspace_fixture_path("smokes-jit-closure", fixture);
    let source = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "{fixture}: failed to read fixture {}: {error}",
            path.display()
        )
    });
    let program = shape_ast::parser::parse_program(&source)
        .unwrap_or_else(|error| panic!("{fixture}: failed to parse fixture: {error}"));
    assert!(
        !shape_vm::compiler::program_has_top_level_comptime(&program),
        "{fixture}: top-level comptime silently excludes the fixture from native JIT"
    );
}

pub(super) fn run_shape_path(mode: &str, path: &Path) -> CapturedRun {
    // Both JIT matrices share this one lock. Each child creates a Tokio
    // runtime, so serializing subprocesses keeps low-TasksMax gates stable.
    let _guard = cli_process_lock()
        .lock()
        .expect("CLI JIT subprocess lock poisoned");
    let assertion = Command::new(assert_cmd::cargo::cargo_bin!("shape"))
        .args(["run", "--mode", mode])
        .arg(path)
        .timeout(std::time::Duration::from_secs(60))
        .assert();
    let output = assertion.get_output();
    CapturedRun {
        exit_code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

pub(super) fn count_fallback_lines(stderr: &str) -> usize {
    stderr
        .lines()
        .filter(|line| line.starts_with("[jit-fallback]"))
        .count()
}
