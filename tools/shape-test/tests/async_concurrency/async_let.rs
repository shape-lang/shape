//! Async let tests for task spawning.
//!
//! Covers:
//! - Basic async let (spawn + bind future)
//! - Multiple async let bindings
//! - Awaiting a spawned task
//! - Async let with computed expressions
//!
//! `async let x = expr` spawns a task, binds the future handle to `x`.
//! `await x` resolves the future. Must be inside an `async fn`.
//!
//! Known limitation: The semantic analyzer does not track variable bindings
//! from `async let`, so code like `async let x = 42; await x` produces
//! "Undefined variable: 'x'" at the semantic analysis phase. The compiler
//! and VM handle it correctly. These tests currently fail (TDD) until
//! the semantic analyzer is updated.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// Basic async let
// =========================================================================

#[test]
// TDD: Semantic analyzer does not register async let variable bindings
fn async_let_spawns_and_awaits() {
    let code = r#"
async fn spawn_one() {
    async let x = 42
    let result = await x
    print(result)
}

await spawn_one()
"#;

    ShapeTest::new(code).expect_run_ok().expect_output("42");
}

#[test]
// TDD: Semantic analyzer does not register async let variable bindings
fn async_let_with_expression() {
    let code = r#"
async fn compute() {
    async let total = 10 + 20 + 30
    let result = await total
    print(result)
}

await compute()
"#;

    ShapeTest::new(code).expect_run_ok().expect_output("60");
}

// =========================================================================
// Multiple async let bindings
// =========================================================================

#[test]
// TDD: Semantic analyzer does not register async let variable bindings
fn multiple_async_let_bindings() {
    let code = r#"
async fn multi() {
    async let a = 1
    async let b = 2
    async let c = 3
    let va = await a
    let vb = await b
    let vc = await c
    print(va + vb + vc)
}

await multi()
"#;

    ShapeTest::new(code).expect_run_ok().expect_output("6");
}

// =========================================================================
// Async let with string values
// =========================================================================

#[test]
// TDD: Semantic analyzer does not register async let variable bindings
fn async_let_with_string_value() {
    let code = r#"
async fn fetch_name() {
    async let name = "Shape"
    let result = await name
    print(f"Hello, {result}!")
}

await fetch_name()
"#;

    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("Hello, Shape!");
}

// Regression (WF-2D-fu repair): a zero-arg async fn whose DECLARED return type
// is a heap value (here `string`) used in an `async let` must return its value
// correctly via the eager path — NOT surface the isolation boundary's
// `NotImplemented: ... non-scalar result kind String ...`. The deferral guard
// gates on the proven declared return type; only leaf scalars (int/number/bool)
// take the isolated-task path. Heap returns keep the pre-WF-2D-fu eager path.
#[test]
fn async_let_heap_return_async_fn_keeps_eager_path() {
    let code = r#"
use std::core::time

async fn fetch() -> string {
    await time::sleep(0.0)
    return "hello"
}

async fn run() {
    async let a = fetch()
    print(await a)
}

await run()
"#;

    ShapeTest::new(code)
        .with_stdlib()
        .expect_run_ok()
        .expect_output("hello");
}

// =========================================================================
// Async let requires async function context
// =========================================================================

#[test]
// TDD: Semantic analyzer reports "Undefined variable" before compiler can emit the async error
fn async_let_outside_async_fn_is_compile_error() {
    // The compiler should reject `async let` outside an async function,
    // but the semantic analyzer currently errors first with "Undefined variable".
    let code = r#"
fn sync_fn() {
    async let x = 42
    x
}

sync_fn()
"#;

    ShapeTest::new(code).expect_run_err_contains("async");
}

// =========================================================================
// Timing regression: async let must overlap, not serialize (WF-2D-fu)
// =========================================================================

// Two independent async lets, each awaiting a user-defined async fn that
// sleeps ~1s. If the spawn is real (deferred-thunk + isolated task VM), the
// two sleeps overlap and add ~1s of wall-clock over the fixed spawn/init
// overhead. If it regresses to the pre-WF-2D-fu eager-RHS path, the bodies
// run inline on the spawner thread and add ~2s.
//
// Absolute wall-clock is NOT usable as the threshold: the isolated task VMs
// re-initialize state per spawn, and that fixed overhead is large and
// build-mode dependent (seconds in an unoptimized test binary, ~200ms in a
// release build). So we measure the SLEEP contribution differentially against
// a zero-sleep baseline that walks the identical spawn path. The delta
// isolates the sleeps: ~1s when they overlap, ~2s when they serialize.
fn spawn_two_ms(sleep_ms: &str) -> u128 {
    let code = format!(
        r#"
use std::core::time

async fn work() -> int {{
    await time::sleep({sleep_ms})
    42
}}

async fn run_two() -> int {{
    async let a = work()
    async let b = work()
    let ra = await a
    let rb = await b
    print(ra + rb)
    0
}}

await run_two()
"#
    );
    // IMPORTANT: exactly ONE terminal assertion here. Every `expect_*` method
    // on the fluent builder RE-EXECUTES the whole program, so chaining
    // `expect_run_ok().expect_output(..)` would run the two async lets twice
    // and double the measured wall-clock. `expect_output` alone both proves
    // the run succeeded (84 = 42+42) and drives a single execution.
    let start = std::time::Instant::now();
    ShapeTest::new(&code).with_stdlib().expect_output("84");
    start.elapsed().as_millis()
}

#[test]
fn async_let_two_one_second_tasks_overlap() {
    // Warm up: the FIRST ShapeTest run in a process pays large one-time
    // runtime/JIT init (many seconds in a debug test binary). Discard it so
    // the measured runs below reflect steady-state cost only.
    let _ = spawn_two_ms("0.0");

    // Zero-sleep baseline measures the fixed spawn/init overhead of the two
    // isolated task VMs on this exact code path and build (warm).
    let baseline = spawn_two_ms("0.0");
    // Same path, but each task now sleeps ~1s.
    let with_sleep = spawn_two_ms("1000.0");

    let sleep_contribution = with_sleep.saturating_sub(baseline);
    eprintln!(
        "async_let overlap timing: baseline={baseline}ms with_sleep={with_sleep}ms \
         sleep_contribution={sleep_contribution}ms"
    );

    // Overlap => ~1000ms added; serial regression => ~2000ms added. The 1500ms
    // midpoint pins overlap while tolerating scheduling jitter without being
    // flaky on a loaded box.
    assert!(
        sleep_contribution < 1500,
        "async let did not overlap: two 1s tasks added {sleep_contribution}ms over the \
         {baseline}ms zero-sleep baseline (with_sleep={with_sleep}ms). Expected < 1500ms; \
         a serial (eager-RHS) regression adds ~2000ms."
    );
}
