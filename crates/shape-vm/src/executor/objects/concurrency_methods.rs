//! Method handlers for concurrency primitive types: Mutex<T>, Atomic<T>, Lazy<T>
//!
//! ## W17-concurrency rebuild (2026-05-11)
//!
//! Per ADR-006 §2.7.25 amendment (Wave 17 W17-concurrency), the
//! Mutex / Atomic / Lazy carriers are typed-`Arc<MutexData>` /
//! `Arc<AtomicData>` / `Arc<LazyData>`-backed `HeapValue` arms — full
//! HeapValue arms, mirror of the §2.7.20 Channel rebuild structure.
//!
//! Receiver dispatch follows §2.7.6 / Q8: kind check on `args[0].kind ==
//! NativeKind::Ptr(HeapKind::Mutex/Atomic/Lazy)`, then a typed-Arc
//! recovery via the canonical `Arc::from_raw → clone → into_raw`
//! pattern (post-`3ac2f11` 5-arm receiver-recovery soundness rule).
//!
//! ## Method surface
//!
//! ### Mutex<T>
//! - `lock()` — at landing a no-op marker (single-threaded VM); when
//!   the runtime grows real concurrency, this is the acquire point.
//!   Returns the mutex itself for chained access.
//! - `try_lock()` — returns `true` (always succeeds at landing).
//! - `set(value)` — replace the wrapped value. Prior slot drops.
//!
//! ### Atomic<i64>
//! - `load()` — read the current value (SeqCst).
//! - `store(v)` — write a value (SeqCst). Returns the atomic itself.
//! - `fetch_add(delta)` — atomic add, returns prior value.
//! - `fetch_sub(delta)` — atomic sub, returns prior value.
//! - `compare_exchange(expected, new)` — returns prior value.
//!
//! ### Lazy<T>
//! - `get()` — return cached value or run initializer closure once.
//!   The closure-call path goes through `vm.call_value_immediate_nb`
//!   (W17-make-closure partial-gate, merged at `aa47364`).
//! - `is_initialized()` — bool whether `get()` has cached a value.
//!
//! ADR-006 §2.7.25 + Wave 2.5 W17-concurrency.

use crate::executor::VirtualMachine;
use shape_runtime::context::ExecutionContext;
use shape_value::heap_value::{AtomicData, HeapKind, LazyData, MutexData};
use shape_value::{KindedSlot, NativeKind, VMError};
use std::sync::Arc;

// ── Local helpers ─────────────────────────────────────────────────────────

#[inline]
fn type_error(msg: impl Into<String>) -> VMError {
    VMError::RuntimeError(msg.into())
}

/// Project the receiver `KindedSlot` to an owned `Arc<MutexData>` via
/// the §2.7.6 / Q8 canonical typed-Arc recovery pattern (post-`3ac2f11`
/// 5-arm soundness rule). The receiver retains its share — the
/// returned `Arc` is an independent strong-count owner.
#[inline]
fn as_mutex(slot: &KindedSlot) -> Result<Arc<MutexData>, VMError> {
    if !matches!(slot.kind, NativeKind::Ptr(HeapKind::Mutex)) {
        return Err(type_error(format!(
            "Mutex method receiver must be a Mutex (got kind {:?})",
            slot.kind
        )));
    }
    let bits = slot.slot.raw();
    if bits == 0 {
        return Err(type_error("Mutex method receiver slot bits null"));
    }
    // SAFETY: per the construction-side contract on
    // `KindedSlot::from_mutex`, `Mutex`-kind bits are
    // `Arc::into_raw(Arc<MutexData>)` and the slot owns one strong-
    // count share. Reconstruct, clone, restore.
    let arc = unsafe { Arc::<MutexData>::from_raw(bits as *const MutexData) };
    let cloned = Arc::clone(&arc);
    let _ = Arc::into_raw(arc);
    Ok(cloned)
}

/// Project the receiver `KindedSlot` to an owned `Arc<AtomicData>`.
/// Same shape as `as_mutex`.
#[inline]
fn as_atomic(slot: &KindedSlot) -> Result<Arc<AtomicData>, VMError> {
    if !matches!(slot.kind, NativeKind::Ptr(HeapKind::Atomic)) {
        return Err(type_error(format!(
            "Atomic method receiver must be an Atomic (got kind {:?})",
            slot.kind
        )));
    }
    let bits = slot.slot.raw();
    if bits == 0 {
        return Err(type_error("Atomic method receiver slot bits null"));
    }
    // SAFETY: per the construction-side contract on
    // `KindedSlot::from_atomic`.
    let arc = unsafe { Arc::<AtomicData>::from_raw(bits as *const AtomicData) };
    let cloned = Arc::clone(&arc);
    let _ = Arc::into_raw(arc);
    Ok(cloned)
}

/// Project the receiver `KindedSlot` to an owned `Arc<LazyData>`.
/// Same shape as `as_mutex`.
#[inline]
fn as_lazy(slot: &KindedSlot) -> Result<Arc<LazyData>, VMError> {
    if !matches!(slot.kind, NativeKind::Ptr(HeapKind::Lazy)) {
        return Err(type_error(format!(
            "Lazy method receiver must be a Lazy (got kind {:?})",
            slot.kind
        )));
    }
    let bits = slot.slot.raw();
    if bits == 0 {
        return Err(type_error("Lazy method receiver slot bits null"));
    }
    // SAFETY: per the construction-side contract on
    // `KindedSlot::from_lazy`.
    let arc = unsafe { Arc::<LazyData>::from_raw(bits as *const LazyData) };
    let cloned = Arc::clone(&arc);
    let _ = Arc::into_raw(arc);
    Ok(cloned)
}

/// Coerce a method-arg `KindedSlot` to `i64`. Used by Atomic.{store,
/// fetch_add, fetch_sub, compare_exchange} args.
#[inline]
fn arg_as_i64(slot: &KindedSlot, method: &str, idx: usize) -> Result<i64, VMError> {
    slot.as_i64().ok_or_else(|| {
        type_error(format!(
            "{}: argument {} must be an int (got kind {:?})",
            method, idx, slot.kind
        ))
    })
}

// ═══════════════════════════════════════════════════════════════════════════
// Mutex<T> handlers
// ═══════════════════════════════════════════════════════════════════════════

/// `mutex.lock()` — at landing a no-op marker (single-threaded VM).
/// Returns the mutex itself for chained access.
pub fn v2_mutex_lock(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 1 {
        return Err(type_error("Mutex.lock() takes no arguments"));
    }
    let m: Arc<MutexData> = as_mutex(&args[0])?;
    m.lock();
    Ok(KindedSlot::from_mutex(m))
}

/// `mutex.try_lock()` — at landing always returns `true`
/// (single-threaded VM; no contention to fail).
pub fn v2_mutex_try_lock(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 1 {
        return Err(type_error("Mutex.try_lock() takes no arguments"));
    }
    let m: Arc<MutexData> = as_mutex(&args[0])?;
    let ok = m.try_lock();
    Ok(KindedSlot::from_bool(ok))
}

/// `mutex.get()` — read the wrapped value. Returns a clone of the
/// inner `KindedSlot` (bumps any heap-bearing share).
///
/// Surface complement to `set()`: the playbook-listed smoke target
/// (`print(m.value)`) wants a read accessor. The surface-level `m.value`
/// property-access form requires GetProp dispatch on heap receivers —
/// out of scope for W17-concurrency; this method is the equivalent.
pub fn v2_mutex_get(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 1 {
        return Err(type_error("Mutex.get() takes no arguments"));
    }
    let m: Arc<MutexData> = as_mutex(&args[0])?;
    Ok(m.get())
}

/// `mutex.set(value)` — replace the wrapped value. Returns the mutex
/// itself.
pub fn v2_mutex_set(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 2 {
        return Err(type_error(
            "Mutex.set() requires exactly 1 argument (value)",
        ));
    }
    let m: Arc<MutexData> = as_mutex(&args[0])?;
    // Clone the value's KindedSlot — bumps refcount for heap shares,
    // no-op for inline scalars (Drop discipline preserved through
    // KindedSlot::Clone per §2.7.7).
    let new_value = args[1].clone();
    m.set(new_value);
    Ok(KindedSlot::from_mutex(m))
}

// ═══════════════════════════════════════════════════════════════════════════
// Atomic<i64> handlers
// ═══════════════════════════════════════════════════════════════════════════

/// `atomic.load()` — atomic read (SeqCst).
pub fn v2_atomic_load(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 1 {
        return Err(type_error("Atomic.load() takes no arguments"));
    }
    let a: Arc<AtomicData> = as_atomic(&args[0])?;
    let v = a.load();
    Ok(KindedSlot::from_int(v))
}

/// `atomic.store(value)` — atomic write (SeqCst). Returns the atomic
/// itself for chained access.
pub fn v2_atomic_store(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 2 {
        return Err(type_error(
            "Atomic.store() requires exactly 1 argument (value)",
        ));
    }
    let a: Arc<AtomicData> = as_atomic(&args[0])?;
    let v = arg_as_i64(&args[1], "Atomic.store", 1)?;
    a.store(v);
    Ok(KindedSlot::from_atomic(a))
}

/// `atomic.fetch_add(delta)` — atomic add (SeqCst). Returns the prior
/// value.
pub fn v2_atomic_fetch_add(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 2 {
        return Err(type_error(
            "Atomic.fetch_add() requires exactly 1 argument (delta)",
        ));
    }
    let a: Arc<AtomicData> = as_atomic(&args[0])?;
    let delta = arg_as_i64(&args[1], "Atomic.fetch_add", 1)?;
    let prior = a.fetch_add(delta);
    Ok(KindedSlot::from_int(prior))
}

/// `atomic.fetch_sub(delta)` — atomic sub (SeqCst). Returns the prior
/// value.
pub fn v2_atomic_fetch_sub(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 2 {
        return Err(type_error(
            "Atomic.fetch_sub() requires exactly 1 argument (delta)",
        ));
    }
    let a: Arc<AtomicData> = as_atomic(&args[0])?;
    let delta = arg_as_i64(&args[1], "Atomic.fetch_sub", 1)?;
    let prior = a.fetch_sub(delta);
    Ok(KindedSlot::from_int(prior))
}

/// `atomic.compare_exchange(expected, new)` — atomic compare-and-swap
/// (SeqCst). Returns the prior value (callers infer success by
/// comparing to `expected`).
pub fn v2_atomic_compare_exchange(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 3 {
        return Err(type_error(
            "Atomic.compare_exchange() requires exactly 2 arguments \
             (expected, new)",
        ));
    }
    let a: Arc<AtomicData> = as_atomic(&args[0])?;
    let expected = arg_as_i64(&args[1], "Atomic.compare_exchange", 1)?;
    let new_v = arg_as_i64(&args[2], "Atomic.compare_exchange", 2)?;
    let prior = a.compare_exchange(expected, new_v);
    Ok(KindedSlot::from_int(prior))
}

// ═══════════════════════════════════════════════════════════════════════════
// Lazy<T> handlers
// ═══════════════════════════════════════════════════════════════════════════

/// `lazy.get()` — run the initializer the first time and cache the
/// result; subsequent calls return the cached value.
///
/// Closure-call path goes through `vm.call_value_immediate_nb` per
/// ADR-006 §2.7.11 / Q12. The W17-make-closure partial-gate (merged
/// at `aa47364`) unlocked this re-entry shape; without it the
/// initializer could not be invoked from a method handler.
pub fn v2_lazy_get(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 1 {
        return Err(type_error("Lazy.get() takes no arguments"));
    }
    let l: Arc<LazyData> = as_lazy(&args[0])?;
    // Fast path: value already cached. KindedSlot::Clone bumps any
    // inner heap share.
    if let Some(cached) = l.cached() {
        return Ok(cached);
    }
    // Slow path: run the initializer. `take_initializer()` returns
    // `Some(closure_slot)` if not yet initialized — the closure share
    // transfers out of the cell into our local slot, and the call
    // through `call_value_immediate_nb` consumes it.
    let initializer = l.take_initializer().ok_or_else(|| {
        // Race-window guard: between cached()=None and
        // take_initializer()=None, another thread cached the value.
        // At single-threaded landing this branch is unreachable but
        // defensive for future concurrency. Recover by re-reading
        // the cache.
        type_error(
            "Lazy.get(): initializer already taken but value not \
             cached — concurrent get() race (single-threaded landing \
             treats as defensive bug)",
        )
    })?;
    // Closure-call shape per array_transform::handle_map_v2 precedent
    // (call_value_immediate_nb). The closure share moves into the call;
    // the returned KindedSlot owns the result.
    let result = vm.call_value_immediate_nb(&initializer, &[], ctx)?;
    // Cache the result. KindedSlot::Clone bumps share for return.
    let cached_copy = result.clone();
    l.store_result(result);
    Ok(cached_copy)
}

/// `lazy.is_initialized()` — bool whether `get()` has been called and
/// the value cached.
pub fn v2_lazy_is_initialized(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 1 {
        return Err(type_error(
            "Lazy.is_initialized() takes no arguments",
        ));
    }
    let l: Arc<LazyData> = as_lazy(&args[0])?;
    Ok(KindedSlot::from_bool(l.is_initialized()))
}
