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
