//! Tests for unified `?` semantics (Result + Option + nullable Option encoding).

use crate::VMConfig;
use crate::bytecode::*;
use crate::compiler::BytecodeCompiler;
use crate::executor::VirtualMachine;
use shape_ast::parser::parse_program;
use shape_value::VMError;
use std::collections::HashMap;
use std::sync::Arc;

// Phase-2c surface (helper deleted): see playbook §7 REVISED part 4 + ADR-006 §2.7.4.

// Phase-2c surface (helper deleted): see playbook §7 REVISED part 4 + ADR-006 §2.7.4.

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

// Phase-2c surface (helper deleted): see playbook §7 REVISED part 4 + ADR-006 §2.7.4.

/// Slot-based TypedObject to HashMap conversion for test assertions.
/// Looks up schemas from: program registry, then runtime registry.
// Phase-2c surface (helper deleted): see playbook §7 REVISED part 4 + ADR-006 §2.7.4.

// Phase-2c surface (helper deleted): see playbook §7 REVISED part 4 + ADR-006 §2.7.4.

#[test]
fn test_try_unwrap_ok_extracts_inner_value() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
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
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

#[test]
fn test_try_unwrap_none_raises_uncaught_exception_at_top_level() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted execute_bytecode_with_vm helper)")
}

#[test]
fn test_try_unwrap_passes_through_plain_non_none_values() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

#[test]
fn test_try_unwrap_unwraps_explicit_some() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
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
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

#[test]
fn test_error_context_wraps_err_with_context_and_cause() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

#[test]
fn test_error_context_wraps_none_with_synthetic_cause() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

#[test]
fn test_error_context_then_try_short_circuits_with_err() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

#[test]
fn test_error_context_inline_try_syntax_without_parentheses() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted execute_source_with_vm helper)")
}

/// Create a TraceFrame object matching the builtin schema field order:
/// [ip(0), line(1), file(2), function(3)]
// Phase-2c surface (helper deleted): see playbook §7 REVISED part 4 + ADR-006 §2.7.4.

/// Create a TraceInfoSingle matching builtin schema: [kind(0), frame(1)]
// Phase-2c surface (helper deleted): see playbook §7 REVISED part 4 + ADR-006 §2.7.4.

/// Create a TraceInfoFull matching builtin schema: [kind(0), frames(1)]
// Phase-2c surface (helper deleted): see playbook §7 REVISED part 4 + ADR-006 §2.7.4.

/// Create an AnyError object matching the builtin schema field order:
/// [category(0), payload(1), cause(2), trace_info(3), message(4), code(5)]
// Phase-2c surface (helper deleted): see playbook §7 REVISED part 4 + ADR-006 §2.7.4.

#[test]
fn test_uncaught_any_error_formats_chain_and_trace() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
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
    // After Wave-E+5, the trailing `==` may emit raw native bool bits
    // at the top of stack. Read the raw bits and assert the boolean
    // payload directly (`0u64` → false, `1u64` → true).
    let bytecode = compile_source(source).expect("compile should succeed");
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(bytecode);
    let raw = vm.execute_raw(None).expect("execution should succeed");
    assert_eq!(raw, 1u64, "None as number should remain None");
}

#[test]
fn option_some_bool_as_int_lifts_to_some_int() {
    // Option<bool> as int → Option<int>: Some(true) → Some(1)
    let source = r#"
let opt: Option<bool> = Some(true)
let val = opt as int
val
"#;
    // After Wave-E+5, the cast result lands as raw native i64 bits at
    // the top-level. Stamp Int64 kind on the program so the host
    // synthesizer re-tags the bits correctly.
    let mut bytecode = compile_source(source).expect("compile should succeed");
    let mut frame = bytecode
        .top_level_frame
        .clone()
        .unwrap_or_else(crate::type_tracking::FrameDescriptor::new);
    frame.return_kind = Some(crate::type_tracking::NativeKind::Int64);
    bytecode.top_level_frame = Some(frame);
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
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}
