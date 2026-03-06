//! Phase 4: typed numeric array-site planning.
//!
//! This pass classifies array reads/writes at instruction sites where we can
//! confidently keep numeric values in typed form during lowering.

use std::collections::HashSet;

use shape_vm::bytecode::{BytecodeProgram, Constant, OpCode, Operand};

#[derive(Debug, Clone, Default)]
pub struct NumericArrayPlan {
    /// Dynamic `GetProp` sites that are consumed as integer numerics.
    pub int_get_sites: HashSet<usize>,
    /// Dynamic `GetProp` sites that are consumed as float numerics.
    pub float_get_sites: HashSet<usize>,
    /// Dynamic `GetProp` sites that are consumed as strict booleans.
    pub bool_get_sites: HashSet<usize>,
    /// Indexed write sites whose value expression is numeric.
    pub numeric_set_sites: HashSet<usize>,
    /// Indexed write sites whose value expression is strict boolean.
    pub bool_set_sites: HashSet<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NumericKind {
    Int,
    Float,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GetKind {
    Int,
    Float,
    Bool,
}

#[derive(Clone, Copy)]
enum Tracked {
    OnStack(i32),
    InLocal(u16),
    InModuleBinding(u16),
}

fn is_typed_int_consumer(op: OpCode) -> bool {
    matches!(
        op,
        OpCode::AddInt
            | OpCode::SubInt
            | OpCode::MulInt
            | OpCode::DivInt
            | OpCode::ModInt
            | OpCode::PowInt
            | OpCode::AddIntTrusted
            | OpCode::SubIntTrusted
            | OpCode::MulIntTrusted
            | OpCode::DivIntTrusted
            | OpCode::GtInt
            | OpCode::LtInt
            | OpCode::GteInt
            | OpCode::LteInt
            | OpCode::GtIntTrusted
            | OpCode::LtIntTrusted
            | OpCode::GteIntTrusted
            | OpCode::LteIntTrusted
            | OpCode::EqInt
            | OpCode::NeqInt
    )
}

fn is_typed_float_consumer(op: OpCode) -> bool {
    matches!(
        op,
        OpCode::AddNumber
            | OpCode::SubNumber
            | OpCode::MulNumber
            | OpCode::DivNumber
            | OpCode::ModNumber
            | OpCode::PowNumber
            | OpCode::AddNumberTrusted
            | OpCode::SubNumberTrusted
            | OpCode::MulNumberTrusted
            | OpCode::DivNumberTrusted
            | OpCode::GtNumber
            | OpCode::LtNumber
            | OpCode::GteNumber
            | OpCode::LteNumber
            | OpCode::GtNumberTrusted
            | OpCode::LtNumberTrusted
            | OpCode::GteNumberTrusted
            | OpCode::LteNumberTrusted
            | OpCode::EqNumber
            | OpCode::NeqNumber
    )
}

fn is_generic_numeric_consumer(op: OpCode) -> bool {
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
            | OpCode::Gt
            | OpCode::Lt
            | OpCode::Gte
            | OpCode::Lte
            | OpCode::Eq
            | OpCode::Neq
    )
}

fn is_comparison_consumer(op: OpCode) -> bool {
    matches!(
        op,
        OpCode::Gt
            | OpCode::Lt
            | OpCode::Gte
            | OpCode::Lte
            | OpCode::Eq
            | OpCode::Neq
            | OpCode::GtInt
            | OpCode::LtInt
            | OpCode::GteInt
            | OpCode::LteInt
            | OpCode::GtIntTrusted
            | OpCode::LtIntTrusted
            | OpCode::GteIntTrusted
            | OpCode::LteIntTrusted
            | OpCode::EqInt
            | OpCode::NeqInt
            | OpCode::GtNumber
            | OpCode::LtNumber
            | OpCode::GteNumber
            | OpCode::LteNumber
            | OpCode::GtNumberTrusted
            | OpCode::LtNumberTrusted
            | OpCode::GteNumberTrusted
            | OpCode::LteNumberTrusted
            | OpCode::EqNumber
            | OpCode::NeqNumber
            | OpCode::GtDecimal
            | OpCode::LtDecimal
            | OpCode::GteDecimal
            | OpCode::LteDecimal
    )
}

fn is_unknown_stack_effect(op: OpCode) -> bool {
    matches!(
        op,
        OpCode::Call
            | OpCode::CallValue
            | OpCode::CallMethod
            | OpCode::BuiltinCall
            | OpCode::Pattern
            | OpCode::RunSimulation
            | OpCode::DynMethodCall
            | OpCode::CallForeign
    )
}

fn stack_effect(op: OpCode) -> Option<(i32, i32)> {
    let eff = match op {
        OpCode::LoadLocal
        | OpCode::LoadLocalTrusted
        | OpCode::LoadModuleBinding
        | OpCode::LoadClosure
        | OpCode::PushConst
        | OpCode::PushNull
        | OpCode::DerefLoad => (0, 1),
        OpCode::IntToNumber
        | OpCode::NumberToInt
        | OpCode::CastWidth
        | OpCode::Neg
        | OpCode::Not
        | OpCode::Length => (1, 1),
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
        | OpCode::AddIntTrusted
        | OpCode::SubIntTrusted
        | OpCode::MulIntTrusted
        | OpCode::DivIntTrusted
        | OpCode::AddNumber
        | OpCode::SubNumber
        | OpCode::MulNumber
        | OpCode::DivNumber
        | OpCode::ModNumber
        | OpCode::PowNumber
        | OpCode::AddNumberTrusted
        | OpCode::SubNumberTrusted
        | OpCode::MulNumberTrusted
        | OpCode::DivNumberTrusted
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
        | OpCode::GtIntTrusted
        | OpCode::LtIntTrusted
        | OpCode::GteIntTrusted
        | OpCode::LteIntTrusted
        | OpCode::GtNumber
        | OpCode::LtNumber
        | OpCode::GteNumber
        | OpCode::LteNumber
        | OpCode::GtNumberTrusted
        | OpCode::LtNumberTrusted
        | OpCode::GteNumberTrusted
        | OpCode::LteNumberTrusted
        | OpCode::EqInt
        | OpCode::EqNumber
        | OpCode::NeqInt
        | OpCode::NeqNumber
        | OpCode::GetProp => (2, 1),
        OpCode::Dup => (1, 2),
        OpCode::Swap => (2, 2),
        OpCode::Pop
        | OpCode::StoreLocal
        | OpCode::StoreLocalTyped
        | OpCode::StoreModuleBinding
        | OpCode::StoreClosure
        | OpCode::DerefStore
        | OpCode::DropCall
        | OpCode::DropCallAsync => (1, 0),
        _ => return None,
    };
    Some(eff)
}

fn producer_index_for_stack_pos(
    program: &BytecodeProgram,
    before_idx: usize,
    mut pos_from_top: i32,
) -> Option<usize> {
    for j in (0..before_idx).rev() {
        let (pops, pushes) = stack_effect(program.instructions[j].opcode)?;
        if pos_from_top < pushes {
            return Some(j);
        }
        pos_from_top = pos_from_top - pushes + pops;
        if pos_from_top < 0 {
            return None;
        }
    }
    None
}

fn local_init_kind(
    program: &BytecodeProgram,
    before_idx: usize,
    local_idx: u16,
) -> Option<NumericKind> {
    for i in (0..before_idx).rev() {
        let instr = &program.instructions[i];
        let is_store_to_local = matches!(
            (instr.opcode, instr.operand.as_ref()),
            (OpCode::StoreLocal, Some(Operand::Local(idx))) if *idx == local_idx
        ) || matches!(
            (instr.opcode, instr.operand.as_ref()),
            (OpCode::StoreLocalTyped, Some(Operand::TypedLocal(idx, _))) if *idx == local_idx
        );
        if !is_store_to_local {
            continue;
        }
        if i == 0 {
            return None;
        }
        let prev = &program.instructions[i - 1];
        return match prev.opcode {
            OpCode::PushConst => match prev.operand.as_ref() {
                Some(Operand::Const(const_idx)) => match program.constants.get(*const_idx as usize)
                {
                    Some(Constant::Int(_)) | Some(Constant::UInt(_)) => Some(NumericKind::Int),
                    Some(Constant::Number(_)) => Some(NumericKind::Float),
                    _ => None,
                },
                _ => None,
            },
            OpCode::AddInt
            | OpCode::SubInt
            | OpCode::MulInt
            | OpCode::DivInt
            | OpCode::ModInt
            | OpCode::AddIntTrusted
            | OpCode::SubIntTrusted
            | OpCode::MulIntTrusted
            | OpCode::DivIntTrusted => Some(NumericKind::Int),
            OpCode::AddNumber
            | OpCode::SubNumber
            | OpCode::MulNumber
            | OpCode::DivNumber
            | OpCode::ModNumber
            | OpCode::AddNumberTrusted
            | OpCode::SubNumberTrusted
            | OpCode::MulNumberTrusted
            | OpCode::DivNumberTrusted => Some(NumericKind::Float),
            _ => None,
        };
    }
    None
}

fn local_init_bool(program: &BytecodeProgram, before_idx: usize, local_idx: u16) -> bool {
    for i in (0..before_idx).rev() {
        let instr = &program.instructions[i];
        let is_store_to_local = matches!(
            (instr.opcode, instr.operand.as_ref()),
            (OpCode::StoreLocal, Some(Operand::Local(idx))) if *idx == local_idx
        ) || matches!(
            (instr.opcode, instr.operand.as_ref()),
            (OpCode::StoreLocalTyped, Some(Operand::TypedLocal(idx, _))) if *idx == local_idx
        );
        if !is_store_to_local {
            continue;
        }
        if i == 0 {
            return false;
        }
        let prev = &program.instructions[i - 1];
        return matches!(
            (prev.opcode, prev.operand.as_ref()),
            (OpCode::PushConst, Some(Operand::Const(const_idx)))
                if matches!(program.constants.get(*const_idx as usize), Some(Constant::Bool(_)))
        );
    }
    false
}

fn generic_consumer_kind(program: &BytecodeProgram, op_idx: usize) -> NumericKind {
    // Generic div/pow preserve floating semantics.
    if matches!(
        program.instructions[op_idx].opcode,
        OpCode::Div | OpCode::Pow
    ) {
        return NumericKind::Float;
    }

    let start = op_idx.saturating_sub(12);
    let mut saw_int = false;
    let mut saw_float = false;

    for j in (start..op_idx).rev() {
        let instr = &program.instructions[j];
        match instr.opcode {
            OpCode::PushConst => {
                if let Some(Operand::Const(const_idx)) = instr.operand.as_ref() {
                    match program.constants.get(*const_idx as usize) {
                        Some(Constant::Int(_)) | Some(Constant::UInt(_)) => saw_int = true,
                        Some(Constant::Number(_)) => saw_float = true,
                        _ => {}
                    }
                }
            }
            OpCode::LoadLocal | OpCode::LoadLocalTrusted => {
                if let Some(Operand::Local(local_idx)) = instr.operand.as_ref() {
                    match local_init_kind(program, j, *local_idx) {
                        Some(NumericKind::Int) => saw_int = true,
                        Some(NumericKind::Float) => saw_float = true,
                        None => {}
                    }
                }
            }
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
            | OpCode::AddIntTrusted
            | OpCode::SubIntTrusted
            | OpCode::MulIntTrusted
            | OpCode::DivIntTrusted
            | OpCode::AddNumber
            | OpCode::SubNumber
            | OpCode::MulNumber
            | OpCode::DivNumber
            | OpCode::ModNumber
            | OpCode::AddNumberTrusted
            | OpCode::SubNumberTrusted
            | OpCode::MulNumberTrusted
            | OpCode::DivNumberTrusted
            | OpCode::LoadModuleBinding
            | OpCode::Dup
            | OpCode::Swap => {}
            _ => break,
        }
    }

    if saw_float {
        NumericKind::Float
    } else if saw_int {
        NumericKind::Int
    } else {
        NumericKind::Float
    }
}

fn producer_is_bool(program: &BytecodeProgram, producer_idx: usize, depth: u8) -> bool {
    if depth == 0 {
        return false;
    }
    let instr = &program.instructions[producer_idx];
    match instr.opcode {
        OpCode::PushConst => match instr.operand.as_ref() {
            Some(Operand::Const(const_idx)) => matches!(
                program.constants.get(*const_idx as usize),
                Some(Constant::Bool(_))
            ),
            _ => false,
        },
        OpCode::Not => true,
        OpCode::LoadLocal => match instr.operand.as_ref() {
            Some(Operand::Local(local_idx)) => local_init_bool(program, producer_idx, *local_idx),
            _ => false,
        },
        OpCode::Dup => {
            let Some(src_idx) = producer_index_for_stack_pos(program, producer_idx, 0) else {
                return false;
            };
            producer_is_bool(program, src_idx, depth - 1)
        }
        op if is_comparison_consumer(op) => true,
        _ => false,
    }
}

fn value_producer_is_bool(program: &BytecodeProgram, instr_idx: usize) -> bool {
    let Some(value_src_idx) = producer_index_for_stack_pos(program, instr_idx, 0) else {
        return false;
    };
    producer_is_bool(program, value_src_idx, 5)
}

fn local_array_looks_bool(program: &BytecodeProgram, before_idx: usize, local_idx: u16) -> bool {
    let mut saw_bool_write = false;
    let mut saw_non_bool_write = false;

    for i in 0..before_idx {
        let instr = &program.instructions[i];
        match (instr.opcode, instr.operand.as_ref()) {
            (OpCode::ArrayPushLocal, Some(Operand::Local(idx))) if *idx == local_idx => {
                if value_producer_is_bool(program, i) {
                    saw_bool_write = true;
                } else {
                    saw_non_bool_write = true;
                }
            }
            (OpCode::SetLocalIndex, Some(Operand::Local(idx))) if *idx == local_idx => {
                if value_producer_is_bool(program, i) {
                    saw_bool_write = true;
                } else {
                    saw_non_bool_write = true;
                }
            }
            (OpCode::StoreLocal, Some(Operand::Local(idx)))
            | (OpCode::StoreLocalTyped, Some(Operand::TypedLocal(idx, _)))
                if *idx == local_idx =>
            {
                if i == 0 {
                    saw_non_bool_write = true;
                } else {
                    let prev = &program.instructions[i - 1];
                    if prev.opcode != OpCode::NewArray {
                        saw_non_bool_write = true;
                    }
                }
            }
            _ => {}
        }

        if saw_non_bool_write {
            return false;
        }
    }

    saw_bool_write
}

fn module_binding_array_looks_bool(
    program: &BytecodeProgram,
    before_idx: usize,
    binding_idx: u16,
) -> bool {
    let mut saw_bool_write = false;
    let mut saw_non_bool_write = false;

    for i in 0..before_idx {
        let instr = &program.instructions[i];
        match (instr.opcode, instr.operand.as_ref()) {
            (OpCode::ArrayPushLocal, Some(Operand::ModuleBinding(idx))) if *idx == binding_idx => {
                if value_producer_is_bool(program, i) {
                    saw_bool_write = true;
                } else {
                    saw_non_bool_write = true;
                }
            }
            (OpCode::SetModuleBindingIndex, Some(Operand::ModuleBinding(idx)))
                if *idx == binding_idx =>
            {
                if value_producer_is_bool(program, i) {
                    saw_bool_write = true;
                } else {
                    saw_non_bool_write = true;
                }
            }
            (OpCode::StoreModuleBinding, Some(Operand::ModuleBinding(idx)))
                if *idx == binding_idx =>
            {
                if i == 0 {
                    saw_non_bool_write = true;
                } else {
                    let prev = &program.instructions[i - 1];
                    if prev.opcode != OpCode::NewArray {
                        saw_non_bool_write = true;
                    }
                }
            }
            _ => {}
        }

        if saw_non_bool_write {
            return false;
        }
    }

    saw_bool_write
}

fn get_site_looks_bool(program: &BytecodeProgram, get_idx: usize) -> bool {
    // Dynamic GetProp stack shape: [..., obj, key]
    let Some(obj_src_idx) = producer_index_for_stack_pos(program, get_idx, 1) else {
        return false;
    };
    let obj_src = &program.instructions[obj_src_idx];
    match (obj_src.opcode, obj_src.operand.as_ref()) {
        (OpCode::LoadLocal, Some(Operand::Local(local_idx))) => {
            local_array_looks_bool(program, get_idx, *local_idx)
        }
        (OpCode::LoadModuleBinding, Some(Operand::ModuleBinding(binding_idx))) => {
            module_binding_array_looks_bool(program, get_idx, *binding_idx)
        }
        _ => false,
    }
}

fn classify_get_site(program: &BytecodeProgram, get_idx: usize) -> Option<GetKind> {
    let mut tracked = Tracked::OnStack(0);
    let start = get_idx.saturating_add(1);
    let end = (start + 28).min(program.instructions.len());

    for j in start..end {
        let instr = &program.instructions[j];
        let op = instr.opcode;

        tracked = match tracked {
            Tracked::InLocal(local_idx) => match (op, instr.operand.as_ref()) {
                (OpCode::LoadLocal, Some(Operand::Local(idx))) if *idx == local_idx => {
                    Tracked::OnStack(0)
                }
                (OpCode::StoreLocal, Some(Operand::Local(idx))) if *idx == local_idx => {
                    return None;
                }
                (OpCode::StoreLocalTyped, Some(Operand::TypedLocal(idx, _)))
                    if *idx == local_idx =>
                {
                    return None;
                }
                _ => Tracked::InLocal(local_idx),
            },
            Tracked::InModuleBinding(binding_idx) => match (op, instr.operand.as_ref()) {
                (OpCode::LoadModuleBinding, Some(Operand::ModuleBinding(idx)))
                    if *idx == binding_idx =>
                {
                    Tracked::OnStack(0)
                }
                (OpCode::StoreModuleBinding, Some(Operand::ModuleBinding(idx)))
                    if *idx == binding_idx =>
                {
                    return None;
                }
                _ => Tracked::InModuleBinding(binding_idx),
            },
            Tracked::OnStack(mut depth_from_top) => {
                if is_unknown_stack_effect(op) {
                    return None;
                }

                let pops = op.stack_pops() as i32;
                let pushes = op.stack_pushes() as i32;

                if depth_from_top < pops {
                    if matches!(
                        op,
                        OpCode::JumpIfFalse | OpCode::JumpIfFalseTrusted | OpCode::JumpIfTrue
                    ) && get_site_looks_bool(program, get_idx)
                    {
                        return Some(GetKind::Bool);
                    }
                    if is_typed_int_consumer(op) {
                        return Some(GetKind::Int);
                    }
                    if is_typed_float_consumer(op) {
                        return Some(GetKind::Float);
                    }
                    if is_generic_numeric_consumer(op) {
                        return Some(match generic_consumer_kind(program, j) {
                            NumericKind::Int => GetKind::Int,
                            NumericKind::Float => GetKind::Float,
                        });
                    }
                    match (op, instr.operand.as_ref()) {
                        (OpCode::StoreLocal, Some(Operand::Local(idx))) => Tracked::InLocal(*idx),
                        (OpCode::StoreLocalTyped, Some(Operand::TypedLocal(idx, _))) => {
                            Tracked::InLocal(*idx)
                        }
                        (OpCode::StoreModuleBinding, Some(Operand::ModuleBinding(idx))) => {
                            Tracked::InModuleBinding(*idx)
                        }
                        (OpCode::Dup, _) => Tracked::OnStack(0),
                        (OpCode::Swap, _) => Tracked::OnStack(1),
                        _ => return None,
                    }
                } else {
                    depth_from_top = depth_from_top - pops + pushes;
                    Tracked::OnStack(depth_from_top)
                }
            }
        };
    }

    None
}

fn producer_is_numeric(program: &BytecodeProgram, producer_idx: usize, depth: u8) -> bool {
    if depth == 0 {
        return false;
    }
    let instr = &program.instructions[producer_idx];
    match instr.opcode {
        OpCode::PushConst => match instr.operand.as_ref() {
            Some(Operand::Const(const_idx)) => matches!(
                program.constants.get(*const_idx as usize),
                Some(Constant::Int(_)) | Some(Constant::UInt(_)) | Some(Constant::Number(_))
            ),
            _ => false,
        },
        OpCode::LoadLocal => match instr.operand.as_ref() {
            Some(Operand::Local(local_idx)) => {
                local_init_kind(program, producer_idx, *local_idx).is_some()
            }
            _ => false,
        },
        OpCode::Dup => {
            let Some(src_idx) = producer_index_for_stack_pos(program, producer_idx, 0) else {
                return false;
            };
            producer_is_numeric(program, src_idx, depth - 1)
        }
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
        | OpCode::IntToNumber
        | OpCode::NumberToInt
        | OpCode::Neg => true,
        _ => false,
    }
}

fn value_producer_is_numeric(program: &BytecodeProgram, set_idx: usize) -> bool {
    // For indexed writes, top-of-stack is value, second is key.
    let Some(value_src_idx) = producer_index_for_stack_pos(program, set_idx, 0) else {
        return false;
    };
    producer_is_numeric(program, value_src_idx, 5)
}

pub fn analyze_numeric_arrays(
    program: &BytecodeProgram,
    trusted_get_indices: &HashSet<usize>,
    non_negative_get_indices: &HashSet<usize>,
    trusted_set_indices: &HashSet<usize>,
    non_negative_set_indices: &HashSet<usize>,
) -> NumericArrayPlan {
    let mut plan = NumericArrayPlan::default();

    for (idx, instr) in program.instructions.iter().enumerate() {
        match instr.opcode {
            OpCode::GetProp if instr.operand.is_none() => {
                let index_proven =
                    trusted_get_indices.contains(&idx) || non_negative_get_indices.contains(&idx);
                match classify_get_site(program, idx) {
                    Some(GetKind::Int) => {
                        if index_proven {
                            plan.int_get_sites.insert(idx);
                        }
                    }
                    Some(GetKind::Float) => {
                        if index_proven {
                            plan.float_get_sites.insert(idx);
                        }
                    }
                    Some(GetKind::Bool) => {
                        plan.bool_get_sites.insert(idx);
                    }
                    None => {}
                }
            }
            OpCode::SetLocalIndex | OpCode::SetModuleBindingIndex | OpCode::SetIndexRef => {
                let index_proven =
                    trusted_set_indices.contains(&idx) || non_negative_set_indices.contains(&idx);
                let bool_value = value_producer_is_bool(program, idx);
                let numeric_value = value_producer_is_numeric(program, idx);
                if bool_value {
                    // Bool set lowering has checked/non-negative/trusted variants;
                    // it does not require proven indices to stay safe.
                    plan.bool_set_sites.insert(idx);
                } else if index_proven && numeric_value {
                    plan.numeric_set_sites.insert(idx);
                }
            }
            _ => {}
        }
    }

    plan
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_vm::bytecode::{DebugInfo, Instruction};

    fn make_instr(opcode: OpCode, operand: Option<Operand>) -> Instruction {
        Instruction { opcode, operand }
    }

    fn make_program(instrs: Vec<Instruction>, constants: Vec<Constant>) -> BytecodeProgram {
        BytecodeProgram {
            instructions: instrs,
            constants,
            strings: vec![],
            functions: vec![],
            debug_info: DebugInfo::default(),
            data_schema: None,
            module_binding_names: vec![],
            top_level_locals_count: 0,
            top_level_local_storage_hints: vec![],
            type_schema_registry: Default::default(),
            module_binding_storage_hints: vec![],
            function_local_storage_hints: vec![],
            compiled_annotations: Default::default(),
            trait_method_symbols: Default::default(),
            expanded_function_defs: Default::default(),
            string_index: Default::default(),
            foreign_functions: vec![],
            native_struct_layouts: vec![],
            content_addressed: None,
            function_blob_hashes: vec![],
            top_level_frame: None,
            ..Default::default()
        }
    }

    #[test]
    fn classifies_int_get_when_consumed_by_typed_int_op() {
        let program = make_program(
            vec![
                make_instr(OpCode::LoadLocal, Some(Operand::Local(0))),
                make_instr(OpCode::LoadLocal, Some(Operand::Local(1))),
                make_instr(OpCode::GetProp, None), // idx 2
                make_instr(OpCode::PushConst, Some(Operand::Const(0))),
                make_instr(OpCode::AddInt, None),
                make_instr(OpCode::Pop, None),
            ],
            vec![Constant::Int(1)],
        );
        let mut trusted_get = HashSet::new();
        trusted_get.insert(2usize);
        let plan = analyze_numeric_arrays(
            &program,
            &trusted_get,
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        );
        assert!(plan.int_get_sites.contains(&2));
        assert!(!plan.float_get_sites.contains(&2));
    }

    #[test]
    fn classifies_float_get_when_consumed_by_generic_with_float_const() {
        let program = make_program(
            vec![
                make_instr(OpCode::LoadLocal, Some(Operand::Local(0))),
                make_instr(OpCode::LoadLocal, Some(Operand::Local(1))),
                make_instr(OpCode::GetProp, None), // idx 2
                make_instr(OpCode::PushConst, Some(Operand::Const(0))),
                make_instr(OpCode::Mul, None),
                make_instr(OpCode::Pop, None),
            ],
            vec![Constant::Number(2.5)],
        );
        let mut trusted_get = HashSet::new();
        trusted_get.insert(2usize);
        let plan = analyze_numeric_arrays(
            &program,
            &trusted_get,
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        );
        assert!(plan.float_get_sites.contains(&2));
    }

    #[test]
    fn marks_numeric_set_sites_when_value_is_numeric() {
        let program = make_program(
            vec![
                make_instr(OpCode::LoadLocal, Some(Operand::Local(1))), // key
                make_instr(OpCode::PushConst, Some(Operand::Const(0))), // value
                make_instr(OpCode::SetLocalIndex, Some(Operand::Local(0))), // idx 2
            ],
            vec![Constant::Int(7)],
        );
        let mut trusted_set = HashSet::new();
        trusted_set.insert(2usize);
        let plan = analyze_numeric_arrays(
            &program,
            &HashSet::new(),
            &HashSet::new(),
            &trusted_set,
            &HashSet::new(),
        );
        assert!(plan.numeric_set_sites.contains(&2));
    }

    #[test]
    fn classifies_bool_get_for_bool_array_predicate() {
        let program = make_program(
            vec![
                make_instr(OpCode::NewArray, Some(Operand::Count(0))),
                make_instr(OpCode::StoreLocal, Some(Operand::Local(0))),
                make_instr(OpCode::PushConst, Some(Operand::Const(0))), // true
                make_instr(OpCode::ArrayPushLocal, Some(Operand::Local(0))),
                make_instr(OpCode::LoadLocal, Some(Operand::Local(0))),
                make_instr(OpCode::PushConst, Some(Operand::Const(1))), // index
                make_instr(OpCode::GetProp, None),                      // idx 6
                make_instr(OpCode::JumpIfFalse, Some(Operand::Offset(1))),
                make_instr(OpCode::Pop, None),
            ],
            vec![Constant::Bool(true), Constant::Int(0)],
        );
        let mut trusted_get = HashSet::new();
        trusted_get.insert(6usize);
        let plan = analyze_numeric_arrays(
            &program,
            &trusted_get,
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        );
        assert!(plan.bool_get_sites.contains(&6));
        assert!(!plan.int_get_sites.contains(&6));
        assert!(!plan.float_get_sites.contains(&6));
    }

    #[test]
    fn classifies_bool_get_without_index_proof() {
        let program = make_program(
            vec![
                make_instr(OpCode::NewArray, Some(Operand::Count(0))),
                make_instr(OpCode::StoreLocal, Some(Operand::Local(0))),
                make_instr(OpCode::PushConst, Some(Operand::Const(0))), // true
                make_instr(OpCode::ArrayPushLocal, Some(Operand::Local(0))),
                make_instr(OpCode::LoadLocal, Some(Operand::Local(0))),
                make_instr(OpCode::PushConst, Some(Operand::Const(1))), // index
                make_instr(OpCode::GetProp, None),                      // idx 6
                make_instr(OpCode::JumpIfFalse, Some(Operand::Offset(1))),
                make_instr(OpCode::Pop, None),
            ],
            vec![Constant::Bool(true), Constant::Int(0)],
        );
        let plan = analyze_numeric_arrays(
            &program,
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        );
        assert!(plan.bool_get_sites.contains(&6));
        assert!(!plan.int_get_sites.contains(&6));
        assert!(!plan.float_get_sites.contains(&6));
    }

    #[test]
    fn marks_bool_set_sites_when_value_is_bool() {
        let program = make_program(
            vec![
                make_instr(OpCode::LoadLocal, Some(Operand::Local(1))), // key
                make_instr(OpCode::PushConst, Some(Operand::Const(0))), // value=true
                make_instr(OpCode::SetLocalIndex, Some(Operand::Local(0))), // idx 2
            ],
            vec![Constant::Bool(true)],
        );
        let mut trusted_set = HashSet::new();
        trusted_set.insert(2usize);
        let plan = analyze_numeric_arrays(
            &program,
            &HashSet::new(),
            &HashSet::new(),
            &trusted_set,
            &HashSet::new(),
        );
        assert!(plan.bool_set_sites.contains(&2));
        assert!(!plan.numeric_set_sites.contains(&2));
    }

    #[test]
    fn marks_bool_set_sites_without_index_proof() {
        let program = make_program(
            vec![
                make_instr(OpCode::LoadLocal, Some(Operand::Local(1))), // key
                make_instr(OpCode::PushConst, Some(Operand::Const(0))), // value=true
                make_instr(OpCode::SetLocalIndex, Some(Operand::Local(0))), // idx 2
            ],
            vec![Constant::Bool(true)],
        );
        let plan = analyze_numeric_arrays(
            &program,
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        );
        assert!(plan.bool_set_sites.contains(&2));
    }

    #[test]
    fn marks_bool_set_sites_when_value_passes_through_dup_storelocal() {
        let program = make_program(
            vec![
                make_instr(OpCode::LoadLocal, Some(Operand::Local(1))), // key
                make_instr(OpCode::PushConst, Some(Operand::Const(0))), // value=false
                make_instr(OpCode::Dup, None),                          // keep one value for set
                make_instr(OpCode::StoreLocal, Some(Operand::Local(2))), // consume duplicate
                make_instr(OpCode::SetIndexRef, Some(Operand::Local(3))), // idx 4
            ],
            vec![Constant::Bool(false)],
        );
        let mut trusted_set = HashSet::new();
        trusted_set.insert(4usize);
        let plan = analyze_numeric_arrays(
            &program,
            &HashSet::new(),
            &HashSet::new(),
            &trusted_set,
            &HashSet::new(),
        );
        assert!(plan.bool_set_sites.contains(&4));
    }
}
