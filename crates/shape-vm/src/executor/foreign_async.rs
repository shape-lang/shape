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

#[cfg(test)]
mod tests {
    //! Runtime tripwires for ADR-019 §5 / #202 (POLY-ASYNC-OFFLOAD).
    //!
    //! These drive the REAL VM path (`invoke_foreign_async_kinded` → offload →
    //! `resolve_pending_async_task`) against an in-process fake extension whose
    //! `invoke` sleeps and records the threads it ran on. A fake rather than the
    //! built `.so` on purpose, for the same reason the #196 stub-channel tests
    //! use one: what is under test is the HOST's behaviour, and it must fail if
    //! the host stops offloading — independently of whether a Python
    //! interpreter or a V8 build is present on the machine. The end-to-end
    //! proof against the real extensions is a separate, environment-dependent
    //! exercise; this is the one that runs everywhere and gates merges.

    use super::*;
    use crate::executor::{VMConfig, VirtualMachine};
    use shape_abi_v1::LanguageRuntimeVTable;
    use std::time::{Duration, Instant};

    /// A sleep long enough that serialization is unmistakable and short enough
    /// that the suite stays fast. Two of these overlapped finish in about one
    /// of them; serialized they take two.
    const SLEEP: Duration = Duration::from_millis(200);

    /// A fake language runtime whose `invoke` sleeps, so wall-clock is a direct
    /// readout of whether calls overlapped.
    mod sleepy_extension {
        use super::SLEEP;
        use shape_abi_v1::{ErrorModel, LanguageRuntimeVTable, STATE_MODEL_STATEFUL_OPAQUE};
        use std::collections::HashSet;
        use std::ffi::{c_char, c_void};
        use std::sync::Mutex;
        use std::sync::atomic::{AtomicUsize, Ordering};

        /// How many invokes are inside the sleep RIGHT NOW, and the high-water
        /// mark. The high-water mark is the direct evidence of overlap — a wall
        /// clock can be fooled by a slow machine, a concurrency peak of 2
        /// cannot be reached by serialized calls.
        pub static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);
        pub static PEAK_IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);
        /// Total completed invokes, including ones whose result nobody collected.
        pub static COMPLETED: AtomicUsize = AtomicUsize::new(0);
        /// How many distinct `init` instances exist (one per thread-affine
        /// worker, one for a shared runtime).
        pub static INSTANCES: AtomicUsize = AtomicUsize::new(0);
        /// Thread ids that ran an `invoke`, as strings.
        pub static INVOKE_THREADS: Mutex<Vec<String>> = Mutex::new(Vec::new());
        /// Instances that received `register_types`.
        pub static REGISTERED_INSTANCES: Mutex<Option<HashSet<usize>>> = Mutex::new(None);

        pub fn reset() {
            IN_FLIGHT.store(0, Ordering::SeqCst);
            PEAK_IN_FLIGHT.store(0, Ordering::SeqCst);
            COMPLETED.store(0, Ordering::SeqCst);
            INSTANCES.store(0, Ordering::SeqCst);
            INVOKE_THREADS.lock().unwrap().clear();
            *REGISTERED_INSTANCES.lock().unwrap() = Some(HashSet::new());
        }

        pub fn peak() -> usize {
            PEAK_IN_FLIGHT.load(Ordering::SeqCst)
        }

        pub fn completed() -> usize {
            COMPLETED.load(Ordering::SeqCst)
        }

        pub fn distinct_invoke_threads() -> usize {
            INVOKE_THREADS
                .lock()
                .unwrap()
                .iter()
                .collect::<HashSet<_>>()
                .len()
        }

        pub fn registered_instance_count() -> usize {
            REGISTERED_INSTANCES
                .lock()
                .unwrap()
                .as_ref()
                .map(|s| s.len())
                .unwrap_or(0)
        }

        unsafe extern "C" fn init(_config: *const u8, _len: usize) -> *mut c_void {
            // Distinct non-null instance pointers so a per-worker instance is
            // distinguishable from the shared one.
            (INSTANCES.fetch_add(1, Ordering::SeqCst) + 1) as *mut c_void
        }

        unsafe extern "C" fn register_types(
            instance: *mut c_void,
            _types: *const u8,
            _types_len: usize,
        ) -> i32 {
            if let Some(set) = REGISTERED_INSTANCES.lock().unwrap().as_mut() {
                set.insert(instance as usize);
            }
            0
        }

        unsafe extern "C" fn generate_stubs(
            _instance: *mut c_void,
            out_ptr: *mut *mut u8,
            out_len: *mut usize,
        ) -> i32 {
            unsafe {
                *out_ptr = std::ptr::null_mut();
                *out_len = 0;
            }
            0
        }

        #[allow(clippy::too_many_arguments)]
        unsafe extern "C" fn compile(
            _instance: *mut c_void,
            _name: *const u8,
            _name_len: usize,
            _source: *const u8,
            _source_len: usize,
            _param_names: *const u8,
            _param_names_len: usize,
            _param_types: *const u8,
            _param_types_len: usize,
            _return_type: *const u8,
            _return_type_len: usize,
            _is_async: bool,
            _out_error: *mut *mut u8,
            _out_error_len: *mut usize,
        ) -> *mut c_void {
            1usize as *mut c_void
        }

        unsafe extern "C" fn invoke(
            _instance: *mut c_void,
            _handle: *mut c_void,
            _args: *const u8,
            _args_len: usize,
            out_ptr: *mut *mut u8,
            out_len: *mut usize,
        ) -> i32 {
            INVOKE_THREADS
                .lock()
                .unwrap()
                .push(format!("{:?}", std::thread::current().id()));
            let now = IN_FLIGHT.fetch_add(1, Ordering::SeqCst) + 1;
            PEAK_IN_FLIGHT.fetch_max(now, Ordering::SeqCst);
            std::thread::sleep(SLEEP);
            IN_FLIGHT.fetch_sub(1, Ordering::SeqCst);
            COMPLETED.fetch_add(1, Ordering::SeqCst);

            // msgpack for the integer 7.
            let mut buf = vec![7u8];
            let len = buf.len();
            let ptr = buf.as_mut_ptr();
            std::mem::forget(buf);
            unsafe {
                *out_ptr = ptr;
                *out_len = len;
            }
            0
        }

        unsafe extern "C" fn dispose_function(_instance: *mut c_void, _handle: *mut c_void) {}

        unsafe extern "C" fn language_id(_instance: *mut c_void) -> *const c_char {
            c"python".as_ptr()
        }

        unsafe extern "C" fn free_buffer(ptr: *mut u8, len: usize) {
            if !ptr.is_null() {
                unsafe { drop(Vec::from_raw_parts(ptr, len, len)) };
            }
        }

        unsafe extern "C" fn drop_instance(_instance: *mut c_void) {}

        unsafe extern "C" fn declare_shared(_instance: *mut c_void) -> u32 {
            shape_abi_v1::INSTANCE_CONCURRENCY_SHARED
        }

        unsafe extern "C" fn declare_thread_affine(_instance: *mut c_void) -> u32 {
            shape_abi_v1::INSTANCE_CONCURRENCY_THREAD_AFFINE
        }

        const fn base(
            concurrency: Option<unsafe extern "C" fn(*mut c_void) -> u32>,
        ) -> LanguageRuntimeVTable {
            LanguageRuntimeVTable {
                init: Some(init),
                register_types: Some(register_types),
                compile: Some(compile),
                invoke: Some(invoke),
                dispose_function: Some(dispose_function),
                language_id: Some(language_id),
                get_lsp_config: None,
                free_buffer: Some(free_buffer),
                drop: Some(drop_instance),
                error_model: ErrorModel::Dynamic,
                get_shape_source: None,
                runtime_descriptor: None,
                state_model: STATE_MODEL_STATEFUL_OPAQUE,
                generate_stubs: Some(generate_stubs),
                instance_concurrency: concurrency,
                reserved2: None,
                reserved3: None,
            }
        }

        /// Declares the shared model, like the real Python runtime.
        pub static SHARED: LanguageRuntimeVTable = base(Some(declare_shared));
        /// Declares the thread-affine model, like the real TypeScript runtime.
        pub static THREAD_AFFINE: LanguageRuntimeVTable = base(Some(declare_thread_affine));
        /// Declares nothing — the shape of an extension built before #202.
        pub static UNDECLARED: LanguageRuntimeVTable = base(None);
    }

    /// The fake's counters are process-global statics, so the tests that read
    /// them run one at a time.
    static SLEEPY_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    const ASYNC_PROGRAM: &str = r#"
async fn python slow(a: int) -> Result<int> {
    return a
}
"#;

    const SYNC_PROGRAM: &str = r#"
fn python quick(a: int) -> Result<int> {
    return a
}
"#;

    fn vm_with(code: &str, vtable: &'static LanguageRuntimeVTable) -> VirtualMachine {
        use crate::compiler::BytecodeCompiler;

        let program = shape_ast::parser::parse_program(code).expect("parse failed");
        let bytecode = BytecodeCompiler::new()
            .compile(&program)
            .expect("compile failed");
        let runtime = PluginLanguageRuntime::new(vtable, &serde_json::Value::Null)
            .expect("fake runtime initializes");
        let mut runtimes = std::collections::HashMap::new();
        runtimes.insert("python".to_string(), std::sync::Arc::new(runtime));

        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(bytecode);
        vm.foreign_fn_handles = vec![None];
        vm.set_language_runtimes(runtimes);
        vm
    }

    fn future_id(slot: &shape_value::KindedSlot) -> u64 {
        assert_eq!(
            slot.kind(),
            shape_value::NativeKind::Ptr(shape_value::heap_value::HeapKind::Future),
            "an async foreign call must yield a Future handle"
        );
        slot.raw()
    }

    /// TRIPWIRE 1 — two async foreign calls overlap.
    ///
    /// Both are started before either is awaited, which is the whole point of
    /// returning a `Future(id)` from the call rather than the result. Two 200ms
    /// invokes must finish in about 200ms, not 400ms; and the fake's in-flight
    /// high-water mark must reach 2, which serialized calls cannot do however
    /// slow the machine is.
    #[test]
    fn two_async_foreign_calls_overlap_instead_of_serializing() {
        let _guard = SLEEPY_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        sleepy_extension::reset();

        let mut vm = vm_with(ASYNC_PROGRAM, &sleepy_extension::SHARED);
        let start = Instant::now();

        let first = vm
            .invoke_foreign_async_kinded(0, &[])
            .expect("the first async call starts");
        let second = vm
            .invoke_foreign_async_kinded(0, &[])
            .expect("the second async call starts");

        // Both are in flight now: starting them cost nothing like the sleep.
        assert!(
            start.elapsed() < SLEEP,
            "starting two async foreign calls must not block the interpreter thread \
             for the length of the work; took {:?}",
            start.elapsed()
        );

        vm.resolve_pending_async_task(future_id(&first))
            .expect("the first result arrives");
        vm.resolve_pending_async_task(future_id(&second))
            .expect("the second result arrives");
        let elapsed = start.elapsed();

        assert_eq!(
            sleepy_extension::peak(),
            2,
            "both invokes must have been inside the extension at the same time"
        );
        assert!(
            elapsed < SLEEP * 2,
            "two overlapped {SLEEP:?} calls must finish in well under twice that; \
             took {elapsed:?} (serialized would be ~{:?})",
            SLEEP * 2
        );
        assert_eq!(sleepy_extension::completed(), 2);
    }

    /// TRIPWIRE 2 — a SYNC foreign call is unchanged: it runs on the
    /// interpreter thread, returns the value itself (not a future), and blocks
    /// for its own duration. The offload must not leak onto declarations that
    /// never asked for it.
    #[test]
    fn a_sync_foreign_call_still_runs_inline_and_returns_its_value() {
        let _guard = SLEEPY_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        sleepy_extension::reset();

        let mut vm = vm_with(SYNC_PROGRAM, &sleepy_extension::SHARED);
        let interpreter_thread = format!("{:?}", std::thread::current().id());
        let start = Instant::now();
        let result = vm.invoke_foreign_kinded(0, &[]).expect("sync call runs");
        let elapsed = start.elapsed();

        assert_ne!(
            result.kind(),
            shape_value::NativeKind::Ptr(shape_value::heap_value::HeapKind::Future),
            "a sync foreign call must return its value, not a future"
        );
        assert!(
            elapsed >= SLEEP,
            "a sync call blocks the caller for its own duration; took {elapsed:?}"
        );
        let threads = sleepy_extension::INVOKE_THREADS.lock().unwrap().clone();
        assert_eq!(
            threads,
            vec![interpreter_thread],
            "the sync invoke must run on the interpreter thread"
        );
    }

    /// TRIPWIRE 4 — cancellation is run-to-completion-then-discard, and it is
    /// never reported as confirmed foreign termination.
    ///
    /// Cancelling an in-flight foreign await does not and cannot interrupt code
    /// running inside the extension. What it does is stop waiting and drop the
    /// result. The evidence that this is what happened, rather than a silent
    /// leak: the invoke still COMPLETES exactly once, and no second completion
    /// is ever delivered.
    #[test]
    fn cancelling_an_in_flight_foreign_await_discards_without_double_completion() {
        let _guard = SLEEPY_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        sleepy_extension::reset();

        let mut vm = vm_with(ASYNC_PROGRAM, &sleepy_extension::SHARED);
        let handle = vm
            .invoke_foreign_async_kinded(0, &[])
            .expect("the async call starts");
        let task_id = future_id(&handle);

        // Cancel while the extension is still inside the sleep.
        assert!(vm.task_scheduler.has_pending_async(task_id));
        vm.task_scheduler.cancel(task_id);
        assert!(
            !vm.task_scheduler.has_pending_async(task_id),
            "a cancelled task is no longer awaitable"
        );

        // The foreign body was NOT stopped: it runs to completion on its own
        // schedule. Give it time to do so, then assert it ran exactly once.
        std::thread::sleep(SLEEP * 2);
        assert_eq!(
            sleepy_extension::completed(),
            1,
            "the foreign body runs to completion exactly once after cancellation — \
             cancellation discards the result, it does not terminate the foreign runtime"
        );

        // And nothing re-delivers it: a second await of the cancelled id does
        // not produce a value.
        assert!(
            vm.resolve_pending_async_task(task_id).is_err(),
            "a cancelled task must not later yield a result — that would be the \
             double completion this tripwire exists to catch"
        );
    }

    /// TRIPWIRE 5 — concurrent invokes on ONE instance are serialized against
    /// the mutating vtable calls, and the contract delivery is inside that
    /// discipline (the #196 exposure note: `register_types` is now on a live
    /// path and takes the instance pointer like `invoke` does).
    ///
    /// For the SHARED model the extension has declared that concurrent invokes
    /// are sound on its own state, so what the host must guarantee is that the
    /// instance received its contract before any offloaded invoke reached it.
    #[test]
    fn the_shared_instance_receives_its_contract_before_any_offloaded_invoke() {
        let _guard = SLEEPY_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        sleepy_extension::reset();

        let mut vm = vm_with(ASYNC_PROGRAM, &sleepy_extension::SHARED);
        let handles: Vec<_> = (0..4)
            .map(|_| {
                vm.invoke_foreign_async_kinded(0, &[])
                    .expect("async call starts")
            })
            .collect();
        for handle in &handles {
            vm.resolve_pending_async_task(future_id(handle))
                .expect("result arrives");
        }

        assert_eq!(
            sleepy_extension::registered_instance_count(),
            1,
            "the one shared instance received the declared contract"
        );
        assert_eq!(sleepy_extension::completed(), 4);
        assert!(
            sleepy_extension::peak() > 1,
            "the shared model must actually overlap; peak was {}",
            sleepy_extension::peak()
        );
    }

    /// TRIPWIRE 5, thread-affine half — a thread-affine runtime never has one
    /// instance touched by two threads. Each worker builds its OWN instance and
    /// delivers the contract to it before its first compile, so overlap comes
    /// from there being several workers rather than from one isolate being
    /// re-entered.
    #[test]
    fn a_thread_affine_runtime_gives_each_worker_its_own_instance() {
        let _guard = SLEEPY_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        sleepy_extension::reset();

        let mut vm = vm_with(ASYNC_PROGRAM, &sleepy_extension::THREAD_AFFINE);
        let handles: Vec<_> = (0..2)
            .map(|_| {
                vm.invoke_foreign_async_kinded(0, &[])
                    .expect("async call starts")
            })
            .collect();
        for handle in &handles {
            vm.resolve_pending_async_task(future_id(handle))
                .expect("result arrives");
        }

        assert_eq!(
            sleepy_extension::distinct_invoke_threads(),
            2,
            "two concurrent thread-affine calls must run on two different workers"
        );
        assert!(
            sleepy_extension::registered_instance_count() >= 2,
            "each worker's own instance must receive the declared contract before its \
             first compile; only {} did",
            sleepy_extension::registered_instance_count()
        );
        assert_eq!(
            sleepy_extension::peak(),
            2,
            "the dedicated workers must overlap"
        );
    }

    /// An extension that declares no off-thread model is REFUSED, not quietly
    /// run on the interpreter thread. Running it there is exactly the
    /// untruthful `async` contract ADR-019 §5 forbids, and it is the failure
    /// mode a permissive default would produce.
    #[test]
    fn an_undeclared_runtime_refuses_the_offload_instead_of_faking_it() {
        let _guard = SLEEPY_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        sleepy_extension::reset();

        let mut vm = vm_with(ASYNC_PROGRAM, &sleepy_extension::UNDECLARED);
        let err = vm
            .invoke_foreign_async_kinded(0, &[])
            .expect_err("an undeclared runtime must refuse");
        let message = err.to_string();
        assert!(
            message.contains("instance_concurrency"),
            "the refusal names the declaration slot the extension is missing, got: {message}"
        );
        assert_eq!(
            sleepy_extension::completed(),
            0,
            "the refusal must happen BEFORE any foreign code runs"
        );
    }

    /// The strategy is chosen once per language and reused, so a second async
    /// call does not start a second worker pool.
    #[test]
    fn the_offload_strategy_is_built_once_per_language() {
        let runtime = std::sync::Arc::new(
            PluginLanguageRuntime::new(&sleepy_extension::THREAD_AFFINE, &serde_json::Value::Null)
                .expect("fake runtime initializes"),
        );
        let mut offloads = ForeignAsyncOffloads::default();
        let first = offloads.strategy("python", &runtime).expect("declared") as *const _;
        let second = offloads.strategy("python", &runtime).expect("declared") as *const _;
        assert_eq!(
            first, second,
            "the same strategy value must be handed out, not a fresh worker pool"
        );
    }

    /// Guards the worker count against being silently raised: each affine
    /// worker owns a V8-class isolate, so the number is a memory decision, not
    /// a tuning knob to reach for when a benchmark looks slow.
    #[test]
    fn the_affine_worker_count_is_deliberate() {
        assert_eq!(AFFINE_WORKERS, 4);
    }
}
