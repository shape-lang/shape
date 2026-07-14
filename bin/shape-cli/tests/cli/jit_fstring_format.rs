//! CLI proof for typed MIR f-string formatting carriers.

use assert_cmd::Command;
use std::sync::{Mutex, OnceLock};

fn cli_process_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn shape_cmd() -> Command {
    Command::new(assert_cmd::cargo::cargo_bin!("shape"))
}

struct CapturedRun {
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn run_source(mode: &str, source: &str) -> CapturedRun {
    let dir = tempfile::tempdir().expect("temporary Shape source directory");
    let path = dir.path().join("fstring-format.shape");
    std::fs::write(&path, source).expect("write temporary Shape source");
    let _guard = cli_process_lock()
        .lock()
        .expect("JIT f-string CLI process lock poisoned");
    let assertion = shape_cmd()
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

fn fallback_lines(stderr: &str) -> Vec<&str> {
    stderr
        .lines()
        .filter(|line| line.starts_with("[jit-fallback]"))
        .collect()
}

#[test]
fn implemented_fstring_forms_execute_natively_with_vm_parity() {
    let source = r#"
fn tag_int(value: int) -> string { f"{value}" }
fn tag_bool(value: bool) -> string { f"{value}" }
fn tag_number(value: number) -> string { f"{value}" }
fn tag_string(value: string) -> string { f"{value}" }

print(tag_int(7))
print(tag_bool(true))
print(tag_number(1.0))
print(tag_string("shape"))
let left = 7
let right = true
print(f"{left}{right}")
print(f"value={left}")
print(f"{1.5:fixed(2)}")
"#;
    let vm = run_source("vm", source);
    let jit = run_source("jit", source);

    assert_eq!(vm.exit_code, Some(0), "VM failed: {}", vm.stderr);
    assert_eq!(jit.exit_code, Some(0), "JIT failed: {}", jit.stderr);
    assert_eq!(jit.stdout, vm.stdout, "VM/JIT f-string output divergence");
    assert_eq!(
        vm.stdout.trim(),
        "7\ntrue\n1.0\nshape\n7true\nvalue=7\n1.50"
    );
    assert!(
        fallback_lines(&jit.stderr).is_empty(),
        "implemented f-string forms silently fell back: {}",
        jit.stderr
    );
}

#[test]
fn table_spec_falls_back_before_vm_reports_its_explicit_rejection() {
    let source = r#"f"{1:table()}""#;
    let vm = run_source("vm", source);
    let jit = run_source("jit", source);

    assert_eq!(jit.exit_code, vm.exit_code, "JIT must preserve VM failure");
    assert_ne!(
        vm.exit_code,
        Some(0),
        "Table rendering must remain rejected"
    );
    assert!(vm.stderr.contains("FORMAT_SPEC_TABLE rendering deferred"));
    assert!(jit.stderr.contains("FORMAT_SPEC_TABLE rendering deferred"));
    let lines = fallback_lines(&jit.stderr);
    assert_eq!(lines.len(), 1, "expected one fallback: {}", jit.stderr);
    assert!(
        lines[0].contains("FormatValue Table spec"),
        "fallback must name the refused format class: {}",
        lines[0]
    );
}

#[test]
fn content_style_falls_back_before_native_execution() {
    let source = r#"print(f"{1:bold}")"#;
    let vm = run_source("vm", source);
    let jit = run_source("jit", source);

    assert_eq!(jit.exit_code, vm.exit_code, "JIT must preserve VM behavior");
    assert_eq!(
        jit.stdout, vm.stdout,
        "styled content output must stay VM-owned"
    );
    let lines = fallback_lines(&jit.stderr);
    assert_eq!(lines.len(), 1, "expected one fallback: {}", jit.stderr);
    assert!(
        lines[0].contains("FormatValue ContentStyle"),
        "fallback must name the refused format class: {}",
        lines[0]
    );
}
