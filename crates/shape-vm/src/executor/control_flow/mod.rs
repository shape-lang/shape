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
        NativeKind::String | NativeKind::Ptr(_) => bits != 0,
    }
}

impl VirtualMachine {
    #[inline(always)]
    pub(in crate::executor) fn exec_control_flow(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use OpCode::*;
        match instruction.opcode {
            Jump => self.op_jump(instruction)?,
            JumpIfFalse => self.op_jump_if_false(instruction)?,
            JumpIfFalseTrusted => self.op_jump_if_false_trusted(instruction)?,
            JumpIfTrue => self.op_jump_if_true(instruction)?,
            Call => self.op_call(instruction)?,
            CallValue => self.op_call_value()?,
            CallClosure => self.op_call_closure(instruction)?,
            CallFunctionIndirect => self.op_call_function_indirect(instruction)?,
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
    ) -> Result<(), VMError> {
        let _arity = match instruction.operand {
            Some(Operand::Count(n)) => n as usize,
            _ => return Err(VMError::InvalidOperand),
        };
        // ADR-006 §2.7.4 / §2.7.5 SURFACE: closure dispatch through
        // `VmClosureHandle` requires `as_heap_value()` + `HeapValue::*`
        // match plus the `extract_closure_info` raw-bits helper. The
        // current call shape passes `&[ValueWord]` slices into
        // `call_closure_with_nb_args_keepalive`, which is the consumer
        // side of the §2.7.5 cross-crate ABI. Rebuild is phase-2c.
        Err(VMError::NotImplemented(format!(
            "op_call_closure: {}",
            PHASE_2C_CALL_REBUILD_SURFACE
        )))
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
    ) -> Result<(), VMError> {
        let _arity = match instruction.operand {
            Some(Operand::Count(n)) => n as usize,
            _ => return Err(VMError::InvalidOperand),
        };
        // ADR-006 §2.7.4 / §2.7.5 SURFACE: same `dispatch_call_closure_like`
        // path as `op_call_closure` — see surface there for the rebuild
        // scope.
        Err(VMError::NotImplemented(format!(
            "op_call_function_indirect: {}",
            PHASE_2C_CALL_REBUILD_SURFACE
        )))
    }

    /// Shared VM dispatch helper for `CallClosure` / `CallFunctionIndirect`.
    ///
    /// The arity comes from the opcode operand rather than the stack, so
    /// this helper does not pop a Count sentinel before peeking the
    /// callee. Otherwise the dispatch tree mirrors `op_call_value`.
    ///
    /// Closure spec Phase G §5.4: records the resolved target `function_id`
    /// into the current function's feedback vector so the JIT Tier 2 can
    /// emit speculative direct-call guards when the site has gone
    /// monomorphic. The feedback recording happens on the indirect path
    /// (closure / function-ref callees). Host closures and module
    /// functions are not recorded (no stable `function_id` / different
    /// call ABI).
    #[allow(dead_code)]
    fn dispatch_call_closure_like(&mut self, _arg_count: usize) -> Result<(), VMError> {
        // ADR-006 §2.7.4 / §2.7.5 SURFACE: the indirect-call dispatch tree
        // (TAG_FUNCTION / TAG_MODULE_FN / TAG_HEAP closure / HostClosure)
        // depends on tag-bit dispatch on a stack-resident `ValueWord`-shape
        // callee plus `as_heap_value()` + `HeapValue::*` match for
        // `HeapValue::HostClosure(callable)`. The TAG_HEAP closure arm
        // additionally calls `raw_helpers::extract_closure_info(bits)`
        // (the D-raw-helpers sub-cluster rewrite landed at `a27c0e4` —
        // `extract_closure_info` was deleted with the rest of the
        // forbidden `tag_bits` consumer surface). The arg-slicing path
        // collects `Vec<ValueWord>` for `call_closure_with_nb_args_keepalive`
        // — that consumer is `&[ValueWord]` per §2.7.5 cross-crate ABI on
        // the call_convention boundary, also out of B11 territory.
        //
        // Re-emission needs (a) a kinded callee dispatch on
        // `(NativeKind, bits)` rather than tag bits, (b) closure handle
        // recovery via `slot.as_heap_value()` (single discriminator per
        // ADR-005 §1) and (c) the `&[KindedSlot]` consumer-side migration
        // of `call_closure_with_nb_args_keepalive`. All three cross
        // territories — surface and stop per playbook §8.
        Err(VMError::NotImplemented(format!(
            "dispatch_call_closure_like: {}",
            PHASE_2C_CALL_REBUILD_SURFACE
        )))
    }

    pub(in crate::executor) fn op_call_value(&mut self) -> Result<(), VMError> {
        // ADR-006 §2.7.7 / playbook §2: arg-count is post-proof integer
        // kind; same body-site `int_operand` dispatch as `op_call`.
        let (arg_count_bits, arg_count_kind) = self.pop_kinded()?;
        let arg_count_slot = KindedSlot::new(ValueSlot::from_raw(arg_count_bits), arg_count_kind);
        let _arg_count = int_operand(&arg_count_slot)
            .map_err(|_| VMError::RuntimeError("Expected integer for arg count".to_string()))?
            as usize;
        crate::executor::vm_impl::stack::drop_with_kind(arg_count_bits, arg_count_kind);

        // ADR-006 §2.7.4 / §2.7.5 SURFACE: same indirect-call dispatch
        // surface as `dispatch_call_closure_like` — see surface there.
        // The `op_call_value` body additionally peeks the callee via
        // `stack_read_raw(idx)` (deleted shim) — the kinded peek
        // (`stack_read_kinded_raw` / `stack_peek_kinded`) is available,
        // but the downstream `tag_bits::get_tag` / `as_heap_ref()` /
        // `extract_closure_info` chain is the unmigrated forbidden-pattern
        // surface that the rebuild needs.
        Err(VMError::NotImplemented(format!(
            "op_call_value: {}",
            PHASE_2C_CALL_REBUILD_SURFACE
        )))
    }

    pub(in crate::executor) fn op_make_closure(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        // Closure spec H5: `MakeClosure` accepts two operand shapes:
        //   - `Operand::Function(fid)`            — non-escaping closure.
        //   - `Operand::ClosureAlloc { fid, .. }` — compiler-tagged with
        //     escape status (the VM path is identical; the JIT's MIR
        //     lowering reads `escapes` to pick stack vs. heap codegen).
        let func_id_opt = match instruction.operand {
            Some(Operand::Function(fid)) => Some(fid),
            Some(Operand::ClosureAlloc { fid, .. }) => Some(fid),
            _ => None,
        };
        let Some(func_id) = func_id_opt else {
            return Err(VMError::InvalidOperand);
        };
        // ADR-006 §2.7.4 / §2.7.5 SURFACE: closure construction populates
        // a typed `TypedClosureHeader` block per `ClosureLayout` —
        // immutable captures via `write_capture_typed` with refcount
        // retain on heap arms, OwnedMutable captures via per-FieldKind
        // `Box<T>` allocators (`alloc_owned_mutable_<kind>`), and Shared
        // captures via `Arc::increment_strong_count` on a pre-existing
        // `*const SharedCell` pointer. The pre-rebuild shape consumed
        // the deleted ValueWord-shape capture surface (raw-bits clone +
        // `from_heap_value` push). Rebuild needs kinded capture reads
        // (per-slot `NativeKind` driving `clone_with_kind` on heap arms),
        // §2.7.8 closure-cell parallel-track recording, and a kinded
        // push of the resulting closure block — coordinated with the
        // shape-jit FFI side. Surface and stop.
        let _ = func_id;
        Err(VMError::NotImplemented(format!(
            "op_make_closure: {}",
            PHASE_2C_CALL_REBUILD_SURFACE
        )))
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
    fn return_value_inner(
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

