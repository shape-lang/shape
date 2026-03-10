use super::*;
use shape_vm::bytecode::*;

fn make_instr(opcode: OpCode, operand: Option<Operand>) -> Instruction {
    Instruction { opcode, operand }
}

fn make_func(name: &str, arity: u16, locals_count: u16, entry_point: usize) -> Function {
    make_func_with_body(name, arity, locals_count, entry_point, 0)
}

fn make_func_with_body(
    name: &str,
    arity: u16,
    locals_count: u16,
    entry_point: usize,
    body_length: usize,
) -> Function {
    Function {
        name: name.to_string(),
        arity,
        param_names: vec![],
        locals_count,
        entry_point,
        body_length,
        is_closure: false,
        captures_count: 0,
        is_async: false,
        ref_params: vec![],
        ref_mutates: vec![],
        mutable_captures: vec![],
        frame_descriptor: None,
        osr_entry_points: vec![],
    }
}

fn make_program(instrs: Vec<Instruction>, functions: Vec<Function>) -> BytecodeProgram {
    BytecodeProgram {
        instructions: instrs,
        constants: vec![Constant::Number(1.0)],
        strings: vec![],
        functions,
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
fn test_inline_candidate_simple_leaf() {
    // Main program: PushConst, Halt
    // Function "double" at index 2: LoadLocal(0), LoadLocal(0), AddInt, ReturnValue
    let instrs = vec![
        make_instr(OpCode::PushConst, Some(Operand::Const(0))),
        make_instr(OpCode::Halt, None),
        // Function "double" starts here (entry_point = 2)
        make_instr(OpCode::LoadLocal, Some(Operand::Local(0))),
        make_instr(OpCode::LoadLocal, Some(Operand::Local(0))),
        make_instr(OpCode::AddInt, None),
        make_instr(OpCode::ReturnValue, None),
    ];

    let functions = vec![make_func_with_body("double", 1, 1, 2, 4)];
    let program = make_program(instrs, functions);

    let candidates = BytecodeToIR::analyze_inline_candidates(&program);
    assert_eq!(candidates.len(), 1);

    let c = candidates.get(&0).unwrap();
    assert_eq!(c.entry_point, 2);
    assert_eq!(c.instruction_count, 4);
    assert_eq!(c.arity, 1);
    assert_eq!(c.locals_count, 1);
}

#[test]
fn test_inline_candidate_with_calls_allowed() {
    // Functions with Call instructions ARE now inlineable (non-leaf allowed)
    let instrs = vec![
        make_instr(OpCode::Halt, None),
        // Function with a Call instruction
        make_instr(OpCode::LoadLocal, Some(Operand::Local(0))),
        make_instr(OpCode::PushConst, Some(Operand::Const(0))),
        make_instr(
            OpCode::Call,
            Some(Operand::Function(shape_value::FunctionId(1))),
        ),
        make_instr(OpCode::ReturnValue, None),
    ];

    let functions = vec![make_func_with_body("caller", 1, 1, 1, 4)];
    let program = make_program(instrs, functions);

    let candidates = BytecodeToIR::analyze_inline_candidates(&program);
    assert_eq!(candidates.len(), 1); // Call -> inlineable (non-leaf allowed)
}

#[test]
fn test_inline_candidate_excluded_with_call_value() {
    // Functions with CallValue (closure calls) are NOT inlineable
    let instrs = vec![
        make_instr(OpCode::Halt, None),
        // Function with a CallValue instruction (closure call)
        make_instr(OpCode::LoadLocal, Some(Operand::Local(0))),
        make_instr(OpCode::PushConst, Some(Operand::Const(0))),
        make_instr(OpCode::CallValue, None),
        make_instr(OpCode::ReturnValue, None),
    ];

    let functions = vec![make_func_with_body("closure_caller", 1, 1, 1, 4)];
    let program = make_program(instrs, functions);

    let candidates = BytecodeToIR::analyze_inline_candidates(&program);
    assert_eq!(candidates.len(), 0); // CallValue -> not inlineable
}

#[test]
fn test_inline_candidate_excluded_with_branches() {
    // Function with JumpIfFalse - NOT inlineable (has control flow)
    let instrs = vec![
        make_instr(OpCode::Halt, None),
        // Function with a branch
        make_instr(OpCode::LoadLocal, Some(Operand::Local(0))),
        make_instr(OpCode::JumpIfFalse, Some(Operand::Offset(2))),
        make_instr(OpCode::LoadLocal, Some(Operand::Local(0))),
        make_instr(OpCode::ReturnValue, None),
        make_instr(OpCode::PushNull, None),
        make_instr(OpCode::ReturnValue, None),
    ];

    let functions = vec![make_func_with_body("branchy", 1, 1, 1, 6)];
    let program = make_program(instrs, functions);

    let candidates = BytecodeToIR::analyze_inline_candidates(&program);
    assert_eq!(candidates.len(), 0); // Branch -> not inlineable
}

#[test]
fn test_inline_candidate_excluded_closure() {
    // Closure - NOT inlineable
    let instrs = vec![
        make_instr(OpCode::Halt, None),
        make_instr(OpCode::LoadLocal, Some(Operand::Local(0))),
        make_instr(OpCode::ReturnValue, None),
    ];

    let mut func = make_func_with_body("closure_fn", 1, 1, 1, 2);
    func.is_closure = true;
    let functions = vec![func];
    let program = make_program(instrs, functions);

    let candidates = BytecodeToIR::analyze_inline_candidates(&program);
    assert_eq!(candidates.len(), 0); // Closure -> not inlineable
}

// ============================================================================
// OSR Entry Point Tests
// ============================================================================

#[test]
fn test_osr_entry_point_serialization_roundtrip() {
    use shape_vm::type_tracking::SlotKind;

    let entry = OsrEntryPoint {
        bytecode_ip: 42,
        live_locals: vec![0, 1, 3],
        local_kinds: vec![SlotKind::Int64, SlotKind::Float64, SlotKind::Unknown],
        exit_ip: 100,
    };

    let json = serde_json::to_string(&entry).expect("serialize OsrEntryPoint");
    let roundtripped: OsrEntryPoint =
        serde_json::from_str(&json).expect("deserialize OsrEntryPoint");

    assert_eq!(roundtripped.bytecode_ip, 42);
    assert_eq!(roundtripped.live_locals, vec![0, 1, 3]);
    assert_eq!(
        roundtripped.local_kinds,
        vec![SlotKind::Int64, SlotKind::Float64, SlotKind::Unknown]
    );
    assert_eq!(roundtripped.exit_ip, 100);
}

#[test]
fn test_deopt_info_construction_and_local_mapping() {
    use shape_vm::type_tracking::SlotKind;

    let deopt = DeoptInfo {
        resume_ip: 55,
        local_mapping: vec![(0, 2), (1, 5), (2, 8)],
        local_kinds: vec![SlotKind::Int64, SlotKind::Float64, SlotKind::Bool],
        stack_depth: 1,
        innermost_function_id: None,
        inline_frames: Vec::new(),
    };

    // Verify mapping: JIT local 0 -> bytecode local 2, etc.
    assert_eq!(deopt.local_mapping[0], (0, 2));
    assert_eq!(deopt.local_mapping[1], (1, 5));
    assert_eq!(deopt.local_mapping[2], (2, 8));
    assert_eq!(deopt.resume_ip, 55);
    assert_eq!(deopt.stack_depth, 1);
}

#[test]
fn test_deopt_info_serialization_roundtrip() {
    use shape_vm::type_tracking::SlotKind;

    let deopt = DeoptInfo {
        resume_ip: 77,
        local_mapping: vec![(0, 0), (1, 1)],
        local_kinds: vec![SlotKind::Float64, SlotKind::Int64],
        stack_depth: 0,
        innermost_function_id: None,
        inline_frames: Vec::new(),
    };

    let json = serde_json::to_string(&deopt).expect("serialize DeoptInfo");
    let roundtripped: DeoptInfo = serde_json::from_str(&json).expect("deserialize DeoptInfo");

    assert_eq!(roundtripped.resume_ip, 77);
    assert_eq!(roundtripped.local_mapping.len(), 2);
    assert_eq!(roundtripped.local_kinds[0], SlotKind::Float64);
    assert_eq!(roundtripped.local_kinds[1], SlotKind::Int64);
    assert_eq!(roundtripped.stack_depth, 0);
}

#[test]
fn test_compile_osr_loop_basic() {
    use super::loop_analysis;
    use crate::compiler::JITCompiler;
    use crate::context::JITConfig;
    use crate::translator::osr_compiler::compile_osr_loop;
    use shape_vm::type_tracking::{FrameDescriptor, SlotKind};

    // Build a simple loop: for (i = 0; i < n; i++) { sum = sum + i }
    let instrs = vec![
        make_instr(OpCode::LoopStart, None),
        // condition: i < n
        make_instr(OpCode::LoadLocal, Some(Operand::Local(0))), // load i
        make_instr(OpCode::LoadLocal, Some(Operand::Local(1))), // load n
        make_instr(OpCode::LtInt, None),
        make_instr(OpCode::JumpIfFalse, Some(Operand::Offset(7))),
        // body: sum = sum + i
        make_instr(OpCode::LoadLocal, Some(Operand::Local(2))), // load sum
        make_instr(OpCode::LoadLocal, Some(Operand::Local(0))), // load i
        make_instr(OpCode::AddInt, None),
        make_instr(OpCode::StoreLocal, Some(Operand::Local(2))), // store sum
        // increment: i = i + 1
        make_instr(OpCode::LoadLocal, Some(Operand::Local(0))),
        make_instr(OpCode::PushConst, Some(Operand::Const(0))),
        make_instr(OpCode::AddInt, None),
        make_instr(OpCode::StoreLocal, Some(Operand::Local(0))),
        make_instr(OpCode::LoopEnd, None),
    ];

    let program = BytecodeProgram {
        instructions: instrs.clone(),
        constants: vec![Constant::Int(1)],
        strings: vec![],
        functions: vec![],
        debug_info: DebugInfo::default(),
        data_schema: None,
        module_binding_names: vec![],
        top_level_locals_count: 3,
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
    };

    let loop_infos = loop_analysis::analyze_loops(&program);
    let loop_info = loop_infos.get(&0).expect("should find loop at index 0");

    let func = make_func("test_fn", 0, 3, 0);
    let frame = FrameDescriptor::from_slots(vec![
        SlotKind::Int64, // i
        SlotKind::Int64, // n
        SlotKind::Int64, // sum
    ]);

    let mut jit = JITCompiler::new(JITConfig::default()).expect("JITCompiler::new should succeed");

    let result = compile_osr_loop(&mut jit, &func, &instrs, loop_info, &frame)
        .expect("compile_osr_loop should succeed");

    // Entry point metadata should be populated
    assert_eq!(result.entry_point.bytecode_ip, 0); // LoopStart at index 0
    assert_eq!(result.entry_point.exit_ip, 14); // LoopEnd at index 13, exit = 14

    // All three locals should be live (i, n, sum are all read/written)
    assert!(result.entry_point.live_locals.contains(&0)); // i
    assert!(result.entry_point.live_locals.contains(&1)); // n
    assert!(result.entry_point.live_locals.contains(&2)); // sum

    // Kinds should match frame descriptor
    for (idx, &local) in result.entry_point.live_locals.iter().enumerate() {
        let expected = frame.slots[local as usize];
        assert_eq!(result.entry_point.local_kinds[idx], expected);
    }

    // Native code should be non-null (real compilation now)
    assert!(!result.native_code.is_null());

    // No deopt points in MVP
    assert!(result.deopt_points.is_empty());
}

#[test]
fn test_compile_osr_loop_out_of_bounds() {
    use super::loop_analysis::LoopInfo;
    use crate::compiler::JITCompiler;
    use crate::context::JITConfig;
    use crate::translator::osr_compiler::compile_osr_loop;
    use shape_vm::type_tracking::{FrameDescriptor, SlotKind};

    let func = make_func("test_fn", 0, 2, 0);
    let instrs = vec![
        make_instr(OpCode::PushNull, None),
        make_instr(OpCode::Halt, None),
    ];
    let frame = FrameDescriptor::from_slots(vec![SlotKind::Int64, SlotKind::Int64]);

    // Loop info pointing out of bounds
    let loop_info = LoopInfo {
        header_idx: 100, // way out of bounds
        end_idx: 200,
        body_locals_written: Default::default(),
        body_locals_read: Default::default(),
        body_module_bindings_written: Default::default(),
        body_module_bindings_read: Default::default(),
        induction_vars: vec![],
        invariant_locals: Default::default(),
        invariant_module_bindings: Default::default(),
        body_can_allocate: false,
    };

    let mut jit = JITCompiler::new(JITConfig::default()).expect("JITCompiler::new should succeed");

    let result = compile_osr_loop(&mut jit, &func, &instrs, &loop_info, &frame);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("out of bounds"));
}

#[test]
fn test_function_osr_entry_points_field() {
    // Verify the Function struct has the new osr_entry_points field
    // and it defaults to empty via serde
    let func = make_func("test", 0, 0, 0);
    assert!(func.osr_entry_points.is_empty());

    // Can store entry points
    let mut func = func;
    func.osr_entry_points.push(OsrEntryPoint {
        bytecode_ip: 10,
        live_locals: vec![0, 1],
        local_kinds: vec![
            shape_vm::type_tracking::SlotKind::Int64,
            shape_vm::type_tracking::SlotKind::Float64,
        ],
        exit_ip: 25,
    });
    assert_eq!(func.osr_entry_points.len(), 1);
    assert_eq!(func.osr_entry_points[0].bytecode_ip, 10);
}

// ============================================================================
// DeoptTracker Integration with New Metadata
// ============================================================================

#[test]
fn test_deopt_tracker_with_osr_metadata() {
    use crate::optimizer::{DeoptTracker, OptimizationDependencies};

    let mut tracker = DeoptTracker::new();

    // Register a function with dependencies
    let func_hash = [1u8; 32];
    let inlined_hash = [2u8; 32];
    let mut deps = OptimizationDependencies::default();
    deps.inlined_functions.insert(inlined_hash);
    deps.assumed_constant_bindings.insert(3);

    tracker.register(func_hash, deps);
    assert_eq!(tracker.tracked_count(), 1);

    // Invalidate via function change -> the function should be invalidated
    let invalidated = tracker.invalidate_function(&inlined_hash);
    assert_eq!(invalidated.len(), 1);
    assert_eq!(invalidated[0], func_hash);
    assert_eq!(tracker.tracked_count(), 0);
}

#[test]
fn test_deopt_tracker_binding_invalidation_with_multiple_functions() {
    use crate::optimizer::{DeoptTracker, OptimizationDependencies};

    let mut tracker = DeoptTracker::new();

    // Two functions depend on binding 5
    let func1 = [1u8; 32];
    let func2 = [2u8; 32];

    let mut deps1 = OptimizationDependencies::default();
    deps1.assumed_constant_bindings.insert(5);
    tracker.register(func1, deps1);

    let mut deps2 = OptimizationDependencies::default();
    deps2.assumed_constant_bindings.insert(5);
    tracker.register(func2, deps2);

    assert_eq!(tracker.tracked_count(), 2);

    // Invalidate binding 5 -> both functions should be invalidated
    let mut invalidated = tracker.invalidate_binding(5);
    invalidated.sort();
    assert_eq!(invalidated.len(), 2);
    assert_eq!(tracker.tracked_count(), 0);
}

// ============================================================================
// Speculative IR Tests (Feedback-Guided Tier 2)
// ============================================================================

#[test]
fn test_speculative_call_target_monomorphic() {
    use shape_vm::feedback::FeedbackVector;

    let mut fv = FeedbackVector::new(0);
    fv.record_call(10, 5); // monomorphic: target fn 5
    fv.record_call(10, 5); // same target

    // Verify the feedback slot is monomorphic
    assert!(fv.is_monomorphic(10));

    match fv.get_slot(10).unwrap() {
        shape_vm::feedback::FeedbackSlot::Call(fb) => {
            assert_eq!(fb.state, shape_vm::feedback::ICState::Monomorphic);
            assert_eq!(fb.targets[0].function_id, 5);
            assert_eq!(fb.targets[0].count, 2);
        }
        _ => panic!("expected Call slot"),
    }
}

#[test]
fn test_speculative_call_target_polymorphic_not_eligible() {
    use shape_vm::feedback::FeedbackVector;

    let mut fv = FeedbackVector::new(0);
    fv.record_call(10, 5);
    fv.record_call(10, 6); // different target -> polymorphic

    assert!(!fv.is_monomorphic(10));
}

#[test]
fn test_speculative_property_monomorphic() {
    use shape_vm::feedback::FeedbackVector;

    let mut fv = FeedbackVector::new(0);
    fv.record_property(20, 100, 3, 1, 0); // schema 100, field_idx 3, type_tag 1

    assert!(fv.is_monomorphic(20));

    match fv.get_slot(20).unwrap() {
        shape_vm::feedback::FeedbackSlot::Property(fb) => {
            assert_eq!(fb.state, shape_vm::feedback::ICState::Monomorphic);
            assert_eq!(fb.entries[0].schema_id, 100);
            assert_eq!(fb.entries[0].field_idx, 3);
            assert_eq!(fb.entries[0].field_type_tag, 1);
        }
        _ => panic!("expected Property slot"),
    }
}

#[test]
fn test_speculative_property_polymorphic_not_eligible() {
    use shape_vm::feedback::FeedbackVector;

    let mut fv = FeedbackVector::new(0);
    fv.record_property(20, 100, 3, 1, 0);
    fv.record_property(20, 200, 5, 2, 0); // different schema -> polymorphic

    assert!(!fv.is_monomorphic(20));
}

#[test]
fn test_speculative_arithmetic_monomorphic_int() {
    use shape_vm::feedback::FeedbackVector;

    let mut fv = FeedbackVector::new(0);
    // Tag 1 = I48 (integer)
    fv.record_arithmetic(30, 1, 1);
    fv.record_arithmetic(30, 1, 1); // same types

    assert!(fv.is_monomorphic(30));

    match fv.get_slot(30).unwrap() {
        shape_vm::feedback::FeedbackSlot::Arithmetic(fb) => {
            assert_eq!(fb.state, shape_vm::feedback::ICState::Monomorphic);
            assert_eq!(fb.type_pairs[0].left_tag, 1);
            assert_eq!(fb.type_pairs[0].right_tag, 1);
        }
        _ => panic!("expected Arithmetic slot"),
    }
}

#[test]
fn test_speculative_arithmetic_monomorphic_float() {
    use shape_vm::feedback::FeedbackVector;

    let mut fv = FeedbackVector::new(0);
    // Tag 0 = f64 (number)
    fv.record_arithmetic(30, 0, 0);

    assert!(fv.is_monomorphic(30));

    match fv.get_slot(30).unwrap() {
        shape_vm::feedback::FeedbackSlot::Arithmetic(fb) => {
            assert_eq!(fb.type_pairs[0].left_tag, 0);
            assert_eq!(fb.type_pairs[0].right_tag, 0);
        }
        _ => panic!("expected Arithmetic slot"),
    }
}

#[test]
fn test_speculative_arithmetic_mixed_types_not_eligible() {
    use shape_vm::feedback::FeedbackVector;

    let mut fv = FeedbackVector::new(0);
    fv.record_arithmetic(30, 1, 0); // int + float: monomorphic but mixed
    // This is still monomorphic (one pair observed), but our speculative
    // paths only handle homogeneous int+int or float+float.
    assert!(fv.is_monomorphic(30));

    // Verify the type pair is mixed (should NOT match our fast paths)
    match fv.get_slot(30).unwrap() {
        shape_vm::feedback::FeedbackSlot::Arithmetic(fb) => {
            let pair = &fb.type_pairs[0];
            // left=1 (int), right=0 (float): our speculative paths reject this
            assert!(pair.left_tag != pair.right_tag);
        }
        _ => panic!("expected Arithmetic slot"),
    }
}

#[test]
fn test_speculative_arithmetic_megamorphic_not_eligible() {
    use shape_vm::feedback::FeedbackVector;

    let mut fv = FeedbackVector::new(0);
    for i in 0u8..5 {
        fv.record_arithmetic(30, i, i);
    }

    assert!(!fv.is_monomorphic(30));
}

#[test]
fn test_feedback_vector_monomorphic_ratio() {
    use shape_vm::feedback::FeedbackVector;

    let mut fv = FeedbackVector::new(0);

    // All monomorphic -> ratio = 1.0
    fv.record_call(0, 1);
    fv.record_property(10, 100, 0, 1, 0);
    fv.record_arithmetic(20, 1, 1);
    assert!((fv.monomorphic_ratio() - 1.0).abs() < 0.01);

    // Make one polymorphic -> ratio = 2/3
    fv.record_call(0, 2);
    assert!(!fv.is_monomorphic(0));
    let ratio = fv.monomorphic_ratio();
    assert!(ratio > 0.6 && ratio < 0.7, "ratio was {}", ratio);
}

#[test]
fn test_compilation_result_deopt_points() {
    use shape_vm::tier::CompilationResult;
    use shape_vm::type_tracking::SlotKind;

    // Verify deopt_points Vec can hold multiple entries
    let result = CompilationResult {
        function_id: 0,
        compiled_tier: shape_vm::tier::Tier::OptimizingJit,
        native_code: Some(0x1000 as *const u8),
        error: None,
        osr_entry: None,
        deopt_points: vec![
            DeoptInfo {
                resume_ip: 10,
                local_mapping: vec![(0, 0), (1, 1)],
                local_kinds: vec![SlotKind::Int64, SlotKind::Float64],
                stack_depth: 0,
                innermost_function_id: None,
                inline_frames: Vec::new(),
            },
            DeoptInfo {
                resume_ip: 25,
                local_mapping: vec![(0, 2)],
                local_kinds: vec![SlotKind::Int64],
                stack_depth: 1,
                innermost_function_id: None,
                inline_frames: Vec::new(),
            },
        ],
        loop_header_ip: None,
        shape_guards: Vec::new(),
    };

    assert_eq!(result.deopt_points.len(), 2);
    assert_eq!(result.deopt_points[0].resume_ip, 10);
    assert_eq!(result.deopt_points[1].resume_ip, 25);
    assert_eq!(result.deopt_points[1].stack_depth, 1);
}

#[test]
fn test_compilation_request_with_feedback() {
    use shape_vm::feedback::FeedbackVector;
    use shape_vm::tier::{CompilationRequest, Tier};

    let mut fv = FeedbackVector::new(42);
    fv.record_call(10, 5);
    fv.record_arithmetic(20, 1, 1);

    let request = CompilationRequest {
        function_id: 42,
        target_tier: Tier::OptimizingJit,
        blob_hash: None,
        osr: false,
        loop_header_ip: None,
        feedback: Some(fv),
        callee_feedback: std::collections::HashMap::new(),
    };

    assert_eq!(request.function_id, 42);
    assert_eq!(request.target_tier, Tier::OptimizingJit);
    assert!(request.feedback.is_some());

    let fv = request.feedback.as_ref().unwrap();
    assert_eq!(fv.function_id, 42);
    assert!(fv.is_monomorphic(10));
    assert!(fv.is_monomorphic(20));
    assert_eq!(fv.monomorphic_ratio(), 1.0);
}

// =========================================================================
// Workstream A+B: Typed deopt metadata + unboxed local spill tests
// =========================================================================

#[test]
fn test_deopt_info_slot_kind_int64_for_unboxed_locals() {
    use shape_vm::type_tracking::SlotKind;

    // Simulate a DeoptInfo that would be produced for an unboxed int local
    let deopt = DeoptInfo {
        resume_ip: 10,
        local_mapping: vec![(0, 0), (1, 1), (128, 2)],
        local_kinds: vec![SlotKind::Int64, SlotKind::NanBoxed, SlotKind::NanBoxed],
        stack_depth: 1,
        innermost_function_id: None,
        inline_frames: Vec::new(),
    };

    // Verify Int64 local is properly tagged
    assert_eq!(deopt.local_kinds[0], SlotKind::Int64);
    // Regular local is NanBoxed (NaN-boxed passthrough)
    assert_eq!(deopt.local_kinds[1], SlotKind::NanBoxed);
    // Stack values are NanBoxed
    assert_eq!(deopt.local_kinds[2], SlotKind::NanBoxed);
}

#[test]
fn test_deopt_info_slot_kind_float64_for_unboxed_locals() {
    use shape_vm::type_tracking::SlotKind;

    let deopt = DeoptInfo {
        resume_ip: 20,
        local_mapping: vec![(0, 0), (1, 1)],
        local_kinds: vec![SlotKind::Float64, SlotKind::NanBoxed],
        stack_depth: 0,
        innermost_function_id: None,
        inline_frames: Vec::new(),
    };

    assert_eq!(deopt.local_kinds[0], SlotKind::Float64);
    assert_eq!(deopt.local_kinds[1], SlotKind::NanBoxed);
}

#[test]
fn test_verify_deopt_points_passes_for_correct_metadata() {
    use shape_vm::type_tracking::SlotKind;
    use std::collections::HashSet;

    let points = vec![DeoptInfo {
        resume_ip: 10,
        local_mapping: vec![(0, 0), (1, 1)],
        local_kinds: vec![SlotKind::Int64, SlotKind::Float64],
        stack_depth: 0,
        innermost_function_id: None,
        inline_frames: Vec::new(),
    }];

    let mut unboxed_ints = HashSet::new();
    unboxed_ints.insert(0u16);
    let mut unboxed_f64s = HashSet::new();
    unboxed_f64s.insert(1u16);

    let result = BytecodeToIR::verify_deopt_points(&points, &unboxed_ints, &unboxed_f64s);
    assert!(result.is_ok());
}

#[test]
fn test_verify_deopt_points_fails_for_unboxed_int_tagged_unknown() {
    use shape_vm::type_tracking::SlotKind;
    use std::collections::HashSet;

    let points = vec![DeoptInfo {
        resume_ip: 10,
        local_mapping: vec![(0, 0)],
        local_kinds: vec![SlotKind::Unknown], // Wrong! Should be Int64
        stack_depth: 0,
        innermost_function_id: None,
        inline_frames: Vec::new(),
    }];

    let mut unboxed_ints = HashSet::new();
    unboxed_ints.insert(0u16);
    let unboxed_f64s = HashSet::new();

    let result = BytecodeToIR::verify_deopt_points(&points, &unboxed_ints, &unboxed_f64s);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("tagged as Unknown"));
}

#[test]
fn test_verify_deopt_points_fails_for_unboxed_f64_tagged_unknown() {
    use shape_vm::type_tracking::SlotKind;
    use std::collections::HashSet;

    let points = vec![DeoptInfo {
        resume_ip: 10,
        local_mapping: vec![(0, 0)],
        local_kinds: vec![SlotKind::Unknown], // Wrong! Should be Float64
        stack_depth: 0,
        innermost_function_id: None,
        inline_frames: Vec::new(),
    }];

    let unboxed_ints = HashSet::new();
    let mut unboxed_f64s = HashSet::new();
    unboxed_f64s.insert(0u16);

    let result = BytecodeToIR::verify_deopt_points(&points, &unboxed_ints, &unboxed_f64s);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("tagged as Unknown"));
}

#[test]
fn test_verify_deopt_points_allows_empty_deopt() {
    use std::collections::HashSet;

    // Empty deopt points (generic fallback) should pass
    let points = vec![DeoptInfo {
        resume_ip: 10,
        local_mapping: vec![],
        local_kinds: vec![],
        stack_depth: 0,
        innermost_function_id: None,
        inline_frames: Vec::new(),
    }];

    let mut unboxed_ints = HashSet::new();
    unboxed_ints.insert(0u16);
    let unboxed_f64s = HashSet::new();

    let result = BytecodeToIR::verify_deopt_points(&points, &unboxed_ints, &unboxed_f64s);
    assert!(result.is_ok());
}

#[test]
fn test_verify_deopt_points_rejects_length_mismatch() {
    use shape_vm::type_tracking::SlotKind;
    use std::collections::HashSet;

    let points = vec![DeoptInfo {
        resume_ip: 10,
        local_mapping: vec![(0, 0), (1, 1)],
        local_kinds: vec![SlotKind::Unknown], // Length mismatch!
        stack_depth: 0,
        innermost_function_id: None,
        inline_frames: Vec::new(),
    }];

    let result = BytecodeToIR::verify_deopt_points(&points, &HashSet::new(), &HashSet::new());
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .contains("local_mapping len 2 != local_kinds len 1")
    );
}

// =========================================================================
// Workstream C: Multi-frame inline deopt tests
// =========================================================================

#[test]
fn test_inline_frame_info_construction() {
    use shape_vm::bytecode::InlineFrameInfo;
    use shape_vm::type_tracking::SlotKind;

    let frame = InlineFrameInfo {
        function_id: 5,
        resume_ip: 42,
        local_mapping: vec![(200, 0), (201, 1)],
        local_kinds: vec![SlotKind::NanBoxed, SlotKind::Int64],
        stack_depth: 0,
    };

    assert_eq!(frame.function_id, 5);
    assert_eq!(frame.resume_ip, 42);
    assert_eq!(frame.local_mapping.len(), 2);
}

#[test]
fn test_deopt_info_with_inline_frames_roundtrip() {
    use shape_vm::bytecode::InlineFrameInfo;
    use shape_vm::type_tracking::SlotKind;

    let deopt = DeoptInfo {
        resume_ip: 15,
        local_mapping: vec![(0, 0)],
        local_kinds: vec![SlotKind::NanBoxed],
        stack_depth: 0,
        innermost_function_id: Some(5),
        inline_frames: vec![InlineFrameInfo {
            function_id: 3,
            resume_ip: 50,
            local_mapping: vec![(200, 0), (201, 1)],
            local_kinds: vec![SlotKind::NanBoxed, SlotKind::Float64],
            stack_depth: 1,
        }],
    };

    let json = serde_json::to_string(&deopt).expect("serialize");
    let roundtripped: DeoptInfo = serde_json::from_str(&json).expect("deserialize");

    assert_eq!(roundtripped.inline_frames.len(), 1);
    assert_eq!(roundtripped.inline_frames[0].function_id, 3);
    assert_eq!(roundtripped.inline_frames[0].resume_ip, 50);
    assert_eq!(
        roundtripped.inline_frames[0].local_kinds[1],
        SlotKind::Float64
    );
}

#[test]
fn test_deopt_info_backward_compat_deserialize_no_inline_frames() {
    // Simulate old serialized DeoptInfo without inline_frames field
    let json =
        r#"{"resume_ip":10,"local_mapping":[[0,0]],"local_kinds":["Unknown"],"stack_depth":0}"#;
    let deopt: DeoptInfo = serde_json::from_str(json).expect("deserialize old format");

    assert_eq!(deopt.resume_ip, 10);
    assert!(deopt.inline_frames.is_empty()); // defaults to empty
}

// =========================================================================
// Workstream D: Cross-function speculative direct calls tests
// =========================================================================

#[test]
fn test_speculative_call_target_returns_cross_function_target() {
    use shape_vm::feedback::FeedbackVector;

    // Create feedback showing function 5 is called monomorphically at IP 10
    let mut fv = FeedbackVector::new(0);
    fv.record_call(10, 5);

    // speculative_call_target should return 5 even without a user_func ref
    // (The test validates the feedback lookup; actual JIT integration is tested
    // via integration tests.)
    assert!(fv.is_monomorphic(10));
    assert_eq!(
        fv.get_slot(10).and_then(|s| match s {
            shape_vm::feedback::FeedbackSlot::Call(fb)
                if fb.state == shape_vm::feedback::ICState::Monomorphic =>
                Some(fb.targets[0].function_id),
            _ => None,
        }),
        Some(5)
    );
}

#[test]
fn test_cross_function_speculation_without_func_ref() {
    // Validate that speculative_call_target returns a target even when
    // user_funcs has no entry for it (cross-function V1 path).
    use shape_vm::feedback::FeedbackVector;

    let mut fv = FeedbackVector::new(0);
    fv.record_call(10, 7); // function 7 at IP 10

    let user_funcs: std::collections::HashMap<u16, cranelift::codegen::ir::FuncRef> =
        std::collections::HashMap::new();

    // speculative_call_target returns 7
    let target = fv
        .get_slot(10)
        .and_then(|s| match s {
            shape_vm::feedback::FeedbackSlot::Call(fb)
                if fb.state == shape_vm::feedback::ICState::Monomorphic =>
            {
                Some(fb.targets[0].function_id)
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(target, 7);

    // No FuncRef → cross-function guard+FFI path is taken (not direct call).
    assert!(!user_funcs.contains_key(&target));
}

#[test]
fn test_cross_function_direct_call_with_compiled_dc_func() {
    // When a callee has been Tier-2 compiled (present in compiled_dc_funcs),
    // compile_optimizing_function should declare its FuncRef so compile_call_value
    // can emit a direct `call` instruction instead of FFI.
    //
    // We verify the data flow: compiled_dc_funcs maps function_id → (FuncId, arity),
    // and the feedback scan selects monomorphic cross-function targets from it.
    use shape_vm::feedback::FeedbackVector;

    let mut fv = FeedbackVector::new(0);
    fv.record_call(10, 7); // function 7 at IP 10 (monomorphic)

    // Simulate that function 7 was already Tier-2 compiled
    let compiled_dc_funcs: std::collections::HashMap<u16, (u32, u16)> =
        std::collections::HashMap::from([(7, (42u32, 2u16))]); // FuncId placeholder, arity 2

    // The cross-function scan should find function 7 in compiled_dc_funcs
    let mut cross_func_targets = Vec::new();
    let func_index: u16 = 0; // compiling function 0
    for (_offset, slot) in fv.slots.iter() {
        if let shape_vm::feedback::FeedbackSlot::Call(fb) = slot {
            if fb.state == shape_vm::feedback::ICState::Monomorphic {
                if let Some(target) = fb.targets.first() {
                    let target_id = target.function_id;
                    if target_id != func_index {
                        if let Some(&(func_id, arity)) = compiled_dc_funcs.get(&target_id) {
                            cross_func_targets.push((target_id, func_id, arity));
                        }
                    }
                }
            }
        }
    }

    assert_eq!(cross_func_targets.len(), 1);
    assert_eq!(cross_func_targets[0].0, 7); // target function_id
    assert_eq!(cross_func_targets[0].2, 2); // arity
}

#[test]
fn test_cross_function_scan_excludes_self_recursive() {
    // The cross-function scan should NOT declare a FuncRef for the function
    // being compiled (self-recursive calls are handled separately).
    use shape_vm::feedback::FeedbackVector;

    let mut fv = FeedbackVector::new(0);
    fv.record_call(10, 3); // self-recursive: function 3 calls itself

    let compiled_dc_funcs: std::collections::HashMap<u16, (u32, u16)> =
        std::collections::HashMap::from([(3, (99u32, 1u16))]);

    let func_index: u16 = 3; // compiling function 3
    let mut cross_func_targets = Vec::new();
    for (_offset, slot) in fv.slots.iter() {
        if let shape_vm::feedback::FeedbackSlot::Call(fb) = slot {
            if fb.state == shape_vm::feedback::ICState::Monomorphic {
                if let Some(target) = fb.targets.first() {
                    let target_id = target.function_id;
                    if target_id != func_index {
                        if let Some(&(func_id, arity)) = compiled_dc_funcs.get(&target_id) {
                            cross_func_targets.push((target_id, func_id, arity));
                        }
                    }
                }
            }
        }
    }

    // Self-recursive target should be excluded
    assert!(cross_func_targets.is_empty());
}

#[test]
fn test_cross_function_scan_skips_uncompiled_callees() {
    // When the callee has NOT been Tier-2 compiled (not in compiled_dc_funcs),
    // no FuncRef is declared — falls back to guard+FFI.
    use shape_vm::feedback::FeedbackVector;

    let mut fv = FeedbackVector::new(0);
    fv.record_call(10, 7); // function 7

    let compiled_dc_funcs: std::collections::HashMap<u16, (u32, u16)> =
        std::collections::HashMap::new(); // empty: nothing compiled yet

    let func_index: u16 = 0;
    let mut cross_func_targets = Vec::new();
    for (_offset, slot) in fv.slots.iter() {
        if let shape_vm::feedback::FeedbackSlot::Call(fb) = slot {
            if fb.state == shape_vm::feedback::ICState::Monomorphic {
                if let Some(target) = fb.targets.first() {
                    let target_id = target.function_id;
                    if target_id != func_index {
                        if let Some(&(func_id, arity)) = compiled_dc_funcs.get(&target_id) {
                            cross_func_targets.push((target_id, func_id, arity));
                        }
                    }
                }
            }
        }
    }

    // No compiled callees → no cross-function direct calls
    assert!(cross_func_targets.is_empty());
}

#[test]
fn test_polymorphic_feedback_rejects_speculation() {
    use shape_vm::feedback::FeedbackVector;

    let mut fv = FeedbackVector::new(0);
    // Record two different targets at same IP → polymorphic
    fv.record_call(10, 5);
    fv.record_call(10, 8);

    assert!(!fv.is_monomorphic(10));

    // speculative_call_target should return None for polymorphic sites
    let target = fv.get_slot(10).and_then(|s| match s {
        shape_vm::feedback::FeedbackSlot::Call(fb)
            if fb.state == shape_vm::feedback::ICState::Monomorphic =>
        {
            Some(fb.targets[0].function_id)
        }
        _ => None,
    });
    assert!(target.is_none());
}

#[test]
fn test_deopt_info_innermost_function_id_serialization() {
    let deopt = DeoptInfo {
        resume_ip: 20,
        local_mapping: vec![(0, 0)],
        local_kinds: vec![SlotKind::NanBoxed],
        stack_depth: 0,
        innermost_function_id: Some(42),
        inline_frames: Vec::new(),
    };

    let json = serde_json::to_string(&deopt).expect("serialize");
    let roundtripped: DeoptInfo = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(roundtripped.innermost_function_id, Some(42));
}

#[test]
fn test_deopt_info_innermost_function_id_defaults_none() {
    // Old format without innermost_function_id should default to None
    let json =
        r#"{"resume_ip":10,"local_mapping":[[0,0]],"local_kinds":["Unknown"],"stack_depth":0}"#;
    let deopt: DeoptInfo = serde_json::from_str(json).expect("deserialize old format");
    assert_eq!(deopt.innermost_function_id, None);
}

#[test]
fn test_verify_deopt_points_ctx_bounds_check() {
    // Verify that ctx_pos >= 208 is rejected
    let points = vec![DeoptInfo {
        resume_ip: 5,
        local_mapping: vec![(210, 0)], // ctx_pos 210 > 208 limit
        local_kinds: vec![SlotKind::NanBoxed],
        stack_depth: 0,
        innermost_function_id: None,
        inline_frames: Vec::new(),
    }];
    let result = BytecodeToIR::verify_deopt_points(
        &points,
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
    );
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("exceeds ctx_buf limit"));
}

#[test]
fn test_verify_deopt_points_inline_frame_bounds_check() {
    use shape_vm::bytecode::InlineFrameInfo;

    let points = vec![DeoptInfo {
        resume_ip: 5,
        local_mapping: vec![(0, 0)],
        local_kinds: vec![SlotKind::NanBoxed],
        stack_depth: 0,
        innermost_function_id: Some(2),
        inline_frames: vec![InlineFrameInfo {
            function_id: 1,
            resume_ip: 20,
            local_mapping: vec![(250, 0)], // ctx_pos 250 > 208 limit
            local_kinds: vec![SlotKind::NanBoxed],
            stack_depth: 0,
        }],
    }];
    let result = BytecodeToIR::verify_deopt_points(
        &points,
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
    );
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("inline_frames"));
}

#[test]
fn test_verify_deopt_points_inline_frame_length_mismatch() {
    use shape_vm::bytecode::InlineFrameInfo;

    let points = vec![DeoptInfo {
        resume_ip: 5,
        local_mapping: vec![(0, 0)],
        local_kinds: vec![SlotKind::NanBoxed],
        stack_depth: 0,
        innermost_function_id: Some(2),
        inline_frames: vec![InlineFrameInfo {
            function_id: 1,
            resume_ip: 20,
            local_mapping: vec![(100, 0), (101, 1)],
            local_kinds: vec![SlotKind::NanBoxed], // only 1, but 2 mappings
            stack_depth: 0,
        }],
    }];
    let result = BytecodeToIR::verify_deopt_points(
        &points,
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
    );
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("inline_frames"));
}

// =========================================================================
// Tier-2 inline-deopt integration test
//
// Exercises the full pipeline: compile_optimizing_function → inlining →
// speculative guard → deopt point with inline_frames.
// =========================================================================

#[test]
fn test_tier2_inline_deopt_produces_inline_frames() {
    use crate::compiler::JITCompiler;
    use crate::context::JITConfig;
    use shape_value::FunctionId;
    use shape_vm::feedback::FeedbackVector;

    // Program layout:
    //
    //   outer (fn_id=0, entry_point=0, arity=1, locals=2):
    //     0: LoadLocal(0)            // push x
    //     1: PushConst(0)            // push arg_count = 1.0
    //     2: Call(Function(1))       // call inner(x) — inlining target
    //     3: StoreLocal(1)           // store result
    //     4: LoadLocal(1)            // push result
    //     5: ReturnValue
    //
    //   inner (fn_id=1, entry_point=6, arity=1, locals=1):
    //     6: LoadLocal(0)            // push x
    //     7: LoadLocal(0)            // push x again
    //     8: Add                     // speculative int add (has feedback)
    //     9: ReturnValue
    //
    let instrs = vec![
        // outer body [0..6]
        make_instr(OpCode::LoadLocal, Some(Operand::Local(0))),
        make_instr(OpCode::PushConst, Some(Operand::Const(0))),
        make_instr(OpCode::Call, Some(Operand::Function(FunctionId(1)))),
        make_instr(OpCode::StoreLocal, Some(Operand::Local(1))),
        make_instr(OpCode::LoadLocal, Some(Operand::Local(1))),
        make_instr(OpCode::ReturnValue, None),
        // inner body [6..10]
        make_instr(OpCode::LoadLocal, Some(Operand::Local(0))),
        make_instr(OpCode::LoadLocal, Some(Operand::Local(0))),
        make_instr(OpCode::Add, None),
        make_instr(OpCode::ReturnValue, None),
    ];

    let functions = vec![
        make_func_with_body("outer", 1, 2, 0, 6),
        make_func_with_body("inner", 1, 1, 6, 4),
    ];

    let program = BytecodeProgram {
        instructions: instrs,
        constants: vec![Constant::Number(1.0)], // arg_count = 1
        strings: vec![],
        functions,
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
    };

    // Outer feedback: monomorphic call to fn_id=1 at IP 2 (global).
    let mut outer_fv = FeedbackVector::new(0);
    outer_fv.record_call(2, 1);

    // Inner feedback: monomorphic int+int arithmetic at IP 8 (global).
    // Tag 1 = I48 (inline integer).
    let mut inner_fv = FeedbackVector::new(1);
    inner_fv.record_arithmetic(8, 1, 1);

    let mut callee_feedback = std::collections::HashMap::new();
    callee_feedback.insert(1u16, inner_fv);

    let mut jit = JITCompiler::new(JITConfig::default()).expect("JITCompiler init");
    let result = jit.compile_optimizing_function(&program, 0, outer_fv, &callee_feedback);

    let (code_ptr, deopt_points, _shape_guards) = result.expect("compile_optimizing_function");

    // Compilation should produce real native code.
    assert!(!code_ptr.is_null(), "code_ptr should be non-null");

    // The speculative int add inside inlined `inner` should have produced
    // at least one deopt point with inline_frames (because inline_depth > 0
    // when the guard fires inside `inner`).
    let inline_deopts: Vec<_> = deopt_points
        .iter()
        .filter(|dp| !dp.inline_frames.is_empty())
        .collect();

    assert!(
        !inline_deopts.is_empty(),
        "Expected at least one deopt point with inline_frames, got {} total deopt points: {:?}",
        deopt_points.len(),
        deopt_points
            .iter()
            .map(|dp| (dp.resume_ip, dp.inline_frames.len()))
            .collect::<Vec<_>>()
    );

    // Verify the inline frame structure: the innermost function should be inner (fn_id=1).
    for dp in &inline_deopts {
        assert_eq!(
            dp.innermost_function_id,
            Some(1),
            "innermost_function_id should be inner's fn_id=1"
        );
        // inline_frames should contain the outer (caller) frame.
        // Outermost-first ordering: inline_frames[0] = outermost caller.
        assert!(
            !dp.inline_frames.is_empty(),
            "inline_frames should be non-empty"
        );
        let caller_frame = &dp.inline_frames[0];
        assert_eq!(
            caller_frame.function_id, 0,
            "caller frame function_id should be outer's fn_id=0"
        );
    }

    // Resume IPs should be rebased to global program coordinates:
    // - Inner's Add was at global IP 8, so resume_ip should be 8.
    // - Caller frame's resume_ip is the call site in outer (IP 2).
    let add_deopt = inline_deopts
        .iter()
        .find(|dp| dp.resume_ip == 8)
        .expect("should have deopt at inner's Add (global IP=8)");
    assert_eq!(
        add_deopt.inline_frames[0].resume_ip, 2,
        "caller resume_ip should be call site IP=2"
    );
}
