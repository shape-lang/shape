#[path = "support/distributed_snapshot_polyglot.rs"]
mod support;

use std::time::Duration;
use support::*;

#[test]
#[ignore = "dark window: E4 re-implements @remote on typed HookDecision — see issue #68"]
fn remote_python_snapshot_hash_can_be_resumed_from_receiver_store() {
    remote_dynamic_snapshot_resume(
        "python",
        "padd",
        "return a + b",
        "REMOTE_PY_SNAPSHOT",
        100,
        5,
        15,
        "RESUMED:120",
    );
}

#[test]
#[ignore = "dark window: E4 re-implements @remote on typed HookDecision — see issue #68"]
fn remote_typescript_snapshot_hash_can_be_resumed_from_receiver_store() {
    remote_dynamic_snapshot_resume(
        "typescript",
        "tadd",
        "return a + b;",
        "REMOTE_TS_SNAPSHOT",
        20,
        1,
        9,
        "RESUMED:30",
    );
}

fn remote_dynamic_snapshot_resume(
    language: &str,
    function_name: &str,
    body: &str,
    marker: &str,
    input: i64,
    before_delta: i64,
    after_delta: i64,
    expected_resume: &str,
) {
    let _guard = lock_process();
    let Some(so) = require_language_ext_so(language) else {
        println!(
            "SKIP remote_{language}_snapshot_resume: libshape_ext_{language}.so not found; \
             build extensions, set SHAPE_FFI_EXT_DIR, or set SHAPE_REQUIRE_FFI_EXT=1 to require it"
        );
        return;
    };

    let env = IsolatedEnv::new("shape-remote-dynamic-snapshot-resume-e2e-");
    let receiver_store = env.snapshot_store("receiver-dynamic-snapshots");
    let server = start_serve_with_snapshot_store("none", Some(&so), &[language], &receiver_store);
    let program = format!(
        r#"use std::core::remote
use std::core::snapshot

fn {language} {function_name}(a: int, b: int) -> Result<int> {{
    {body}
}}

@remote("{addr}")
fn remote_language_snapshot(x: int) -> string {{
    let before = match {function_name}(x, {before_delta}) {{ Ok(v) => v, Err(e) => 0 - 1 }}
    match snapshot() {{
        Ok(Snapshot::Hash(id)) => f"HASH:{{id}}"
        Ok(Snapshot::Resumed) => {{
            let after = match {function_name}(before, {after_delta}) {{ Ok(v) => v, Err(e) => 0 - 1 }}
            f"RESUMED:{{after}}"
        }}
        Err(e) => f"ERR:{{e}}"
    }}
}}

print(f"{marker}={{remote_language_snapshot({input})}}")
"#,
        addr = server.addr
    );

    let run = run_shape_program(&program, "vm", &env);
    assert_success(&run, &format!("remote {language} snapshot producer"));
    assert_serve_logged_foreign_stub(&server, language);
    let value = marker_value(&run.stdout, &format!("{marker}="))
        .unwrap_or_else(|| panic!("{marker} marker missing from stdout {:?}", run.stdout));
    let hash = value
        .strip_prefix("HASH:")
        .unwrap_or_else(|| panic!("remote {language} snapshot should save hash, got {value:?}"));
    assert_hexish(hash, &format!("remote {language} receiver snapshot hash"));

    let mut resume = shape_cmd();
    resume
        .arg("--snapshot-store")
        .arg(&receiver_store)
        .arg("--extension")
        .arg(&so)
        .args(["--mode", "vm", "--resume", hash])
        .timeout(Duration::from_secs(120));
    env.apply_assert_cmd(&mut resume);
    let resumed = resume.output().unwrap();
    let stdout = String::from_utf8_lossy(&resumed.stdout);
    assert!(
        resumed.status.success(),
        "receiver {language} resume stderr={}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    assert!(
        stdout.contains(expected_resume),
        "receiver {language} resume should restore the pre-snapshot value and re-link the runtime after resume; stdout={stdout:?}"
    );
}
