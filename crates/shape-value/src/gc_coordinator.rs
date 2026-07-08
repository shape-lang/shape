//! GC Phase 3b — cross-worker stop-the-world safepoint rendezvous.
//!
//! real-gc-cycle-collection.md §0 ratification #2 (MT-rendezvous-required) /
//! R1-RESOLVED (owner ruling 2026-07-07: build the full cross-worker STW now) /
//! R4 sub-phase 3b. Entirely gated behind the default-off `gc` Cargo feature —
//! feature-off this module compiles to nothing, so the safepoint poll is
//! byte-identical to the pre-3b build and carries zero new hot-path cost.
//!
//! ## What this is
//!
//! Phase 3a shipped [`crate::gc::collect_cycles`] running at a **same-thread**
//! safepoint. Phase 3b wraps that unchanged single-thread engine in a
//! **rendezvous**: before a thread runs `collect_cycles` it stops every *other*
//! registered mutator at its safepoint, so no mutator can touch any heap while
//! the collector runs. Collection stays per-heap (today's heaps are
//! thread-disjoint — see design R0/S1), but the halt-all-mutators protocol is
//! real, so the collector is already sound the moment real cross-thread
//! `Arc<HeapValue>` sharing lands.
//!
//! ## The protocol (design R1-RESOLVED)
//!
//! A process-global [`GcCoordinator`] (a `OnceLock` static) holds:
//! - `stop_requested: AtomicBool` — the one byte
//!   [`JITContext.gc_safepoint_flag_ptr`] points at and the VM `& 0x3FF`
//!   safepoint polls. Raised by the collection initiator across the stop.
//! - `state: Mutex<{ registered, parked }>` + an **ack** `Condvar` (a mutator
//!   signals it when it parks / deregisters, so the initiator re-checks the ack
//!   condition) + a **resume** `Condvar` (the initiator signals it to wake
//!   parked mutators).
//! - `collector_lock: Mutex<()>` — serializes collections; a failed `try_lock`
//!   means another thread is already collecting, so the second initiator no-ops.
//!   Prevents nested / concurrent collection.
//!
//! Each mutator thread (the main VM plus every isolated async task VM — they all
//! run the same `dispatch.rs` loop) registers on its **first safepoint** via a
//! thread-local RAII [`MutatorHandle`] and deregisters when the handle's `Drop`
//! runs at thread exit. That RAII deregister is exactly what makes
//! exit-mid-rendezvous clean: a thread that exits mid-stop lowers the ack target.
//!
//! At each safepoint a thread does one **relaxed load** of `stop_requested`:
//! - set   → [`park_at_safepoint`] (increment `parked`, notify ack, wait on
//!   resume until the stop clears, decrement `parked`);
//! - clear → if its own candidate buffer exceeds the quantum it becomes the
//!   initiator via [`collect_under_stop`].
//!
//! The initiator `try_lock`s `collector_lock`, raises `stop_requested`, waits on
//! ack until `parked == registered - 1` (all others parked), runs the UNCHANGED
//! `collect_cycles()` under the global stop, then clears `stop_requested`, wakes
//! resume, and releases the lock. A lone registered thread has `registered - 1 ==
//! 0`, so `parked == 0` is satisfied immediately — a trivial self-rendezvous for
//! the common single-VM case.
//!
//! ## No-deadlock policy (design S2 / hard no-deadlock constraint)
//!
//! A thread that never reaches a safepoint (blocked on IO, a channel `recv`, or a
//! long FFI/polyglot call) must not stall the rendezvous forever. Two mechanisms:
//! 1. **GC-safe regions** ([`GcSafeRegion`]) — a thread wraps a known-blocking
//!    call in this RAII guard, which marks it pre-parked (JNI at-safepoint-region
//!    style). A thread that mutates no heap while blocked counts as already
//!    acked and never delays the rendezvous; on region exit it re-checks the
//!    stop and waits it out before touching the heap again.
//! 2. **Bounded ack wait** — as a backstop for any thread that neither
//!    safepoints nor enters a safe region, the initiator's ack wait is bounded
//!    ([`STW_ACK_TIMEOUT`]); on timeout it ABORTS (clears the stop, wakes any
//!    parked threads, returns 0 freed) rather than block forever. This is always
//!    safe because collection is memory-only and unobservable (§0 #1): a skipped
//!    cycle only defers reclamation to the next safepoint — never a safety loss,
//!    never a deadlock.
//!
//! ADR-006: the non-safepointing-thread deadlock policy (GC-safe regions +
//! bounded-wait-then-abort) is the one in-envelope implementation choice, taken
//! per the hard no-deadlock constraint and consistent with design S2.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Bounded wait for the ack barrier before the initiator aborts a collection.
///
/// Long enough that any thread cooperatively reaching a safepoint (VM loop every
/// ≤1024 instructions; JIT every loop back-edge) always parks in time; short
/// enough that a truly-stuck non-safepointing thread only defers one collection
/// quantum rather than hanging the process. Abort is safe (memory-only, §0 #1).
const STW_ACK_TIMEOUT: Duration = Duration::from_millis(500);

/// Rendezvous accounting, guarded by [`GcCoordinator::state`].
struct CoordState {
    /// Live mutator threads (each holds a [`MutatorHandle`]).
    registered: usize,
    /// Threads currently at a safepoint-park **or** inside a [`GcSafeRegion`]
    /// (both count as "not mutating any heap right now").
    parked: usize,
}

/// Process-global stop-the-world coordinator. One instance, in [`coordinator`].
pub struct GcCoordinator {
    /// The stop-request flag. Hot-path readers load it Relaxed; the initiator
    /// stores it SeqCst around the ack/resume handshake. The `JITContext`
    /// safepoint pointer aliases this byte.
    stop_requested: AtomicBool,
    state: Mutex<CoordState>,
    /// Signalled by a parking / deregistering mutator so the initiator re-checks
    /// `parked == registered - 1`.
    ack: Condvar,
    /// Signalled by the initiator on resume (stop cleared) to release parked
    /// mutators.
    resume: Condvar,
    /// Serializes collections: a failed `try_lock` ⇒ another collection is in
    /// progress ⇒ this initiator no-ops (no nested / concurrent collection).
    collector_lock: Mutex<()>,
}

static COORDINATOR: OnceLock<GcCoordinator> = OnceLock::new();

/// The process-global coordinator, lazily created on first use.
#[inline]
pub fn coordinator() -> &'static GcCoordinator {
    COORDINATOR.get_or_init(|| GcCoordinator {
        stop_requested: AtomicBool::new(false),
        state: Mutex::new(CoordState {
            registered: 0,
            parked: 0,
        }),
        ack: Condvar::new(),
        resume: Condvar::new(),
        collector_lock: Mutex::new(()),
    })
}

/// Raw pointer to the stop-request flag byte, for
/// `JITContext.gc_safepoint_flag_ptr`. The `AtomicBool` lives in the `'static`
/// coordinator, so the pointer is stable for the program's lifetime; reading it
/// as a `u8` yields 0 (clear) / 1 (set).
#[inline]
pub fn stop_flag_ptr() -> *const u8 {
    &coordinator().stop_requested as *const AtomicBool as *const u8
}

/// Hot-path safepoint poll: a single **relaxed** load of the stop flag. Cheap
/// enough to sit on the VM `& 0x3FF` gate; parking machinery is only touched
/// when this returns `true` (a stop is in progress — rare).
#[inline]
pub fn stop_requested() -> bool {
    coordinator().stop_requested.load(Ordering::Relaxed)
}

// ── Registration (RAII) ─────────────────────────────────────────────────────

/// RAII registration token for one mutator thread. `register` bumps the live
/// mutator count; `Drop` (at thread exit, when the thread-local is torn down)
/// decrements it and wakes the initiator so a thread exiting mid-rendezvous
/// lowers the ack target instead of hanging it.
struct MutatorHandle;

impl MutatorHandle {
    fn register() -> Self {
        let c = coordinator();
        let mut st = c.state.lock().unwrap();
        st.registered += 1;
        MutatorHandle
    }
}

impl Drop for MutatorHandle {
    fn drop(&mut self) {
        let c = coordinator();
        let mut st = c.state.lock().unwrap();
        // Saturating: register/deregister are balanced in production; the
        // saturating floor only matters against the test-only `test_reset`.
        st.registered = st.registered.saturating_sub(1);
        // Lower the ack target: an initiator waiting on `parked == registered-1`
        // must re-evaluate now that `registered` dropped.
        c.ack.notify_all();
        drop(st);
    }
}

thread_local! {
    /// This thread's registration token. `None` until the first safepoint
    /// registers it; dropped (deregistered) when the thread exits.
    static HANDLE: std::cell::RefCell<Option<MutatorHandle>> =
        const { std::cell::RefCell::new(None) };
}

/// Register the calling thread as a GC mutator on its first safepoint
/// (idempotent, cheap after the first call). Kept out of the returned-value hot
/// path — only the safepoint entry calls it.
#[inline]
pub fn ensure_registered() {
    HANDLE.with(|h| {
        let mut slot = h.borrow_mut();
        if slot.is_none() {
            *slot = Some(MutatorHandle::register());
        }
    });
}

// ── Parking (the stop-request slow path) ────────────────────────────────────

/// Park the calling thread at its safepoint until the in-progress global stop
/// clears. Increments `parked` (acking the initiator), waits on resume, then
/// decrements `parked`. Early-returns if the stop already cleared between the
/// caller's flag poll and acquiring the state lock.
pub fn park_at_safepoint() {
    let c = coordinator();
    let mut st = c.state.lock().unwrap();
    if !c.stop_requested.load(Ordering::Relaxed) {
        // The stop cleared in the race window — nothing to park for.
        return;
    }
    st.parked += 1;
    c.ack.notify_all();
    while c.stop_requested.load(Ordering::Relaxed) {
        st = c.resume.wait(st).unwrap();
    }
    st.parked = st.parked.saturating_sub(1);
}

/// JIT back-edge park entry (called from `jit_gc_safepoint` when the aliased
/// stop flag byte is set). Registers the thread (idempotent — the JIT runs
/// inline on the VM thread, already registered) then parks.
#[inline]
pub fn jit_safepoint_park() {
    ensure_registered();
    park_at_safepoint();
}

// ── Collection under the global stop (the initiator path) ───────────────────

/// Become the collection initiator: stop every other registered mutator at its
/// safepoint, run `collect` under the global stop, then resume all mutators.
/// Returns whatever `collect` returned (the freed-node count), or `0` if another
/// collection was already in progress (serialized by `collector_lock`) or the
/// ack barrier timed out (a non-safepointing thread — collection aborted, which
/// is safe: memory-only, retried next safepoint).
///
/// `collect` runs with `parked == registered - 1` guaranteed — i.e. no other
/// registered mutator is running — so it may touch the heap freely.
#[inline]
pub fn collect_under_stop(collect: impl FnOnce() -> usize) -> usize {
    collect_under_stop_bounded(collect, STW_ACK_TIMEOUT)
}

/// [`collect_under_stop`] with an explicit ack-wait bound (test hook + the
/// single real caller which passes [`STW_ACK_TIMEOUT`]).
pub fn collect_under_stop_bounded(collect: impl FnOnce() -> usize, ack_timeout: Duration) -> usize {
    let c = coordinator();

    // Serialize collections. A failed try_lock ⇒ another thread is already the
    // initiator (or this thread re-entered) ⇒ no-op. try_lock never blocks, so
    // re-entry from the same thread cannot deadlock.
    let _collecting = match c.collector_lock.try_lock() {
        Ok(g) => g,
        Err(_) => return 0,
    };

    // Raise the global stop. SeqCst pairs with the mutators' polls and the
    // state-lock handshake below.
    c.stop_requested.store(true, Ordering::SeqCst);

    // Ack barrier: wait until every OTHER registered mutator is parked (or in a
    // GC-safe region, which is counted in `parked`). Bounded — abort on timeout.
    let acked = {
        let mut st = c.state.lock().unwrap();
        let deadline = Instant::now() + ack_timeout;
        loop {
            // Target excludes the initiator itself.
            let target = st.registered.saturating_sub(1);
            if st.parked >= target {
                break true;
            }
            let now = Instant::now();
            if now >= deadline {
                break false;
            }
            let (guard, _timed_out) = c.ack.wait_timeout(st, deadline - now).unwrap();
            st = guard;
        }
    };

    // Run the unchanged single-thread collector under the global stop. On abort
    // (a non-safepointing thread never parked) skip it — a deferred cycle, not a
    // safety loss.
    let freed = if acked { collect() } else { 0 };

    // Resume: clear the stop and wake every parked mutator. Acquire the state
    // lock before notifying so a mutator that is between `parked += 1` and
    // `resume.wait` cannot miss the wakeup.
    c.stop_requested.store(false, Ordering::SeqCst);
    {
        let _st = c.state.lock().unwrap();
        c.resume.notify_all();
    }
    freed
}

// ── GC-safe regions (the no-deadlock policy, blocking-call half) ─────────────

/// RAII guard marking the calling thread **GC-safe** for the duration of a
/// known-blocking native call (channel `recv`, sleep, long FFI/polyglot). While
/// the guard is live the thread counts as parked (it mutates no heap), so it
/// never delays the ack barrier. On drop it waits out any collection in progress
/// before rejoining as an active mutator, so it cannot touch the heap
/// mid-collection.
///
/// Not yet wired into the runtime's blocking sites (there is no cross-thread
/// shared heap today — design R0). Shipped now so those sites can adopt it the
/// moment cross-thread heap sharing lands, and covered by the rendezvous tests.
#[must_use = "the thread stays GC-safe only while the guard is live"]
pub struct GcSafeRegion {
    _priv: (),
}

impl GcSafeRegion {
    /// Enter a GC-safe region: register (idempotent) and count this thread as
    /// parked so the rendezvous does not wait on it while it blocks.
    pub fn enter() -> Self {
        ensure_registered();
        let c = coordinator();
        let mut st = c.state.lock().unwrap();
        st.parked += 1;
        c.ack.notify_all();
        drop(st);
        GcSafeRegion { _priv: () }
    }
}

impl Drop for GcSafeRegion {
    fn drop(&mut self) {
        let c = coordinator();
        let mut st = c.state.lock().unwrap();
        // Wait out any collection in progress BEFORE dropping our parked status
        // — we are about to touch the heap again and must not race the collector.
        while c.stop_requested.load(Ordering::Relaxed) {
            st = c.resume.wait(st).unwrap();
        }
        st.parked = st.parked.saturating_sub(1);
    }
}

// ── Test hooks ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod test_hooks {
    use super::*;

    /// Live registered-mutator count.
    pub(super) fn registered_count() -> usize {
        coordinator().state.lock().unwrap().registered
    }

    /// Currently-parked count.
    pub(super) fn parked_count() -> usize {
        coordinator().state.lock().unwrap().parked
    }

    /// Force the coordinator back to a clean baseline between hermetic tests
    /// (all worker threads already joined ⇒ their handles dropped). Belt-and-
    /// suspenders around the natural RAII cleanup.
    pub(super) fn test_reset() {
        let c = coordinator();
        c.stop_requested.store(false, Ordering::SeqCst);
        let mut st = c.state.lock().unwrap();
        st.registered = 0;
        st.parked = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::test_hooks::*;
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex as StdMutex};
    use std::thread;
    use std::time::Duration;

    /// Serializes the rendezvous tests — they share the one process-global
    /// coordinator, so they must not run concurrently.
    static TEST_SERIAL: StdMutex<()> = StdMutex::new(());

    /// Spin until `cond` holds or `bound` elapses; returns whether it held.
    fn wait_until(bound: Duration, mut cond: impl FnMut() -> bool) -> bool {
        let deadline = Instant::now() + bound;
        while Instant::now() < deadline {
            if cond() {
                return true;
            }
            thread::yield_now();
        }
        cond()
    }

    /// A cooperative worker: registers, signals ready, then loops hitting the
    /// safepoint (parking on a stop) and bumping its own progress counter until
    /// told to quit. Its handle deregisters when this closure returns.
    struct Worker {
        progress: Arc<AtomicU64>,
        quit: Arc<AtomicBool>,
        handle: thread::JoinHandle<()>,
    }

    fn spawn_worker(ready: Arc<AtomicUsize>) -> Worker {
        let progress = Arc::new(AtomicU64::new(0));
        let quit = Arc::new(AtomicBool::new(false));
        let p = Arc::clone(&progress);
        let q = Arc::clone(&quit);
        let handle = thread::spawn(move || {
            ensure_registered();
            ready.fetch_add(1, Ordering::SeqCst);
            while !q.load(Ordering::Relaxed) {
                // Cooperative safepoint: park iff a stop is requested.
                if stop_requested() {
                    park_at_safepoint();
                }
                p.fetch_add(1, Ordering::Relaxed);
                thread::yield_now();
            }
        });
        Worker {
            progress,
            quit,
            handle,
        }
    }

    fn stop_and_join(workers: Vec<Worker>) {
        for w in &workers {
            w.quit.store(true, Ordering::Relaxed);
        }
        for w in workers {
            w.handle.join().unwrap();
        }
    }

    /// **Core rendezvous.** N cooperative workers all park under a stop; the
    /// collector runs exactly once with every worker frozen; all workers resume
    /// and make progress after. Proves request-stop → ack barrier → collect →
    /// resume.
    #[test]
    fn n_workers_park_collect_once_and_resume() {
        let _serial = TEST_SERIAL.lock().unwrap();
        test_reset();

        const N: usize = 6;
        let ready = Arc::new(AtomicUsize::new(0));
        let workers: Vec<Worker> = (0..N).map(|_| spawn_worker(Arc::clone(&ready))).collect();

        // Wait until all N have registered (so target == N under the initiator).
        assert!(
            wait_until(Duration::from_secs(5), || ready.load(Ordering::SeqCst) == N),
            "all workers registered"
        );

        // Initiator on its own thread so its handle deregisters on exit (keeps
        // the coordinator clean for the next test).
        let progresses: Vec<Arc<AtomicU64>> = workers.iter().map(|w| Arc::clone(&w.progress)).collect();
        let collect_calls = Arc::new(AtomicUsize::new(0));
        let cc = Arc::clone(&collect_calls);
        let init = thread::spawn(move || {
            ensure_registered();
            // registered == N + 1 (workers + initiator); target == N.
            assert!(
                wait_until(Duration::from_secs(5), || registered_count() == N + 1),
                "initiator + N workers registered"
            );
            collect_under_stop(move || {
                cc.fetch_add(1, Ordering::SeqCst);
                // Under the stop every OTHER mutator is parked.
                assert_eq!(parked_count(), N, "all N workers parked under the stop");
                // Workers are frozen: progress does not advance while stopped.
                let before: Vec<u64> = progresses.iter().map(|p| p.load(Ordering::Relaxed)).collect();
                thread::sleep(Duration::from_millis(30));
                let after: Vec<u64> = progresses.iter().map(|p| p.load(Ordering::Relaxed)).collect();
                assert_eq!(before, after, "workers make no progress while stopped");
                4242 // sentinel freed-count
            })
        });

        let freed = init.join().unwrap();
        assert_eq!(freed, 4242, "collect return propagates through the rendezvous");
        assert_eq!(collect_calls.load(Ordering::SeqCst), 1, "collected exactly once");

        // After resume the workers advance again.
        let snap: Vec<u64> = workers.iter().map(|w| w.progress.load(Ordering::Relaxed)).collect();
        assert!(
            wait_until(Duration::from_secs(5), || {
                workers
                    .iter()
                    .zip(&snap)
                    .all(|(w, &s)| w.progress.load(Ordering::Relaxed) > s)
            }),
            "every worker resumes and makes progress"
        );

        stop_and_join(workers);
        test_reset();
    }

    /// **Trivial single-thread rendezvous.** A lone registered thread self-
    /// rendezvouses (`registered - 1 == 0`) and collects immediately — the
    /// common single-VM case.
    #[test]
    fn lone_thread_self_rendezvous_collects_immediately() {
        let _serial = TEST_SERIAL.lock().unwrap();
        test_reset();

        let init = thread::spawn(|| {
            ensure_registered();
            assert_eq!(registered_count(), 1);
            collect_under_stop(|| {
                assert_eq!(parked_count(), 0, "no others to park");
                7 // sentinel
            })
        });
        assert_eq!(init.join().unwrap(), 7, "lone thread collects without waiting");

        test_reset();
    }

    /// **Clean deregister on exit mid-rendezvous.** A worker that exits (drops
    /// its handle) instead of parking lowers the ack target, so the barrier
    /// still completes and the collector still runs.
    #[test]
    fn worker_exit_mid_rendezvous_does_not_deadlock() {
        let _serial = TEST_SERIAL.lock().unwrap();
        test_reset();

        let ready = Arc::new(AtomicUsize::new(0));
        // Two cooperative workers that park...
        let coop: Vec<Worker> = (0..2).map(|_| spawn_worker(Arc::clone(&ready))).collect();
        // ...and one "deserter" that exits the moment it observes the stop.
        let d_ready = Arc::clone(&ready);
        let deserter = thread::spawn(move || {
            ensure_registered();
            d_ready.fetch_add(1, Ordering::SeqCst);
            loop {
                if stop_requested() {
                    return; // exit instead of parking → handle drops → deregister
                }
                thread::yield_now();
            }
        });

        assert!(
            wait_until(Duration::from_secs(5), || ready.load(Ordering::SeqCst) == 3),
            "all three registered"
        );

        let init = thread::spawn(|| {
            ensure_registered();
            assert!(
                wait_until(Duration::from_secs(5), || registered_count() == 4),
                "initiator + 3 registered"
            );
            // 3 others registered, but the deserter exits rather than parks; its
            // deregister drops the target from 3 to 2, which the 2 coop workers
            // meet. Must complete (bounded) and RUN the collect (not abort).
            collect_under_stop(|| 99)
        });

        let freed = init.join().unwrap();
        deserter.join().unwrap();
        assert_eq!(freed, 99, "barrier completed and collected despite the exit");

        stop_and_join(coop);
        test_reset();
    }

    /// **Exactly one collector under concurrent initiators.** Two threads race
    /// to initiate; `collector_lock` admits exactly one, the other no-ops. No
    /// double-collect, no hang.
    #[test]
    fn two_initiators_yield_exactly_one_collection() {
        let _serial = TEST_SERIAL.lock().unwrap();
        test_reset();

        let ready = Arc::new(AtomicUsize::new(0));
        let workers: Vec<Worker> = (0..3).map(|_| spawn_worker(Arc::clone(&ready))).collect();
        assert!(
            wait_until(Duration::from_secs(5), || ready.load(Ordering::SeqCst) == 3),
            "workers registered"
        );

        let collect_calls = Arc::new(AtomicUsize::new(0));
        let start = Arc::new(std::sync::Barrier::new(2));

        let mut inits = Vec::new();
        for _ in 0..2 {
            let cc = Arc::clone(&collect_calls);
            let b = Arc::clone(&start);
            inits.push(thread::spawn(move || {
                ensure_registered();
                b.wait(); // maximise the race
                collect_under_stop(move || {
                    cc.fetch_add(1, Ordering::SeqCst);
                    thread::sleep(Duration::from_millis(15));
                    1
                })
            }));
        }

        let results: Vec<usize> = inits.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(
            collect_calls.load(Ordering::SeqCst),
            1,
            "collector_lock admitted exactly one collection"
        );
        // Exactly one initiator ran the collect (returned 1); the other no-oped (0).
        let mut sorted = results.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, vec![0, 1], "one collector, one no-op");

        stop_and_join(workers);
        test_reset();
    }

    /// **Bounded wait / abort on a non-safepointing thread.** A registered
    /// worker that blocks WITHOUT ever reaching a safepoint must not hang the
    /// rendezvous: the initiator times out and ABORTS (clears the stop, returns
    /// 0, does not run collect).
    #[test]
    fn non_safepointing_thread_times_out_and_aborts() {
        let _serial = TEST_SERIAL.lock().unwrap();
        test_reset();

        let ready = Arc::new(AtomicUsize::new(0));
        let quit = Arc::new(AtomicBool::new(false));
        let r = Arc::clone(&ready);
        let q = Arc::clone(&quit);
        // Registers, then busy-loops ignoring the stop flag entirely.
        let blocker = thread::spawn(move || {
            ensure_registered();
            r.fetch_add(1, Ordering::SeqCst);
            while !q.load(Ordering::Relaxed) {
                thread::yield_now();
            }
        });

        assert!(
            wait_until(Duration::from_secs(5), || ready.load(Ordering::SeqCst) == 1),
            "blocker registered"
        );

        let collect_calls = Arc::new(AtomicUsize::new(0));
        let cc = Arc::clone(&collect_calls);
        let init = thread::spawn(move || {
            ensure_registered();
            assert!(
                wait_until(Duration::from_secs(5), || registered_count() == 2),
                "initiator + blocker registered"
            );
            // Short bound so the abort is fast; the blocker never parks.
            collect_under_stop_bounded(
                move || {
                    cc.fetch_add(1, Ordering::SeqCst);
                    5
                },
                Duration::from_millis(120),
            )
        });

        let freed = init.join().unwrap();
        assert_eq!(freed, 0, "aborted collection returns 0");
        assert_eq!(collect_calls.load(Ordering::SeqCst), 0, "collect did not run");
        assert!(!stop_requested(), "abort cleared the stop flag");

        quit.store(true, Ordering::Relaxed);
        blocker.join().unwrap();
        test_reset();
    }

    /// **A GC-safe region counts as parked.** A worker that enters a
    /// [`GcSafeRegion`] and then blocks (never polling the stop) must NOT cause
    /// the initiator to time out — the region marks it pre-parked, so the
    /// barrier completes and the collector runs.
    #[test]
    fn gc_safe_region_counts_as_parked() {
        let _serial = TEST_SERIAL.lock().unwrap();
        test_reset();

        let ready = Arc::new(AtomicUsize::new(0));
        let release = Arc::new(AtomicBool::new(false));
        let r = Arc::clone(&ready);
        let rel = Arc::clone(&release);
        // Enters a GC-safe region and blocks there (no safepoint polling).
        let blocker = thread::spawn(move || {
            ensure_registered();
            let region = GcSafeRegion::enter();
            r.fetch_add(1, Ordering::SeqCst);
            while !rel.load(Ordering::Relaxed) {
                thread::yield_now();
            }
            drop(region); // exits the region (waits out any live collection)
        });

        assert!(
            wait_until(Duration::from_secs(5), || ready.load(Ordering::SeqCst) == 1),
            "blocker in its safe region"
        );

        let collect_calls = Arc::new(AtomicUsize::new(0));
        let cc = Arc::clone(&collect_calls);
        let init = thread::spawn(move || {
            ensure_registered();
            assert!(
                wait_until(Duration::from_secs(5), || registered_count() == 2),
                "initiator + blocker registered"
            );
            // Blocker is in a GC-safe region ⇒ counts as parked ⇒ no timeout even
            // with a short bound.
            collect_under_stop_bounded(
                move || {
                    cc.fetch_add(1, Ordering::SeqCst);
                    assert_eq!(parked_count(), 1, "the safe-region thread is counted parked");
                    3
                },
                Duration::from_millis(250),
            )
        });

        let freed = init.join().unwrap();
        assert_eq!(freed, 3, "safe region let the barrier complete — collect ran");
        assert_eq!(collect_calls.load(Ordering::SeqCst), 1);

        release.store(true, Ordering::Relaxed);
        blocker.join().unwrap();
        test_reset();
    }
}
