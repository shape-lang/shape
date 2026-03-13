//! Phase 2/4: loop lowering and nested-loop specialization planning.

use std::collections::{HashMap, HashSet};

use shape_vm::bytecode::{BytecodeProgram, OpCode};

use crate::translator::loop_analysis::LoopInfo;

use super::typed_mir::TypedMirFunction;

#[derive(Debug, Clone, Default)]
pub struct LoopLoweringPlan {
    pub header_idx: usize,
    pub end_idx: usize,
    pub canonical_iv: Option<u16>,
    pub bound_slot: Option<u16>,
    pub step_value: Option<i64>,
    pub nested_depth: u16,
    pub unroll_factor: u8,
    pub register_carried_locals: HashSet<u16>,
    pub register_carried_module_bindings: HashSet<u16>,
}

fn compute_nested_depth(loop_header: usize, loops: &HashMap<usize, LoopInfo>) -> u16 {
    let mut depth = 0u16;
    if let Some(target) = loops.get(&loop_header) {
        for (header, other) in loops {
            if *header == loop_header {
                continue;
            }
            if other.header_idx < target.header_idx && other.end_idx > target.end_idx {
                depth = depth.saturating_add(1);
            }
        }
    }
    depth
}

fn estimate_unroll_factor(
    loop_info: &LoopInfo,
    body_numeric_ops: usize,
    body_memory_ops: usize,
) -> u8 {
    let body_len = loop_info.end_idx.saturating_sub(loop_info.header_idx);
    if loop_info.body_can_allocate {
        return 1;
    }
    // Memory-heavy loops (e.g. sieve-style indexed writes) still benefit from
    // modest unrolling even with few arithmetic ops.
    if body_len <= 16 && body_memory_ops >= 1 && body_numeric_ops >= 1 {
        return 4;
    }
    if body_len <= 32 && body_memory_ops >= 1 && body_numeric_ops >= 1 {
        return 2;
    }
    if body_len <= 20 && body_numeric_ops >= 4 {
        return 4;
    }
    // Medium-sized numeric kernels (e.g. spectral inner loops) still benefit
    // from modest unrolling even when their bodies exceed the small-loop window.
    if body_len <= 64 && body_numeric_ops >= 4 {
        return 2;
    }
    if body_len <= 120 && body_numeric_ops >= 8 {
        return 2;
    }
    if body_len <= 40 && body_numeric_ops >= 2 {
        return 2;
    }
    1
}

pub fn plan_loops(
    program: &BytecodeProgram,
    loops: &HashMap<usize, LoopInfo>,
    _typed_mir: &TypedMirFunction,
) -> HashMap<usize, LoopLoweringPlan> {
    let mut out = HashMap::new();

    for (header, info) in loops {
        let mut plan = LoopLoweringPlan {
            header_idx: info.header_idx,
            end_idx: info.end_idx,
            nested_depth: compute_nested_depth(*header, loops),
            ..LoopLoweringPlan::default()
        };

        let preferred_iv = info.induction_vars.iter().find(|iv| {
            !iv.is_module_binding && iv.bound_slot.is_some() && iv.step_value == Some(1)
        });
        let fallback_iv = info
            .induction_vars
            .iter()
            .find(|iv| !iv.is_module_binding && iv.bound_slot.is_some());
        if let Some(iv) = preferred_iv.or(fallback_iv) {
            plan.canonical_iv = Some(iv.local_slot);
            plan.bound_slot = iv.bound_slot;
            plan.step_value = iv.step_value;
        }

        let mut numeric_ops = 0usize;
        let mut memory_ops = 0usize;
        for i in (info.header_idx + 1)..info.end_idx {
            let op = program.instructions[i].opcode;
            if matches!(
                op,
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
            ) {
                numeric_ops += 1;
            }
            if matches!(
                op,
                OpCode::GetProp
                    | OpCode::SetLocalIndex
                    | OpCode::SetModuleBindingIndex
                    | OpCode::SetIndexRef
                    | OpCode::ArrayPushLocal
            ) {
                memory_ops += 1;
            }
        }

        plan.unroll_factor = estimate_unroll_factor(info, numeric_ops, memory_ops);
        if plan.nested_depth > 0 {
            plan.unroll_factor = plan.unroll_factor.min(2);
        }

        for local in &info.body_locals_written {
            if info.body_locals_read.contains(local) {
                plan.register_carried_locals.insert(*local);
            }
        }
        for binding in &info.body_module_bindings_written {
            if info.body_module_bindings_read.contains(binding) {
                plan.register_carried_module_bindings.insert(*binding);
            }
        }

        out.insert(*header, plan);
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use cranelift::prelude::IntCC;
    use shape_vm::bytecode::{BytecodeProgram, DebugInfo, Instruction, Operand};

    use crate::translator::loop_analysis::{InductionVar, LoopInfo};

    fn make_instr(opcode: OpCode, operand: Option<Operand>) -> Instruction {
        Instruction { opcode, operand }
    }

    fn make_program(instrs: Vec<Instruction>) -> BytecodeProgram {
        BytecodeProgram {
            instructions: instrs,
            constants: vec![],
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

    fn make_loop_info(
        header_idx: usize,
        end_idx: usize,
        induction_vars: Vec<InductionVar>,
    ) -> LoopInfo {
        LoopInfo {
            header_idx,
            end_idx,
            body_locals_written: HashSet::new(),
            body_locals_read: HashSet::new(),
            body_module_bindings_written: HashSet::new(),
            body_module_bindings_read: HashSet::new(),
            induction_vars,
            invariant_locals: HashSet::new(),
            invariant_module_bindings: HashSet::new(),
            body_can_allocate: false,
        }
    }

    #[test]
    fn picks_fallback_canonical_iv_when_step_is_not_const_one() {
        let program = make_program(vec![
            make_instr(OpCode::PushNull, None),
            make_instr(OpCode::PushNull, None),
            make_instr(OpCode::PushNull, None),
            make_instr(OpCode::PushNull, None),
            make_instr(OpCode::PushNull, None),
            make_instr(OpCode::PushNull, None),
        ]);
        let loop_info = make_loop_info(
            1,
            5,
            vec![InductionVar {
                local_slot: 4,
                is_module_binding: false,
                bound_cmp: IntCC::SignedLessThan,
                bound_slot: Some(6),
                step_value: None,
            }],
        );
        let mut loops = HashMap::new();
        loops.insert(1usize, loop_info);

        let plans = plan_loops(&program, &loops, &TypedMirFunction::default());
        let plan = plans.get(&1).expect("missing loop plan");
        assert_eq!(plan.canonical_iv, Some(4));
        assert_eq!(plan.bound_slot, Some(6));
        assert_eq!(plan.step_value, None);
    }

    #[test]
    fn prefers_const_one_iv_when_multiple_candidates_exist() {
        let program = make_program(vec![
            make_instr(OpCode::PushNull, None),
            make_instr(OpCode::PushNull, None),
            make_instr(OpCode::PushNull, None),
            make_instr(OpCode::PushNull, None),
            make_instr(OpCode::PushNull, None),
            make_instr(OpCode::PushNull, None),
            make_instr(OpCode::PushNull, None),
        ]);
        let loop_info = make_loop_info(
            1,
            6,
            vec![
                InductionVar {
                    local_slot: 2,
                    is_module_binding: false,
                    bound_cmp: IntCC::SignedLessThan,
                    bound_slot: Some(5),
                    step_value: None,
                },
                InductionVar {
                    local_slot: 3,
                    is_module_binding: false,
                    bound_cmp: IntCC::SignedLessThan,
                    bound_slot: Some(5),
                    step_value: Some(1),
                },
            ],
        );
        let mut loops = HashMap::new();
        loops.insert(1usize, loop_info);

        let plans = plan_loops(&program, &loops, &TypedMirFunction::default());
        let plan = plans.get(&1).expect("missing loop plan");
        assert_eq!(plan.canonical_iv, Some(3));
        assert_eq!(plan.bound_slot, Some(5));
        assert_eq!(plan.step_value, Some(1));
    }
}
