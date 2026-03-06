//! Phase 6: call-path planning (inline/direct call heuristics).

use std::collections::{HashMap, HashSet};

use shape_vm::bytecode::{BytecodeProgram, Constant, OpCode, Operand};

use super::loop_lowering::LoopLoweringPlan;

#[derive(Debug, Clone)]
pub struct CallPathPlan {
    /// Call instruction indices that should prefer direct-call lowering.
    pub prefer_direct_call_sites: HashSet<usize>,
    /// Per-call-site parameter local slots that must be restored after a
    /// direct-call argument write into ctx.locals[0..argc).
    pub restore_param_slots_by_call_site: HashMap<usize, Vec<u16>>,
    /// Depth guard for nested inlining.
    pub inline_depth_limit: u8,
}

impl Default for CallPathPlan {
    fn default() -> Self {
        Self {
            prefer_direct_call_sites: HashSet::new(),
            restore_param_slots_by_call_site: HashMap::new(),
            inline_depth_limit: 4,
        }
    }
}

fn read_arg_count_from_prev(program: &BytecodeProgram, idx: usize) -> Option<usize> {
    if idx == 0 {
        return None;
    }
    let prev = &program.instructions[idx - 1];
    if prev.opcode != OpCode::PushConst {
        return None;
    }
    let Some(Operand::Const(const_idx)) = prev.operand.as_ref() else {
        return None;
    };
    match program.constants.get(*const_idx as usize) {
        Some(Constant::Int(v)) => Some((*v).max(0) as usize),
        Some(Constant::UInt(v)) => Some(*v as usize),
        Some(Constant::Number(v)) if *v >= 0.0 => Some(*v as usize),
        _ => None,
    }
}

fn call_is_inside_hot_loop(idx: usize, loops: &HashMap<usize, LoopLoweringPlan>) -> bool {
    loops
        .values()
        .any(|l| idx > l.header_idx && idx < l.end_idx && l.unroll_factor > 1)
}

fn local_needs_restore_after(program: &BytecodeProgram, start_idx: usize, local_slot: u16) -> bool {
    for instr in &program.instructions[start_idx..] {
        match (instr.opcode, instr.operand.as_ref()) {
            // Most local reads use SSA vars (`LoadLocal`) and do not observe
            // ctx.locals memory clobbering done by direct-call arg setup.
            // Restore is only needed for opcodes that explicitly read
            // ctx.locals[] memory (closure-local loads).
            (OpCode::LoadClosure, Some(Operand::Local(idx))) if *idx == local_slot => {
                return true;
            }
            (OpCode::StoreLocal, Some(Operand::Local(idx))) if *idx == local_slot => return false,
            (OpCode::StoreLocalTyped, Some(Operand::TypedLocal(idx, _))) if *idx == local_slot => {
                return false;
            }
            _ => {}
        }
    }
    false
}

pub fn analyze_call_path(
    program: &BytecodeProgram,
    loops: &HashMap<usize, LoopLoweringPlan>,
) -> CallPathPlan {
    let mut plan = CallPathPlan::default();
    let mut call_count = 0usize;

    for (idx, instr) in program.instructions.iter().enumerate() {
        if instr.opcode != OpCode::Call {
            continue;
        }
        call_count += 1;
        let argc = read_arg_count_from_prev(program, idx).unwrap_or(0);
        if argc <= 4 || call_is_inside_hot_loop(idx, loops) {
            plan.prefer_direct_call_sites.insert(idx);
        }

        let mut restore_slots = Vec::new();
        let limit = argc.min(64);
        for local_slot in 0..limit {
            let local_slot = local_slot as u16;
            if local_needs_restore_after(program, idx + 1, local_slot) {
                restore_slots.push(local_slot);
            }
        }
        plan.restore_param_slots_by_call_site
            .insert(idx, restore_slots);
    }

    if call_count <= 8 {
        plan.inline_depth_limit = 6;
    }
    plan
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_value::FunctionId;
    use shape_vm::bytecode::{Constant, DebugInfo, Instruction};

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
    fn restore_slots_ignore_ssa_load_local_after_call() {
        let program = make_program(
            vec![
                make_instr(OpCode::PushConst, Some(Operand::Const(0))), // argc = 2
                make_instr(OpCode::Call, Some(Operand::Function(FunctionId(0)))),
                make_instr(OpCode::LoadLocal, Some(Operand::Local(1))), // SSA local read
                make_instr(OpCode::Pop, None),
                make_instr(OpCode::Halt, None),
            ],
            vec![Constant::Int(2)],
        );
        let plan = analyze_call_path(&program, &HashMap::new());
        let slots = plan
            .restore_param_slots_by_call_site
            .get(&1)
            .expect("restore slots for call site");
        assert_eq!(slots, &Vec::<u16>::new());
    }

    #[test]
    fn restore_slots_include_load_closure_reads_after_call() {
        let program = make_program(
            vec![
                make_instr(OpCode::PushConst, Some(Operand::Const(0))), // argc = 2
                make_instr(OpCode::Call, Some(Operand::Function(FunctionId(0)))),
                make_instr(OpCode::LoadClosure, Some(Operand::Local(1))), // ctx.locals read
                make_instr(OpCode::Pop, None),
                make_instr(OpCode::Halt, None),
            ],
            vec![Constant::Int(2)],
        );
        let plan = analyze_call_path(&program, &HashMap::new());
        let slots = plan
            .restore_param_slots_by_call_site
            .get(&1)
            .expect("restore slots for call site");
        assert_eq!(slots, &vec![1]);
    }
}
