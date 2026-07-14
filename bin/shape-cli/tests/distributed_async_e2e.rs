#[path = "support/distributed_snapshot_polyglot.rs"]
mod support;

use support::*;

#[test]
fn remote_call_async_awaits_result_over_shape_serve() {
    let _guard = lock_process();
    let server = start_serve("none", None, &[]);
    let env = IsolatedEnv::new("shape-remote-call-async-e2e-");
    let program = r#"use std::core::remote
fn mul(a: int, b: int) -> int { a * b }

async fn run() {
    let r = await remote::call_async("__ADDR__", mul, 6, 7)
    match r {
        Ok(v) => print(f"REMOTE_CALL_ASYNC_OK={v}")
        Err(_) => print("REMOTE_CALL_ASYNC_ERR")
    }
}

await run()
"#
    .replace("__ADDR__", &server.addr);
    let run = run_shape_program(&program, "vm", &env);
    assert_success(&run, "remote::call_async await client");
    assert!(
        run.stdout.contains("REMOTE_CALL_ASYNC_OK=42")
            && !run.stdout.contains("REMOTE_CALL_ASYNC_ERR"),
        "remote::call_async should await to Ok(42); stdout={:?} stderr={}",
        run.stdout,
        run.stderr
    );
}

#[test]
fn remote_call_async_transport_err_is_inner_result() {
    let _guard = lock_process();
    let env = IsolatedEnv::new("shape-remote-call-async-err-e2e-");
    let dead_addr = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().to_string()
    };
    let program = r#"use std::core::remote
fn mul(a: int, b: int) -> int { a * b }

async fn run() {
    let r = await remote::call_async("__DEAD__", mul, 6, 7)
    match r {
        Ok(v) => print(f"REMOTE_CALL_ASYNC_UNEXPECTED_OK={v}")
        Err(_) => print("REMOTE_CALL_ASYNC_TRANSPORT_ERR")
    }
}

await run()
"#
    .replace("__DEAD__", &dead_addr);
    let run = run_shape_program(&program, "vm", &env);
    assert_success(&run, "remote::call_async transport error client");
    assert!(
        run.stdout.contains("REMOTE_CALL_ASYNC_TRANSPORT_ERR")
            && !run.stdout.contains("REMOTE_CALL_ASYNC_UNEXPECTED_OK"),
        "remote::call_async transport failure should be Err inside the awaited Result; stdout={:?} stderr={}",
        run.stdout,
        run.stderr
    );
}

#[test]
fn remote_call_async_receiver_snapshot_returns_hash() {
    let _guard = lock_process();
    let server = start_serve("none", None, &[]);
    let env = IsolatedEnv::new("shape-remote-call-async-snapshot-e2e-");
    let program = r#"use std::core::remote
use std::core::snapshot

fn remote_snapshot_hash() -> string {
    match snapshot() {
        Ok(Snapshot::Hash(id)) => id
        Ok(Snapshot::Resumed) => "RESUMED"
        Err(e) => f"ERR={e}"
    }
}

async fn run() {
    let r = await remote::call_async("__ADDR__", remote_snapshot_hash)
    match r {
        Ok(hash) => print(f"REMOTE_CALL_ASYNC_SNAPSHOT={hash}")
        Err(_) => print("REMOTE_CALL_ASYNC_SNAPSHOT_ERR")
    }
}

await run()
"#
    .replace("__ADDR__", &server.addr);
    let run = run_shape_program(&program, "vm", &env);
    assert_success(&run, "remote::call_async receiver snapshot client");
    let hash = marker_value(&run.stdout, "REMOTE_CALL_ASYNC_SNAPSHOT=")
        .expect("REMOTE_CALL_ASYNC_SNAPSHOT marker");
    assert!(
        hash != "RESUMED" && !hash.starts_with("ERR="),
        "remote snapshot should save on receiver, not resume or hit barrier; stdout={:?} stderr={}",
        run.stdout,
        run.stderr
    );
    assert_hexish(&hash, "remote::call_async receiver snapshot hash");
}

#[test]
fn remote_call_async_two_calls_compose_after_awaiting() {
    let _guard = lock_process();
    let server = start_serve("none", None, &[]);
    let env = IsolatedEnv::new("shape-remote-call-async-compose-e2e-");
    let program = r#"use std::core::remote
fn add(a: int, b: int) -> int { a + b }
fn mul(a: int, b: int) -> int { a * b }

async fn run() {
    let left = remote::call_async("__ADDR__", add, 10, 5)
    let right = remote::call_async("__ADDR__", mul, 6, 7)
    let a = await left
    let b = await right
    let x: int = match a { Ok(v) => v, Err(_) => 0 - 1000 }
    let y: int = match b { Ok(v) => v, Err(_) => 0 - 1000 }
    print(f"REMOTE_CALL_ASYNC_COMPOSE={x + y}")
}

await run()
"#
    .replace("__ADDR__", &server.addr);
    let run = run_shape_program(&program, "vm", &env);
    assert_success(&run, "remote::call_async composed awaits client");
    assert!(
        run.stdout.contains("REMOTE_CALL_ASYNC_COMPOSE=57")
            && !run.stdout.contains("REMOTE_CALL_ASYNC_COMPOSE_ERR"),
        "remote::call_async composed awaits should add remote results; stdout={:?} stderr={}",
        run.stdout,
        run.stderr
    );
}

#[test]
fn remote_call_async_join_all_materializes_ordered_results() {
    let _guard = lock_process();
    let server = start_serve("none", None, &[]);
    let env = IsolatedEnv::new("shape-remote-call-async-join-all-e2e-");
    let program = r#"use std::core::remote
fn add(a: int, b: int) -> int { a + b }
fn mul(a: int, b: int) -> int { a * b }

async fn run() {
    let results: Array<Result<int>> = await join all {
        remote::call_async("__ADDR__", add, 10, 5),
        remote::call_async("__ADDR__", mul, 6, 7)
    }
    let x: int = match results[0] { Ok(v) => v, Err(_) => 0 - 1000 }
    let y: int = match results[1] { Ok(v) => v, Err(_) => 0 - 1000 }
    print(f"REMOTE_CALL_ASYNC_JOIN_ALL={x + y}")
}

await run()
"#
    .replace("__ADDR__", &server.addr);
    let run = run_shape_program(&program, "vm", &env);
    assert_success(&run, "remote::call_async join all client");
    assert!(
        run.stdout.contains("REMOTE_CALL_ASYNC_JOIN_ALL=57"),
        "remote::call_async join all should materialize ordered Result values; stdout={:?} stderr={}",
        run.stdout,
        run.stderr
    );
}

#[test]
fn remote_call_callee_returned_future_materializes_payload() {
    let _guard = lock_process();
    let server = start_serve("none", None, &[]);
    let env = IsolatedEnv::new("shape-remote-callee-future-call-e2e-");
    let program = r#"use std::core::remote

async fn remote_future() -> Future<int> {
    async let value = 42
    value
}

match remote::call("__ADDR__", remote_future) {
    Ok(v) => print(f"REMOTE_CALL_FUTURE_OK={v}")
    Err(_) => print("REMOTE_CALL_FUTURE_ERR")
}
"#
    .replace("__ADDR__", &server.addr);
    let run = run_shape_program(&program, "vm", &env);
    assert_success(&run, "remote::call callee Future<T> client");
    assert!(
        run.stdout.contains("REMOTE_CALL_FUTURE_OK=42")
            && !run.stdout.contains("REMOTE_CALL_FUTURE_ERR"),
        "remote::call should materialize a receiver-local Future<T> payload; stdout={:?} stderr={}",
        run.stdout,
        run.stderr
    );
}

#[test]
fn remote_call_async_callee_returned_future_materializes_payload() {
    let _guard = lock_process();
    let server = start_serve("none", None, &[]);
    let env = IsolatedEnv::new("shape-remote-callee-future-call-async-e2e-");
    let program = r#"use std::core::remote

async fn remote_future() -> Future<int> {
    async let value = 42
    value
}

async fn run() {
    let r = await remote::call_async("__ADDR__", remote_future)
    match r {
        Ok(v) => print(f"REMOTE_CALL_ASYNC_FUTURE_OK={v}")
        Err(_) => print("REMOTE_CALL_ASYNC_FUTURE_ERR")
    }
}

await run()
"#
    .replace("__ADDR__", &server.addr);
    let run = run_shape_program(&program, "vm", &env);
    assert_success(&run, "remote::call_async callee Future<T> client");
    assert!(
        run.stdout.contains("REMOTE_CALL_ASYNC_FUTURE_OK=42")
            && !run.stdout.contains("REMOTE_CALL_ASYNC_FUTURE_ERR"),
        "remote::call_async should materialize a receiver-local Future<T> payload; stdout={:?} stderr={}",
        run.stdout,
        run.stderr
    );
}

#[test]
fn remote_call_async_join_all_callee_returned_futures() {
    let _guard = lock_process();
    let server = start_serve("none", None, &[]);
    let env = IsolatedEnv::new("shape-remote-callee-future-join-all-e2e-");
    let program = r#"use std::core::remote

async fn future_add() -> Future<int> {
    async let value = 10 + 5
    value
}

async fn future_mul() -> Future<int> {
    async let value = 6 * 7
    value
}

async fn run() {
    let results: Array<Result<int>> = await join all {
        remote::call_async("__ADDR__", future_add),
        remote::call_async("__ADDR__", future_mul)
    }
    let x: int = match results[0] { Ok(v) => v, Err(_) => 0 - 1000 }
    let y: int = match results[1] { Ok(v) => v, Err(_) => 0 - 1000 }
    print(f"REMOTE_CALL_ASYNC_FUTURE_JOIN_ALL={x + y}")
}

await run()
"#
    .replace("__ADDR__", &server.addr);
    let run = run_shape_program(&program, "vm", &env);
    assert_success(&run, "remote::call_async callee Future<T> join all client");
    assert!(
        run.stdout.contains("REMOTE_CALL_ASYNC_FUTURE_JOIN_ALL=57"),
        "join all should materialize ordered Result<T> values when remote callees return Future<T>; stdout={:?} stderr={}",
        run.stdout,
        run.stderr
    );
}

#[test]
fn remote_call_async_live_future_checkpoint_barrier_then_awaits() {
    let _guard = lock_process();
    let server = start_serve("none", None, &[]);
    let env = IsolatedEnv::new("shape-remote-call-async-snapshot-barrier-e2e-");
    let store = env.snapshot_store("future-barrier-snapshots");
    let program = r#"use std::core::remote
use std::core::snapshot

fn mul(a: int, b: int) -> int { a * b }

async fn run() {
    let pending = remote::call_async("__ADDR__", mul, 6, 7)
    match snapshot() {
        Ok(Snapshot::Hash(id)) => print(f"FUTURE_SNAPSHOT_UNEXPECTED_HASH={id}")
        Ok(Snapshot::Resumed) => print("FUTURE_SNAPSHOT_UNEXPECTED_RESUMED")
        Err(e) => print(f"FUTURE_SNAPSHOT_ERR={e}")
    }
    let r = await pending
    match r {
        Ok(v) => print(f"FUTURE_AFTER_BARRIER={v}")
        Err(_) => print("FUTURE_AFTER_BARRIER_ERR")
    }
}

await run()
"#
    .replace("__ADDR__", &server.addr);
    let run = run_shape_program_with_snapshot_store(&program, "vm", &env, &store);
    assert_success(
        &run,
        "remote::call_async live future snapshot barrier client",
    );
    assert!(
        run.stdout.contains("FUTURE_SNAPSHOT_ERR=")
            && run.stdout.contains("Future(")
            && run.stdout.contains("resumable futures are not implemented"),
        "snapshot() should reject the live remote Future with an explicit diagnostic; stdout={:?} stderr={}",
        run.stdout,
        run.stderr
    );
    assert!(
        run.stdout.contains("FUTURE_AFTER_BARRIER=42")
            && !run.stdout.contains("FUTURE_AFTER_BARRIER_ERR")
            && !run.stdout.contains("FUTURE_SNAPSHOT_UNEXPECTED_HASH")
            && !run.stdout.contains("FUTURE_SNAPSHOT_UNEXPECTED_RESUMED"),
        "barrier should not persist or cancel the remote future; stdout={:?} stderr={}",
        run.stdout,
        run.stderr
    );
}
