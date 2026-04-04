//! JIT Compilation Backend
//!
//! Implements the `CompilationBackend` trait from `shape-vm` so the TierManager
//! can drive JIT compilation on a background worker thread.

use shape_vm::bytecode::BytecodeProgram;
use shape_vm::tier::{CompilationBackend, CompilationRequest, CompilationResult, Tier};
use shape_vm::type_tracking::FrameDescriptor;

use crate::compiler::JITCompiler;
use crate::context::JITConfig;
use crate::loop_analysis;
use crate::osr_compiler;

/// JIT compilation backend that compiles hot loops to native code via Cranelift.
///
/// Owns a `JITCompiler` instance and implements the `CompilationBackend` trait.
/// The `TierManager::set_backend()` spawns a worker thread that drives this.
pub struct JitCompilationBackend {
    jit: JITCompiler,
}

impl JitCompilationBackend {
    /// Create a new JIT compilation backend with default configuration.
    pub fn new() -> Result<Self, crate::error::JitError> {
        Ok(Self {
            jit: JITCompiler::new(JITConfig::default())?,
        })
    }

    /// Create a new JIT compilation backend with custom configuration.
    pub fn with_config(config: JITConfig) -> Result<Self, crate::error::JitError> {
        Ok(Self {
            jit: JITCompiler::new(config)?,
        })
    }

    /// Compile an OSR loop from a compilation request.
    fn compile_osr(
        &mut self,
        request: &CompilationRequest,
        program: &BytecodeProgram,
    ) -> CompilationResult {
        let func_id = request.function_id;
        let loop_header_ip = request.loop_header_ip;

        // Get the target function
        let function = match program.functions.get(func_id as usize) {
            Some(f) => f,
            None => {
                return CompilationResult {
                    function_id: func_id,
                    compiled_tier: Tier::Interpreted,
                    native_code: None,
                    error: Some(format!("Function {} not found in program", func_id)),
                    osr_entry: None,
                    deopt_points: Vec::new(),
                    loop_header_ip,
                    shape_guards: Vec::new(),
                };
            }
        };

        // Extract the function's instruction range
        let entry = function.entry_point;
        let end = find_function_end(program, func_id as usize);
        if entry >= program.instructions.len() || end > program.instructions.len() {
            return CompilationResult {
                function_id: func_id,
                compiled_tier: Tier::Interpreted,
                native_code: None,
                error: Some(format!(
                    "Function {} instruction range [{}, {}) out of bounds",
                    func_id, entry, end
                )),
                osr_entry: None,
                deopt_points: Vec::new(),
                loop_header_ip,
                shape_guards: Vec::new(),
            };
        }
        let func_instructions = &program.instructions[entry..end];

        // Run loop analysis on a sub-program containing just this function's instructions
        let sub_program = build_sub_program(program, entry, end);
        let loop_infos = loop_analysis::analyze_loops(&sub_program);

        // Find the target loop. The loop_header_ip from the request is in
        // global instruction coordinates; convert to function-local offset.
        let target_local_ip = match loop_header_ip {
            Some(ip) => {
                if ip < entry {
                    return CompilationResult {
                        function_id: func_id,
                        compiled_tier: Tier::Interpreted,
                        native_code: None,
                        error: Some(format!(
                            "OSR loop header IP {} is before function entry {}",
                            ip, entry
                        )),
                        osr_entry: None,
                        deopt_points: Vec::new(),
                        loop_header_ip: Some(ip),
                        shape_guards: Vec::new(),
                    };
                }
                ip - entry
            }
            None => {
                return CompilationResult {
                    function_id: func_id,
                    compiled_tier: Tier::Interpreted,
                    native_code: None,
                    error: Some("OSR request without loop_header_ip".to_string()),
                    osr_entry: None,
                    deopt_points: Vec::new(),
                    loop_header_ip: None,
                    shape_guards: Vec::new(),
                };
            }
        };

        let loop_info = match loop_infos.get(&target_local_ip) {
            Some(li) => li,
            None => {
                return CompilationResult {
                    function_id: func_id,
                    compiled_tier: Tier::Interpreted,
                    native_code: None,
                    error: Some(format!(
                        "No loop found at local IP {} (global IP {:?})",
                        target_local_ip, loop_header_ip
                    )),
                    osr_entry: None,
                    deopt_points: Vec::new(),
                    loop_header_ip,
                    shape_guards: Vec::new(),
                };
            }
        };

        // Build frame descriptor (use function's if available, else default)
        let default_frame = FrameDescriptor::default();
        let frame_descriptor = function.frame_descriptor.as_ref().unwrap_or(&default_frame);

        // Compile the loop
        match osr_compiler::compile_osr_loop(
            &mut self.jit,
            function,
            func_instructions,
            loop_info,
            frame_descriptor,
        ) {
            Ok(osr_result) => {
                // Adjust entry point bytecode_ip back to global coordinates
                let mut entry_point = osr_result.entry_point;
                entry_point.bytecode_ip += entry;
                entry_point.exit_ip += entry;

                CompilationResult {
                    function_id: func_id,
                    compiled_tier: Tier::BaselineJit,
                    native_code: Some(osr_result.native_code),
                    error: None,
                    osr_entry: Some(entry_point),
                    deopt_points: osr_result.deopt_points,
                    loop_header_ip,
                    shape_guards: Vec::new(),
                }
            }
            Err(e) => CompilationResult {
                function_id: func_id,
                compiled_tier: Tier::Interpreted,
                native_code: None,
                error: Some(e),
                osr_entry: None,
                deopt_points: Vec::new(),
                loop_header_ip,
                shape_guards: Vec::new(),
            },
        }
    }
}

// SAFETY: JitCompilationBackend is used exclusively on its own worker thread.
// The raw pointers in JITCompiler (compiled_functions, function_table) point
// to JIT code that is immutable after compilation and valid for the module's
// lifetime. Access is single-threaded (the worker thread).
unsafe impl Send for JitCompilationBackend {}

impl JitCompilationBackend {
    /// Compile a whole function for Tier 1/2 promotion.
    ///
    /// Tier 1 (BaselineJit, no feedback): uses `compile_single_function` with
    /// empty user_funcs — cross-function calls deopt to interpreter.
    ///
    /// Tier 2 (OptimizingJit, with feedback): uses `compile_optimizing_function`
    /// which enables speculative calls based on monomorphic call feedback.
    /// Self-recursive calls get direct-call FuncRefs. Cross-function monomorphic
    /// calls get callee identity guard + FFI fallthrough (guard deopt on mismatch).
    fn compile_function(
        &mut self,
        request: &CompilationRequest,
        program: &BytecodeProgram,
    ) -> CompilationResult {
        let func_id = request.function_id;

        // Tier 2: feedback-guided optimizing compilation with populated user_funcs
        if let Some(fv) = request.feedback.clone() {
            return match self.jit.compile_optimizing_function(
                program,
                func_id as usize,
                fv,
                &request.callee_feedback,
            ) {
                Ok((code_ptr, deopt_points, shape_guards)) => CompilationResult {
                    function_id: func_id,
                    compiled_tier: request.target_tier,
                    native_code: Some(code_ptr),
                    error: None,
                    osr_entry: None,
                    deopt_points,
                    loop_header_ip: None,
                    shape_guards,
                },
                Err(e) => CompilationResult {
                    function_id: func_id,
                    compiled_tier: Tier::Interpreted,
                    native_code: None,
                    error: Some(e),
                    osr_entry: None,
                    deopt_points: Vec::new(),
                    loop_header_ip: None,
                    shape_guards: Vec::new(),
                },
            };
        }

        // Tier 1: baseline compilation without cross-function speculation
        match self
            .jit
            .compile_single_function(program, func_id as usize, None)
        {
            Ok((code_ptr, deopt_points, shape_guards)) => CompilationResult {
                function_id: func_id,
                compiled_tier: request.target_tier,
                native_code: Some(code_ptr),
                error: None,
                osr_entry: None,
                deopt_points,
                loop_header_ip: None,
                shape_guards,
            },
            Err(e) => CompilationResult {
                function_id: func_id,
                compiled_tier: Tier::Interpreted,
                native_code: None,
                error: Some(e),
                osr_entry: None,
                deopt_points: Vec::new(),
                loop_header_ip: None,
                shape_guards: Vec::new(),
            },
        }
    }
}

impl CompilationBackend for JitCompilationBackend {
    fn compile(
        &mut self,
        request: &CompilationRequest,
        program: &BytecodeProgram,
    ) -> CompilationResult {
        if request.osr {
            self.compile_osr(request, program)
        } else {
            self.compile_function(request, program)
        }
    }
}

/// Find the end of a function's instruction range.
///
/// For the last function, this is the end of the instruction stream.
/// For other functions, this is the entry point of the next function.
fn find_function_end(program: &BytecodeProgram, func_index: usize) -> usize {
    let func = &program.functions[func_index];
    func.entry_point + func.body_length
}

/// Build a minimal sub-program containing only the instructions in [start, end).
///
/// The sub-program's instructions are indexed from 0, making it compatible
/// with `analyze_loops()` which expects a contiguous instruction stream.
fn build_sub_program(program: &BytecodeProgram, start: usize, end: usize) -> BytecodeProgram {
    BytecodeProgram {
        instructions: program.instructions[start..end].to_vec(),
        constants: program.constants.clone(),
        strings: program.strings.clone(),
        functions: vec![],
        debug_info: Default::default(),
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
            top_level_mir: None,
        function_blob_hashes: vec![],
        top_level_frame: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_vm::bytecode::*;
    use shape_vm::type_tracking::{FrameDescriptor, SlotKind};

    fn make_instr(opcode: OpCode, operand: Option<Operand>) -> Instruction {
        Instruction { opcode, operand }
    }

    #[test]
    fn test_backend_compiles_whole_function() {
        let mut backend = JitCompilationBackend::new().unwrap();

        // Simple function: return local 0 + local 1
        let instrs = vec![
            // Function body at entry_point=0
            make_instr(OpCode::LoadLocal, Some(Operand::Local(0))), // 0
            make_instr(OpCode::LoadLocal, Some(Operand::Local(1))), // 1
            make_instr(OpCode::AddInt, None),                       // 2
            make_instr(OpCode::ReturnValue, None),                  // 3
            // Main code (trampoline target)
            make_instr(OpCode::Halt, None), // 4
        ];

        let func = Function {
            name: "add_two".to_string(),
            arity: 2,
            param_names: vec![],
            locals_count: 2,
            entry_point: 0,
            body_length: 4,
            is_closure: false,
            captures_count: 0,
            is_async: false,
            ref_params: vec![],
                    mir_data: None,
            ref_mutates: vec![],
            mutable_captures: vec![],
            frame_descriptor: Some(FrameDescriptor::from_slots(vec![
                SlotKind::Int64, // arg0
                SlotKind::Int64, // arg1
            ])),
            osr_entry_points: vec![],
        };

        let program = BytecodeProgram {
            instructions: instrs,
            constants: vec![],
            strings: vec![],
            functions: vec![func],
            debug_info: Default::default(),
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
            top_level_mir: None,
            function_blob_hashes: vec![],
            top_level_frame: None,
            ..Default::default()
        };

        let request = CompilationRequest {
            function_id: 0,
            target_tier: Tier::BaselineJit,
            blob_hash: None,
            osr: false,
            loop_header_ip: None,
            feedback: None,
            callee_feedback: std::collections::HashMap::new(),
        };

        let result = backend.compile(&request, &program);
        assert!(
            result.error.is_none(),
            "Expected successful whole-function compilation, got: {:?}",
            result.error
        );
        assert!(result.native_code.is_some());
        assert_eq!(result.compiled_tier, Tier::BaselineJit);
        assert!(result.osr_entry.is_none()); // Not an OSR result
    }

    #[test]
    fn test_backend_whole_function_invalid_id() {
        let mut backend = JitCompilationBackend::new().unwrap();
        let program = BytecodeProgram {
            instructions: vec![make_instr(OpCode::Halt, None)],
            constants: vec![],
            strings: vec![],
            functions: vec![], // No functions
            debug_info: Default::default(),
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
            top_level_mir: None,
            function_blob_hashes: vec![],
            top_level_frame: None,
            ..Default::default()
        };
        let request = CompilationRequest {
            function_id: 99,
            target_tier: Tier::BaselineJit,
            blob_hash: None,
            osr: false,
            loop_header_ip: None,
            feedback: None,
            callee_feedback: std::collections::HashMap::new(),
        };
        let result = backend.compile(&request, &program);
        assert!(result.error.is_some());
        assert!(result.error.unwrap().contains("not found"));
    }

    #[test]
    fn test_backend_osr_compiles_simple_loop() {
        let mut backend = JitCompilationBackend::new().unwrap();

        // Function at entry_point=0: for (i=0; i<n; i++) { sum += i }
        let instrs = vec![
            make_instr(OpCode::LoopStart, None),                       // 0
            make_instr(OpCode::LoadLocal, Some(Operand::Local(0))),    // 1: i
            make_instr(OpCode::LoadLocal, Some(Operand::Local(1))),    // 2: n
            make_instr(OpCode::LtInt, None),                           // 3
            make_instr(OpCode::JumpIfFalse, Some(Operand::Offset(7))), // 4
            make_instr(OpCode::LoadLocal, Some(Operand::Local(2))),    // 5: sum
            make_instr(OpCode::LoadLocal, Some(Operand::Local(0))),    // 6: i
            make_instr(OpCode::AddInt, None),                          // 7
            make_instr(OpCode::StoreLocal, Some(Operand::Local(2))),   // 8
            make_instr(OpCode::LoadLocal, Some(Operand::Local(0))),    // 9: i
            make_instr(OpCode::PushConst, Some(Operand::Const(0))),    // 10: 1
            make_instr(OpCode::AddInt, None),                          // 11
            make_instr(OpCode::StoreLocal, Some(Operand::Local(0))),   // 12
            make_instr(OpCode::LoopEnd, None),                         // 13
            make_instr(OpCode::ReturnValue, None),                     // 14
        ];

        let func = Function {
            name: "test_loop".to_string(),
            arity: 0,
            param_names: vec![],
            locals_count: 3,
            entry_point: 0,
            body_length: 15,
            is_closure: false,
            captures_count: 0,
            is_async: false,
            ref_params: vec![],
                    mir_data: None,
            ref_mutates: vec![],
            mutable_captures: vec![],
            frame_descriptor: Some(FrameDescriptor::from_slots(vec![
                SlotKind::Int64, // i
                SlotKind::Int64, // n
                SlotKind::Int64, // sum
            ])),
            osr_entry_points: vec![],
        };

        let program = BytecodeProgram {
            instructions: instrs,
            constants: vec![Constant::Int(1)],
            strings: vec![],
            functions: vec![func],
            debug_info: Default::default(),
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
            top_level_mir: None,
            function_blob_hashes: vec![],
            top_level_frame: None,
            ..Default::default()
        };

        let request = CompilationRequest {
            function_id: 0,
            target_tier: Tier::BaselineJit,
            blob_hash: None,
            osr: true,
            loop_header_ip: Some(0), // Global IP of LoopStart
            feedback: None,
            callee_feedback: std::collections::HashMap::new(),
        };

        let result = backend.compile(&request, &program);
        assert!(
            result.error.is_none(),
            "Expected successful compilation, got: {:?}",
            result.error
        );
        assert!(result.native_code.is_some());
        assert!(result.osr_entry.is_some());
        assert_eq!(result.compiled_tier, Tier::BaselineJit);

        let entry = result.osr_entry.unwrap();
        assert_eq!(entry.bytecode_ip, 0);
        assert!(entry.live_locals.contains(&0)); // i
        assert!(entry.live_locals.contains(&1)); // n
        assert!(entry.live_locals.contains(&2)); // sum
    }

    #[test]
    fn test_backend_osr_blacklists_unsupported_loop() {
        let mut backend = JitCompilationBackend::new().unwrap();

        // Function with a loop containing CallMethod (unsupported)
        let instrs = vec![
            make_instr(OpCode::LoopStart, None),
            make_instr(OpCode::LoadLocal, Some(Operand::Local(0))),
            make_instr(OpCode::CallMethod, None), // Unsupported!
            make_instr(OpCode::Pop, None),
            make_instr(OpCode::LoopEnd, None),
            make_instr(OpCode::Halt, None),
        ];

        let func = Function {
            name: "unsupported_loop".to_string(),
            arity: 0,
            param_names: vec![],
            locals_count: 1,
            entry_point: 0,
            body_length: 6,
            is_closure: false,
            captures_count: 0,
            is_async: false,
            ref_params: vec![],
                    mir_data: None,
            ref_mutates: vec![],
            mutable_captures: vec![],
            frame_descriptor: Some(FrameDescriptor::from_slots(vec![SlotKind::Unknown])),
            osr_entry_points: vec![],
        };

        let program = BytecodeProgram {
            instructions: instrs,
            constants: vec![],
            strings: vec![],
            functions: vec![func],
            debug_info: Default::default(),
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
            top_level_mir: None,
            function_blob_hashes: vec![],
            top_level_frame: None,
            ..Default::default()
        };

        let request = CompilationRequest {
            function_id: 0,
            target_tier: Tier::BaselineJit,
            blob_hash: None,
            osr: true,
            loop_header_ip: Some(0),
            feedback: None,
            callee_feedback: std::collections::HashMap::new(),
        };

        let result = backend.compile(&request, &program);
        assert!(result.error.is_some());
        assert!(result.error.unwrap().contains("unsupported opcode"));
        assert_eq!(result.loop_header_ip, Some(0)); // For blacklisting
    }
}
