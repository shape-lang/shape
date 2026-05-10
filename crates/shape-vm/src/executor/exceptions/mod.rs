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
use shape_runtime::type_schema::builtin_schemas::{
    ANYERROR_CATEGORY, ANYERROR_CAUSE, ANYERROR_CODE, ANYERROR_MESSAGE,
    ANYERROR_PAYLOAD, ANYERROR_TRACE_INFO,
};
use shape_value::{
    HeapKind, KindedSlot, NativeKind, TypedObjectStorage, VMError, ValueSlot,
};
use std::sync::Arc;

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

    /// `Throw`: pop the payload, normalize to an AnyError TypedObject
    /// per playbook §10 E-exceptions row, and hand off to
    /// `handle_exception`. The pre-W13 path threaded the producing
    /// opcode's kind verbatim; W13-anyerror (close, 2026-05-10) wraps
    /// the popped carrier via `normalize_err_payload` so the catch
    /// block always sees the canonical
    /// `NativeKind::Ptr(HeapKind::TypedObject)` payload kind, and
    /// `e.message` reads back via the existing `op_get_prop`
    /// TypedObject path (already-AnyError throws pass through
    /// unchanged so cause chains are preserved).
    pub(in crate::executor) fn op_throw(&mut self) -> Result<(), VMError> {
        let (error_bits, error_kind) = self.pop_kinded()?;
        let raw_payload = KindedSlot::new(ValueSlot::from_raw(error_bits), error_kind);
        let payload = self.normalize_err_payload(raw_payload)?;
        self.handle_exception(payload)
    }

    /// Trace-info builders. Today the AnyError schema's `trace_info`
    /// field is a String slot (per `register_builtin_schemas` in
    /// `shape-runtime/src/type_schema/builtin_schemas.rs:114`); the
    /// pre-bulldozer design wrapped the trace into a
    /// `__TraceInfoFull` / `__TraceInfoSingle` TypedObject and then
    /// stringified it for the AnyError slot. Until the trace-frame
    /// recovery path is rebuilt (full backtrace walking, source-map
    /// resolution, frame-name lookup — Phase-2c surface tier per
    /// playbook §10), the trace_info slot is filled with an empty
    /// string. The AnyError construction path remains kind-correct
    /// (NativeKind::String slots; heap_mask bit clear for empty
    /// trace).
    ///
    /// W13-anyerror (close): the helpers return a None-kinded slot
    /// (zero bits, `NativeKind::String` so `build_any_error` can copy
    /// it into the trace_info field with heap_mask=0). Real
    /// frame-walk rebuild lives in a follow-up Phase-2c cluster
    /// (the Drop dispatch does not change once frame data lands —
    /// the slot stays String-typed via stringify).
    pub(in crate::executor) fn trace_info_full(&mut self) -> Result<KindedSlot, VMError> {
        Ok(empty_string_kinded_slot())
    }

    pub(in crate::executor) fn trace_info_single(&mut self) -> Result<KindedSlot, VMError> {
        Ok(empty_string_kinded_slot())
    }

    /// AnyError TypedObject builder.
    ///
    /// Builds an `Arc<TypedObjectStorage>` matching the AnyError
    /// schema (6 String fields: category, payload, cause, trace_info,
    /// message, code) per `register_builtin_schemas`
    /// (`shape-runtime/src/type_schema/builtin_schemas.rs:114`). Each
    /// input `KindedSlot`'s strong-count share transfers into the
    /// matching AnyError field slot when the input is String-kinded;
    /// non-String inputs are stringified via `kinded_to_string` and
    /// the source share is retired (the new `Arc<String>` owns the
    /// payload). The returned `KindedSlot` carries kind
    /// `NativeKind::Ptr(HeapKind::TypedObject)` and one strong-count
    /// share on the AnyError storage.
    ///
    /// Field semantics (matches the pre-bulldozer construction):
    ///
    /// - **payload** — the original error carrier stringified; same
    ///   text as `message` for runtime-error converters (the input
    ///   `payload` carrier is shared into both fields with separate
    ///   `Arc<String>` allocations).
    /// - **cause** — the inner error chain entry; empty when None.
    /// - **trace_info** — stringified trace info; empty when no
    ///   frame-walk is available (today's path).
    /// - **message** — same as `payload` text; user-visible field
    ///   read by `e.message`.
    /// - **category** / **code** — fixed strings ("RuntimeError" and
    ///   the optional `code` parameter); empty when missing.
    ///
    /// W13-anyerror (close): the AnyError TypedObject construction
    /// per ADR-006 §2.4 (`Arc<TypedObjectStorage>` typed-Arc payload)
    /// + §2.5 (per-slot `field_kinds` track for Drop dispatch) +
    /// playbook §3 TypedObject pattern (`Arc::into_raw` →
    /// `KindedSlot::from_typed_object`). Mirrors the
    /// `op_new_typed_object` precedent in `objects/object_creation.rs`
    /// (W9-property-access close `85bdb2a`).
    pub(in crate::executor) fn build_any_error(
        &mut self,
        payload: KindedSlot,
        cause: Option<KindedSlot>,
        trace: KindedSlot,
        code: Option<&str>,
    ) -> Result<KindedSlot, VMError> {
        // Stringify each carrier to `Arc<String>`; this consumes the
        // input carrier's share (the new Arc<String> owns the payload
        // text). For already-String inputs we transfer the share
        // directly; for non-String inputs we fall back to a stub
        // string ("<…>") and retire the input via `KindedSlot::Drop`.
        let message_arc = kinded_to_string_arc(payload);
        let payload_arc = Arc::clone(&message_arc);
        let cause_arc = cause.map(kinded_to_string_arc);
        let trace_arc = kinded_to_arc_or_none(trace);
        let category_arc = Arc::new("RuntimeError".to_string());
        let code_arc = code.map(|s| Arc::new(s.to_string()));

        let schema_id = self.builtin_schemas.any_error;

        // Build the 6 slots per AnyError field-index ordering. Each
        // String field's slot is `Arc::into_raw::<String>` bits when
        // the field has a value (heap_mask bit set), else zero bits
        // (heap_mask bit clear so Drop skips). field_kinds is uniform
        // `NativeKind::String` per the schema's all-String declaration.
        let mut slots: Vec<ValueSlot> = vec![ValueSlot::none(); 6];
        let mut heap_mask: u64 = 0;
        let mut set_field = |idx: usize, arc: Arc<String>| {
            let bits = Arc::into_raw(arc) as u64;
            slots[idx] = ValueSlot::from_raw(bits);
            heap_mask |= 1u64 << idx;
        };
        set_field(ANYERROR_CATEGORY, category_arc);
        set_field(ANYERROR_PAYLOAD, payload_arc);
        if let Some(arc) = cause_arc {
            set_field(ANYERROR_CAUSE, arc);
        }
        if let Some(arc) = trace_arc {
            set_field(ANYERROR_TRACE_INFO, arc);
        }
        set_field(ANYERROR_MESSAGE, message_arc);
        if let Some(arc) = code_arc {
            set_field(ANYERROR_CODE, arc);
        }

        // field_kinds is a uniform `NativeKind::String` table per
        // the AnyError schema's all-String field declaration. The
        // `Arc<[NativeKind]>` is allocated fresh here; per-schema
        // sharing (one allocation per schema) is an optimization
        // tracked separately — the Drop dispatch only cares that
        // each entry matches the slot's actual payload type.
        let field_kinds: Arc<[NativeKind]> = Arc::from(
            vec![NativeKind::String; 6].into_boxed_slice(),
        );

        let storage = Arc::new(TypedObjectStorage::new(
            schema_id as u64,
            slots.into_boxed_slice(),
            heap_mask,
            field_kinds,
        ));
        Ok(KindedSlot::from_typed_object(storage))
    }

    /// Normalize an arbitrary thrown payload to an AnyError-shaped
    /// TypedObject (so the catch block always sees a uniform shape).
    ///
    /// W13-anyerror (close): wraps non-AnyError payloads via
    /// `build_any_error` so `e.message` reads back correctly via the
    /// existing `op_get_prop` TypedObject path. Already-AnyError
    /// payloads (kind `NativeKind::Ptr(HeapKind::TypedObject)` + the
    /// AnyError schema_id) pass through unchanged so the catch chain
    /// preserves cause threading.
    pub(in crate::executor) fn normalize_err_payload(
        &mut self,
        payload: KindedSlot,
    ) -> Result<KindedSlot, VMError> {
        // Already-AnyError payloads (the typical case once a runtime
        // error has been wrapped once) flow through verbatim. The
        // schema-id check guards against a foreign TypedObject sneaking
        // in via a user `throw` of an unrelated typed value.
        if let NativeKind::Ptr(HeapKind::TypedObject) = payload.kind() {
            let bits = payload.slot().raw();
            if bits != 0 {
                // SAFETY: kind says Ptr(TypedObject); bits are
                // `Arc::into_raw::<TypedObjectStorage>`; carrier owns one
                // strong-count share. Borrow transiently to read schema_id.
                let arc: Arc<TypedObjectStorage> =
                    unsafe { Arc::from_raw(bits as *const _) };
                let is_any_error = arc.schema_id == self.builtin_schemas.any_error as u64;
                let _ = Arc::into_raw(arc);
                if is_any_error {
                    return Ok(payload);
                }
            }
        }

        // Non-AnyError payload: wrap in an AnyError TypedObject. The
        // payload carrier's share transfers into the AnyError's
        // payload/message fields via `build_any_error`'s stringify
        // path.
        let trace = self.trace_info_full()?;
        self.build_any_error(payload, None, trace, None)
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
// AnyError construction helpers (W13-anyerror)
//
// These free functions live next to the `build_any_error` /
// `trace_info_*` impl methods because they encode the AnyError-specific
// stringify discipline: each input `KindedSlot` either contributes its
// String share directly (the common case — runtime-error converters in
// dispatch.rs already feed `KindedSlot::from_string_arc`) or is
// stringified via a per-kind text projection and the source carrier is
// retired through `KindedSlot::Drop` (kind-dispatched refcount retire
// per ADR-006 §2.7.6 / Q8).
//
// The text projections are deliberately minimal: the full kinded
// formatter (`executor/printing.rs`) is its own Phase-2c surface
// (W13-print-formatter cluster) and routing through it from the
// exception path would couple two clusters that are landing in
// parallel. The exception payload kind at runtime is overwhelmingly
// `NativeKind::String` (every dispatch.rs converter site emits that
// kind today); the non-String fallback path produces a stable
// "<kind=…>" stub so the AnyError machinery surfaces the gap rather
// than silently dropping payload text.
// =========================================================================

/// Build a fresh `KindedSlot` carrying a zero-bits String slot. Used
/// by the trace-info builders for the empty-trace case (the AnyError
/// schema's `trace_info` field is String-typed; an empty trace is
/// represented as a zero-bits slot which the AnyError construction
/// path treats as "field unset" via heap_mask).
#[inline]
fn empty_string_kinded_slot() -> KindedSlot {
    KindedSlot::new(ValueSlot::none(), NativeKind::String)
}

/// Project a `KindedSlot` carrier to an owned `Arc<String>`, consuming
/// the carrier's share. `NativeKind::String` inputs transfer their
/// `Arc<String>` directly (zero-copy, no clone of the string body);
/// other kinds are formatted via a minimal per-kind stringifier and
/// the source carrier is retired through `KindedSlot::Drop`.
fn kinded_to_string_arc(slot: KindedSlot) -> Arc<String> {
    if matches!(slot.kind(), NativeKind::String) {
        let bits = slot.slot().raw();
        if bits != 0 {
            // Transfer the `Arc<String>` share directly; `mem::forget`
            // the carrier so its `Drop` doesn't decrement the share
            // we just moved into the returned `Arc<String>`.
            // SAFETY: kind says `NativeKind::String`; bits are
            // `Arc::into_raw::<String>`; carrier owns one strong-count
            // share. `Arc::from_raw` reclaims that share into the
            // returned `Arc<String>`.
            let arc: Arc<String> =
                unsafe { Arc::from_raw(bits as *const String) };
            std::mem::forget(slot);
            return arc;
        }
        // Zero-bits String slot — return an empty Arc<String>. The
        // carrier's `Drop` is a no-op on zero bits.
        return Arc::new(String::new());
    }
    // Non-String kind: format via minimal per-kind text projection.
    // The `Drop` impl on the carrier retires its share via
    // `drop_with_kind` (kind-dispatched refcount retire per §2.7.6 /
    // Q8) — same discipline as `read_as_string` in
    // `builtins/type_ops.rs`.
    let text = stringify_non_string_kinded(&slot);
    drop(slot);
    Arc::new(text)
}

/// Like `kinded_to_string_arc` but returns `None` when the carrier is
/// a zero-bits String slot (used by the `trace` parameter of
/// `build_any_error` so an empty trace info skips heap_mask
/// allocation).
fn kinded_to_arc_or_none(slot: KindedSlot) -> Option<Arc<String>> {
    if matches!(slot.kind(), NativeKind::String) && slot.slot().raw() == 0 {
        // Empty trace info — skip allocating an Arc<String>; the
        // AnyError trace_info slot stays zero-bits with heap_mask
        // bit clear.
        return None;
    }
    Some(kinded_to_string_arc(slot))
}

/// Format a non-String `KindedSlot` to a `String`. Minimal per-kind
/// stringifier; intentionally narrower than `executor::printing`'s
/// `ValueFormatter` (which is its own Phase-2c surface). The output
/// is informational — it appears in the AnyError TypedObject's
/// `payload` / `message` slots when a non-String value is thrown
/// (rare today; runtime-error converters in `dispatch.rs` always
/// produce `NativeKind::String`).
fn stringify_non_string_kinded(slot: &KindedSlot) -> String {
    match slot.kind() {
        NativeKind::Bool => slot.slot().as_bool().to_string(),
        NativeKind::Int8
        | NativeKind::Int16
        | NativeKind::Int32
        | NativeKind::Int64
        | NativeKind::IntSize
        | NativeKind::NullableInt8
        | NativeKind::NullableInt16
        | NativeKind::NullableInt32
        | NativeKind::NullableInt64
        | NativeKind::NullableIntSize => slot.slot().as_i64().to_string(),
        NativeKind::UInt8
        | NativeKind::UInt16
        | NativeKind::UInt32
        | NativeKind::UInt64
        | NativeKind::UIntSize
        | NativeKind::NullableUInt8
        | NativeKind::NullableUInt16
        | NativeKind::NullableUInt32
        | NativeKind::NullableUInt64
        | NativeKind::NullableUIntSize => slot.slot().as_u64().to_string(),
        NativeKind::Float64 | NativeKind::NullableFloat64 => {
            slot.slot().as_f64().to_string()
        }
        other => format!("<error payload kind={:?}>", other),
    }
}

// =========================================================================
// W13-anyerror unit tests — AnyError TypedObject construction
// =========================================================================

#[cfg(test)]
mod build_any_error_tests {
    use super::*;
    use crate::executor::VMConfig;
    use shape_value::heap_value::TypedObjectStorage;

    /// `build_any_error` produces a TypedObject whose schema_id matches
    /// the AnyError schema and whose `message` slot reads back as the
    /// input payload string.
    #[test]
    fn build_any_error_message_reads_back() {
        let mut vm = VirtualMachine::new(VMConfig::default());
        let payload = KindedSlot::from_string_arc(Arc::new("boom".to_string()));
        let trace = empty_string_kinded_slot();
        let result = vm.build_any_error(payload, None, trace, None).unwrap();

        // Result kind is Ptr(TypedObject); bits are Arc<TypedObjectStorage>.
        assert_eq!(result.kind(), NativeKind::Ptr(HeapKind::TypedObject));
        let bits = result.slot().raw();
        assert!(bits != 0, "AnyError TypedObject pointer should be non-null");

        // SAFETY: kind says Ptr(TypedObject); bits are Arc::into_raw of
        // an Arc<TypedObjectStorage>. We claim ownership of the share
        // for the duration of the test (the `result` carrier still owns
        // its share — we reconstruct without bumping).
        let storage: Arc<TypedObjectStorage> =
            unsafe { Arc::from_raw(bits as *const _) };

        // Schema ID matches AnyError.
        assert_eq!(storage.schema_id, vm.builtin_schemas.any_error as u64);
        assert_eq!(storage.slots.len(), 6);
        assert_eq!(storage.field_kinds.len(), 6);

        // All field_kinds are NativeKind::String per the schema's
        // all-String declaration.
        for k in storage.field_kinds.iter() {
            assert_eq!(*k, NativeKind::String);
        }

        // The message field's bits are an Arc<String> raw pointer.
        let msg_bits = storage.slots[ANYERROR_MESSAGE].raw();
        assert!(msg_bits != 0);
        // SAFETY: field_kinds[ANYERROR_MESSAGE] = NativeKind::String;
        // slot bits are Arc::into_raw::<String>; storage owns the share.
        let msg_str: &String = unsafe { &*(msg_bits as *const String) };
        assert_eq!(msg_str.as_str(), "boom");

        // The category field is "RuntimeError".
        let cat_bits = storage.slots[ANYERROR_CATEGORY].raw();
        let cat_str: &String = unsafe { &*(cat_bits as *const String) };
        assert_eq!(cat_str.as_str(), "RuntimeError");

        // The cause field is None (zero-bits + heap_mask bit clear).
        assert_eq!(storage.slots[ANYERROR_CAUSE].raw(), 0);
        assert_eq!((storage.heap_mask >> ANYERROR_CAUSE) & 1, 0);

        // Re-into_raw to balance the temporary Arc; the original
        // `result` carrier's Drop will release the storage share.
        let _ = Arc::into_raw(storage);
        drop(result);
    }

    /// `normalize_err_payload` wraps a String payload into an AnyError
    /// TypedObject; reading back via the storage's message slot
    /// recovers the original text.
    #[test]
    fn normalize_err_payload_wraps_string() {
        let mut vm = VirtualMachine::new(VMConfig::default());
        let raw = KindedSlot::from_string_arc(Arc::new("oops".to_string()));
        let wrapped = vm.normalize_err_payload(raw).unwrap();

        assert_eq!(wrapped.kind(), NativeKind::Ptr(HeapKind::TypedObject));
        let bits = wrapped.slot().raw();
        let storage: Arc<TypedObjectStorage> =
            unsafe { Arc::from_raw(bits as *const _) };
        let msg_bits = storage.slots[ANYERROR_MESSAGE].raw();
        let msg_str: &String = unsafe { &*(msg_bits as *const String) };
        assert_eq!(msg_str.as_str(), "oops");
        let _ = Arc::into_raw(storage);
        drop(wrapped);
    }

    /// `normalize_err_payload` on an already-AnyError TypedObject
    /// passes through unchanged (the same pointer bits flow through).
    #[test]
    fn normalize_err_payload_already_anyerror_passthrough() {
        let mut vm = VirtualMachine::new(VMConfig::default());
        let raw = KindedSlot::from_string_arc(Arc::new("inner".to_string()));
        let first = vm.normalize_err_payload(raw).unwrap();
        let first_bits = first.slot().raw();
        let again = vm.normalize_err_payload(first).unwrap();
        // Pass-through: same pointer bits.
        assert_eq!(again.slot().raw(), first_bits);
        assert_eq!(again.kind(), NativeKind::Ptr(HeapKind::TypedObject));
        drop(again);
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
