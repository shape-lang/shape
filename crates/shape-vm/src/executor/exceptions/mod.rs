//! Exception handling operations for the VM executor.
//!
//! Wave 6.5 cluster E-exceptions (ADR-006 §2.7.6, §2.7.7, §2.7.8 / Q7-Q10):
//! the 26 transitional shim caller sites are migrated to the kinded API
//! (`push_kinded(bits, kind)` / `pop_kinded() -> (bits, kind)`). Per the
//! playbook §10 E-exceptions row, the exception payload is carried with
//! kind = `NativeKind::Ptr(HeapKind::TypedObject)` (the AnyError /
//! TypedObject-shaped payload that wraps the user-thrown value plus
//! attached trace info).
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
//!   `drop_with_kind`),
//! - `nb_to_slot` / `as_value_word` / `as_heap_nb` ValueWord-bridging
//!   helpers on `ValueSlot` (CLAUDE.md "Renames to refuse on sight").
//!
//! Per playbook §7 REVISED #3, those forbidden patterns are migrated
//! off rather than preserved. The full exception object machinery is
//! surfaced as Phase-2c per ADR-006 §2.7.4: it must be re-emitted on
//! top of the kinded `Arc<TypedObjectStorage>` model after
//! D-raw-helpers cleans up the heap-decode primitives and the
//! cross-cluster callers (`dispatch.rs`, `control_flow/mod.rs`,
//! `builtins/type_ops.rs`) have migrated their `ValueWord`-typed
//! arguments to `(u64, NativeKind)` / `KindedSlot` per the §2.7
//! / Q7 carrier-shape ruling.
//!
//! Cross-cluster cascade (per playbook §8 surface-and-stop):
//!
//! - `dispatch.rs` calls `handle_exception_nb` at runtime-error
//!   conversion sites and constructs a `ValueWord::from_string` for
//!   the error payload — both forbidden. E-execution / supervisor
//!   migrate.
//! - `control_flow/mod.rs` calls `trace_info_full_nb` +
//!   `build_any_error_nb` for the `?` operator's inner-value path —
//!   both forbidden after §2.7.7 deletes ValueWord.
//! - `builtins/type_ops.rs` calls `trace_info_single_nb` +
//!   `build_any_error_nb` and uses `format_value_default_nb` (the
//!   default-formatter, defined elsewhere — also broken upstream).
//!
//! Until those migrate, the helpers in this module preserve their
//! `(u64, NativeKind)` signatures (for opcode-handler call sites that
//! drive the state machine) but their bodies surface to Phase 2c.
//! Opcode handlers themselves do the kinded pop/push correctly so
//! the parallel kind track stays in lockstep (ADR-006 §2.7.7 invariant).

use crate::{
    bytecode::{Instruction, OpCode, Operand},
    executor::{ExceptionHandler, VirtualMachine},
    executor::vm_impl::stack::drop_with_kind,
};
use shape_value::heap_value::HeapKind;
use shape_value::{NativeKind, VMError};

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
    /// Wave 6.5 E-exceptions: the payload is carried as `(bits, kind)`
    /// with `kind = NativeKind::Ptr(HeapKind::TypedObject)` (the
    /// AnyError-shaped payload — see playbook §10 E-exceptions row and
    /// ADR-006 §2.7 / Q7 carrier-shape ruling).
    ///
    /// SURFACE: dispatch.rs currently calls this with a `ValueWord`
    /// argument and constructs the payload via the deleted
    /// `ValueWord::from_string` constructor; that's owned by
    /// E-execution / supervisor and must migrate together with this
    /// helper's signature.
    pub(in crate::executor) fn handle_exception_nb(
        &mut self,
        error_bits: u64,
        error_kind: NativeKind,
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

            // Push error value for catch block. The payload kind is
            // TypedObject-shaped per playbook §10 (exceptions wrap into
            // AnyError once they reach a handler), but the upstream
            // caller (dispatch.rs / op_throw) is responsible for
            // having normalized to that shape — here we honor whatever
            // kind it threaded in.
            self.push_kinded(error_bits, error_kind)?;

            // Jump to catch handler.
            self.ip = handler.catch_ip;
            Ok(())
        } else {
            // No handler — propagate as a runtime error. The Phase-2c
            // surface covers AnyError-chain formatting; release the
            // payload share and surface a generic runtime error so
            // the kind track stays balanced.
            drop_with_kind(error_bits, error_kind);
            Err(VMError::RuntimeError(format!(
                "Uncaught exception (kind {:?}): {}",
                error_kind, PHASE_2C_EXCEPTION_OBJECT_SURFACE
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
    /// inspects `kind: NativeKind` directly and dispatches on
    /// `slot.as_heap_value()` per Q8, but it depends on the
    /// `KindedSlot` consumer-side helpers landing first (D-type-ops
    /// territory has the closest pattern). Until then we drop the
    /// popped share, push a Bool=false, and surface.
    pub(in crate::executor) fn op_type_check(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let (value_bits, value_kind) = self.pop_kinded()?;
        // Validate the operand carries a type-annotation constant so
        // callers see the same `InvalidOperand` shape they always
        // have. We don't actually consult the annotation — the runtime
        // matcher is part of the Phase-2c surface.
        let _annotation = match instruction.operand {
            Some(Operand::Const(idx)) => match self.program.constants.get(idx as usize) {
                Some(crate::bytecode::Constant::TypeAnnotation(annotation)) => annotation.clone(),
                _ => {
                    drop_with_kind(value_bits, value_kind);
                    return Err(VMError::RuntimeError(
                        "TypeCheck expects type annotation constant".to_string(),
                    ));
                }
            },
            _ => {
                drop_with_kind(value_bits, value_kind);
                return Err(VMError::InvalidOperand);
            }
        };

        drop_with_kind(value_bits, value_kind);
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

    /// `Throw`: pop the payload, hand it off to `handle_exception_nb`
    /// with kind threaded through. Per playbook §10 the payload kind
    /// at the throw boundary is `NativeKind::Ptr(HeapKind::TypedObject)`
    /// (the AnyError TypedObject), but we honor whatever the producing
    /// opcode pushed and let `handle_exception_nb` thread it through.
    pub(in crate::executor) fn op_throw(&mut self) -> Result<(), VMError> {
        let (error_bits, error_kind) = self.pop_kinded()?;
        self.handle_exception_nb(error_bits, error_kind)
    }

    /// Trace-info / AnyError builders are Phase-2c per ADR-006 §2.7.4.
    /// The signatures stay so cross-cluster callers
    /// (`control_flow/mod.rs`, `builtins/type_ops.rs`) keep their
    /// call shapes intact — those callers will surface their own
    /// cascade when they migrate.
    ///
    /// Returns a `(u64, NativeKind)` tuple that an upstream caller
    /// would push onto the stack; in the Phase-2c stub we surface
    /// instead.
    pub(in crate::executor) fn trace_info_full_nb(&mut self) -> Result<(u64, NativeKind), VMError> {
        Err(VMError::NotImplemented(format!(
            "trace_info_full: {}",
            PHASE_2C_EXCEPTION_OBJECT_SURFACE
        )))
    }

    pub(in crate::executor) fn trace_info_single_nb(
        &mut self,
    ) -> Result<(u64, NativeKind), VMError> {
        Err(VMError::NotImplemented(format!(
            "trace_info_single: {}",
            PHASE_2C_EXCEPTION_OBJECT_SURFACE
        )))
    }

    /// AnyError TypedObject builder. Returns a `(bits, kind)` pair
    /// owning a fresh `Arc<TypedObjectStorage>` share — caller must
    /// either push or drop with the kinded API.
    ///
    /// SURFACE: the pre-existing implementation used six
    /// `ValueSlot`-laden field writes plus a `nb_to_slot` bridge that
    /// re-tagged inline scalars through the deleted `ValueWord` shape.
    /// Re-emission is deferred to Phase 2c per the module-level note.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::executor) fn build_any_error_nb(
        &mut self,
        payload_bits: u64,
        payload_kind: NativeKind,
        cause: Option<(u64, NativeKind)>,
        trace_bits: u64,
        trace_kind: NativeKind,
        _code: Option<&str>,
    ) -> Result<(u64, NativeKind), VMError> {
        // Release the shares the caller threaded in — they own us a
        // share each; without re-emission we just drop them on the
        // floor and surface.
        drop_with_kind(payload_bits, payload_kind);
        if let Some((cb, ck)) = cause {
            drop_with_kind(cb, ck);
        }
        drop_with_kind(trace_bits, trace_kind);
        Err(VMError::NotImplemented(format!(
            "build_any_error: {}",
            PHASE_2C_EXCEPTION_OBJECT_SURFACE
        )))
    }

    /// Normalize an arbitrary thrown payload to an AnyError-shaped
    /// TypedObject (so the catch block always sees a uniform shape).
    /// SURFACE: depends on `build_any_error_nb` + `trace_info_full_nb`
    /// — both Phase-2c. Caller-owned `(bits, kind)` is passed through
    /// untouched to keep the kind track balanced.
    pub(in crate::executor) fn normalize_err_payload_nb(
        &mut self,
        payload_bits: u64,
        payload_kind: NativeKind,
    ) -> Result<(u64, NativeKind), VMError> {
        // Pass-through: the AnyError wrap is Phase-2c; until then the
        // payload is already what the catch block sees.
        Ok((payload_bits, payload_kind))
    }

    /// `ErrorContext` (`!!` operator): pop context + value, wrap value
    /// into AnyError with context. Phase-2c stub — drop both shares
    /// and surface so the stack stays balanced.
    pub(in crate::executor) fn op_error_context(&mut self) -> Result<(), VMError> {
        let (context_bits, context_kind) = self.pop_kinded()?;
        let (value_bits, value_kind) = self.pop_kinded()?;
        drop_with_kind(context_bits, context_kind);
        drop_with_kind(value_bits, value_kind);
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
        drop_with_kind(bits, kind);
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
        drop_with_kind(bits, kind);
        Err(VMError::NotImplemented(format!(
            "op_unwrap_option: {}",
            PHASE_2C_EXCEPTION_OBJECT_SURFACE
        )))
    }

    #[inline(always)]
    pub(in crate::executor) fn op_is_ok(&mut self) -> Result<(), VMError> {
        let (bits, kind) = self.pop_kinded()?;
        drop_with_kind(bits, kind);
        Err(VMError::NotImplemented(format!(
            "op_is_ok: {}",
            PHASE_2C_EXCEPTION_OBJECT_SURFACE
        )))
    }

    #[inline(always)]
    pub(in crate::executor) fn op_is_err(&mut self) -> Result<(), VMError> {
        let (bits, kind) = self.pop_kinded()?;
        drop_with_kind(bits, kind);
        Err(VMError::NotImplemented(format!(
            "op_is_err: {}",
            PHASE_2C_EXCEPTION_OBJECT_SURFACE
        )))
    }

    /// `UnwrapOk`: pop an `Ok(_)`, push the inner value.
    ///
    /// At re-emission the retain-on-extract pattern (per WB2.4 / ADR-
    /// 006 §2.7.7) is: `clone_with_kind(inner_bits, inner_kind)` to
    /// retain the inner Arc share, `drop_with_kind(outer_bits,
    /// outer_kind)` to release the wrapper, then `push_kinded(...)`.
    /// The unit-test regression docs in this module's tail (preserved
    /// as `#[ignore]` for Phase-2c) name the exact aliasing class.
    ///
    /// SURFACE: extract-inner variant discriminators are Phase-2c.
    #[inline(always)]
    pub(in crate::executor) fn op_unwrap_ok(&mut self) -> Result<(), VMError> {
        let (bits, kind) = self.pop_kinded()?;
        drop_with_kind(bits, kind);
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
        drop_with_kind(bits, kind);
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
