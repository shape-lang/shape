#[path = "support/distributed_snapshot_polyglot.rs"]
mod support;

use std::path::Path;
use std::process::Output;
use std::time::Duration;
use support::*;

#[test]
fn tls_remote_call_async_join_all_snapshots_land_in_receiver_store_only() {
    let _guard = lock_process();
    let env = IsolatedEnv::new("shape-distributed-proof-tls-async-snapshot-");
    let caller_store = env.snapshot_store("caller-snapshots");
    let receiver_store = env.snapshot_store("receiver-snapshots");
    let server = start_tls_serve_with_snapshot_store("none", None, &[], &receiver_store);
    let program = r#"use std::core::remote
use std::core::snapshot

fn remote_tagged_snapshot(seed: int, delta: int) -> string {
    let tag = seed + delta
    match snapshot() {
        Ok(Snapshot::Hash(id)) => f"HASH:{tag}:{id}"
        Ok(Snapshot::Resumed) => f"RESUMED:{tag}"
        Err(e) => f"ERR:{tag}:{e}"
    }
}

async fn run() {
    let results: Array<Result<string>> = await join all {
        remote::call_async("__TLS_ADDR__", remote_tagged_snapshot, 10, 1),
        remote::call_async("__TLS_ADDR__", remote_tagged_snapshot, 20, 2)
    }
    match results[0] {
        Ok(value) => print(f"TLS_ASYNC_SNAPSHOT_0={value}")
        Err(e) => print(f"TLS_ASYNC_SNAPSHOT_0_ERR={e}")
    }
    match results[1] {
        Ok(value) => print(f"TLS_ASYNC_SNAPSHOT_1={value}")
        Err(e) => print(f"TLS_ASYNC_SNAPSHOT_1_ERR={e}")
    }
}

await run()
"#
    .replace("__TLS_ADDR__", server.tls_addr());

    let run = run_shape_program_with_snapshot_store(&program, "vm", &env, &caller_store);
    assert_success(&run, "TLS remote::call_async join-all snapshot client");
    assert!(
        !run.stdout.contains("_ERR="),
        "join-all remote snapshots should return Ok values; stdout={:?} stderr={}",
        run.stdout,
        run.stderr
    );

    let first = marker_value(&run.stdout, "TLS_ASYNC_SNAPSHOT_0=")
        .expect("TLS_ASYNC_SNAPSHOT_0 marker");
    let second = marker_value(&run.stdout, "TLS_ASYNC_SNAPSHOT_1=")
        .expect("TLS_ASYNC_SNAPSHOT_1 marker");
    let first_hash = tagged_snapshot_hash(&first, "11", "first TLS async snapshot");
    let second_hash = tagged_snapshot_hash(&second, "22", "second TLS async snapshot");
    assert_ne!(
        first_hash, second_hash,
        "different remote snapshot states should not collapse to the same hash"
    );

    assert_hash_store_visibility(&env, &receiver_store, &caller_store, &first_hash);
    assert_hash_store_visibility(&env, &receiver_store, &caller_store, &second_hash);
}

fn tagged_snapshot_hash(value: &str, expected_tag: &str, context: &str) -> String {
    let rest = value
        .strip_prefix("HASH:")
        .unwrap_or_else(|| panic!("{context} should return HASH:<tag>:<hash>, got {value:?}"));
    let (tag, hash) = rest
        .split_once(':')
        .unwrap_or_else(|| panic!("{context} should include tag and hash, got {value:?}"));
    assert_eq!(
        tag, expected_tag,
        "{context} should preserve join-all source order"
    );
    assert_hexish(hash, context);
    hash.to_string()
}

fn assert_hash_store_visibility(
    env: &IsolatedEnv,
    receiver_store: &Path,
    caller_store: &Path,
    hash: &str,
) {
    let receiver_info = snapshot_info(env, receiver_store, hash);
    assert!(
        receiver_info.status.success(),
        "receiver store must contain TLS async remote snapshot {hash}; stderr={}",
        String::from_utf8_lossy(&receiver_info.stderr)
    );

    let caller_info = snapshot_info(env, caller_store, hash);
    assert!(
        !caller_info.status.success(),
        "caller store must not contain receiver-owned snapshot hash {hash}; stdout={} stderr={}",
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
