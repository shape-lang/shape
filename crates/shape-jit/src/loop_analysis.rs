//! Loop Analysis for JIT Optimization
//!
//! Analyzes bytecode loops to identify:
//! - Loop-invariant locals (for LICM — Loop-Invariant Code Motion)
//! - Induction variables (for bounds check hoisting and strength reduction)
//! - Simple loops amenable to unrolling
//!
//! This analysis runs before code generation and produces LoopInfo
//! structs that the compiler consults during IR emission.

use std::collections::{HashMap, HashSet};

use cranelift::prelude::IntCC;
use shape_vm::bytecode::{BytecodeProgram, Instruction, OpCode, Operand};

/// Information about a single loop in the bytecode.
#[derive(Debug, Clone)]
pub struct LoopInfo {
    /// Bytecode index of LoopStart
    pub header_idx: usize,
    /// Bytecode index of LoopEnd
    pub end_idx: usize,
    /// Local variables written inside the loop body
    pub body_locals_written: HashSet<u16>,
    /// Local variables read inside the loop body
    pub body_locals_read: HashSet<u16>,
    /// Module bindings written inside the loop body
    pub body_module_bindings_written: HashSet<u16>,
    /// Module bindings read inside the loop body
    pub body_module_bindings_read: HashSet<u16>,
    /// Identified induction variables
    pub induction_vars: Vec<InductionVar>,
    /// Locals that are loop-invariant (read but not written in loop body)
    pub invariant_locals: HashSet<u16>,
    /// Module bindings that are loop-invariant
    pub invariant_module_bindings: HashSet<u16>,
    /// Whether the loop body contains opcodes that may trigger heap allocation.
    /// When false, the GC safepoint poll at the loop header can be skipped,
    /// eliminating a load + compare + branch per iteration (~3 cycles saved).
    pub body_can_allocate: bool,
    /// Bytecode indices of calls that the LICM pass identified as hoistable.
    /// Populated by the optimizer's LICM analysis after loop detection.
    /// The translator consults this to emit hoisted calls in the loop pre-header.
    pub hoistable_calls: Vec<usize>,
}

/// An induction variable: a local or module binding that follows the pattern
/// `var = var + step` each iteration.
#[derive(Debug, Clone)]
pub struct InductionVar {
    /// The local variable slot used as induction variable
    pub local_slot: u16,
    /// Whether this is a module binding (true) or local variable (false)
    pub is_module_binding: bool,
    /// Comparison condition used in loop test
    pub bound_cmp: IntCC,
    /// The local slot that the induction var is compared against (bound)
    pub bound_slot: Option<u16>,
    /// The constant step value (e.g. 1 for `i = i + 1`), if detected
    pub step_value: Option<i64>,
}

/// Analyze all loops in a bytecode program.
///
/// Scans for LoopStart/LoopEnd pairs and collects read/write sets,
/// induction variables, and invariant locals for each loop.
pub fn analyze_loops(program: &BytecodeProgram) -> HashMap<usize, LoopInfo> {
    let mut result = HashMap::new();

    // First pass: find LoopStart/LoopEnd pairs
    let mut loop_starts: Vec<usize> = Vec::new();
    let mut loop_pairs: Vec<(usize, usize)> = Vec::new();

    for (i, instr) in program.instructions.iter().enumerate() {
        match instr.opcode {
            OpCode::LoopStart => loop_starts.push(i),
            OpCode::LoopEnd => {
                if let Some(start) = loop_starts.pop() {
                    loop_pairs.push((start, i));
                }
            }
            _ => {}
        }
    }

    // Second pass: analyze each loop
    for (start_idx, end_idx) in loop_pairs {
        // Check if any opcode in the loop body can trigger allocation
        let body_can_allocate = program.instructions[start_idx + 1..end_idx]
            .iter()
            .any(|instr| !opcode_is_non_allocating(instr.opcode));

        let mut info = LoopInfo {
            header_idx: start_idx,
            end_idx,
            body_locals_written: HashSet::new(),
            body_locals_read: HashSet::new(),
            body_module_bindings_written: HashSet::new(),
            body_module_bindings_read: HashSet::new(),
            induction_vars: Vec::new(),
            invariant_locals: HashSet::new(),
            invariant_module_bindings: HashSet::new(),
            hoistable_calls: Vec::new(),
            body_can_allocate,
        };

        // Scan loop body for local and module binding reads and writes.
        // For an outer loop, ignore instructions inside nested loops so we don't
        // accidentally classify inner-loop locals/IVs as outer-loop candidates.
        let mut nested_depth = 0usize;
        for i in (start_idx + 1)..end_idx {
            let instr = &program.instructions[i];
            match instr.opcode {
                OpCode::LoopStart => {
                    nested_depth += 1;
                    continue;
                }
                OpCode::LoopEnd => {
                    nested_depth = nested_depth.saturating_sub(1);
                    continue;
                }
                _ => {}
            }
            if nested_depth > 0 {
                continue;
            }
            match instr.opcode {
                OpCode::StoreLocal => {
                    if let Some(Operand::Local(idx)) = &instr.operand {
                        info.body_locals_written.insert(*idx);
                    }
                }
                OpCode::StoreLocalTyped => {
                    if let Some(Operand::TypedLocal(idx, _)) = &instr.operand {
                        info.body_locals_written.insert(*idx);
                    }
                }
                OpCode::LoadLocal | OpCode::LoadLocalTrusted => {
                    if let Some(Operand::Local(idx)) = &instr.operand {
                        info.body_locals_read.insert(*idx);
                    }
                }
                OpCode::StoreModuleBinding => {
                    if let Some(Operand::ModuleBinding(idx)) = &instr.operand {
                        info.body_module_bindings_written.insert(*idx);
                    }
                }
                OpCode::LoadModuleBinding => {
                    if let Some(Operand::ModuleBinding(idx)) = &instr.operand {
                        info.body_module_bindings_read.insert(*idx);
                    }
                }
                _ => {}
            }
        }

        // Invariant locals: read but not written
        for &local in &info.body_locals_read {
            if !info.body_locals_written.contains(&local) {
                info.invariant_locals.insert(local);
            }
        }

        // Invariant module bindings: read but not written
        for &mb in &info.body_module_bindings_read {
            if !info.body_module_bindings_written.contains(&mb) {
                info.invariant_module_bindings.insert(mb);
            }
        }

        // Detect induction variables:
        // Pattern: LoadLocal(X) → PushConst(1) → AddInt → StoreLocal(X)
        // This is the canonical `x = x + 1` pattern
        detect_induction_vars(
            &program.instructions,
            start_idx,
            end_idx,
            &mut info,
            &program.constants,
        );

        result.insert(start_idx, info);
    }

    result
}

/// Detect induction variable patterns in a loop body.
///
/// Looks for patterns like:
///   LoadLocal(X), PushConst(step), AddInt/SubInt, StoreLocal(X)
///   LoadModuleBinding(X), PushConst(step), Add, StoreModuleBinding(X)
///
/// Also detects the bound comparison:
///   LoadLocal(X), LoadLocal(Y), LtInt/GtInt/etc. → JumpIfFalse
fn detect_induction_vars(
    instrs: &[Instruction],
    start_idx: usize,
    end_idx: usize,
    info: &mut LoopInfo,
    constants: &[shape_vm::bytecode::Constant],
) {
    // Precompute loop nesting depth for each instruction in this loop body.
    // Depth 0 = instruction belongs to this loop directly (not a nested loop).
    let mut depth_by_idx = vec![0usize; instrs.len()];
    let mut depth = 0usize;
    for i in (start_idx + 1)..end_idx {
        depth_by_idx[i] = depth;
        match instrs[i].opcode {
            OpCode::LoopStart => depth += 1,
            OpCode::LoopEnd => depth = depth.saturating_sub(1),
            _ => {}
        }
    }

    // Look for increment patterns at depth 0 only.
    for i in (start_idx + 1)..end_idx.saturating_sub(3) {
        if depth_by_idx[i] != 0
            || depth_by_idx[i + 1] != 0
            || depth_by_idx[i + 2] != 0
            || depth_by_idx[i + 3] != 0
        {
            continue;
        }
        let (load, step_src, arith, store) =
            (&instrs[i], &instrs[i + 1], &instrs[i + 2], &instrs[i + 3]);

        let is_arith = matches!(
            arith.opcode,
            OpCode::AddInt | OpCode::SubInt | OpCode::AddDynamic | OpCode::SubDynamic
        );
        let is_supported_step_src = matches!(
            step_src.opcode,
            OpCode::PushConst
                | OpCode::LoadLocal
                | OpCode::LoadLocalTrusted
                | OpCode::LoadModuleBinding
        );
        if !is_arith || !is_supported_step_src {
            continue;
        }

        // Extract constant step value when available; variable-step IVs are
        // represented with `None` and validated later in bounds analysis.
        let step_value = if step_src.opcode == OpCode::PushConst {
            if let Some(Operand::Const(const_idx)) = &step_src.operand {
                match constants.get(*const_idx as usize) {
                    Some(shape_vm::bytecode::Constant::Int(n)) => {
                        let step = if matches!(arith.opcode, OpCode::SubInt | OpCode::SubDynamic) {
                            -n
                        } else {
                            *n
                        };
                        Some(step)
                    }
                    Some(shape_vm::bytecode::Constant::UInt(n)) => {
                        let step = *n as i64;
                        let step = if matches!(arith.opcode, OpCode::SubInt | OpCode::SubDynamic) {
                            -step
                        } else {
                            step
                        };
                        Some(step)
                    }
                    Some(shape_vm::bytecode::Constant::Number(n)) if *n == (*n as i64) as f64 => {
                        let step = if matches!(arith.opcode, OpCode::SubInt | OpCode::SubDynamic) {
                            -(*n as i64)
                        } else {
                            *n as i64
                        };
                        Some(step)
                    }
                    _ => None,
                }
            } else {
                None
            }
        } else {
            None
        };

        // Check: LoadLocal(X) ... Add/AddInt ... StoreLocal(X)
        if matches!(load.opcode, OpCode::LoadLocal | OpCode::LoadLocalTrusted)
            && matches!(store.opcode, OpCode::StoreLocal | OpCode::StoreLocalTyped)
        {
            let store_local_idx = match &store.operand {
                Some(Operand::Local(idx)) => Some(*idx),
                Some(Operand::TypedLocal(idx, _)) => Some(*idx),
                _ => None,
            };
            if let (Some(Operand::Local(load_idx)), Some(store_idx)) =
                (&load.operand, store_local_idx)
            {
                if *load_idx == store_idx {
                    let bound_info =
                        detect_bound_comparison(instrs, start_idx, end_idx, *load_idx, false);
                    if bound_info.1.is_none() {
                        continue;
                    }

                    info.induction_vars.push(InductionVar {
                        local_slot: *load_idx,
                        is_module_binding: false,
                        bound_cmp: bound_info.0,
                        bound_slot: bound_info.1,
                        step_value,
                    });
                }
            }
        }

        // Check: LoadModuleBinding(X) ... Add/AddInt ... StoreModuleBinding(X)
        if load.opcode == OpCode::LoadModuleBinding && store.opcode == OpCode::StoreModuleBinding {
            if let (
                Some(Operand::ModuleBinding(load_idx)),
                Some(Operand::ModuleBinding(store_idx)),
            ) = (&load.operand, &store.operand)
            {
                if load_idx == store_idx {
                    let bound_info =
                        detect_bound_comparison(instrs, start_idx, end_idx, *load_idx, true);
                    if bound_info.1.is_none() {
                        continue;
                    }

                    info.induction_vars.push(InductionVar {
                        local_slot: *load_idx,
                        is_module_binding: true,
                        bound_cmp: bound_info.0,
                        bound_slot: bound_info.1,
                        step_value,
                    });
                }
            }
        }
    }
}

/// Detect the bound comparison for an induction variable.
///
/// Looks for: Load(indvar), Load(bound), Lt/Gt/etc. → JumpIfFalse
/// Handles both LoadLocal and LoadModuleBinding patterns.
fn detect_bound_comparison(
    instrs: &[Instruction],
    start_idx: usize,
    end_idx: usize,
    indvar_slot: u16,
    is_module_binding: bool,
) -> (IntCC, Option<u16>) {
    let (load_op, bound_load_op) = if is_module_binding {
        (OpCode::LoadModuleBinding, OpCode::LoadModuleBinding)
    } else {
        (OpCode::LoadLocal, OpCode::LoadLocal)
    };

    // Scan the first few instructions after LoopStart for the comparison
    let scan_end = (start_idx + 10).min(end_idx);
    for window in instrs[start_idx + 1..scan_end].windows(3) {
        let (load1, load2, cmp) = (&window[0], &window[1], &window[2]);

        let load1_matches = load1.opcode == load_op
            || (!is_module_binding && load1.opcode == OpCode::LoadLocalTrusted);
        let load2_matches = load2.opcode == bound_load_op
            || (!is_module_binding && load2.opcode == OpCode::LoadLocalTrusted);
        if load1_matches && load2_matches {
            let l1 = match &load1.operand {
                Some(Operand::Local(idx)) if !is_module_binding => Some(*idx),
                Some(Operand::ModuleBinding(idx)) if is_module_binding => Some(*idx),
                _ => None,
            };
            let l2 = match &load2.operand {
                Some(Operand::Local(idx)) if !is_module_binding => Some(*idx),
                Some(Operand::ModuleBinding(idx)) if is_module_binding => Some(*idx),
                _ => None,
            };

            if let (Some(l1), Some(l2)) = (l1, l2) {
                if l1 == indvar_slot {
                    let cc = match cmp.opcode {
                        OpCode::LtInt | OpCode::LtDynamic => IntCC::SignedLessThan,
                        OpCode::LteInt | OpCode::LteDynamic => {
                            IntCC::SignedLessThanOrEqual
                        }
                        OpCode::GtInt | OpCode::GtDynamic => {
                            IntCC::SignedGreaterThan
                        }
                        OpCode::GteInt | OpCode::GteDynamic => {
                            IntCC::SignedGreaterThanOrEqual
                        }
                        _ => continue,
                    };
                    return (cc, Some(l2));
                }
            }
        }
    }

    (IntCC::SignedLessThan, None) // Default
}

/// Returns true if the opcode is definitively non-allocating.
///
/// Non-allocating opcodes never trigger heap allocation, so loops containing
/// only these opcodes don't need GC safepoint polling at the loop header.
///
/// Conservative: returns false for any opcode that might allocate, including
/// function calls, array/object creation, and operations with known allocating
/// FFI slow paths.
///
/// Note: generic numeric arithmetic/comparison opcodes are non-allocating in the
/// current JIT lowering (they do not dispatch through allocating string/object
/// paths), so they are treated as safe here.
fn opcode_is_non_allocating(opcode: OpCode) -> bool {
    matches!(
        opcode,
        // Stack manipulation (pure register/variable ops)
        OpCode::PushConst
            | OpCode::PushNull
            | OpCode::Pop
            | OpCode::Dup
            | OpCode::Swap
            // Typed arithmetic (inline f64/i64 ops, no FFI)
            | OpCode::AddInt
            | OpCode::SubInt
            | OpCode::MulInt
            | OpCode::DivInt
            | OpCode::ModInt
            | OpCode::AddNumber
            | OpCode::SubNumber
            | OpCode::MulNumber
            | OpCode::DivNumber
            | OpCode::ModNumber
            | OpCode::AddDecimal
            | OpCode::SubDecimal
            | OpCode::MulDecimal
            | OpCode::DivDecimal
            | OpCode::ModDecimal
            // Generic add/sub are lowered to numeric ops in JIT (no alloc path)
            | OpCode::AddDynamic
            | OpCode::SubDynamic
            // Generic mul/div/mod/neg (always use inline typed_binary_op, no FFI path)
            | OpCode::MulDynamic
            | OpCode::DivDynamic
            | OpCode::ModDynamic
            | OpCode::NegInt
            | OpCode::NegNumber
            // Equality comparisons (always inline, no FFI)
            | OpCode::EqDynamic
            | OpCode::NeqDynamic
            // Generic ordered comparisons are inline numeric comparisons in JIT.
            | OpCode::GtDynamic
            | OpCode::LtDynamic
            | OpCode::GteDynamic
            | OpCode::LteDynamic
            // Typed comparisons (inline fcmp/icmp, no FFI)
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
            | OpCode::IsNull
            | OpCode::GtDecimal
            | OpCode::LtDecimal
            | OpCode::GteDecimal
            | OpCode::LteDecimal
            // Logical (inline)
            | OpCode::And
            | OpCode::Or
            | OpCode::Not
            // Variable access (Cranelift Variables / memory loads, no allocation)
            | OpCode::LoadLocal
            | OpCode::LoadLocalTrusted
            | OpCode::StoreLocal
            | OpCode::StoreLocalTyped
            | OpCode::LoadModuleBinding
            | OpCode::StoreModuleBinding
            | OpCode::StoreModuleBindingTyped
            | OpCode::LoadClosure
            | OpCode::StoreClosure
            // Type casting (inline, no allocation)
            | OpCode::CastWidth
            // Control flow (inline jumps/branches)
            | OpCode::Jump
            | OpCode::JumpIfFalse
            | OpCode::JumpIfFalseTrusted
            | OpCode::JumpIfTrue
            | OpCode::LoopStart
            | OpCode::LoopEnd
            | OpCode::Break
            | OpCode::Continue
            | OpCode::Return
            | OpCode::ReturnValue
            // Bitwise (inline i64 ops)
            | OpCode::BitAnd
            | OpCode::BitOr
            | OpCode::BitXor
            | OpCode::BitShl
            | OpCode::BitShr
            | OpCode::BitNot
            // Type coercion (inline, no allocation)
            | OpCode::IntToNumber
            | OpCode::NumberToInt
            // Typed object field access (reads/writes existing object memory)
            | OpCode::GetFieldTyped
            | OpCode::SetFieldTyped
            // Reference ops (access existing memory, no allocation)
            | OpCode::MakeRef
            | OpCode::DerefLoad
            | OpCode::DerefStore
            | OpCode::SetIndexRef
            // In-place array index writes (no growth path in these opcodes)
            | OpCode::GetProp
            | OpCode::SetLocalIndex
            | OpCode::SetModuleBindingIndex
            | OpCode::Length
            // No-ops in JIT
            | OpCode::Halt
            | OpCode::Nop
            | OpCode::PushTimeframe
            | OpCode::PopTimeframe
            | OpCode::WrapTypeAnnotation
            | OpCode::BindSchema
            | OpCode::ErrorContext
            | OpCode::CloseUpvalue
            // Async/task no-ops in JIT
            | OpCode::Yield
            | OpCode::Suspend
            | OpCode::Resume
            | OpCode::Poll
            | OpCode::AwaitBar
            | OpCode::AwaitTick
            | OpCode::Await
            // Drop/box no-ops in JIT
            | OpCode::DropCall
            | OpCode::DropCallAsync
            | OpCode::BoxLocal
            | OpCode::BoxModuleBinding
            // Event/async scope no-ops in JIT
            | OpCode::EmitAlert
            | OpCode::EmitEvent
            | OpCode::AsyncScopeEnter
            | OpCode::AsyncScopeExit
            // Trait object no-ops in JIT
            | OpCode::BoxTraitObject
            | OpCode::DynMethodCall
    )
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
            foreign_functions: Vec::new(),
            native_struct_layouts: vec![],
            content_addressed: None,
            function_blob_hashes: vec![],
            top_level_frame: None,
            ..Default::default()
        }
    }

    #[test]
    fn test_detect_invariant_locals() {
        // Simulate: loop { x = x + y } where y is invariant
        let instrs = vec![
            make_instr(OpCode::LoopStart, None),
            make_instr(OpCode::LoadLocal, Some(Operand::Local(0))), // load x
            make_instr(OpCode::LoadLocal, Some(Operand::Local(1))), // load y (invariant)
            make_instr(OpCode::AddInt, None),
            make_instr(OpCode::StoreLocal, Some(Operand::Local(0))), // store x
            make_instr(OpCode::LoopEnd, None),
        ];

        let loops = analyze_loops(&make_program(instrs, vec![]));
        assert_eq!(loops.len(), 1);

        let info = loops.get(&0).unwrap();
        assert!(info.body_locals_written.contains(&0)); // x is written
        assert!(!info.body_locals_written.contains(&1)); // y is NOT written
        assert!(info.invariant_locals.contains(&1)); // y is invariant
        assert!(!info.invariant_locals.contains(&0)); // x is NOT invariant
    }

    #[test]
    fn test_detect_induction_variable() {
        // Simulate: for (i = 0; i < n; i++) { ... }
        let instrs = vec![
            make_instr(OpCode::LoopStart, None),
            // Loop condition: i < n
            make_instr(OpCode::LoadLocal, Some(Operand::Local(0))), // load i
            make_instr(OpCode::LoadLocal, Some(Operand::Local(1))), // load n
            make_instr(OpCode::LtInt, None),
            make_instr(OpCode::JumpIfFalse, Some(Operand::Offset(5))),
            // Loop body (empty for this test)
            // Increment: i = i + 1
            make_instr(OpCode::LoadLocal, Some(Operand::Local(0))), // load i
            make_instr(OpCode::PushConst, Some(Operand::Const(0))), // push 1
            make_instr(OpCode::AddInt, None),
            make_instr(OpCode::StoreLocal, Some(Operand::Local(0))), // store i
            make_instr(OpCode::LoopEnd, None),
        ];

        let loops = analyze_loops(&make_program(instrs, vec![Constant::Int(1)]));
        let info = loops.get(&0).unwrap();

        assert_eq!(info.induction_vars.len(), 1);
        assert_eq!(info.induction_vars[0].local_slot, 0);
        assert_eq!(info.induction_vars[0].bound_slot, Some(1));
        assert_eq!(info.induction_vars[0].bound_cmp, IntCC::SignedLessThan);
    }

    #[test]
    fn test_non_allocating_loop() {
        // Pure arithmetic loop: s = s + i; i = i + 1
        // Should NOT need GC safepoint
        let instrs = vec![
            make_instr(OpCode::LoopStart, None),
            make_instr(OpCode::LoadLocal, Some(Operand::Local(0))),
            make_instr(OpCode::LoadLocal, Some(Operand::Local(1))),
            make_instr(OpCode::LtInt, None),
            make_instr(OpCode::JumpIfFalse, Some(Operand::Offset(7))),
            make_instr(OpCode::LoadLocal, Some(Operand::Local(2))),
            make_instr(OpCode::LoadLocal, Some(Operand::Local(0))),
            make_instr(OpCode::AddInt, None),
            make_instr(OpCode::StoreLocal, Some(Operand::Local(2))),
            make_instr(OpCode::LoadLocal, Some(Operand::Local(0))),
            make_instr(OpCode::PushConst, Some(Operand::Const(0))),
            make_instr(OpCode::AddInt, None),
            make_instr(OpCode::StoreLocal, Some(Operand::Local(0))),
            make_instr(OpCode::LoopEnd, None),
        ];

        let loops = analyze_loops(&make_program(instrs, vec![Constant::Int(1)]));
        let info = loops.get(&0).unwrap();
        assert!(
            !info.body_can_allocate,
            "Pure arithmetic loop should not need GC safepoint"
        );
    }

    #[test]
    fn test_allocating_loop() {
        // Loop with array push: arr.push(i)
        // SHOULD need GC safepoint (ArrayPush may reallocate)
        let instrs = vec![
            make_instr(OpCode::LoopStart, None),
            make_instr(OpCode::LoadLocal, Some(Operand::Local(0))),
            make_instr(OpCode::LoadLocal, Some(Operand::Local(1))),
            make_instr(OpCode::ArrayPush, None),
            make_instr(OpCode::LoopEnd, None),
        ];

        let loops = analyze_loops(&make_program(instrs, vec![]));
        let info = loops.get(&0).unwrap();
        assert!(
            info.body_can_allocate,
            "Loop with ArrayPush should need GC safepoint"
        );
    }

    #[test]
    fn test_loop_with_function_call_allocates() {
        // Loop with function call: f(x)
        // SHOULD need GC safepoint (calls can allocate anything)
        let instrs = vec![
            make_instr(OpCode::LoopStart, None),
            make_instr(OpCode::LoadLocal, Some(Operand::Local(0))),
            make_instr(
                OpCode::Call,
                Some(Operand::Function(shape_value::FunctionId(0))),
            ),
            make_instr(OpCode::Pop, None),
            make_instr(OpCode::LoopEnd, None),
        ];

        let loops = analyze_loops(&make_program(instrs, vec![]));
        let info = loops.get(&0).unwrap();
        assert!(
            info.body_can_allocate,
            "Loop with Call should need GC safepoint"
        );
    }

    #[test]
    fn test_loop_with_set_index_ref_is_non_allocating() {
        // In-place array mutation through references should not force
        // per-iteration safepoint checks.
        let instrs = vec![
            make_instr(OpCode::MakeRef, Some(Operand::Local(0))),
            make_instr(OpCode::StoreLocal, Some(Operand::Local(3))), // r = &arr
            make_instr(OpCode::LoopStart, None),
            make_instr(OpCode::LoadLocal, Some(Operand::Local(1))), // i
            make_instr(OpCode::LoadLocal, Some(Operand::Local(2))), // n
            make_instr(OpCode::LtInt, None),
            make_instr(OpCode::JumpIfFalse, Some(Operand::Offset(8))),
            make_instr(OpCode::PushConst, Some(Operand::Const(0))), // value
            make_instr(OpCode::LoadLocal, Some(Operand::Local(1))), // i
            make_instr(OpCode::SetIndexRef, Some(Operand::Local(3))),
            make_instr(OpCode::LoadLocal, Some(Operand::Local(1))),
            make_instr(OpCode::PushConst, Some(Operand::Const(1))),
            make_instr(OpCode::AddInt, None),
            make_instr(OpCode::StoreLocal, Some(Operand::Local(1))),
            make_instr(OpCode::LoopEnd, None),
        ];

        let loops = analyze_loops(&make_program(
            instrs,
            vec![Constant::Bool(false), Constant::Int(1)],
        ));
        let info = loops.get(&2).unwrap();
        assert!(
            !info.body_can_allocate,
            "Loop with SetIndexRef should be treated as non-allocating"
        );
    }
}
