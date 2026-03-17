//! Phase 8: HOF (Higher-Order Function) method inlining analysis.
//!
//! Identifies CallMethod sites for array HOF methods (map, filter, reduce, find,
//! some, every, forEach, findIndex) where the callback function_id is statically
//! resolvable from the preceding bytecode. When the callback is a known function,
//! the JIT can emit an inline Cranelift loop instead of routing through FFI.

use std::collections::HashMap;

use shape_value::MethodId;
use shape_vm::bytecode::{BytecodeProgram, OpCode, Operand};

/// Describes a HOF method call site eligible for inlining.
#[derive(Debug, Clone)]
pub struct HofInlineSite {
    /// The callback function_id if statically resolvable from bytecode
    pub callback_fn_id: Option<u16>,
}

/// Plan of HOF inline sites keyed by instruction index.
#[derive(Debug, Clone, Default)]
pub struct HofInlinePlan {
    pub sites: HashMap<usize, HofInlineSite>,
}

/// HOF method IDs we want to inline.
const HOF_METHODS: &[(u16, &str, usize)] = &[
    (MethodId::MAP.0, "map", 1),
    (MethodId::FILTER.0, "filter", 1),
    (MethodId::REDUCE.0, "reduce", 2),
    (MethodId::FIND.0, "find", 1),
    (MethodId::FIND_INDEX.0, "findIndex", 1),
    (MethodId::SOME.0, "some", 1),
    (MethodId::EVERY.0, "every", 1),
    (MethodId::FOR_EACH.0, "forEach", 1),
];

/// Analyze bytecode to find HOF method call sites with statically resolvable callbacks.
pub fn analyze_hof_inline(program: &BytecodeProgram) -> HofInlinePlan {
    let mut plan = HofInlinePlan::default();

    for (idx, instr) in program.instructions.iter().enumerate() {
        if instr.opcode != OpCode::CallMethod {
            continue;
        }

        let Some(Operand::TypedMethodCall {
            method_id,
            arg_count,
            ..
        }) = instr.operand.as_ref()
        else {
            continue;
        };

        // Check if this is a HOF method
        let Some(&(_, _name, _expected_args)) =
            HOF_METHODS.iter().find(|(id, _, _)| *id == *method_id)
        else {
            continue;
        };

        let arg_count = *arg_count as usize;
        if arg_count < 1 {
            continue;
        }

        // Try to resolve the callback function_id from preceding bytecode.
        // The stack layout before CallMethod is:
        //   [..., receiver, arg1, ..., argN, method_name_str, arg_count_num]
        // The callback is arg1, which was pushed before the method_name and arg_count.
        // We look backwards for a PushConst with a Function operand.
        let callback_fn_id = resolve_callback_fn_id(program, idx, arg_count);

        plan.sites.insert(
            idx,
            HofInlineSite {
                callback_fn_id,
            },
        );
    }

    plan
}

/// Look backwards from a CallMethod site to find the callback function_id.
///
/// Stack layout at CallMethod: [..., receiver, callback, (init?), method_name, arg_count]
/// We scan backwards to find the PushConst that pushes the callback function.
fn resolve_callback_fn_id(
    program: &BytecodeProgram,
    call_idx: usize,
    arg_count: usize,
) -> Option<u16> {
    // Walk backwards from the CallMethod instruction.
    // The arg_count_num is pushed just before CallMethod.
    // Before that: method_name_str.
    // Before that: the args (callback is first arg, init is second for reduce).
    // We need to skip arg_count + 2 pushes to find the callback.
    //
    // However, a simpler heuristic: scan backwards for PushConst with Function operand
    // within a reasonable window. The callback is typically very close.
    let search_start = call_idx.saturating_sub(1);
    let search_end = call_idx.saturating_sub(10 + arg_count * 3);

    // Count how many push-like operations we need to skip:
    // - 1 for arg_count_num (PushConst number)
    // - 1 for method_name (PushConst string)
    // - (arg_count - 1) for other args (e.g., reduce's initial value)
    // The next PushConst with Function operand should be the callback.
    let mut pushes_to_skip = 2 + (arg_count - 1); // method_name + arg_count + extra args

    for i in (search_end..=search_start).rev() {
        let instr = &program.instructions[i];
        match instr.opcode {
            OpCode::PushConst => {
                if pushes_to_skip > 0 {
                    pushes_to_skip -= 1;
                    continue;
                }
                // This should be the callback
                if let Some(Operand::Function(fn_id)) = &instr.operand {
                    return Some(fn_id.0);
                }
                // If it's a MakeClosure or other non-function, we can't inline
                return None;
            }
            OpCode::MakeClosure => {
                if pushes_to_skip > 0 {
                    pushes_to_skip -= 1;
                    continue;
                }
                // Closure — can't statically resolve
                return None;
            }
            _ => {}
        }
    }

    None
}
