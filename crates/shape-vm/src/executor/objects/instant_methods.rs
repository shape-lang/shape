//! Method handlers for `Instant` values (`std::time::Instant`).
//!
//! Receiver kind: `NativeKind::Ptr(HeapKind::Instant)` — slot bits are the
//! raw `Arc::into_raw::<std::time::Instant>` pointer (ADR-006 §2.3 / §2.4).
//!
//! ## Phase 1.B-vm Wave-γ-followup `MR-datetime-instant` body migration
//!
//! This file migrates from the Wave-β surface stubs (close commit
//! `19bdeaa` — kinded ABI not yet landed at the time) to real bodies on
//! the §2.7.10 / Q11 kinded `MethodFnV2` ABI.
//!
//! Per the dispatcher contract for `INSTANT_METHODS` (PHF map in
//! `method_registry.rs`), these handlers are only invoked when
//! `args[0].kind == NativeKind::Ptr(HeapKind::Instant)`. The dispatcher
//! owns one strong-count share for each `KindedSlot` in `args` for the
//! call duration and retires it after the handler returns (carrier
//! `Drop`). We therefore **borrow** the inner `&std::time::Instant`
//! through a direct `*const std::time::Instant` cast (mirrors the live
//! `printing.rs:204` pattern and the `vm_impl/stack.rs:99` /
//! `kinded_slot.rs:325` retain/drop arms keyed on `HeapKind::Instant`).
//! No `Arc` reconstitution is needed and no refcount is touched.
//!
//! Result construction follows playbook §3:
//!
//! - `f64` results — `KindedSlot::from_number(secs)`, kind `Float64`.
//! - `i64` results — `KindedSlot::from_int(ns)`, kind `Int64`.
//! - `String` results — `KindedSlot::from_string_arc(Arc::new(s))`, kind
//!   `String`. The fresh `Arc<String>` strong-count share transfers to
//!   the returned carrier; `mem::forget` in the dispatch shell hands the
//!   share to the stack (see `objects/mod.rs:291`).
//!
//! ## Forbidden patterns refused on sight
//!
//! - `slot.as_heap_value()` for the receiver — slot bits are
//!   `Arc::into_raw::<std::time::Instant>`, not `Box<HeapValue>`. The
//!   `as_heap_value` accessor is a legacy `Box<HeapValue>` artifact; using
//!   it on a typed-Arc slot would be a type-confused read.
//! - `Arc::from_raw(...)` on the receiver bits without a paired
//!   `Arc::into_raw` — would consume the dispatcher's share and double-
//!   free at carrier drop.
//! - Per-heap-variant accessors on `KindedSlot` (e.g. `as_instant()`) —
//!   ADR-006 §2.7.6 / Q8 forbids them on the carrier surface; receiver
//!   kind is statically known from the PHF map's keying so the body
//!   reads bits directly.
//!
//! See `docs/cluster-audits/phase-1b-vm-wave-6-5-playbook.md` §3 + §10
//! row `MR-datetime-instant`, ADR-006 §2.7.6 (Q8) / §2.7.10 (Q11).

use crate::executor::VirtualMachine;
use shape_runtime::context::ExecutionContext;
use shape_value::heap_value::HeapKind;
use shape_value::{KindedSlot, NativeKind, VMError};
use std::sync::Arc;

/// Borrow the receiver `&std::time::Instant` from `args[0]`.
///
/// Per the `INSTANT_METHODS` PHF dispatcher contract, the receiver kind
/// is statically `NativeKind::Ptr(HeapKind::Instant)`. The defensive kind
/// check here surfaces a `RuntimeError` rather than panicking on a
/// mis-keyed dispatch — a cheap belt-and-braces guard at one site per
/// handler.
///
/// SAFETY: when `args[0].kind == NativeKind::Ptr(HeapKind::Instant)`, the
/// slot bits are `Arc::into_raw::<std::time::Instant>` (ADR-006 §2.3 /
/// §2.4) and the dispatcher's `KindedSlot` owns one strong-count share
/// for the call duration. The returned `&Instant` borrows for the
/// lifetime of `args` (bounded by the dispatcher's carrier ownership).
#[inline]
fn recv_instant<'a>(args: &'a [KindedSlot]) -> Result<&'a std::time::Instant, VMError> {
    if args.is_empty() {
        return Err(VMError::RuntimeError(
            "Instant method invoked with no receiver".to_string(),
        ));
    }
    match args[0].kind {
        NativeKind::Ptr(HeapKind::Instant) => {
            let bits = args[0].slot.raw();
            // SAFETY: see function-level note above. The pointer is
            // guaranteed non-null by every push site (see
            // `kind_check::HeapKind::Instant` Arc-share invariant).
            Ok(unsafe { &*(bits as *const std::time::Instant) })
        }
        other => Err(VMError::RuntimeError(format!(
            "Instant method expected Instant receiver, got {:?}",
            other
        ))),
    }
}

/// `.elapsed() -> number` (seconds as f64).
pub fn v2_elapsed(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let instant = recv_instant(args)?;
    let secs = instant.elapsed().as_secs_f64();
    Ok(KindedSlot::from_number(secs))
}

/// `.elapsed_ms() -> number` (milliseconds as f64).
pub fn v2_elapsed_ms(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let instant = recv_instant(args)?;
    let ms = instant.elapsed().as_secs_f64() * 1000.0;
    Ok(KindedSlot::from_number(ms))
}

/// `.elapsed_us() -> number` (microseconds as f64).
pub fn v2_elapsed_us(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let instant = recv_instant(args)?;
    let us = instant.elapsed().as_secs_f64() * 1_000_000.0;
    Ok(KindedSlot::from_number(us))
}

/// `.elapsed_ns() -> int` (nanoseconds).
pub fn v2_elapsed_ns(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let instant = recv_instant(args)?;
    let ns = instant.elapsed().as_nanos() as i64;
    Ok(KindedSlot::from_int(ns))
}

/// `.duration_since(other: Instant) -> number` (milliseconds as f64).
pub fn v2_duration_since(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let this = recv_instant(args)?;
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "Instant.duration_since requires one Instant argument".to_string(),
        ));
    }
    let other = match args[1].kind {
        NativeKind::Ptr(HeapKind::Instant) => {
            let bits = args[1].slot.raw();
            // SAFETY: same Arc-share invariant as `recv_instant`. Borrow
            // for the lifetime of this call; the dispatcher's
            // `KindedSlot` carrier holds the share until handler return.
            unsafe { &*(bits as *const std::time::Instant) }
        }
        other => {
            return Err(VMError::RuntimeError(format!(
                "Instant.duration_since expected Instant argument, got {:?}",
                other
            )));
        }
    };
    let ms = this.duration_since(*other).as_secs_f64() * 1000.0;
    Ok(KindedSlot::from_number(ms))
}

/// `.to_string() -> string`.
pub fn v2_to_string(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let instant = recv_instant(args)?;
    let elapsed = instant.elapsed();
    let s = format!("Instant(elapsed: {:.6}s)", elapsed.as_secs_f64());
    Ok(KindedSlot::from_string_arc(Arc::new(s)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::{VMConfig, VirtualMachine};
    use shape_value::ValueSlot;

    fn create_test_vm() -> VirtualMachine {
        VirtualMachine::new(VMConfig::default())
    }

    /// Build a single-slot `KindedSlot` carrying an `Arc<std::time::Instant>`.
    /// One strong-count share owned by the returned `KindedSlot`; on Drop
    /// the share is retired via `Arc::decrement_strong_count::<Instant>`
    /// per `kinded_slot.rs:325`.
    fn make_instant_arg(instant: std::time::Instant) -> KindedSlot {
        let arc = Arc::new(instant);
        let bits = Arc::into_raw(arc) as u64;
        KindedSlot::new(
            ValueSlot::from_raw(bits),
            NativeKind::Ptr(HeapKind::Instant),
        )
    }

    #[test]
    fn test_elapsed_returns_number() {
        let mut vm = create_test_vm();
        let args = [make_instant_arg(std::time::Instant::now())];
        let result = v2_elapsed(&mut vm, &args, None).unwrap();
        assert_eq!(result.kind, NativeKind::Float64);
        let secs = result.as_f64().unwrap();
        assert!(secs >= 0.0);
        assert!(secs < 1.0);
    }

    #[test]
    fn test_elapsed_ms_returns_milliseconds() {
        let mut vm = create_test_vm();
        let args = [make_instant_arg(std::time::Instant::now())];
        let result = v2_elapsed_ms(&mut vm, &args, None).unwrap();
        assert_eq!(result.kind, NativeKind::Float64);
        let ms = result.as_f64().unwrap();
        assert!(ms >= 0.0);
        assert!(ms < 1000.0);
    }

    #[test]
    fn test_elapsed_us_returns_microseconds() {
        let mut vm = create_test_vm();
        let args = [make_instant_arg(std::time::Instant::now())];
        let result = v2_elapsed_us(&mut vm, &args, None).unwrap();
        assert_eq!(result.kind, NativeKind::Float64);
        let us = result.as_f64().unwrap();
        assert!(us >= 0.0);
        assert!(us < 1_000_000.0);
    }

    #[test]
    fn test_elapsed_ns_returns_int() {
        let mut vm = create_test_vm();
        let args = [make_instant_arg(std::time::Instant::now())];
        let result = v2_elapsed_ns(&mut vm, &args, None).unwrap();
        assert_eq!(result.kind, NativeKind::Int64);
        let ns = result.as_i64().unwrap();
        assert!(ns >= 0);
    }

    #[test]
    fn test_to_string_format() {
        let mut vm = create_test_vm();
        let args = [make_instant_arg(std::time::Instant::now())];
        let result = v2_to_string(&mut vm, &args, None).unwrap();
        assert_eq!(result.kind, NativeKind::String);
        let s = result.as_str().unwrap();
        assert!(s.starts_with("Instant(elapsed:"));
        assert!(s.ends_with("s)"));
    }

    #[test]
    fn test_duration_since() {
        let mut vm = create_test_vm();
        let earlier = std::time::Instant::now();
        std::hint::black_box(0u64.wrapping_add(1));
        let later = std::time::Instant::now();
        let args = [make_instant_arg(later), make_instant_arg(earlier)];
        let result = v2_duration_since(&mut vm, &args, None).unwrap();
        assert_eq!(result.kind, NativeKind::Float64);
        let ms = result.as_f64().unwrap();
        assert!(ms >= 0.0);
    }

    /// Wrong receiver kind surfaces a runtime error rather than panicking.
    #[test]
    fn test_recv_instant_wrong_kind_errors() {
        let mut vm = create_test_vm();
        let args = [KindedSlot::from_int(42)];
        let err = v2_elapsed(&mut vm, &args, None).unwrap_err();
        match err {
            VMError::RuntimeError(msg) => assert!(msg.contains("expected Instant receiver")),
            other => panic!("expected RuntimeError, got {:?}", other),
        }
    }
}
