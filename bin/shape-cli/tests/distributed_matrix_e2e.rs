#[path = "support/distributed_snapshot_polyglot.rs"]
mod support;

use std::path::Path;
use std::process::Output;
use std::time::Duration;
use support::*;

#[test]
fn tls_remote_call_refuses_missing_ca_trust_root() {
    let _guard = lock_process();
    let server = start_tls_serve("none", None, &[]);
    let env = IsolatedEnv::new("shape-distributed-matrix-tls-no-ca-");
    let no_ca_addr = format!("shape+tls://{}?server_name=localhost", server.addr);
    let program = r#"use std::core::remote
fn mul(a: int, b: int) -> int { a * b }

match remote::call("__NO_CA_ADDR__", mul, 6, 7) {
    Ok(value) => print(f"TLS_NO_CA_UNEXPECTED_OK={value}")
    Err(e) => print(f"TLS_NO_CA_ERR={e}")
}
"#
    .replace("__NO_CA_ADDR__", &no_ca_addr);

    let run = run_shape_program(&program, "vm", &env);
    assert_success(&run, "TLS missing CA refusal client");
    assert!(
        run.stdout.contains("TLS_NO_CA_ERR=")
            && run.stdout.contains("ca=/path/to/cert.pem")
            && !run.stdout.contains("TLS_NO_CA_UNEXPECTED_OK"),
        "TLS address without ca= must be a user-visible Err; stdout={:?} stderr={}",
        run.stdout,
        run.stderr
    );
}

#[test]
fn tls_remote_call_refuses_mismatched_server_name() {
    let _guard = lock_process();
    let server = start_tls_serve("none", None, &[]);
    let env = IsolatedEnv::new("shape-distributed-matrix-tls-name-");
    let wrong_name_addr = server
        .tls_addr()
        .replace("server_name=localhost", "server_name=wrong.localhost");
    let program = r#"use std::core::remote
fn mul(a: int, b: int) -> int { a * b }

match remote::call("__WRONG_NAME_ADDR__", mul, 6, 7) {
    Ok(value) => print(f"TLS_WRONG_NAME_UNEXPECTED_OK={value}")
    Err(e) => print(f"TLS_WRONG_NAME_ERR={e}")
}
"#
    .replace("__WRONG_NAME_ADDR__", &wrong_name_addr);

    let run = run_shape_program(&program, "vm", &env);
    assert_success(&run, "TLS wrong server_name refusal client");
    assert!(
        run.stdout.contains("TLS_WRONG_NAME_ERR=")
            && !run.stdout.contains("TLS_WRONG_NAME_UNEXPECTED_OK"),
        "TLS address with a mismatched server_name must be a user-visible Err; stdout={:?} stderr={}",
        run.stdout,
        run.stderr
    );
}

#[test]
#[ignore = "0-ary @remote unsupported: the `@remote fn remote_snapshot_hash() -> string` \
target (see assert_remote_snapshot_store_isolation) declares no parameters, so the decision \
`before` hook has no argument to receive/short-circuit and is loud-rejected (\"the target \
declares no parameters\"). Capability tracked in #83 (E4-S5 @remote residual: async, 0-ary, \
heterogeneous multi-arg)."]
fn plaintext_remote_snapshot_uses_receiver_store_not_caller_store() {
    let _guard = lock_process();
    assert_remote_snapshot_store_isolation(false);
}

#[test]
#[ignore = "0-ary @remote unsupported: the `@remote fn remote_snapshot_hash() -> string` \
target (see assert_remote_snapshot_store_isolation) declares no parameters, so the decision \
`before` hook has no argument to receive/short-circuit and is loud-rejected (\"the target \
declares no parameters\"). TLS transport itself is fine; only the 0-ary shape blocks. \
Capability tracked in #83 (E4-S5 @remote residual: async, 0-ary, heterogeneous multi-arg)."]
fn tls_remote_snapshot_uses_receiver_store_not_caller_store() {
    let _guard = lock_process();
    assert_remote_snapshot_store_isolation(true);
}

#[test]
fn remote_python_call_refuses_receiver_without_language_opt_in() {
    let _guard = lock_process();
    assert_dynamic_remote_call_refuses_receiver_without_language_opt_in(
        "python",
        "py_add_for_refusal",
        "return x + 5",
        "PY_DYNAMIC_REFUSAL_UNEXPECTED_OK",
        100,
    );
}

#[test]
fn remote_typescript_call_refuses_receiver_without_language_opt_in() {
    let _guard = lock_process();
    assert_dynamic_remote_call_refuses_receiver_without_language_opt_in(
        "typescript",
        "ts_add_for_refusal",
        "return x + 1;",
        "TS_DYNAMIC_REFUSAL_UNEXPECTED_OK",
        20,
    );
}

fn assert_dynamic_remote_call_refuses_receiver_without_language_opt_in(
    language: &str,
    function_name: &str,
    body: &str,
    unexpected_marker: &str,
    input: i64,
) {
    let env = IsolatedEnv::new("shape-distributed-matrix-dynamic-refusal-");
    let server = start_serve("none", None, &[]);
    let program = format!(
        r#"from std::core::remote use {{ @remote }}
fn {language} {function_name}(x: int) -> Result<int> {{
    {body}
}}

@remote("{addr}")
fn remote_dynamic_add(x: int) -> int {{
    match {function_name}(x) {{
        Ok(v) => v
        Err(e) => 0 - 1
    }}
}}

print(f"{unexpected_marker}={{remote_dynamic_add({input})}}")
"#,
        addr = server.addr
    );

    let run = run_shape_program(&program, "vm", &env);
    let msg = combined(&run);
    assert!(
        run.code != Some(0),
        "{language} dynamic remote call should fail without receiver opt-in; stdout={:?} stderr={}",
        run.stdout,
        run.stderr
    );
    assert!(
        !msg.contains(unexpected_marker)
            && msg.contains(&format!(
                "server has not opted into the '{language}' language runtime"
            ))
            && msg.contains(&format!("--ffi-languages {language}"))
            && !msg.contains("no extension provides language"),
        "{language} dynamic remote call must refuse at the receiver opt-in gate before runtime lookup; output={msg:?}"
    );
    assert_serve_logged_foreign_stub(&server, language);
}

fn assert_remote_snapshot_store_isolation(tls: bool) {
    let prefix = if tls {
        "shape-distributed-matrix-tls-store-"
    } else {
        "shape-distributed-matrix-plain-store-"
    };
    let env = IsolatedEnv::new(prefix);
    let caller_store = env.snapshot_store("caller-snapshots");
    let receiver_store = env.snapshot_store("receiver-snapshots");
    let server = if tls {
        start_tls_serve_with_snapshot_store("none", None, &[], &receiver_store)
    } else {
        start_serve_with_snapshot_store("none", None, &[], &receiver_store)
    };
    let addr = if tls {
        server.tls_addr().to_string()
    } else {
        server.addr.clone()
    };
    let marker = if tls {
        "TLS_STORE_MATRIX_HASH"
    } else {
        "PLAIN_STORE_MATRIX_HASH"
    };
    let program = format!(
        r#"use std::core::remote
use std::core::snapshot

@remote("{addr}")
fn remote_snapshot_hash() -> string {{
    match snapshot() {{
        Ok(Snapshot::Hash(id)) => id
        Ok(Snapshot::Resumed) => "RESUMED"
        Err(e) => f"ERR={{e}}"
    }}
}}

print(f"{marker}={{remote_snapshot_hash()}}")
"#
    );

    let run = run_shape_program_with_snapshot_store(&program, "vm", &env, &caller_store);
    assert_success(&run, "remote snapshot store isolation client");
    let hash = marker_value(&run.stdout, &format!("{marker}="))
        .unwrap_or_else(|| panic!("{marker} marker missing from stdout {:?}", run.stdout));
    assert!(
        hash != "RESUMED" && !hash.starts_with("ERR="),
        "remote snapshot should save on receiver; stdout={:?} stderr={}",
        run.stdout,
        run.stderr
    );
    assert_hexish(&hash, "remote receiver snapshot hash");

    let receiver_info = snapshot_info(&env, &receiver_store, &hash);
    assert!(
        receiver_info.status.success(),
        "receiver store must contain the remote snapshot; stderr={}",
        String::from_utf8_lossy(&receiver_info.stderr)
    );

    let caller_info = snapshot_info(&env, &caller_store, &hash);
    assert!(
        !caller_info.status.success(),
        "caller store must not contain receiver-owned snapshot hash; stdout={} stderr={}",
        String::from_utf8_lossy(&caller_info.stdout),
        String::from_utf8_lossy(&caller_info.stderr)
    );
}

fn snapshot_info(env: &IsolatedEnv, store: &Path, hash: &str) -> Output {
    let mut cmd = shape_cmd();
    cmd.arg("--snapshot-store")
        .arg(store)
        .args(["snapshot", "info", hash])
        .timeout(Duration::from_secs(30));
    env.apply_assert_cmd(&mut cmd);
    cmd.output().unwrap()
}
