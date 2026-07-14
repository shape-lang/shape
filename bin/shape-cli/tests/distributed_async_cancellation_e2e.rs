#[path = "support/distributed_snapshot_polyglot.rs"]
mod support;

use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use support::*;

fn timed_shape_program(program: &str, mode: &str, env: &IsolatedEnv) -> (Run, Duration) {
    let start = Instant::now();
    let run = run_shape_program(program, mode, env);
    let elapsed = start.elapsed();
    (run, elapsed)
}

fn awaited_slow_remote_elapsed(addr: &str, env: &IsolatedEnv) -> Duration {
    let program = r#"use std::core::remote
use std::core::time

fn slow_remote_control(ms: float) -> int {
    time::sleep_sync(ms)
    31
}

async fn run() {
    let pending = remote::call_async("__ADDR__", slow_remote_control, 1500.0)
    let result = await pending
    match result {
        Ok(v) => print(f"AWAITED_REMOTE={v}")
        Err(_) => print("AWAITED_REMOTE_ERR")
    }
}

await run()
"#
    .replace("__ADDR__", addr);

    let (run, elapsed) = timed_shape_program(&program, "vm", env);
    assert_success(&run, "remote::call_async awaited slow control");
    assert!(
        run.stdout.contains("AWAITED_REMOTE=31"),
        "awaited control should complete normally; stdout={:?} stderr={}",
        run.stdout,
        run.stderr
    );
    elapsed
}

fn assert_substantially_faster_than_awaiting(
    cancelled_elapsed: Duration,
    awaited_elapsed: Duration,
    context: &str,
    stdout: &str,
    stderr: &str,
    serve_log: &str,
) {
    assert!(
        cancelled_elapsed + Duration::from_millis(700) < awaited_elapsed,
        "{context} should not wait for the 1500ms remote call; cancelled={cancelled_elapsed:?} awaited-control={awaited_elapsed:?} stdout={stdout:?} stderr={stderr} serve stderr:\n{serve_log}"
    );
}

struct TlsBlackhole {
    addr: String,
    tls_addr: String,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
    _dir: tempfile::TempDir,
}

impl TlsBlackhole {
    fn tls_addr(&self) -> &str {
        &self.tls_addr
    }
}

impl Drop for TlsBlackhole {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(&self.addr);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn start_tls_blackhole(prefix: &str) -> TlsBlackhole {
    let dir = tempfile::Builder::new().prefix(prefix).tempdir().unwrap();
    let generated = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
    let cert_path = dir.path().join("cert.pem");
    std::fs::write(&cert_path, generated.cert.pem()).unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let tls_addr = format!(
        "shape+tls://{}?ca={}&server_name=localhost",
        addr,
        cert_path.display()
    );
    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = Arc::clone(&stop);
    let handle = thread::spawn(move || {
        let mut held_connections = Vec::new();
        while !worker_stop.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((stream, _)) => held_connections.push(stream),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(_) => break,
            }
        }
    });

    TlsBlackhole {
        addr,
        tls_addr,
        stop,
        handle: Some(handle),
        _dir: dir,
    }
}

fn scope_cancel_returns_promptly_for(
    addr: &str,
    server: &ServeNode,
    env_prefix: &str,
    context: &str,
) {
    let env = IsolatedEnv::new(env_prefix);
    let awaited_elapsed = awaited_slow_remote_elapsed(addr, &env);
    let program = r#"use std::core::remote
use std::core::time

fn slow_remote(ms: float) -> int {
    time::sleep_sync(ms)
    9
}

async fn run() {
    let result = async scope {
        let slow = remote::call_async("__ADDR__", slow_remote, 1500.0)
        await time::sleep(100.0)
        7
    }
    await time::sleep(100.0)
    print(f"SCOPE_CANCEL_RESULT={result}")
}

await run()
"#
    .replace("__ADDR__", addr);

    let (run, elapsed) = timed_shape_program(&program, "vm", &env);
    let label = format!("{context} remote::call_async scope cancellation client");
    assert_success(&run, &label);
    assert!(
        run.stdout.contains("SCOPE_CANCEL_RESULT=7"),
        "{context}: scope body should return without awaiting the cancelled remote future; stdout={:?} stderr={}",
        run.stdout,
        run.stderr
    );
    let serve_log = serve_stderr(server);
    assert!(
        serve_log.contains("CancelCall"),
        "{context}: scope exit should trigger the remote cancellation hook; serve stderr:\n{serve_log}"
    );
    assert_substantially_faster_than_awaiting(
        elapsed,
        awaited_elapsed,
        context,
        &run.stdout,
        &run.stderr,
        &serve_log,
    );
}

fn race_cancels_slow_loser_for(
    addr: &str,
    server: &ServeNode,
    env_prefix: &str,
    context: &str,
) {
    let env = IsolatedEnv::new(env_prefix);
    let awaited_elapsed = awaited_slow_remote_elapsed(addr, &env);
    let program = r#"use std::core::remote
use std::core::time

fn slow_remote(ms: float) -> int {
    time::sleep_sync(ms)
    2
}

fn fast_remote() -> int {
    1
}

async fn run() {
    let winner = await join race {
        remote::call_async("__ADDR__", fast_remote),
        remote::call_async("__ADDR__", slow_remote, 1500.0)
    }
    match winner {
        Ok(v) => print(f"RACE_WINNER={v}")
        Err(_) => print("RACE_WINNER_ERR")
    }
    await time::sleep(100.0)
}

await run()
"#
    .replace("__ADDR__", addr);

    let (run, elapsed) = timed_shape_program(&program, "vm", &env);
    let label = format!("{context} remote::call_async race cancellation client");
    assert_success(&run, &label);
    assert!(
        run.stdout.contains("RACE_WINNER=1") && !run.stdout.contains("RACE_WINNER_ERR"),
        "{context}: race should return the fast remote result; stdout={:?} stderr={}",
        run.stdout,
        run.stderr
    );
    std::thread::sleep(Duration::from_millis(100));
    let serve_log = serve_stderr(server);
    assert!(
        serve_log.contains("CancelCall"),
        "{context}: race loser should trigger the remote cancellation hook; serve stderr:\n{serve_log}"
    );
    assert_substantially_faster_than_awaiting(
        elapsed,
        awaited_elapsed,
        context,
        &run.stdout,
        &run.stderr,
        &serve_log,
    );
}

fn cancel_queued_call_is_receiver_visible_for(
    addr: &str,
    server: &ServeNode,
    env_prefix: &str,
    context: &str,
) {
    let env = IsolatedEnv::new(env_prefix);
    let program = r#"use std::core::remote
use std::core::time

fn slow_remote(ms: float) -> int {
    time::sleep_sync(ms)
    1
}

fn queued_remote() -> int {
    print("QUEUED_REMOTE_EXECUTED")
    2
}

async fn run() {
    let first = remote::call_async("__ADDR__", slow_remote, 1200.0)
    await time::sleep(100.0)
    let scoped = async scope {
        let queued = remote::call_async("__ADDR__", queued_remote)
        77
    }
    let first_result = await first
    match first_result {
        Ok(v) => print(f"FIRST_REMOTE_DONE={v}")
        Err(_) => print("FIRST_REMOTE_ERR")
    }
    print(f"QUEUED_SCOPE_RESULT={scoped}")
}

await run()
"#
    .replace("__ADDR__", addr);

    let run = run_shape_program(&program, "vm", &env);
    let label = format!("{context} remote::call_async queued cancellation client");
    assert_success(&run, &label);
    assert!(
        run.stdout.contains("FIRST_REMOTE_DONE=1")
            && run.stdout.contains("QUEUED_SCOPE_RESULT=77"),
        "{context}: first call should complete while queued scoped call is cancelled; stdout={:?} stderr={}",
        run.stdout,
        run.stderr
    );
    std::thread::sleep(Duration::from_millis(150));
    let serve_log = serve_stderr(server);
    assert!(
        serve_log.contains("outcome=AcceptedQueued")
            && serve_log.contains("before receiver execution"),
        "{context}: receiver should record queued cancellation instead of executing the queued call; serve stderr:\n{serve_log}"
    );
}

fn cancel_running_call_is_honest_for(
    addr: &str,
    server: &ServeNode,
    env_prefix: &str,
    context: &str,
) {
    let env = IsolatedEnv::new(env_prefix);
    let program = r#"use std::core::remote
use std::core::time

fn slow_remote(ms: float) -> int {
    time::sleep_sync(ms)
    5
}

async fn run() {
    let result = async scope {
        let slow = remote::call_async("__ADDR__", slow_remote, 900.0)
        await time::sleep(150.0)
        13
    }
    await time::sleep(100.0)
    print(f"RUNNING_CANCEL_SCOPE_RESULT={result}")
}

await run()
"#
    .replace("__ADDR__", addr);

    let run = run_shape_program(&program, "vm", &env);
    let label = format!("{context} remote::call_async running cancellation client");
    assert_success(&run, &label);
    assert!(
        run.stdout.contains("RUNNING_CANCEL_SCOPE_RESULT=13"),
        "{context}: scope should return its body value after cancelling the remote future; stdout={:?} stderr={}",
        run.stdout,
        run.stderr
    );
    std::thread::sleep(Duration::from_millis(150));
    let serve_log = serve_stderr(server);
    assert!(
        serve_log.contains("outcome=AlreadyRunning")
            && serve_log.contains("not preemptible"),
        "{context}: running receiver call must report honest non-preemptibility; serve stderr:\n{serve_log}"
    );
}

#[test]
#[ignore = "timing-sensitive distributed cancellation proof; run serialized under the supervisor cgroup lane"]
fn remote_call_async_scope_cancel_returns_promptly() {
    let _guard = lock_process();
    let server = start_serve("none", None, &[]);
    scope_cancel_returns_promptly_for(
        &server.addr,
        &server,
        "shape-remote-call-async-scope-cancel-",
        "TCP scope exit",
    );
}

#[test]
#[ignore = "timing-sensitive distributed cancellation proof; run serialized under the supervisor cgroup lane"]
fn tls_remote_call_async_scope_cancel_returns_promptly() {
    let _guard = lock_process();
    let server = start_tls_serve("none", None, &[]);
    scope_cancel_returns_promptly_for(
        server.tls_addr(),
        &server,
        "shape-remote-call-async-tls-scope-cancel-",
        "TLS scope exit",
    );
}

#[test]
#[ignore = "timing-sensitive distributed cancellation proof; run serialized under the supervisor cgroup lane"]
fn tls_remote_call_async_scope_cancel_during_handshake_returns_promptly() {
    let _guard = lock_process();
    let blackhole = start_tls_blackhole("shape-remote-call-async-tls-blackhole-");
    let env = IsolatedEnv::new("shape-remote-call-async-tls-blackhole-client-");
    let program = r#"use std::core::remote
use std::core::time

fn unreachable_remote() -> int {
    99
}

async fn run() {
    let result = async scope {
        let pending = remote::call_async("__ADDR__", unreachable_remote)
        await time::sleep(100.0)
        23
    }
    print(f"TLS_BLACKHOLE_SCOPE_RESULT={result}")
}

await run()
"#
    .replace("__ADDR__", blackhole.tls_addr());

    let (run, elapsed) = timed_shape_program(&program, "vm", &env);
    assert_success(&run, "TLS blackhole handshake cancellation client");
    assert!(
        run.stdout.contains("TLS_BLACKHOLE_SCOPE_RESULT=23"),
        "TLS blackhole scope should return its body value; stdout={:?} stderr={}",
        run.stdout,
        run.stderr
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "TLS blackhole cancellation should not wait for the handshake timeout; elapsed={elapsed:?} stdout={:?} stderr={}",
        run.stdout,
        run.stderr
    );
}

#[test]
#[ignore = "timing-sensitive distributed cancellation proof; run serialized under the supervisor cgroup lane"]
fn remote_call_async_race_cancels_slow_loser() {
    let _guard = lock_process();
    let server = start_serve("none", None, &[]);
    race_cancels_slow_loser_for(
        &server.addr,
        &server,
        "shape-remote-call-async-race-cancel-",
        "TCP race loser cancellation",
    );
}

#[test]
#[ignore = "timing-sensitive distributed cancellation proof; run serialized under the supervisor cgroup lane"]
fn tls_remote_call_async_race_cancels_slow_loser() {
    let _guard = lock_process();
    let server = start_tls_serve("none", None, &[]);
    race_cancels_slow_loser_for(
        server.tls_addr(),
        &server,
        "shape-remote-call-async-tls-race-cancel-",
        "TLS race loser cancellation",
    );
}

#[test]
#[ignore = "timing-sensitive distributed cancellation proof; run serialized under the supervisor cgroup lane"]
fn remote_call_async_cancel_queued_call_is_receiver_visible() {
    let _guard = lock_process();
    let server = start_serve_with_max_concurrent("none", 1);
    cancel_queued_call_is_receiver_visible_for(
        &server.addr,
        &server,
        "shape-remote-call-async-queued-cancel-",
        "TCP queued cancellation",
    );
}

#[test]
#[ignore = "timing-sensitive distributed cancellation proof; run serialized under the supervisor cgroup lane"]
fn tls_remote_call_async_cancel_queued_call_is_receiver_visible() {
    let _guard = lock_process();
    let server = start_tls_serve_with_max_concurrent("none", 1);
    cancel_queued_call_is_receiver_visible_for(
        server.tls_addr(),
        &server,
        "shape-remote-call-async-tls-queued-cancel-",
        "TLS queued cancellation",
    );
}

#[test]
#[ignore = "timing-sensitive distributed cancellation proof; run serialized under the supervisor cgroup lane"]
fn remote_call_async_cancel_running_call_is_honest() {
    let _guard = lock_process();
    let server = start_serve("none", None, &[]);
    cancel_running_call_is_honest_for(
        &server.addr,
        &server,
        "shape-remote-call-async-running-cancel-",
        "TCP running cancellation",
    );
}

#[test]
#[ignore = "timing-sensitive distributed cancellation proof; run serialized under the supervisor cgroup lane"]
fn tls_remote_call_async_cancel_running_call_is_honest() {
    let _guard = lock_process();
    let server = start_tls_serve("none", None, &[]);
    cancel_running_call_is_honest_for(
        server.tls_addr(),
        &server,
        "shape-remote-call-async-tls-running-cancel-",
        "TLS running cancellation",
    );
}
