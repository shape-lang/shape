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
///
/// Also #202 tripwire (2), the `op_await` pass-through differential: `await` on
/// a SYNCHRONOUS foreign call yields the same value as not awaiting it. Only an
/// `async` declaration produces a future, so `await` here has nothing to
/// resolve and passes the value through — folded into this fixture rather than
/// given its own, so the two results are compared on one call in one process.
#[test]
#[ignore = "needs built python extension + CPython; run via `just test-ffi`"]
fn python_scalar_ok_vm_jit() {
    let ext = extension_dir();
    let program = "fn python add(a: int, b: int) -> Result<int> {\n\
                   \x20   return a + b\n\
                   }\n\
                   match add(3, 4) { Ok(v) => print(f\"RESULT={v}\"), Err(e) => print(f\"ERR={e}\") }\n\
                   match await add(3, 4) { Ok(v) => print(f\"AWAITED={v}\"), Err(e) => print(f\"ERR={e}\") }\n";
    assert_vm_jit_stdout(program, Some(&ext), "RESULT=7");
    let run = run_shape(program, "vm", Some(&ext));
    assert!(
        run.stdout.contains("AWAITED=7"),
        "#202 tripwire (2): `await` on a sync foreign call must pass the value \
         through unchanged; stdout={:?} stderr={}",
        run.stdout,
        run.stderr
    );
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

/// The #202 overlap assertion, measured INSIDE the foreign bodies.
///
/// Timing the subprocess from the outside cannot work here: under `cargo test`
/// the `shape` binary is the DEBUG build, whose stdlib compilation takes tens of
/// seconds and varies by seconds between runs — noise that swamps the
/// sub-second sleeps being measured. So each call reports the wall-clock
/// interval it actually occupied, as `[start, end]`, and the test asserts the
/// two intervals OVERLAP. That is a direct observation of two foreign bodies
/// being inside the runtime at the same moment, and no amount of startup cost or
/// machine load can fake it or break it.
///
/// `program` must print `A0=<start> A1=<end>` and `B0=… B1=…`. Units are
/// whatever the language's clock uses (seconds for Python's `time.time()`,
/// milliseconds for JS `Date.now()`); `min_span` is the nap length in those
/// units, and exists so a probe that returned instantly cannot pass vacuously.
fn assert_foreign_calls_overlapped(
    program: &str,
    ext_dir: Option<&Path>,
    min_span: f64,
    label: &str,
) {
    let run = run_shape(program, "vm", ext_dir);
    assert_eq!(run.exit_code, Some(0), "stderr={}", run.stderr);

    let field = |name: &str| -> f64 {
        let needle = format!("{name}=");
        let rest = run.stdout.split(&needle).nth(1).unwrap_or_else(|| {
            panic!(
                "{label}: missing {name} in stdout={:?} stderr={}",
                run.stdout, run.stderr
            )
        });
        let text: String = rest
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-')
            .collect();
        text.parse()
            .unwrap_or_else(|e| panic!("{label}: {name} is not a number ({text:?}): {e}"))
    };

    let (a_start, a_end) = (field("A0"), field("A1"));
    let (b_start, b_end) = (field("B0"), field("B1"));

    assert!(
        a_end - a_start >= min_span * 0.9 && b_end - b_start >= min_span * 0.9,
        "{label}: each call must actually have occupied its nap — got spans {} and {}, \
         expected about {min_span}",
        a_end - a_start,
        b_end - b_start
    );

    let overlap = a_end.min(b_end) - a_start.max(b_start);
    assert!(
        overlap > 0.0,
        "{label}: the two foreign calls did not overlap — A ran [{a_start}, {a_end}] and \
         B ran [{b_start}, {b_end}], which is serialization. The whole point of an async \
         foreign call is that both are in flight before either is awaited."
    );
    assert!(
        overlap >= min_span * 0.5,
        "{label}: the calls overlapped by only {overlap}, less than half of {min_span} — \
         they are mostly serialized"
    );
}

// =========================================================================
// async fn <language> — real offload (ADR-019 §5 / #202, POLY-ASYNC-OFFLOAD)
// =========================================================================
//
// The host-level proof of overlap lives in shape-vm's
// `executor::foreign_async` tripwires, against an instrumented fake
// extension, so it runs on every machine. These are the end-to-end half:
// the real CPython and V8 embeddings, driven through the real `shape`
// binary, sleeping in their own languages. They belong to the FFI tier for
// the same reason every other `fn python` / `fn typescript` test does.
//
// A wall-clock budget rather than an equality: the assertion is "clearly
// less than serialized", with headroom for interpreter startup and a loaded
// machine. Serialized would be at least 1.0s of sleep alone; the budget is
// well under that and cannot be met by accident.

/// The overlap tripwire, python. Two `async fn python` calls that each sleep
/// 500ms are STARTED before either is awaited; both must finish in about
/// 500ms of sleep, not 1000ms.
///
/// `time.sleep` is the right probe precisely because CPython releases the GIL
/// across it — which is the property the Python runtime's
/// `INSTANCE_CONCURRENCY_SHARED` declaration is claiming.
#[test]
#[ignore = "needs built python extension + CPython; run via `just test-ffi`"]
fn python_two_async_calls_overlap() {
    let ext = extension_dir();
    // Each call reports the interval it occupied. `time.sleep` is the right
    // probe precisely because CPython releases the GIL across it — the property
    // the extension's INSTANCE_CONCURRENCY_SHARED declaration is claiming.
    let program = "async fn python nap(ms: int) -> Result<Array<number>> {\n\
                   \x20   import time\n\
                   \x20   start = time.time()\n\
                   \x20   time.sleep(ms / 1000.0)\n\
                   \x20   return [start, time.time()]\n\
                   }\n\
                   let a = nap(500)\n\
                   let b = nap(500)\n\
                   match await a { Ok(v) => print(f\"A0={v[0]} A1={v[1]}\"), Err(e) => print(f\"ERR={e}\") }\n\
                   match await b { Ok(v) => print(f\"B0={v[0]} B1={v[1]}\"), Err(e) => print(f\"ERR={e}\") }\n";
    // `time.time()` is in seconds.
    assert_foreign_calls_overlapped(program, Some(&ext), 0.5, "async fn python");
}

/// The overlap tripwire, typescript. The TypeScript runtime declares
/// `INSTANCE_CONCURRENCY_THREAD_AFFINE`, so the overlap comes from two
/// dedicated workers each owning their own V8 isolate, not from one isolate
/// being re-entered.
///
/// A BUSY WAIT rather than a timer, for two reasons. The practical one: the
/// extension embeds a bare `deno_core::JsRuntime` with no web extensions, so
/// `setTimeout` is not defined (a pre-existing limitation of the TypeScript
/// vertical, unrelated to #202 — the offload itself runs fine and surfaces the
/// `ReferenceError` as a clean `Err`). The better one: a CPU-bound loop cannot
/// be overlapped by event-loop interleaving on a single isolate, only by two
/// isolates on two threads — which is exactly the claim being tested.
#[test]
#[ignore = "needs built typescript extension + V8; run via `just test-ffi`"]
fn typescript_two_async_calls_overlap() {
    let ext = extension_dir();
    // A BUSY WAIT rather than a timer, for two reasons. The practical one: the
    // extension embeds a bare `deno_core::JsRuntime` with no web extensions, so
    // `setTimeout` is not defined (a pre-existing limitation of the TypeScript
    // vertical, unrelated to #202 — the offload itself runs fine and surfaces
    // the `ReferenceError` as a clean `Err`). The better one: a CPU-bound loop
    // cannot be overlapped by event-loop interleaving on a single isolate, only
    // by two isolates on two threads — which is exactly the claim being tested.
    let program = "async fn typescript nap(ms: int) -> Result<Array<number>> {\n\
                   \x20   const start = Date.now();\n\
                   \x20   const stop = start + ms;\n\
                   \x20   while (Date.now() < stop) {}\n\
                   \x20   return [start, Date.now()];\n\
                   }\n\
                   let a = nap(500)\n\
                   let b = nap(500)\n\
                   match await a { Ok(v) => print(f\"A0={v[0]} A1={v[1]}\"), Err(e) => print(f\"ERR={e}\") }\n\
                   match await b { Ok(v) => print(f\"B0={v[0]} B1={v[1]}\"), Err(e) => print(f\"ERR={e}\") }\n";
    // `Date.now()` is in milliseconds.
    assert_foreign_calls_overlapped(program, Some(&ext), 500.0, "async fn typescript");
}

/// An `async fn python` call delivers its VALUE, its body's own `await` still
/// runs, and vm ≡ jit.
///
/// All three in one fixture because they are one claim about one call: the
/// python extension wraps an async declaration in `async def` +
/// `asyncio.run(...)`, so `await` inside the body is legal and drives to
/// completion — now on a worker thread rather than on the interpreter thread —
/// and the result is the declared `Result<int>` after the Shape-side `await`.
/// vm ≡ jit holds by construction: a function containing a foreign call is
/// interpreter-only in both modes (`vm_only_opcode_reason(CallForeignAsync)`).
#[test]
#[ignore = "needs built python extension + CPython; run via `just test-ffi`"]
fn python_async_body_awaits_internally_and_delivers_its_value_vm_jit() {
    let ext = extension_dir();
    let program = "async fn python fetch(x: int) -> Result<int> {\n\
                   \x20   import asyncio\n\
                   \x20   await asyncio.sleep(0)\n\
                   \x20   return x * 2\n\
                   }\n\
                   match await fetch(21) { Ok(v) => print(f\"RESULT={v}\"), Err(e) => print(f\"ERR={e}\") }\n";
    assert_vm_jit_stdout(program, Some(&ext), "RESULT=42");
}

/// REGRESSION (#202 work item 6): a foreign call inside a SPAWNED user
/// `async fn` used to fail link-now with "no extension provides language
/// 'python'", because the isolated task VM was built with an empty extension
/// registry. The same call from the parent VM worked, which is what made it
/// confusing rather than merely broken.
#[test]
#[ignore = "needs built python extension + CPython; run via `just test-ffi`"]
fn python_foreign_call_inside_a_spawned_async_fn_succeeds() {
    let ext = extension_dir();
    // `async let` is only legal inside an async fn, hence the wrapper; the
    // deferred zero-arg call is what routes `work()` onto the isolated task VM.
    let program = "fn python double(a: int) -> Result<int> {\n\
                   \x20   return a * 2\n\
                   }\n\
                   async fn work() -> int {\n\
                   \x20   match double(21) { Ok(v) => return v, Err(e) => return 0 }\n\
                   }\n\
                   async fn driver() -> int {\n\
                   \x20   async let t = work()\n\
                   \x20   return await t\n\
                   }\n\
                   print(f\"RESULT={await driver()}\")\n";
    let run = run_shape(program, "vm", Some(&ext));
    assert_eq!(
        run.exit_code,
        Some(0),
        "a foreign call inside a spawned async fn must link; stderr={}",
        run.stderr
    );
    assert!(
        !run.stderr.contains("no extension provides language"),
        "the isolated task VM must inherit the parent's extension registry; stderr={}",
        run.stderr
    );
    assert!(
        run.stdout.contains("RESULT=42"),
        "stdout={:?} stderr={}",
        run.stdout,
        run.stderr
    );
}

// =========================================================================
// #199 POLY-ZERO-COPY — `shared` buffers through the real extension `.so`
// =========================================================================

/// The whole path, end to end: a `shared` declaration, a negotiated capability
/// in a loaded `.so`, a `memoryview` over Shape's own array, and the release
/// accounting on the way back.
///
/// The in-process fakes in `shape-vm` assert the host's behaviour and the
/// `shape-ext-python` unit tests assert CPython's; this is the one that proves
/// they meet — that the capability block survives the dlopen, the ABI
/// fingerprint still matches, and the pointer the body reads is the array the
/// Shape program built.
#[test]
#[ignore = "needs built python extension + CPython; run via `just test-ffi`"]
fn python_shared_buffer_reads_the_shape_arrays_own_memory() {
    let ext = extension_dir();
    let program = "fn python total(shared xs: Array<number>) -> Result<number> {\n\
                   \x20   assert xs.format == 'd'\n\
                   \x20   return float(sum(xs))\n\
                   }\n\
                   match total([1.5, 2.5, 3.0]) { Ok(v) => print(f\"RESULT={v}\"), Err(e) => print(f\"ERR={e}\") }\n";
    assert_vm_jit_stdout(program, Some(&ext), "RESULT=7.0");
}

/// The half a copy could not fake: `shared mut` writes land in the CALLER's
/// array, so the Shape side sees them after the call returns.
#[test]
#[ignore = "needs built python extension + CPython; run via `just test-ffi`"]
fn python_shared_mut_writes_are_visible_to_shape_after_the_call() {
    let ext = extension_dir();
    let program = "fn python double(shared mut xs: Array<number>) -> Result<int> {\n\
                   \x20   for i in range(len(xs)):\n\
                   \x20       xs[i] = xs[i] * 2.0\n\
                   \x20   return len(xs)\n\
                   }\n\
                   let mut xs = [1.0, 2.0, 3.0]\n\
                   match double(xs) { Ok(n) => print(f\"N={n}\"), Err(e) => print(f\"ERR={e}\") }\n\
                   print(f\"AFTER={xs[0]},{xs[1]},{xs[2]}\")\n";
    let run = run_shape(program, "vm", Some(&ext));
    assert!(
        run.stdout.contains("N=3"),
        "the call ran; stdout={:?} stderr={}",
        run.stdout,
        run.stderr
    );
    assert!(
        run.stdout.contains("AFTER=2.0,4.0,6.0"),
        "the body wrote into Shape's own buffer, with no copy back; stdout={:?} stderr={}",
        run.stdout,
        run.stderr
    );
}

/// #199 tripwire (1) through the real interpreter: a write through a
/// read-only view is refused by CPython itself, and Shape's array is untouched.
#[test]
#[ignore = "needs built python extension + CPython; run via `just test-ffi`"]
fn python_write_through_a_shared_view_is_refused_by_cpython() {
    let ext = extension_dir();
    let program = "fn python poke(shared xs: Array<number>) -> Result<int> {\n\
                   \x20   xs[0] = 99.0\n\
                   \x20   return 0\n\
                   }\n\
                   let xs = [1.0, 2.0]\n\
                   match poke(xs) { Ok(v) => print(f\"OK={v}\"), Err(e) => print(\"REFUSED\") }\n\
                   print(f\"AFTER={xs[0]}\")\n";
    let run = run_shape(program, "vm", Some(&ext));
    assert!(
        run.stdout.contains("REFUSED"),
        "writing through an immutable view raises; stdout={:?} stderr={}",
        run.stdout,
        run.stderr
    );
    assert!(
        run.stdout.contains("AFTER=1.0"),
        "and Shape's buffer is unchanged; stdout={:?} stderr={}",
        run.stdout,
        run.stderr
    );
}

/// #199 tripwire (2), the named corruption class, through the real path: a body
/// that stashes a live view of the buffer fails the call with a structured
/// boundary error naming the parameter — instead of leaving foreign code
/// pointing at memory Shape is about to reuse.
#[test]
#[ignore = "needs built python extension + CPython; run via `just test-ffi`"]
fn python_a_stashed_live_view_fails_the_call_at_the_boundary() {
    let ext = extension_dir();
    // A memoryview slice is the stdlib shape of `numpy.asarray(xs)` kept in a
    // global: the view object goes away, a live export of the buffer does not.
    let program = "fn python leak(shared xs: Array<number>) -> Result<int> {\n\
                   \x20   global KEPT\n\
                   \x20   KEPT = xs[0:2]\n\
                   \x20   return 0\n\
                   }\n\
                   match leak([1.0, 2.0, 3.0]) { Ok(v) => print(f\"OK={v}\"), Err(e) => print(f\"ERR={e}\") }\n";
    let run = run_shape(program, "vm", Some(&ext));
    let combined = format!("{}{}", run.stdout, run.stderr);
    assert!(
        combined.contains("still held a view"),
        "the boundary fails the call rather than reclaiming under a live view; \
         stdout={:?} stderr={}",
        run.stdout,
        run.stderr
    );
    assert!(
        combined.contains("'xs'"),
        "and names the parameter; stdout={:?} stderr={}",
        run.stdout,
        run.stderr
    );
}

/// The negative control ADR-019 §2 asks for: TypeScript declares no buffer
/// capability, so a `shared` parameter there is REFUSED rather than quietly
/// deep-copied. A silent fallback would make the declaration untrue.
#[test]
#[ignore = "needs built typescript extension; run via `just test-ffi`"]
fn typescript_shared_is_refused_because_the_runtime_offers_no_buffers() {
    let ext = extension_dir();
    let program = "fn typescript total(shared xs: Array<number>) -> Result<number> {\n\
                   \x20   return xs.reduce((a, b) => a + b, 0);\n\
                   }\n\
                   match total([1.0, 2.0]) { Ok(v) => print(f\"OK={v}\"), Err(e) => print(f\"ERR={e}\") }\n";
    let run = run_shape(program, "vm", Some(&ext));
    let combined = format!("{}{}", run.stdout, run.stderr);
    assert!(
        combined.contains("does not offer buffer sharing"),
        "the refusal names the missing capability; stdout={:?} stderr={}",
        run.stdout,
        run.stderr
    );
    assert!(
        !combined.contains("OK="),
        "and the call does not quietly succeed by copying; stdout={:?}",
        run.stdout
    );
}
