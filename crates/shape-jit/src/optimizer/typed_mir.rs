//! Phase 1: lightweight typed MIR extraction from bytecode.

use std::collections::{HashMap, HashSet};

use shape_vm::bytecode::{BytecodeProgram, Constant, OpCode, Operand};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScalarType {
    I64,
    F64,
    Bool,
    Boxed,
    Unknown,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum MirOp {
    LoadLocal(u16),
    StoreLocal(u16),
    LoadModuleBinding(u16),
    StoreModuleBinding(u16),
    PushConst,
    Arithmetic(OpCode),
    Comparison(OpCode),
    LoopStart,
    LoopEnd,
    Call,
    CallMethod,
    LoadCol(OpCode),
    GetProp,
    SetProp,
    SetIndex(OpCode),
    Other(OpCode),
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct MirInst {
    pub idx: usize,
    pub op: MirOp,
    pub result_type: ScalarType,
}

#[derive(Debug, Clone, Default)]
pub struct TypedMirFunction {
    pub instructions: Vec<MirInst>,
    pub numeric_locals: HashSet<u16>,
    pub numeric_module_bindings: HashSet<u16>,
    pub typed_column_reads: HashSet<usize>,
}

fn is_typed_int_arith(op: OpCode) -> bool {
    matches!(
        op,
        OpCode::AddInt
            | OpCode::SubInt
            | OpCode::MulInt
            | OpCode::DivInt
            | OpCode::ModInt
            | OpCode::PowInt
    )
}

fn is_typed_float_arith(op: OpCode) -> bool {
    matches!(
        op,
        OpCode::AddNumber
            | OpCode::SubNumber
            | OpCode::MulNumber
            | OpCode::DivNumber
            | OpCode::ModNumber
            | OpCode::PowNumber
    )
}

fn is_generic_numeric(op: OpCode) -> bool {
    matches!(
        op,
        OpCode::Add
            | OpCode::Sub
            | OpCode::Mul
            | OpCode::Div
            | OpCode::Mod
            | OpCode::Pow
            | OpCode::IntToNumber
            | OpCode::NumberToInt
            | OpCode::Neg
    )
}

fn is_comparison(op: OpCode) -> bool {
    matches!(
        op,
        OpCode::Gt
            | OpCode::Lt
            | OpCode::Gte
            | OpCode::Lte
            | OpCode::EqDynamic
            | OpCode::NeqDynamic
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
            | OpCode::NeqNumber
            | OpCode::EqString
            | OpCode::GtString
            | OpCode::LtString
            | OpCode::GteString
            | OpCode::LteString
            | OpCode::EqDecimal
    )
}

fn const_type(program: &BytecodeProgram, operand: &Option<Operand>) -> ScalarType {
    match operand {
        Some(Operand::Const(idx)) => match program.constants.get(*idx as usize) {
            Some(Constant::Int(_)) | Some(Constant::UInt(_)) => ScalarType::I64,
            Some(Constant::Number(_)) => ScalarType::F64,
            Some(Constant::Bool(_)) => ScalarType::Bool,
            _ => ScalarType::Boxed,
        },
        _ => ScalarType::Unknown,
    }
}

pub fn build_typed_mir(program: &BytecodeProgram) -> TypedMirFunction {
    let mut local_types: HashMap<u16, ScalarType> = HashMap::new();
    let mut module_types: HashMap<u16, ScalarType> = HashMap::new();
    let mut stack: Vec<ScalarType> = Vec::new();

    let mut out = TypedMirFunction::default();

    for (idx, instr) in program.instructions.iter().enumerate() {
        let op = instr.opcode;
        let mut result_type = ScalarType::Unknown;
        let mir_op = match (op, &instr.operand) {
            (OpCode::LoadLocal, Some(Operand::Local(local))) => {
                result_type = *local_types.get(local).unwrap_or(&ScalarType::Unknown);
                stack.push(result_type);
                MirOp::LoadLocal(*local)
            }
            (OpCode::StoreLocal, Some(Operand::Local(local)))
            | (OpCode::StoreLocalTyped, Some(Operand::TypedLocal(local, _))) => {
                let ty = stack.pop().unwrap_or(ScalarType::Unknown);
                local_types.insert(*local, ty);
                if matches!(ty, ScalarType::I64 | ScalarType::F64) {
                    out.numeric_locals.insert(*local);
                }
                MirOp::StoreLocal(*local)
            }
            (OpCode::LoadModuleBinding, Some(Operand::ModuleBinding(binding))) => {
                result_type = *module_types.get(binding).unwrap_or(&ScalarType::Unknown);
                stack.push(result_type);
                MirOp::LoadModuleBinding(*binding)
            }
            (OpCode::StoreModuleBinding, Some(Operand::ModuleBinding(binding)))
            | (OpCode::StoreModuleBindingTyped, Some(Operand::TypedModuleBinding(binding, _))) => {
                let ty = stack.pop().unwrap_or(ScalarType::Unknown);
                module_types.insert(*binding, ty);
                if matches!(ty, ScalarType::I64 | ScalarType::F64) {
                    out.numeric_module_bindings.insert(*binding);
                }
                MirOp::StoreModuleBinding(*binding)
            }
            (OpCode::PushConst, _) => {
                result_type = const_type(program, &instr.operand);
                stack.push(result_type);
                MirOp::PushConst
            }
            (OpCode::LoopStart, _) => MirOp::LoopStart,
            (OpCode::LoopEnd, _) => MirOp::LoopEnd,
            (OpCode::Call, _) | (OpCode::CallValue, _) => {
                stack.clear();
                MirOp::Call
            }
            (OpCode::CallMethod, _) => {
                stack.clear();
                MirOp::CallMethod
            }
            (
                OpCode::LoadColF64 | OpCode::LoadColI64 | OpCode::LoadColBool | OpCode::LoadColStr,
                _,
            ) => {
                out.typed_column_reads.insert(idx);
                result_type = match op {
                    OpCode::LoadColF64 => ScalarType::F64,
                    OpCode::LoadColI64 => ScalarType::I64,
                    OpCode::LoadColBool => ScalarType::Bool,
                    _ => ScalarType::Boxed,
                };
                stack.push(result_type);
                MirOp::LoadCol(op)
            }
            (OpCode::GetProp, _) => {
                stack.pop();
                stack.pop();
                result_type = ScalarType::Boxed;
                stack.push(result_type);
                MirOp::GetProp
            }
            (OpCode::SetProp, _) => {
                stack.pop();
                stack.pop();
                stack.pop();
                MirOp::SetProp
            }
            (OpCode::SetLocalIndex | OpCode::SetModuleBindingIndex | OpCode::SetIndexRef, _) => {
                stack.pop();
                stack.pop();
                MirOp::SetIndex(op)
            }
            // Unary typed arithmetic (pops 1, pushes 1)
            (OpCode::NegInt, _) => {
                stack.pop();
                result_type = ScalarType::I64;
                stack.push(result_type);
                MirOp::Arithmetic(op)
            }
            (OpCode::NegNumber, _) => {
                stack.pop();
                result_type = ScalarType::F64;
                stack.push(result_type);
                MirOp::Arithmetic(op)
            }
            // Unary comparison (pops 1, pushes 1)
            (OpCode::IsNull, _) => {
                stack.pop();
                result_type = ScalarType::Bool;
                stack.push(result_type);
                MirOp::Comparison(op)
            }
            _ if is_typed_int_arith(op) => {
                stack.pop();
                stack.pop();
                result_type = ScalarType::I64;
                stack.push(result_type);
                MirOp::Arithmetic(op)
            }
            _ if is_typed_float_arith(op) || is_generic_numeric(op) => {
                stack.pop();
                stack.pop();
                result_type = ScalarType::F64;
                stack.push(result_type);
                MirOp::Arithmetic(op)
            }
            _ if is_comparison(op) => {
                stack.pop();
                stack.pop();
                result_type = ScalarType::Bool;
                stack.push(result_type);
                MirOp::Comparison(op)
            }
            _ => {
                let pops = op.stack_pops() as usize;
                let pushes = op.stack_pushes() as usize;
                for _ in 0..pops.min(stack.len()) {
                    stack.pop();
                }
                for _ in 0..pushes {
                    stack.push(ScalarType::Unknown);
                }
                MirOp::Other(op)
            }
        };

        out.instructions.push(MirInst {
            idx,
            op: mir_op,
            result_type,
        });
    }

    out
}
