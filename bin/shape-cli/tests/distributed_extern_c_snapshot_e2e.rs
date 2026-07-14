#[path = "support/distributed_snapshot_polyglot.rs"]
mod support;

use std::time::Duration;
use support::*;

#[test]
fn remote_extern_c_snapshot_hash_can_be_resumed_from_receiver_store() {
    let _guard = lock_process();
    let env = IsolatedEnv::new("shape-remote-c-snapshot-resume-e2e-");
    let receiver_store = env.snapshot_store("receiver-c-snapshots");
    let server = start_serve_with_snapshot_store("none", None, &[], &receiver_store);
    let program = r#"use std::core::remote
use std::core::snapshot

extern "C" fn labs(x: int) -> int from "c"

@remote("__ADDR__")
fn remote_abs_snapshot(x: int) -> string {
    let before = labs(x)
    match snapshot() {
        Ok(Snapshot::Hash(id)) => f"HASH:{id}"
        Ok(Snapshot::Resumed) => f"RESUMED:{before + labs(0 - 1)}"
        Err(e) => f"ERR:{e}"
    }
}

print(f"REMOTE_C_SNAPSHOT={remote_abs_snapshot(0 - 42)}")
"#
    .replace("__ADDR__", &server.addr);

    let run = run_shape_program(&program, "vm", &env);
    assert_success(&run, "remote extern C snapshot producer");
    let value = marker_value(&run.stdout, "REMOTE_C_SNAPSHOT=").expect("REMOTE_C_SNAPSHOT marker");
    let hash = value
        .strip_prefix("HASH:")
        .unwrap_or_else(|| panic!("remote extern C snapshot should save hash, got {value:?}"));
    assert_hexish(hash, "remote extern C receiver snapshot hash");

    let mut resume = shape_cmd();
    resume
        .arg("--snapshot-store")
        .arg(&receiver_store)
        .args(["--mode", "vm", "--resume", hash])
        .timeout(Duration::from_secs(120));
    env.apply_assert_cmd(&mut resume);
    let resumed = resume.output().unwrap();
    let stdout = String::from_utf8_lossy(&resumed.stdout);
    assert!(
        resumed.status.success(),
        "receiver extern C resume stderr={}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    assert!(
        stdout.contains("RESUMED:43"),
        "receiver extern C resume should restore pre-snapshot value and re-link C after resume; stdout={stdout:?}"
    );
}
