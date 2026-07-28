//! End-to-end foreign-call acceptance tests — the "the path can never
//! silently die again" guard (ffi-rebuild §7 / goal 7, WF-2A stage 5–6).
//!
//! These drive the REAL `shape` binary as a subprocess, so they exercise the
//! genuine end-to-end path a user hits: CLI extension discovery → lazy
//! link-now in `op_call_foreign` → `invoke_foreign_kinded` shared core →
//! typed `KindedSlot`/`NativeKind` marshal → extension/libffi → unmarshal →
//! `Result` carrier. A regression that re-stubs `op_call_foreign` (the exact
//! failure the 2026-07-04 audit found — the only foreign e2e tests were
//! feature-gated out of every tier AND written against compiler-rejected
//! `-> string` signatures) fails HERE instead of shipping green.
//!
//! ## CI tier placement (deliberate, not incidental)
//!
//! * **`extern C` tests need no extension** and run in the DEFAULT gate —
//!   `cargo test --workspace --all-targets`, i.e. `.github/workflows/ci.yml`
//!   and `just test-all` / `just ci-test`. `extern_c_labs_scalar` is the
//!   zero-build-cost sentinel from acceptance probe 1: it links libc's `labs`
//!   with no fixture, so the foreign path can never silently die on an
//!   always-on tier.
//! * **`fn python` / `fn typescript` tests need the built extension `.so`s**
//!   (PyO3 CPython, deno_core V8) plus those runtimes, so they are
//!   `#[ignore]`d in the default gate and run by the dedicated **`just
//!   test-ffi`** tier (`build-extensions` then `cargo test … --
//!   --include-ignored`), which is wired into `ci.yml` as the `ffi` job. They
//!   are NOT feature-gated-into-oblivion: `--include-ignored` runs them and
//!   [`extension_dir`] PANICS (never silently skips) when the extensions were
//!   not built, so the FFI tier fails loudly rather than quietly running zero
//!   foreign tests.

use assert_cmd::Command;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

fn shape_cmd() -> Command {
    Command::new(assert_cmd::cargo::cargo_bin!("shape"))
}

/// Serialize subprocess launches: every child spins a Tokio runtime and, for
/// the dynamic verticals, a CPython / V8 embedding. One-at-a-time keeps a
/// resource spike in one child from destabilizing a sibling (same rationale
/// as `cli/jit_fallback_diagnostic_matrix.rs`).
fn cli_process_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

struct Run {
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
}

/// Run `program` through `shape run --mode <mode>`, optionally with
/// `--extension-dir <ext>`. stdout carries `print` output; the extension-load
/// banner and any `[jit-fallback]` line go to stderr, so assertions key on
/// stdout with distinctive `MARKER=` tokens.
fn run_shape(program: &str, mode: &str, ext_dir: Option<&Path>) -> Run {
    let dir = tempfile::tempdir().expect("tempdir");
    let script = dir.path().join("ffi_e2e.shape");
    {
        let mut f = std::fs::File::create(&script).expect("write script");
        f.write_all(program.as_bytes()).expect("write body");
    }

    let _guard = cli_process_lock().lock().expect("ffi e2e process lock");
    let mut cmd = shape_cmd();
    cmd.args(["run", "--mode", mode]);
    if let Some(ext) = ext_dir {
        cmd.arg("--extension-dir").arg(ext);
    }
    cmd.arg(&script).timeout(Duration::from_secs(120));

    let assertion = cmd.assert();
    let output = assertion.get_output();
    Run {
        exit_code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// Run the same program under `--mode vm` AND `--mode jit`, assert BOTH exit 0
/// with the expected stdout marker, and assert the two stdouts are byte-equal.
/// This is the e2e half of the "JIT cannot silently diverge" invariant
/// (ffi-rebuild §4.9 / probe 11): one shared `invoke_foreign_kinded` core, so
/// vm ≡ jit for foreign-call semantics by construction.
fn assert_vm_jit_stdout(program: &str, ext_dir: Option<&Path>, marker: &str) {
    let vm = run_shape(program, "vm", ext_dir);
    let jit = run_shape(program, "jit", ext_dir);
    assert_eq!(
        vm.exit_code,
        Some(0),
        "vm mode should exit 0; stdout={:?} stderr={}",
        vm.stdout,
        vm.stderr
    );
    assert_eq!(
        jit.exit_code,
        Some(0),
        "jit mode should exit 0; stdout={:?} stderr={}",
        jit.stdout,
        jit.stderr
    );
    assert!(
        vm.stdout.contains(marker),
        "vm stdout missing {marker:?}; stdout={:?} stderr={}",
        vm.stdout,
        vm.stderr
    );
    assert_eq!(
        vm.stdout, jit.stdout,
        "vm/jit stdout diverged for a foreign call (§4.9 non-divergence): vm={:?} jit={:?}",
        vm.stdout, jit.stdout
    );
}

/// The built extension directory holding the dynamic-language runtime `.so`s.
/// `just build-extensions` copies them to `<workspace>/extensions`; override
/// with `$SHAPE_FFI_EXT_DIR`. PANICS (never silently skips) when the directory
/// contains no `.so`, so a `--include-ignored` FFI run without a prior
/// `build-extensions` fails loudly instead of quietly running nothing.
fn extension_dir() -> PathBuf {
    let dir = std::env::var_os("SHAPE_FFI_EXT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            // CARGO_MANIFEST_DIR = <workspace>/bin/shape-cli
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .expect("bin/ parent")
                .parent()
                .expect("workspace root")
                .join("extensions")
        });
    let has_so = std::fs::read_dir(&dir)
        .ok()
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .any(|e| e.path().extension().is_some_and(|x| x == "so"))
        })
        .unwrap_or(false);
    assert!(
        has_so,
        "FFI e2e tier requires the language-runtime extensions to be built first.\n\
         No `.so` found in {}.\n\
         Run `just test-ffi` (which builds them) or `just build-extensions`, \
         or set $SHAPE_FFI_EXT_DIR to a directory containing \
         libshape_ext_python.so / libshape_ext_typescript.so.",
        dir.display()
    );
    dir
}

// =========================================================================
// extern C — default gate (no extension, always-on sentinel)
// =========================================================================

/// Acceptance probe 1 (the never-die sentinel): `labs` from the libc alias,
/// zero fixtures. `labs` (not `abs`) because `long labs(long)` is a genuine
/// i64→i64 on LP64, so the i64 cif reads defined return bits. Runs in BOTH
/// modes and asserts vm ≡ jit.
#[test]
fn extern_c_labs_scalar_vm_jit() {
    let program = "extern \"C\" fn labs(x: int) -> int from \"c\"\n\
                   let a: int = labs(-42)\n\
                   print(f\"ABS={a}\")\n";
    assert_vm_jit_stdout(program, None, "ABS=42");
}

/// Aggregate/container argument across the C boundary: a Shape `string`
/// marshals to `const char*` for libc `strlen` (§4.6.2 string arg encoding).
/// This is the extern-C analogue of "pass a container" — a heap value crossing
/// the boundary, not a scalar in a register.
#[test]
fn extern_c_string_aggregate_arg() {
    let program = "extern \"C\" fn strlen(s: string) -> int from \"c\"\n\
                   let n: int = strlen(\"hello world\")\n\
                   print(f\"LEN={n}\")\n";
    let run = run_shape(program, "vm", None);
    assert_eq!(run.exit_code, Some(0), "stderr={}", run.stderr);
    assert!(
        run.stdout.contains("LEN=11"),
        "stdout={:?} stderr={}",
        run.stdout,
        run.stderr
    );
}

/// Lazy linking (§4.2): declaring an `extern C fn` against a symbol that does
/// not exist is NEVER fatal — a program that declares but never calls it runs
/// to completion, exit 0.
#[test]
fn extern_c_declaration_is_never_fatal() {
    let program = "extern \"C\" fn nope(x: int) -> int from \"c\" as \"definitely_missing_symbol_xyz\"\n\
         print(\"DECLARED_OK\")\n";
    let run = run_shape(program, "vm", None);
    assert_eq!(
        run.exit_code,
        Some(0),
        "declaring-without-calling must be non-fatal; stderr={}",
        run.stderr
    );
    assert!(
        run.stdout.contains("DECLARED_OK"),
        "stdout={:?}",
        run.stdout
    );
}

/// Error channel, extern-C link failure (§4.2 / §4.5 class 3): CALLING an
/// unresolvable symbol yields a structured `RuntimeError` naming the function,
/// the library, and the symbol — not a silent null, not a panic.
#[test]
fn extern_c_missing_symbol_is_structured_error() {
    let program = "extern \"C\" fn nope(x: int) -> int from \"c\" as \"definitely_missing_symbol_xyz\"\n\
         print(nope(1))\n";
    let run = run_shape(program, "vm", None);
    assert_ne!(
        run.exit_code,
        Some(0),
        "calling an unresolvable symbol must fail; stdout={:?}",
        run.stdout
    );
    let msg = format!("{}{}", run.stdout, run.stderr);
    assert!(
        msg.contains("nope") && msg.contains("definitely_missing_symbol_xyz"),
        "error must name the function and symbol; got: {msg}"
    );
}

// =========================================================================
// fn python — FFI tier (`just test-ffi`, #[ignore] in the default gate)
// =========================================================================

/// Happy path scalar: `fn python` returns `Result<int>` (the dynamic-language
/// Result mandate, §3.6), the call evaluates to `Ok(7)`. Runs vm ≡ jit.
#[test]
#[ignore = "needs built python extension + CPython; run via `just test-ffi`"]
fn python_scalar_ok_vm_jit() {
    let ext = extension_dir();
    let program = "fn python add(a: int, b: int) -> Result<int> {\n\
                   \x20   return a + b\n\
                   }\n\
                   match add(3, 4) { Ok(v) => print(f\"RESULT={v}\"), Err(e) => print(f\"ERR={e}\") }\n";
    assert_vm_jit_stdout(program, Some(&ext), "RESULT=7");
}

/// Container argument: an `Array<int>` marshals across the boundary as a list
/// (§4.4 scalar-element `Array<T>` arm, stage 3). `total([1,2,3,4]) == Ok(10)`.
#[test]
#[ignore = "needs built python extension + CPython; run via `just test-ffi`"]
fn python_container_array_arg() {
    let ext = extension_dir();
    let program = "fn python total(xs: Array<int>) -> Result<int> {\n\
                   \x20   return sum(xs)\n\
                   }\n\
                   match total([1, 2, 3, 4]) { Ok(v) => print(f\"RESULT={v}\"), Err(e) => print(f\"ERR={e}\") }\n";
    let run = run_shape(program, "vm", Some(&ext));
    assert_eq!(run.exit_code, Some(0), "stderr={}", run.stderr);
    assert!(
        run.stdout.contains("RESULT=10"),
        "stdout={:?} stderr={}",
        run.stdout,
        run.stderr
    );
}

/// Error channel class 1a (§4.5): a genuine Python exception becomes a class-1
/// `Err` on the user's `Result` — catchable via `match`, program continues
/// (exit 0). Distinguishability companion: the payload does NOT carry the
/// `TypeConformanceError:` prefix (that prefix is reserved for nonconforming
/// returns, §4.5 (1b)).
#[test]
#[ignore = "needs built python extension + CPython; run via `just test-ffi`"]
fn python_exception_becomes_catchable_err() {
    let ext = extension_dir();
    let program = "fn python boom(x: int) -> Result<int> {\n\
                   \x20   raise ValueError(\"boom\")\n\
                   }\n\
                   match boom(1) { Ok(v) => print(f\"OK={v}\"), Err(e) => print(f\"ERR={e}\") }\n";
    let run = run_shape(program, "vm", Some(&ext));
    assert_eq!(
        run.exit_code,
        Some(0),
        "foreign exception must be a catchable Err, not an abort; stderr={}",
        run.stderr
    );
    assert!(
        run.stdout.contains("ERR=")
            && run.stdout.contains("ValueError")
            && run.stdout.contains("boom"),
        "Err payload should carry the foreign exception; stdout={:?}",
        run.stdout
    );
    assert!(
        !run.stdout.contains("TypeConformanceError:"),
        "a genuine exception must NOT carry the TypeConformanceError prefix \
         (that discriminator is reserved for nonconforming returns); stdout={:?}",
        run.stdout
    );
}

/// Error channel class 1b (§4.5, Q13/OQ10 override 2026-07-05): a Python body
/// returning a value that VIOLATES its declared `Result<int>` (returns a
/// string) is a class-1 `Err` whose payload begins with the stable
/// `TypeConformanceError: ` discriminator and names the expected type and the
/// offending value — NOT a `VMError` abort, NOT a silent null. Program
/// continues (exit 0), so `match`/`?` handle it.
#[test]
#[ignore = "needs built python extension + CPython; run via `just test-ffi`"]
fn python_nonconforming_return_is_typeconformance_err() {
    let ext = extension_dir();
    let program = "fn python bad(x: int) -> Result<int> {\n\
                   \x20   return \"not an int\"\n\
                   }\n\
                   match bad(1) { Ok(v) => print(f\"OK={v}\"), Err(e) => print(f\"ERR={e}\") }\n";
    let run = run_shape(program, "vm", Some(&ext));
    assert_eq!(
        run.exit_code,
        Some(0),
        "nonconforming return must be a catchable Err, not an abort; stderr={}",
        run.stderr
    );
    assert!(
        run.stdout.contains("ERR=TypeConformanceError: "),
        "nonconforming return must carry the stable TypeConformanceError prefix; stdout={:?}",
        run.stdout
    );
    assert!(
        run.stdout.contains("int"),
        "the conformance error must name the declared type; stdout={:?}",
        run.stdout
    );
}

// =========================================================================
// fn typescript — FFI tier (`just test-ffi`, #[ignore] in the default gate)
// =========================================================================

/// Happy path scalar via deno_core: `tadd(5, 6) == Ok(11)`. Runs vm ≡ jit.
#[test]
#[ignore = "needs built typescript extension + V8; run via `just test-ffi`"]
fn typescript_scalar_ok_vm_jit() {
    let ext = extension_dir();
    let program = "fn typescript tadd(a: int, b: int) -> Result<int> {\n\
                   \x20   return a + b;\n\
                   }\n\
                   match tadd(5, 6) { Ok(v) => print(f\"RESULT={v}\"), Err(e) => print(f\"ERR={e}\") }\n";
    assert_vm_jit_stdout(program, Some(&ext), "RESULT=11");
}

/// Container argument: an `Array<int>` marshals to a JS array;
/// `tsum([10,20,30]) == Ok(60)`.
#[test]
#[ignore = "needs built typescript extension + V8; run via `just test-ffi`"]
fn typescript_container_array_arg() {
    let ext = extension_dir();
    let program = "fn typescript tsum(xs: Array<int>) -> Result<int> {\n\
                   \x20   return xs.reduce((a, b) => a + b, 0);\n\
                   }\n\
                   match tsum([10, 20, 30]) { Ok(v) => print(f\"RESULT={v}\"), Err(e) => print(f\"ERR={e}\") }\n";
    let run = run_shape(program, "vm", Some(&ext));
    assert_eq!(run.exit_code, Some(0), "stderr={}", run.stderr);
    assert!(
        run.stdout.contains("RESULT=60"),
        "stdout={:?} stderr={}",
        run.stdout,
        run.stderr
    );
}

/// Error channel class 1a for TypeScript: a JS `throw` becomes a catchable
/// class-1 `Err`, program continues (exit 0), and the payload carries the
/// thrown message without the `TypeConformanceError:` prefix.
#[test]
#[ignore = "needs built typescript extension + V8; run via `just test-ffi`"]
fn typescript_throw_becomes_catchable_err() {
    let ext = extension_dir();
    let program = "fn typescript tboom(x: int) -> Result<int> {\n\
                   \x20   throw new Error(\"kaboom\");\n\
                   }\n\
                   match tboom(1) { Ok(v) => print(f\"OK={v}\"), Err(e) => print(f\"ERR={e}\") }\n";
    let run = run_shape(program, "vm", Some(&ext));
    assert_eq!(
        run.exit_code,
        Some(0),
        "JS throw must be a catchable Err, not an abort; stderr={}",
        run.stderr
    );
    assert!(
        run.stdout.contains("ERR=") && run.stdout.contains("kaboom"),
        "Err payload should carry the thrown message; stdout={:?}",
        run.stdout
    );
    assert!(
        !run.stdout.contains("TypeConformanceError:"),
        "a genuine throw must NOT carry the TypeConformanceError prefix; stdout={:?}",
        run.stdout
    );
}
