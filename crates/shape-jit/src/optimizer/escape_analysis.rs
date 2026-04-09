//! Escape analysis and scalar replacement planning for JIT compilation.
//!
//! Identifies small, non-escaping arrays that can be replaced with scalar
//! SSA variables, eliminating heap allocation entirely. This is a conservative
//! single-basic-block analysis: only arrays whose entire lifetime is confined
//! to a straight-line sequence of instructions (no control flow) are eligible.
//!
//! **Eligibility criteria:**
//! - Array created by `NewArray` with element count <= 8
//! - All uses are `GetProp` (index read) or `SetLocalIndex` (index write)
//!   with constant indices
//! - Array is not passed to any Call/CallMethod/CallValue/BuiltinCall
//! - Array is not stored to the heap (object fields, closures)
//! - Array is not returned from the function
//! - Array lifetime is within a single basic block (no branches cross it)

use std::collections::{HashMap, HashSet};

use shape_vm::bytecode::{BytecodeProgram, Constant, OpCode, Operand};

/// Maximum number of elements for scalar-replaceable arrays.
pub const MAX_SCALAR_ARRAY_ELEMENTS: usize = 8;

/// Describes one array eligible for scalar replacement.
#[derive(Debug, Clone)]
pub struct ScalarArrayEntry {
    /// Local variable slot where the array is stored immediately after creation.
    pub local_slot: u16,
    /// Number of elements in the array (from `Operand::Count`).
    pub element_count: usize,
    /// Instruction indices of `GetProp` reads with their constant index.
    /// Maps instruction index -> element index.
    pub get_sites: HashMap<usize, usize>,
    /// Instruction indices of `SetLocalIndex` writes with their constant index.
    /// Maps instruction index -> element index.
    pub set_sites: HashMap<usize, usize>,
}

/// The escape analysis plan: a collection of arrays eligible for scalar replacement.
#[derive(Debug, Clone, Default)]
pub struct EscapeAnalysisPlan {
    /// Arrays that can be scalar-replaced, keyed by the `NewArray` instruction index.
    pub scalar_arrays: HashMap<usize, ScalarArrayEntry>,
}

impl EscapeAnalysisPlan {
    /// Returns true if any arrays are eligible for scalar replacement.
    #[cfg(test)]
    pub fn has_candidates(&self) -> bool {
        !self.scalar_arrays.is_empty()
    }
}

/// Track an array candidate through the bytecode.
struct ArrayCandidate {
    /// Instruction index of the NewArray.
    new_array_idx: usize,
    /// Local variable slot assigned to the array.
    local_slot: u16,
    /// Element count from the NewArray operand.
    element_count: usize,
    /// GetProp sites: instruction index -> constant element index.
    get_sites: HashMap<usize, usize>,
    /// SetLocalIndex sites: instruction index -> constant element index.
    set_sites: HashMap<usize, usize>,
    /// Whether the array has escaped (been used in a non-scalarizable way).
    escaped: bool,
}

/// Returns true if the opcode terminates or starts a basic block.
fn is_block_boundary(op: OpCode) -> bool {
    matches!(
        op,
        OpCode::Jump
            | OpCode::JumpIfFalse
            | OpCode::JumpIfFalseTrusted
            | OpCode::JumpIfTrue
            | OpCode::LoopStart
            | OpCode::LoopEnd
            | OpCode::Break
            | OpCode::Continue
            | OpCode::Return
            | OpCode::ReturnValue
            | OpCode::Halt
            | OpCode::SetupTry
            | OpCode::PopHandler
            | OpCode::Throw
    )
}

/// Returns true if the opcode is a call that could capture an argument.
fn is_escaping_call(op: OpCode) -> bool {
    matches!(
        op,
        OpCode::Call
            | OpCode::CallValue
            | OpCode::CallMethod
            | OpCode::BuiltinCall
            | OpCode::DynMethodCall
            | OpCode::CallForeign
            | OpCode::DropCall
            | OpCode::DropCallAsync
    )
}

/// Resolve a constant index from a `PushConst` instruction.
/// Returns `Some(index)` if the constant is a non-negative integer that fits in usize.
fn resolve_constant_index(program: &BytecodeProgram, const_idx: u16) -> Option<usize> {
    match program.constants.get(const_idx as usize)? {
        Constant::Int(v) if *v >= 0 => Some(*v as usize),
        Constant::UInt(v) => Some(*v as usize),
        Constant::Number(v) if *v >= 0.0 && *v == (*v as usize as f64) => Some(*v as usize),
        _ => None,
    }
}

/// Run escape analysis on a bytecode program.
///
/// Identifies `NewArray` instructions that produce small, non-escaping arrays
/// stored into local variables and accessed only via constant-index reads/writes.
pub fn analyze_escape(program: &BytecodeProgram) -> EscapeAnalysisPlan {
    let mut plan = EscapeAnalysisPlan::default();
    let instructions = &program.instructions;

    if instructions.is_empty() {
        return plan;
    }

    // Phase 1: Find candidate arrays.
    //
    // Pattern: NewArray(count) followed immediately by StoreLocal(slot).
    // The array must have count <= MAX_SCALAR_ARRAY_ELEMENTS.
    let mut candidates: Vec<ArrayCandidate> = Vec::new();
    // Map from local slot -> candidate index (for tracking uses).
    let mut slot_to_candidate: HashMap<u16, usize> = HashMap::new();

    for i in 0..instructions.len().saturating_sub(1) {
        let instr = &instructions[i];
        if instr.opcode != OpCode::NewArray {
            continue;
        }
        let count = match &instr.operand {
            Some(Operand::Count(c)) => *c as usize,
            _ => continue,
        };
        if count > MAX_SCALAR_ARRAY_ELEMENTS {
            continue;
        }

        // Must be immediately followed by StoreLocal.
        let next = &instructions[i + 1];
        let local_slot = match (next.opcode, &next.operand) {
            (OpCode::StoreLocal, Some(Operand::Local(slot))) => *slot,
            (OpCode::StoreLocalTyped, Some(Operand::TypedLocal(slot, _))) => *slot,
            _ => continue,
        };

        // If this slot was already tracked by a prior candidate, invalidate the old one.
        if let Some(&old_idx) = slot_to_candidate.get(&local_slot) {
            candidates[old_idx].escaped = true;
        }

        let cand_idx = candidates.len();
        candidates.push(ArrayCandidate {
            new_array_idx: i,
            local_slot,
            element_count: count,
            get_sites: HashMap::new(),
            set_sites: HashMap::new(),
            escaped: false,
        });
        slot_to_candidate.insert(local_slot, cand_idx);
    }

    if candidates.is_empty() {
        return plan;
    }

    // Phase 2: Scan all instructions for uses of candidate arrays.
    //
    // We need to track which local slots hold candidate arrays and detect
    // any uses that would cause the array to escape.

    // Track "active" candidates per local slot, and the basic block they were
    // created in. A basic block boundary kills all active candidates.
    let mut active_slots: HashSet<u16> = HashSet::new();
    // Track which candidates have been "activated" (past their NewArray+StoreLocal).
    let mut activated: HashSet<usize> = HashSet::new();

    // Collect jump targets so we can detect basic block entries.
    let mut jump_targets: HashSet<usize> = HashSet::new();
    for instr in instructions.iter() {
        // Extract jump target offsets.
        if let Some(Operand::Offset(off)) = &instr.operand {
            match instr.opcode {
                OpCode::Jump
                | OpCode::JumpIfFalse
                | OpCode::JumpIfFalseTrusted
                | OpCode::JumpIfTrue => {
                    // We need the instruction's index to compute the target.
                    // We'll do a second pass below.
                }
                _ => {}
            }
            let _ = off; // suppress unused warning
        }
    }
    // Second pass for jump targets with correct indices.
    for (i, instr) in instructions.iter().enumerate() {
        if let Some(Operand::Offset(off)) = &instr.operand {
            match instr.opcode {
                OpCode::Jump
                | OpCode::JumpIfFalse
                | OpCode::JumpIfFalseTrusted
                | OpCode::JumpIfTrue => {
                    let target = (i as i64 + *off as i64 + 1) as usize;
                    if target < instructions.len() {
                        jump_targets.insert(target);
                    }
                }
                _ => {}
            }
        }
    }

    for i in 0..instructions.len() {
        let instr = &instructions[i];

        // A basic block boundary kills all active candidates.
        if is_block_boundary(instr.opcode) || jump_targets.contains(&i) {
            for &slot in &active_slots {
                if let Some(&cand_idx) = slot_to_candidate.get(&slot) {
                    if activated.contains(&cand_idx) {
                        candidates[cand_idx].escaped = true;
                    }
                }
            }
            active_slots.clear();
        }

        // Check if this instruction activates a candidate (the StoreLocal after NewArray).
        if i > 0 && instructions[i - 1].opcode == OpCode::NewArray {
            match (instr.opcode, &instr.operand) {
                (OpCode::StoreLocal, Some(Operand::Local(slot)))
                | (OpCode::StoreLocalTyped, Some(Operand::TypedLocal(slot, _))) => {
                    if let Some(&cand_idx) = slot_to_candidate.get(slot) {
                        if candidates[cand_idx].new_array_idx == i - 1 && !candidates[cand_idx].escaped
                        {
                            activated.insert(cand_idx);
                            active_slots.insert(*slot);
                        }
                    }
                }
                _ => {}
            }
        }

        // Track uses of candidate array locals.
        match (instr.opcode, &instr.operand) {
            // LoadLocal of a candidate slot: track where the value goes.
            (OpCode::LoadLocal | OpCode::LoadLocalTrusted, Some(Operand::Local(slot))) => {
                if let Some(&cand_idx) = slot_to_candidate.get(slot) {
                    if activated.contains(&cand_idx) && !candidates[cand_idx].escaped {
                        // The loaded value will be on the stack. We need to check what
                        // consumes it. Look ahead for the consumer.
                        // For GetProp (dynamic index): stack is [..., array, index] -> GetProp
                        // We check if i+2 is GetProp with no property operand (dynamic index).
                        // And i+1 is a PushConst with a constant integer index.
                        if i + 2 < instructions.len() {
                            let next1 = &instructions[i + 1];
                            let next2 = &instructions[i + 2];
                            if next2.opcode == OpCode::GetProp && next2.operand.is_none() {
                                // Dynamic index read: check if index is constant.
                                if let (
                                    OpCode::PushConst,
                                    Some(Operand::Const(const_idx)),
                                ) = (next1.opcode, &next1.operand)
                                {
                                    if let Some(elem_idx) =
                                        resolve_constant_index(program, *const_idx)
                                    {
                                        if elem_idx < candidates[cand_idx].element_count {
                                            candidates[cand_idx]
                                                .get_sites
                                                .insert(i + 2, elem_idx);
                                            continue;
                                        }
                                    }
                                }
                            }
                        }

                        // If we get here, the LoadLocal was not followed by a recognized
                        // constant-index GetProp pattern. The array escapes.
                        candidates[cand_idx].escaped = true;
                    }
                }
            }

            // SetLocalIndex with the candidate's slot: constant-index write.
            (OpCode::SetLocalIndex, Some(Operand::Local(slot))) => {
                if let Some(&cand_idx) = slot_to_candidate.get(slot) {
                    if activated.contains(&cand_idx) && !candidates[cand_idx].escaped {
                        // Stack before SetLocalIndex: [..., index, value]
                        // We need to check that the index is a constant.
                        // Look backwards for the index producer.
                        // The index is the second-from-top value. We scan back
                        // to find the PushConst that produced it.
                        if let Some(const_index) =
                            find_constant_index_for_set(program, i)
                        {
                            if const_index < candidates[cand_idx].element_count {
                                candidates[cand_idx].set_sites.insert(i, const_index);
                                continue;
                            }
                        }
                        // Non-constant or out-of-range index -- array escapes.
                        candidates[cand_idx].escaped = true;
                    }
                }
            }

            // Re-assignment of the local slot kills the candidate.
            (OpCode::StoreLocal, Some(Operand::Local(slot)))
            | (OpCode::StoreLocalTyped, Some(Operand::TypedLocal(slot, _))) => {
                if let Some(&cand_idx) = slot_to_candidate.get(slot) {
                    // If this is the initial store (activating the candidate), skip.
                    if activated.contains(&cand_idx)
                        && candidates[cand_idx].new_array_idx + 1 != i
                    {
                        candidates[cand_idx].escaped = true;
                    }
                }
            }

            // Reference operations: taking a reference to the array (MakeRef),
            // projecting through it (MakeFieldRef, MakeIndexRef), reading/writing
            // through it (DerefLoad, DerefStore, SetIndexRef) all constitute escape.
            (OpCode::MakeRef, Some(Operand::Local(slot))) => {
                if let Some(&cand_idx) = slot_to_candidate.get(slot) {
                    if activated.contains(&cand_idx) {
                        candidates[cand_idx].escaped = true;
                    }
                }
            }
            (OpCode::SetIndexRef | OpCode::MakeFieldRef | OpCode::MakeIndexRef
             | OpCode::DerefLoad | OpCode::DerefStore, _) => {
                // Conservative: any reference manipulation while candidates are
                // active causes all of them to escape (the reference could alias
                // any candidate's local).
                for &slot in &active_slots {
                    if let Some(&cand_idx) = slot_to_candidate.get(&slot) {
                        candidates[cand_idx].escaped = true;
                    }
                }
            }

            // Any call instruction: check if any candidate array is on the stack.
            // Conservative: if a call happens while any candidate is active, and
            // the candidate's local is live, the array could be read from the local.
            // We don't try to track the stack precisely -- just mark all active
            // candidates as escaped if a call occurs.
            _ if is_escaping_call(instr.opcode) => {
                for &slot in &active_slots {
                    if let Some(&cand_idx) = slot_to_candidate.get(&slot) {
                        candidates[cand_idx].escaped = true;
                    }
                }
            }

            // Return: arrays on the stack or in locals escape.
            (OpCode::Return | OpCode::ReturnValue, _) => {
                for &slot in &active_slots {
                    if let Some(&cand_idx) = slot_to_candidate.get(&slot) {
                        candidates[cand_idx].escaped = true;
                    }
                }
            }

            // ArrayPush, ArrayPop, Length, SliceAccess on active candidates: escape.
            (OpCode::ArrayPush | OpCode::ArrayPushLocal | OpCode::ArrayPop | OpCode::Length | OpCode::SliceAccess, _) => {
                // These modify or read the array in ways we can't scalarize.
                // Check if the operand references a candidate slot.
                if let Some(Operand::Local(slot)) = &instr.operand {
                    if let Some(&cand_idx) = slot_to_candidate.get(slot) {
                        if activated.contains(&cand_idx) {
                            candidates[cand_idx].escaped = true;
                        }
                    }
                }
                // For stack-based operations (ArrayPush, ArrayPop, Length, SliceAccess),
                // the array might be from any active candidate.
                // Conservative: mark all active.
                if matches!(instr.opcode, OpCode::ArrayPush | OpCode::ArrayPop | OpCode::Length | OpCode::SliceAccess) {
                    for &slot in &active_slots {
                        if let Some(&cand_idx) = slot_to_candidate.get(&slot) {
                            candidates[cand_idx].escaped = true;
                        }
                    }
                }
            }

            // SetProp with dynamic key on the stack might store the array.
            (OpCode::SetProp, _) => {
                for &slot in &active_slots {
                    if let Some(&cand_idx) = slot_to_candidate.get(&slot) {
                        candidates[cand_idx].escaped = true;
                    }
                }
            }

            // Closure capture: array escapes.
            (OpCode::BoxLocal, Some(Operand::Local(slot))) => {
                if let Some(&cand_idx) = slot_to_candidate.get(slot) {
                    candidates[cand_idx].escaped = true;
                }
            }
            (OpCode::MakeClosure, _) => {
                for &slot in &active_slots {
                    if let Some(&cand_idx) = slot_to_candidate.get(&slot) {
                        candidates[cand_idx].escaped = true;
                    }
                }
            }

            _ => {}
        }
    }

    // Phase 3: Collect surviving candidates into the plan.
    for candidate in candidates {
        if candidate.escaped {
            continue;
        }
        // Must have at least one use to be worth scalarizing.
        if candidate.get_sites.is_empty() && candidate.set_sites.is_empty() {
            continue;
        }
        plan.scalar_arrays.insert(
            candidate.new_array_idx,
            ScalarArrayEntry {
                local_slot: candidate.local_slot,
                element_count: candidate.element_count,
                get_sites: candidate.get_sites,
                set_sites: candidate.set_sites,
            },
        );
    }

    plan
}

/// For a `SetLocalIndex` at instruction `set_idx`, try to resolve the constant
/// index value from the second-from-top stack position.
///
/// The stack layout before SetLocalIndex is: [..., index, value].
/// We look for the instruction that produced the index (second from top).
fn find_constant_index_for_set(
    program: &BytecodeProgram,
    set_idx: usize,
) -> Option<usize> {
    // Walk backwards from set_idx to find the index producer.
    // The stack at set_idx has: [..., key, value] with key at depth 1 from top.
    // We need the producer of the second-from-top element.
    let mut depth_from_top: i32 = 1; // looking for the key (under the value)
    for j in (0..set_idx).rev() {
        let instr = &program.instructions[j];
        let op = instr.opcode;

        // Bail on block boundaries or calls.
        if is_block_boundary(op) || is_escaping_call(op) {
            return None;
        }

        let (pops, pushes) = stack_effect_simple(op)?;
        if depth_from_top < pushes {
            // This instruction produced the value at our target depth.
            if op == OpCode::PushConst {
                if let Some(Operand::Const(const_idx)) = &instr.operand {
                    return resolve_constant_index(program, *const_idx);
                }
            }
            // Not a constant -- can't resolve.
            return None;
        }
        depth_from_top = depth_from_top - pushes + pops;
        if depth_from_top < 0 {
            return None;
        }
    }
    None
}

/// Simple stack effect for escape analysis backward scanning.
/// Returns (pops, pushes) or None for variable-arity opcodes.
fn stack_effect_simple(op: OpCode) -> Option<(i32, i32)> {
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
        | OpCode::NegInt
        | OpCode::NegNumber
        | OpCode::IsNull
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
        | OpCode::NeqNumber
        | OpCode::EqString
        | OpCode::EqDecimal
        | OpCode::GetProp
        | OpCode::And
        | OpCode::Or => (2, 1),
        OpCode::Dup => (1, 2),
        OpCode::Swap => (2, 2),
        OpCode::Pop
        | OpCode::StoreLocal
        | OpCode::StoreLocalTyped
        | OpCode::StoreModuleBinding
        | OpCode::StoreModuleBindingTyped
        | OpCode::StoreClosure
        | OpCode::DerefStore
        | OpCode::DropCall
        | OpCode::DropCallAsync => (1, 0),
        OpCode::NewArray => (0, 1), // pops elements from stack, pushes array
        _ => return None,
    };
    Some(eff)
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
    fn simple_scalar_replacement_candidate() {
        // let arr = [0, 0]  =>  NewArray(2), StoreLocal(0)
        // arr[0] = 42       =>  PushConst(0=index0), PushConst(1=value42), SetLocalIndex(0)
        // x = arr[1]        =>  LoadLocal(0), PushConst(2=index1), GetProp
        let program = make_program(
            vec![
                make_instr(OpCode::NewArray, Some(Operand::Count(2))),    // 0
                make_instr(OpCode::StoreLocal, Some(Operand::Local(0))), // 1
                make_instr(OpCode::PushConst, Some(Operand::Const(0))), // 2: index 0
                make_instr(OpCode::PushConst, Some(Operand::Const(2))), // 3: value 42
                make_instr(OpCode::SetLocalIndex, Some(Operand::Local(0))), // 4
                make_instr(OpCode::LoadLocal, Some(Operand::Local(0))), // 5
                make_instr(OpCode::PushConst, Some(Operand::Const(1))), // 6: index 1
                make_instr(OpCode::GetProp, None),                       // 7
                make_instr(OpCode::Pop, None),                           // 8
            ],
            vec![
                Constant::Int(0), // const 0: index 0
                Constant::Int(1), // const 1: index 1
                Constant::Int(42), // const 2: value 42
            ],
        );

        let plan = analyze_escape(&program);
        assert!(plan.has_candidates());
        let entry = plan.scalar_arrays.get(&0).expect("should have candidate at idx 0");
        assert_eq!(entry.local_slot, 0);
        assert_eq!(entry.element_count, 2);
        assert_eq!(entry.set_sites.get(&4), Some(&0)); // SetLocalIndex at 4, element 0
        assert_eq!(entry.get_sites.get(&7), Some(&1)); // GetProp at 7, element 1
    }

    #[test]
    fn array_escapes_via_call() {
        let program = make_program(
            vec![
                make_instr(OpCode::NewArray, Some(Operand::Count(2))),    // 0
                make_instr(OpCode::StoreLocal, Some(Operand::Local(0))), // 1
                make_instr(OpCode::LoadLocal, Some(Operand::Local(0))), // 2
                make_instr(OpCode::Call, Some(Operand::Count(1))),      // 3: escaping call
            ],
            vec![],
        );

        let plan = analyze_escape(&program);
        assert!(!plan.has_candidates());
    }

    #[test]
    fn array_too_large_rejected() {
        let program = make_program(
            vec![
                make_instr(OpCode::NewArray, Some(Operand::Count(9))),    // > MAX_SCALAR_ARRAY_ELEMENTS
                make_instr(OpCode::StoreLocal, Some(Operand::Local(0))),
                make_instr(OpCode::LoadLocal, Some(Operand::Local(0))),
                make_instr(OpCode::PushConst, Some(Operand::Const(0))),
                make_instr(OpCode::GetProp, None),
                make_instr(OpCode::Pop, None),
            ],
            vec![Constant::Int(0)],
        );

        let plan = analyze_escape(&program);
        assert!(!plan.has_candidates());
    }

    #[test]
    fn array_escapes_via_return() {
        let program = make_program(
            vec![
                make_instr(OpCode::NewArray, Some(Operand::Count(2))),
                make_instr(OpCode::StoreLocal, Some(Operand::Local(0))),
                make_instr(OpCode::LoadLocal, Some(Operand::Local(0))),
                make_instr(OpCode::PushConst, Some(Operand::Const(0))),
                make_instr(OpCode::GetProp, None),
                make_instr(OpCode::ReturnValue, None),
            ],
            vec![Constant::Int(0)],
        );

        let plan = analyze_escape(&program);
        assert!(!plan.has_candidates());
    }

    #[test]
    fn array_escapes_at_block_boundary() {
        let program = make_program(
            vec![
                make_instr(OpCode::NewArray, Some(Operand::Count(2))),
                make_instr(OpCode::StoreLocal, Some(Operand::Local(0))),
                make_instr(OpCode::Jump, Some(Operand::Offset(0))),       // block boundary
                make_instr(OpCode::LoadLocal, Some(Operand::Local(0))),   // in new block
                make_instr(OpCode::PushConst, Some(Operand::Const(0))),
                make_instr(OpCode::GetProp, None),
                make_instr(OpCode::Pop, None),
            ],
            vec![Constant::Int(0)],
        );

        let plan = analyze_escape(&program);
        assert!(!plan.has_candidates());
    }

    #[test]
    fn no_uses_not_scalarized() {
        let program = make_program(
            vec![
                make_instr(OpCode::NewArray, Some(Operand::Count(2))),
                make_instr(OpCode::StoreLocal, Some(Operand::Local(0))),
                make_instr(OpCode::PushNull, None),
                make_instr(OpCode::Pop, None),
            ],
            vec![],
        );

        let plan = analyze_escape(&program);
        // No get/set sites => not worth scalarizing.
        assert!(!plan.has_candidates());
    }

    #[test]
    fn array_escapes_via_array_push() {
        let program = make_program(
            vec![
                make_instr(OpCode::NewArray, Some(Operand::Count(2))),
                make_instr(OpCode::StoreLocal, Some(Operand::Local(0))),
                make_instr(OpCode::PushConst, Some(Operand::Const(0))),
                make_instr(OpCode::ArrayPushLocal, Some(Operand::Local(0))),
            ],
            vec![Constant::Int(42)],
        );

        let plan = analyze_escape(&program);
        assert!(!plan.has_candidates());
    }
}
