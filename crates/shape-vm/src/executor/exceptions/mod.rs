//! Exception handling operations for the VM executor.
//!
//! W8-EX (ADR-006 §2.7.6 / Q8 carrier-API-bound, §2.7.7 stack
//! parallel-kind, §2.7.10 / §2.7.11 dispatch precedent): the exception
//! payload ABI on every internal Rust dispatch path through this
//! module is the `KindedSlot` carrier per §2.7.6 / Q8. The opcode
//! handlers source the payload from `pop_kinded()` (§2.7.7 stack
//! parallel-kind track) and wrap into a `KindedSlot`; the unwind path
//! (`handle_exception`) re-pushes via `push_kinded_slot` so the
//! parallel-kind track stays in lockstep with the data slots
//! (§2.7.7 invariant).
//!
//! Per playbook §10 E-exceptions row, the post-rebuild exception
//! payload kind at the catch-site is `NativeKind::Ptr(HeapKind::TypedObject)`
//! (the AnyError / TypedObject-shaped wrapper with attached trace
//! info). Today every kind-source carrying an exception payload is
//! preserved verbatim from the §2.7.7 stack parallel-kind track —
//! producing opcodes / dispatch.rs runtime-error converter own the
//! kind, this module never fabricates one.
//!
//! ## Phase-2c surface
//!
//! The pre-existing exception machinery (AnyError construction,
//! TraceFrame / TraceInfoFull / TraceInfoSingle TypedObject builders,
//! error-chain formatting, `format_uncaught_exception`, the cause-chain
//! walker, `is_any_error` discrimination, the `Result<_,_>` /
//! `Option<_>` extract-inner fast paths) was implemented on top of:
//!
//! - the deleted `ValueWord` / `ValueWordExt` carrier (CLAUDE.md
//!   "Forbidden code"),
//! - `executor::objects::raw_helpers::extract_*` heap-side accessors
//!   (forbidden #7 in playbook §4 — owned by D-raw-helpers cluster),
//! - the deleted `vw_clone(bits)` / `vw_drop(bits)` retain/release
//!   primitives (forbidden #8 — replaced by `clone_with_kind` /
//!   `drop_with_kind`).
//!
//! Per playbook §7 REVISED #3, those forbidden patterns are migrated
//! off rather than preserved. The full exception object machinery is
//! surfaced as Phase-2c per ADR-006 §2.7.4: it must be re-emitted on
//! top of the kinded `Arc<TypedObjectStorage>` model after
//! D-raw-helpers cleans up the heap-decode primitives.
//!
//! Cross-cluster cascade (per playbook §8 surface-and-stop):
//!
//! - `dispatch.rs` calls `handle_exception` at runtime-error
//!   conversion sites with a `KindedSlot::from_string_arc(error_arc)`
//!   payload (kind = `NativeKind::String`). The W8-EX rebuild flips
//!   the entry-point ABI from the pre-§2.7.6 `(error_bits, error_kind)`
//!   parallel-pair to the `KindedSlot` carrier per §2.7.6 / Q8.
//! - `control_flow/mod.rs` calls `trace_info_full` +
//!   `build_any_error` for the `?` operator's inner-value path —
//!   bodies remain Phase-2c; signatures are kinded.
//! - `builtins/type_ops.rs` calls `trace_info_single` +
//!   `build_any_error` — same Phase-2c body status.
//!
//! Until the Phase-2c bodies land, the helpers in this module take
//! `KindedSlot` carriers for every exception-payload argument and
//! return `Result<KindedSlot, VMError>` for builders, matching the
//! §2.7.6 / Q8 carrier-API-bound vocabulary the project speaks at
//! every other dispatch boundary (§2.7.10 method dispatch,
//! §2.7.11 value-call dispatch).

use crate::{
    bytecode::{Instruction, OpCode, Operand},
    executor::{ExceptionHandler, VirtualMachine},
    executor::vm_impl::stack::drop_with_kind,
};
use shape_value::{KindedSlot, NativeKind, VMError, ValueSlot};

/// Phase-2c surface message used by every helper body that depends on
/// the deleted `ValueWord` / `raw_helpers` machinery. Centralized so
/// the supervisor can grep one literal at re-emission time.
const PHASE_2C_EXCEPTION_OBJECT_SURFACE: &str =
    "phase-2c — exception object machinery (AnyError TypedObject build, \
     trace-frame build, cause-chain format) pending re-emission on the \
     kinded Arc<TypedObjectStorage> model. Depends on D-raw-helpers \
     cleanup of heap-decode primitives and cross-cluster cascade in \
     dispatch.rs / control_flow/mod.rs / builtins/type_ops.rs migrating \
     ValueWord-typed arguments to (u64, NativeKind). See ADR-006 \
     §2.7.4 / playbook §10 E-exceptions row.";

impl VirtualMachine {
    // ===== Helper Methods =====

    /// Handle an exception by unwinding to the nearest handler.
    ///
    /// W8-EX: the payload arrives as a `KindedSlot` carrier per
    /// §2.7.6 / Q8 (the project's canonical boundary-carrier shape;
    /// same as §2.7.10 method dispatch, §2.7.11 value-call dispatch).
    /// The carrier owns one strong-count share for heap-bearing kinds;
    /// on catch-recovery the share transfers to the new top-of-stack
    /// slot via `push_kinded_slot`. Per playbook §10 E-exceptions row,
    /// the payload kind at the catch-site is
    /// `NativeKind::Ptr(HeapKind::TypedObject)` once Phase-2c
    /// AnyError-wrapping lands; the kind threaded in today is whatever
    /// the producing site emitted (`NativeKind::String` for runtime-
    /// error converters in dispatch.rs, the user-thrown payload's kind
    /// for `op_throw`).
    pub(in crate::executor) fn handle_exception(
        &mut self,
        payload: KindedSlot,
    ) -> Result<(), VMError> {
        if let Some(handler) = self.exception_handlers.pop() {
            self.clear_last_uncaught_exception();
            // Unwind stack to handler's saved state (sp-based).
            // Each unwound slot owns a heap share that must be released
            // via `drop_with_kind` per ADR-006 §2.7.7 WB2.4 — read the
            // kind from the parallel kinds track, drop the share, and
            // poison the slot to NONE_BITS / Bool kind so it doesn't
            // leak into a later read.
            for i in handler.stack_size..self.sp {
                let (bits, kind) = self.stack_read_kinded_raw(i);
                drop_with_kind(bits, kind);
                self.stack[i] = Self::NONE_BITS;
                self.kinds[i] = NativeKind::Bool;
            }
            self.sp = handler.stack_size;
            self.call_stack.truncate(handler.call_depth);

            // Push error value for catch block. `push_kinded_slot`
            // transfers the carrier's share onto the stack and
            // `mem::forget`s the carrier so its `Drop` doesn't double-
            // retire — same WB2.4 retain-on-read discipline §2.7.10
            // established at the method-dispatch result-push site.
            self.push_kinded_slot(payload)?;

            // Jump to catch handler.
            self.ip = handler.catch_ip;
            Ok(())
        } else {
            // No handler — propagate as a runtime error. The Phase-2c
            // surface covers AnyError-chain formatting; release the
            // payload share via `KindedSlot::Drop` (kind-dispatched
            // refcount retire per §2.7.6 / Q8) and surface a generic
            // runtime error so the kind track stays balanced.
            let kind = payload.kind();
            drop(payload);
            Err(VMError::RuntimeError(format!(
                "Uncaught exception (kind {:?}): {}",
                kind, PHASE_2C_EXCEPTION_OBJECT_SURFACE
            )))
        }
    }

    // ===== Opcode Implementations =====

    #[inline(always)]
    pub(in crate::executor) fn exec_exceptions(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use OpCode::*;
        match instruction.opcode {
            TypeCheck => self.op_type_check(instruction)?,
            SetupTry => self.op_setup_try(instruction)?,
            PopHandler => self.op_pop_handler()?,
            Throw => self.op_throw()?,
            TryUnwrap => self.op_try_unwrap()?,
            UnwrapOption => self.op_unwrap_option()?,
            ErrorContext => self.op_error_context()?,
            IsOk => self.op_is_ok()?,
            IsErr => self.op_is_err()?,
            UnwrapOk => self.op_unwrap_ok()?,
            UnwrapErr => self.op_unwrap_err()?,
            _ => unreachable!(
                "exec_exceptions called with non-exception opcode: {:?}",
                instruction.opcode
            ),
        }
        Ok(())
    }

    /// `TypeCheck`: pop a value, compare against a type-annotation
    /// constant, push a `Bool` result.
    ///
    /// SURFACE: the runtime-tier `check_instanceof` body relied on
    /// `ValueWord::heap_kind()`, `as_str`, `as_i64`, `as_f64`,
    /// `as_any_array`, `as_decimal`, `as_char`, `is_function` etc. —
    /// all on the deleted `ValueWord` carrier. The kinded equivalent
    /// inspects `value.kind()` directly and dispatches on
    /// `value.slot().as_heap_value()` per §2.7.6 / Q8, but it depends
    /// on the `KindedSlot` consumer-side helpers landing first
    /// (D-type-ops territory has the closest pattern). Until then we
    /// drop the popped carrier and surface.
    pub(in crate::executor) fn op_type_check(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let (value_bits, value_kind) = self.pop_kinded()?;
        let value = KindedSlot::new(ValueSlot::from_raw(value_bits), value_kind);
        // Validate the operand carries a type-annotation constant so
        // callers see the same `InvalidOperand` shape they always
        // have. We don't actually consult the annotation — the runtime
        // matcher is part of the Phase-2c surface.
        let _annotation = match instruction.operand {
            Some(Operand::Const(idx)) => match self.program.constants.get(idx as usize) {
                Some(crate::bytecode::Constant::TypeAnnotation(annotation)) => annotation.clone(),
                _ => {
                    drop(value);
                    return Err(VMError::RuntimeError(
                        "TypeCheck expects type annotation constant".to_string(),
                    ));
                }
            },
            _ => {
                drop(value);
                return Err(VMError::InvalidOperand);
            }
        };

        drop(value);
        Err(VMError::NotImplemented(format!(
            "op_type_check: {}",
            PHASE_2C_EXCEPTION_OBJECT_SURFACE
        )))
    }

    pub(in crate::executor) fn op_setup_try(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        if let Some(Operand::Offset(offset)) = instruction.operand {
            let catch_ip = (self.ip as i32 + offset) as usize;
            self.exception_handlers.push(ExceptionHandler {
                catch_ip,
                stack_size: self.sp,
                call_depth: self.call_stack.len(),
            });
            Ok(())
        } else {
            Err(VMError::InvalidOperand)
        }
    }

    pub(in crate::executor) fn op_pop_handler(&mut self) -> Result<(), VMError> {
        self.exception_handlers.pop();
        Ok(())
    }

    /// `Throw`: pop the payload, hand it off to `handle_exception`
    /// with kind threaded through. Per playbook §10 the payload kind
    /// at the throw boundary is `NativeKind::Ptr(HeapKind::TypedObject)`
    /// (the AnyError TypedObject) post-Phase-2c; today we honor
    /// whatever the producing opcode pushed (kind sourced from the
    /// §2.7.7 stack parallel-kind track via `pop_kinded`).
    pub(in crate::executor) fn op_throw(&mut self) -> Result<(), VMError> {
        let (error_bits, error_kind) = self.pop_kinded()?;
        let payload = KindedSlot::new(ValueSlot::from_raw(error_bits), error_kind);
        self.handle_exception(payload)
    }

    /// Trace-info / AnyError builders are Phase-2c per ADR-006 §2.7.4.
    /// Signatures speak the §2.7.6 / Q8 carrier (`KindedSlot` /
    /// `Result<KindedSlot, VMError>`) so cross-cluster callers
    /// (`control_flow/mod.rs`, `builtins/type_ops.rs`) align with the
    /// project's canonical boundary vocabulary; bodies surface to
    /// Phase-2c until the AnyError-wrap re-emission lands.
    pub(in crate::executor) fn trace_info_full(&mut self) -> Result<KindedSlot, VMError> {
        Err(VMError::NotImplemented(format!(
            "trace_info_full: {}",
            PHASE_2C_EXCEPTION_OBJECT_SURFACE
        )))
    }

    pub(in crate::executor) fn trace_info_single(&mut self) -> Result<KindedSlot, VMError> {
        Err(VMError::NotImplemented(format!(
            "trace_info_single: {}",
            PHASE_2C_EXCEPTION_OBJECT_SURFACE
        )))
    }

    /// AnyError TypedObject builder. Takes ownership of every input
    /// `KindedSlot`'s share; on Phase-2c re-emission these shares
    /// transfer into the AnyError TypedObject's field slots. Returns
    /// a `KindedSlot` owning a fresh `Arc<TypedObjectStorage>` share
    /// (kind = `NativeKind::Ptr(HeapKind::TypedObject)`).
    ///
    /// SURFACE: the pre-existing implementation used six
    /// `ValueSlot`-laden field writes plus the deleted `ValueWord`
    /// carrier. Re-emission is deferred to Phase-2c per the module-
    /// level note.
    pub(in crate::executor) fn build_any_error(
        &mut self,
        payload: KindedSlot,
        cause: Option<KindedSlot>,
        trace: KindedSlot,
        _code: Option<&str>,
    ) -> Result<KindedSlot, VMError> {
        // Release each carrier's share via `KindedSlot::Drop` (kind-
        // dispatched refcount retire per §2.7.6 / Q8). Phase-2c body
        // will instead transfer them into the AnyError field slots.
        drop(payload);
        drop(cause);
        drop(trace);
        Err(VMError::NotImplemented(format!(
            "build_any_error: {}",
            PHASE_2C_EXCEPTION_OBJECT_SURFACE
        )))
    }

    /// Normalize an arbitrary thrown payload to an AnyError-shaped
    /// TypedObject (so the catch block always sees a uniform shape).
    /// SURFACE: depends on `build_any_error` + `trace_info_full` —
    /// both Phase-2c. The caller-owned carrier is passed through
    /// untouched today (no AnyError wrap yet); ownership transfers
    /// back to the caller to keep the kind track balanced.
    pub(in crate::executor) fn normalize_err_payload(
        &mut self,
        payload: KindedSlot,
    ) -> Result<KindedSlot, VMError> {
        // Pass-through: the AnyError wrap is Phase-2c; until then the
        // payload is already what the catch block sees.
        Ok(payload)
    }

    /// `ErrorContext` (`!!` operator): pop context + value, wrap value
    /// into AnyError with context. Phase-2c stub — drop both carriers
    /// (kind-dispatched refcount retire via `KindedSlot::Drop`) and
    /// surface so the stack stays balanced.
    pub(in crate::executor) fn op_error_context(&mut self) -> Result<(), VMError> {
        let (context_bits, context_kind) = self.pop_kinded()?;
        let (value_bits, value_kind) = self.pop_kinded()?;
        let context = KindedSlot::new(ValueSlot::from_raw(context_bits), context_kind);
        let value = KindedSlot::new(ValueSlot::from_raw(value_bits), value_kind);
        drop(context);
        drop(value);
        Err(VMError::NotImplemented(format!(
            "op_error_context: {}",
            PHASE_2C_EXCEPTION_OBJECT_SURFACE
        )))
    }

    /// `TryUnwrap` (`?` operator) for unified Result/Option propagation.
    ///
    /// Behavior at re-emission:
    /// - `Ok(value)` => unwraps to `value`
    /// - `Err(error)` => returns early with `Err(error)`
    /// - `None` => returns early with AnyError-wrapped OPTION_NONE
    /// - `Some(value)` => unwraps to `value`
    /// - bare non-`None` values => pass-through
    ///
    /// SURFACE: the variant discriminator (`extract_ok_inner` /
    /// `extract_err_inner` / `extract_some_inner` / `is_none`) lived
    /// in `raw_helpers` (forbidden #7) and on the `ValueWord`
    /// accessor surface (CLAUDE.md). The kinded equivalent dispatches
    /// on `kind == NativeKind::Ptr(HeapKind::TypedObject)` plus
    /// pattern-match on `slot.as_heap_value()` per Q8 — but
    /// `Result<_,_>` and `Option<_>` are heap-side discriminators
    /// owned by the variant-codegen path, which is part of the same
    /// Phase-2c work.
    pub(in crate::executor) fn op_try_unwrap(&mut self) -> Result<(), VMError> {
        let (bits, kind) = self.pop_kinded()?;
        let value = KindedSlot::new(ValueSlot::from_raw(bits), kind);
        drop(value);
        Err(VMError::NotImplemented(format!(
            "op_try_unwrap: {}",
            PHASE_2C_EXCEPTION_OBJECT_SURFACE
        )))
    }

    /// `UnwrapOption` (`opt!`-style): pop a `T?` and unwrap to `T`,
    /// throwing if `None`.
    ///
    /// SURFACE: same discriminator dependency as `op_try_unwrap` —
    /// the inner-extract helpers and the `is_none` predicate are part
    /// of the variant-codegen Phase-2c surface.
    pub(in crate::executor) fn op_unwrap_option(&mut self) -> Result<(), VMError> {
        let (bits, kind) = self.pop_kinded()?;
        let value = KindedSlot::new(ValueSlot::from_raw(bits), kind);
        drop(value);
        Err(VMError::NotImplemented(format!(
            "op_unwrap_option: {}",
            PHASE_2C_EXCEPTION_OBJECT_SURFACE
        )))
    }

    #[inline(always)]
    pub(in crate::executor) fn op_is_ok(&mut self) -> Result<(), VMError> {
        let (bits, kind) = self.pop_kinded()?;
        let value = KindedSlot::new(ValueSlot::from_raw(bits), kind);
        drop(value);
        Err(VMError::NotImplemented(format!(
            "op_is_ok: {}",
            PHASE_2C_EXCEPTION_OBJECT_SURFACE
        )))
    }

    #[inline(always)]
    pub(in crate::executor) fn op_is_err(&mut self) -> Result<(), VMError> {
        let (bits, kind) = self.pop_kinded()?;
        let value = KindedSlot::new(ValueSlot::from_raw(bits), kind);
        drop(value);
        Err(VMError::NotImplemented(format!(
            "op_is_err: {}",
            PHASE_2C_EXCEPTION_OBJECT_SURFACE
        )))
    }

    /// `UnwrapOk`: pop an `Ok(_)`, push the inner value.
    ///
    /// At Phase-2c re-emission the retain-on-extract pattern (per
    /// WB2.4 / ADR-006 §2.7.7) constructs an inner-value `KindedSlot`
    /// that retains the underlying `Arc<T>` share, drops the outer
    /// wrapper carrier (kind-dispatched refcount retire via
    /// `KindedSlot::Drop`), and re-pushes via `push_kinded_slot`.
    /// The unit-test regression docs in this module's tail (preserved
    /// as `#[ignore]` for Phase-2c) name the exact aliasing class.
    ///
    /// SURFACE: extract-inner variant discriminators are Phase-2c.
    #[inline(always)]
    pub(in crate::executor) fn op_unwrap_ok(&mut self) -> Result<(), VMError> {
        let (bits, kind) = self.pop_kinded()?;
        let value = KindedSlot::new(ValueSlot::from_raw(bits), kind);
        drop(value);
        Err(VMError::NotImplemented(format!(
            "op_unwrap_ok: {}",
            PHASE_2C_EXCEPTION_OBJECT_SURFACE
        )))
    }

    /// `UnwrapErr`: pop an `Err(_)`, push the inner error value
    /// (unwrapping the AnyError wrapper if the inner is itself an
    /// AnyError TypedObject).
    ///
    /// SURFACE: same Phase-2c surface as `op_unwrap_ok`. The
    /// AnyError-unwrap path additionally requires `is_any_error`
    /// discrimination (depends on `raw_helpers::extract_typed_object`
    /// — forbidden) plus `ANYERROR_PAYLOAD` slot read.
    #[inline(always)]
    pub(in crate::executor) fn op_unwrap_err(&mut self) -> Result<(), VMError> {
        let (bits, kind) = self.pop_kinded()?;
        let value = KindedSlot::new(ValueSlot::from_raw(bits), kind);
        drop(value);
        Err(VMError::NotImplemented(format!(
            "op_unwrap_err: {}",
            PHASE_2C_EXCEPTION_OBJECT_SURFACE
        )))
    }
}

// =========================================================================
// Phase-2c regression tests (preserved as documentation; gated until the
// re-emission lands so they don't drag the test binary into the broken
// machinery).
// =========================================================================

#[cfg(test)]
#[cfg(feature = "phase-2c-exception-rebuild")]
mod unwrap_refcount_regression_tests {
    use crate::test_utils::eval;

    /// Regression: `op_unwrap_ok` used to expose the inner value without
    /// a retain and leak the outer `Ok(...)` wrapper's share. With the
    /// interner-backed `Arc<String>` for small literals the off-by-one
    /// refcount eventually freed a `HeapValue::String` that the leaked
    /// wrapper still pointed at, corrupting the allocator freelist
    /// (malloc_consolidate SIGABRT under release glibc).
    ///
    /// The minimal trigger is `match Ok(<small-string>) { Ok(data) => len(data) }`
    /// — the inner local is first written un-retained, then its
    /// destructor at frame unwind decrements below zero. The fix retains
    /// the inner on extract and releases the wrapper before push.
    ///
    /// Phase-2c re-emission must reproduce this discipline using
    /// `clone_with_kind` / `drop_with_kind` per ADR-006 §2.7.7.
    #[test]
    fn match_ok_small_string_then_len_no_heap_corruption() {
        let v = eval(
            r#"
            let encoded: Result<string, string> = Ok("hello")
            match encoded {
                Ok(data) => data.len(),
                Err(_) => 0,
            }
            "#,
        );
        assert_eq!(v.as_i64(), Some(5));
    }

    /// Mirror test for `op_unwrap_err`: the same refcount imbalance
    /// applied to the Err path.
    #[test]
    fn match_err_small_string_then_len_no_heap_corruption() {
        let v = eval(
            r#"
            let encoded: Result<int, string> = Err("oops!")
            match encoded {
                Ok(_) => 0,
                Err(msg) => msg.len(),
            }
            "#,
        );
        assert_eq!(v.as_i64(), Some(5));
    }
}
