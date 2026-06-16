//! Tests for unified `?` semantics (Result + Option + nullable Option encoding).

use crate::VMConfig;
use crate::bytecode::*;
use crate::compiler::BytecodeCompiler;
use crate::executor::VirtualMachine;
use shape_ast::parser::parse_program;
use shape_value::VMError;

// Phase-2c surface (helper deleted): see playbook §7 REVISED part 4 + ADR-006 §2.7.4.

// Phase-2c surface (helper deleted): see playbook §7 REVISED part 4 + ADR-006 §2.7.4.

fn compile_source(source: &str) -> Result<BytecodeProgram, VMError> {
    let program = parse_program(source).map_err(|e| VMError::RuntimeError(format!("{:?}", e)))?;
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
    todo!(
        "phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted execute_bytecode_with_vm helper)"
    )
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
    todo!(
        "phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild — deleted execute_source_with_vm helper)"
    )
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
    assert_eq!(
        result.as_f64(),
        Some(42.0),
        "Some(42) as number should be 42.0"
    );
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

// =========================================================================
// PB5 (v0.3.3 Wave-1-extension, 2026-05-29) — direct fallible cast
// `expr as Type?` yields None on conversion failure, not a throw.
//
// `TryConvertTo*` is the fallible cast opcode. Its result is an
// `Option<Target>` in the null-coded convention `op_try_unwrap` consumes:
// a bare scalar ≡ Some, the `(0, NativeKind::Null)` sentinel ≡ None. A
// successful parse produces Some(v); a FAILED parse produces None — so
// the enclosing `?` propagates Err rather than the cast throwing. Before
// PB5 the `op_try_convert_to_*` bodies delegated to the THROWING
// infallible `op_convert_to_*`, so `(raw as int?)?` on a non-numeric
// string threw "cannot convert string '…' to int" instead of yielding
// None for `?` to propagate. Root: `executor/builtins/type_ops.rs`
// `op_try_convert_to_int` family.
// =========================================================================

#[test]
fn pb5_direct_string_as_int_fallible_success_yields_some() {
    // "42" as int? → Some(42): the bare scalar (null-coded Some).
    let source = r#"
"42" as int?
"#;
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
    assert_eq!(
        result.as_i64(),
        Some(42),
        "\"42\" as int? should be Some(42)"
    );
}

#[test]
fn pb5_direct_string_as_int_fallible_failure_yields_none_not_throw() {
    // "not-int" as int? → None (null sentinel). Must NOT throw — this is
    // the PB5 root defect: the fallible cast on a non-numeric string used
    // to surface VMError::RuntimeError instead of None.
    let source = r#"
("not-int" as int?) == None
"#;
    let bytecode = compile_source(source).expect("compile should succeed");
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(bytecode);
    let raw = vm
        .execute_raw(None)
        .expect("execution should succeed (None, not a throw)");
    assert_eq!(raw, 1u64, "\"not-int\" as int? should be None");
}

#[test]
fn pb5_direct_string_as_int_fallible_failure_propagates_via_try() {
    // The end-to-end shape from the failing shape-test fixture: a failed
    // fallible cast feeds `?`, which lifts None to Err in a Result-fn and
    // returns it — `match` then hits the Err arm (no uncaught throw).
    let source = r#"
fn parse(raw: string) -> Result<int> {
    let n = (raw as int?)?
    Ok(n)
}

match parse("not-int") {
    Ok(v) => v
    Err(_) => -1
}
"#;
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
    assert_eq!(
        result.as_i64(),
        Some(-1),
        "failed fallible cast should propagate Err through ? to the match Err arm"
    );
}

// =========================================================================
// WS-12 — Option as-cast Some/None symmetry regression tests
//
// The `Option<T> as U` cast is an element-wise lift: `Some(x) as U`
// unwraps to `x` and converts; `None as U` stays the null sentinel.
// Before WS-12 the `Some` path fed the whole `Ptr(HeapKind::Option)`
// carrier into the `ConvertTo*` opcode (which only accepts proven
// scalar/heap-numeric source kinds) and surfaced
// `"cannot convert kind Ptr(Option) to <U>"`, while the `None` path
// (null sentinel, skipped by `IsNull`) succeeded — an asymmetric,
// inconsistent cast. The fix inserts the missing `UnwrapOption` step
// in `emit_option_lift_infallible` / `emit_option_lift_fallible`,
// mirroring `emit_result_lift_infallible`'s `IsOk; UnwrapOk; convert`.
// =========================================================================

#[test]
fn ws12_option_some_and_none_as_int_are_symmetric() {
    // Both the Some path and the None path must execute cleanly — the
    // Some path lifts to the converted inner value, the None path stays
    // null. Neither errors. (GATING-B #13/#14 root defect.)
    let some_src = r#"
let s: Option<int> = Some(7)
s as int
"#;
    let mut bytecode = compile_source(some_src).expect("Some cast should compile");
    let mut frame = bytecode
        .top_level_frame
        .clone()
        .unwrap_or_else(crate::type_tracking::FrameDescriptor::new);
    frame.return_kind = Some(crate::type_tracking::NativeKind::Int64);
    bytecode.top_level_frame = Some(frame);
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(bytecode);
    let some_result = vm
        .execute(None)
        .expect("Some(7) as int must succeed, not error")
        .clone();
    assert_eq!(
        some_result.as_i64(),
        Some(7),
        "Some(7) as int should lift to the unwrapped converted value 7"
    );

    let none_src = r#"
let n: Option<int> = None
n as int
"#;
    let none_bytecode = compile_source(none_src).expect("None cast should compile");
    let mut none_vm = VirtualMachine::new(VMConfig::default());
    none_vm.load_program(none_bytecode);
    let none_result = none_vm.execute(None);
    assert!(
        none_result.is_ok(),
        "None as int should succeed (the null sentinel passes through), got: {:?}",
        none_result.err()
    );
}

#[test]
fn ws12_option_some_int_as_number_unwraps_and_converts() {
    // `Some(13) as number` must lift to the bare converted scalar 13.0
    // — the cast unwraps the Some payload, never feeds the Option
    // carrier into ConvertToNumber.
    let source = r#"
let s: Option<int> = Some(13)
s as number
"#;
    let bytecode = compile_source(source).expect("compile should succeed");
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(bytecode);
    let result = vm.execute(None).expect("execution should succeed").clone();
    assert_eq!(
        result.as_f64(),
        Some(13.0),
        "Some(13) as number should be 13.0, not a Ptr(Option) carrier"
    );
}

#[test]
fn direct_number_as_int_accepts_at_compile_time() {
    // THE RULE (user 2026-06-01) / numeric-conversion-spec §3.2: `number as int`
    // is a LEGAL explicit infallible cast that truncates toward zero. It must
    // COMPILE (the inference + compiler cast gates now recognize the primitive
    // numeric lattice and bypass the user-`Into` requirement — D1 root fix). The
    // runtime truncation semantics (`3.7 as int == 3`) are pinned by the
    // permanent conformance suite (`tools/shape-test/tests/numeric_conversions`,
    // category C) and finalized in the runtime stage; here we only assert the
    // cast is accepted by the compiler. (Pre-RULE this test asserted a
    // compile-reject because `number` has only `TryInto<int>`, not `Into<int>`;
    // the RULE overturns that.)
    let source = r#"
let x: number = 42.0
let y = x as int
"#;
    let result = compile_source(source);
    assert!(
        result.is_ok(),
        "number as int should compile (primitive numeric cast, truncates at runtime), got: {:?}",
        result.err()
    );
}

#[test]
fn test_uncaught_non_any_error_uses_value_formatting() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

#[test]
fn null_coalesce_unwraps_some_option_to_inner() {
    // v0.3.3 book-gate fix: `Some(5) ?? 99` must UNWRAP the Option carrier
    // to its inner `5` (was leaking the whole `Some(5)` wrapper). The
    // `CoalesceProbe` opcode replaces the old `Dup; IsNull` prologue.
    let some_src = r#"
let v = Some(5) ?? 99
v
"#;
    let bytecode = compile_source(some_src).expect("compile should succeed");
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(bytecode);
    let result = vm.execute(None).expect("execution should succeed").clone();
    assert_eq!(
        result.as_i64(),
        Some(5),
        "Some(5) ?? 99 must unwrap to 5, not leak the Option wrapper"
    );
}

#[test]
fn null_coalesce_none_option_uses_default() {
    // `None ?? 99` (typed Option<int> None) must take the default `99`.
    let none_src = r#"
let n: Option<int> = None
let v = n ?? 99
v
"#;
    let bytecode = compile_source(none_src).expect("compile should succeed");
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(bytecode);
    let result = vm.execute(None).expect("execution should succeed").clone();
    assert_eq!(
        result.as_i64(),
        Some(99),
        "None ?? 99 must yield the default 99"
    );
}

#[test]
fn null_coalesce_some_with_mismatched_default_type_is_rejected() {
    // `Some(5) ?? "x"`: the default must match the unwrapped inner type
    // `int`; a `string` default is a type error (the `??` result types as
    // the unwrapped `T`, not `Option<T>`).
    let bad_src = r#"
fn f() -> int {
    return Some(5) ?? "x"
}
f()
"#;
    let compiled = compile_source(bad_src);
    assert!(
        compiled.is_err(),
        "Some(5) ?? \"x\" must be rejected (default type must match unwrapped T)"
    );
}
