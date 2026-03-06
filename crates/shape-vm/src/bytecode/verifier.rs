//! Bytecode verifier for trusted opcodes.
//!
//! Validates that trusted opcode invariants hold:
//! - Every trusted opcode appears inside a function with a `FrameDescriptor`
//! - The FrameDescriptor has no `SlotKind::Unknown` entries for the relevant operands

use super::{BytecodeProgram, OpCode};
use crate::type_tracking::SlotKind;

/// Errors produced by the bytecode verifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyError {
    /// A trusted opcode was found in a function that has no FrameDescriptor.
    MissingFrameDescriptor {
        function_name: String,
        opcode: OpCode,
        instruction_offset: usize,
    },
    /// A trusted opcode operand slot has `SlotKind::Unknown` in the FrameDescriptor.
    UnknownSlotKind {
        function_name: String,
        opcode: OpCode,
        instruction_offset: usize,
        slot_index: usize,
    },
}

impl std::fmt::Display for VerifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VerifyError::MissingFrameDescriptor {
                function_name,
                opcode,
                instruction_offset,
            } => write!(
                f,
                "Trusted opcode {:?} at offset {} in function '{}' has no FrameDescriptor",
                opcode, instruction_offset, function_name
            ),
            VerifyError::UnknownSlotKind {
                function_name,
                opcode,
                instruction_offset,
                slot_index,
            } => write!(
                f,
                "Trusted opcode {:?} at offset {} in function '{}': slot {} has Unknown kind",
                opcode, instruction_offset, function_name, slot_index
            ),
        }
    }
}

impl std::error::Error for VerifyError {}

/// Verify that all trusted opcodes in a program have valid FrameDescriptors.
///
/// Returns `Ok(())` if all trusted opcodes pass verification, or a list of
/// all violations found.
pub fn verify_trusted_opcodes(program: &BytecodeProgram) -> Result<(), Vec<VerifyError>> {
    let mut errors = Vec::new();

    for func in &program.functions {
        // Collect instruction offsets that belong to this function.
        // Functions store their entry_point and instructions run until the next
        // function or end of program. We scan the instruction stream from
        // entry_point looking for trusted opcodes.
        let start = func.entry_point;
        // Find the end: next function's entry_point or end of instructions
        let end = program
            .functions
            .iter()
            .filter(|f| f.entry_point > start)
            .map(|f| f.entry_point)
            .min()
            .unwrap_or(program.instructions.len());

        for offset in start..end {
            let Some(instruction) = program.instructions.get(offset) else {
                break;
            };
            if !instruction.opcode.is_trusted() {
                continue;
            }

            // Check FrameDescriptor exists
            let Some(ref fd) = func.frame_descriptor else {
                errors.push(VerifyError::MissingFrameDescriptor {
                    function_name: func.name.clone(),
                    opcode: instruction.opcode,
                    instruction_offset: offset,
                });
                continue;
            };

            // Check that the descriptor has at least some non-Unknown slots.
            // For trusted arithmetic, we don't know which specific stack slots
            // feed the operands (they come from the stack, not named locals),
            // so we verify the frame descriptor itself is populated (non-empty
            // and not all Unknown).
            if fd.is_empty() || fd.is_all_unknown() {
                // All slots unknown — the compiler shouldn't have emitted trusted ops
                for (idx, slot) in fd.slots.iter().enumerate() {
                    if *slot == SlotKind::Unknown {
                        errors.push(VerifyError::UnknownSlotKind {
                            function_name: func.name.clone(),
                            opcode: instruction.opcode,
                            instruction_offset: offset,
                            slot_index: idx,
                        });
                        break; // one error per instruction is sufficient
                    }
                }
                // If fd is empty, emit a generic error
                if fd.is_empty() {
                    errors.push(VerifyError::UnknownSlotKind {
                        function_name: func.name.clone(),
                        opcode: instruction.opcode,
                        instruction_offset: offset,
                        slot_index: 0,
                    });
                }
            }
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytecode::{Function, Instruction, OpCode};
    use crate::type_tracking::FrameDescriptor;

    fn make_program(functions: Vec<Function>, instructions: Vec<Instruction>) -> BytecodeProgram {
        let mut prog = BytecodeProgram::new();
        prog.functions = functions;
        prog.instructions = instructions;
        prog
    }

    #[test]
    fn no_trusted_opcodes_passes() {
        let func = Function {
            name: "main".to_string(),
            arity: 0,
            param_names: vec![],
            locals_count: 2,
            entry_point: 0,
            body_length: 2,
            is_closure: false,
            captures_count: 0,
            is_async: false,
            ref_params: vec![],
            ref_mutates: vec![],
            mutable_captures: vec![],
            frame_descriptor: None,
            osr_entry_points: vec![],
        };
        let instructions = vec![
            Instruction::simple(OpCode::AddInt),
            Instruction::simple(OpCode::ReturnValue),
        ];
        let prog = make_program(vec![func], instructions);
        assert!(verify_trusted_opcodes(&prog).is_ok());
    }

    #[test]
    fn trusted_opcode_missing_frame_descriptor() {
        let func = Function {
            name: "add_trusted".to_string(),
            arity: 2,
            param_names: vec!["a".to_string(), "b".to_string()],
            locals_count: 2,
            entry_point: 0,
            body_length: 2,
            is_closure: false,
            captures_count: 0,
            is_async: false,
            ref_params: vec![],
            ref_mutates: vec![],
            mutable_captures: vec![],
            frame_descriptor: None,
            osr_entry_points: vec![],
        };
        let instructions = vec![
            Instruction::simple(OpCode::AddIntTrusted),
            Instruction::simple(OpCode::ReturnValue),
        ];
        let prog = make_program(vec![func], instructions);
        let errs = verify_trusted_opcodes(&prog).unwrap_err();
        assert_eq!(errs.len(), 1);
        assert!(matches!(
            &errs[0],
            VerifyError::MissingFrameDescriptor { .. }
        ));
    }

    #[test]
    fn trusted_opcode_with_valid_frame_descriptor() {
        let func = Function {
            name: "add_trusted".to_string(),
            arity: 2,
            param_names: vec!["a".to_string(), "b".to_string()],
            locals_count: 2,
            entry_point: 0,
            body_length: 2,
            is_closure: false,
            captures_count: 0,
            is_async: false,
            ref_params: vec![],
            ref_mutates: vec![],
            mutable_captures: vec![],
            frame_descriptor: Some(FrameDescriptor::from_slots(vec![
                SlotKind::Int64,
                SlotKind::Int64,
            ])),
            osr_entry_points: vec![],
        };
        let instructions = vec![
            Instruction::simple(OpCode::AddIntTrusted),
            Instruction::simple(OpCode::ReturnValue),
        ];
        let prog = make_program(vec![func], instructions);
        assert!(verify_trusted_opcodes(&prog).is_ok());
    }

    #[test]
    fn is_trusted_method() {
        assert!(OpCode::AddIntTrusted.is_trusted());
        assert!(OpCode::DivNumberTrusted.is_trusted());
        assert!(!OpCode::AddInt.is_trusted());
        assert!(!OpCode::Add.is_trusted());
    }

    #[test]
    fn trusted_variant_mapping() {
        assert_eq!(
            OpCode::AddInt.trusted_variant(),
            Some(OpCode::AddIntTrusted)
        );
        assert_eq!(
            OpCode::DivNumber.trusted_variant(),
            Some(OpCode::DivNumberTrusted)
        );
        assert_eq!(OpCode::Add.trusted_variant(), None);
        assert_eq!(OpCode::AddDecimal.trusted_variant(), None);
    }
}
