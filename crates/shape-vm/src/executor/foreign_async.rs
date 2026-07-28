//! Off-thread execution of `async fn <language>` foreign calls.
//!
//! ADR-019 §5 / issue #202 (POLY-ASYNC-OFFLOAD).
//!
//! # The shape this copies
//!
//! Shape's own async module calls (`spawn_async_module_future` /
//! `resolve_pending_async_task`, `vm_impl/modules.rs`) are an eager offload plus
//! a blocking receive at the await point: evaluating the call starts the work on
//! a background thread and yields a `Future(id)` immediately, so the interpreter
//! runs on and a later `await` collects a result that has been making progress
//! the whole time. Foreign async is that same shape with a different payload —
//! the extension's msgpack `Vec<u8>` instead of a `TypedReturn` — and it reuses
//! the same `PendingAsyncTask` completion channel, so `await`, `join`, `race`
//! and `any` need no foreign-specific knowledge.
//!
//! Everything that crosses to the worker is owned and `Send`: the argument bytes
//! are marshalled on the interpreter thread (marshalling reads the VM's schema
//! registry) and the result bytes are unmarshalled back on it. No VM state, no
//! heap share, and no `KindedSlot` ever leaves the interpreter thread.
//!
//! # Why the extension gets to decide how it is driven
//!
//! Offloading makes the extension instance genuinely shared, and whether that is
//! sound is a property only the extension knows: a CPython instance behind
//! interior synchronization tolerates concurrent invokes (and overlaps, because
//! CPython releases the GIL across `time.sleep` and blocking IO), while a V8
//! isolate must never leave the thread that created it. So the host reads
//! [`InstanceConcurrency`] from the vtable and picks a strategy. Deciding this
//! from the language id would be a terminal-name switch selecting a capability,
//! which §Forbidden Patterns refuses; an extension that declares nothing keeps
//! exactly the synchronous behaviour it had before this module existed.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};

use shape_abi_v1::foreign_types::ForeignContractExport;
use shape_runtime::plugins::language_runtime::{
    CompiledForeignFunction, InstanceConcurrency, PluginLanguageRuntime,
};

/// How many dedicated worker threads a thread-affine language gets.
///
/// Each worker owns its own extension instance — for TypeScript, its own V8
/// isolate — so this is also the number of `async fn typescript` calls that can
/// be in flight at once. Kept small: an isolate is not cheap, and a program that
/// wants more foreign concurrency than this is better served by batching inside
/// one foreign body than by paying for more isolates.
const AFFINE_WORKERS: usize = 4;

/// Everything a worker needs to compile a foreign function without reading the
/// VM's program table.
///
/// A thread-affine worker owns a private instance, so a function compiled on the
/// interpreter thread means nothing to it — it must compile the body itself, on
/// its own instance, the first time it is asked to run it.
#[derive(Clone)]
pub(crate) struct ForeignCompileSpec {
    pub name: String,
    pub body_text: String,
    pub param_names: Vec<String>,
    pub param_types: Vec<String>,
    pub return_type: Option<String>,
}

impl ForeignCompileSpec {
    fn compile_with(&self, runtime: &PluginLanguageRuntime) -> Result<CompiledForeignFunction, String> {
        runtime
            .compile(
                &self.name,
                &self.body_text,
                &self.param_names,
                &self.param_types,
                self.return_type.as_deref(),
                // Always `true` here: this spec only ever describes a function
                // reached through the async offload path.
                true,
            )
            .map_err(|e| e.to_string())
    }
}

/// One unit of work handed to a thread-affine worker.
struct AffineJob {
    /// Identifies the function within the program, and therefore within the
    /// worker's own compile cache.
    foreign_idx: usize,
    spec: ForeignCompileSpec,
    /// Delivered to the worker's private instance before its first compile, so
    /// a worker-owned instance is not a contract-less instance.
    contract: Arc<ForeignContractExport>,
    args_bytes: Vec<u8>,
    /// Where the msgpack result goes. Dropping the receiver is how cancellation
    /// reaches the worker: the send fails, the work has already run to
    /// completion, and the result is discarded.
    reply: Sender<Result<Vec<u8>, String>>,
}

/// A pool of dedicated worker threads for a thread-affine language runtime.
///
/// ADR-019 §5's "dedicated worker thread owning the V8 isolate" pattern, with
/// enough workers that two async calls can genuinely overlap. Each worker builds
/// its instance with [`PluginLanguageRuntime::fresh_instance`] *on its own
/// thread* and never lets it out, so thread-affinity is preserved by
/// construction rather than by a lock — a lock would make the calls safe from
/// each other but would not stop the isolate from being entered from the wrong
/// thread.
pub(crate) struct AffineWorkerPool {
    submit: Sender<AffineJob>,
}

impl AffineWorkerPool {
    fn start(language: &str, template: Arc<PluginLanguageRuntime>) -> Self {
        let (submit, jobs) = std::sync::mpsc::channel::<AffineJob>();
        let jobs = Arc::new(Mutex::new(jobs));
        for worker in 0..AFFINE_WORKERS {
            let jobs = Arc::clone(&jobs);
            let template = Arc::clone(&template);
            let language = language.to_string();
            // Detached: the pool lives as long as the VM that owns it, and the
            // worker exits when the submit side is dropped.
            let _ = std::thread::Builder::new()
                .name(format!("shape-foreign-{language}-{worker}"))
                .spawn(move || affine_worker_loop(&template, &jobs));
        }
        Self { submit }
    }
}

/// One worker's whole life: build a private instance, then serve jobs on it.
///
/// A failure to build the instance is reported to every job the worker would
/// have taken rather than logged and forgotten — a silently dead worker would
/// look exactly like a slow one.
fn affine_worker_loop(
    template: &PluginLanguageRuntime,
    jobs: &Mutex<Receiver<AffineJob>>,
) {
    let runtime = template.fresh_instance();
    let mut compiled: HashMap<usize, CompiledForeignFunction> = HashMap::new();
    let mut contract_delivered = false;

    loop {
        // The lock covers only the receive, so workers block on the queue, not
        // on each other's foreign calls.
        let job = {
            let Ok(guard) = jobs.lock() else { return };
            match guard.recv() {
                Ok(job) => job,
                // Submit side dropped: the VM is gone.
                Err(_) => return,
            }
        };

        let runtime = match &runtime {
            Ok(runtime) => runtime,
            Err(e) => {
                let _ = job.reply.send(Err(format!(
                    "foreign function '{}': could not build a dedicated runtime instance \
                     for the async worker: {e}",
                    job.spec.name
                )));
                continue;
            }
        };

        if !contract_delivered {
            match runtime.register_contract(&job.contract) {
                Ok(_) => contract_delivered = true,
                Err(e) => {
                    let _ = job.reply.send(Err(format!(
                        "foreign function '{}': the async worker's runtime instance refused \
                         the declared contract: {e}",
                        job.spec.name
                    )));
                    continue;
                }
            }
        }

        let handle = match compiled.entry(job.foreign_idx) {
            std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
            std::collections::hash_map::Entry::Vacant(e) => match job.spec.compile_with(runtime) {
                Ok(handle) => e.insert(handle),
                Err(err) => {
                    let _ = job.reply.send(Err(err));
                    continue;
                }
            },
        };

        let outcome = runtime
            .invoke(handle, &job.args_bytes)
            .map_err(|e| e.to_string());
        // The receiver is gone when the await was cancelled. The work already
        // ran to completion — that is the declared semantics — and the result
        // is dropped here.
        let _ = job.reply.send(outcome);
    }
}

/// The offload strategies, chosen per language from the extension's declaration.
pub(crate) enum ForeignOffload {
    /// The instance tolerates concurrent invokes: run on tokio's blocking pool
    /// against the shared instance and the handle compiled at link-now.
    Shared,
    /// The instance is thread-affine: hand the work to a dedicated worker that
    /// owns a private instance.
    Affine(Arc<AffineWorkerPool>),
}

/// Per-VM registry of foreign async offload strategies, one per language.
///
/// Built lazily: a program with no `async fn <language>` never starts a worker
/// thread, and a program that has one starts them on its first such call.
#[derive(Default)]
pub(crate) struct ForeignAsyncOffloads {
    by_language: HashMap<String, ForeignOffload>,
}

impl ForeignAsyncOffloads {
    /// The offload strategy for `language`, starting its workers on first use.
    ///
    /// `Err` when the runtime declares no off-thread capability: the async call
    /// is refused rather than run synchronously behind an `async` spelling,
    /// which is the untruthful contract ADR-019 §5 forbids.
    pub(crate) fn strategy(
        &mut self,
        language: &str,
        runtime: &Arc<PluginLanguageRuntime>,
    ) -> Result<&ForeignOffload, String> {
        if !self.by_language.contains_key(language) {
            let strategy = match runtime.instance_concurrency() {
                InstanceConcurrency::Shared => ForeignOffload::Shared,
                InstanceConcurrency::ThreadAffine => ForeignOffload::Affine(Arc::new(
                    AffineWorkerPool::start(language, Arc::clone(runtime)),
                )),
                InstanceConcurrency::InterpreterThreadOnly => {
                    return Err(format!(
                        "async foreign call into '{language}' is refused: the installed \
                         '{language}' extension does not declare an off-thread invocation \
                         model (`LanguageRuntimeVTable::instance_concurrency`), so the host \
                         cannot run its `invoke` anywhere but the interpreter thread. \
                         Running it there would make `async` promise a concurrency the \
                         runtime does not provide (ADR-019 §5). Rebuild the extension \
                         against an ABI that declares the model, or drop the `async` \
                         keyword from the declaration."
                    ));
                }
            };
            self.by_language.insert(language.to_string(), strategy);
        }
        Ok(&self.by_language[language])
    }
}

/// A foreign invoke that is now genuinely in flight off the interpreter thread.
pub(crate) struct InFlightForeignCall {
    /// Delivers the extension's raw msgpack return payload. Unmarshalling waits
    /// for the interpreter thread, which owns the schema registry.
    pub completion: Receiver<Result<Vec<u8>, String>>,
    /// Present only for the blocking-pool strategy. Aborting a `spawn_blocking`
    /// task does not interrupt a closure that has already started: the foreign
    /// body runs to completion and its result is discarded, which is exactly the
    /// run-to-completion-then-discard semantics #202 declares. It is NOT a
    /// confirmation that the foreign runtime stopped, and nothing in this module
    /// reports it as one.
    pub abort: Option<tokio::task::AbortHandle>,
}

/// Start a foreign invoke off the interpreter thread.
///
/// `compiled` is the handle from the interpreter-thread link-now; it is used
/// only by the [`ForeignOffload::Shared`] strategy, since a thread-affine worker
/// must compile on its own instance.
pub(crate) fn start_offloaded_invoke(
    strategy: &ForeignOffload,
    runtime: &Arc<PluginLanguageRuntime>,
    compiled: &CompiledForeignFunction,
    foreign_idx: usize,
    spec: &ForeignCompileSpec,
    contract: &Arc<ForeignContractExport>,
    args_bytes: Vec<u8>,
) -> InFlightForeignCall {
    let (tx, rx) = std::sync::mpsc::channel();
    match strategy {
        ForeignOffload::Shared => {
            let runtime = Arc::clone(runtime);
            let compiled = compiled.clone();
            let handle = crate::executor::async_runtime::shared_runtime().spawn_blocking(
                move || {
                    let outcome = runtime.invoke(&compiled, &args_bytes).map_err(|e| e.to_string());
                    // Receiver may be gone (await cancelled, VM torn down);
                    // the result is discarded, not retried.
                    let _ = tx.send(outcome);
                },
            );
            InFlightForeignCall {
                completion: rx,
                abort: Some(handle.abort_handle()),
            }
        }
        ForeignOffload::Affine(pool) => {
            let job = AffineJob {
                foreign_idx,
                spec: spec.clone(),
                contract: Arc::clone(contract),
                args_bytes,
                reply: tx.clone(),
            };
            if pool.submit.send(job).is_err() {
                // Every worker is gone. Report it through the same channel the
                // caller is about to await, so the failure surfaces at the
                // await point like any other.
                let _ = tx.send(Err(format!(
                    "foreign function '{}': the dedicated async worker pool for this \
                     language is not running",
                    spec.name
                )));
            }
            InFlightForeignCall {
                completion: rx,
                // A queued or running affine job is discarded by dropping the
                // reply receiver; there is no handle that could interrupt the
                // foreign body, and pretending otherwise would misreport
                // cancellation as confirmed termination.
                abort: None,
            }
        }
    }
}
