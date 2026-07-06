//! Shared background async runtime for real concurrent task execution.
//!
//! # WF-2D real concurrency (Decision D1, 2026-07-05)
//!
//! The Shape VM interpreter is single-threaded and `!Sync`: Shape bytecode
//! for a task cannot be split across OS threads, and the interpreter cannot
//! suspend a mid-flight frame (coroutine-style suspension is the deferred
//! "Phase-2c snapshot-tier" work referenced throughout ADR-006 §2.7.4).
//!
//! Genuine wall-clock overlap is therefore only achievable at the point where
//! Shape work actually *waits*: an async **module** function (e.g.
//! `time::sleep`) whose body is a self-contained `Send + 'static` tokio
//! future (`register_typed_async_function` in `shape-runtime`). Those futures
//! do not borrow the VM (the `&ModuleContext` borrow "cannot cross await
//! points" — see `marshal.rs::VariadicTypedAsyncBody`), so they can be spawned
//! onto a background multi-threaded runtime and made progress concurrently
//! while the interpreter thread does other work or blocks on a completion.
//!
//! This module owns that background runtime. It is a process-global,
//! lazily-initialized multi-threaded tokio runtime. Async module futures are
//! `spawn`ed onto it (see `vm_impl/modules.rs::spawn_async_module_future`);
//! the interpreter thread collects results through a plain
//! `std::sync::mpsc` channel (blocking `recv` for `await`, non-blocking
//! `try_recv` for the `race`/`any` first-completion poll).
//!
//! ## Why a dedicated runtime rather than the ambient one
//!
//! The CLI entrypoint is `#[tokio::main(flavor = "current_thread")]`. Spawning
//! onto the ambient current-thread runtime would not overlap (a single worker
//! thread), and blocking on it (`block_on`) from inside the ambient runtime
//! panics ("Cannot start a runtime from within a runtime"). A separate
//! multi-threaded runtime sidesteps both: `spawn` merely enqueues onto its own
//! worker pool (never blocks, never nests), and completion is delivered over a
//! non-tokio `std::sync::mpsc` channel so the interpreter thread's blocking
//! `recv` never touches the ambient runtime's reactor.

use std::sync::OnceLock;
use tokio::runtime::Runtime;

static SHARED_ASYNC_RT: OnceLock<Runtime> = OnceLock::new();

/// Return the process-global background async runtime, building it on first
/// use.
///
/// Multi-threaded with `enable_all` (timers + IO drivers) so `time::sleep`
/// and future IO-bearing async module functions make real concurrent
/// progress on the worker pool.
pub(crate) fn shared_runtime() -> &'static Runtime {
    SHARED_ASYNC_RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("shape-async-worker")
            .build()
            .expect("failed to build shared Shape async runtime")
    })
}
