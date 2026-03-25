//! Tests for unified `?` semantics (Result + Option + nullable Option encoding).

use crate::VMConfig;
use crate::bytecode::*;
use crate::compiler::BytecodeCompiler;
use crate::executor::VirtualMachine;
use shape_ast::parser::parse_program;
use shape_value::{VMError, ValueWord};
use std::collections::HashMap;
use std::sync::Arc;

fn execute_bytecode(
    instructions: Vec<Instruction>,
    constants: Vec<Constant>,
) -> Result<ValueWord, VMError> {
    let (result, _vm) = execute_bytecode_with_vm(instructions, constants)?;
    Ok(result)
}

fn execute_bytecode_with_vm(
    instructions: Vec<Instruction>,
    constants: Vec<Constant>,
) -> Result<(ValueWord, VirtualMachine), VMError> {
    let program = BytecodeProgram {
        instructions,
        constants,
        ..Default::default()
    };

    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(program);
    let result = vm.execute(None)?.clone();
    Ok((result, vm))
}

fn compile_source(source: &str) -> Result<BytecodeProgram, VMError> {
    let program =
        parse_program(source).map_err(|e| VMError::RuntimeError(format!("{:?}", e)))?;
    let mut loader = shape_runtime::module_loader::ModuleLoader::new();
    let (graph, stdlib_names, prelude_imports) =
        crate::module_resolution::build_graph_and_stdlib_names(&program, &mut loader, &[])
            .map_err(|e| VMError::RuntimeError(format!("{:?}", e)))?;
    let mut compiler = BytecodeCompiler::new();
    compiler.stdlib_function_names = stdlib_names;
    compiler.set_source(source);
    let bytecode = compiler
        .compile_with_graph_and_prelude(&program, graph, &prelude_imports)
        .map_err(|e| VMError::RuntimeError(format!("{:?}", e)))?;
    Ok(bytecode)
}

fn execute_source_with_vm(source: &str) -> Result<(ValueWord, VirtualMachine), VMError> {
    let bytecode = compile_source(source)?;
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(bytecode);
    let result = vm.execute(None)?.clone();
    Ok((result, vm))
}

/// Slot-based TypedObject to HashMap conversion for test assertions.
/// Looks up schemas from: program registry, then runtime registry.
fn to_obj_map(err: &ValueWord, vm: &VirtualMachine) -> HashMap<String, ValueWord> {
    if let Some((schema_id, slots, heap_mask)) = err.as_typed_object() {
        let sid = schema_id as u32;
        let schema = vm.lookup_schema(sid);
        if let Some(schema) = schema {
            let mut map = HashMap::with_capacity(schema.fields.len());
            for (i, field_def) in schema.fields.iter().enumerate() {
                if i < slots.len() {
                    let val = if heap_mask & (1u64 << i) != 0 {
                        ValueWord::from_heap_value(slots[i].as_heap_value().clone())
                    } else {
                        ValueWord::from_f64(slots[i].as_f64())
                    };
                    map.insert(field_def.name.clone(), val);
                }
            }
            return map;
        }
    }
    shape_runtime::type_schema::typed_object_to_hashmap(err).expect("Expected object-like value")
}

fn assert_any_error_payload(err: &ValueWord, expected_payload: &str, vm: &VirtualMachine) {
    let obj = to_obj_map(err, vm);

    if let Some(cat) = obj.get("category") {
        let s = cat
            .as_arc_string()
            .expect("Expected category to be a string");
        assert_eq!(s.as_ref() as &str, "AnyError");
    } else {
        panic!("Expected category AnyError, got None");
    }

    if let Some(payload) = obj.get("payload") {
        let s = payload.as_arc_string().unwrap_or_else(|| {
            panic!(
                "Expected payload string '{}', got {:?}",
                expected_payload, payload
            )
        });
        assert_eq!(s.as_ref() as &str, expected_payload);
    } else {
        panic!("Expected payload string '{}', got None", expected_payload);
    }
}

#[test]
fn test_try_unwrap_ok_extracts_inner_value() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::simple(OpCode::TryUnwrap),
    ];
    let constants = vec![Constant::Value(ValueWord::from_ok(ValueWord::from_i64(42)))];

    let result = execute_bytecode(instructions, constants).expect("execution should succeed");
    assert_eq!(result, ValueWord::from_i64(42));
}

#[test]
fn test_fallible_type_assertion_accepts_local_try_into_impl() {
    let source = r#"
impl TryInto<int> for string as int {
    method tryInto() {
        self as int?
    }
}

fn parse(raw: string) -> Result<int> {
    let n = (raw as int?)?
    Ok(n)
}

match parse("12") {
    Ok(v) => v
    Err(_) => -1
}
"#;

    let program = parse_program(source).unwrap_or_else(|e| panic!("source should parse: {e:?}"));
    let mut compiler = BytecodeCompiler::new();
    compiler.set_source(source);
    let compiled = compiler.compile(&program);
    assert!(
        compiled.is_ok(),
        "compiler should accept local TryInto impl for fallible assertion: {:?}",
        compiled.err()
    );
}

#[test]
fn test_try_unwrap_err_raises_uncaught_exception_at_top_level() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::simple(OpCode::TryUnwrap),
        // Must not execute because TryUnwrap should return early on Err.
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
    ];
    let constants = vec![
        Constant::Value(ValueWord::from_err(ValueWord::from_string(Arc::new(
            "boom".to_string(),
        )))),
        Constant::Int(999),
    ];

    let err = execute_bytecode(instructions, constants).expect_err("execution should fail");
    let VMError::RuntimeError(message) = err else {
        panic!("Expected runtime error, got {:?}", err);
    };
    assert!(message.contains("Uncaught exception: boom"), "{}", message);
}

#[test]
fn test_try_unwrap_none_raises_uncaught_exception_at_top_level() {
    let instructions = vec![
        Instruction::simple(OpCode::PushNull),
        Instruction::simple(OpCode::TryUnwrap),
        // Must not execute because TryUnwrap should return early on None.
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
    ];
    let constants = vec![Constant::Int(999)];

    let err = match execute_bytecode_with_vm(instructions, constants) {
        Ok(_) => panic!("execution should fail"),
        Err(err) => err,
    };
    let VMError::RuntimeError(message) = err else {
        panic!("Expected runtime error, got {:?}", err);
    };
    assert!(message.contains("OPTION_NONE"), "{}", message);
    assert!(message.contains("Value was None"), "{}", message);
}

#[test]
fn test_try_unwrap_passes_through_plain_non_none_values() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::simple(OpCode::TryUnwrap),
    ];
    let constants = vec![Constant::Int(7)];

    let result = execute_bytecode(instructions, constants).expect("execution should succeed");
    assert_eq!(result, ValueWord::from_i64(7));
}

#[test]
fn test_try_unwrap_unwraps_explicit_some() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::simple(OpCode::TryUnwrap),
    ];
    let constants = vec![Constant::Value(ValueWord::from_some(ValueWord::from_i64(
        9,
    )))];

    let result = execute_bytecode(instructions, constants).expect("execution should succeed");
    assert_eq!(result, ValueWord::from_i64(9));
}

#[test]
fn test_try_operator_inside_pipe_lambda_compiles() {
    let source = r#"
        let unwrap = |x| x?;
        0
    "#;

    let bytecode = compile_source(source).expect("compilation should succeed");
    assert!(
        bytecode
            .instructions
            .iter()
            .any(|instr| instr.opcode == OpCode::TryUnwrap),
        "expected TryUnwrap opcode in compiled closure bytecode"
    );
}

#[test]
fn test_fallible_type_assertion_compiles_to_try_into_dispatch_metadata() {
    let source = r#"
        let x = "42" as int?
        x
    "#;

    let bytecode = compile_source(source).expect("compilation should succeed");
    // Primitive fallible assertion now emits a typed TryConvertToInt opcode
    // instead of Convert + __TryIntoDispatch metadata.
    assert!(
        bytecode
            .instructions
            .iter()
            .any(|instr| instr.opcode == OpCode::TryConvertToInt),
        "expected TryConvertToInt opcode in compiled bytecode"
    );
}

#[test]
fn test_infallible_type_assertion_compiles_to_into_dispatch_metadata() {
    let source = r#"
        let x = true as int
        x
    "#;

    let bytecode = compile_source(source).expect("compilation should succeed");
    // Primitive infallible assertion now emits a typed ConvertToInt opcode
    // instead of Convert + __IntoDispatch metadata.
    assert!(
        bytecode
            .instructions
            .iter()
            .any(|instr| instr.opcode == OpCode::ConvertToInt),
        "expected ConvertToInt opcode in compiled bytecode"
    );
}

#[test]
fn test_error_context_lifts_ok_into_result_ok() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::simple(OpCode::ErrorContext),
    ];
    let constants = vec![
        Constant::Value(ValueWord::from_ok(ValueWord::from_i64(42))),
        Constant::Value(ValueWord::from_string(Arc::new("ctx".to_string()))),
    ];

    let result = execute_bytecode(instructions, constants).expect("execution should succeed");
    let inner = result.as_ok_inner().expect("Expected Ok(..)");
    assert_eq!(inner.as_i64(), Some(42));
}

#[test]
fn test_error_context_wraps_err_with_context_and_cause() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::simple(OpCode::ErrorContext),
    ];
    let constants = vec![
        Constant::Value(ValueWord::from_err(ValueWord::from_string(Arc::new(
            "low level".to_string(),
        )))),
        Constant::Value(ValueWord::from_string(Arc::new("high level".to_string()))),
    ];

    let (result, vm) =
        execute_bytecode_with_vm(instructions, constants).expect("execution should succeed");
    let outer = result.as_err_inner().expect("Expected Err(..)").clone();
    assert_any_error_payload(&outer, "high level", &vm);

    let outer_map = to_obj_map(&outer, &vm);
    let cause = outer_map.get("cause").expect("cause should be present");
    assert_any_error_payload(cause, "low level", &vm);
}

#[test]
fn test_error_context_wraps_none_with_synthetic_cause() {
    let instructions = vec![
        Instruction::simple(OpCode::PushNull),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::simple(OpCode::ErrorContext),
    ];
    let constants = vec![Constant::Value(ValueWord::from_string(Arc::new(
        "User not found".to_string(),
    )))];

    let (result, vm) =
        execute_bytecode_with_vm(instructions, constants).expect("execution should succeed");
    let outer = result.as_err_inner().expect("Expected Err(..)").clone();
    assert_any_error_payload(&outer, "User not found", &vm);

    let outer_map = to_obj_map(&outer, &vm);
    let cause = outer_map.get("cause").expect("cause should be present");
    assert_any_error_payload(cause, "Value was None", &vm);
}

#[test]
fn test_error_context_then_try_short_circuits_with_err() {
    let instructions = vec![
        Instruction::simple(OpCode::PushNull),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::simple(OpCode::ErrorContext),
        Instruction::simple(OpCode::TryUnwrap),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
    ];
    let constants = vec![
        Constant::Value(ValueWord::from_string(Arc::new(
            "User not found".to_string(),
        ))),
        Constant::Int(999),
    ];

    let err = match execute_bytecode_with_vm(instructions, constants) {
        Ok(_) => panic!("execution should fail"),
        Err(err) => err,
    };
    let VMError::RuntimeError(message) = err else {
        panic!("Expected runtime error, got {:?}", err);
    };
    assert!(message.contains("User not found"), "{}", message);
    assert!(message.contains("Value was None"), "{}", message);
}

#[test]
fn test_error_context_inline_try_syntax_without_parentheses() {
    let source = r#"
        let y = Err("low level") !! "high level context"?;
        y
    "#;

    let err = match execute_source_with_vm(source) {
        Ok(_) => panic!("execution should fail"),
        Err(err) => err,
    };
    let VMError::RuntimeError(message) = err else {
        panic!("Expected runtime error, got {:?}", err);
    };
    assert!(message.contains("high level context"), "{}", message);
    assert!(message.contains("low level"), "{}", message);
}

/// Create a TraceFrame object matching the builtin schema field order:
/// [ip(0), line(1), file(2), function(3)]
fn make_trace_frame(function: &str, file: &str, line: i64, ip: i64) -> ValueWord {
    shape_runtime::type_schema::typed_object_from_pairs(&[
        ("ip", ValueWord::from_i64(ip)),
        ("line", ValueWord::from_i64(line)),
        ("file", ValueWord::from_string(Arc::new(file.to_string()))),
        (
            "function",
            ValueWord::from_string(Arc::new(function.to_string())),
        ),
    ])
}

/// Create a TraceInfoSingle matching builtin schema: [kind(0), frame(1)]
fn make_trace_info_single(frame: ValueWord) -> ValueWord {
    shape_runtime::type_schema::typed_object_from_pairs(&[
        (
            "kind",
            ValueWord::from_string(Arc::new("single".to_string())),
        ),
        ("frame", frame),
    ])
}

/// Create a TraceInfoFull matching builtin schema: [kind(0), frames(1)]
fn make_trace_info_full(frames: Vec<ValueWord>) -> ValueWord {
    shape_runtime::type_schema::typed_object_from_pairs(&[
        ("kind", ValueWord::from_string(Arc::new("full".to_string()))),
        ("frames", ValueWord::from_array(Arc::new(frames))),
    ])
}

/// Create an AnyError object matching the builtin schema field order:
/// [category(0), payload(1), cause(2), trace_info(3), message(4), code(5)]
fn make_any_error(
    payload: &str,
    message: &str,
    cause: ValueWord,
    trace_info: ValueWord,
    code: Option<&str>,
) -> ValueWord {
    shape_runtime::type_schema::typed_object_from_pairs(&[
        (
            "category",
            ValueWord::from_string(Arc::new("AnyError".to_string())),
        ),
        (
            "payload",
            ValueWord::from_string(Arc::new(payload.to_string())),
        ),
        ("cause", cause),
        ("trace_info", trace_info),
        (
            "message",
            ValueWord::from_string(Arc::new(message.to_string())),
        ),
        (
            "code",
            code.map(|c| ValueWord::from_string(Arc::new(c.to_string())))
                .unwrap_or(ValueWord::none()),
        ),
    ])
}

#[test]
fn test_uncaught_any_error_formats_chain_and_trace() {
    let root = make_any_error(
        "low level",
        "low level",
        ValueWord::none(),
        make_trace_info_full(vec![make_trace_frame("read_file", "cfg.shape", 3, 11)]),
        None,
    );
    let outer = make_any_error(
        "high level context",
        "high level context",
        root,
        make_trace_info_single(make_trace_frame("load_config", "cfg.shape", 7, 29)),
        Some("OPTION_NONE"),
    );

    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::simple(OpCode::Throw),
    ];
    let constants = vec![Constant::Value(outer)];

    let err = execute_bytecode(instructions, constants).expect_err("execution should fail");
    let VMError::RuntimeError(message) = err else {
        panic!("Expected runtime error, got {:?}", err);
    };

    assert!(message.contains("Uncaught exception:"));
    assert!(message.contains("Error [OPTION_NONE]: high level context"));
    assert!(message.contains("at load_config (cfg.shape:7) [ip 29]"));
    assert!(message.contains("Caused by: low level"));
    assert!(message.contains("at read_file (cfg.shape:3) [ip 11]"));
}

// =========================================================================
// Option/Result lifting execution tests
// =========================================================================

#[test]
fn option_some_int_as_number_lifts_to_some_number() {
    // Option<int> as number → Option<number>: Some(42) → Some(42.0)
    let source = r#"
let opt: Option<int> = Some(42)
let val = opt as number
val
"#;
    let bytecode = compile_source(source).expect("compile should succeed");
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(bytecode);
    let result = vm.execute(None).expect("execution should succeed").clone();
    assert_eq!(result.as_f64(), Some(42.0), "Some(42) as number should be 42.0");
}

#[test]
fn option_none_as_number_lifts_to_none() {
    // Option<int> as number → Option<number>: None → None
    let source = r#"
let opt: Option<int> = None
let val = opt as number
val == None
"#;
    let bytecode = compile_source(source).expect("compile should succeed");
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(bytecode);
    let result = vm.execute(None).expect("execution should succeed").clone();
    assert_eq!(result.as_bool(), Some(true), "None as number should remain None");
}

#[test]
fn option_some_bool_as_int_lifts_to_some_int() {
    // Option<bool> as int → Option<int>: Some(true) → Some(1)
    let source = r#"
let opt: Option<bool> = Some(true)
let val = opt as int
val
"#;
    let bytecode = compile_source(source).expect("compile should succeed");
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(bytecode);
    let result = vm.execute(None).expect("execution should succeed").clone();
    assert_eq!(result.as_i64(), Some(1), "Some(true) as int should be 1");
}

#[test]
fn invalid_infallible_cast_option_string_as_int_fails_at_runtime() {
    // Option<string> as int: string has no Into<int>, but the type tracker
    // loses generic args for locals so the compiler emits lifting code and
    // the inner conversion fails at runtime.
    let source = r#"
let opt: Option<string> = Some("hello")
let val = opt as int
"#;
    let bytecode = compile_source(source).expect("compile should succeed with bare wrapper");
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(bytecode);
    let result = vm.execute(None);
    assert!(
        result.is_err(),
        "Option<string> as int should fail at runtime, got: {:?}",
        result.ok()
    );
}

#[test]
fn direct_number_as_int_rejected_at_compile_time() {
    // number has no Into<int>, only TryInto<int>
    let source = r#"
let x: number = 42.0
let y = x as int
"#;
    let result = compile_source(source);
    assert!(
        result.is_err(),
        "number as int should be a compile error (no Into<int> for number), got: {:?}",
        result.ok()
    );
}

#[test]
fn test_uncaught_non_any_error_uses_value_formatting() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::simple(OpCode::Throw),
    ];
    let constants = vec![Constant::Int(42)];

    let err = execute_bytecode(instructions, constants).expect_err("execution should fail");
    let VMError::RuntimeError(message) = err else {
        panic!("Expected runtime error, got {:?}", err);
    };

    assert_eq!(message, "Uncaught exception: 42");
}
