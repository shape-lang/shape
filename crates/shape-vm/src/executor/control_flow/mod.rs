//! Control flow operations for the VM executor
//!
//! Handles: Jump, JumpIfFalse, JumpIfTrue, Call, CallValue, CallForeign, Return, ReturnValue

pub mod foreign_marshal;
pub mod jit_abi;
pub mod native_abi;

use crate::executor::builtins::kind_coerce::int_operand;
use crate::{
    bytecode::{Instruction, OpCode, Operand},
    executor::{ForeignFunctionHandle, VirtualMachine},
};
use shape_value::{HeapKind, KindedSlot, NativeKind, VMError, ValueSlot};

/// ADR-006 §2.7.4 / §2.7.7 surface marker for the closure-call /
/// extern-FFI / JIT-dispatch paths in this module that still depended on
/// the deleted ValueWord / tag_bits / as_heap_ref / vmarray_from_vec
/// surfaces. The B11-control-flow-heap sub-cluster migrates the
/// arg-slicing / typed return / jump-condition paths to the kinded API
/// (ADR-006 §2.7.7 / Q9); the closure-construction + indirect-callee
/// dispatch + extern-C / foreign-runtime invoke paths cross the
/// `KindedSlot` / `ValueWord`-extension-contract boundary (§2.7.5) and
/// are deferred to phase-2c per §2.7.4.
const PHASE_2C_CALL_REBUILD_SURFACE: &str =
    "phase-2c — closure / call / extern-FFI rebuild (ADR-006 §2.7.4 / §2.7.5)";

/// Glob match for `ffi_libraries` / `ffi_symbols` scope patterns
/// (ffi-rebuild §4.8.2). Mirrors the `allowed_paths` prefix-glob style used by
/// `module_exports::check_fs_permission`: a pattern ending in `*` / `**`
/// matches any value with that prefix; otherwise the match is exact. A bare
/// `*` / `**` matches anything.
fn ffi_glob_match(pattern: &str, value: &str) -> bool {
    if let Some(prefix) = pattern
        .strip_suffix("**")
        .or_else(|| pattern.strip_suffix('*'))
    {
        value.starts_with(prefix)
    } else {
        pattern == value
    }
}

/// Exhaustive HeapKind sink for pointer-truthiness checks.
#[inline]
fn heap_ptr_is_truthy(bits: u64, heap_kind: HeapKind) -> bool {
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
        | HeapKind::MatrixSlice => bits != 0,
    }
}

/// ADR-006 §2.7.7: bool truthiness from raw bits + kind. Mirrors the
/// helper in `executor/logical/mod.rs` (kept module-local; no cross-
/// territory dependency).
#[inline]
fn kinded_truthy(bits: u64, kind: NativeKind) -> bool {
    match kind {
        NativeKind::Bool => bits != 0,
        NativeKind::Float64 => f64::from_bits(bits) != 0.0,
        NativeKind::Int8
        | NativeKind::Int16
        | NativeKind::Int32
        | NativeKind::Int64
        | NativeKind::IntSize
        | NativeKind::UInt8
        | NativeKind::UInt16
        | NativeKind::UInt32
        | NativeKind::UInt64
        | NativeKind::UIntSize => bits != 0,
        NativeKind::NullableFloat64
        | NativeKind::NullableInt8
        | NativeKind::NullableInt16
        | NativeKind::NullableInt32
        | NativeKind::NullableInt64
        | NativeKind::NullableIntSize
        | NativeKind::NullableUInt8
        | NativeKind::NullableUInt16
        | NativeKind::NullableUInt32
        | NativeKind::NullableUInt64
        | NativeKind::NullableUIntSize => bits != 0,
        // Round 19 S1.5 W12-nativekind-scalar-additions (2026-05-14):
        // F32 truthy iff `!= 0.0` (matches Float64); Char truthy iff
        // codepoint bits non-zero (NUL is the only falsy char).
        NativeKind::Float32 => f32::from_bits(bits as u32) != 0.0,
        NativeKind::Char => bits != 0,
        // Wave 2 Agent B W12-StringV2-DecimalV2-NativeKind-additions
        // (2026-05-14): truthy iff `bits != 0` — the v2-raw carrier ptr is
        // non-null when live (same shape as the String / Ptr(HeapKind::*)
        // heap-arm truthy rule below).
        NativeKind::StringV2 | NativeKind::DecimalV2 => bits != 0,
        NativeKind::String => bits != 0,
        NativeKind::Ptr(heap_kind) => heap_ptr_is_truthy(bits, heap_kind),
        // R5b-2-bool-null-sentinel-cluster (ADR-006 §2.7 + §2.7.7/Q9,
        // 2026-05-19): `NativeKind::Null` is the absence-of-value
        // sentinel; falsy by definition.
        NativeKind::Null => false,
    }
}

impl VirtualMachine {
    #[inline(always)]
    pub(in crate::executor) fn exec_control_flow(
        &mut self,
        instruction: &Instruction,
        ctx: Option<&mut shape_runtime::context::ExecutionContext>,
    ) -> Result<(), VMError> {
        use OpCode::*;
        match instruction.opcode {
            Jump => self.op_jump(instruction)?,
            JumpIfFalse => self.op_jump_if_false(instruction)?,
            JumpIfFalseTrusted => self.op_jump_if_false_trusted(instruction)?,
            JumpIfTrue => self.op_jump_if_true(instruction)?,
            Call => self.op_call(instruction)?,
            // ADR-006 §2.7.11 / Q12: value-call dispatch shells route
            // through `call_value_immediate_nb`, which drives the callee
            // synchronously via `execute_until_call_depth(ctx)`. The
            // outer dispatch loop owns the `ExecutionContext`; thread it
            // through so the sub-loop can resolve any nested foreign /
            // remote / suspension surfaces the callee body raises.
            CallValue => self.op_call_value(ctx)?,
            CallClosure => self.op_call_closure(instruction, ctx)?,
            CallFunctionIndirect => self.op_call_function_indirect(instruction, ctx)?,
            CallForeign => self.op_call_foreign(instruction)?,
            Return => self.op_return()?,
            ReturnValue => self.op_return_value()?,
            ReturnValueI64 => self.op_return_value_i64()?,
            ReturnValueU64 => self.op_return_value_u64()?,
            ReturnValueF64 => self.op_return_value_f64()?,
            ReturnValueI32 => self.op_return_value_i32()?,
            ReturnValueU32 => self.op_return_value_u32()?,
            ReturnValueI16 => self.op_return_value_i16()?,
            ReturnValueU16 => self.op_return_value_u16()?,
            ReturnValueI8 => self.op_return_value_i8()?,
            ReturnValueU8 => self.op_return_value_u8()?,
            ReturnValueBool => self.op_return_value_bool()?,
            ReturnValuePtr => self.op_return_value_ptr()?,
            _ => unreachable!(
                "exec_control_flow called with non-control-flow opcode: {:?}",
                instruction.opcode
            ),
        }
        Ok(())
    }

    // Jump operations

    pub(in crate::executor) fn op_jump(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        if let Some(Operand::Offset(offset)) = instruction.operand {
            // OSR: backward jumps (negative offset) are loop back-edges.
            // Record the iteration and attempt OSR entry if JIT code is ready.
            #[cfg(feature = "jit")]
            if offset < 0 {
                let target_ip = (self.ip as i32 + offset) as usize;
                if let Some(func_id) = self.current_function_id() {
                    self.check_osr_back_edge(func_id, target_ip);
                    // Note: we do NOT attempt try_osr_entry here because the
                    // canonical OSR entry point is at LoopStart, not at an
                    // arbitrary backward jump. The back-edge counter is
                    // incremented here to catch loops that use Jump-backward
                    // without a LoopStart instruction.
                }
            }
            self.ip = (self.ip as i32 + offset) as usize;
        } else {
            return Err(VMError::InvalidOperand);
        }
        Ok(())
    }

    pub(in crate::executor) fn op_jump_if_false(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        if let Some(Operand::Offset(offset)) = instruction.operand {
            // ADR-006 §2.7.7 / playbook §2: jump condition is always
            // post-proof Bool kind. The compiler emits typed comparison
            // / `Not` opcodes that push `NativeKind::Bool`. Heterogeneous
            // truthiness inputs go through `op_jump_if_false_trusted`'s
            // bool-only path or are pre-coerced by an explicit bool op.
            let (bits, kind) = self.pop_kinded()?;
            let condition = kinded_truthy(bits, kind);
            // Release any heap share carried by the popped slot (kinded
            // truthy reads bits + kind without consuming an Arc share).
            crate::executor::vm_impl::stack::drop_with_kind(bits, kind);
            if !condition {
                self.ip = (self.ip as i32 + offset) as usize;
            }
        } else {
            return Err(VMError::InvalidOperand);
        }
        Ok(())
    }

    /// JumpIfFalse — trusted variant.
    ///
    /// The compiler has proved the condition is a boolean value.
    /// Producers (typed comparison, `Not`) push `NativeKind::Bool` slots
    /// (0u64 / 1u64 in the data track, `NativeKind::Bool` in the kind
    /// track) — read the bits directly with `pop_kinded`.
    #[inline(always)]
    pub(in crate::executor) fn op_jump_if_false_trusted(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        if let Some(Operand::Offset(offset)) = instruction.operand {
            let (bits, _kind) = self.pop_kinded()?;
            let cond = bits != 0;
            if !cond {
                self.ip = (self.ip as i32 + offset) as usize;
            }
        } else {
            return Err(VMError::InvalidOperand);
        }
        Ok(())
    }

    pub(in crate::executor) fn op_jump_if_true(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        if let Some(Operand::Offset(offset)) = instruction.operand {
            // See `op_jump_if_false` for the polymorphic-truthiness
            // contract. Same kinded-truthy + drop pattern.
            let (bits, kind) = self.pop_kinded()?;
            let condition = kinded_truthy(bits, kind);
            crate::executor::vm_impl::stack::drop_with_kind(bits, kind);
            if condition {
                self.ip = (self.ip as i32 + offset) as usize;
            }
        } else {
            return Err(VMError::InvalidOperand);
        }
        Ok(())
    }

    // Call operations

    pub(in crate::executor) fn op_call(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        // ADR-006 §2.7.7 / playbook §2: arg-count slot is post-proof
        // integer kind (the compiler emits a typed integer push for
        // arg-count). Kind dispatch lives at the body site via
        // `int_operand` (§2.7.6 heterogeneous-kind body pattern).
        let (arg_count_bits, arg_count_kind) = self.pop_kinded()?;
        let arg_count_slot = KindedSlot::new(ValueSlot::from_raw(arg_count_bits), arg_count_kind);
        let arg_count = int_operand(&arg_count_slot)
            .map_err(|_| VMError::RuntimeError("Expected integer for arg count".to_string()))?
            as usize;
        crate::executor::vm_impl::stack::drop_with_kind(arg_count_bits, arg_count_kind);

        if let Some(Operand::Function(func_id)) = instruction.operand {
            // ADR-006 §2.7.4 / §2.7.5 SURFACE: the JIT dispatch fast path
            // marshalled VM stack slots through `jit_abi::marshal_arg_to_jit`
            // which takes `&ValueWord`; the JIT context buffer ABI is raw
            // bits on the JIT side per §2.7.5 cross-crate policy. The
            // conversion shape — `stack_slice_raw` → `&[ValueWord]` → JIT
            // context — plus the deopt-recovery and `unmarshal_jit_result`
            // path cross the VM↔JIT FFI boundary and are out of B11
            // territory. The rebuild is tracked under phase-2c per §2.7.4
            // (cross-crate ABI consumer-side migration). Surface so the
            // VM↔JIT bridge is rebuilt under the kinded-stack API rather
            // than papered over.
            #[cfg(feature = "jit")]
            {
                let jit_fn_present = self.jit_dispatch_table.contains_key(&func_id.0)
                    || self
                        .tier_manager
                        .as_ref()
                        .and_then(|mgr| mgr.get_native_code(func_id.0))
                        .is_some();
                if jit_fn_present {
                    return Err(VMError::NotImplemented(format!(
                        "op_call JIT dispatch (func_id={}): {}",
                        func_id.0, PHASE_2C_CALL_REBUILD_SURFACE
                    )));
                }
            }

            // ---- Tier promotion ----
            // Record the call and check if promotion threshold is crossed.
            // This is a no-op when tier_manager is None.
            if let Some(ref mut tier_mgr) = self.tier_manager {
                let fv = self
                    .feedback_vectors
                    .get(func_id.0 as usize)
                    .and_then(|o| o.as_ref());
                let _ = tier_mgr.record_call(func_id.0, fv);
            }

            // Record interpreter fallback in metrics.
            if let Some(ref mut metrics) = self.metrics {
                metrics.record_interpreter_call();
            }

            // Record call target for IC profiling.
            {
                let ip = self.ip;
                if let Some(fv) = self.current_feedback_vector() {
                    fv.record_call(ip, func_id.0);
                }
            }

            // Args are already on the stack in left-to-right order.
            // Read them directly into locals — no Vec allocation needed.
            self.call_function_from_stack(func_id.0, arg_count)?;
        } else {
            return Err(VMError::InvalidOperand);
        }
        Ok(())
    }

    /// Closure spec Phase F — direct dispatch on a statically-typed closure.
    ///
    /// `CallClosure(Count(arity))` is emitted at call sites where the
    /// compiler has proven the callee's `ClosureTypeId` (typically because a
    /// closure literal was bound to a `let` and then called through that
    /// binding, or after Phase C-style specialization narrowed the closure
    /// type). The VM behaviourally equals `CallValue` with `arg_count` read
    /// from the operand instead of popped from the stack. The JIT uses the
    /// statically-known type id to emit a direct `call` with typed capture
    /// loads — see `docs/v2-closure-specialization.md` §1.3.
    ///
    /// Stack layout (both before and after mirrors `CallValue`):
    /// - Before: `[..., callee, arg0, arg1, ..., arg_{N-1}]`
    /// - After:  `[..., result]`
    pub(in crate::executor) fn op_call_closure(
        &mut self,
        instruction: &Instruction,
        ctx: Option<&mut shape_runtime::context::ExecutionContext>,
    ) -> Result<(), VMError> {
        let arity = match instruction.operand {
            Some(Operand::Count(n)) => n as usize,
            _ => return Err(VMError::InvalidOperand),
        };
        // Closure spec Phase F: arity from the opcode operand; otherwise
        // the dispatch tree is the same as `op_call_value`. The kinded
        // value-call dispatch in `call_value_immediate_nb` (ADR-006
        // §2.7.11 / Q12) already classifies the callee on `kind`
        // (`Ptr(HeapKind::Closure)` vs `UInt64`), so the typed-vs-
        // polymorphic distinction collapses at the runtime tier — the
        // JIT keeps the type discrimination at codegen time per
        // `docs/v2-closure-specialization.md` §1.3.
        self.dispatch_call_value_immediate(arity, ctx)
    }

    /// Closure spec Phase F — polymorphic dispatch through `Function<A, R>`.
    ///
    /// `CallFunctionIndirect(Count(arity))` is emitted at call sites where
    /// the callee's concrete `ClosureTypeId` is not known but the signature
    /// is (i.e. the callee is typed as `Function<A, R>`). The JIT lowers
    /// this to a `call_indirect` with the `FunctionTypeId`'s Cranelift
    /// signature; the VM dispatches through the same runtime path as
    /// `CallValue`. The opcode distinction exists so the JIT can avoid the
    /// full tag-dispatch cost when it knows the callee is a callable value
    /// (not an arbitrary ValueWord).
    pub(in crate::executor) fn op_call_function_indirect(
        &mut self,
        instruction: &Instruction,
        ctx: Option<&mut shape_runtime::context::ExecutionContext>,
    ) -> Result<(), VMError> {
        let arity = match instruction.operand {
            Some(Operand::Count(n)) => n as usize,
            _ => return Err(VMError::InvalidOperand),
        };
        // Closure spec Phase F: same dispatch tree as `op_call_closure`;
        // distinction only matters to the JIT (`call_indirect` signature
        // selection from `FunctionTypeId`). At runtime the ADR-006
        // §2.7.11 / Q12 value-call dispatch shell handles both the
        // `Ptr(HeapKind::Closure)` and `UInt64` callee cases uniformly.
        self.dispatch_call_value_immediate(arity, ctx)
    }

    /// Shared value-call dispatch helper for `CallValue`, `CallClosure`,
    /// and `CallFunctionIndirect` (ADR-006 §2.7.11 / Q12, W7-op-call-value
    /// Round 3).
    ///
    /// `arg_count` is supplied by the caller — popped from the stack for
    /// `CallValue` (legacy arg-count-on-stack form), or read from the
    /// opcode operand for `CallClosure` / `CallFunctionIndirect` (typed
    /// dispatch forms). From here the body shape is identical and mirrors
    /// the §2.7.10 op_call_method dispatch-shell precedent in
    /// `executor/objects/mod.rs:267-296`:
    ///
    /// 1. Pop `arg_count` slots via `pop_kinded()`, wrapping each
    ///    `(bits, kind)` pair into a transient `KindedSlot` carrier.
    ///    `pop_kinded` transfers one strong-count share into the
    ///    returned bits + kind (WB2.4 retain-on-read), and the
    ///    `KindedSlot::new` carrier takes ownership of that share.
    ///    Args are popped in reverse order; reverse the vec back to
    ///    push order so `args[0]` is the first arg.
    /// 2. Pop the callee the same way; build the callee `KindedSlot`.
    /// 3. Dispatch through `call_value_immediate_nb(&callee, &args[..],
    ///    ctx)` (W7-cv-static `06cdfce`). The borrow contract leaves the
    ///    shares with the carriers in this stack frame — the dispatch
    ///    body never moves them out.
    /// 4. The dispatch returns a `KindedSlot` carrying the callee's
    ///    return value with one strong-count share. Push raw + kind
    ///    onto the kinded stack via `push_kinded`, then `mem::forget`
    ///    the result carrier so the share transfers to the stack
    ///    cleanly (no double-drop).
    /// 5. `args` and `callee` carriers drop at end of scope; their
    ///    `Drop` impls dispatch on `kind` and release the shares via
    ///    `drop_with_kind` (Round 2.5 + 2.5b wired
    ///    `HeapKind::Closure → Arc::decrement_strong_count` for the
    ///    carrier-drop path; `HeapKind::Future` is no-op for inline
    ///    future-id payloads).
    ///
    /// Forbidden (W7 playbook §6 #12-18): `Vec<KindedSlot>` by-move
    /// into `call_value_immediate_nb`; `&[(u64, NativeKind)]` pair-slice
    /// runtime-tier ABI; tag-bits decode on callee bits;
    /// Bool-default fallback; defection-attractor framing per CLAUDE.md
    /// "Renames to refuse on sight" (the value-call ABI family).
    ///
    /// Closure spec Phase G §5.4 (deferred): pre-§2.7.11 dispatch records
    /// the resolved target `function_id` into the current function's
    /// feedback vector for JIT Tier-2 speculative direct-call guards.
    /// The feedback recording is a downstream IC concern; surfacing the
    /// `function_id` from the dispatch body cleanly requires either
    /// peeking the callee before the dispatch shell takes ownership, or
    /// pushing the recording into `call_value_immediate_nb` itself. The
    /// JIT-dispatch path in `op_call:222-236` stays SURFACE until W10 —
    /// the IC recording lands with that wave.
    fn dispatch_call_value_immediate(
        &mut self,
        arg_count: usize,
        ctx: Option<&mut shape_runtime::context::ExecutionContext>,
    ) -> Result<(), VMError> {
        let mut args: Vec<KindedSlot> = Vec::with_capacity(arg_count);
        for _ in 0..arg_count {
            // ADR-006 §2.7.7 WB2.4: pop_kinded transfers one share
            // (heap-bearing kinds) from the stack into the returned
            // (bits, kind) pair. KindedSlot::new takes ownership of
            // that share; its Drop releases it via drop_with_kind on
            // scope exit.
            let (bits, kind) = self.pop_kinded()?;
            args.push(KindedSlot::new(ValueSlot::from_raw(bits), kind));
        }
        // Pop order is reverse of push order; flip to restore positional
        // alignment so args[0] is the first call argument.
        args.reverse();

        let (callee_bits, callee_kind) = self.pop_kinded()?;
        let callee = KindedSlot::new(ValueSlot::from_raw(callee_bits), callee_kind);

        // The dispatch body borrows the carriers (`&KindedSlot`,
        // `&[KindedSlot]`) — share ownership stays here. The returned
        // `KindedSlot` is a fresh carrier with one share transferred
        // from the callee body's `ReturnValue` opcode.
        let result = self.call_value_immediate_nb(&callee, &args, ctx)?;

        // Transfer the result share back to the kinded stack. The
        // share is now owned by the stack slot; mem::forget on the
        // carrier prevents the carrier's Drop from releasing it
        // (which would be a double-drop).
        self.push_kinded(result.raw(), result.kind())?;
        std::mem::forget(result);

        // `args` and `callee` carriers drop here, releasing each share
        // via `KindedSlot::drop` (kind-dispatched
        // Arc::decrement_strong_count per ADR-006 §2.7.6 / Q8 — no
        // bare vw_drop, no Bool-default fallback).
        Ok(())
    }

    pub(in crate::executor) fn op_call_value(
        &mut self,
        ctx: Option<&mut shape_runtime::context::ExecutionContext>,
    ) -> Result<(), VMError> {
        // ADR-006 §2.7.7 / playbook §2: arg-count is post-proof integer
        // kind; same body-site `int_operand` dispatch as `op_call`.
        let (arg_count_bits, arg_count_kind) = self.pop_kinded()?;
        let arg_count_slot = KindedSlot::new(ValueSlot::from_raw(arg_count_bits), arg_count_kind);
        let arg_count = int_operand(&arg_count_slot)
            .map_err(|_| VMError::RuntimeError("Expected integer for arg count".to_string()))?
            as usize;
        crate::executor::vm_impl::stack::drop_with_kind(arg_count_bits, arg_count_kind);

        self.dispatch_call_value_immediate(arg_count, ctx)
    }

    pub(in crate::executor) fn op_make_closure(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        // ADR-006 §2.7.8 / §2.7.11 / Q10 / Q12 — closure-producer side
        // (W12-op-make-closure rebuild). Symmetric mirror of W7's consumer
        // `call_closure_with_nb_args_keepalive` (`call_convention.rs`):
        // the consumer reads each capture via
        // `OwnedClosureBlock::read_capture_kinded(idx) -> (bits, kind)`
        // where the kind comes from `layout.capture_native_kinds[idx]` set
        // here at construction. No fabrication, no Bool-default fallback
        // (§2.7.8 #4 forbidden) — the kind track is single-sourced from
        // the layout descriptor (which the compiler set at closure-literal
        // lowering time).
        //
        // Stack contract: captures live on the kinded stack at
        // `[sp - capture_count .. sp]` from the producing `LoadLocal` /
        // `AllocSharedLocal` / `LoadModuleBinding` opcodes (per
        // `compile_expr_closure` in `compiler/expressions/closures.rs`).
        // Each `pop_kinded` transfers one strong-count share out of the
        // slot (WB2.4 retain-on-read installed it via `clone_with_kind`),
        // and the share moves into the closure block's capture slot —
        // no extra retain/release pair at the producer site.
        //
        // Slot tier (§2.7.11 / Q12): the produced closure value is
        // `Arc::into_raw(Arc::new(HeapValue::ClosureRaw(OwnedClosureBlock)))`
        // pushed with `NativeKind::Ptr(HeapKind::Closure)`. The W7
        // Round-2.5 close `5fa4b19` wired
        // `clone_with_kind` / `drop_with_kind` for `HeapKind::Closure` to
        // `Arc::increment/decrement_strong_count(bits as *const HeapValue)`
        // — this push must produce that exact shape so the consumer's
        // `callee.slot.as_heap_value()` -> `HeapValue::ClosureRaw(block)`
        // pattern in `call_value_immediate_nb` succeeds.
        //
        // Closure spec H5: `MakeClosure` accepts two operand shapes:
        //   - `Operand::Function(fid)`            — non-escaping closure.
        //   - `Operand::ClosureAlloc { fid, .. }` — compiler-tagged with
        //     escape status (the VM path is identical; the JIT's MIR
        //     lowering reads `escapes` to pick stack vs. heap codegen).
        use shape_value::heap_value::HeapValue;
        use shape_value::v2::closure_layout::CaptureKind;
        use shape_value::v2::closure_raw::{
            OwnedClosureBlock, alloc_owned_mutable_bool, alloc_owned_mutable_f64,
            alloc_owned_mutable_i8, alloc_owned_mutable_i16, alloc_owned_mutable_i32,
            alloc_owned_mutable_i64, alloc_owned_mutable_ptr, alloc_owned_mutable_u8,
            alloc_owned_mutable_u16, alloc_owned_mutable_u32, alloc_owned_mutable_u64,
            alloc_typed_closure, write_capture_raw_u64,
        };
        use shape_value::v2::struct_layout::FieldKind;
        use std::sync::Arc;

        let func_id = match instruction.operand {
            Some(Operand::Function(fid)) => fid,
            Some(Operand::ClosureAlloc { fid, .. }) => fid,
            _ => return Err(VMError::InvalidOperand),
        };

        // Source the closure layout from the program's per-function
        // side-table. A missing entry is a compile/link-time bug — every
        // `MakeClosure` emission must register a layout (Track A.5
        // retired the legacy fallback path).
        let layout: Arc<shape_value::v2::closure_layout::ClosureLayout> = self
            .program
            .closure_function_layouts
            .get(func_id.index())
            .and_then(|opt| opt.clone())
            .ok_or_else(|| {
                VMError::RuntimeError(format!(
                    "op_make_closure: no ClosureLayout registered for function {} \
                     (compile/link-time bug — every MakeClosure emission must \
                     register a layout)",
                    func_id.0
                ))
            })?;

        let capture_count = layout.capture_count();

        // Pop captures in reverse push order. Each `pop_kinded` transfers
        // one strong-count share (heap-bearing kinds) out of the stack
        // slot via the WB2.4 lockstep contract. We collect into a
        // `Vec<(bits, kind)>` and reverse so `popped[0]` aligns with
        // capture slot 0, etc.
        let mut popped: Vec<(u64, NativeKind)> = Vec::with_capacity(capture_count);
        for _ in 0..capture_count {
            popped.push(self.pop_kinded()?);
        }
        popped.reverse();

        // Allocate a freshly-zeroed `TypedClosureHeader` block (refcount=1)
        // and write each capture into its typed offset based on the
        // layout's per-capture `CaptureKind`. The three branches mirror
        // `release_typed_closure`'s drop dispatch (closure_raw.rs:376) so
        // every share installed here is released exactly once on the
        // block's last refcount drop.
        //
        // SAFETY: `alloc_typed_closure` returns a non-null pointer to a
        // block sized for `layout.total_heap_size()` with the
        // `TypedClosureHeader` prefix initialised. `write_capture_raw_u64`
        // writes the 8-byte slot at `layout.heap_capture_offset(i)` —
        // in-bounds for every `i < capture_count`.
        let owned = unsafe {
            let ptr = alloc_typed_closure(func_id.0, 0, &layout);
            for (i, (bits, kind)) in popped.iter().enumerate() {
                match layout.capture_storage_kind(i) {
                    CaptureKind::Immutable => {
                        // Named-function-value carrier reconciliation
                        // (R1 named-fn-as-value capture fix).
                        //
                        // A *named* function value is produced as a bare
                        // function-id with runtime kind
                        // `NativeKind::UInt64` (see
                        // `PushConst(Constant::Function)` in
                        // `stack_ops/mod.rs` — `(id as u64, UInt64)`),
                        // whereas a *closure* value is produced as
                        // `Arc::into_raw(Arc<HeapValue::ClosureRaw>)` with
                        // kind `Ptr(HeapKind::Closure)`.
                        //
                        // A callable capture is stamped
                        // `Ptr(HeapKind::Closure)` in the layout
                        // (`resolve_capture_concrete_type` →
                        // `ConcreteType::Function` →
                        // `native_kind_from_concrete_type`) whenever the
                        // compiler classifies the capture as a function
                        // value (called or passed onward in the body).
                        //
                        // Storing the bare fn-id verbatim into the
                        // `Ptr(HeapKind::Closure)` slot is a carrier
                        // mismatch: `release_typed_closure` (and any later
                        // value-call via `as_heap_value()`) would treat the
                        // fn-id integer as an `Arc<HeapValue>` pointer and
                        // dereference garbage → SIGSEGV. The popped runtime
                        // `kind` is the single source of truth here
                        // (ADR-006 §2.7.7 — no fabrication): when the layout
                        // expects a closure carrier but the value arrived as
                        // a `UInt64` fn-id, materialize a real zero-capture
                        // closure carrier for that fn-id so the carrier
                        // matches the layout's `Ptr(HeapKind::Closure)`
                        // release discipline, and every downstream path
                        // (capture read, value-call, refcount drop) is
                        // sound. A genuine closure value (already
                        // `Ptr(HeapKind::Closure)`) is written verbatim.
                        if layout.capture_native_kind(i) == NativeKind::Ptr(HeapKind::Closure) {
                            match *kind {
                                // Genuine closure value: write the
                                // `Arc<HeapValue::ClosureRaw>` pointer bits
                                // verbatim — the popped share transfers into
                                // the slot and `release_typed_closure` drops
                                // it via `drop_with_kind(bits, Ptr(Closure))`.
                                NativeKind::Ptr(HeapKind::Closure) => {
                                    write_capture_raw_u64(ptr, &layout, i, *bits);
                                }
                                // Named-function value (`UInt64` fn-id):
                                // materialize a real zero-capture closure
                                // carrier so the slot holds a true
                                // `Arc<HeapValue::ClosureRaw>` matching the
                                // `Ptr(HeapKind::Closure)` drop discipline.
                                NativeKind::UInt64 => {
                                    let fn_id = *bits as u16;
                                    let empty_layout = Arc::new(
                                        shape_value::v2::closure_layout::ClosureLayout
                                            ::from_capture_types_with_native_kinds(
                                                &[], &[], &[],
                                            ),
                                    );
                                    let inner_ptr = alloc_typed_closure(fn_id, 0, &empty_layout);
                                    let inner_block = OwnedClosureBlock::from_raw(
                                        inner_ptr as *const u8,
                                        empty_layout,
                                    );
                                    let arc = Arc::new(HeapValue::ClosureRaw(inner_block));
                                    let closure_bits = Arc::into_raw(arc) as u64;
                                    write_capture_raw_u64(ptr, &layout, i, closure_bits);
                                }
                                // The compiler stamped this capture's slot as
                                // a closure carrier (`Ptr(HeapKind::Closure)`)
                                // but the value that arrived is neither a
                                // closure pointer nor a function-id — a
                                // construction-side classification mismatch
                                // (e.g. a non-callable capture mis-classified
                                // as a function value). Writing the bits
                                // verbatim would have
                                // `release_typed_closure` drop a non-pointer
                                // value as an `Arc<HeapValue>` → heap
                                // corruption / SIGSEGV. Surface-and-stop
                                // instead (ADR-006 §2.7.8 #4 — no fabrication,
                                // no garbage write): a clean RuntimeError is
                                // the binding-compliant outcome.
                                //
                                // The partially-built block (slot `i` still
                                // zeroed) is intentionally NOT reclaimed via
                                // `release_typed_closure`: walking the masks
                                // would `drop_with_kind(0, Ptr(Closure))` on
                                // the null slot `i` and segfault. We leak the
                                // raw allocation — this arm is a fatal,
                                // should-be-unreachable error exit (a
                                // well-formed program never mis-classifies a
                                // capture), so a one-shot leak on the
                                // terminating path is the safe choice over a
                                // null deref.
                                bad => {
                                    return Err(VMError::RuntimeError(format!(
                                        "op_make_closure: capture {} is stamped a \
                                         closure carrier (Ptr(HeapKind::Closure)) but \
                                         the captured value has kind {:?}; a \
                                         non-callable value cannot be captured into a \
                                         function-typed capture slot",
                                        i, bad
                                    )));
                                }
                            }
                        } else {
                            // Carrier-mismatch guard (closures_hof transitive-
                            // capture SIGSEGV fix, 2026-06-22). The layout's
                            // `capture_native_kind(i)` is the compile-time stamp
                            // from `resolve_capture_concrete_type`. When a
                            // capture's `ConcreteType` could NOT be proven at
                            // compile time (e.g. a transitively-captured outer
                            // closure param — `|c| a + b + c` where `b` is the
                            // enclosing closure's unannotated param), the
                            // resolver falls to the `Pointer(Void)` "opaque
                            // heap slot" sentinel → `Ptr(HeapKind::NativeView)`.
                            // That stamp puts the slot in `heap_capture_mask`,
                            // so `release_typed_closure` will
                            // `drop_with_kind(bits, Ptr(NativeView))` it on the
                            // closure's last drop — i.e. reinterpret the bits as
                            // an `Arc<NativeViewData>`. But the value that
                            // actually arrived here is a scalar (the body proves
                            // `b: int` via `a + b` and reads it as `Int64`); its
                            // bits are an integer, not a heap pointer. Writing
                            // them verbatim and later dropping them as an `Arc`
                            // dereferences a small integer as a pointer →
                            // SIGSEGV (the exact closures_hof crash class).
                            //
                            // The popped runtime `kind` is the single source of
                            // truth (ADR-006 §2.7.7 — no fabrication, no
                            // Bool-default). When the layout stamps a heap `Ptr`
                            // carrier for this slot but the value arrived with a
                            // non-`Ptr` scalar kind, that is a construction-side
                            // classification mismatch the compiler could not
                            // prove away. We refuse to write a scalar into a
                            // heap-drop-masked slot (which would corrupt the heap
                            // on drop) and surface-and-stop with a clean
                            // RuntimeError — never a garbage Arc drop. This is
                            // the same discipline as the R1 named-fn `bad =>`
                            // arm above.
                            //
                            // The partially-built block (slot `i` still zeroed)
                            // is intentionally NOT reclaimed via
                            // `release_typed_closure`: walking the masks would
                            // `drop_with_kind(0, Ptr(..))` on the null slot and
                            // segfault. We leak the raw allocation on this fatal
                            // terminating path — a one-shot leak is the safe
                            // choice over a null deref.
                            if matches!(layout.capture_native_kind(i), NativeKind::Ptr(_))
                                && !matches!(*kind, NativeKind::Ptr(_))
                            {
                                return Err(VMError::RuntimeError(format!(
                                    "op_make_closure: capture {} is stamped a \
                                     heap carrier ({:?}) but the captured value \
                                     arrived with scalar kind {:?}; the closure \
                                     captures a value whose type could not be \
                                     proven at compile time (e.g. a \
                                     transitively-captured un-annotated closure \
                                     parameter). Annotate the capture's source \
                                     binding so its type is statically known.",
                                    i,
                                    layout.capture_native_kind(i),
                                    *kind
                                )));
                            }
                            // Write the popped bits verbatim. For `Ptr`
                            // captures the popped share transfers into the
                            // block's slot; `release_typed_closure` walks
                            // `heap_capture_mask` and drops via
                            // `drop_with_kind(bits, layout.capture_native_kind(i))`
                            // (closure_raw.rs:412-418). For non-Ptr scalars
                            // the slot is value-only — no refcount semantics
                            // apply.
                            write_capture_raw_u64(ptr, &layout, i, *bits);
                        }
                    }
                    CaptureKind::OwnedMutable => {
                        // Allocate a typed `Box<T>` matching the layout's
                        // `capture_inner_kind(i)` and store the box ptr
                        // bits in the closure slot. `release_typed_closure`
                        // walks `owned_mutable_capture_mask` and reclaims
                        // each typed box via `drop_owned_mutable_capture`
                        // (closure_raw.rs:421-429).
                        //
                        // For `FieldKind::Ptr` the popped `*bits` already
                        // carries the heap refcount share verbatim — the
                        // box swallows that share, so no retain/release
                        // pair at this site.
                        let inner = layout.capture_inner_kind(i);
                        let cell_ptr_bits: u64 = match inner {
                            FieldKind::I64 => alloc_owned_mutable_i64(*bits as i64) as u64,
                            FieldKind::F64 => alloc_owned_mutable_f64(f64::from_bits(*bits)) as u64,
                            FieldKind::I32 => alloc_owned_mutable_i32(*bits as i64 as i32) as u64,
                            FieldKind::I16 => alloc_owned_mutable_i16(*bits as i64 as i16) as u64,
                            FieldKind::I8 => alloc_owned_mutable_i8(*bits as i64 as i8) as u64,
                            FieldKind::U64 => alloc_owned_mutable_u64(*bits) as u64,
                            FieldKind::U32 => alloc_owned_mutable_u32(*bits as u32) as u64,
                            FieldKind::U16 => alloc_owned_mutable_u16(*bits as u16) as u64,
                            FieldKind::U8 => alloc_owned_mutable_u8(*bits as u8) as u64,
                            FieldKind::Bool => alloc_owned_mutable_bool(*bits != 0) as u64,
                            FieldKind::Ptr => alloc_owned_mutable_ptr(*bits) as u64,
                        };
                        write_capture_raw_u64(ptr, &layout, i, cell_ptr_bits);
                    }
                    CaptureKind::Shared => {
                        // The popped bits are `*const SharedCell` (from
                        // `Arc::into_raw`) produced by an upstream
                        // `AllocSharedLocal` / `AllocSharedModuleBinding`
                        // and pushed via `LoadLocal` / `LoadModuleBinding`,
                        // which already cloned the share through
                        // `clone_with_kind` (HeapKind::SharedCell arm —
                        // `Arc::increment_strong_count`). The popped
                        // share transfers into the closure block's slot;
                        // `release_typed_closure` walks
                        // `shared_capture_mask` and reclaims via
                        // `drop_shared_capture` (closure_raw.rs:430-437) —
                        // reconstructs `Arc::<SharedCell>::from_raw` to
                        // reclaim that share.
                        // No additional retain at this site.
                        write_capture_raw_u64(ptr, &layout, i, *bits);
                    }
                }
            }
            // Take ownership of the block's single refcount share via the
            // owning `OwnedClosureBlock` wrapper.
            OwnedClosureBlock::from_raw(ptr as *const u8, layout)
        };

        // Wrap the owned block in `Arc<HeapValue>` per the §2.7.11 / Q12
        // slot-tier convention (W7 Round-2.5 close `5fa4b19`): the slot
        // bits for `NativeKind::Ptr(HeapKind::Closure)` are
        // `Arc::into_raw(Arc::new(HeapValue::ClosureRaw(...)))`, and the
        // matching `clone_with_kind` / `drop_with_kind` arm for
        // `HeapKind::Closure` operates on `Arc<HeapValue>` directly. The
        // inner `OwnedClosureBlock` manages its own typed-closure-header
        // refcount on its own Clone/Drop — invoked transitively via
        // `Arc<HeapValue>`'s drop when the slot share count hits zero.
        let arc = Arc::new(HeapValue::ClosureRaw(owned));
        let bits = Arc::into_raw(arc) as u64;
        self.push_kinded(bits, NativeKind::Ptr(HeapKind::Closure))
    }

    // Foreign function call

    pub(in crate::executor) fn op_call_foreign(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let foreign_idx = match instruction.operand {
            Some(Operand::ForeignFunction(idx)) => idx as usize,
            _ => return Err(VMError::InvalidOperand),
        };

        // ADR-006 §2.7.7 / playbook §2: the arg-count slot is a post-proof
        // integer; pop kinded and dispatch via `int_operand`.
        let (arg_count_bits, arg_count_kind) = self.pop_kinded()?;
        let arg_count_slot = KindedSlot::new(ValueSlot::from_raw(arg_count_bits), arg_count_kind);
        let arg_count = int_operand(&arg_count_slot)
            .map_err(|_| VMError::RuntimeError("Expected integer for arg count".to_string()))?
            as usize;
        crate::executor::vm_impl::stack::drop_with_kind(arg_count_bits, arg_count_kind);

        // Pop the args into declaration order. Each `pop_kinded` transfers one
        // share (heap-bearing kinds) from the stack into the owning
        // `KindedSlot`; the args vector is the sole owner for the duration of
        // the call (§3.5 pop-not-peek). Kinds come from the §2.7.7 parallel
        // NativeKind stack track — never fabricated from bits, never
        // Bool-defaulted. `KindedSlot` is never stored ON the stack (the
        // §2.7.7-forbidden `Vec<KindedSlot>`-for-stack shape); this owned args
        // vector is a call-local frame-setup carrier, the same shape
        // §2.7.10/§2.7.11 already use.
        let mut args: Vec<KindedSlot> = Vec::with_capacity(arg_count);
        for _ in 0..arg_count {
            let (bits, kind) = self.pop_kinded()?;
            args.push(KindedSlot::new(ValueSlot::from_raw(bits), kind));
        }
        args.reverse();

        // The SHARED foreign-call core (ffi-rebuild §4.9): the one
        // implementation both the interpreter and (later) the JIT call.
        // Divergence between vm and jit modes is impossible for foreign-call
        // semantics by construction.
        let result = self.invoke_foreign_kinded(foreign_idx, &args)?;

        // Transfer the result share to the kinded stack; `mem::forget`
        // prevents the carrier's Drop from double-releasing it.
        self.push_kinded(result.raw(), result.kind())?;
        std::mem::forget(result);
        // `args` drops here — each `KindedSlot` releases its share.
        Ok(())
    }

    /// SHARED foreign-call core (ffi-rebuild §4.9). Called by BOTH tiers (the
    /// interpreter here; the JIT out-of-line call in stage J2). There is
    /// exactly ONE foreign-call implementation in the system, so `--mode vm`
    /// and `--mode jit` cannot diverge on foreign-call semantics.
    ///
    /// Pinned order (§4.2 / §4.8.2): read entry metadata → `Ffi` permission +
    /// `ffi_languages` scope (phase 1, pre-link) → link-now if the handle is
    /// absent (native: `ffi_libraries`/`ffi_symbols` scope BEFORE `dlopen`;
    /// dynamic: runtime lookup) → dispatch. No foreign code (including
    /// `dlopen` ELF constructors) ever runs before its scope check.
    pub(crate) fn invoke_foreign_kinded(
        &mut self,
        foreign_idx: usize,
        args: &[KindedSlot],
    ) -> Result<KindedSlot, VMError> {
        // Read the entry metadata (clone the small bits so the `&self` borrow
        // is released before the link-now `&mut self` calls below).
        let entry = self
            .program
            .foreign_functions
            .get(foreign_idx)
            .ok_or_else(|| {
                VMError::RuntimeError(format!(
                    "Foreign function index {} out of bounds",
                    foreign_idx
                ))
            })?;
        let name = entry.name.clone();
        let language = entry.language.clone();
        let native_spec = entry.native_abi.clone();
        let is_native = native_spec.is_some();

        // §4.8.2 phase 1: coarse `Ffi` presence + `ffi_languages` scope
        // (computable from the entry alone — no handle, no resolution).
        self.check_ffi_permission(&name, &language, is_native)?;

        // Link-now if the handle is absent (§4.2). A failed link is NOT
        // cached — retry on next call (environment may change).
        let already_linked = self
            .foreign_fn_handles
            .get(foreign_idx)
            .map(|h| h.is_some())
            .unwrap_or(false);
        if !already_linked {
            match &native_spec {
                // extern C native ABI: resolve → scope-check → dlopen.
                Some(spec) => {
                    // §4.8.2 phase 2: library / symbol scope BEFORE `dlopen`
                    // (ELF constructors must never run pre-refusal).
                    self.check_ffi_native_scope(&name, spec)?;
                    let layouts = self.program.native_struct_layouts.clone();
                    let resolutions = self.native_resolutions.clone();
                    let root_key = self.native_root_package_key.clone();
                    let linked = native_abi::link_native_function(
                        spec,
                        &layouts,
                        &mut self.native_library_cache,
                        resolutions.as_deref(),
                        root_key.as_deref(),
                    )
                    .map_err(|e| {
                        VMError::RuntimeError(format!(
                            "Failed to link native function '{}' (library '{}', symbol '{}'): {}",
                            name, spec.library, spec.symbol, e
                        ))
                    })?;
                    if foreign_idx >= self.foreign_fn_handles.len() {
                        self.foreign_fn_handles.resize_with(foreign_idx + 1, || None);
                    }
                    self.foreign_fn_handles[foreign_idx] =
                        Some(ForeignFunctionHandle::Native(std::sync::Arc::new(linked)));
                }
                // Dynamic-language (`fn python` / `fn typescript`): look up
                // the runtime in the VM registry (lookup only — no compile),
                // then compile-now via the modular extension vtable (§4.2).
                // The `ffi_languages` scope check already ran in phase 1
                // (`check_ffi_permission`) before this link-now.
                None => {
                    let Some(runtime) = self.language_runtimes.get(&language).cloned() else {
                        return Err(VMError::RuntimeError(format!(
                            "foreign function '{}': no extension provides language '{}'; install \
                             it with `shape ext install {}` or check your frontmatter / shape.toml",
                            name, language, language
                        )));
                    };
                    // Clone the compile inputs so the `&self.program` borrow is
                    // released before the `&mut self.foreign_fn_handles` write.
                    let (body_text, param_names, param_types, return_type, is_async) = {
                        let entry = &self.program.foreign_functions[foreign_idx];
                        (
                            entry.body_text.clone(),
                            entry.param_names.clone(),
                            entry.param_types.clone(),
                            entry.return_type.clone(),
                            entry.is_async,
                        )
                    };
                    // `runtime.compile` carries the extension's compile-error
                    // text verbatim (it contains the foreign-language syntax
                    // error). Note: Python defers syntax checking to invoke, so
                    // a syntax error surfaces at first call, not here.
                    let compiled = runtime
                        .compile(
                            &name,
                            &body_text,
                            &param_names,
                            &param_types,
                            return_type.as_deref(),
                            is_async,
                        )
                        .map_err(|e| {
                            VMError::RuntimeError(format!(
                                "foreign function '{}' ({}): {}",
                                name, language, e
                            ))
                        })?;
                    if foreign_idx >= self.foreign_fn_handles.len() {
                        self.foreign_fn_handles.resize_with(foreign_idx + 1, || None);
                    }
                    self.foreign_fn_handles[foreign_idx] =
                        Some(ForeignFunctionHandle::Runtime { runtime, compiled });
                }
            }
        }

        // Dispatch on the (now-present) handle. The dispatch is a live foreign
        // frame (polyglot-distributed §4.5): a Shape callback re-entered from
        // the foreign body could reach `snapshot()`, whose capture must refuse
        // while foreign runtime state is on the stack. Bracket the dispatch with
        // the reentry counter so the barrier fires even though the foreign
        // activation is not itself a Shape call frame. The counter maintenance
        // lives in this one shared core (ffi-rebuild §4.9 invariant) so vm/jit
        // tiers cannot diverge. No `?`/panic escapes the match below (link-now
        // ran above; the dynamic path's own `catch_unwind` converts panics to
        // `Err`), so the decrement is always reached.
        let handle = self.foreign_fn_handles.get(foreign_idx).cloned().flatten();
        self.foreign_reentry_depth += 1;
        self.foreign_frame_stack
            .push((name.clone(), language.clone()));
        let dispatch_result = match handle {
            // extern-C native ABI path — the libffi marshal + C call over the
            // shared `&[KindedSlot]` carrier.
            Some(ForeignFunctionHandle::Native(linked)) => {
                native_abi::invoke_linked_function(&linked, args).map_err(VMError::RuntimeError)
            }
            // Dynamic-language runtime path (WF-2A stage 3): marshal args
            // (typed, §4.4 container oracle) → invoke the modular extension
            // over the stable msgpack ABI → wrap the outcome into the user's
            // `Result<T,string>` per the Q13/OQ10 error channel (§4.5).
            Some(ForeignFunctionHandle::Runtime { runtime, compiled }) => {
                let (param_types, return_type, schema_id) = {
                    let entry = &self.program.foreign_functions[foreign_idx];
                    (
                        entry.param_types.clone(),
                        entry.return_type.clone().unwrap_or_default(),
                        entry.return_type_schema_id,
                    )
                };
                let schemas = &self.program.type_schema_registry;
                let builtin = &self.builtin_schemas;

                // Host-side panic guard (§4.5): a shape-vm marshalling bug
                // becomes a class-2 `VMError`, never a VM abort. Extension-side
                // panics are contained by the stage-0 `language_runtime_plugin!`
                // `catch_unwind`; `runtime.invoke` returns them as `Err`.
                let guarded = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let args_bytes =
                        foreign_marshal::marshal_args_typed(args, &param_types, schemas)?;
                    let outcome = runtime.invoke(&compiled, &args_bytes).map_err(|e| e.to_string());
                    foreign_marshal::wrap_dynamic_result(
                        outcome,
                        &name,
                        &language,
                        &return_type,
                        schema_id,
                        schemas,
                        builtin,
                    )
                }));
                match guarded {
                    Ok(r) => r,
                    Err(_) => Err(VMError::RuntimeError(format!(
                        "foreign function '{}' ({}): host-side marshalling panicked",
                        name, language
                    ))),
                }
            }
            None => Err(VMError::RuntimeError(format!(
                "foreign function '{}': link produced no handle (internal error)",
                name
            ))),
        };
        self.foreign_reentry_depth -= 1;
        self.foreign_frame_stack.pop();
        dispatch_result
    }

    /// `(function, language)` of the innermost live foreign frame for the §4.5
    /// barrier message. Reads the parallel identity stack maintained by
    /// `invoke_foreign_kinded`; degrades to a generic label if the depth was
    /// bumped without a recorded identity (defensive — should not happen on the
    /// vm path).
    pub(crate) fn live_foreign_frame_identity(&self) -> (String, String) {
        self.foreign_frame_stack
            .last()
            .cloned()
            .unwrap_or_else(|| ("<foreign>".to_string(), "foreign".to_string()))
    }

    /// §4.8.2 phase 1: `Ffi` permission presence + `ffi_languages` scope.
    /// Both are computable from the entry alone (no handle, no resolution),
    /// so they run on every call BEFORE link-now. When no permission envelope
    /// is installed (`granted_permissions == None`, trusted-local `shape run`),
    /// `Ffi` is granted unscoped (ratified posture, OQ13).
    fn check_ffi_permission(
        &self,
        name: &str,
        language: &str,
        is_native: bool,
    ) -> Result<(), VMError> {
        if let Some(granted) = &self.granted_permissions {
            // Defense-in-depth backstop (§4.8.3): the Deterministic-mode refusal
            // is primarily a LOAD-time gate (`deterministic_foreign_gate`,
            // keyed on the compile-time-derived `required_permissions ∋ Ffi`).
            // This call-time check catches the residual cases the load gate
            // cannot see — a non-content-addressed program, or a blob whose
            // permission derivation predates the derivation rule — refusing the
            // foreign call before any link/dlopen even when `Ffi` is granted.
            if granted.contains(&shape_abi_v1::Permission::Deterministic) {
                return Err(VMError::RuntimeError(format!(
                    "foreign call '{}' refused: deterministic execution cannot run foreign code \
                     (extern C / embedded {}); foreign bodies are not attestable deterministic",
                    name, language
                )));
            }
            if !granted.contains(&shape_abi_v1::Permission::Ffi) {
                return Err(VMError::RuntimeError(format!(
                    "foreign call '{}' requires permission Ffi; grant it in shape.toml: \
                     [permissions] ffi = true (optionally scoped: ffi_languages = [\"{}\"])",
                    name, language
                )));
            }
        }
        // `ffi_languages` scopes the dynamic-language verticals; native
        // (`extern C`) entries are scoped by `ffi_libraries`/`ffi_symbols`
        // at link-now instead (§4.8.2).
        if !is_native {
            let opted_in = self
                .scope_constraints
                .as_ref()
                .is_some_and(|sc| sc.ffi_languages.iter().any(|l| l == language));
            if self.ffi_receiver_strict {
                // WF-2F axis C — wire-serve receiver posture (§4.6 / OQ-6,
                // ratified 2026-07-05). `ffi_languages` is a strict OPT-IN
                // allow-list on the network path: a dynamic foreign language
                // is refused unless the receiving operator explicitly listed
                // it, so an EMPTY list refuses ALL dynamic foreign (the
                // deliberate asymmetry against local unscoped-empty-means-all).
                // Zero sender trust: the receiver's own scope is the authority.
                if !opted_in {
                    let allowed: Vec<&str> = self
                        .scope_constraints
                        .as_ref()
                        .map(|sc| sc.ffi_languages.iter().map(|s| s.as_str()).collect())
                        .unwrap_or_default();
                    return Err(VMError::RuntimeError(format!(
                        "foreign call '{}': the server has not opted into the '{}' language \
                         runtime (opted-in ffi_languages: {:?}); the operator must start \
                         `shape serve` with --ffi-languages {} to allow it",
                        name, language, allowed, language
                    )));
                }
            } else if let Some(sc) = &self.scope_constraints {
                // Local posture (ffi-rebuild §4.8.2): an empty list means "all
                // dynamic foreign allowed" (parity with unscoped `FsRead`); a
                // non-empty list narrows to the named languages.
                if !sc.ffi_languages.is_empty() && !opted_in {
                    return Err(VMError::RuntimeError(format!(
                        "foreign call '{}': language '{}' is not in the allowed ffi_languages \
                         scope {:?}",
                        name, language, sc.ffi_languages
                    )));
                }
            }
        }
        Ok(())
    }

    /// §4.8.2 phase 2: `ffi_libraries` (glob over the declared library) +
    /// `ffi_symbols` (glob over the symbol) for `extern C`. Runs at link-now,
    /// BEFORE `dlopen`, so an unauthorized library is refused before any ELF
    /// constructor can execute. An empty list means "all allowed" (parity with
    /// unscoped `FsRead`); no scope envelope installed also means all allowed.
    fn check_ffi_native_scope(
        &self,
        name: &str,
        spec: &crate::bytecode::NativeAbiSpec,
    ) -> Result<(), VMError> {
        let Some(sc) = &self.scope_constraints else {
            return Ok(());
        };
        if !sc.ffi_libraries.is_empty()
            && !sc
                .ffi_libraries
                .iter()
                .any(|pat| ffi_glob_match(pat, &spec.library))
        {
            return Err(VMError::RuntimeError(format!(
                "foreign call '{}': native library '{}' is not in the allowed ffi_libraries \
                 scope {:?}",
                name, spec.library, sc.ffi_libraries
            )));
        }
        if !sc.ffi_symbols.is_empty()
            && !sc
                .ffi_symbols
                .iter()
                .any(|pat| ffi_glob_match(pat, &spec.symbol))
        {
            return Err(VMError::RuntimeError(format!(
                "foreign call '{}': native symbol '{}' is not in the allowed ffi_symbols \
                 scope {:?}",
                name, spec.symbol, sc.ffi_symbols
            )));
        }
        Ok(())
    }

    // Return operations

    pub(in crate::executor) fn op_return(&mut self) -> Result<(), VMError> {
        if let Some(frame) = self.call_stack.pop() {
            // Restore instruction pointer
            self.ip = frame.return_ip;

            // Clean up register window: release each slot's share then
            // restore sp to base_pointer.
            //
            // ADR-006 §2.7.7 WB2.4: with retain-on-read in force, every
            // slot in `[bp..sp)` holds an **owning** share; releasing per
            // slot is required to avoid leaks. `truncate_stack` walks the
            // parallel kind track and dispatches `drop_with_kind` per
            // slot — replaces the deleted `vw_drop`.
            let bp = frame.base_pointer;
            self.truncate_stack(bp);
            // WB2.3 retain-on-read: release the closure keep-alive (if
            // any) now that the callee's `OwnedMutable` / `Shared`
            // pointer captures are no longer in scope.
            //
            // ADR-006 §2.7.8 / Q10: `closure_heap_kind` is the lockstep
            // companion to `closure_heap_bits` — both `Some` together
            // or both `None` together at every observable boundary.
            // The release path dispatches via `drop_with_kind(bits, kind)`
            // — never bare `vw_drop` (forbidden §2.7.7 #8) and never a
            // Bool-default fallback (forbidden §2.7.7 #9).
            match (frame.closure_heap_bits, frame.closure_heap_kind) {
                (Some(bits), Some(kind)) => {
                    crate::executor::vm_impl::stack::drop_with_kind(bits, kind);
                }
                (None, None) => {}
                (bits, kind) => {
                    debug_assert!(
                        false,
                        "ADR-006 §2.7.8 / Q10: CallFrame.closure_heap_bits / closure_heap_kind \
                         lockstep violated: bits={:?}, kind={:?}",
                        bits.is_some(),
                        kind.is_some(),
                    );
                }
            }
        } else {
            // Return from main
            self.ip = self.program.instructions.len();
        }
        Ok(())
    }

    pub(in crate::executor) fn op_return_value(&mut self) -> Result<(), VMError> {
        // ADR-006 §2.7.7 / playbook §2: pop_kinded captures the return
        // value's kind from the callee's last produced opcode.
        let (return_bits, return_kind) = self.pop_kinded()?;
        self.return_value_inner(return_bits, return_kind)
    }

    /// Shared inner body for `op_return_value` and the typed
    /// `op_return_value_<kind>` family (Wave E+3, opcodes 0x198..=0x1A2).
    ///
    /// The typed variants pop the return value via `pop_kinded` and feed
    /// it here for frame cleanup + caller-side push, supplying the kind
    /// that matches their opcode suffix. The encoded `<Kind>` flows
    /// through to the caller's parallel kind track via `push_kinded`.
    #[inline]
    pub(in crate::executor) fn return_value_inner(
        &mut self,
        return_bits: u64,
        return_kind: shape_value::NativeKind,
    ) -> Result<(), VMError> {
        if let Some(frame) = self.call_stack.pop() {
            // Restore instruction pointer
            self.ip = frame.return_ip;

            // Clean up register window (see `op_return` for the
            // retain-on-read rationale). ADR-006 §2.7.7 WB2.4: every
            // slot in [bp..sp) is kind-tracked; `truncate_stack` walks
            // the parallel kind track and dispatches `drop_with_kind`
            // per slot — replaces the deleted `vw_drop`.
            let bp = frame.base_pointer;
            self.truncate_stack(bp);

            // WB2.3 retain-on-read: release the closure keep-alive.
            //
            // ADR-006 §2.7.8 / Q10: `closure_heap_kind` is the lockstep
            // companion to `closure_heap_bits` — both `Some` together
            // or both `None` together. Release dispatches via
            // `drop_with_kind(bits, kind)` — never bare `vw_drop`
            // (forbidden §2.7.7 #8), never a Bool-default fallback
            // (forbidden §2.7.7 #9).
            match (frame.closure_heap_bits, frame.closure_heap_kind) {
                (Some(bits), Some(kind)) => {
                    crate::executor::vm_impl::stack::drop_with_kind(bits, kind);
                }
                (None, None) => {}
                (bits, kind) => {
                    debug_assert!(
                        false,
                        "ADR-006 §2.7.8 / Q10: CallFrame.closure_heap_bits / closure_heap_kind \
                         lockstep violated: bits={:?}, kind={:?}",
                        bits.is_some(),
                        kind.is_some(),
                    );
                }
            }

            // Push return value with its kind on the parallel track.
            self.push_kinded(return_bits, return_kind)?;
        } else {
            // Return from main
            self.push_kinded(return_bits, return_kind)?;
            self.ip = self.program.instructions.len();
        }
        Ok(())
    }

    // ─────────────────────────────────────────────────────────────────
    // Wave E+3: typed `ReturnValue<Kind>` handlers (opcodes 0x198..=0x1A2)
    //
    // Each typed handler is a thin wrapper around `return_value_inner`.
    // Per ADR-006 §2.7.7 the kind on the parallel-kind track MUST come
    // from the actual producer (popped from the callee stack via
    // `pop_kinded`), never fabricated from the opcode suffix. The
    // encoded `<Kind>` suffix is a JIT static-annotation only — a
    // compile-time witness that the producer kind matched the declared
    // return type. Fabricating from the suffix here would clobber heap
    // carriers (`Ptr(HeapKind::Result)` / `Ptr(HeapKind::Option)` /
    // enum TypedObject carriers) when the compile-time picker
    // mis-classifies the function body (joint-fix-1 c7+c3 bug).
    //
    // The legacy `op_return_value` (0x45) stays live for unproven-type
    // return positions and uses the same "kind from producer" rule.
    // ─────────────────────────────────────────────────────────────────

    pub(in crate::executor) fn op_return_value_i64(&mut self) -> Result<(), VMError> {
        let (bits, src_kind) = self.pop_kinded()?;
        self.return_value_inner(bits, src_kind)
    }

    pub(in crate::executor) fn op_return_value_u64(&mut self) -> Result<(), VMError> {
        let (bits, src_kind) = self.pop_kinded()?;
        self.return_value_inner(bits, src_kind)
    }

    pub(in crate::executor) fn op_return_value_f64(&mut self) -> Result<(), VMError> {
        let (bits, src_kind) = self.pop_kinded()?;
        self.return_value_inner(bits, src_kind)
    }

    pub(in crate::executor) fn op_return_value_i32(&mut self) -> Result<(), VMError> {
        let (bits, src_kind) = self.pop_kinded()?;
        self.return_value_inner(bits, src_kind)
    }

    pub(in crate::executor) fn op_return_value_u32(&mut self) -> Result<(), VMError> {
        let (bits, src_kind) = self.pop_kinded()?;
        self.return_value_inner(bits, src_kind)
    }

    pub(in crate::executor) fn op_return_value_i16(&mut self) -> Result<(), VMError> {
        let (bits, src_kind) = self.pop_kinded()?;
        self.return_value_inner(bits, src_kind)
    }

    pub(in crate::executor) fn op_return_value_u16(&mut self) -> Result<(), VMError> {
        let (bits, src_kind) = self.pop_kinded()?;
        self.return_value_inner(bits, src_kind)
    }

    pub(in crate::executor) fn op_return_value_i8(&mut self) -> Result<(), VMError> {
        let (bits, src_kind) = self.pop_kinded()?;
        self.return_value_inner(bits, src_kind)
    }

    pub(in crate::executor) fn op_return_value_u8(&mut self) -> Result<(), VMError> {
        let (bits, src_kind) = self.pop_kinded()?;
        self.return_value_inner(bits, src_kind)
    }

    pub(in crate::executor) fn op_return_value_bool(&mut self) -> Result<(), VMError> {
        let (bits, src_kind) = self.pop_kinded()?;
        self.return_value_inner(bits, src_kind)
    }

    pub(in crate::executor) fn op_return_value_ptr(&mut self) -> Result<(), VMError> {
        // Ptr returns: preserve the source kind (the producing typed
        // opcode emitted the concrete `NativeKind::Ptr(HeapKind::*)` /
        // `::String`); the parallel kind track records exactly what the
        // callee returned so the caller's stack interpretation is
        // correct.
        let (bits, src_kind) = self.pop_kinded()?;
        self.return_value_inner(bits, src_kind)
    }
}
