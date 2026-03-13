//! Speculative analysis helpers for collection opcodes.

use shape_vm::bytecode::{OpCode, Operand};

use crate::translator::types::BytecodeToIR;

#[derive(Clone, Copy)]
enum Tracked {
    OnStack(i32), // number of values above tracked value
    InLocal(u16),
    InModuleBinding(u16),
}

fn is_numeric_consumer(op: OpCode) -> bool {
    matches!(
        op,
        OpCode::Add
            | OpCode::Sub
            | OpCode::Mul
            | OpCode::Div
            | OpCode::Mod
            | OpCode::Pow
            | OpCode::Neg
            | OpCode::IntToNumber
            | OpCode::NumberToInt
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
            | OpCode::AddDecimal
            | OpCode::SubDecimal
            | OpCode::MulDecimal
            | OpCode::DivDecimal
            | OpCode::ModDecimal
            | OpCode::PowDecimal
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
            | OpCode::EqInt
            | OpCode::NeqInt
            | OpCode::GtNumber
            | OpCode::LtNumber
            | OpCode::GteNumber
            | OpCode::LteNumber
            | OpCode::EqNumber
            | OpCode::NeqNumber
            | OpCode::GtDecimal
            | OpCode::LtDecimal
            | OpCode::GteDecimal
            | OpCode::LteDecimal
    )
}

// Variable-arity opcodes don't have reliable stack effects at this layer.
// If the tracked value reaches one, treat as non-numeric/unknown.
fn is_unknown_stack_effect(op: OpCode) -> bool {
    matches!(
        op,
        OpCode::Call
            | OpCode::CallValue
            | OpCode::CallMethod
            | OpCode::BuiltinCall
            | OpCode::DynMethodCall
            | OpCode::CallForeign
    )
}

impl<'a, 'b> BytecodeToIR<'a, 'b> {
    /// Return true when the current `GetProp` result is consumed by nearby
    /// numeric operations. This is a compile-time hint only.
    pub(super) fn should_speculate_numeric_array_read(&self) -> bool {
        let mut tracked = Tracked::OnStack(0);
        let start = self.current_instr_idx.saturating_add(1);
        let end = (start + 24).min(self.program.instructions.len());

        for j in start..end {
            let instr = &self.program.instructions[j];
            let op = instr.opcode;

            tracked = match tracked {
                Tracked::InLocal(local_idx) => match (op, instr.operand) {
                    (OpCode::LoadLocal, Some(Operand::Local(idx))) if idx == local_idx => {
                        Tracked::OnStack(0)
                    }
                    (OpCode::StoreLocal, Some(Operand::Local(idx))) if idx == local_idx => {
                        return false;
                    }
                    (OpCode::StoreLocalTyped, Some(Operand::TypedLocal(idx, _)))
                        if idx == local_idx =>
                    {
                        return false;
                    }
                    _ => Tracked::InLocal(local_idx),
                },
                Tracked::InModuleBinding(binding_idx) => match (op, instr.operand) {
                    (OpCode::LoadModuleBinding, Some(Operand::ModuleBinding(idx)))
                        if idx == binding_idx =>
                    {
                        Tracked::OnStack(0)
                    }
                    (OpCode::StoreModuleBinding, Some(Operand::ModuleBinding(idx)))
                        if idx == binding_idx =>
                    {
                        return false;
                    }
                    _ => Tracked::InModuleBinding(binding_idx),
                },
                Tracked::OnStack(mut depth_from_top) => {
                    if is_unknown_stack_effect(op) {
                        return false;
                    }

                    let pops = op.stack_pops() as i32;
                    let pushes = op.stack_pushes() as i32;

                    if depth_from_top < pops {
                        if is_numeric_consumer(op) {
                            return true;
                        }
                        match (op, instr.operand) {
                            (OpCode::StoreLocal, Some(Operand::Local(idx))) => {
                                Tracked::InLocal(idx)
                            }
                            (OpCode::StoreLocalTyped, Some(Operand::TypedLocal(idx, _))) => {
                                Tracked::InLocal(idx)
                            }
                            (OpCode::StoreModuleBinding, Some(Operand::ModuleBinding(idx))) => {
                                Tracked::InModuleBinding(idx)
                            }
                            // Tracked value survives through these stack shuffles.
                            (OpCode::Dup, _) => Tracked::OnStack(0),
                            (OpCode::Swap, _) => Tracked::OnStack(1),
                            _ => return false,
                        }
                    } else {
                        depth_from_top = depth_from_top - pops + pushes;
                        Tracked::OnStack(depth_from_top)
                    }
                }
            };
        }

        false
    }
}
