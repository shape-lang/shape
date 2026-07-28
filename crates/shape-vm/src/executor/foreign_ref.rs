//! Minting foreign references and routing their disposal.
//!
//! ADR-019 §3 / #200 (POLY-FOREIGN-REF).
//!
//! `shape-value` owns the carrier; this module owns the half that knows what an
//! extension instance is. Its whole job is to answer one question at mint time —
//! *which instance will have to release this object, and how does a `Drop`
//! running on the interpreter thread reach it* — and to bake the answer into
//! the reference, because at drop time there is no VM in scope to ask.
//!
//! # Two owners, because there are two instance models
//!
//! A `INSTANCE_CONCURRENCY_SHARED` runtime has one instance that any thread may
//! touch, so disposal is a direct call. A `INSTANCE_CONCURRENCY_THREAD_AFFINE`
//! runtime has one instance per dedicated async worker plus the interpreter
//! thread's own, and an object minted inside worker 2's isolate can be released
//! only by worker 2 — not by worker 3, not by the interpreter thread, not under
//! a lock. So a reference born in an offloaded call records the worker that ran
//! it, and disposal is addressed to that worker's queue.
//!
//! Which model applies is READ from the extension's declaration, never inferred
//! from the language id: deciding a capability from a terminal name is the
//! spelling-selected semantics this codebase forbids.

use std::sync::Arc;

use shape_runtime::plugins::language_runtime::{InstanceConcurrency, PluginLanguageRuntime};
use shape_value::{ForeignRefData, ForeignRefDisposer, ForeignRefOrigin, KindedSlot, VMError};

use super::foreign_async::{AffineWorkerId, AffineWorkerPool, ForeignOffload, InFlightForeignCall};

/// The instance that will have to release a foreign object.
#[derive(Clone)]
pub enum ForeignRefOwner {
    /// The runtime's own instance — the interpreter thread's, or a shared one.
    Instance(Arc<PluginLanguageRuntime>),
    /// A dedicated worker's private instance, reached through its queue.
    AffineWorker {
        pool: Arc<AffineWorkerPool>,
        worker: AffineWorkerId,
    },
}

/// Disposal against an instance the host may call directly.
///
/// For a `Shared` runtime that is true from any thread, by declaration. For an
/// `InterpreterThreadOnly` runtime it is true only on the interpreter thread —
/// and it holds because a foreign reference never leaves that thread: arguments
/// cross to workers as marshalled bytes, and no `KindedSlot` is ever sent. The
/// `debug_assert` below states that invariant where it is cheap to check rather
/// than leaving it to a comment.
struct InstanceDisposer {
    runtime: Arc<PluginLanguageRuntime>,
    /// The thread that may call this instance, when the runtime is not declared
    /// shared. `None` for a shared runtime, which any thread may call.
    bound_thread: Option<std::thread::ThreadId>,
}

// `ForeignRefDisposer: Debug` so a leaked reference can be printed while it is
// being chased; neither owner type carries printable state, so both render as
// their role rather than their contents.
impl std::fmt::Debug for InstanceDisposer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InstanceDisposer")
            .field("language", &self.runtime.language_id())
            .field("bound_thread", &self.bound_thread)
            .finish()
    }
}

impl ForeignRefDisposer for InstanceDisposer {
    fn dispose(&self, handle: u64) {
        debug_assert!(
            self.bound_thread
                .is_none_or(|owner| owner == std::thread::current().id()),
            "ADR-019 §3: a foreign reference minted against a thread-bound \
             extension instance was dropped on a different thread. References \
             are not supposed to leave the interpreter thread; whatever moved \
             this one must be fixed rather than the disposal skipped, because \
             calling the instance from here would enter the runtime from the \
             wrong thread."
        );
        self.runtime.dispose_ref(handle);
    }
}

/// Disposal routed to the dedicated worker whose isolate minted the object.
///
/// The send does not wait for the worker to act. The disposal runs
/// synchronously on the only thread that may run it, but blocking a Shape scope
/// exit until that worker finishes whatever unrelated foreign call it is inside
/// would turn an ordinary drop into an unbounded pause. Nothing is lost by not
/// waiting: the command is queued ahead of the worker's shutdown, and
/// `std::sync::mpsc` delivers everything already queued before the receiver
/// reports disconnection.
struct AffineWorkerDisposer {
    pool: Arc<AffineWorkerPool>,
    worker: AffineWorkerId,
}

impl std::fmt::Debug for AffineWorkerDisposer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AffineWorkerDisposer")
            .field("worker", &self.worker)
            .finish()
    }
}

impl ForeignRefDisposer for AffineWorkerDisposer {
    fn dispose(&self, handle: u64) {
        self.pool.dispose_ref(self.worker, handle);
    }
}

/// Decide who owns a reference returned by a foreign call.
///
/// This is the join between #202's offload and #200's carrier, and the reason
/// [`InFlightForeignCall::origin_worker`] exists: a call that ran on a
/// dedicated worker minted its result inside *that worker's* isolate, so the
/// reference has to be bound to the worker rather than to the language.
/// Everything else — a synchronous call, or an offloaded call against a shared
/// instance — belongs to the runtime's own instance.
///
/// `in_flight` is `None` for a synchronous call, which by construction ran on
/// the interpreter thread's instance.
pub fn owner_for_call(
    runtime: &Arc<PluginLanguageRuntime>,
    strategy: Option<&ForeignOffload>,
    in_flight: Option<&InFlightForeignCall>,
) -> ForeignRefOwner {
    if let (Some(ForeignOffload::Affine(pool)), Some(worker)) =
        (strategy, in_flight.and_then(|call| call.origin_worker))
    {
        return ForeignRefOwner::AffineWorker {
            pool: Arc::clone(pool),
            worker,
        };
    }
    ForeignRefOwner::Instance(Arc::clone(runtime))
}

/// Build a foreign reference slot for an object an extension just handed back.
///
/// The caller supplies the extension's `handle` verbatim and the `origin` facts
/// a later refusal will quote. On success the returned slot owns one share; the
/// object is released when the last share is retired.
///
/// # Refusals
///
/// A runtime that declares no `dispose_ref` entry cannot release what it mints,
/// so a reference against it would leak by construction. That is refused here,
/// at the boundary, rather than producing a value whose drop is a no-op — the
/// silent-skip shape ADR-019 §3 rules out.
pub fn mint_foreign_ref(
    owner: ForeignRefOwner,
    runtime: &Arc<PluginLanguageRuntime>,
    handle: u64,
    origin: ForeignRefOrigin,
) -> Result<KindedSlot, VMError> {
    if !runtime.can_dispose_refs() {
        return Err(VMError::RuntimeError(format!(
            "foreign function `{}` returned a reference to {}, but the installed \
             '{}' extension declares no way to release it \
             (`LanguageRuntimeVTable::dispose_ref`). Holding it would leak the \
             foreign object with no way to ever free it, so the reference is \
             refused here rather than at some later point (ADR-019 §3). Rebuild \
             the extension against an ABI that declares a disposer.",
            origin.produced_by,
            origin.describe(),
            origin.language,
        )));
    }

    let disposer: Arc<dyn ForeignRefDisposer> = match owner {
        ForeignRefOwner::Instance(runtime) => {
            let bound_thread = match runtime.instance_concurrency() {
                // Declared safe from any thread.
                InstanceConcurrency::Shared => None,
                // Bound to whichever thread minted it — which is the thread
                // running this code.
                InstanceConcurrency::ThreadAffine | InstanceConcurrency::InterpreterThreadOnly => {
                    Some(std::thread::current().id())
                }
            };
            Arc::new(InstanceDisposer {
                runtime,
                bound_thread,
            })
        }
        ForeignRefOwner::AffineWorker { pool, worker } => {
            Arc::new(AffineWorkerDisposer { pool, worker })
        }
    };

    Ok(KindedSlot::from_foreign_ref(Arc::new(ForeignRefData::new(
        handle, origin, disposer,
    ))))
}

#[cfg(test)]
mod tests {
    //! Disposal tripwires for ADR-019 §3 / #200, driven by a
    //! finalization-observing fake extension.
    //!
    //! A fake rather than a built `.so`, for the same reason the #196
    //! stub-channel and #202 offload tests use one: what is under test is the
    //! HOST's behaviour — that it disposes at all, exactly once, and against
    //! the right instance — and it must fail if the host stops doing that,
    //! independently of whether a Python interpreter or a V8 build is present.
    //!
    //! The fake's `dispose_ref` records `(instance, thread, handle)`, which is
    //! what makes the thread-affine claim checkable rather than asserted: a
    //! reference minted inside worker 2's isolate must be released by worker
    //! 2's instance, on worker 2's thread.

    use super::*;
    use crate::executor::foreign_async::AffineWorkerPool;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    /// One observed disposal.
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct Disposal {
        /// Which extension instance was asked. Distinct per `init`, so it
        /// identifies a thread-affine worker's private instance.
        instance: usize,
        thread: std::thread::ThreadId,
        handle: u64,
    }

    mod finalizing_extension {
        use super::Disposal;
        use shape_abi_v1::{ErrorModel, LanguageRuntimeVTable, STATE_MODEL_STATEFUL_OPAQUE};
        use std::ffi::{c_char, c_void};
        use std::sync::Mutex;
        use std::sync::atomic::{AtomicUsize, Ordering};

        /// Every disposal this fake was asked to perform, in order.
        pub static DISPOSALS: Mutex<Vec<Disposal>> = Mutex::new(Vec::new());
        /// Hands each `init` a distinct instance identity.
        static NEXT_INSTANCE: AtomicUsize = AtomicUsize::new(1);

        pub fn reset() {
            DISPOSALS.lock().unwrap().clear();
        }

        pub fn disposals() -> Vec<Disposal> {
            DISPOSALS.lock().unwrap().clone()
        }

        unsafe extern "C" fn init(_config: *const u8, _len: usize) -> *mut c_void {
            NEXT_INSTANCE.fetch_add(1, Ordering::SeqCst) as *mut c_void
        }

        unsafe extern "C" fn dispose_ref(instance: *mut c_void, handle: u64) {
            DISPOSALS.lock().unwrap().push(Disposal {
                instance: instance as usize,
                thread: std::thread::current().id(),
                handle,
            });
        }

        unsafe extern "C" fn language_id(_instance: *mut c_void) -> *const c_char {
            c"python".as_ptr()
        }

        unsafe extern "C" fn free_buffer(ptr: *mut u8, len: usize) {
            if !ptr.is_null() {
                unsafe { drop(Vec::from_raw_parts(ptr, len, len)) };
            }
        }

        unsafe extern "C" fn drop_instance(_instance: *mut c_void) {}

        unsafe extern "C" fn shared(_instance: *mut c_void) -> u32 {
            shape_abi_v1::INSTANCE_CONCURRENCY_SHARED
        }

        unsafe extern "C" fn thread_affine(_instance: *mut c_void) -> u32 {
            shape_abi_v1::INSTANCE_CONCURRENCY_THREAD_AFFINE
        }

        const fn base(
            concurrency: unsafe extern "C" fn(*mut c_void) -> u32,
            disposer: Option<unsafe extern "C" fn(*mut c_void, u64)>,
        ) -> LanguageRuntimeVTable {
            LanguageRuntimeVTable {
                init: Some(init),
                register_types: None,
                compile: None,
                invoke: None,
                dispose_function: None,
                language_id: Some(language_id),
                get_lsp_config: None,
                free_buffer: Some(free_buffer),
                drop: Some(drop_instance),
                error_model: ErrorModel::Dynamic,
                get_shape_source: None,
                runtime_descriptor: None,
                state_model: STATE_MODEL_STATEFUL_OPAQUE,
                generate_stubs: None,
                instance_concurrency: Some(concurrency),
                dispose_ref: disposer,
                capabilities: None,
            }
        }

        /// Declares the shared model and a working disposer.
        pub static SHARED: LanguageRuntimeVTable = base(shared, Some(dispose_ref));
        /// Declares the thread-affine model and a working disposer.
        pub static AFFINE: LanguageRuntimeVTable = base(thread_affine, Some(dispose_ref));
        /// Mints nothing it can release — the shape a pre-#200 extension has.
        pub static WITHOUT_DISPOSER: LanguageRuntimeVTable = base(shared, None);
    }

    /// Serializes this module's access to the fake's statics.
    static FAKE_LOCK: Mutex<()> = Mutex::new(());

    fn runtime(vtable: &'static shape_abi_v1::LanguageRuntimeVTable) -> Arc<PluginLanguageRuntime> {
        Arc::new(
            PluginLanguageRuntime::new(vtable, &serde_json::Value::Null)
                .expect("the fake runtime initializes"),
        )
    }

    fn origin(produced_by: &str) -> ForeignRefOrigin {
        ForeignRefOrigin {
            language: "python".into(),
            foreign_type: "module".into(),
            produced_by: produced_by.into(),
        }
    }

    /// Wait until `predicate` holds over the recorded disposals, or fail.
    ///
    /// Worker disposal is asynchronous by design (see `AffineWorkerDisposer`),
    /// so the affine assertions need a settle point. Generous timeout: this is
    /// a correctness check, not a timing one.
    fn await_disposals(predicate: impl Fn(&[Disposal]) -> bool) -> Vec<Disposal> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let seen = finalizing_extension::disposals();
            if predicate(&seen) {
                return seen;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for the expected disposals; saw {seen:?}"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    // ── Tripwire 2: drop order and double-drop balance ────────────────────

    #[test]
    fn last_share_disposes_once_against_the_shared_instance() {
        let _guard = FAKE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        finalizing_extension::reset();
        let runtime = runtime(&finalizing_extension::SHARED);

        let slot = mint_foreign_ref(
            ForeignRefOwner::Instance(Arc::clone(&runtime)),
            &runtime,
            77,
            origin("load"),
        )
        .expect("a runtime that declares a disposer can mint references");

        let duplicate = slot.clone();
        drop(slot);
        assert!(
            finalizing_extension::disposals().is_empty(),
            "a surviving share must keep the foreign object alive"
        );

        drop(duplicate);
        let seen = finalizing_extension::disposals();
        assert_eq!(
            seen.iter().map(|d| d.handle).collect::<Vec<_>>(),
            vec![77],
            "the last share disposes exactly once — a double-drop would show twice"
        );
    }

    #[test]
    fn references_dispose_in_the_order_their_scopes_end() {
        let _guard = FAKE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        finalizing_extension::reset();
        let runtime = runtime(&finalizing_extension::SHARED);

        let outer = mint_foreign_ref(
            ForeignRefOwner::Instance(Arc::clone(&runtime)),
            &runtime,
            1,
            origin("outer"),
        )
        .unwrap();
        let inner = mint_foreign_ref(
            ForeignRefOwner::Instance(Arc::clone(&runtime)),
            &runtime,
            2,
            origin("inner"),
        )
        .unwrap();

        // Inner scope ends first, as it would lexically.
        drop(inner);
        drop(outer);

        assert_eq!(
            finalizing_extension::disposals()
                .iter()
                .map(|d| d.handle)
                .collect::<Vec<_>>(),
            vec![2, 1],
            "disposal order follows ownership, not mint order (ADR-010 §4)"
        );
    }

    // ── The hard part: thread-affine disposal reaches the OWNING worker ───

    #[test]
    fn a_worker_bound_reference_disposes_on_that_worker_and_no_other() {
        let _guard = FAKE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        finalizing_extension::reset();
        let template = runtime(&finalizing_extension::AFFINE);
        let pool = Arc::new(AffineWorkerPool::start("python", Arc::clone(&template)));

        // Two references, each declared to have been minted inside a different
        // worker's private isolate — the shape an offloaded call produces.
        let first = mint_foreign_ref(
            ForeignRefOwner::AffineWorker {
                pool: Arc::clone(&pool),
                worker: crate::executor::foreign_async::AffineWorkerId(0),
            },
            &template,
            10,
            origin("on_worker_0"),
        )
        .unwrap();
        let second = mint_foreign_ref(
            ForeignRefOwner::AffineWorker {
                pool: Arc::clone(&pool),
                worker: crate::executor::foreign_async::AffineWorkerId(1),
            },
            &template,
            20,
            origin("on_worker_1"),
        )
        .unwrap();
        let also_first = mint_foreign_ref(
            ForeignRefOwner::AffineWorker {
                pool: Arc::clone(&pool),
                worker: crate::executor::foreign_async::AffineWorkerId(0),
            },
            &template,
            11,
            origin("on_worker_0_again"),
        )
        .unwrap();

        drop(first);
        drop(second);
        drop(also_first);

        let seen = await_disposals(|d| d.len() == 3);
        let by_handle = |h: u64| {
            seen.iter()
                .find(|d| d.handle == h)
                .unwrap_or_else(|| panic!("handle {h} was never disposed; saw {seen:?}"))
        };
        let (w0, w1, w0_again) = (by_handle(10), by_handle(20), by_handle(11));

        assert_eq!(
            (w0.instance, w0.thread),
            (w0_again.instance, w0_again.thread),
            "two references bound to worker 0 must be released by ONE instance \
             on ONE thread — that instance is the isolate that minted them"
        );
        assert_ne!(
            w0.instance, w1.instance,
            "worker 1's reference must not be released by worker 0's instance: \
             the handle means nothing in another isolate"
        );
        assert_ne!(
            w0.thread, w1.thread,
            "each worker owns its instance on its own thread; a disposal that \
             crossed would be entering a runtime from the wrong thread"
        );
        assert_ne!(
            w0.thread,
            std::thread::current().id(),
            "the interpreter thread must not be the one calling into a \
             worker-owned instance"
        );
    }

    #[test]
    fn a_disposal_queued_at_shutdown_still_reaches_its_worker() {
        let _guard = FAKE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        finalizing_extension::reset();
        let template = runtime(&finalizing_extension::AFFINE);
        let pool = Arc::new(AffineWorkerPool::start("python", Arc::clone(&template)));

        let slot = mint_foreign_ref(
            ForeignRefOwner::AffineWorker {
                pool: Arc::clone(&pool),
                worker: crate::executor::foreign_async::AffineWorkerId(2),
            },
            &template,
            99,
            origin("at_shutdown"),
        )
        .unwrap();

        // Retire the reference and then immediately tear the pool down, the
        // order a program exiting straight after its last scope produces.
        drop(slot);
        drop(pool);

        let seen = await_disposals(|d| d.iter().any(|d| d.handle == 99));
        assert_eq!(
            seen.iter().filter(|d| d.handle == 99).count(),
            1,
            "the queued disposal is delivered before the worker exits, once"
        );
    }

    // ── Minting is refused when the extension could not release ───────────

    #[test]
    fn a_runtime_without_a_disposer_cannot_mint_references() {
        let _guard = FAKE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        finalizing_extension::reset();
        let runtime = runtime(&finalizing_extension::WITHOUT_DISPOSER);

        let err = mint_foreign_ref(
            ForeignRefOwner::Instance(Arc::clone(&runtime)),
            &runtime,
            5,
            origin("load"),
        )
        .expect_err("a runtime that cannot release must not be allowed to mint");

        let message = err.to_string();
        assert!(
            message.contains("dispose_ref"),
            "the refusal names the missing vtable entry: {message}"
        );
        assert!(
            message.contains("a python module returned by `load`"),
            "the refusal names the value and its origin (ADR-019 §3): {message}"
        );
        assert!(
            finalizing_extension::disposals().is_empty(),
            "nothing was minted, so nothing is disposed"
        );
    }

    // ── The join with #202: who owns what a call returned ────────────────

    #[test]
    fn an_offloaded_affine_call_binds_its_reference_to_the_worker_that_ran_it() {
        let _guard = FAKE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let template = runtime(&finalizing_extension::AFFINE);
        let pool = Arc::new(AffineWorkerPool::start("python", Arc::clone(&template)));
        let strategy = ForeignOffload::Affine(Arc::clone(&pool));
        let (_tx, rx) = std::sync::mpsc::channel();
        let in_flight = InFlightForeignCall {
            completion: rx,
            abort: None,
            origin_worker: Some(crate::executor::foreign_async::AffineWorkerId(3)),
        };

        match owner_for_call(&template, Some(&strategy), Some(&in_flight)) {
            ForeignRefOwner::AffineWorker { worker, .. } => assert_eq!(
                worker,
                crate::executor::foreign_async::AffineWorkerId(3),
                "the reference must follow the worker the call actually ran on"
            ),
            ForeignRefOwner::Instance(_) => panic!(
                "an affine call's result belongs to the worker's isolate, not \
                 to the interpreter thread's instance"
            ),
        }
    }

    #[test]
    fn a_synchronous_call_binds_its_reference_to_the_runtime_instance() {
        let _guard = FAKE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let runtime = runtime(&finalizing_extension::SHARED);

        // No offload strategy and no in-flight call: the ordinary sync path.
        match owner_for_call(&runtime, None, None) {
            ForeignRefOwner::Instance(_) => {}
            ForeignRefOwner::AffineWorker { .. } => {
                panic!("a synchronous call never ran on a worker")
            }
        }
    }

    #[test]
    fn a_shared_offload_binds_to_the_instance_because_there_is_no_worker() {
        let _guard = FAKE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let runtime = runtime(&finalizing_extension::SHARED);
        let (_tx, rx) = std::sync::mpsc::channel();
        let in_flight = InFlightForeignCall {
            completion: rx,
            abort: None,
            origin_worker: None,
        };

        match owner_for_call(&runtime, Some(&ForeignOffload::Shared), Some(&in_flight)) {
            ForeignRefOwner::Instance(_) => {}
            ForeignRefOwner::AffineWorker { .. } => {
                panic!("the shared strategy has one instance and no workers to address")
            }
        }
    }

    /// The counter exists so the affine test can tell instances apart; if it
    /// ever stopped handing out distinct identities that test would pass
    /// vacuously.
    #[test]
    fn the_fake_hands_each_instance_a_distinct_identity() {
        let _guard = FAKE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let first = runtime(&finalizing_extension::SHARED);
        let second = first.fresh_instance().expect("a second instance builds");
        let seen = AtomicUsize::new(0);
        seen.fetch_add(1, Ordering::Relaxed);
        assert_eq!(first.language_id(), second.language_id());
        // Distinctness is observed through disposal, which is the only place
        // the instance pointer is visible to this module.
        finalizing_extension::reset();
        first.dispose_ref(1);
        second.dispose_ref(2);
        let seen = finalizing_extension::disposals();
        assert_ne!(
            seen[0].instance, seen[1].instance,
            "two instances must be distinguishable, or the affine routing test \
             proves nothing"
        );
    }
}
