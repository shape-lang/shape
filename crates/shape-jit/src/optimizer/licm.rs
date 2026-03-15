//! Call LICM (Loop-Invariant Code Motion for pure function calls).
//!
//! Identifies pure/hoistable function calls within loops and produces
//! hoisting recommendations. A call is hoistable when:
//!
//! 1. The call target is in the purity whitelist (deterministic, no side effects).
//! 2. All arguments to the call are loop-invariant (defined outside the loop
//!    or are constants).
//!
//! When both conditions are met, the call can be evaluated once in the loop
//! pre-header rather than on every iteration.
//!
//! This pass covers:
//! - Built-in math functions: `sin`, `cos`, `sqrt`, `abs`, `floor`, `ceil`,
//!   `tan`, `asin`, `acos`, `atan`, `exp`, `ln`, `log`, `round`
//! - Matrix/collection methods: `row`, `col`, `transpose`, `shape`, `len`

use std::collections::{HashMap, HashSet};

use shape_vm::bytecode::{BuiltinFunction, BytecodeProgram, OpCode, Operand};

use crate::translator::loop_analysis::LoopInfo;

/// A single hoistable call site within a loop.
#[derive(Debug, Clone)]
pub struct HoistableCall {
    /// Bytecode index of the call instruction (BuiltinCall or CallMethod).
    pub call_idx: usize,
    /// Number of arguments consumed by the call (not counting receiver for methods).
    pub arg_count: usize,
    /// Bytecode index of the first argument push instruction for this call.
    /// Used by the translator to identify the instruction range to hoist.
    pub first_arg_idx: usize,
}

/// LICM plan for the entire function: maps loop header index to hoistable calls.
#[derive(Debug, Clone, Default)]
pub struct LicmPlan {
    /// Hoistable calls keyed by loop header bytecode index.
    pub hoistable_calls_by_loop: HashMap<usize, Vec<HoistableCall>>,
}

/// Returns true if the builtin function is pure (deterministic, no side effects).
fn is_pure_builtin(builtin: &BuiltinFunction) -> bool {
    matches!(
        builtin,
        BuiltinFunction::Sin
            | BuiltinFunction::Cos
            | BuiltinFunction::Tan
            | BuiltinFunction::Asin
            | BuiltinFunction::Acos
            | BuiltinFunction::Atan
            | BuiltinFunction::Sqrt
            | BuiltinFunction::Abs
            | BuiltinFunction::Floor
            | BuiltinFunction::Ceil
            | BuiltinFunction::Round
            | BuiltinFunction::Exp
            | BuiltinFunction::Ln
            | BuiltinFunction::Log
            | BuiltinFunction::Pow
            | BuiltinFunction::Sign
            | BuiltinFunction::Hypot
    )
}

/// Returns true if the method name (looked up from the string pool) is pure.
fn is_pure_method_name(name: &str) -> bool {
    matches!(name, "row" | "col" | "transpose" | "shape" | "len")
}

/// Check if an instruction produces a loop-invariant value.
///
/// An instruction's result is loop-invariant if it:
/// - Loads a constant (`PushConst`)
/// - Loads a local that is not written inside the loop (`LoadLocal`/`LoadLocalTrusted`
///   for invariant locals)
/// - Loads a module binding that is not written inside the loop
fn is_invariant_value_producer(
    instr_idx: usize,
    program: &BytecodeProgram,
    info: &LoopInfo,
) -> bool {
    let instr = &program.instructions[instr_idx];
    match instr.opcode {
        OpCode::PushConst | OpCode::PushNull => true,
        OpCode::LoadLocal | OpCode::LoadLocalTrusted => {
            if let Some(Operand::Local(slot)) = &instr.operand {
                info.invariant_locals.contains(slot)
            } else {
                false
            }
        }
        OpCode::LoadModuleBinding => {
            if let Some(Operand::ModuleBinding(slot)) = &instr.operand {
                info.invariant_module_bindings.contains(slot)
            } else {
                false
            }
        }
        _ => false,
    }
}

/// Analyze a single loop for hoistable pure calls.
fn analyze_loop_calls(
    program: &BytecodeProgram,
    info: &LoopInfo,
) -> Vec<HoistableCall> {
    let mut hoistable = Vec::new();

    // Skip instructions inside nested loops (same approach as loop_analysis.rs).
    let mut nested_depth = 0usize;
    let mut i = info.header_idx + 1;
    while i < info.end_idx {
        let instr = &program.instructions[i];
        match instr.opcode {
            OpCode::LoopStart => {
                nested_depth += 1;
                i += 1;
                continue;
            }
            OpCode::LoopEnd if nested_depth > 0 => {
                nested_depth -= 1;
                i += 1;
                continue;
            }
            _ => {}
        }
        if nested_depth > 0 {
            i += 1;
            continue;
        }

        // Check for BuiltinCall with a pure builtin.
        if instr.opcode == OpCode::BuiltinCall {
            if let Some(Operand::Builtin(builtin)) = &instr.operand {
                if is_pure_builtin(builtin) {
                    if let Some(call) =
                        try_hoist_builtin_call(program, info, i)
                    {
                        hoistable.push(call);
                    }
                }
            }
        }

        // Check for CallMethod with a pure method name.
        if instr.opcode == OpCode::CallMethod {
            match &instr.operand {
                Some(Operand::MethodCall { name, arg_count: _ }) => {
                    let str_idx = name.0 as usize;
                    if let Some(method_name) = program.strings.get(str_idx) {
                        if is_pure_method_name(method_name) {
                            if let Some(call) =
                                try_hoist_method_call(program, info, i)
                            {
                                hoistable.push(call);
                            }
                        }
                    }
                }
                Some(Operand::TypedMethodCall {
                    string_id,
                    arg_count: _,
                    method_id: _,
                }) => {
                    let str_idx = *string_id as usize;
                    if let Some(method_name) = program.strings.get(str_idx) {
                        if is_pure_method_name(method_name) {
                            if let Some(call) =
                                try_hoist_method_call(program, info, i)
                            {
                                hoistable.push(call);
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        i += 1;
    }

    hoistable
}

/// Try to determine if a BuiltinCall at `call_idx` has all loop-invariant arguments.
///
/// Bytecode pattern for builtin calls:
///   arg0_push, arg1_push, ..., PushConst(arg_count), BuiltinCall(builtin)
///
/// We walk backwards from the BuiltinCall to find the PushConst(arg_count),
/// then check that the preceding `arg_count` instructions all produce
/// loop-invariant values.
fn try_hoist_builtin_call(
    program: &BytecodeProgram,
    info: &LoopInfo,
    call_idx: usize,
) -> Option<HoistableCall> {
    // The instruction immediately before the BuiltinCall should be PushConst(arg_count).
    if call_idx == 0 {
        return None;
    }
    let argc_instr = &program.instructions[call_idx - 1];
    if argc_instr.opcode != OpCode::PushConst {
        return None;
    }
    let arg_count = read_const_int(program, &argc_instr.operand)?;
    if arg_count > 8 {
        return None; // Sanity limit
    }

    // The arg_count args are pushed immediately before the PushConst(arg_count).
    let first_arg_idx = (call_idx - 1).checked_sub(arg_count)?;
    if first_arg_idx <= info.header_idx {
        return None; // Args would be outside/at loop header
    }

    // Check each argument is an invariant value producer.
    for j in first_arg_idx..(call_idx - 1) {
        if !is_invariant_value_producer(j, program, info) {
            return None;
        }
    }

    Some(HoistableCall {
        call_idx,
        arg_count,
        first_arg_idx,
    })
}

/// Try to determine if a CallMethod at `call_idx` has all loop-invariant arguments.
///
/// Bytecode pattern for method calls:
///   receiver_push, arg0_push, ..., PushConst(arg_count), CallMethod(name)
///
/// The receiver is counted separately from arg_count. We check that
/// the receiver and all arguments are loop-invariant.
fn try_hoist_method_call(
    program: &BytecodeProgram,
    info: &LoopInfo,
    call_idx: usize,
) -> Option<HoistableCall> {
    // Get arg_count from the operand directly.
    let operand_arg_count = match &program.instructions[call_idx].operand {
        Some(Operand::MethodCall { arg_count, .. }) => *arg_count as usize,
        Some(Operand::TypedMethodCall { arg_count, .. }) => *arg_count as usize,
        _ => return None,
    };

    if operand_arg_count > 8 {
        return None; // Sanity limit
    }

    // The instruction before CallMethod should be PushConst(arg_count).
    if call_idx == 0 {
        return None;
    }
    let argc_instr = &program.instructions[call_idx - 1];
    if argc_instr.opcode != OpCode::PushConst {
        return None;
    }

    // Total values pushed before the PushConst: receiver + args.
    let total_pushes = 1 + operand_arg_count;
    let first_arg_idx = (call_idx - 1).checked_sub(total_pushes)?;
    if first_arg_idx <= info.header_idx {
        return None;
    }

    // Check receiver + all args are invariant value producers.
    for j in first_arg_idx..(call_idx - 1) {
        if !is_invariant_value_producer(j, program, info) {
            return None;
        }
    }

    Some(HoistableCall {
        call_idx,
        arg_count: total_pushes, // receiver + args for the full hoist range
        first_arg_idx,
    })
}

/// Read a small non-negative integer from a PushConst operand.
fn read_const_int(
    program: &BytecodeProgram,
    operand: &Option<Operand>,
) -> Option<usize> {
    let Some(Operand::Const(const_idx)) = operand else {
        return None;
    };
    match program.constants.get(*const_idx as usize) {
        Some(shape_vm::bytecode::Constant::Int(v)) => {
            if *v >= 0 {
                Some(*v as usize)
            } else {
                None
            }
        }
        Some(shape_vm::bytecode::Constant::UInt(v)) => Some(*v as usize),
        Some(shape_vm::bytecode::Constant::Number(v)) if *v >= 0.0 && *v == (*v as usize) as f64 => {
            Some(*v as usize)
        }
        _ => None,
    }
}

/// Analyze all loops in the program for hoistable pure calls.
pub fn analyze_licm(
    program: &BytecodeProgram,
    loop_info: &HashMap<usize, LoopInfo>,
) -> LicmPlan {
    let mut plan = LicmPlan::default();

    for (header, info) in loop_info {
        let calls = analyze_loop_calls(program, info);
        if !calls.is_empty() {
            plan.hoistable_calls_by_loop.insert(*header, calls);
        }
    }

    plan
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_vm::bytecode::*;

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

    fn make_program_with_strings(
        instrs: Vec<Instruction>,
        constants: Vec<Constant>,
        strings: Vec<String>,
    ) -> BytecodeProgram {
        let mut p = make_program(instrs, constants);
        p.strings = strings;
        p
    }

    #[test]
    fn test_pure_builtin_hoistable_single_arg() {
        // Loop with sin(x) where x is loop-invariant:
        //   LoopStart
        //   LoadLocal(0)  // i (IV)
        //   LoadLocal(1)  // n (bound)
        //   LtInt
        //   JumpIfFalse(+6)
        //   LoadLocal(2)  // x (invariant arg)
        //   PushConst(1)  // arg_count = 1
        //   BuiltinCall(Sin)
        //   StoreLocal(3) // result
        //   ...increment i...
        //   LoopEnd
        let instrs = vec![
            make_instr(OpCode::LoopStart, None),                                    // 0
            make_instr(OpCode::LoadLocal, Some(Operand::Local(0))),                  // 1: i
            make_instr(OpCode::LoadLocal, Some(Operand::Local(1))),                  // 2: n
            make_instr(OpCode::LtInt, None),                                         // 3
            make_instr(OpCode::JumpIfFalse, Some(Operand::Offset(8))),               // 4
            make_instr(OpCode::LoadLocal, Some(Operand::Local(2))),                  // 5: x (invariant)
            make_instr(OpCode::PushConst, Some(Operand::Const(0))),                  // 6: argc=1
            make_instr(OpCode::BuiltinCall, Some(Operand::Builtin(BuiltinFunction::Sin))), // 7
            make_instr(OpCode::StoreLocal, Some(Operand::Local(3))),                 // 8
            make_instr(OpCode::LoadLocal, Some(Operand::Local(0))),                  // 9
            make_instr(OpCode::PushConst, Some(Operand::Const(0))),                  // 10
            make_instr(OpCode::AddInt, None),                                        // 11
            make_instr(OpCode::StoreLocal, Some(Operand::Local(0))),                 // 12
            make_instr(OpCode::LoopEnd, None),                                       // 13
        ];

        let program = make_program(instrs, vec![Constant::Int(1)]);
        let loop_info = crate::translator::loop_analysis::analyze_loops(&program);
        let plan = analyze_licm(&program, &loop_info);

        assert!(plan.hoistable_calls_by_loop.contains_key(&0));
        let calls = &plan.hoistable_calls_by_loop[&0];
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].call_idx, 7);
        assert_eq!(calls[0].arg_count, 1);
        assert_eq!(calls[0].first_arg_idx, 5);
    }

    #[test]
    fn test_non_invariant_arg_not_hoisted() {
        // Loop with sin(i) where i is the induction variable (not invariant):
        let instrs = vec![
            make_instr(OpCode::LoopStart, None),                                    // 0
            make_instr(OpCode::LoadLocal, Some(Operand::Local(0))),                  // 1: i
            make_instr(OpCode::LoadLocal, Some(Operand::Local(1))),                  // 2: n
            make_instr(OpCode::LtInt, None),                                         // 3
            make_instr(OpCode::JumpIfFalse, Some(Operand::Offset(8))),               // 4
            make_instr(OpCode::LoadLocal, Some(Operand::Local(0))),                  // 5: i (NOT invariant)
            make_instr(OpCode::PushConst, Some(Operand::Const(0))),                  // 6: argc=1
            make_instr(OpCode::BuiltinCall, Some(Operand::Builtin(BuiltinFunction::Sin))), // 7
            make_instr(OpCode::StoreLocal, Some(Operand::Local(3))),                 // 8
            make_instr(OpCode::LoadLocal, Some(Operand::Local(0))),                  // 9
            make_instr(OpCode::PushConst, Some(Operand::Const(0))),                  // 10
            make_instr(OpCode::AddInt, None),                                        // 11
            make_instr(OpCode::StoreLocal, Some(Operand::Local(0))),                 // 12
            make_instr(OpCode::LoopEnd, None),                                       // 13
        ];

        let program = make_program(instrs, vec![Constant::Int(1)]);
        let loop_info = crate::translator::loop_analysis::analyze_loops(&program);
        let plan = analyze_licm(&program, &loop_info);

        // sin(i) should NOT be hoisted because i is the induction variable
        assert!(
            plan.hoistable_calls_by_loop.get(&0).map_or(true, |c| c.is_empty()),
            "sin(i) should not be hoisted when i is the IV"
        );
    }

    #[test]
    fn test_impure_builtin_not_hoisted() {
        // Loop with print(x) where x is loop-invariant but print is impure:
        let instrs = vec![
            make_instr(OpCode::LoopStart, None),                                    // 0
            make_instr(OpCode::LoadLocal, Some(Operand::Local(0))),                  // 1
            make_instr(OpCode::LoadLocal, Some(Operand::Local(1))),                  // 2
            make_instr(OpCode::LtInt, None),                                         // 3
            make_instr(OpCode::JumpIfFalse, Some(Operand::Offset(8))),               // 4
            make_instr(OpCode::LoadLocal, Some(Operand::Local(2))),                  // 5
            make_instr(OpCode::PushConst, Some(Operand::Const(0))),                  // 6
            make_instr(OpCode::BuiltinCall, Some(Operand::Builtin(BuiltinFunction::Print))), // 7
            make_instr(OpCode::StoreLocal, Some(Operand::Local(3))),                 // 8
            make_instr(OpCode::LoadLocal, Some(Operand::Local(0))),                  // 9
            make_instr(OpCode::PushConst, Some(Operand::Const(0))),                  // 10
            make_instr(OpCode::AddInt, None),                                        // 11
            make_instr(OpCode::StoreLocal, Some(Operand::Local(0))),                 // 12
            make_instr(OpCode::LoopEnd, None),                                       // 13
        ];

        let program = make_program(instrs, vec![Constant::Int(1)]);
        let loop_info = crate::translator::loop_analysis::analyze_loops(&program);
        let plan = analyze_licm(&program, &loop_info);

        assert!(
            plan.hoistable_calls_by_loop.get(&0).map_or(true, |c| c.is_empty()),
            "print() should not be hoisted (impure)"
        );
    }

    #[test]
    fn test_pure_method_call_hoistable() {
        // Loop with matrix.shape() where matrix is loop-invariant:
        //   LoopStart
        //   ...loop condition...
        //   LoadLocal(2)  // matrix (receiver, invariant)
        //   PushConst(0)  // arg_count = 0
        //   CallMethod(MethodCall { name: "shape", arg_count: 0 })
        //   StoreLocal(3)
        //   ...increment...
        //   LoopEnd
        use shape_value::StringId;
        let instrs = vec![
            make_instr(OpCode::LoopStart, None),                                    // 0
            make_instr(OpCode::LoadLocal, Some(Operand::Local(0))),                  // 1
            make_instr(OpCode::LoadLocal, Some(Operand::Local(1))),                  // 2
            make_instr(OpCode::LtInt, None),                                         // 3
            make_instr(OpCode::JumpIfFalse, Some(Operand::Offset(8))),               // 4
            make_instr(OpCode::LoadLocal, Some(Operand::Local(2))),                  // 5: matrix (invariant)
            make_instr(OpCode::PushConst, Some(Operand::Const(1))),                  // 6: argc=0
            make_instr(
                OpCode::CallMethod,
                Some(Operand::MethodCall {
                    name: StringId(0),
                    arg_count: 0,
                }),
            ),                                                                        // 7
            make_instr(OpCode::StoreLocal, Some(Operand::Local(3))),                 // 8
            make_instr(OpCode::LoadLocal, Some(Operand::Local(0))),                  // 9
            make_instr(OpCode::PushConst, Some(Operand::Const(0))),                  // 10
            make_instr(OpCode::AddInt, None),                                        // 11
            make_instr(OpCode::StoreLocal, Some(Operand::Local(0))),                 // 12
            make_instr(OpCode::LoopEnd, None),                                       // 13
        ];

        let program = make_program_with_strings(
            instrs,
            vec![Constant::Int(1), Constant::Int(0)],
            vec!["shape".to_string()],
        );
        let loop_info = crate::translator::loop_analysis::analyze_loops(&program);
        let plan = analyze_licm(&program, &loop_info);

        assert!(plan.hoistable_calls_by_loop.contains_key(&0));
        let calls = &plan.hoistable_calls_by_loop[&0];
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].call_idx, 7);
    }

    #[test]
    fn test_nested_loop_ignores_inner() {
        // Outer loop with sin(x) where x is invariant to outer loop.
        // Inner loop body should not produce LICM candidates for the outer loop.
        let instrs = vec![
            make_instr(OpCode::LoopStart, None),                                    // 0: outer start
            make_instr(OpCode::LoadLocal, Some(Operand::Local(0))),                  // 1
            make_instr(OpCode::LoadLocal, Some(Operand::Local(1))),                  // 2
            make_instr(OpCode::LtInt, None),                                         // 3
            make_instr(OpCode::JumpIfFalse, Some(Operand::Offset(15))),              // 4
            // sin(x) in outer loop body - should be hoistable
            make_instr(OpCode::LoadLocal, Some(Operand::Local(2))),                  // 5
            make_instr(OpCode::PushConst, Some(Operand::Const(0))),                  // 6
            make_instr(OpCode::BuiltinCall, Some(Operand::Builtin(BuiltinFunction::Sin))), // 7
            make_instr(OpCode::StoreLocal, Some(Operand::Local(3))),                 // 8
            // Inner loop
            make_instr(OpCode::LoopStart, None),                                    // 9: inner start
            make_instr(OpCode::LoadLocal, Some(Operand::Local(2))),                  // 10
            make_instr(OpCode::PushConst, Some(Operand::Const(0))),                  // 11
            make_instr(OpCode::BuiltinCall, Some(Operand::Builtin(BuiltinFunction::Cos))), // 12
            make_instr(OpCode::Pop, None),                                           // 13
            make_instr(OpCode::LoopEnd, None),                                       // 14: inner end
            // Increment outer IV
            make_instr(OpCode::LoadLocal, Some(Operand::Local(0))),                  // 15
            make_instr(OpCode::PushConst, Some(Operand::Const(0))),                  // 16
            make_instr(OpCode::AddInt, None),                                        // 17
            make_instr(OpCode::StoreLocal, Some(Operand::Local(0))),                 // 18
            make_instr(OpCode::LoopEnd, None),                                       // 19: outer end
        ];

        let program = make_program(instrs, vec![Constant::Int(1)]);
        let loop_info = crate::translator::loop_analysis::analyze_loops(&program);
        let plan = analyze_licm(&program, &loop_info);

        // Outer loop should have sin(x) hoistable but NOT cos(x) from inner loop
        if let Some(calls) = plan.hoistable_calls_by_loop.get(&0) {
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0].call_idx, 7, "should be the sin() call in outer body");
        }
    }

    #[test]
    fn test_constant_arg_hoistable() {
        // sin(3.14) where the argument is a constant
        let instrs = vec![
            make_instr(OpCode::LoopStart, None),                                    // 0
            make_instr(OpCode::LoadLocal, Some(Operand::Local(0))),                  // 1
            make_instr(OpCode::LoadLocal, Some(Operand::Local(1))),                  // 2
            make_instr(OpCode::LtInt, None),                                         // 3
            make_instr(OpCode::JumpIfFalse, Some(Operand::Offset(8))),               // 4
            make_instr(OpCode::PushConst, Some(Operand::Const(1))),                  // 5: 3.14
            make_instr(OpCode::PushConst, Some(Operand::Const(0))),                  // 6: argc=1
            make_instr(OpCode::BuiltinCall, Some(Operand::Builtin(BuiltinFunction::Sin))), // 7
            make_instr(OpCode::Pop, None),                                           // 8
            make_instr(OpCode::LoadLocal, Some(Operand::Local(0))),                  // 9
            make_instr(OpCode::PushConst, Some(Operand::Const(0))),                  // 10
            make_instr(OpCode::AddInt, None),                                        // 11
            make_instr(OpCode::StoreLocal, Some(Operand::Local(0))),                 // 12
            make_instr(OpCode::LoopEnd, None),                                       // 13
        ];

        let program = make_program(
            instrs,
            vec![Constant::Int(1), Constant::Number(3.14)],
        );
        let loop_info = crate::translator::loop_analysis::analyze_loops(&program);
        let plan = analyze_licm(&program, &loop_info);

        assert!(plan.hoistable_calls_by_loop.contains_key(&0));
        let calls = &plan.hoistable_calls_by_loop[&0];
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].call_idx, 7);
    }
}
