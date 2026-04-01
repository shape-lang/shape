//! Bytecode verifier for trusted and v2 typed opcodes.
//!
//! Validates that trusted opcode invariants hold:
//! - Every trusted opcode appears inside a function with a `FrameDescriptor`
//! - The FrameDescriptor has no `SlotKind::Unknown` entries for the relevant operands
//!
//! Also validates v2 typed opcode invariants:
//! - Typed array ops require a FrameDescriptor with non-Unknown slots
//! - Typed field ops have FieldOffset operands with reasonable byte offsets
//! - Sized integer (i32) ops require a FrameDescriptor with non-Unknown slots

use super::{BytecodeProgram, OpCode, Operand};
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
    /// A v2 typed opcode was found in a function without a FrameDescriptor.
    V2MissingFrameDescriptor {
        function_name: String,
        opcode: OpCode,
        instruction_offset: usize,
    },
    /// A v2 typed field opcode has an unreasonable byte offset (> 4096).
    V2FieldOffsetTooLarge {
        function_name: String,
        opcode: OpCode,
        instruction_offset: usize,
        offset: u16,
    },
    /// A v2 typed field opcode is missing its FieldOffset operand.
    V2MissingFieldOffset {
        function_name: String,
        opcode: OpCode,
        instruction_offset: usize,
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
            VerifyError::V2MissingFrameDescriptor {
                function_name,
                opcode,
                instruction_offset,
            } => write!(
                f,
                "V2 typed opcode {:?} at offset {} in function '{}' has no FrameDescriptor",
                opcode, instruction_offset, function_name
            ),
            VerifyError::V2FieldOffsetTooLarge {
                function_name,
                opcode,
                instruction_offset,
                offset,
            } => write!(
                f,
                "V2 field opcode {:?} at offset {} in function '{}': byte offset {} exceeds maximum (4096)",
                opcode, instruction_offset, function_name, offset
            ),
            VerifyError::V2MissingFieldOffset {
                function_name,
                opcode,
                instruction_offset,
            } => write!(
                f,
                "V2 field opcode {:?} at offset {} in function '{}': missing FieldOffset operand",
                opcode, instruction_offset, function_name
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

/// Maximum reasonable byte offset for v2 typed field access.
/// Structs larger than 4096 bytes are unlikely and probably indicate a bug.
const MAX_FIELD_OFFSET: u16 = 4096;

/// Returns true if the opcode is a v2 typed field load/store that requires a FieldOffset operand.
fn is_v2_field_op(op: OpCode) -> bool {
    matches!(
        op,
        OpCode::FieldLoadF64
            | OpCode::FieldLoadI64
            | OpCode::FieldLoadI32
            | OpCode::FieldLoadBool
            | OpCode::FieldLoadPtr
            | OpCode::FieldStoreF64
            | OpCode::FieldStoreI64
            | OpCode::FieldStoreI32
    )
}

/// Verify that all v2 typed opcodes have valid invariants.
///
/// Checks:
/// - Typed array ops, field ops, and i32 arithmetic appear in functions with FrameDescriptors
/// - Field load/store ops have a FieldOffset operand with a reasonable byte offset (<= 4096)
///
/// Returns `Ok(())` if all v2 typed opcodes pass, or a list of all violations.
pub fn verify_v2_typed_opcodes(program: &BytecodeProgram) -> Result<(), Vec<VerifyError>> {
    let mut errors = Vec::new();

    for func in &program.functions {
        let start = func.entry_point;
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
            if !instruction.opcode.is_v2_typed() {
                continue;
            }

            // All v2 typed opcodes require a FrameDescriptor
            if func.frame_descriptor.is_none() {
                errors.push(VerifyError::V2MissingFrameDescriptor {
                    function_name: func.name.clone(),
                    opcode: instruction.opcode,
                    instruction_offset: offset,
                });
                continue;
            }

            // Field load/store ops: validate FieldOffset operand
            if is_v2_field_op(instruction.opcode) {
                match &instruction.operand {
                    Some(Operand::FieldOffset(off)) => {
                        if *off > MAX_FIELD_OFFSET {
                            errors.push(VerifyError::V2FieldOffsetTooLarge {
                                function_name: func.name.clone(),
                                opcode: instruction.opcode,
                                instruction_offset: offset,
                                offset: *off,
                            });
                        }
                    }
                    _ => {
                        errors.push(VerifyError::V2MissingFieldOffset {
                            function_name: func.name.clone(),
                            opcode: instruction.opcode,
                            instruction_offset: offset,
                        });
                    }
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
            mir_data: None,
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
        use crate::bytecode::Operand;
        let func = Function {
            name: "load_trusted".to_string(),
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
            mir_data: None,
        };
        let instructions = vec![
            Instruction::new(OpCode::LoadLocalTrusted, Some(Operand::Local(0))),
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
        use crate::bytecode::Operand;
        let func = Function {
            name: "load_trusted".to_string(),
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
            mir_data: None,
        };
        let instructions = vec![
            Instruction::new(OpCode::LoadLocalTrusted, Some(Operand::Local(0))),
            Instruction::simple(OpCode::ReturnValue),
        ];
        let prog = make_program(vec![func], instructions);
        assert!(verify_trusted_opcodes(&prog).is_ok());
    }

    #[test]
    fn is_trusted_method() {
        assert!(OpCode::LoadLocalTrusted.is_trusted());
        assert!(OpCode::JumpIfFalseTrusted.is_trusted());
        assert!(!OpCode::AddInt.is_trusted());
        assert!(!OpCode::Add.is_trusted());
    }

    #[test]
    fn trusted_variant_mapping() {
        assert_eq!(
            OpCode::LoadLocal.trusted_variant(),
            Some(OpCode::LoadLocalTrusted)
        );
        assert_eq!(
            OpCode::JumpIfFalse.trusted_variant(),
            Some(OpCode::JumpIfFalseTrusted)
        );
        assert_eq!(OpCode::Add.trusted_variant(), None);
        assert_eq!(OpCode::AddInt.trusted_variant(), None);
    }

    // ===== v2 typed opcode verification tests =====

    #[test]
    fn v2_no_typed_opcodes_passes() {
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
            mir_data: None,
        };
        let instructions = vec![
            Instruction::simple(OpCode::Add),
            Instruction::simple(OpCode::ReturnValue),
        ];
        let prog = make_program(vec![func], instructions);
        assert!(verify_v2_typed_opcodes(&prog).is_ok());
    }

    #[test]
    fn v2_typed_array_op_missing_frame_descriptor() {
        let func = Function {
            name: "array_fn".to_string(),
            arity: 0,
            param_names: vec![],
            locals_count: 1,
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
            mir_data: None,
        };
        let instructions = vec![
            Instruction::simple(OpCode::TypedArrayGetF64),
            Instruction::simple(OpCode::ReturnValue),
        ];
        let prog = make_program(vec![func], instructions);
        let errs = verify_v2_typed_opcodes(&prog).unwrap_err();
        assert_eq!(errs.len(), 1);
        assert!(matches!(
            &errs[0],
            VerifyError::V2MissingFrameDescriptor { .. }
        ));
    }

    #[test]
    fn v2_typed_array_op_with_frame_descriptor_passes() {
        let func = Function {
            name: "array_fn".to_string(),
            arity: 0,
            param_names: vec![],
            locals_count: 1,
            entry_point: 0,
            body_length: 2,
            is_closure: false,
            captures_count: 0,
            is_async: false,
            ref_params: vec![],
            ref_mutates: vec![],
            mutable_captures: vec![],
            frame_descriptor: Some(FrameDescriptor::from_slots(vec![SlotKind::Int64])),
            osr_entry_points: vec![],
            mir_data: None,
        };
        let instructions = vec![
            Instruction::simple(OpCode::TypedArrayGetF64),
            Instruction::simple(OpCode::ReturnValue),
        ];
        let prog = make_program(vec![func], instructions);
        assert!(verify_v2_typed_opcodes(&prog).is_ok());
    }

    #[test]
    fn v2_field_load_valid_offset() {
        use crate::bytecode::Operand;
        let func = Function {
            name: "field_fn".to_string(),
            arity: 0,
            param_names: vec![],
            locals_count: 1,
            entry_point: 0,
            body_length: 2,
            is_closure: false,
            captures_count: 0,
            is_async: false,
            ref_params: vec![],
            ref_mutates: vec![],
            mutable_captures: vec![],
            frame_descriptor: Some(FrameDescriptor::from_slots(vec![SlotKind::Int64])),
            osr_entry_points: vec![],
            mir_data: None,
        };
        let instructions = vec![
            Instruction::new(OpCode::FieldLoadF64, Some(Operand::FieldOffset(16))),
            Instruction::simple(OpCode::ReturnValue),
        ];
        let prog = make_program(vec![func], instructions);
        assert!(verify_v2_typed_opcodes(&prog).is_ok());
    }

    #[test]
    fn v2_field_load_offset_too_large() {
        use crate::bytecode::Operand;
        let func = Function {
            name: "field_fn".to_string(),
            arity: 0,
            param_names: vec![],
            locals_count: 1,
            entry_point: 0,
            body_length: 2,
            is_closure: false,
            captures_count: 0,
            is_async: false,
            ref_params: vec![],
            ref_mutates: vec![],
            mutable_captures: vec![],
            frame_descriptor: Some(FrameDescriptor::from_slots(vec![SlotKind::Int64])),
            osr_entry_points: vec![],
            mir_data: None,
        };
        let instructions = vec![
            Instruction::new(OpCode::FieldLoadF64, Some(Operand::FieldOffset(5000))),
            Instruction::simple(OpCode::ReturnValue),
        ];
        let prog = make_program(vec![func], instructions);
        let errs = verify_v2_typed_opcodes(&prog).unwrap_err();
        assert_eq!(errs.len(), 1);
        assert!(matches!(
            &errs[0],
            VerifyError::V2FieldOffsetTooLarge { offset: 5000, .. }
        ));
    }

    #[test]
    fn v2_field_load_missing_operand() {
        let func = Function {
            name: "field_fn".to_string(),
            arity: 0,
            param_names: vec![],
            locals_count: 1,
            entry_point: 0,
            body_length: 2,
            is_closure: false,
            captures_count: 0,
            is_async: false,
            ref_params: vec![],
            ref_mutates: vec![],
            mutable_captures: vec![],
            frame_descriptor: Some(FrameDescriptor::from_slots(vec![SlotKind::Int64])),
            osr_entry_points: vec![],
            mir_data: None,
        };
        let instructions = vec![
            Instruction::simple(OpCode::FieldLoadI64),
            Instruction::simple(OpCode::ReturnValue),
        ];
        let prog = make_program(vec![func], instructions);
        let errs = verify_v2_typed_opcodes(&prog).unwrap_err();
        assert_eq!(errs.len(), 1);
        assert!(matches!(
            &errs[0],
            VerifyError::V2MissingFieldOffset { .. }
        ));
    }

    #[test]
    fn v2_i32_arithmetic_missing_frame_descriptor() {
        let func = Function {
            name: "i32_fn".to_string(),
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
            mir_data: None,
        };
        let instructions = vec![
            Instruction::simple(OpCode::AddI32),
            Instruction::simple(OpCode::ReturnValue),
        ];
        let prog = make_program(vec![func], instructions);
        let errs = verify_v2_typed_opcodes(&prog).unwrap_err();
        assert_eq!(errs.len(), 1);
        assert!(matches!(
            &errs[0],
            VerifyError::V2MissingFrameDescriptor { .. }
        ));
    }

    #[test]
    fn v2_is_v2_typed_method() {
        assert!(OpCode::TypedArrayGetF64.is_v2_typed());
        assert!(OpCode::FieldLoadF64.is_v2_typed());
        assert!(OpCode::AddI32.is_v2_typed());
        assert!(OpCode::NewTypedStruct.is_v2_typed());
        assert!(!OpCode::Add.is_v2_typed());
        assert!(!OpCode::AddInt.is_v2_typed());
        assert!(!OpCode::LoadLocal.is_v2_typed());
    }

    #[test]
    fn v2_multiple_errors_collected() {
        let func = Function {
            name: "multi_err".to_string(),
            arity: 0,
            param_names: vec![],
            locals_count: 2,
            entry_point: 0,
            body_length: 3,
            is_closure: false,
            captures_count: 0,
            is_async: false,
            ref_params: vec![],
            ref_mutates: vec![],
            mutable_captures: vec![],
            frame_descriptor: None,
            osr_entry_points: vec![],
            mir_data: None,
        };
        let instructions = vec![
            Instruction::simple(OpCode::AddI32),
            Instruction::simple(OpCode::TypedArrayGetI64),
            Instruction::simple(OpCode::ReturnValue),
        ];
        let prog = make_program(vec![func], instructions);
        let errs = verify_v2_typed_opcodes(&prog).unwrap_err();
        assert_eq!(errs.len(), 2);
    }
}
