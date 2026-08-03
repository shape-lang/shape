//! Bytecode verifier for trusted and v2 typed opcodes.
//!
//! Validates that trusted opcode invariants hold:
//! - Every trusted opcode carries the operand shape its handler requires
//!   (`LoadLocalTrusted` → `Operand::Local`, `JumpIfFalseTrusted` →
//!   `Operand::Offset`).
//!
//! Also validates v2 typed opcode invariants:
//! - Typed array ops require a FrameDescriptor with non-Unknown slots
//! - Typed field ops have FieldOffset operands with reasonable byte offsets
//! - Sized integer (i32) ops require a FrameDescriptor with non-Unknown slots
//!
//! ## WS-10b — stale `MissingFrameDescriptor` rule removed (2026-05-22)
//!
//! `verify_trusted_opcodes` previously errored `MissingFrameDescriptor` /
//! `UnknownSlotKind` for any trusted opcode in a function whose
//! `Function.frame_descriptor` was `None` / empty. That rule encoded the
//! **pre-ADR-006 §2.7.7 trusted-opcode contract**, when trusted opcodes
//! skipped runtime tag-bit validation and relied on descriptor-supplied
//! slot-kind metadata to justify the skip.
//!
//! Post-§2.7.7 only two trusted opcodes survive — `LoadLocalTrusted`
//! (0xD7) and `JumpIfFalseTrusted` (0xD8). Their executors
//! (`executor/variables/mod.rs::op_load_local_trusted`,
//! `executor/control_flow/mod.rs::op_jump_if_false_trusted`) source slot
//! kind from the §2.7.7 stack parallel-`Vec<NativeKind>` track, NOT the
//! `FrameDescriptor`. `LoadLocalTrusted` is byte-for-byte identical to
//! non-trusted `LoadLocal`; `JumpIfFalseTrusted` pops the kinded
//! condition slot directly. `current_frame_descriptor()` has zero VM
//! executor call sites — the descriptor is consumed at runtime only by
//! the JIT, which already has an explicit absent-descriptor fallback
//! (`shape-jit::worker.rs`, `mir_compiler/v2_call_abi.rs`).
//!
//! The stale rule fired 16 false positives on every program run (stdlib
//! prelude functions with an unannotated / `any`-typed local whose whole
//! frame the storage-hint pass could not prove). It enforced nothing —
//! `load_program` only `eprintln!`'d — but printed "Bytecode verification
//! failed" on a clean prelude. The rule is dropped; `verify_trusted_opcodes`
//! now verifies the still-meaningful invariant (operand shape). The
//! `verify_v2_typed_opcodes` pass — which checks real v2 invariants — is
//! unchanged and keeps its enforcement structure.
//!
//! ## #260 — extents come from `body_length`, not from the next entry point
//!
//! Both passes used to derive a function's instruction range as
//! `entry_point → next function's entry_point`, never consulting
//! `body_length`. That is wrong for any zero-length row: the permission
//! carriers synthesized by `publish_dependency_permission_blob`
//! (`compiler/import_permissions.rs`) have `body_length: 0` and
//! `frame_descriptor: None`, so each one inherited the entire instruction
//! range of the following real function and reported that function's V2
//! typed opcodes as its own violations.
//!
//! That accounted for the whole observed violation class. Measured over the
//! 488-program vm/jit corpus (518 program-load blocks, 76,242 function rows):
//! all 26,772 zero-length rows are permission carriers, every one of them
//! `frame_descriptor: None`, and 26,528 of them inherited a non-empty range.
//! Because the carrier set varies with `HashMap` iteration order upstream
//! (`import_permissions.rs`), the reported violation count varied run to run
//! on identical input — nine distinct counts over 40 runs of a three-line
//! program.
//!
//! Extents now come from `BytecodeProgram::direct_function_windows`, the
//! ownership notion the crate already uses elsewhere: an instruction belongs
//! to the innermost function whose `[entry_point, entry_point + body_length)`
//! window contains it, and empty rows own nothing. On the same corpus this is
//! byte-identical to the old rule for every one of the 49,470 real functions
//! (`entry_point + body_length == next entry_point` in all 49,470 cases, no
//! nested windows observed), so the change subtracts the false positives
//! without relaxing anything that was being checked.
//!
//! It does not silence real violations: `__main__` in three corpus programs
//! genuinely carries V2 typed opcodes inside its own window with
//! `frame_descriptor: None`, and is still reported.

use std::ops::Range;

use super::{BytecodeProgram, OpCode, Operand};

/// Errors produced by the bytecode verifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyError {
    /// A trusted opcode carries the wrong operand shape for its handler
    /// (e.g. `LoadLocalTrusted` without an `Operand::Local`).
    TrustedOpcodeBadOperand {
        function_name: String,
        opcode: OpCode,
        instruction_offset: usize,
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
    /// The function table does not describe unambiguous instruction ownership
    /// (a window overflows, leaves the instruction stream, or overlaps another
    /// without containing it). No function's extent can be derived, so neither
    /// pass can verify anything; reported once per program.
    MalformedFunctionWindows { detail: String },
}

impl std::fmt::Display for VerifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VerifyError::TrustedOpcodeBadOperand {
                function_name,
                opcode,
                instruction_offset,
            } => write!(
                f,
                "Trusted opcode {:?} at offset {} in function '{}' has the wrong operand shape",
                opcode, instruction_offset, function_name
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
            VerifyError::MalformedFunctionWindows { detail } => write!(
                f,
                "function table does not describe unambiguous instruction ownership: {detail}"
            ),
        }
    }
}

/// Instruction offsets directly owned by each function, in table order.
///
/// See the `#260` section of the module docs: ownership is
/// `[entry_point, entry_point + body_length)` minus any contained function
/// windows, so zero-length rows own nothing instead of inheriting the next
/// function's range. A malformed function table is a single program-level
/// error rather than one error per function.
fn direct_windows_per_function(
    program: &BytecodeProgram,
) -> Result<Vec<Vec<Range<usize>>>, VerifyError> {
    (0..program.functions.len())
        .map(|index| {
            program.direct_function_windows(index).map_err(|error| {
                VerifyError::MalformedFunctionWindows {
                    detail: error.to_string(),
                }
            })
        })
        .collect()
}

impl std::error::Error for VerifyError {}

/// Verify that all trusted opcodes in a program are well-formed.
///
/// Each function is checked over the instructions it directly owns (see the
/// `#260` module-doc section), so a zero-length row owns nothing and a
/// violation names the function that actually contains it.
///
/// Post-ADR-006 §2.7.7 the two surviving trusted opcodes (`LoadLocalTrusted`,
/// `JumpIfFalseTrusted`) source slot kind from the stack
/// parallel-`Vec<NativeKind>` track, not the `FrameDescriptor` — see the
/// module doc comment (`WS-10b`). The descriptor-presence rule that used to
/// live here was stale (it fired 16 false positives on a clean stdlib
/// prelude) and is removed. The verifier still checks the invariant that is
/// still real: each trusted opcode carries the operand shape its executor
/// requires, so a future malformed trusted opcode still surfaces.
///
/// Returns `Ok(())` if all trusted opcodes pass verification, or a list of
/// all violations found.
pub fn verify_trusted_opcodes(program: &BytecodeProgram) -> Result<(), Vec<VerifyError>> {
    let mut errors = Vec::new();
    let windows = match direct_windows_per_function(program) {
        Ok(windows) => windows,
        Err(error) => return Err(vec![error]),
    };

    for (func, owned) in program.functions.iter().zip(&windows) {
        for offset in owned.iter().cloned().flatten() {
            let Some(instruction) = program.instructions.get(offset) else {
                break;
            };
            if !instruction.opcode.is_trusted() {
                continue;
            }

            // Operand-shape check — the still-meaningful trusted-opcode
            // invariant. `LoadLocalTrusted` indexes a local; `JumpIfFalseTrusted`
            // branches by a signed offset. A mismatch is a real malformed-bytecode
            // bug the executor would reject with `VMError::InvalidOperand` at
            // runtime; surface it statically.
            let operand_ok = match instruction.opcode {
                OpCode::LoadLocalTrusted => {
                    matches!(instruction.operand, Some(Operand::Local(_)))
                }
                OpCode::JumpIfFalseTrusted => {
                    matches!(instruction.operand, Some(Operand::Offset(_)))
                }
                // Any future trusted opcode without an operand-shape rule
                // is conservatively accepted here; add its rule alongside
                // its executor.
                _ => true,
            };
            if !operand_ok {
                errors.push(VerifyError::TrustedOpcodeBadOperand {
                    function_name: func.name.clone(),
                    opcode: instruction.opcode,
                    instruction_offset: offset,
                });
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
/// Each function is checked over the instructions it directly owns (see the
/// `#260` module-doc section), so a zero-length permission carrier owns
/// nothing instead of inheriting the next function's range.
///
/// Returns `Ok(())` if all v2 typed opcodes pass, or a list of all violations.
pub fn verify_v2_typed_opcodes(program: &BytecodeProgram) -> Result<(), Vec<VerifyError>> {
    let mut errors = Vec::new();
    let windows = match direct_windows_per_function(program) {
        Ok(windows) => windows,
        Err(error) => return Err(vec![error]),
    };

    for (func, owned) in program.functions.iter().zip(&windows) {
        for offset in owned.iter().cloned().flatten() {
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
    use crate::type_tracking::{FrameDescriptor, NativeKind};

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

    /// WS-10b: post-ADR-006 §2.7.7 a trusted opcode in a function with NO
    /// `FrameDescriptor` is NOT a violation — the surviving trusted opcodes
    /// (`LoadLocalTrusted`, `JumpIfFalseTrusted`) source slot kind from the
    /// §2.7.7 stack parallel-`NativeKind` track, not the descriptor. The
    /// stale `MissingFrameDescriptor` rule that fired 16 false positives on
    /// every program run is removed. This is the regression guard for that
    /// removal: well-formed trusted opcodes with `frame_descriptor: None`
    /// pass clean.
    #[test]
    fn trusted_opcode_no_frame_descriptor_is_not_a_violation() {
        use crate::bytecode::Operand;
        let func = Function {
            name: "load_trusted".to_string(),
            arity: 2,
            param_names: vec!["a".to_string(), "b".to_string()],
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
            Instruction::new(OpCode::LoadLocalTrusted, Some(Operand::Local(0))),
            Instruction::new(OpCode::JumpIfFalseTrusted, Some(Operand::Offset(1))),
            Instruction::simple(OpCode::ReturnValue),
        ];
        let prog = make_program(vec![func], instructions);
        assert!(
            verify_trusted_opcodes(&prog).is_ok(),
            "trusted opcodes with no FrameDescriptor must pass (WS-10b stale-rule removal)"
        );
    }

    /// WS-10b: the still-meaningful trusted-opcode invariant — operand shape.
    /// `LoadLocalTrusted` must carry an `Operand::Local`; a wrong operand
    /// shape is a real malformed-bytecode bug and is still surfaced.
    #[test]
    fn trusted_opcode_bad_operand_is_a_violation() {
        use crate::bytecode::Operand;
        let func = Function {
            name: "bad_load".to_string(),
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
            frame_descriptor: Some(FrameDescriptor::from_slots(
                vec![NativeKind::Int64],
                crate::type_tracking::FrameReturnWrapper::Plain,
            )),
            osr_entry_points: vec![],
            mir_data: None,
        };
        // LoadLocalTrusted carrying an Offset operand instead of Local — malformed.
        let instructions = vec![
            Instruction::new(OpCode::LoadLocalTrusted, Some(Operand::Offset(3))),
            Instruction::simple(OpCode::ReturnValue),
        ];
        let prog = make_program(vec![func], instructions);
        let errs = verify_trusted_opcodes(&prog).unwrap_err();
        assert_eq!(errs.len(), 1);
        assert!(matches!(
            &errs[0],
            VerifyError::TrustedOpcodeBadOperand { .. }
        ));
    }

    #[test]
    fn trusted_opcode_with_valid_operand_passes() {
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
            frame_descriptor: Some(FrameDescriptor::from_slots(
                vec![NativeKind::Int64, NativeKind::Int64],
                crate::type_tracking::FrameReturnWrapper::Plain,
            )),
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
        assert!(!OpCode::Halt.is_trusted());
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
        assert_eq!(OpCode::Halt.trusted_variant(), None);
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
            Instruction::simple(OpCode::PushNull),
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
            frame_descriptor: Some(FrameDescriptor::from_slots(
                vec![NativeKind::Int64],
                crate::type_tracking::FrameReturnWrapper::Plain,
            )),
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
            frame_descriptor: Some(FrameDescriptor::from_slots(
                vec![NativeKind::Int64],
                crate::type_tracking::FrameReturnWrapper::Plain,
            )),
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
            frame_descriptor: Some(FrameDescriptor::from_slots(
                vec![NativeKind::Int64],
                crate::type_tracking::FrameReturnWrapper::Plain,
            )),
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
            frame_descriptor: Some(FrameDescriptor::from_slots(
                vec![NativeKind::Int64],
                crate::type_tracking::FrameReturnWrapper::Plain,
            )),
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
        assert!(matches!(&errs[0], VerifyError::V2MissingFieldOffset { .. }));
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
        assert!(!OpCode::Halt.is_v2_typed());
        assert!(!OpCode::AddInt.is_v2_typed());
        assert!(!OpCode::LoadLocal.is_v2_typed());
    }

    /// V1.1A: the new MoveLocal/CloneLocal/DropLocal opcodes are not trusted
    /// and are not v2-typed. Both verifier passes should accept them as no-ops
    /// (they pass through without the respective FrameDescriptor requirements).
    /// V1.1B will add an ownership-specific verifier pass; until then, these
    /// opcodes are unreachable in execution, so no verification is required.
    #[test]
    fn v11a_ownership_opcodes_pass_both_verifiers() {
        use crate::bytecode::Operand;
        let func = Function {
            name: "own_fn".to_string(),
            arity: 0,
            param_names: vec![],
            locals_count: 1,
            entry_point: 0,
            body_length: 4,
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
            Instruction::new(OpCode::MoveLocal, Some(Operand::Local(0))),
            Instruction::new(OpCode::CloneLocal, Some(Operand::Local(0))),
            Instruction::new(OpCode::DropLocal, Some(Operand::Local(0))),
            Instruction::simple(OpCode::ReturnValue),
        ];
        let prog = make_program(vec![func], instructions);
        assert!(
            verify_trusted_opcodes(&prog).is_ok(),
            "V1.1A ownership opcodes should pass trusted verification"
        );
        assert!(
            verify_v2_typed_opcodes(&prog).is_ok(),
            "V1.1A ownership opcodes should pass v2-typed verification"
        );
    }

    /// V1.2A: the new `PromoteToShared` opcode is not trusted and not
    /// v2-typed. Both verifier passes accept it as a no-op — the opcode
    /// operates on top-of-stack with no operand, identical in shape to
    /// `PromoteToOwned`, and needs no FrameDescriptor. V1.2B adds the
    /// handler; until then reaching this opcode panics in dispatch.
    #[test]
    fn v12a_promote_to_shared_passes_both_verifiers() {
        let func = Function {
            name: "promote_shared_fn".to_string(),
            arity: 0,
            param_names: vec![],
            locals_count: 0,
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
            Instruction::simple(OpCode::PromoteToShared),
            Instruction::simple(OpCode::ReturnValue),
        ];
        let prog = make_program(vec![func], instructions);
        assert!(
            verify_trusted_opcodes(&prog).is_ok(),
            "V1.2A PromoteToShared should pass trusted verification"
        );
        assert!(
            verify_v2_typed_opcodes(&prog).is_ok(),
            "V1.2A PromoteToShared should pass v2-typed verification"
        );
    }

    /// R5.1A: the six new typed bitwise opcodes
    /// (BitAndInt/BitOrInt/BitXorInt/BitShlInt/BitShrInt/BitNotInt) are not
    /// trusted and not v2-typed. Both verifier passes accept them as no-ops
    /// (no FrameDescriptor requirement), matching the behavior of the
    /// existing int-typed arithmetic family (AddInt/SubInt/MulInt). R5.1B
    /// will add executor handlers; until then these opcodes are unreachable
    /// via dispatch — reaching them panics.
    #[test]
    fn r51a_typed_bitwise_opcodes_pass_both_verifiers() {
        let func = Function {
            name: "bitwise_fn".to_string(),
            arity: 0,
            param_names: vec![],
            locals_count: 0,
            entry_point: 0,
            body_length: 7,
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
            Instruction::simple(OpCode::BitAndInt),
            Instruction::simple(OpCode::BitOrInt),
            Instruction::simple(OpCode::BitXorInt),
            Instruction::simple(OpCode::BitShlInt),
            Instruction::simple(OpCode::BitShrInt),
            Instruction::simple(OpCode::BitNotInt),
            Instruction::simple(OpCode::ReturnValue),
        ];
        let prog = make_program(vec![func], instructions);
        assert!(
            verify_trusted_opcodes(&prog).is_ok(),
            "R5.1A typed bitwise opcodes should pass trusted verification"
        );
        assert!(
            verify_v2_typed_opcodes(&prog).is_ok(),
            "R5.1A typed bitwise opcodes should pass v2-typed verification"
        );
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

    // ===== #260: extents come from body_length, not the next entry point =====

    /// A zero-length permission carrier, shaped exactly as
    /// `publish_dependency_permission_blob` builds one: `body_length: 0`,
    /// `frame_descriptor: None`, entry point at the offset the next real
    /// function starts from.
    fn permission_carrier(entry_point: usize) -> Function {
        Function {
            name: "\0shape.module-import-permissions::std::core::index".to_string(),
            arity: 0,
            param_names: vec![],
            locals_count: 0,
            entry_point,
            body_length: 0,
            is_closure: false,
            captures_count: 0,
            is_async: false,
            ref_params: vec![],
            ref_mutates: vec![],
            mutable_captures: vec![],
            frame_descriptor: None,
            osr_entry_points: vec![],
            mir_data: None,
        }
    }

    fn v2_function(
        name: &str,
        entry_point: usize,
        body_length: usize,
        descriptor: bool,
    ) -> Function {
        Function {
            name: name.to_string(),
            arity: 0,
            param_names: vec![],
            locals_count: 1,
            entry_point,
            body_length,
            is_closure: false,
            captures_count: 0,
            is_async: false,
            ref_params: vec![],
            ref_mutates: vec![],
            mutable_captures: vec![],
            frame_descriptor: descriptor.then(|| {
                FrameDescriptor::from_slots(
                    vec![NativeKind::Int64],
                    crate::type_tracking::FrameReturnWrapper::Plain,
                )
            }),
            osr_entry_points: vec![],
            mir_data: None,
        }
    }

    /// The reported #260 class: a zero-length carrier sitting at the entry
    /// point of a real, fully-descriptored function used to inherit that
    /// function's whole instruction range and report its typed opcodes as
    /// violations against itself.
    #[test]
    fn zero_length_permission_carrier_owns_no_instructions() {
        let instructions = vec![
            Instruction::simple(OpCode::TypedArrayGetF64),
            Instruction::simple(OpCode::TypedArrayGetF64),
            Instruction::simple(OpCode::ReturnValue),
        ];
        let prog = make_program(
            vec![permission_carrier(0), v2_function("real_fn", 0, 3, true)],
            instructions,
        );
        assert!(
            verify_v2_typed_opcodes(&prog).is_ok(),
            "a body_length:0 carrier must not inherit the following function's range"
        );
    }

    /// POSITIVE CONTROL for the fix. A real function that genuinely carries a
    /// V2 typed opcode inside its own window with no `FrameDescriptor` is
    /// still reported — including when a carrier sits at its entry point, so
    /// the carrier fix cannot be silencing the whole pass. This is the shape
    /// `__main__` has in three corpus programs (`ACC__comptime__pb`, `pb3`,
    /// `probe2`), which are real violations, not carrier false positives.
    #[test]
    fn real_function_violation_is_still_reported_next_to_a_carrier() {
        let instructions = vec![
            Instruction::simple(OpCode::TypedArrayGetF64),
            Instruction::simple(OpCode::ReturnValue),
        ];
        let prog = make_program(
            vec![permission_carrier(0), v2_function("__main__", 0, 2, false)],
            instructions,
        );
        let errs = verify_v2_typed_opcodes(&prog).unwrap_err();
        assert_eq!(errs.len(), 1, "exactly the real violation, once");
        match &errs[0] {
            VerifyError::V2MissingFrameDescriptor {
                function_name,
                instruction_offset,
                ..
            } => {
                assert_eq!(function_name, "__main__");
                assert_eq!(*instruction_offset, 0);
            }
            other => panic!("expected the real function's violation, got {other:?}"),
        }
    }

    /// The old rule attributed a function's instructions to whichever row had
    /// the next-lowest entry point, so a violation in the SECOND function was
    /// reported against the FIRST. Ownership now follows `body_length`, so
    /// each violation names the function that actually contains it.
    #[test]
    fn violations_are_attributed_to_the_containing_function() {
        let instructions = vec![
            Instruction::simple(OpCode::ReturnValue),
            Instruction::simple(OpCode::TypedArrayGetF64),
            Instruction::simple(OpCode::ReturnValue),
        ];
        let prog = make_program(
            vec![
                v2_function("first", 0, 1, true),
                v2_function("second", 1, 2, false),
            ],
            instructions,
        );
        let errs = verify_v2_typed_opcodes(&prog).unwrap_err();
        assert_eq!(errs.len(), 1);
        assert!(matches!(
            &errs[0],
            VerifyError::V2MissingFrameDescriptor { function_name, .. } if function_name == "second"
        ));
    }

    /// Instructions past the last function's window belong to no function and
    /// are not verified against the last row — the old `unwrap_or(len)` end
    /// swept them up.
    #[test]
    fn instructions_beyond_the_last_window_are_not_attributed() {
        let instructions = vec![
            Instruction::simple(OpCode::ReturnValue),
            Instruction::simple(OpCode::TypedArrayGetF64),
        ];
        let prog = make_program(vec![v2_function("only", 0, 1, false)], instructions);
        assert!(verify_v2_typed_opcodes(&prog).is_ok());
    }

    /// A nested function body physically inside its parent's window (the
    /// layout `function_instructions.rs` documents) is verified once, against
    /// the function that directly owns it — not a second time against an
    /// enclosing row that may have no descriptor.
    #[test]
    fn nested_function_body_is_verified_only_against_its_own_row() {
        let instructions = vec![
            Instruction::simple(OpCode::ReturnValue), // 0: outer, jump-over
            Instruction::simple(OpCode::TypedArrayGetF64), // 1: inner body
            Instruction::simple(OpCode::ReturnValue), // 2: inner body
            Instruction::simple(OpCode::ReturnValue), // 3: outer tail
        ];
        let prog = make_program(
            vec![
                v2_function("outer", 0, 4, false),
                v2_function("inner", 1, 2, true),
            ],
            instructions,
        );
        assert!(
            verify_v2_typed_opcodes(&prog).is_ok(),
            "the inner body is the inner function's, and it has a descriptor"
        );
    }

    /// A function table that cannot describe unambiguous ownership is a real
    /// malformed-bytecode defect and is surfaced once, not swallowed and not
    /// repeated per function.
    #[test]
    fn malformed_function_windows_are_reported_once() {
        let instructions = vec![
            Instruction::simple(OpCode::TypedArrayGetF64),
            Instruction::simple(OpCode::ReturnValue),
        ];
        let prog = make_program(
            vec![
                v2_function("a", 0, 2, false),
                v2_function("b", 1, 4, false), // leaves the instruction stream
            ],
            instructions,
        );
        let errs = verify_v2_typed_opcodes(&prog).unwrap_err();
        assert_eq!(errs.len(), 1);
        assert!(matches!(
            &errs[0],
            VerifyError::MalformedFunctionWindows { .. }
        ));
    }

    /// The trusted pass shares the extent rule, so a carrier must not make it
    /// re-report the following function's trusted opcodes.
    #[test]
    fn trusted_pass_ignores_zero_length_carriers() {
        use crate::bytecode::Operand;
        let instructions = vec![
            Instruction::new(OpCode::LoadLocalTrusted, Some(Operand::Offset(3))),
            Instruction::simple(OpCode::ReturnValue),
        ];
        let prog = make_program(
            vec![permission_carrier(0), v2_function("real_fn", 0, 2, true)],
            instructions,
        );
        let errs = verify_trusted_opcodes(&prog).unwrap_err();
        assert_eq!(
            errs.len(),
            1,
            "the malformed operand is reported exactly once"
        );
        assert!(matches!(
            &errs[0],
            VerifyError::TrustedOpcodeBadOperand { function_name, .. } if function_name == "real_fn"
        ));
    }
}
