//! Array LICM tracing helpers used by loop setup in control-flow lowering.

use shape_vm::bytecode::{OpCode, Operand};

use crate::translator::loop_analysis::LoopInfo;
use crate::translator::types::BytecodeToIR;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArrayBaseSource {
    Local(u16),
    RefLocal(u16),
}

impl<'a, 'b> BytecodeToIR<'a, 'b> {
    /// Stack effect model for tracing the object source of `GetProp` backwards.
    fn array_trace_stack_effect(op: OpCode) -> Option<(i32, i32)> {
        let eff = match op {
            // Push-only
            OpCode::LoadLocal
            | OpCode::LoadLocalTrusted
            | OpCode::LoadModuleBinding
            | OpCode::LoadClosure
            | OpCode::PushConst
            | OpCode::PushNull
            | OpCode::DerefLoad => (0, 1),
            // Unary
            OpCode::IntToNumber
            | OpCode::NumberToInt
            | OpCode::CastWidth
            | OpCode::Neg
            | OpCode::Not => (1, 1),
            // Binary numeric/comparison ops
            OpCode::Add
            | OpCode::Sub
            | OpCode::Mul
            | OpCode::Div
            | OpCode::Mod
            | OpCode::Pow
            | OpCode::AddInt
            | OpCode::SubInt
            | OpCode::MulInt
            | OpCode::DivInt
            | OpCode::ModInt
            | OpCode::PowInt
            | OpCode::AddNumber
            | OpCode::SubNumber
            | OpCode::MulNumber
            | OpCode::DivNumber
            | OpCode::ModNumber
            | OpCode::PowNumber
            | OpCode::Gt
            | OpCode::Lt
            | OpCode::Gte
            | OpCode::Lte
            | OpCode::Eq
            | OpCode::Neq
            | OpCode::GtInt
            | OpCode::LtInt
            | OpCode::GteInt
            | OpCode::LteInt
            | OpCode::GtNumber
            | OpCode::LtNumber
            | OpCode::GteNumber
            | OpCode::LteNumber
            | OpCode::EqInt
            | OpCode::EqNumber
            | OpCode::NeqInt
            | OpCode::NeqNumber => (2, 1),
            // Stack manipulation
            OpCode::Dup => (1, 2),
            OpCode::Swap => (2, 2),
            // Nested indexed/property reads can appear inside index expressions.
            OpCode::GetProp => (2, 1),
            _ => return None,
        };
        Some(eff)
    }

    /// Trace the base object source for `GetProp` at `get_prop_idx` (if possible).
    /// Returns the producer of the object operand (second from top before GetProp).
    fn array_base_source_for_get_prop(
        &self,
        info: &LoopInfo,
        get_prop_idx: usize,
    ) -> Option<ArrayBaseSource> {
        let get = self.program.instructions.get(get_prop_idx)?;
        if get.opcode != OpCode::GetProp || get.operand.is_some() {
            return None;
        }

        let unbox_log = std::env::var_os("SHAPE_JIT_UNBOX_LOG").is_some();

        // Object is second from top before GetProp.
        let mut pos_from_top: i32 = 1;
        for j in ((info.header_idx + 1)..get_prop_idx).rev() {
            let instr = &self.program.instructions[j];
            let se = Self::array_trace_stack_effect(instr.opcode);
            if se.is_none() {
                if unbox_log {
                    eprintln!(
                        "[shape-jit-array-licm-debug] header={} GetProp@{} trace aborted at instr {} {:?}",
                        info.header_idx, get_prop_idx, j, instr.opcode
                    );
                }
                return None;
            }
            let (pops, pushes) = se.unwrap();
            if pos_from_top < pushes {
                return match instr.opcode {
                    OpCode::LoadLocal | OpCode::LoadLocalTrusted => match &instr.operand {
                        Some(Operand::Local(idx)) => Some(ArrayBaseSource::Local(*idx)),
                        _ => None,
                    },
                    OpCode::DerefLoad => match &instr.operand {
                        Some(Operand::Local(idx)) => Some(ArrayBaseSource::RefLocal(*idx)),
                        _ => None,
                    },
                    _ => None,
                };
            }
            pos_from_top = pos_from_top - pushes + pops;
            if pos_from_top < 0 {
                return None;
            }
        }
        None
    }

    /// Check if an invariant local is used as an array base in indexed ops.
    ///
    /// Covers:
    /// - `GetProp` dynamic reads (`arr[idx]`)
    /// - `SetLocalIndex` dynamic writes (`arr[idx] = value`)
    pub(super) fn is_used_as_array_in_loop(&self, info: &LoopInfo, local_idx: u16) -> bool {
        let unbox_log = std::env::var_os("SHAPE_JIT_UNBOX_LOG").is_some();
        for i in (info.header_idx + 1)..info.end_idx {
            let instr = &self.program.instructions[i];
            if instr.opcode == OpCode::GetProp && instr.operand.is_none() {
                let source = self.array_base_source_for_get_prop(info, i);
                if unbox_log {
                    eprintln!(
                        "[shape-jit-array-licm-debug] header={} GetProp@{} source={:?} looking_for=Local({})",
                        info.header_idx, i, source, local_idx
                    );
                }
                if source == Some(ArrayBaseSource::Local(local_idx)) {
                    return true;
                }
            }
            if instr.opcode == OpCode::SetLocalIndex
                && matches!(instr.operand, Some(Operand::Local(idx)) if idx == local_idx)
            {
                return true;
            }
        }
        false
    }

    /// Check if an invariant reference local is used as an array base via
    /// `DerefLoad(ref_slot) ... GetProp`.
    pub(super) fn is_ref_used_as_array_in_loop(&self, info: &LoopInfo, ref_slot: u16) -> bool {
        for i in (info.header_idx + 1)..info.end_idx {
            let instr = &self.program.instructions[i];
            if instr.opcode == OpCode::GetProp
                && instr.operand.is_none()
                && self.array_base_source_for_get_prop(info, i)
                    == Some(ArrayBaseSource::RefLocal(ref_slot))
            {
                return true;
            }
        }
        false
    }

    /// Check if an invariant reference local is used in SetIndexRef inside the loop.
    pub(super) fn is_ref_used_for_set_index_in_loop(&self, info: &LoopInfo, ref_slot: u16) -> bool {
        for i in (info.header_idx + 1)..info.end_idx {
            let instr = &self.program.instructions[i];
            if instr.opcode == OpCode::SetIndexRef
                && matches!(instr.operand, Some(Operand::Local(idx)) if idx == ref_slot)
            {
                return true;
            }
        }
        false
    }
}
