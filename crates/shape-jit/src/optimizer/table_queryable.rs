//! Phase 7: typed Table<T>/Queryable<T> planning.

use std::collections::HashSet;

use shape_value::MethodId;
use shape_vm::bytecode::{BytecodeProgram, OpCode, Operand};

#[derive(Debug, Clone, Default)]
pub struct TableQueryablePlan {
    pub typed_column_load_sites: HashSet<usize>,
    pub filter_sites: HashSet<usize>,
    pub map_sites: HashSet<usize>,
    pub count_sites: HashSet<usize>,
    pub limit_sites: HashSet<usize>,
    pub order_by_sites: HashSet<usize>,
}

pub fn analyze_table_queryable(program: &BytecodeProgram) -> TableQueryablePlan {
    let mut plan = TableQueryablePlan::default();

    for (idx, instr) in program.instructions.iter().enumerate() {
        match instr.opcode {
            OpCode::LoadColF64 | OpCode::LoadColI64 | OpCode::LoadColBool | OpCode::LoadColStr => {
                plan.typed_column_load_sites.insert(idx);
            }
            OpCode::CallMethod => {
                let Some(Operand::TypedMethodCall { method_id, .. }) = instr.operand.as_ref()
                else {
                    continue;
                };
                match *method_id {
                    id if id == MethodId::FILTER.0 => {
                        plan.filter_sites.insert(idx);
                    }
                    id if id == MethodId::MAP.0 => {
                        plan.map_sites.insert(idx);
                    }
                    id if id == MethodId::COUNT.0 => {
                        plan.count_sites.insert(idx);
                    }
                    id if id == MethodId::LIMIT.0 || id == MethodId::TAKE.0 => {
                        plan.limit_sites.insert(idx);
                    }
                    id if id == MethodId::ORDER_BY.0 => {
                        plan.order_by_sites.insert(idx);
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    plan
}
