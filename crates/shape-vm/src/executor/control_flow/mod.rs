//! Control flow operations for the VM executor
//!
//! Handles: Jump, JumpIfFalse, JumpIfTrue, Call, CallValue, CallForeign, Return, ReturnValue

pub mod foreign_marshal;
pub mod jit_abi;
pub mod native_abi;

use crate::executor::builtins::kind_coerce::int_operand;
use crate::{
    bytecode::{Instruction, OpCode, Operand},
    executor::VirtualMachine,
};
use shape_value::{KindedSlot, NativeKind, ValueSlot, VMError};

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
        // non-null when live (same shape as the String / Ptr(_) heap-arm
        // truthy rule below).
        NativeKind::StringV2 | NativeKind::DecimalV2 => bits != 0,
        NativeKind::String | NativeKind::Ptr(_) => bits != 0,
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
        use shape_value::HeapKind;
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
            for (i, (bits, _kind)) in popped.iter().enumerate() {
                match layout.capture_storage_kind(i) {
                    CaptureKind::Immutable => {
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
                            FieldKind::F64 => {
                                alloc_owned_mutable_f64(f64::from_bits(*bits)) as u64
                            }
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
        let _foreign_idx = match instruction.operand {
            Some(Operand::ForeignFunction(idx)) => idx as usize,
            _ => return Err(VMError::InvalidOperand),
        };

        // ADR-006 §2.7.7 / playbook §2: arg-count slot is post-proof
        // integer; pop kinded and dispatch via `int_operand`. The
        // remaining body — argument marshalling for foreign / native
        // ABI calls — crosses the §2.7.5 cross-crate ABI boundary into
        // `foreign_marshal::*` and `native_abi::invoke_linked_function`,
        // both of which are forbidden-pattern carriers (raw `&[ValueWord]`
        // / `vmarray_from_vec` / `tag_bits`). Per the prompt those
        // companion files are stubbed to `NotImplemented(SURFACE: phase-2c
        // — extern C FFI rebuild)` and this consumer follows the same
        // surface; the rebuild needs to thread `&[KindedSlot]` into the
        // marshal call sites with the extension contract still raw u64
        // on the §2.7.5 FFI side.
        let (arg_count_bits, arg_count_kind) = self.pop_kinded()?;
        let arg_count_slot = KindedSlot::new(ValueSlot::from_raw(arg_count_bits), arg_count_kind);
        let _arg_count = int_operand(&arg_count_slot)
            .map_err(|_| VMError::RuntimeError("Expected integer for arg count".to_string()))?
            as usize;
        crate::executor::vm_impl::stack::drop_with_kind(arg_count_bits, arg_count_kind);

        // Drop the arg slots so the stack is balanced even on the
        // surface path. Each argument was pushed by the calling
        // sequence and must be released to keep the parallel kind
        // track in lockstep.
        for _ in 0.._arg_count {
            let (bits, kind) = self.pop_kinded()?;
            crate::executor::vm_impl::stack::drop_with_kind(bits, kind);
        }

        // Phase-2c rebuild notes: the deleted body called
        // `foreign_marshal::marshal_args(&args, ...)` /
        // `foreign_marshal::unmarshal_result(...)` for the dynamic-language
        // runtime path, and `native_abi::invoke_linked_function(&linked,
        // &args, Some(raw_invoker), Some(vw_slice))` for the extern-C
        // path. The latter built a `vm_callable_invoker(callable:
        // &ValueWord, args: &[ValueWord])` on the §2.7.5 stable extension
        // contract. Rebuild needs `&[KindedSlot]` on the runtime side
        // with raw u64 retained on the FFI extension side.
        Err(VMError::NotImplemented(format!(
            "op_call_foreign: phase-2c — extern C FFI rebuild (ADR-006 §2.7.4 / §2.7.5)"
        )))
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
    // The handler bodies are identical at runtime — the encoded `<Kind>`
    // exists for static type information so the caller's stack
    // discipline is known at the call site (consumed by the JIT and
    // other downstream tooling).
    //
    // The legacy `op_return_value` (0x45) stays live for unproven-type
    // return positions.
    // ─────────────────────────────────────────────────────────────────

    pub(in crate::executor) fn op_return_value_i64(&mut self) -> Result<(), VMError> {
        // Opcode-suffix supplies the return kind regardless of what the
        // callee's last opcode reported (typed return is post-proof).
        let (bits, _src_kind) = self.pop_kinded()?;
        self.return_value_inner(bits, shape_value::NativeKind::Int64)
    }

    pub(in crate::executor) fn op_return_value_u64(&mut self) -> Result<(), VMError> {
        let (bits, _src_kind) = self.pop_kinded()?;
        self.return_value_inner(bits, shape_value::NativeKind::UInt64)
    }

    pub(in crate::executor) fn op_return_value_f64(&mut self) -> Result<(), VMError> {
        let (bits, _src_kind) = self.pop_kinded()?;
        self.return_value_inner(bits, shape_value::NativeKind::Float64)
    }

    pub(in crate::executor) fn op_return_value_i32(&mut self) -> Result<(), VMError> {
        let (bits, _src_kind) = self.pop_kinded()?;
        self.return_value_inner(bits, shape_value::NativeKind::Int32)
    }

    pub(in crate::executor) fn op_return_value_u32(&mut self) -> Result<(), VMError> {
        let (bits, _src_kind) = self.pop_kinded()?;
        self.return_value_inner(bits, shape_value::NativeKind::UInt32)
    }

    pub(in crate::executor) fn op_return_value_i16(&mut self) -> Result<(), VMError> {
        let (bits, _src_kind) = self.pop_kinded()?;
        self.return_value_inner(bits, shape_value::NativeKind::Int16)
    }

    pub(in crate::executor) fn op_return_value_u16(&mut self) -> Result<(), VMError> {
        let (bits, _src_kind) = self.pop_kinded()?;
        self.return_value_inner(bits, shape_value::NativeKind::UInt16)
    }

    pub(in crate::executor) fn op_return_value_i8(&mut self) -> Result<(), VMError> {
        let (bits, _src_kind) = self.pop_kinded()?;
        self.return_value_inner(bits, shape_value::NativeKind::Int8)
    }

    pub(in crate::executor) fn op_return_value_u8(&mut self) -> Result<(), VMError> {
        let (bits, _src_kind) = self.pop_kinded()?;
        self.return_value_inner(bits, shape_value::NativeKind::UInt8)
    }

    pub(in crate::executor) fn op_return_value_bool(&mut self) -> Result<(), VMError> {
        let (bits, _src_kind) = self.pop_kinded()?;
        self.return_value_inner(bits, shape_value::NativeKind::Bool)
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

