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
//! ### W13-result-option-ops audit (close 2026-05-10)
//!
//! W13-anyerror (close `e9c7260`) closed the AnyError TypedObject
//! builder + `op_throw` + dispatch.rs converters but explicitly
//! deferred 8 variant-discriminator opcodes (`op_type_check`,
//! `op_error_context`, `op_try_unwrap`, `op_unwrap_option`, `op_is_ok`,
//! `op_is_err`, `op_unwrap_ok`, `op_unwrap_err`) because they need a
//! `Result<_,_>` / `Option<_>` runtime representation that has not
//! been determined post-bulldozer.
//!
//! W13-result-option-ops audited the upstream substrate
//! (`BuiltinFunction::OkCtor` / `ErrCtor` / `SomeCtor`,
//! `HeapKind::Result` / `HeapKind::Option` candidacy, `__Result` /
//! `__Option` schema candidacy) and confirmed: the variant-codegen
//! producers in `executor/vm_impl/builtins.rs:510-518` are still
//! `todo!("phase-1b-vm wave 5e — Option/Result ctor body migration
//! pending")`; no HeapKind variant exists; no schema is registered.
//! Filling the consumer-side discriminator before the producer would
//! either fabricate the runtime contract (defection-attractor — same
//! shape as the deleted pre-bulldozer `extract_ok_inner` /
//! `extract_err_inner` raw_helpers) or surface against an empty
//! contract.
//!
//! The 8 ops were closed by the `W14-variant-codegen` re-emission
//! cluster — OkCtor / ErrCtor / SomeCtor body in
//! Wave-5e closure, register `__Result` / `__Option` schema OR amend
//! `HeapKind::Result` / `HeapKind::Option` per Q8 carrier-API-bound,
//! then close all 8 ops in a single follow-up.
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
    executor::result_option_carrier,
    executor::vm_impl::stack::drop_with_kind,
    executor::{ExceptionHandler, VirtualMachine},
    type_tracking::FrameReturnWrapper,
};
use shape_runtime::type_schema::builtin_schemas::{
    ANYERROR_CATEGORY, ANYERROR_CAUSE, ANYERROR_CODE, ANYERROR_MESSAGE, ANYERROR_PAYLOAD,
    ANYERROR_TRACE_INFO,
};
use shape_value::{HeapKind, KindedSlot, NativeKind, TypedObjectStorage, VMError, ValueSlot};
use std::sync::Arc;

// WS-3 F4: the `PHASE_2C_EXCEPTION_OBJECT_SURFACE` and
// `PHASE_2C_VARIANT_CODEGEN_SURFACE` jargon literals were deleted. The
// machinery they documented as "pending" has since landed: `build_any_error`
// builds a real `Arc<TypedObjectStorage>`, the `OkCtor` / `ErrCtor` /
// `SomeCtor` producers are implemented (W14-variant-codegen close), and the
// 8 variant-discriminator opcodes are filled. The `handle_exception`
// no-handler branch now surfaces a clean `Uncaught error: <message>` instead
// of interpolating the internal-jargon literal.

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
            // No handler — an exception unwound past every `try` block.
            // WS-3 F4: surface a clean user-facing error. When the payload
            // is an AnyError TypedObject (the normal case post
            // `normalize_err_payload` / `build_any_error`), read its
            // `message` field and report `Uncaught error: <message>`.
            // Otherwise stringify the payload. The `trace_info` frame-walk
            // is a v0.4 follow-up (`trace_info_*` builders return empty
            // strings today) — a clean message simply omits the trace.
            let message = self.uncaught_error_message(&payload);
            self.set_last_uncaught_exception(payload.clone());
            // Release the payload share via `KindedSlot::Drop`
            // (kind-dispatched refcount retire per §2.7.6 / Q8) so the
            // kind track stays balanced.
            drop(payload);
            Err(VMError::RuntimeError(message))
        }
    }

    /// WS-3 F4: render a clean user-facing message for an uncaught
    /// exception. Reads the AnyError TypedObject's `message` field when
    /// the payload carries one; falls back to a stringified payload.
    ///
    /// PB1 Wave-1-extension fold-in (audit 14a, 2026-05-29): when the
    /// AnyError carries a `cause` field, include the cause-chain entry
    /// alongside the high-level message so the user-facing render
    /// preserves both layers (matches the book §"## Uncaught Exception
    /// Display" example `Error: <message>` + `Caused by: <cause>` —
    /// minus the trace-frame walk which remains v0.4 follow-up
    /// territory per audit 14a §"Phase B fix-target preview" Q3).
    /// Required for the 6 `context_op_*_includes_*_cause` /
    /// `context_op_err_preserves_cause` Group A tests that assert the
    /// cause is visible in the user-facing error string.
    fn uncaught_error_message(&self, payload: &KindedSlot) -> String {
        if let NativeKind::Ptr(HeapKind::TypedObject) = payload.kind() {
            let bits = payload.slot().raw();
            if bits != 0 {
                // SAFETY: kind says Ptr(TypedObject); bits are
                // `Arc::into_raw::<TypedObjectStorage>`. Borrow transiently
                // (no share retire) to inspect the schema + message field.
                let obj: &TypedObjectStorage = unsafe { &*(bits as *const TypedObjectStorage) };
                if obj.schema_id == self.builtin_schemas.any_error as u64 {
                    let msg = anyerror_message_field(obj);
                    let cause = anyerror_cause_field(obj);
                    return match (msg, cause) {
                        (Some(m), Some(c)) => {
                            format!("Uncaught error: {}\nCaused by: {}", m, c)
                        }
                        (Some(m), None) => format!("Uncaught error: {}", m),
                        (None, Some(c)) => format!("Uncaught error: {}", c),
                        (None, None) => "Uncaught error".to_string(),
                    };
                }
            }
        }
        let formatter =
            crate::executor::printing::ValueFormatter::new(&self.program.type_schema_registry);
        format!("Uncaught error: {}", formatter.format_kinded(payload))
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
            IsTryFailure => self.op_is_try_failure()?,
            UnwrapOption => self.op_unwrap_option()?,
            CoalesceProbe => self.op_coalesce_probe()?,
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
    /// SURFACE (W13-result-option-ops audit, 2026-05-10): the runtime-
    /// tier `check_instanceof` body needs to dispatch a
    /// `TypeAnnotation` (Basic / Reference / Generic{Result|Option,
    /// args} / Array / Tuple / Object / Function / Union / Null / ...)
    /// against an arbitrary `KindedSlot` per §2.7.6 / Q8. The
    /// `Generic { Result, [T, E] }` and `Generic { Option, [T] }`
    /// arms specifically need the variant-discriminator contract that
    /// `op_is_ok` / `op_is_err` / `op_unwrap_option` are blocked on
    /// (W14-variant-codegen close). The Basic-scalar arms
    /// (int / number / bool / string) could land independently, but
    /// the compiler in `compiler/patterns/checking.rs:91` and
    /// `compiler/expressions/type_ops.rs:837` emits `TypeCheck` against
    /// any annotation including Result/Option, so a partial body would
    /// cover only a fraction of emit sites and silently regress the
    /// rest — surface-and-stop is the right shape until W14-variant-
    /// codegen lands. Until then we drop the popped carrier (kind-
    /// dispatched refcount retire via `KindedSlot::Drop` per Q8) and
    /// surface so the stack stays balanced.
    pub(in crate::executor) fn op_type_check(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let (value_bits, value_kind) = self.pop_kinded()?;
        let value = KindedSlot::new(ValueSlot::from_raw(value_bits), value_kind);
        let annotation = match instruction.operand {
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

        // Wave 14 W14-variant-codegen (ADR-006 §2.7.17 / Q18,
        // 2026-05-10): match the kinded carrier against the
        // TypeAnnotation. Basic-scalar arms (int / number / bool /
        // string) match the kind tag; Result/Option Generic arms
        // match via the carrier kind. The semi-bounded matcher below
        // covers the common emit sites; richer match (Union /
        // Intersection / Reference / structural Object / Tuple /
        // Function) lands as a follow-up — for now they conservatively
        // reject (push false), preserving the surface-and-stop
        // refusal-to-fabricate discipline at the cost of false
        // negatives on those forms.
        let matches = type_check_kinded(&self.builtin_schemas, &annotation, &value)?;
        drop(value);
        self.push_kinded_slot(KindedSlot::from_bool(matches))
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

    #[allow(dead_code)]
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
        let field_kinds: Arc<[NativeKind]> =
            Arc::from(vec![NativeKind::String; 6].into_boxed_slice());

        // Wave 2 Round 4 D4 ckpt-1: migrated to v2-raw `_new` + D1's
        // `from_typed_object_raw` constructor — no variant signature
        // dependency at this site.
        let ptr = TypedObjectStorage::_new(
            schema_id as u64,
            slots.into_boxed_slice(),
            heap_mask,
            field_kinds,
        );
        Ok(KindedSlot::from_typed_object_raw(ptr))
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
                // R5 soundness (2026-06-23): `bits` is a v2-raw
                // `TypedObjectStorage::_new` pointer (HeapHeader at offset 0),
                // NOT `Arc::into_raw`. Recovering it via `Arc::from_raw`
                // steps `byte_sub(16)` into a non-ArcInner allocation → Miri
                // UB. Mirror `uncaught_error_message` (~:201): read schema_id
                // through a transient read-only `&TypedObjectStorage` — no
                // `Arc::from_raw`, no refcount touch (the `payload` carrier
                // keeps owning its single share).
                // SAFETY: kind says Ptr(TypedObject); `bits` is a live `_new`
                // pointer (non-null, checked above) owned by `payload`; the
                // borrow is a read-only `schema_id` read that does not escape.
                let obj: &TypedObjectStorage = unsafe { &*(bits as *const TypedObjectStorage) };
                let is_any_error = obj.schema_id == self.builtin_schemas.any_error as u64;
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

    /// `ErrorContext` (`!!` operator): pop context + value, produce a
    /// `Result<T, AnyError>` carrier per the canonical Shape contract.
    ///
    /// Canonical contract (book reference:
    /// `shape-web/book/book-site/src/content/docs/fundamentals/error-handling.mdx`
    /// L232-248 "## `!!` Error Context Operator", as of 2026-05-28):
    ///
    /// > `lhs !! rhs` adds higher-level context and **always** yields a `Result`.
    ///
    /// | `lhs`           | `lhs !! rhs`                                                                  |
    /// | :-------------- | :---------------------------------------------------------------------------- |
    /// | `Ok(v)`         | `Ok(v)`                                                                       |
    /// | `Some(v)`       | `Ok(v)`                                                                       |
    /// | `Err(e)`        | `Err(AnyError { payload: rhs, cause: e, trace_info: single-frame })`          |
    /// | `None`          | `Err(AnyError { payload: rhs, cause: AnyError("Value was None"),
    ///                          trace_info: single-frame })`                                          |
    /// | plain value `v` | `Ok(v)`                                                                       |
    ///
    /// `!!` is **purely a wrap operator** — it never throws. To turn a
    /// wrapped `Err(_)` into a propagated exception, the user composes
    /// with `?`: `expr !! "context"?`. The throw happens in `op_try_unwrap`
    /// + `?`-on-Err early-return when the enclosing fallible-fn frame
    /// surfaces an uncaught Err to the host.
    ///
    /// Doc-coherence: this body, the docstring above, the book chapter
    /// `error-handling.mdx` L232-248, and `docs/cluster-audits/v0.3.3/
    /// 07-result-bang-and-try-broken.md` §8 (empirical correction)
    /// all describe the same contract — WRAP. Audit doc 07 §1.1's
    /// pre-fix "Expected: program raises `Uncaught error: ...`" entry
    /// is empirically FALSIFIED post JOINT-FIX-1a; per the book the
    /// canonical post-fix behavior is `print(r)` showing the wrapped
    /// `Err(AnyError { ... })` value.
    ///
    /// Refcount discipline (per ADR-006 §2.7.6 / Q8 + WB2.4): the
    /// inner extraction paths (`rd.payload.clone()`, `od.payload.clone()`)
    /// retain the inner share via `KindedSlot::Clone`; the outer
    /// wrapper (`value`) is then dropped, releasing the source carrier's
    /// strong-count. For Err/None paths the `context` carrier's share
    /// transfers into the AnyError's `cause`/`payload` String field via
    /// `build_any_error`'s stringify path (no leak).
    pub(in crate::executor) fn op_error_context(&mut self) -> Result<(), VMError> {
        let (context_bits, context_kind) = self.pop_kinded()?;
        let (value_bits, value_kind) = self.pop_kinded()?;
        let context = KindedSlot::new(ValueSlot::from_raw(context_bits), context_kind);
        let value = KindedSlot::new(ValueSlot::from_raw(value_bits), value_kind);

        // Five branches per the canonical contract table above. Every
        // branch produces a canonical `__Result` TypedObject carrier
        // (kind = `Ptr(HeapKind::TypedObject)`). NO throw path:
        // `!!` is a wrap operator, not a runtime assertion.
        //
        // Doc-coherence binder (JOINT-FIX-1a, 2026-05-28): if you find
        // yourself reaching for `handle_exception` in any branch below,
        // STOP — that contradicts the book + the 26 Group A WRAP-shaped
        // tests. The throw shape was the deleted pre-fix behavior; do
        // not reintroduce.
        if let Some(rd) = read_result(self, &value)? {
            if rd.is_ok() {
                // Ok(v) → Ok(v). Re-wrap as a fresh `__Result` carrier.
                // `context` is discarded (book: "should not appear").
                let inner = rd.clone_payload()?;
                drop(rd);
                drop(value);
                drop(context);
                self.push_kinded_slot(result_option_carrier::build_ok(
                    &self.builtin_schemas,
                    inner,
                ))
            } else {
                // Err(e) → Err(AnyError { payload: rhs, cause: e, ... }).
                // The book contract puts the high-level context as the
                // visible `payload`/`message`, and the original error
                // as the `cause`. The uncaught-render path reads
                // `message` (= payload) so the user sees the context.
                let inner = rd.clone_payload()?;
                drop(rd);
                drop(value);
                let trace = self.trace_info_full()?;
                let any_err = self.build_any_error(context, Some(inner), trace, None)?;
                self.push_kinded_slot(result_option_carrier::build_err(
                    &self.builtin_schemas,
                    any_err,
                ))
            }
        } else if let Some(od) = read_option(self, &value)? {
            if od.is_some() {
                // Some(v) → Ok(v). Lift Option → Result. `context`
                // discarded.
                let inner = od.clone_payload()?;
                drop(od);
                drop(value);
                drop(context);
                self.push_kinded_slot(result_option_carrier::build_ok(
                    &self.builtin_schemas,
                    inner,
                ))
            } else {
                // None → Err(AnyError { payload: rhs, cause: "Value was None", ... }).
                drop(od);
                drop(value);
                let none_cause =
                    KindedSlot::from_string_arc(Arc::new("Value was None".to_string()));
                let trace = self.trace_info_full()?;
                let any_err = self.build_any_error(context, Some(none_cause), trace, None)?;
                self.push_kinded_slot(result_option_carrier::build_err(
                    &self.builtin_schemas,
                    any_err,
                ))
            }
        } else if is_null_sentinel(&value) {
            // Null-coded None → Err(AnyError { payload: rhs, cause: "Value was None", ... }).
            // Same shape as the typed-None branch — null-coding is the
            // wire-tier representation of None, the user-facing
            // contract is identical.
            drop(value);
            let none_cause = KindedSlot::from_string_arc(Arc::new("Value was None".to_string()));
            let trace = self.trace_info_full()?;
            let any_err = self.build_any_error(context, Some(none_cause), trace, None)?;
            self.push_kinded_slot(result_option_carrier::build_err(
                &self.builtin_schemas,
                any_err,
            ))
        } else {
            // Bare non-null value `v` → Ok(v). `context` discarded.
            // Per book: "plain value `v` | `Ok(v)`". This is NOT
            // null-coding pass-through — the value must be wrapped in
            // Ok so the value is explicitly wrapped in `__Result`.
            drop(context);
            self.push_kinded_slot(result_option_carrier::build_ok(
                &self.builtin_schemas,
                value,
            ))
        }
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
    /// SURFACE (W13-result-option-ops audit, 2026-05-10): two
    /// substrate gaps stack here.
    ///
    /// (1) Variant discriminator — same gap as `op_is_ok` /
    /// `op_unwrap_ok` (W14-variant-codegen close): no
    /// determined runtime representation for `Result<_,_>` because
    /// `BuiltinFunction::OkCtor` / `ErrCtor` are still `todo!()`. The
    /// pre-bulldozer `extract_ok_inner` / `extract_err_inner` /
    /// `extract_some_inner` / `is_none` raw_helpers are deleted
    /// (forbidden #7 in playbook §4).
    ///
    /// (2) Early-return (Err propagation) machinery: `?` must
    /// terminate the current call frame and return `Err(_)` /
    /// AnyError-wrapped None to the caller, NOT unwind to the nearest
    /// try-block (`handle_exception`'s contract). Two distinct
    /// behaviors that have to be threaded explicitly. The current
    /// frame-return path is `op_return` in `executor/control_flow/
    /// mod.rs` which pops a single result slot and pops the call
    /// frame; the `?`-from-fallible-fn body would call into the same
    /// frame-pop path with the Err/None-wrapped slot as the result.
    /// This second half is buildable on top of existing machinery
    /// (no new ABI), but is meaningless without the variant
    /// discriminator from (1).
    pub(in crate::executor) fn op_try_unwrap(&mut self) -> Result<(), VMError> {
        let (bits, kind) = self.pop_kinded()?;
        let value = KindedSlot::new(ValueSlot::from_raw(bits), kind);
        // Wave 14 W14-variant-codegen (ADR-006 §2.7.17 / Q18,
        // 2026-05-10) + PB1 Wave-1-extension (audit 14a / 14b,
        // 2026-05-29):
        //   Ok(v)   => unwrap to v
        //   Err(e)  => early-return:
        //                - in a fallible-fn frame: return Err(e) to caller
        //                - at script-toplevel: surface as uncaught
        //                  exception via handle_exception (audit 14a
        //                  decision (A) Throw-at-toplevel)
        //   Some(v) => unwrap to v
        //   None    => early-return:
        //                - in a Result-returning fn: LIFT to
        //                  Err(AnyError{OPTION_NONE,...}) and return
        //                  to caller (audit 14b sub-root #1 inside-fn
        //                  None-to-Err lift)
        //                - in an Option-returning fn: propagate None
        //                  verbatim (early-return)
        //                - at script-toplevel: surface as uncaught
        //                  Err(AnyError{OPTION_NONE,...})
        //   bare non-null => pass through (null-coding fallback;
        //                    treated as None at script-toplevel)
        //
        // Audit 14a single-mode framing: `?` early-returns the failure
        // to the nearest enclosing fallible scope. Inside a function
        // returning Result/Option that's an early return to the caller
        // frame; at script-toplevel the enclosing scope is the host
        // process which surfaces the failure as an uncaught error. ONE
        // semantics, two targets — the `self.call_stack.is_empty()`
        // check at each early-return site expresses the target
        // dispatch. There is no `op_try_unwrap_toplevel` variant.
        if let Some(rd) = read_result(self, &value)? {
            if rd.is_ok() {
                let inner = rd.clone_payload()?;
                drop(rd);
                drop(value);
                self.push_kinded_slot(inner)
            } else {
                // Err(e) — early-return.
                let result_kind = value.kind();
                let result_bits = value.slot().raw();
                if self.call_stack.is_empty() {
                    // Toplevel: surface as uncaught exception (audit
                    // 14a (A)). The Err wrapper's payload is normalized
                    // via `normalize_err_payload` (already-AnyError
                    // payloads pass through unchanged; raw payloads
                    // get wrapped) so the catch-render path sees a
                    // canonical TypedObject. We unwrap the Result
                    // shell to expose the inner error payload — `?`
                    // surfaces the error, not the Result-wrapped
                    // carrier (mirrors how a fn-position `?` exposes
                    // the Err carrier as the frame return; here the
                    // host frame consumes the inner directly).
                    let inner = rd.clone_payload()?;
                    drop(rd);
                    drop(value);
                    let normalized = self.normalize_err_payload(inner)?;
                    self.handle_exception(normalized)
                } else {
                    // In-fn: return Err(e) wrapper to caller. We
                    // re-emit the wrapper carrier as the call frame's
                    // return value, NOT unwrap the inner — `?` on Err
                    // propagates the wrapper to the enclosing fallible
                    // scope.
                    drop(rd);
                    std::mem::forget(value);
                    self.return_value_inner(result_bits, result_kind)
                }
            }
        } else if let Some(od) = read_option(self, &value)? {
            if od.is_some() {
                let inner = od.clone_payload()?;
                drop(od);
                drop(value);
                self.push_kinded_slot(inner)
            } else {
                // None — early-return. Target-dispatch identical to
                // the null-coded-None arm below; both delegate to
                // `propagate_none_early_return`.
                drop(od);
                drop(value);
                self.propagate_none_early_return()
            }
        } else if is_null_sentinel(&value) {
            // null-coded None — same shape as typed-None arm.
            drop(value);
            self.propagate_none_early_return()
        } else {
            // Bare non-null value — null-coded Some(x) ≡ x. Pass
            // through the share verbatim via mem::forget (no
            // clone+drop pair).
            let kind = value.kind();
            let bits = value.slot().raw();
            std::mem::forget(value);
            self.push_kinded(bits, kind)
        }
    }

    /// `IsTryFailure`: non-consuming classifier for the `?` lowering's
    /// pending-Drop branch. Pops one carrier (retiring its share via the
    /// kinded `KindedSlot::Drop`), pushes a `Bool`:
    ///   - `true`  when `op_try_unwrap` WOULD short-circuit:
    ///       Err(e) / None / null-coded-None
    ///   - `false` when `op_try_unwrap` would unwrap-and-continue:
    ///       Ok(v) / Some(v) / bare non-null value
    ///
    /// The compiler emits `Dup; IsTryFailure; JumpIfFalse SUCCESS;
    /// <DropCall for each other in-scope Drop local>; SUCCESS: TryUnwrap`.
    /// `Dup` bumps the carrier's refcount so this classifier's popped copy
    /// and the `TryUnwrap` consumer each own an independent share — no
    /// over/under-count. Classification routes through the SAME
    /// `read_result` / `read_option` / `is_null_sentinel` helpers as
    /// `op_try_unwrap`, so the branch the compiler takes can never diverge
    /// from the branch the opcode takes (single source of truth).
    pub(in crate::executor) fn op_is_try_failure(&mut self) -> Result<(), VMError> {
        let (bits, kind) = self.pop_kinded()?;
        let value = KindedSlot::new(ValueSlot::from_raw(bits), kind);
        let is_failure = if let Some(rd) = read_result(self, &value)? {
            !rd.is_ok()
        } else if let Some(od) = read_option(self, &value)? {
            !od.is_some()
        } else {
            // null-coded None => failure; bare non-null value => success.
            is_null_sentinel(&value)
        };
        drop(value);
        self.push_kinded_slot(KindedSlot::from_bool(is_failure))
    }

    /// PB1 Wave-1-extension (audit 14a + 14b, 2026-05-29): shared
    /// early-return helper for the None / null-coded-None arms of
    /// `op_try_unwrap`. Target-dispatched per the audit-14a single-mode
    /// framing (one semantics, two targets):
    ///
    /// - **Script-toplevel** (`self.call_stack.is_empty()`): surface
    ///   `Err(AnyError{ payload: "Value was None", code: "OPTION_NONE" })`
    ///   as an uncaught exception via `handle_exception`. Audit 14a §c
    ///   sibling-consistency analysis: the host is the enclosing
    ///   fallible scope for bare-toplevel `?`.
    ///
    /// - **Inside-fn** — discriminate by the enclosing frame's declared
    ///   return-wrapper semantics (read from
    ///   `current_frame_descriptor().effective_return_wrapper()`):
    ///
    ///   - **Result-returning** (`return_wrapper == Result`): LIFT None to
    ///     `Err(AnyError{ payload: "Value was None", code: "OPTION_NONE" })`
    ///     wrapped in a Result-Err carrier, and return that to the
    ///     caller. Per audit 14b sub-root #1 inside-fn None-to-Err
    ///     lift, this is the documented book contract (L114: "None →
    ///     early-return `Err(AnyError)` (code `OPTION_NONE`)").
    ///
    ///   - **Option-returning**, **plain**, or **unknown**: propagate
    ///     None verbatim as the early-return value. For Option-typed
    ///     enclosing frames, None IS the valid early-return; lifting to
    ///     Err would break that semantics. For unknown frame metadata
    ///     (no FrameDescriptor or no wrapper stamp), propagate verbatim
    ///     — the pre-PB1 behavior is preserved for any compile-time emit
    ///     site that hasn't reached return-wrapper-stamp territory yet.
    ///
    /// Per audit 14a binder: this is NOT two operators. The `?`
    /// semantics is single-mode "early-return failure to nearest
    /// enclosing fallible scope"; the target (host vs frame) and the
    /// frame-return-type sub-case (Result-lift vs Option-propagate)
    /// are mechanical dispatch, not parallel implementations. No
    /// `op_try_unwrap_toplevel` variant; no `op_try_unwrap_option_fn`
    /// variant.
    fn propagate_none_early_return(&mut self) -> Result<(), VMError> {
        if self.call_stack.is_empty() {
            // Toplevel: surface OPTION_NONE AnyError as uncaught
            // exception. Build the AnyError with payload = "Value was
            // None" and code = "OPTION_NONE" per the book contract.
            let any_err = self.build_option_none_any_error()?;
            return self.handle_exception(any_err);
        }

        // In-fn: discriminate on the enclosing frame's declared
        // return-wrapper semantics. `effective_return_wrapper` retains
        // compatibility with old descriptors that encoded Result/Option
        // in `return_kind`.
        let return_wrapper = self
            .current_frame_descriptor()
            .map(|fd| fd.effective_return_wrapper())
            .unwrap_or(FrameReturnWrapper::Unknown);
        let lift_to_result_err = matches!(return_wrapper, FrameReturnWrapper::Result);

        if lift_to_result_err {
            // Result-returning fn: LIFT None to Err(AnyError) wrapped
            // in a Result-Err carrier, then return to caller. Audit
            // 14b sub-root #1 (inside-fn None-to-Err lift). Matches
            // the book contract at L114.
            let any_err = self.build_option_none_any_error()?;
            let carrier = result_option_carrier::build_err(&self.builtin_schemas, any_err);
            let bits = carrier.slot().raw();
            let kind = carrier.kind();
            std::mem::forget(carrier);
            return self.return_value_inner(bits, kind);
        }

        // Option-returning fn OR unknown frame return kind:
        // propagate None verbatim as the early-return value. For
        // Option-typed frames this IS the correct early-return; for
        // unknown frames the pre-PB1 behavior is preserved (the
        // null-sentinel propagates as today).
        self.return_value_inner(Self::NONE_BITS, NativeKind::Null)
    }

    /// PB1 Wave-1-extension (audit 14a + 14b): build an
    /// `Arc<TypedObjectStorage>` AnyError carrier with the canonical
    /// OPTION_NONE shape per the book contract (L114): `payload =
    /// "Value was None"`, `code = "OPTION_NONE"`. Returned as a
    /// `KindedSlot` with kind `NativeKind::Ptr(HeapKind::TypedObject)`
    /// owning one strong-count share — the caller transfers the share
    /// either to `handle_exception` (toplevel surface) or into a
    /// `__Result` Err carrier (in-fn lift).
    fn build_option_none_any_error(&mut self) -> Result<KindedSlot, VMError> {
        let payload = KindedSlot::from_string_arc(Arc::new("Value was None".to_string()));
        let trace = self.trace_info_full()?;
        self.build_any_error(payload, None, trace, Some("OPTION_NONE"))
    }

    /// `UnwrapOption` (`opt!`-style): pop a `T?` and unwrap to `T`,
    /// throwing if `None`.
    ///
    /// SURFACE (W13-result-option-ops audit, 2026-05-10): the audit
    /// observed that compiler emit sites for `UnwrapOption` (the only
    /// site is `compile_match_binding_local` at
    /// `compiler/patterns/binding.rs:417`) are guarded by an
    /// `op_is_null` test in the corresponding pattern-checking phase
    /// (`compiler/patterns/checking.rs:241`), so today's `Option`
    /// representation is null-coding (`Some(x)` ≡ `x`, `None` ≡ the
    /// null sentinel routed through `is_null_kinded` per
    /// `executor/comparison/mod.rs:383`). A null-coding-only body
    /// would be: pop, if `is_null_kinded(bits, kind)` throw via
    /// `handle_exception`, else push back. BUT the `Some(x)` producer
    /// (`BuiltinFunction::SomeCtor` in `executor/vm_impl/builtins.rs:
    /// 510-518`) is still `todo!()`, so no `T?` value with a present
    /// payload can flow through this opcode end-to-end today — and
    /// once `SomeCtor` lands, the ctor's chosen representation may
    /// elect a non-null-coded shape (e.g. `Arc<TypedObjectStorage>`
    /// schema-wrapped, mirroring AnyError) which would invalidate a
    /// null-coding-only body. Surface-and-stop is the correct shape:
    /// the consumer body is contracted by the producer choice, which
    /// must land first (W14-variant-codegen close).
    pub(in crate::executor) fn op_unwrap_option(&mut self) -> Result<(), VMError> {
        let (bits, kind) = self.pop_kinded()?;
        let value = KindedSlot::new(ValueSlot::from_raw(bits), kind);
        // The canonical Option<T> representation is a `__Option`
        // TypedObject. The legacy null-coding path (where `Some(x) ≡ x` and `None ≡ null
        // sentinel`) is preserved as a fallback for compiler emit
        // sites that haven't migrated to the kinded ctor yet — the
        // pattern-checking phase still emits `LoadLocal; IsNull;
        // JumpIfTrue fail` for `None`. Both forms surface the inner
        // value on success.
        let inner = match read_option(self, &value)? {
            Some(od) if od.is_some() => {
                let inner = od.clone_payload()?;
                drop(od);
                inner
            }
            Some(od) => {
                drop(od);
                drop(value);
                return Err(VMError::RuntimeError(
                    "called UnwrapOption on None value".to_string(),
                ));
            }
            None => {
                // Not an Option-kinded slot — apply null-coding fallback.
                if is_null_sentinel(&value) {
                    drop(value);
                    return Err(VMError::RuntimeError(
                        "called UnwrapOption on null/None value".to_string(),
                    ));
                }
                // Bare non-null value — pass through (null-coding's
                // `Some(x) ≡ x` shape). Transfer the share verbatim
                // via mem::forget (no clone+drop pair).
                let kind = value.kind();
                let bits = value.slot().raw();
                std::mem::forget(value);
                return self.push_kinded(bits, kind);
            }
        };
        drop(value);
        self.push_kinded_slot(inner)
    }

    /// `CoalesceProbe` (`??`): pop one value and push back TWO slots
    /// `[present_value, is_absent_bool]`. The `??` lowering replaces its
    /// `Dup; IsNull` prologue with this opcode so that an `Option<T>`
    /// carrier is correctly UNWRAPPED to its inner `T`
    /// on the present branch instead of leaking the whole `Some(v)` wrapper.
    ///
    /// v0.3.3 book-gate fix: `Some(5) ?? 99 -> 5` (was `Some(5)`).
    ///
    /// Cases (mirrors `op_try_unwrap` / `op_unwrap_option` discriminator):
    ///   - `Some(v)` Option → push inner `v` (cloned share), Bool(false)
    ///   - `None` Option     → push Null placeholder, Bool(true)
    ///   - null sentinel     → push the sentinel back, Bool(true)
    ///   - bare non-null     → push the value back (null-coding `Some(x)≡x`),
    ///                         Bool(false)
    ///
    /// The `is_absent_bool` matches `IsNull` polarity (`true` == absent),
    /// so the existing `JumpIfFalse use_lhs` branch structure is unchanged.
    pub(in crate::executor) fn op_coalesce_probe(&mut self) -> Result<(), VMError> {
        let (bits, kind) = self.pop_kinded()?;
        let value = KindedSlot::new(ValueSlot::from_raw(bits), kind);
        if let Some(od) = read_option(self, &value)? {
            if od.is_some() {
                // Some(v): retain a share of the inner payload, drop the
                // Option wrapper, push inner + present(false).
                let inner = od.clone_payload()?;
                drop(od);
                drop(value);
                self.push_kinded_slot(inner)?;
                self.push_kinded_slot(KindedSlot::from_bool(false))
            } else {
                // None: drop the wrapper, push a Null placeholder (it is
                // discarded by the `??` lowering on the absent branch) +
                // absent(true).
                drop(od);
                drop(value);
                self.push_kinded(Self::NONE_BITS, NativeKind::Null)?;
                self.push_kinded_slot(KindedSlot::from_bool(true))
            }
        } else if is_null_sentinel(&value) {
            // null-coded None.
            drop(value);
            self.push_kinded(Self::NONE_BITS, NativeKind::Null)?;
            self.push_kinded_slot(KindedSlot::from_bool(true))
        } else {
            // Bare non-null value — null-coding `Some(x) ≡ x`. Pass the
            // share through verbatim via mem::forget (no clone+drop pair),
            // then push present(false).
            let kind = value.kind();
            let bits = value.slot().raw();
            std::mem::forget(value);
            self.push_kinded(bits, kind)?;
            self.push_kinded_slot(KindedSlot::from_bool(false))
        }
    }

    /// `IsOk`: pop a `Result<_,_>`, push `Bool` indicating Ok variant.
    ///
    /// The value arrives as a canonical `__Result` TypedObject. Its
    /// variant discriminator drives the pushed Bool, and payload ownership
    /// is retained/released through the object's `field_kinds` table.
    #[inline(always)]
    pub(in crate::executor) fn op_is_ok(&mut self) -> Result<(), VMError> {
        let (bits, kind) = self.pop_kinded()?;
        let value = KindedSlot::new(ValueSlot::from_raw(bits), kind);
        let is_ok = match read_result(self, &value)? {
            Some(rd) => rd.is_ok(),
            None => false,
        };
        drop(value);
        self.push_kinded_slot(KindedSlot::from_bool(is_ok))
    }

    /// `IsErr`: pop a `Result<_,_>`, push `Bool` indicating Err variant.
    ///
    /// Wave 14 W14-variant-codegen (ADR-006 §2.7.17 / Q18, 2026-05-10):
    /// mirror of `op_is_ok` with inverted discriminator.
    #[inline(always)]
    pub(in crate::executor) fn op_is_err(&mut self) -> Result<(), VMError> {
        let (bits, kind) = self.pop_kinded()?;
        let value = KindedSlot::new(ValueSlot::from_raw(bits), kind);
        let is_err = match read_result(self, &value)? {
            Some(rd) => !rd.is_ok(),
            None => false,
        };
        drop(value);
        self.push_kinded_slot(KindedSlot::from_bool(is_err))
    }

    /// `UnwrapOk`: pop an `Ok(_)`, push the inner value.
    ///
    /// Retain-on-extract per WB2.4 / §2.7.7: the wrapper owns the payload
    /// field share, `clone_payload` bumps a new share for the stack, and
    /// dropping the wrapper retires only the wrapper-owned share.
    #[inline(always)]
    pub(in crate::executor) fn op_unwrap_ok(&mut self) -> Result<(), VMError> {
        let (bits, kind) = self.pop_kinded()?;
        let value = KindedSlot::new(ValueSlot::from_raw(bits), kind);
        let inner = match read_result(self, &value)? {
            Some(rd) if rd.is_ok() => {
                let inner = rd.clone_payload()?;
                drop(rd);
                inner
            }
            Some(rd) => {
                drop(rd);
                drop(value);
                return Err(VMError::RuntimeError(
                    "called UnwrapOk on Err value".to_string(),
                ));
            }
            None => {
                drop(value);
                return Err(VMError::RuntimeError(format!(
                    "UnwrapOk: expected Result, got kind {:?}",
                    kind
                )));
            }
        };
        drop(value);
        self.push_kinded_slot(inner)
    }

    /// `UnwrapErr`: pop an `Err(_)`, push the inner error value.
    ///
    /// Wave 14 W14-variant-codegen (ADR-006 §2.7.17 / Q18, 2026-05-10):
    /// mirror of `op_unwrap_ok` with the inverted variant gate.
    #[inline(always)]
    pub(in crate::executor) fn op_unwrap_err(&mut self) -> Result<(), VMError> {
        let (bits, kind) = self.pop_kinded()?;
        let value = KindedSlot::new(ValueSlot::from_raw(bits), kind);
        let inner = match read_result(self, &value)? {
            Some(rd) if !rd.is_ok() => {
                let inner = rd.clone_payload()?;
                drop(rd);
                inner
            }
            Some(rd) => {
                drop(rd);
                drop(value);
                return Err(VMError::RuntimeError(
                    "called UnwrapErr on Ok value".to_string(),
                ));
            }
            None => {
                drop(value);
                return Err(VMError::RuntimeError(format!(
                    "UnwrapErr: expected Result, got kind {:?}",
                    kind
                )));
            }
        };
        drop(value);
        self.push_kinded_slot(inner)
    }
}

// =========================================================================
// Result/Option carrier helpers
// =========================================================================

#[inline]
fn read_result<'a>(
    vm: &VirtualMachine,
    slot: &'a KindedSlot,
) -> Result<Option<result_option_carrier::ResultCarrier<'a>>, VMError> {
    result_option_carrier::read_result(&vm.builtin_schemas, slot)
}

#[inline]
fn read_option<'a>(
    vm: &VirtualMachine,
    slot: &'a KindedSlot,
) -> Result<Option<result_option_carrier::OptionCarrier<'a>>, VMError> {
    result_option_carrier::read_option(&vm.builtin_schemas, slot)
}

/// Exhaustive HeapKind sink for pointer-null checks.
#[inline]
fn heap_ptr_is_null(bits: u64, heap_kind: HeapKind) -> bool {
    match heap_kind {
        HeapKind::String
        | HeapKind::TypedObject
        | HeapKind::Closure
        | HeapKind::Decimal
        | HeapKind::BigInt
        | HeapKind::DataTable
        | HeapKind::Future
        | HeapKind::TaskGroup
        | HeapKind::TypedArray
        | HeapKind::Temporal
        | HeapKind::TableView
        | HeapKind::Content
        | HeapKind::Instant
        | HeapKind::IoHandle
        | HeapKind::NativeScalar
        | HeapKind::NativeView
        | HeapKind::Char
        | HeapKind::HashMap
        | HeapKind::FilterExpr
        | HeapKind::Reference
        | HeapKind::SharedCell
        | HeapKind::HashSet
        | HeapKind::Iterator
        | HeapKind::Deque
        | HeapKind::Channel
        | HeapKind::PriorityQueue
        | HeapKind::Range
        | HeapKind::Result
        | HeapKind::Option
        | HeapKind::TraitObject
        | HeapKind::Mutex
        | HeapKind::Atomic
        | HeapKind::Lazy
        | HeapKind::ModuleFn
        | HeapKind::Matrix
        | HeapKind::MatrixSlice => bits == 0,
    }
}

/// Whether a kinded carrier represents the `null` sentinel — used by
/// `op_unwrap_option`, `op_try_unwrap`, and `op_error_context` to
/// recognise the legacy null-coded Option half
/// (`compile_pattern_check_local` at `compiler/patterns/checking.rs:213`
/// still emits `LoadLocal; IsNull; JumpIfTrue fail` for `None`, so a
/// bare null sentinel reaching the discriminator must be treated as
/// None).
///
/// JOINT-FIX-1a (2026-05-28): added the `NativeKind::Null` arm to
/// mirror `comparison/mod.rs::is_null_kinded` after probe4
/// (`let r = None !! "missing"; print(r)`) revealed `None` literals
/// reach this site with `NativeKind::Null` (per R5b-2 disposition —
/// `PushNull` / `Constant::Null` producers stamp `NativeKind::Null`),
/// which the pre-fix arm-set did not classify as null and so fell
/// through to the bare-value branch.
///
/// Mirrors `comparison/mod.rs::is_null_kinded` exactly — `NativeKind::Null`
/// is decisive (bits unused per R5b-2); nullable kinds qualify on
/// zero-bits / NaN-bits; non-nullable scalars (including `0i64` and
/// `false`) are never null.
#[inline]
fn is_null_sentinel(slot: &KindedSlot) -> bool {
    let bits = slot.slot.raw();
    let kind = slot.kind;
    match kind {
        // R5b-2 disposition: Null IS the absence-of-value discriminator;
        // kind alone is decisive, bits unused.
        NativeKind::Null => true,
        NativeKind::String => bits == 0,
        NativeKind::Ptr(heap_kind) => heap_ptr_is_null(bits, heap_kind),
        NativeKind::NullableFloat64 => f64::from_bits(bits).is_nan(),
        NativeKind::NullableInt8
        | NativeKind::NullableInt16
        | NativeKind::NullableInt32
        | NativeKind::NullableInt64
        | NativeKind::NullableIntSize
        | NativeKind::NullableUInt8
        | NativeKind::NullableUInt16
        | NativeKind::NullableUInt32
        | NativeKind::NullableUInt64
        | NativeKind::NullableUIntSize => bits == 0,
        _ => false,
    }
}

/// Match a kinded carrier against a TypeAnnotation. Covers the common
/// emit sites for `op_type_check` per the W14-variant-codegen close —
/// Basic scalars + Result/Option generics. Other forms conservatively
/// return `false` rather than fabricate a match contract.
fn type_check_kinded(
    schemas: &shape_runtime::type_schema::BuiltinSchemaIds,
    annotation: &shape_ast::ast::TypeAnnotation,
    value: &KindedSlot,
) -> Result<bool, VMError> {
    use shape_ast::ast::TypeAnnotation;
    let matches = match annotation {
        TypeAnnotation::Basic(name) => match name.as_str() {
            "int" => matches!(
                value.kind,
                NativeKind::Int8
                    | NativeKind::Int16
                    | NativeKind::Int32
                    | NativeKind::Int64
                    | NativeKind::IntSize
                    | NativeKind::UInt8
                    | NativeKind::UInt16
                    | NativeKind::UInt32
                    | NativeKind::UInt64
                    | NativeKind::UIntSize
            ),
            "number" | "float" => matches!(value.kind, NativeKind::Float64),
            "bool" => matches!(value.kind, NativeKind::Bool),
            "string" => matches!(
                value.kind,
                NativeKind::String | NativeKind::Ptr(HeapKind::String)
            ),
            "char" => matches!(value.kind, NativeKind::Ptr(HeapKind::Char)),
            // WS-3: `emit_destructure_type_check` (`patterns/helpers.rs`)
            // passes the bare name `"array"` (not the `Generic { name:
            // "Array", .. }` form). Without this arm the `TypeCheck`
            // returned `false` for every `let [a, b, c] = xs`, the
            // guard `JumpIfTrue` did not jump, and the `Throw` fired —
            // surfacing the uncaught-exception path on a valid program.
            // Matches the `Generic "Array" | "Vec"` arm below.
            "array" => matches!(value.kind, NativeKind::Ptr(HeapKind::TypedArray)),
            // `let { … } = obj` lowers an `emit_destructure_type_check("object")`
            // guard before field extraction. The slot's `NativeKind` is stamped
            // at compile time; this is a kind-tag match (no decode, no
            // fabrication), the same shape as the `"string"` arm above.
            "object" => matches!(value.kind, NativeKind::Ptr(HeapKind::TypedObject)),
            _ => false,
        },
        TypeAnnotation::Null => value.slot.raw() == 0,
        TypeAnnotation::Generic { name, .. } => match name.as_str() {
            "Result" => return Ok(result_option_carrier::read_result(schemas, value)?.is_some()),
            "Option" => {
                return Ok(
                    result_option_carrier::read_option(schemas, value)?.is_some()
                        || is_null_sentinel(value),
                );
            }
            "Array" | "Vec" => matches!(value.kind, NativeKind::Ptr(HeapKind::TypedArray)),
            "HashMap" | "Map" => matches!(value.kind, NativeKind::Ptr(HeapKind::HashMap)),
            "HashSet" | "Set" => matches!(value.kind, NativeKind::Ptr(HeapKind::HashSet)),
            "Iterator" => matches!(value.kind, NativeKind::Ptr(HeapKind::Iterator)),
            _ => false,
        },
        // Other forms: structural Object / Tuple / Function / Union /
        // Intersection / Reference / Dyn — these need richer runtime
        // matching against schema_id / TypedObject layout, which is
        // its own follow-up. Return false rather than fabricate a
        // match (forbidden patterns: Bool-default fallback).
        _ => false,
    };
    Ok(matches)
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
            let arc: Arc<String> = unsafe { Arc::from_raw(bits as *const String) };
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

/// WS-3 F4: read the `message` field of an AnyError `TypedObjectStorage`
/// without consuming any strong-count share. Returns `None` when the
/// message slot is empty (heap_mask bit clear / zero bits) or absent.
///
/// The caller must already have verified `obj.schema_id` is the AnyError
/// schema id; that schema declares all 6 fields `String`-kinded, so the
/// `ANYERROR_MESSAGE` slot's bits are `Arc::into_raw::<String>` when the
/// `heap_mask` bit is set.
fn anyerror_message_field(obj: &TypedObjectStorage) -> Option<String> {
    let idx = ANYERROR_MESSAGE;
    if (obj.heap_mask >> idx) & 1 == 0 {
        return None;
    }
    let bits = obj.slots().get(idx)?.raw();
    if bits == 0 {
        return None;
    }
    // SAFETY: the AnyError schema declares field `ANYERROR_MESSAGE` as a
    // `String`; `build_any_error` stamps it with `Arc::into_raw::<String>`
    // bits and sets the matching `heap_mask` bit. Borrow the `&String`
    // for the duration of this read — no `Arc::from_raw`, so the object's
    // share is untouched.
    let s: &String = unsafe { &*(bits as *const String) };
    Some(s.clone())
}

/// PB1 Wave-1-extension fold-in (audit 14a, 2026-05-29): mirror of
/// `anyerror_message_field` for the `cause` field. Returns `None` when
/// the cause slot is empty (no cause chain — `!!` was applied to a
/// non-Err / non-None value) or the heap_mask bit is clear.
fn anyerror_cause_field(obj: &TypedObjectStorage) -> Option<String> {
    let idx = ANYERROR_CAUSE;
    if (obj.heap_mask >> idx) & 1 == 0 {
        return None;
    }
    let bits = obj.slots().get(idx)?.raw();
    if bits == 0 {
        return None;
    }
    // SAFETY: AnyError schema declares `ANYERROR_CAUSE` as `String`;
    // `build_any_error` stamps it with `Arc::into_raw::<String>` bits
    // and sets the matching `heap_mask` bit. Borrow the `&String` for
    // the duration of this read — no `Arc::from_raw`, so the object's
    // share is untouched.
    let s: &String = unsafe { &*(bits as *const String) };
    Some(s.clone())
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
        NativeKind::Float64 | NativeKind::NullableFloat64 => slot.slot().as_f64().to_string(),
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

        // R5 soundness (2026-06-23): TypedObject is a `_new`/HeapHeader
        // carrier — `bits` is NOT `Arc::into_raw`. Read through a transient
        // `&TypedObjectStorage` (no `Arc::from_raw`, no refcount touch). The
        // `result` carrier keeps owning its single share.
        // SAFETY: kind == Ptr(TypedObject); `bits` is a live `_new` pointer
        // (non-null, checked above) owned by `result`.
        let storage: &TypedObjectStorage = unsafe { &*(bits as *const TypedObjectStorage) };

        // Schema ID matches AnyError.
        assert_eq!(storage.schema_id, vm.builtin_schemas.any_error as u64);
        assert_eq!(storage.slots().len(), 6);
        assert_eq!(storage.field_kinds.len(), 6);

        // All field_kinds are NativeKind::String per the schema's
        // all-String declaration.
        for k in storage.field_kinds.iter() {
            assert_eq!(*k, NativeKind::String);
        }

        // The message field's bits are an Arc<String> raw pointer.
        let msg_bits = storage.slots()[ANYERROR_MESSAGE].raw();
        assert!(msg_bits != 0);
        // SAFETY: field_kinds[ANYERROR_MESSAGE] = NativeKind::String;
        // slot bits are Arc::into_raw::<String>; storage owns the share.
        let msg_str: &String = unsafe { &*(msg_bits as *const String) };
        assert_eq!(msg_str.as_str(), "boom");

        // The category field is "RuntimeError".
        let cat_bits = storage.slots()[ANYERROR_CATEGORY].raw();
        let cat_str: &String = unsafe { &*(cat_bits as *const String) };
        assert_eq!(cat_str.as_str(), "RuntimeError");

        // The cause field is None (zero-bits + heap_mask bit clear).
        assert_eq!(storage.slots()[ANYERROR_CAUSE].raw(), 0);
        assert_eq!((storage.heap_mask >> ANYERROR_CAUSE) & 1, 0);

        // No Arc to balance (transient `&` borrow). The `result` carrier's
        // Drop releases the storage share.
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
        // R5 soundness (2026-06-23): `_new` carrier — transient `&` read,
        // no `Arc::from_raw`. `wrapped` keeps owning its share.
        // SAFETY: kind == Ptr(TypedObject); `bits` is a live `_new` pointer.
        let storage: &TypedObjectStorage = unsafe { &*(bits as *const TypedObjectStorage) };
        let msg_bits = storage.slots()[ANYERROR_MESSAGE].raw();
        let msg_str: &String = unsafe { &*(msg_bits as *const String) };
        assert_eq!(msg_str.as_str(), "oops");
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
// L5 frame return metadata split tests
// =========================================================================

#[cfg(test)]
mod none_early_return_frame_metadata_tests {
    use super::*;
    use crate::bytecode::{BytecodeProgram, Function};
    use crate::executor::{CallFrame, VMConfig};
    use crate::type_tracking::{FrameDescriptor, FrameReturnWrapper};

    fn function_with_frame(frame: FrameDescriptor) -> Function {
        Function {
            name: "fallible".to_string(),
            arity: 0,
            param_names: Vec::new(),
            locals_count: 0,
            entry_point: 0,
            body_length: 0,
            is_closure: false,
            captures_count: 0,
            is_async: false,
            ref_params: Vec::new(),
            ref_mutates: Vec::new(),
            mutable_captures: Vec::new(),
            frame_descriptor: Some(frame),
            osr_entry_points: Vec::new(),
            mir_data: None,
        }
    }

    fn vm_in_function_frame(frame: FrameDescriptor) -> VirtualMachine {
        let mut program = BytecodeProgram::default();
        program.functions.push(function_with_frame(frame));

        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(program);
        vm.call_stack.push(CallFrame {
            return_ip: 0,
            base_pointer: 0,
            locals_count: 0,
            function_id: Some(0),
            upvalues: None,
            blob_hash: None,
            closure_heap_bits: None,
            closure_heap_kind: None,
        });
        vm
    }

    fn wrapper_frame(wrapper: FrameReturnWrapper) -> FrameDescriptor {
        let mut frame = FrameDescriptor::new();
        frame.return_kind = Some(NativeKind::Ptr(HeapKind::TypedObject));
        frame.return_wrapper = wrapper;
        frame
    }

    #[test]
    fn none_in_result_frame_with_typed_object_abi_lifts_to_err() {
        let mut vm = vm_in_function_frame(wrapper_frame(FrameReturnWrapper::Result));

        vm.propagate_none_early_return()
            .expect("None? should early-return");
        assert!(
            vm.call_stack.is_empty(),
            "early return should pop the function frame"
        );

        let (bits, kind) = vm.pop_kinded().expect("early-return value on stack");
        assert_eq!(kind, NativeKind::Ptr(HeapKind::TypedObject));
        let slot = KindedSlot::new(ValueSlot::from_raw(bits), kind);
        let result = read_result(&vm, &slot)
            .expect("Result carrier should decode")
            .expect("None? in Result frame should produce Result carrier");
        assert!(
            !result.is_ok(),
            "None? in Result-returning frame should lift to Err"
        );
        let payload = result
            .clone_payload()
            .expect("Err payload should be cloneable AnyError");
        assert_eq!(payload.kind(), NativeKind::Ptr(HeapKind::TypedObject));
        drop(payload);
        drop(slot);
    }

    #[test]
    fn none_in_option_frame_with_typed_object_abi_propagates_none() {
        let mut vm = vm_in_function_frame(wrapper_frame(FrameReturnWrapper::Option));

        vm.propagate_none_early_return()
            .expect("None? should early-return");

        let (bits, kind) = vm.pop_kinded().expect("early-return value on stack");
        assert_eq!(bits, VirtualMachine::NONE_BITS);
        assert_eq!(kind, NativeKind::Null);
    }
}

// =========================================================================
// W14-variant-codegen unit tests — Result/Option op_* dispatch
//
// These exercise the kinded recovery path (read_result / read_option)
// directly, validating the smoke target:
// `let r = Ok(42); if r.is_ok() { print(r.unwrap_ok()) }` outputs `42`.
// At the storage tier this is a `__Result` / `__Option` TypedObject with
// a `variant` discriminator and a payload slot. The op_* handler bodies
// exercise the same path but go through pop_kinded / push_kinded for
// stack transit.
// =========================================================================

#[cfg(test)]
mod variant_codegen_tests {
    use super::*;
    use crate::executor::VMConfig;

    /// Smoke target: `Ok(42)` → is_ok() → unwrap_ok() yields 42.
    #[test]
    fn smoke_ok_int_is_ok_then_unwrap() {
        let vm = VirtualMachine::new(VMConfig::default());
        let ok_carrier =
            result_option_carrier::build_ok(&vm.builtin_schemas, KindedSlot::from_int(42));

        // op_is_ok: classify Ok carrier → true.
        let is_ok_value = match read_result(&vm, &ok_carrier).unwrap() {
            Some(rd) => rd.is_ok(),
            None => panic!("Result kind read failed"),
        };
        assert!(is_ok_value);

        // op_unwrap_ok: extract the inner i64 via KindedSlot::Clone
        // (retain-on-extract per WB2.4).
        let unwrapped = match read_result(&vm, &ok_carrier).unwrap() {
            Some(rd) if rd.is_ok() => rd.clone_payload().unwrap(),
            _ => panic!("Ok unwrap failed"),
        };
        assert_eq!(unwrapped.as_i64(), Some(42));

        // Wrapper carrier is still alive. The cloned `unwrapped` owns its
        // own share; Int64 is inline-scalar so its drop is a no-op.
        drop(ok_carrier);
        drop(unwrapped);
    }

    /// `Err("oops")` → is_err() yields true; unwrap_err yields "oops".
    #[test]
    fn err_string_is_err_then_unwrap() {
        let vm = VirtualMachine::new(VMConfig::default());
        let err_carrier = result_option_carrier::build_err(
            &vm.builtin_schemas,
            KindedSlot::from_string_arc(Arc::new("oops".to_string())),
        );

        let is_ok_value = match read_result(&vm, &err_carrier).unwrap() {
            Some(rd) => rd.is_ok(),
            None => panic!("Result kind read failed"),
        };
        assert!(!is_ok_value);

        let unwrapped = match read_result(&vm, &err_carrier).unwrap() {
            Some(rd) if !rd.is_ok() => rd.clone_payload().unwrap(),
            _ => panic!("Err unwrap failed"),
        };
        assert_eq!(unwrapped.as_str(), Some("oops"));

        drop(err_carrier);
        drop(unwrapped);
    }

    /// `Some(42)` → unwrap_option yields 42.
    #[test]
    fn some_int_unwrap_option() {
        let vm = VirtualMachine::new(VMConfig::default());
        let some_carrier =
            result_option_carrier::build_some(&vm.builtin_schemas, KindedSlot::from_int(42));

        let unwrapped = match read_option(&vm, &some_carrier).unwrap() {
            Some(od) if od.is_some() => od.clone_payload().unwrap(),
            _ => panic!("Some unwrap failed"),
        };
        assert_eq!(unwrapped.as_i64(), Some(42));

        drop(some_carrier);
        drop(unwrapped);
    }

    /// `None` → is_some is false.
    #[test]
    fn none_carrier_is_some_false() {
        let vm = VirtualMachine::new(VMConfig::default());
        let none_carrier = result_option_carrier::build_none(&vm.builtin_schemas);

        let is_some_value = match read_option(&vm, &none_carrier).unwrap() {
            Some(od) => od.is_some(),
            None => panic!("Option kind read failed"),
        };
        assert!(!is_some_value);

        drop(none_carrier);
    }

    /// `op_throw` fast-path: an Err carrier flows through the
    /// exception machinery. Verifies the dispatch tables (clone/drop)
    /// don't double-free or leak the inner share.
    #[test]
    fn err_carrier_drop_is_balanced() {
        // Build an Err with an Arc<String> payload to make the share
        // count observable.
        let inner_arc = Arc::new("oops".to_string());
        let strong_before = Arc::strong_count(&inner_arc);
        let payload = KindedSlot::from_string_arc(Arc::clone(&inner_arc));
        // payload now owns one share; outer Arc<String> has 2.
        assert_eq!(Arc::strong_count(&inner_arc), strong_before + 1);

        let vm = VirtualMachine::new(VMConfig::default());
        let err_carrier = result_option_carrier::build_err(&vm.builtin_schemas, payload);
        // Wrapper retained the inner share; same count as after payload.
        assert_eq!(Arc::strong_count(&inner_arc), strong_before + 1);

        // Drop the outer wrapper — share count returns to baseline.
        drop(err_carrier);
        assert_eq!(Arc::strong_count(&inner_arc), strong_before);
    }

    /// Smoke: `op_type_check` against `Result<int, string>` matches
    /// an `Ok(_)` carrier.
    #[test]
    fn type_check_result_matches_ok_carrier() {
        use shape_ast::ast::TypeAnnotation;
        use shape_ast::ast::TypePath;
        let vm = VirtualMachine::new(VMConfig::default());
        let carrier =
            result_option_carrier::build_ok(&vm.builtin_schemas, KindedSlot::from_int(42));
        let annotation = TypeAnnotation::Generic {
            name: TypePath::simple("Result"),
            args: vec![
                TypeAnnotation::Basic("int".to_string()),
                TypeAnnotation::Basic("string".to_string()),
            ],
        };
        assert!(type_check_kinded(&vm.builtin_schemas, &annotation, &carrier).unwrap());
        drop(carrier);
    }

    /// `op_type_check` against `Option<int>` matches a `Some(_)`
    /// carrier and a null sentinel both.
    #[test]
    fn type_check_option_matches_some_and_null() {
        use shape_ast::ast::TypeAnnotation;
        use shape_ast::ast::TypePath;
        let vm = VirtualMachine::new(VMConfig::default());
        let some_carrier =
            result_option_carrier::build_some(&vm.builtin_schemas, KindedSlot::from_int(7));
        let annotation = TypeAnnotation::Generic {
            name: TypePath::simple("Option"),
            args: vec![TypeAnnotation::Basic("int".to_string())],
        };
        assert!(type_check_kinded(&vm.builtin_schemas, &annotation, &some_carrier).unwrap());

        // Null sentinel (zero-bits Bool slot) matches Option per
        // null-coding fallback.
        let null_carrier = KindedSlot::none();
        assert!(type_check_kinded(&vm.builtin_schemas, &annotation, &null_carrier).unwrap());

        drop(some_carrier);
        drop(null_carrier);
    }

    /// Variant-bypass: passing a non-Result kind to read_result returns
    /// None (lets the caller decide whether that's an error).
    #[test]
    fn read_result_on_non_result_returns_none() {
        let vm = VirtualMachine::new(VMConfig::default());
        let int_carrier = KindedSlot::from_int(42);
        let result = read_result(&vm, &int_carrier).unwrap();
        assert!(result.is_none());
        drop(int_carrier);
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

#[cfg(test)]
mod ws3_uncaught_and_array_typecheck_tests {
    //! WS-3 F4 + the `type_check_kinded` `"array"` Basic-name arm.
    //!
    //! F4: the `handle_exception` no-handler branch used to interpolate
    //! the internal-jargon `PHASE_2C_EXCEPTION_OBJECT_SURFACE` literal
    //! (ADR section numbers, deleted-symbol names) as a user-facing
    //! error. It now surfaces a clean `Uncaught error: <message>`.
    //!
    //! `type_check_kinded`: `emit_destructure_type_check` passes the bare
    //! name `"array"`, which no Basic arm matched — so `let [a,b,c] = xs`
    //! failed the runtime `TypeCheck`, fired `Throw`, and dumped the F4
    //! jargon. The new `"array"` arm makes plain array-destructure work.

    use crate::test_utils::eval_result;

    /// A non-exhaustive `match` with no enclosing `try` unwinds to the
    /// `handle_exception` no-handler branch. The surfaced error must be
    /// the clean message, NOT the internal jargon.
    #[test]
    fn ws3_f4_uncaught_match_failure_is_clean_no_jargon() {
        let err = eval_result(
            r#"
            fn run() -> int {
                let x = 5
                match x { 1 => 10, 2 => 20 }
            }
            run()
            "#,
        )
        .expect_err("non-exhaustive match must surface a runtime error");
        let msg = format!("{}", err);
        assert!(
            msg.contains("Uncaught error:"),
            "expected a clean `Uncaught error:` message, got: {}",
            msg
        );
        // The jargon literal's distinctive fragments must NOT appear.
        assert!(
            !msg.contains("phase-2c")
                && !msg.contains("ADR-006")
                && !msg.contains("ValueWord")
                && !msg.contains("D-raw-helpers"),
            "uncaught-exception message must not dump internal jargon: {}",
            msg
        );
    }

    /// WS-3: a plain (non-rest) array destructure `let [a,b,c] = xs` must
    /// run cleanly. Before the `type_check_kinded` `"array"` arm, the
    /// runtime `TypeCheck` against the bare `"array"` name returned
    /// `false`, fired `Throw`, and dumped the F4 jargon.
    #[test]
    fn ws3_non_rest_array_destructure_runs_cleanly() {
        let result = eval_result(
            r#"
            fn run() -> int {
                let xs = [10, 20, 30]
                let [a, b, c] = xs
                b
            }
            run()
            "#,
        );
        let slot = result.expect("plain array destructure must not raise");
        assert_eq!(slot.as_i64(), Some(20));
    }
}
