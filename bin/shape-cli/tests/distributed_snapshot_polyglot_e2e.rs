#[path = "support/distributed_snapshot_polyglot.rs"]
mod support;

use std::process::{Command as StdCommand, Stdio};
use std::time::Duration;
use support::*;

#[test]
fn snapshot_hash_resume_cli_roundtrip() {
    let _guard = lock_process();
    let env = IsolatedEnv::new("shape-snapshot-e2e-");
    let store = env.snapshot_store("explicit-snapshots");
    let program = r#"use std::core::snapshot
match snapshot() {
    Ok(Snapshot::Hash(id)) => print(f"SNAP_HASH={id}")
    Ok(Snapshot::Resumed) => print("SNAP_RESUMED")
    Err(e) => print(f"SNAP_ERR={e}")
}
"#;

    let first = run_shape_program_with_snapshot_store(program, "vm", &env, &store);
    assert_success(&first, "initial snapshot run");
    let hash = marker_value(&first.stdout, "SNAP_HASH=").expect("SNAP_HASH marker");
    assert_hexish(&hash, "snapshot() hash");
    assert!(
        !first.stdout.contains("SNAP_RESUMED") && !first.stdout.contains("SNAP_ERR="),
        "initial run should save, not resume or error; stdout={:?}",
        first.stdout
    );

    let mut info = shape_cmd();
    info.arg("--snapshot-store")
        .arg(&store)
        .args(["snapshot", "info", &hash])
        .timeout(Duration::from_secs(30));
    env.apply_assert_cmd(&mut info);
    let info = info.output().unwrap();
    assert!(
        info.status.success(),
        "snapshot info stderr={}",
        String::from_utf8_lossy(&info.stderr)
    );

    let mut resume = shape_cmd();
    resume
        .arg("--snapshot-store")
        .arg(&store)
        .args(["--mode", "vm", "--resume", &hash])
        .timeout(Duration::from_secs(120));
    env.apply_assert_cmd(&mut resume);
    let out = resume.output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "resume stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(stdout.contains("SNAP_RESUMED"), "resume stdout={stdout:?}");
}

#[test]
fn remote_execute_user_surface_over_shape_serve() {
    let _guard = lock_process();
    let server = start_serve("none", None, &[]);
    let env = IsolatedEnv::new("shape-remote-user-e2e-");

    let execute = r#"use std::core::remote
let r = remote::execute("__ADDR__", "print(\"REMOTE_EXEC_INNER\")\n21 + 21")
match r {
    Ok(result) => print(result)
    Err(e) => print(f"REMOTE_EXEC_ERR={e}")
}
"#
    .replace("__ADDR__", &server.addr);
    let run = run_shape_program(&execute, "vm", &env);
    assert_success(&run, "remote::execute client");
    assert!(
        run.stdout.contains("REMOTE_EXEC_INNER") && !run.stdout.contains("REMOTE_EXEC_ERR="),
        "remote::execute should expose remote stdout through the user Result; stdout={:?} stderr={}",
        run.stdout,
        run.stderr
    );
}

#[test]
fn remote_call_user_result_ok_and_transport_err_over_shape_serve() {
    let _guard = lock_process();
    let server = start_serve("none", None, &[]);
    let env = IsolatedEnv::new("shape-remote-call-result-e2e-");
    let dead_addr = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().to_string()
    };
    let program = r#"use std::core::remote
fn mul(a: int, b: int) -> int { a * b }

let live = remote::call("__ADDR__", mul, 6, 7)
match live {
    Ok(v) => print(f"REMOTE_CALL_OK={v}")
    Err(_) => print("REMOTE_CALL_UNEXPECTED_ERR")
}

let dead = remote::call("__DEAD__", mul, 6, 7)
match dead {
    Ok(v) => print(f"REMOTE_CALL_DEAD_UNEXPECTED_OK={v}")
    Err(_) => print("REMOTE_CALL_DEAD_ERR")
}
"#
    .replace("__ADDR__", &server.addr)
    .replace("__DEAD__", &dead_addr);
    let run = run_shape_program(&program, "vm", &env);
    assert_success(&run, "remote::call Result Ok/Err client");
    assert!(
        run.stdout.contains("REMOTE_CALL_OK=42")
            && run.stdout.contains("REMOTE_CALL_DEAD_ERR")
            && !run.stdout.contains("REMOTE_CALL_UNEXPECTED_ERR")
            && !run.stdout.contains("REMOTE_CALL_DEAD_UNEXPECTED_OK"),
        "remote::call should hit live Ok and dead-port Err arms; stdout={:?} stderr={}",
        run.stdout,
        run.stderr
    );
}

#[test]
#[ignore = "0-ary @remote unsupported: the `@remote fn remote_snapshot_hash() -> string` \
target declares no parameters, so the decision `before` hook has no argument to \
receive/short-circuit and is loud-rejected (\"the target declares no parameters\"). The \
1-ary receiver-store save/resume path itself works (proven in Wave B dynamic-snapshot); \
only the 0-ary shape blocks. Capability tracked in #83 (E4-S5 @remote residual: async, \
0-ary, heterogeneous multi-arg)."]
fn remote_snapshot_returns_receiver_hash_over_remote_call() {
    let _guard = lock_process();
    let server = start_serve("none", None, &[]);
    let env = IsolatedEnv::new("shape-remote-snapshot-e2e-");
    let program = r#"use std::core::remote
use std::core::snapshot

@remote("__ADDR__")
fn remote_snapshot_hash() -> string {
    match snapshot() {
        Ok(Snapshot::Hash(id)) => id
        Ok(Snapshot::Resumed) => "RESUMED"
        Err(e) => f"ERR={e}"
    }
}

print(f"REMOTE_SNAPSHOT={remote_snapshot_hash()}")
"#
    .replace("__ADDR__", &server.addr);
    let run = run_shape_program(&program, "vm", &env);
    assert_success(&run, "remote snapshot client");
    let hash = marker_value(&run.stdout, "REMOTE_SNAPSHOT=").expect("REMOTE_SNAPSHOT marker");
    assert!(
        hash != "RESUMED" && !hash.starts_with("ERR="),
        "remote snapshot should save on receiver, not resume or hit barrier; stdout={:?} stderr={}",
        run.stdout,
        run.stderr
    );
    assert_hexish(&hash, "remote receiver snapshot hash");
}

#[test]
fn remote_call_two_argument_function_over_shape_serve() {
    let _guard = lock_process();
    let server = start_serve("none", None, &[]);
    let env = IsolatedEnv::new("shape-remote-call-mul-e2e-");
    let program = r#"use std::core::remote
fn mul(a: int, b: int) -> int { a * b }
let r = remote::call("__ADDR__", mul, 6, 7)
match r {
    Ok(v) => print(f"REMOTE_CALL_MUL={v}")
    Err(_) => print("REMOTE_CALL_MUL_ERR")
}
"#
    .replace("__ADDR__", &server.addr);
    let run = run_shape_program(&program, "vm", &env);
    assert_success(&run, "remote::call two-arg function client");
    assert!(
        run.stdout.contains("REMOTE_CALL_MUL=42") && !run.stdout.contains("REMOTE_CALL_MUL_ERR"),
        "remote::call mul(6, 7) should return 42; stdout={:?} stderr={}",
        run.stdout,
        run.stderr
    );
}

#[test]
fn remote_call_int_argument_and_return_preserves_value_kind() {
    let _guard = lock_process();
    let server = start_serve("none", None, &[]);
    let env = IsolatedEnv::new("shape-remote-call-int-e2e-");
    let program = r#"use std::core::remote
fn inc(x: int) -> int { x + 1 }
let r = remote::call("__ADDR__", inc, 41)
match r {
    Ok(v) => print(f"REMOTE_CALL_INC={v}")
    Err(_) => print("REMOTE_CALL_INC_ERR")
}
"#
    .replace("__ADDR__", &server.addr);
    let run = run_shape_program(&program, "vm", &env);
    assert_success(&run, "remote::call int client");
    assert!(
        run.stdout.contains("REMOTE_CALL_INC=42") && !run.stdout.contains("REMOTE_CALL_INC_ERR"),
        "remote::call inc(41) should preserve int value 42; stdout={:?} stderr={}",
        run.stdout,
        run.stderr
    );
}

#[test]
fn remote_call_number_closure_capture_over_shape_serve() {
    let _guard = lock_process();
    let server = start_serve("none", None, &[]);
    let env = IsolatedEnv::new("shape-remote-call-closure-e2e-");
    let program = r#"use std::core::remote
let base = 100.0
let add_base = |x| x + base
let r = remote::call("__ADDR__", add_base, 5.0)
match r {
    Ok(v) => print(f"REMOTE_CALL_CLOSURE={v}")
    Err(_) => print("REMOTE_CALL_CLOSURE_ERR")
}
"#
    .replace("__ADDR__", &server.addr);
    let run = run_shape_program(&program, "vm", &env);
    assert_success(&run, "remote::call closure client");
    assert!(
        run.stdout.contains("REMOTE_CALL_CLOSURE=105")
            && !run.stdout.contains("REMOTE_CALL_CLOSURE_ERR"),
        "remote::call closure base=100.0 add_base(5.0) should return 105; stdout={:?} stderr={}",
        run.stdout,
        run.stderr
    );
}

#[test]
fn remote_extern_c_transfer_executes_and_strict_node_refuses_ffi() {
    let _guard = lock_process();
    let env = IsolatedEnv::new("shape-remote-c-e2e-");
    let trusted = start_serve("none", None, &[]);
    let program = r#"from std::core::remote use { @remote }
extern "C" fn labs(x: int) -> int from "c"

@remote("__ADDR__")
fn remote_abs(x: int) -> int {
    labs(x)
}

print(f"REMOTE_C_ABS={remote_abs(-42)}")
"#
    .replace("__ADDR__", &trusted.addr);
    let ok = run_shape_program(&program, "vm", &env);
    assert_success(&ok, "remote extern C trusted receiver");
    assert!(
        ok.stdout.contains("REMOTE_C_ABS=42"),
        "stdout={:?}",
        ok.stdout
    );

    let strict = start_serve("strict", None, &[]);
    let refused = program.replace(&trusted.addr, &strict.addr);
    let denied = run_shape_program(&refused, "vm", &env);
    assert_ne!(denied.code, Some(0), "strict receiver must refuse FFI");
    let msg = combined(&denied);
    assert!(
        !msg.contains("REMOTE_C_ABS=42")
            && (msg.contains("ffi.call") || msg.to_lowercase().contains("permission")),
        "strict receiver refusal should name FFI/permission; output={msg:?}"
    );
}

#[test]
fn remote_python_transfer_self_skips_without_extension_and_refuses_without_opt_in() {
    remote_polyglot_transfer("python", "PY_REMOTE=105", "return x + 5", 100);
}

#[test]
fn remote_typescript_transfer_self_skips_without_extension_and_refuses_without_opt_in() {
    remote_polyglot_transfer("typescript", "TS_REMOTE=21", "return x + 1;", 20);
}

fn remote_polyglot_transfer(language: &str, expected: &str, body: &str, input: i64) {
    let _guard = lock_process();
    let Some(so) = require_language_ext_so(language) else {
        println!(
            "SKIP remote_{language}_transfer: libshape_ext_{language}.so not found; \
             build extensions, set SHAPE_FFI_EXT_DIR, or set SHAPE_REQUIRE_FFI_EXT=1 to require it"
        );
        return;
    };
    let env = IsolatedEnv::new("shape-remote-polyglot-e2e-");
    let opted_in = start_serve("none", Some(&so), &[language]);
    let prefix = if language == "python" {
        "PY_REMOTE"
    } else {
        "TS_REMOTE"
    };
    let program = format!(
        r#"from std::core::remote use {{ @remote }}
fn {language} add_remote(x: int) -> Result<int> {{
    {body}
}}

@remote("{addr}")
fn remote_add(x: int) -> int {{
    match add_remote(x) {{
        Ok(v) => v
        Err(e) => 0 - 1
    }}
}}

print(f"{prefix}={{remote_add({input})}}")
"#,
        addr = opted_in.addr
    );

    for mode in ["vm", "jit"] {
        let run = run_shape_program(&program, mode, &env);
        assert_success(&run, &format!("{language} remote transfer {mode}"));
        assert!(
            run.stdout.contains(expected),
            "{language} remote transfer {mode} expected {expected}; stdout={:?} stderr={}",
            run.stdout,
            run.stderr
        );
    }
    assert_serve_logged_foreign_stub(&opted_in, language);

    let strict = start_serve("none", Some(&so), &[]);
    let refused = program.replace(&opted_in.addr, &strict.addr);
    let run = run_shape_program(&refused, "vm", &env);
    let msg = combined(&run);
    assert!(
        run.code != Some(0) && !msg.contains(expected) && msg.to_lowercase().contains(language),
        "{language} strict receiver should refuse missing --ffi-languages opt-in; output={msg:?}"
    );
    assert_serve_logged_foreign_stub(&strict, language);
}

#[test]
#[ignore = "manual timing-sensitive SIGINT e2e; run serialized under cgroup"]
fn sigint_saves_snapshot_and_plain_resume_continues() {
    let _guard = lock_process();
    let env = IsolatedEnv::new("shape-sigint-resume-e2e-");
    let dir = tempfile::tempdir().unwrap();
    let script = write_script(
        dir.path(),
        "sigint.shape",
        r#"use std::core::time
let mut i = 0
while i < 200 {
    time::sleep_sync(10.0)
    i = i + 1
}
print(f"SIGINT_DONE={i}")
"#,
    );

    let mut child = StdCommand::new(shape_binary());
    child
        .args(["run", "--mode", "vm"])
        .arg(&script)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    env.apply_std_cmd(&mut child);
    let child = child.spawn().unwrap();
    std::thread::sleep(Duration::from_millis(350));
    let status = StdCommand::new("kill")
        .arg("-INT")
        .arg(child.id().to_string())
        .status()
        .unwrap();
    assert!(status.success(), "failed to send SIGINT");
    let interrupted = child.wait_with_output().unwrap();
    assert_eq!(
        interrupted.status.code(),
        Some(130),
        "SIGINT exit should be 130"
    );
    let stderr = String::from_utf8_lossy(&interrupted.stderr);
    let hash = marker_value(&stderr, "Snapshot saved: ").expect("SIGINT snapshot hash");
    assert_hexish(&hash, "SIGINT snapshot hash");

    let mut resume = shape_cmd();
    resume
        .args(["run", "--mode", "vm", "--resume", &hash])
        .timeout(Duration::from_secs(30));
    env.apply_assert_cmd(&mut resume);
    let out = resume.output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "resume stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        stdout.contains("SIGINT_DONE=200"),
        "resume stdout={stdout:?}"
    );
}

#[test]
fn tls_remote_call_user_surface_over_shape_serve() {
    let _guard = lock_process();
    assert_tls_remote_call_user_surface();
}

#[test]
#[ignore = "0-ary @remote unsupported: the `@remote fn remote_snapshot_hash() -> string` \
target declares no parameters, so the decision `before` hook has no argument to \
receive/short-circuit and is loud-rejected (\"the target declares no parameters\"). The \
1-ary receiver-store save path itself works (proven in Wave B dynamic-snapshot); only the \
0-ary shape blocks. Capability tracked in #83 (E4-S5 @remote residual: async, 0-ary, \
heterogeneous multi-arg)."]
fn remote_snapshot_hash_is_saved_in_selected_receiver_store() {
    let _guard = lock_process();
    let env = IsolatedEnv::new("shape-remote-snapshot-resume-e2e-");
    let receiver_store = env.snapshot_store("receiver-snapshots");
    let server = start_serve_with_snapshot_store("none", None, &[], &receiver_store);
    let program = r#"use std::core::remote
use std::core::snapshot

@remote("__ADDR__")
fn remote_snapshot_hash() -> string {
    match snapshot() {
        Ok(Snapshot::Hash(id)) => id
        Ok(Snapshot::Resumed) => "RESUMED"
        Err(e) => f"ERR={e}"
    }
}

print(f"REMOTE_SNAPSHOT={remote_snapshot_hash()}")
"#
    .replace("__ADDR__", &server.addr);

    let run = run_shape_program(&program, "vm", &env);
    assert_success(&run, "remote receiver snapshot producer");
    let hash = marker_value(&run.stdout, "REMOTE_SNAPSHOT=").expect("REMOTE_SNAPSHOT marker");
    assert_hexish(&hash, "remote receiver snapshot hash");

    let mut info = shape_cmd();
    info.arg("--snapshot-store")
        .arg(&receiver_store)
        .args(["snapshot", "info", &hash])
        .timeout(Duration::from_secs(30));
    env.apply_assert_cmd(&mut info);
    let info = info.output().unwrap();
    assert!(
        info.status.success(),
        "receiver snapshot must be visible in selected store; stderr={}",
        String::from_utf8_lossy(&info.stderr)
    );
}

#[test]
#[ignore = "0-ary @remote unsupported: the `@remote fn remote_snapshot_hash() -> string` \
target declares no parameters, so the decision `before` hook has no argument to \
receive/short-circuit and is loud-rejected (\"the target declares no parameters\"). The \
1-ary receiver-store resume path itself works (proven in Wave B dynamic-snapshot); only the \
0-ary shape blocks. Capability tracked in #83 (E4-S5 @remote residual: async, 0-ary, \
heterogeneous multi-arg)."]
fn remote_snapshot_hash_can_be_resumed_from_receiver_store() {
    let _guard = lock_process();
    let env = IsolatedEnv::new("shape-remote-snapshot-resume-e2e-");
    let receiver_store = env.snapshot_store("receiver-snapshots");
    let server = start_serve_with_snapshot_store("none", None, &[], &receiver_store);
    let program = r#"use std::core::remote
use std::core::snapshot

@remote("__ADDR__")
fn remote_snapshot_hash() -> string {
    match snapshot() {
        Ok(Snapshot::Hash(id)) => id
        Ok(Snapshot::Resumed) => "RESUMED"
        Err(e) => f"ERR={e}"
    }
}

print(f"REMOTE_SNAPSHOT={remote_snapshot_hash()}")
"#
    .replace("__ADDR__", &server.addr);

    let run = run_shape_program(&program, "vm", &env);
    assert_success(&run, "remote receiver snapshot producer");
    let hash = marker_value(&run.stdout, "REMOTE_SNAPSHOT=").expect("REMOTE_SNAPSHOT marker");
    assert_hexish(&hash, "remote receiver snapshot hash");

    let mut resume = shape_cmd();
    resume
        .arg("--snapshot-store")
        .arg(&receiver_store)
        .args(["--mode", "vm", "--resume", &hash])
        .timeout(Duration::from_secs(120));
    env.apply_assert_cmd(&mut resume);
    let resumed = resume.output().unwrap();
    let stdout = String::from_utf8_lossy(&resumed.stdout);
    assert!(
        resumed.status.success(),
        "receiver resume stderr={}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    assert!(
        stdout.contains("RESUMED"),
        "receiver resume should continue from snapshot() as resumed; stdout={stdout:?}"
    );
}
