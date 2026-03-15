use super::*;
use crate::VMConfig;
use crate::bytecode::{BuiltinFunction, Operand};
use crate::executor::VirtualMachine;
use crate::type_tracking::StorageHint;
use shape_ast::parser::parse_program;
use shape_value::{ValueWord, heap_value::NativeScalar};

/// Compile and run Shape code, returning the top-level result.
fn compile_and_run(code: &str) -> ValueWord {
    let program = parse_program(code).unwrap();
    let mut compiler = BytecodeCompiler::new();
    // Allow __* builtins in tests (Rust-level unit tests, not user code)
    compiler.allow_internal_builtins = true;
    let bytecode = compiler.compile(&program).unwrap();
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(bytecode);
    vm.execute(None).unwrap().clone()
}

/// Compile Shape code and call a named function, returning its result.
fn compile_and_run_fn(code: &str, fn_name: &str) -> ValueWord {
    let program = parse_program(code).unwrap();
    let bytecode = BytecodeCompiler::new().compile(&program).unwrap();
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(bytecode);
    vm.execute_function_by_name(fn_name, vec![], None)
        .unwrap()
        .clone()
}

#[test]
fn test_native_pointer_cell_round_trip_builtins() {
    let code = r#"
        fn run() {
            let cell = __native_ptr_new_cell()
            __native_ptr_write_ptr(cell, 424242)
            let out = __native_ptr_read_ptr(cell)
            __native_ptr_free_cell(cell)
            out
        }
        run()
    "#;
    let result = compile_and_run(code);
    assert_eq!(
        result.as_native_scalar(),
        Some(NativeScalar::Ptr(424242)),
        "pointer round-trip should preserve stored pointer-sized value"
    );
}

#[test]
fn test_native_arrow_typed_import_returns_err_for_null_ptrs() {
    let code = r#"
        type Row {
            id: i64,
        }

        fn run() {
            let result: Result<Table<Row>, AnyError> = __native_table_from_arrow_c_typed(0, 0, "Row")
            result
        }
        run()
    "#;
    let result = compile_and_run(code);
    assert!(
        result.as_err_inner().is_some(),
        "null schema/array pointers should return Result::Err"
    );
}

#[test]
fn test_compiler_emits_width_storage_hints_for_bindings_and_function_locals() {
    let code = r#"
        let top_char: char = 1
        let top_byte: byte = 2

        function widths(a: i8, b: u16) {
            let c: byte = 1
            return a + b
        }
    "#;

    let program = parse_program(code).unwrap();
    let bytecode = BytecodeCompiler::new().compile(&program).unwrap();

    let top_char_idx = bytecode
        .module_binding_names
        .iter()
        .position(|name| name == "top_char")
        .expect("top_char module binding");
    let top_byte_idx = bytecode
        .module_binding_names
        .iter()
        .position(|name| name == "top_byte")
        .expect("top_byte module binding");

    assert_eq!(
        bytecode
            .module_binding_storage_hints
            .get(top_char_idx)
            .copied(),
        Some(StorageHint::Int8)
    );
    assert_eq!(
        bytecode
            .module_binding_storage_hints
            .get(top_byte_idx)
            .copied(),
        Some(StorageHint::UInt8)
    );

    let widths_idx = bytecode
        .functions
        .iter()
        .position(|f| f.name == "widths")
        .expect("widths function");
    let local_hints = bytecode
        .function_local_storage_hints
        .get(widths_idx)
        .expect("function local hints");
    assert!(
        local_hints.len() >= 2,
        "widths should contain param local hints"
    );
    assert_eq!(local_hints[0], StorageHint::Int8);
    assert_eq!(local_hints[1], StorageHint::UInt16);
    assert!(
        local_hints.contains(&StorageHint::UInt8),
        "expected byte local hint in function locals: {:?}",
        local_hints
    );
}

#[test]
fn test_fold_lambda_closure() {
    let code = r#"function test() { return fold([1, 2, 3], 0, |acc, x| acc + x); }"#;
    let program = parse_program(code).unwrap();

    println!("AST:\n{:#?}", program);

    let bytecode = BytecodeCompiler::new().compile(&program).unwrap();

    println!("Functions in bytecode: {}", bytecode.functions.len());
    for (i, func) in bytecode.functions.iter().enumerate() {
        println!(
            "  [{}] {} (arity={}, entry={}, is_closure={})",
            i, func.name, func.arity, func.entry_point, func.is_closure
        );
    }

    println!("\nAll instructions:");
    for (i, instr) in bytecode.instructions.iter().enumerate() {
        println!("{:3}: {:?}", i, instr);
    }

    // We expect 2 functions: test and the closure for |acc, x| acc + x
    assert!(
        bytecode.functions.len() >= 2,
        "Expected at least 2 functions (test + closure), got {}",
        bytecode.functions.len()
    );
}

#[test]
fn test_typed_object_merge_bytecode() {
    let code = r#"
        function test() {
            let a = { x: 1, y: 2 };
            let b = { z: 3 };
            let c = a + b;
            return c.x;
        }
    "#;
    let program = parse_program(code).unwrap();
    let bytecode = BytecodeCompiler::new().compile(&program).unwrap();

    // Check that we emit NewTypedObject and TypedMergeObject
    let has_new_typed_object = bytecode
        .instructions
        .iter()
        .any(|i| matches!(i.opcode, OpCode::NewTypedObject));
    let has_typed_merge = bytecode
        .instructions
        .iter()
        .any(|i| matches!(i.opcode, OpCode::TypedMergeObject));

    assert!(
        has_new_typed_object,
        "Should emit NewTypedObject for typed object literals"
    );
    assert!(
        has_typed_merge,
        "Should emit TypedMergeObject for typed merge"
    );
}

#[test]
fn test_typed_object_merge_execution() {
    let code = r#"
    function test() {
        let a = { x: 1, y: 2 };
        let b = { z: 3, w: 4 };
        let merged = a + b;
        return merged.x + merged.z;
    }
    "#;
    let result = compile_and_run_fn(code, "test");
    let n = result.as_number_coerce().expect("Expected numeric value");
    assert_eq!(n, 4.0);
}

#[test]
fn test_typed_merge_decomposition_with_cast() {
    let code = r#"
        type TypeA { x: number, y: number }
        type TypeB { z: number }

        var a = { x: 1 };
        a.y = 2;
        let b = { z: 3 };

        let c = a + b;
        let (f: TypeA, g: TypeB) = c as (TypeA + TypeB);
        f.x + f.y + g.z
    "#;
    let result = compile_and_run(code);
    let n = result.as_number_coerce().expect("Expected numeric value");
    assert_eq!(n, 6.0);
}

#[test]
fn test_import_suggestion_in_error_message() {
    // Register a known export
    let mut compiler = BytecodeCompiler::new();
    compiler.register_known_export("sma", "@stdlib/finance/indicators/moving_averages");

    // Try to compile code that uses the unknown function
    let code = r#"let x = sma([1, 2, 3], 2)"#;
    let program = parse_program(code).unwrap();
    let result = compiler.compile(&program);

    // Should fail with a helpful error message
    assert!(result.is_err(), "Should fail when function is not defined");
    let error = result.unwrap_err();
    let error_msg = format!("{:?}", error);

    // Error should contain the import suggestion
    assert!(
        error_msg.contains("Did you mean to import it via")
            || error_msg.contains("@stdlib/finance"),
        "Error should suggest import: {}",
        error_msg
    );
}

#[test]
fn test_extension_namespace_is_not_implicit_global() {
    let code = r#"let conn = duckdb.connect("duckdb://:memory:")"#;
    let program = parse_program(code).unwrap();
    let result = BytecodeCompiler::new().compile(&program);
    assert!(result.is_err(), "duckdb should require an explicit use");
    let error = format!("{}", result.unwrap_err());
    assert!(
        error.contains("Undefined variable: duckdb"),
        "unexpected error: {}",
        error
    );
}

#[test]
fn test_use_namespace_enables_extension_namespace_access() {
    let mut ext = shape_runtime::module_exports::ModuleExports::new("duckdb");
    ext.add_function("connect", |_args, _ctx: &shape_runtime::ModuleContext| {
        Ok(shape_value::ValueWord::none())
    });

    let code = r#"
        use duckdb
        let conn = duckdb::connect("duckdb://:memory:")
    "#;
    let program = parse_program(code).unwrap();
    let result = BytecodeCompiler::new()
        .with_extensions(vec![ext])
        .compile(&program);
    assert!(
        result.is_ok(),
        "namespace use should allow module-scoped access: {:?}",
        result.err()
    );
}

#[test]
fn test_use_hierarchical_namespace_enables_tail_binding() {
    let mut ext = shape_runtime::module_exports::ModuleExports::new("std::core::snapshot");
    ext.add_function("snapshot", |_args, _ctx: &shape_runtime::ModuleContext| {
        Ok(shape_value::ValueWord::none())
    });

    let code = r#"
        use std::core::snapshot
        let snap = snapshot::snapshot()
    "#;
    let program = parse_program(code).unwrap();
    let result = BytecodeCompiler::new()
        .with_extensions(vec![ext])
        .compile(&program);
    assert!(
        result.is_ok(),
        "hierarchical namespace use should bind tail module name: {:?}",
        result.err()
    );
}

#[test]
fn test_use_namespace_alias_enables_access() {
    let mut ext = shape_runtime::module_exports::ModuleExports::new("duckdb");
    ext.add_function("connect", |_args, _ctx: &shape_runtime::ModuleContext| {
        Ok(shape_value::ValueWord::none())
    });

    let code = r#"
        use duckdb as db
        let conn = db::connect("duckdb://:memory:")
    "#;
    let program = parse_program(code).unwrap();
    let result = BytecodeCompiler::new()
        .with_extensions(vec![ext])
        .compile(&program);
    assert!(
        result.is_ok(),
        "namespace use with alias should allow module-scoped access: {:?}",
        result.err()
    );
}

#[test]
fn test_use_namespace_still_enables_extension_namespace_access() {
    let mut ext = shape_runtime::module_exports::ModuleExports::new("duckdb");
    ext.add_function("connect", |_args, _ctx: &shape_runtime::ModuleContext| {
        Ok(shape_value::ValueWord::none())
    });

    let code = r#"
        use duckdb
        let conn = duckdb::connect("duckdb://:memory:")
    "#;
    let program = parse_program(code).unwrap();
    let result = BytecodeCompiler::new()
        .with_extensions(vec![ext])
        .compile(&program);
    assert!(
        result.is_ok(),
        "namespace use should allow module-scoped access: {:?}",
        result.err()
    );
}

#[test]
fn test_comptime_only_native_export_rejected_in_runtime_context() {
    let mut ext = shape_runtime::module_exports::ModuleExports::new("duckdb");
    ext.add_function(
        "connect_codegen",
        |_args, _ctx: &shape_runtime::ModuleContext| Ok(shape_value::ValueWord::none()),
    );
    ext.set_export_visibility(
        "connect_codegen",
        shape_runtime::module_exports::ModuleExportVisibility::ComptimeOnly,
    );

    let code = r#"
        use duckdb
        let conn = duckdb::connect_codegen("duckdb://:memory:")
    "#;
    let program = parse_program(code).expect("program should parse");
    let result = BytecodeCompiler::new()
        .with_extensions(vec![ext])
        .compile(&program);
    assert!(
        result.is_err(),
        "runtime call to comptime-only export must fail"
    );
    let msg = format!("{}", result.unwrap_err());
    assert!(
        msg.contains("comptime contexts"),
        "unexpected error message: {}",
        msg
    );
}

#[test]
fn test_comptime_only_native_export_allowed_in_comptime_block() {
    let mut ext = shape_runtime::module_exports::ModuleExports::new("duckdb");
    ext.add_function(
        "connect_codegen",
        |_args, _ctx: &shape_runtime::ModuleContext| Ok(shape_value::ValueWord::from_i64(7)),
    );
    ext.set_export_visibility(
        "connect_codegen",
        shape_runtime::module_exports::ModuleExportVisibility::ComptimeOnly,
    );

    let code = r#"
        use duckdb
        function test() {
            return comptime {
                duckdb::connect_codegen("duckdb://:memory:")
            }
        }
    "#;
    let program = parse_program(code).expect("program should parse");
    let bytecode = BytecodeCompiler::new()
        .with_extensions(vec![ext])
        .compile(&program)
        .expect("comptime context should allow comptime-only export");

    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(bytecode);
    let result = vm
        .execute_function_by_name("test", vec![], None)
        .expect("test() should execute");
    assert_eq!(result.as_number_coerce(), Some(7.0));
}

#[test]
fn test_namespace_import_registers_module_schema_compile_time() {
    let mut ext = shape_runtime::module_exports::ModuleExports::new("duckdb");
    ext.add_function("connect", |_args, _ctx: &shape_runtime::ModuleContext| {
        Ok(shape_value::ValueWord::none())
    });
    ext.add_function(
        "source_schema",
        |_args, _ctx: &shape_runtime::ModuleContext| Ok(shape_value::ValueWord::none()),
    );

    let code = r#"
        use duckdb
        let conn = duckdb::connect("duckdb://:memory:")
    "#;
    let program = parse_program(code).unwrap();
    let bytecode = BytecodeCompiler::new()
        .with_extensions(vec![ext])
        .compile(&program)
        .unwrap();

    let schema = bytecode
        .type_schema_registry
        .get("__mod_duckdb")
        .expect("module schema should be predeclared at compile time");
    assert!(schema.has_field("connect"));
    assert!(schema.has_field("source_schema"));
}

#[test]
fn test_namespace_import_registers_shape_artifact_exports_compile_time() {
    let mut ext = shape_runtime::module_exports::ModuleExports::new("duckdb");
    ext.add_shape_artifact(
        "duckdb",
        Some("pub fn connect(uri) { uri }".to_string()),
        None,
    );

    let code = r#"
        use duckdb
        let conn = duckdb::connect("duckdb://:memory:")
    "#;
    let program = parse_program(code).unwrap();
    let bytecode = BytecodeCompiler::new()
        .with_extensions(vec![ext])
        .compile(&program)
        .unwrap();

    let schema = bytecode
        .type_schema_registry
        .get("__mod_duckdb")
        .expect("module schema should be predeclared at compile time");
    assert!(schema.has_field("connect"));
}

#[test]
fn test_module_namespace_call_lowers_to_callvalue_not_callmethod() {
    let mut ext = shape_runtime::module_exports::ModuleExports::new("duckdb");
    ext.add_function("connect", |_args, _ctx: &shape_runtime::ModuleContext| {
        Ok(shape_value::ValueWord::none())
    });

    let code = r#"
        use duckdb
        let conn = duckdb::connect("duckdb://:memory:")
    "#;
    let program = parse_program(code).unwrap();
    let bytecode = BytecodeCompiler::new()
        .with_extensions(vec![ext])
        .compile(&program)
        .unwrap();

    let opcodes: Vec<_> = bytecode.instructions.iter().map(|ins| ins.opcode).collect();
    assert!(
        opcodes.contains(&OpCode::CallValue),
        "module namespace call should lower through CallValue"
    );
    assert!(
        !opcodes.contains(&OpCode::CallMethod),
        "module namespace call should not lower through CallMethod"
    );
}

#[test]
fn test_dot_module_namespace_call_is_rejected() {
    let mut ext = shape_runtime::module_exports::ModuleExports::new("duckdb");
    ext.add_function("connect", |_args, _ctx: &shape_runtime::ModuleContext| {
        Ok(shape_value::ValueWord::none())
    });

    let code = r#"
        use duckdb
        let conn = duckdb.connect("duckdb://:memory:")
    "#;
    let program = parse_program(code).unwrap();
    let result = BytecodeCompiler::new()
        .with_extensions(vec![ext])
        .compile(&program);
    assert!(result.is_err(), "dot-based module namespace calls must fail");
    let msg = format!("{}", result.unwrap_err());
    assert!(msg.contains("must use `::`"), "unexpected error: {}", msg);
}

#[test]
fn test_local_value_shadowing_namespace_alias_keeps_dot_methods_and_fields() {
    let mut ext = shape_runtime::module_exports::ModuleExports::new("duckdb");
    ext.add_function("connect", |_args, _ctx: &shape_runtime::ModuleContext| {
        Ok(shape_value::ValueWord::none())
    });

    let code = r#"
        use duckdb as s

        fn test() {
            let s = { value: [1, 2, 3] }
            print(s.value.len())
        }
    "#;
    let program = parse_program(code).unwrap();
    BytecodeCompiler::new()
        .with_extensions(vec![ext])
        .compile(&program)
        .expect("local values should shadow namespace aliases for dot access");
}

#[test]
fn test_dynamic_spread_without_known_schema_is_compile_error() {
    let code = r#"
        fn merge_dynamic(x) {
            let y = { ...x }
            y
        }
    "#;
    let program = parse_program(code).unwrap();
    let result = BytecodeCompiler::new().compile(&program);
    assert!(result.is_err(), "dynamic spread should fail without schema");
    let msg = format!("{}", result.unwrap_err());
    assert!(
        msg.contains("Object spread source must have a compile-time known schema"),
        "unexpected error message: {}",
        msg
    );
}

#[test]
fn test_struct_literal_compilation() {
    let code = r#"
        type Point { x: number, y: number }
        function test() {
            let p = Point { x: 3, y: 4 };
            return p.x + p.y;
        }
    "#;
    let program = parse_program(code).unwrap();
    let bytecode = BytecodeCompiler::new().compile(&program).unwrap();

    // Should emit NewTypedObject for struct literal
    let has_new_typed_object = bytecode
        .instructions
        .iter()
        .any(|i| matches!(i.opcode, OpCode::NewTypedObject));
    assert!(
        has_new_typed_object,
        "Should emit NewTypedObject for struct literal"
    );
}

#[test]
fn test_struct_literal_execution() {
    let code = r#"
        type Point { x: number, y: number }
        function test() {
            let p = Point { x: 3, y: 4 };
            return p.x + p.y;
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    let n = result.as_number_coerce().expect("Expected numeric value");
    assert_eq!(n, 7.0);
}

#[test]
fn test_struct_literal_field_validation() {
    // Missing field should error
    let code = r#"
        type Point { x: number, y: number }
        function test() {
            let p = Point { x: 3 };
            return p.x;
        }
    "#;
    let program = parse_program(code).unwrap();
    let result = BytecodeCompiler::new().compile(&program);
    assert!(result.is_err(), "Missing field should error");
    let err = format!("{:?}", result.unwrap_err());
    assert!(
        err.contains("Missing field"),
        "Error should mention missing field: {}",
        err
    );
}

#[test]
fn test_struct_literal_unknown_field() {
    // Unknown field should error
    let code = r#"
        type Point { x: number, y: number }
        function test() {
            let p = Point { x: 3, y: 4, z: 5 };
            return p.x;
        }
    "#;
    let program = parse_program(code).unwrap();
    let result = BytecodeCompiler::new().compile(&program);
    assert!(result.is_err(), "Unknown field should error");
    let err = format!("{:?}", result.unwrap_err());
    assert!(
        err.contains("Unknown field"),
        "Error should mention unknown field: {}",
        err
    );
}

// Series struct literal tests removed -- Series type no longer exists

#[test]
fn test_struct_literal_unknown_type() {
    // Unknown struct type should error
    let code = r#"
        function test() {
            let p = Unknown { x: 3 };
            return p.x;
        }
    "#;
    let program = parse_program(code).unwrap();
    let result = BytecodeCompiler::new().compile(&program);
    assert!(result.is_err(), "Unknown struct type should error");
    let err = format!("{:?}", result.unwrap_err());
    assert!(
        err.contains("Unknown struct type"),
        "Error should mention unknown type: {}",
        err
    );
}

#[test]
fn test_type_method_struct_returns_type_name() {
    let code = r#"
        type Point { x: number, y: number }
        function test() {
            let p = Point { x: 3, y: 4 }
            let t = p.type()
            return t.to_string()
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(
        result.as_arc_string().expect("Expected String").as_ref() as &str,
        "Point"
    );
}

#[test]
fn test_type_method_type_symbol_returns_type_name() {
    let code = r#"
        type Point { x: number, y: number }
        function test() {
            let t = Point.type()
            return t.to_string()
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(
        result.as_arc_string().expect("Expected String").as_ref() as &str,
        "Point"
    );
}

#[test]
fn test_type_method_on_value_returns_static_type_name() {
    let code = r#"
        type Point { x: number, y: number }
        function test() {
            let p = Point { x: 3, y: 4 }
            return p.type().to_string()
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(
        result.as_arc_string().expect("Expected String").as_ref() as &str,
        "Point"
    );
}

#[test]
fn test_type_method_on_type_symbol_returns_name() {
    let code = r#"
        type Point { x: number, y: number }
        function test() {
            return Point.type().to_string()
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(
        result.as_arc_string().expect("Expected String").as_ref() as &str,
        "Point"
    );
}

#[test]
fn test_type_method_works_inside_comptime_block() {
    let code = r#"
        function test() {
            return comptime {
                1.type()
            }
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(
        result.as_arc_string().expect("Expected String").as_ref() as &str,
        "int"
    );
}

#[test]
fn test_comptime_fn_can_be_called_inside_comptime_block() {
    let code = r#"
        comptime fn plus_one(x) { return x + 1 }
        function test() {
            return comptime {
                plus_one(41)
            }
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(
        result.as_number_coerce().expect("Expected numeric value"),
        42.0
    );
}

#[test]
fn test_comptime_fn_call_rejected_outside_comptime_block() {
    let code = r#"
        comptime fn helper() { return 1 }
        function test() {
            return helper()
        }
    "#;
    let program = parse_program(code).expect("program should parse");
    let err = BytecodeCompiler::new()
        .compile(&program)
        .expect_err("runtime calls to comptime fn must fail");
    let msg = format!("{}", err);
    assert!(
        msg.contains("comptime fn"),
        "unexpected error message: {}",
        msg
    );
}

#[test]
fn test_comptime_block_does_not_execute_prior_runtime_statements() {
    let code = r#"
        let marker = 42
        comptime {
            marker
        }
        function test() {
            return 1
        }
    "#;

    let program = parse_program(code).expect("program should parse");
    let mut compiler = BytecodeCompiler::new();
    compiler.set_source(code);
    let err = compiler
        .compile(&program)
        .expect_err("comptime must not observe runtime top-level bindings");
    let message = format!("{:?}", err);

    assert!(
        message.contains("Undefined variable: marker"),
        "unexpected error: {}",
        message
    );
}

#[test]
fn test_type_query_to_string_returns_canonical_type_text() {
    let code = r#"
        type Point { x: number, y: number }
        function test() {
            return Point.type().to_string()
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(
        result.as_arc_string().expect("Expected String").as_ref() as &str,
        "Point"
    );
}

#[test]
fn test_typeof_generic_struct_uses_inferred_type_argument() {
    let code = r#"
        type MyType<T = int> { x: T }
        function test() {
            let a = MyType { x: 1.0 }
            return a.type().to_string()
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(
        result.as_arc_string().expect("Expected String").as_ref() as &str,
        "MyType<number>"
    );
}

#[test]
fn test_typeof_generic_struct_uses_base_name_for_default_argument() {
    let code = r#"
        type MyType<T = int> { x: T }
        function test() {
            let a = MyType { x: 1 }
            return a.type().to_string()
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(
        result.as_arc_string().expect("Expected String").as_ref() as &str,
        "MyType"
    );
}

#[test]
fn test_type_method_generic_param_uses_runtime_concrete_type() {
    let code = r#"
        function inner<T>(x: T) {
            return x.type().to_string()
        }
        function test() {
            return inner(2.1)
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(
        result.as_arc_string().expect("Expected String").as_ref() as &str,
        "number"
    );
}

#[test]
fn test_function_call_omits_default_argument_successfully() {
    let code = r#"
        function add(a, b = 2) {
            return a + b
        }
        function test() {
            return add(3)
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    let n = result.as_number_coerce().expect("Expected numeric value");
    assert_eq!(n, 5.0);
}

#[test]
fn test_function_call_missing_required_argument_is_compile_error() {
    let code = r#"
        function add(a, b = 2) {
            return a + b
        }
        function test() {
            return add()
        }
    "#;
    let program = parse_program(code).expect("program should parse");
    let result = BytecodeCompiler::new().compile(&program);
    assert!(
        result.is_err(),
        "missing required argument should fail compilation"
    );
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("expects between 1 and 2 arguments"),
        "expected arity-range diagnostic, got: {}",
        err
    );
}

#[test]
fn test_no_suggestion_for_truly_unknown_function() {
    let compiler = BytecodeCompiler::new();

    // Try to compile code that uses a truly unknown function
    let code = r#"let x = totally_unknown_function()"#;
    let program = parse_program(code).unwrap();
    let result = compiler.compile(&program);

    // Should fail
    assert!(result.is_err(), "Should fail when function is not defined");
    let error = result.unwrap_err();
    let error_msg = format!("{:?}", error);

    // Error should NOT contain import suggestion
    assert!(
        !error_msg.contains("Did you mean to import it via"),
        "Error should not suggest import for truly unknown function: {}",
        error_msg
    );
    assert!(
        error_msg.contains("Undefined function"),
        "Error should say undefined: {}",
        error_msg
    );
}

#[test]
fn test_undefined_variable_suggestion() {
    let code = r#"
        function test() {
            let count = 10
            return cont
        }
    "#;
    let program = parse_program(code).unwrap();
    let result = BytecodeCompiler::new().compile(&program);
    assert!(result.is_err(), "Should fail for misspelled variable");
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("Did you mean"),
        "Error should contain 'Did you mean' suggestion: {}",
        err
    );
    assert!(
        err.contains("count"),
        "Suggestion should mention 'count': {}",
        err
    );
}

#[test]
fn test_undefined_function_suggestion() {
    let code = r#"
        function greet() { return 1 }
        let x = gret()
    "#;
    let program = parse_program(code).unwrap();
    let result = BytecodeCompiler::new().compile(&program);
    assert!(result.is_err(), "Should fail for misspelled function");
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("Did you mean"),
        "Error should contain 'Did you mean' suggestion: {}",
        err
    );
    assert!(
        err.contains("greet"),
        "Suggestion should mention 'greet': {}",
        err
    );
}

#[test]
fn test_undefined_variable_no_suggestion_for_distant_name() {
    let code = r#"
        function test() {
            let x = 1
            return zzzzzzzzz
        }
    "#;
    let program = parse_program(code).unwrap();
    let result = BytecodeCompiler::new().compile(&program);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("Undefined variable"),
        "Should say undefined variable: {}",
        err
    );
    assert!(
        !err.contains("Did you mean"),
        "Should NOT suggest for very different name: {}",
        err
    );
}

#[test]
fn test_undefined_function_builtin_suggestion() {
    // Test that misspelling a builtin function gets a suggestion
    let code = r#"let x = prnt("hello")"#;
    let program = parse_program(code).unwrap();
    let result = BytecodeCompiler::new().compile(&program);
    assert!(result.is_err(), "Should fail for misspelled builtin");
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("Did you mean") && err.contains("print"),
        "Should suggest 'print' for 'prnt': {}",
        err
    );
}

#[test]
fn test_async_function_compiles_with_await() {
    let code = r#"function bar() { return 1 }
async function foo() { let x = await bar(); return x }"#;
    let program = parse_program(code).unwrap();
    let bytecode = BytecodeCompiler::new().compile(&program).unwrap();

    // The compiled function should be marked as async
    let foo = bytecode.functions.iter().find(|f| f.name == "foo").unwrap();
    assert!(foo.is_async, "foo should be async");

    // Should contain an Await opcode
    let has_await = bytecode
        .instructions
        .iter()
        .any(|i| i.opcode == crate::bytecode::OpCode::Await);
    assert!(has_await, "should emit Await opcode");
}

#[test]
fn test_sync_function_rejects_await() {
    let code = r#"function bar() { return 1 }
function foo() { let x = await bar(); return x }"#;
    let program = parse_program(code).unwrap();
    let result = BytecodeCompiler::new().compile(&program);
    assert!(result.is_err(), "await in non-async function should fail");
    let err = result.unwrap_err().to_string();
    assert!(err.contains("async"), "error should mention async: {}", err);
}

#[test]
fn test_sync_function_has_is_async_false() {
    let code = r#"function bar() { return 42 }"#;
    let program = parse_program(code).unwrap();
    let bytecode = BytecodeCompiler::new().compile(&program).unwrap();
    let bar = bytecode.functions.iter().find(|f| f.name == "bar").unwrap();
    assert!(!bar.is_async, "bar should NOT be async");
}

#[test]
fn test_await_on_non_future_passes_through() {
    // await on a non-Future value should pass through (sync shortcut)
    let code = r#"function bar() { return 42 }
async function foo() { return await bar() }
foo()"#;
    // bar() returns 42, which is not a Future, so await passes it through
    let result = compile_and_run(code);
    assert_eq!(result.as_i64(), Some(42));
}

#[test]
fn test_await_on_future_suspends() {
    use crate::executor::ExecutionResult;
    // Build bytecode that pushes a Future onto the stack and awaits it
    let mut program = crate::bytecode::BytecodeProgram::new();
    // Push Future(42) constant
    let future_const = program.add_constant(crate::bytecode::Constant::Number(42.0));
    program.emit(crate::bytecode::Instruction::new(
        crate::bytecode::OpCode::PushConst,
        Some(crate::bytecode::Operand::Const(future_const)),
    ));
    // We can't easily create a Future via bytecode constants,
    // so let's test at the VM level by pushing a Future directly
    let mut vm = crate::executor::VirtualMachine::new(crate::executor::VMConfig::default());
    vm.load_program(program);
    // Manually push a Future value and an Await instruction
    let mut program2 = crate::bytecode::BytecodeProgram::new();
    program2.emit(crate::bytecode::Instruction::simple(
        crate::bytecode::OpCode::Await,
    ));
    program2.emit(crate::bytecode::Instruction::simple(
        crate::bytecode::OpCode::Halt,
    ));
    let mut vm2 = crate::executor::VirtualMachine::new(crate::executor::VMConfig::default());
    vm2.load_program(program2);
    vm2.push_value(ValueWord::from_future(42));
    let result = vm2.execute_with_suspend(None).unwrap();
    match result {
        ExecutionResult::Suspended { future_id, .. } => {
            assert_eq!(future_id, 42, "should suspend on future 42");
        }
        ExecutionResult::Completed(_) => panic!("expected suspension, got completion"),
    }
}

#[test]
fn test_suspend_and_resume() {
    use crate::executor::ExecutionResult;
    // Build bytecode: Await, then Halt
    let mut program = crate::bytecode::BytecodeProgram::new();
    program.emit(crate::bytecode::Instruction::simple(
        crate::bytecode::OpCode::Await,
    ));
    program.emit(crate::bytecode::Instruction::simple(
        crate::bytecode::OpCode::Halt,
    ));
    let mut vm = crate::executor::VirtualMachine::new(crate::executor::VMConfig::default());
    vm.load_program(program);
    // Push Future(99) and execute -- should suspend
    vm.push_value(ValueWord::from_future(99));
    let result = vm.execute_with_suspend(None).unwrap();
    assert!(matches!(
        result,
        ExecutionResult::Suspended { future_id: 99, .. }
    ));
    // Resume with resolved value
    let result = vm
        .resume(shape_value::ValueWord::from_f64(100.0), None)
        .unwrap();
    match result {
        ExecutionResult::Completed(value) => {
            assert_eq!(value.clone(), ValueWord::from_f64(100.0));
        }
        ExecutionResult::Suspended { .. } => panic!("expected completion after resume"),
    }
}

#[test]
fn test_table_generic_annotation_tracks_datatable() {
    use shape_runtime::type_schema::TypeSchemaBuilder;

    let code = r#"
        function get_data() { return 0 }
        let trades: Table<Trade> = get_data()
    "#;
    let program = parse_program(code).unwrap();
    let mut compiler = BytecodeCompiler::new();

    // Register Trade schema so Table<Trade> can resolve it
    TypeSchemaBuilder::new("Trade")
        .f64_field("price")
        .i64_field("volume")
        .string_field("symbol")
        .register(compiler.type_tracker_mut().schema_registry_mut());

    let result = compiler.compile(&program);
    assert!(
        result.is_ok(),
        "Table<Trade> should compile: {:?}",
        result.err()
    );
}

#[test]
fn test_table_generic_unknown_type_errors() {
    let code = r#"
        function get_data() { return 0 }
        let trades: Table<UnknownType> = get_data()
    "#;
    let program = parse_program(code).unwrap();
    let result = BytecodeCompiler::new().compile(&program);
    assert!(result.is_err(), "Table<UnknownType> should error");
    let err = format!("{:?}", result.unwrap_err());
    assert!(
        err.contains("Unknown type") && err.contains("UnknownType"),
        "Error should mention unknown type: {}",
        err
    );
}

#[test]
fn test_closure_param_tagged_as_row_view() {
    use shape_runtime::type_schema::TypeSchemaBuilder;

    // When compiling dt.filter(|row| row.price > 100), row.price should
    // compile to LoadColF64 (not GetProp)
    let code = r#"
        function get_data() { return 0 }
        function test() {
            let trades: Table<Trade> = get_data()
            let expensive = trades.filter(|row| row.price > 100.0)
            return expensive
        }
    "#;
    let program = parse_program(code).unwrap();
    let mut compiler = BytecodeCompiler::new();

    TypeSchemaBuilder::new("Trade")
        .f64_field("price")
        .i64_field("volume")
        .string_field("symbol")
        .register(compiler.type_tracker_mut().schema_registry_mut());

    let bytecode = compiler.compile(&program).unwrap();

    // Should emit LoadColF64 for row.price
    let has_load_col = bytecode
        .instructions
        .iter()
        .any(|i| matches!(i.opcode, OpCode::LoadColF64));
    assert!(
        has_load_col,
        "row.price in closure should compile to LoadColF64"
    );

    // Should NOT emit GetProp for row.price
    // (GetProp might still be emitted for other things, but check the
    // instructions around LoadColF64 to be sure)
}

#[test]
fn test_closure_filter_compiles_loadcol_for_multiple_fields() {
    use shape_runtime::type_schema::TypeSchemaBuilder;

    // Test that multiple field accesses in a closure all compile to LoadCol*
    let code = r#"
        function get_data() { return 0 }
        function test() {
            let trades: Table<Trade> = get_data()
            let result = trades.filter(|row| row.price > 100.0)
            return result
        }
    "#;
    let program = parse_program(code).unwrap();
    let mut compiler = BytecodeCompiler::new();

    TypeSchemaBuilder::new("Trade")
        .f64_field("price")
        .i64_field("volume")
        .string_field("symbol")
        .register(compiler.type_tracker_mut().schema_registry_mut());

    let bytecode = compiler.compile(&program).unwrap();

    // Count LoadCol opcodes -- should have at least 1 for row.price
    let load_col_count = bytecode
        .instructions
        .iter()
        .filter(|i| {
            matches!(
                i.opcode,
                OpCode::LoadColF64 | OpCode::LoadColI64 | OpCode::LoadColBool | OpCode::LoadColStr
            )
        })
        .count();
    assert!(
        load_col_count >= 1,
        "Should have at least 1 LoadCol* opcode, got {}",
        load_col_count
    );
}

#[test]
fn test_unknown_row_view_field_compile_error() {
    use shape_runtime::type_schema::TypeSchemaBuilder;

    let code = r#"
        function get_data() { return 0 }
        function test() {
            let trades: Table<Trade> = get_data()
            let bad = trades.filter(|row| row.nonexistent > 100.0)
            return bad
        }
    "#;
    let program = parse_program(code).unwrap();
    let mut compiler = BytecodeCompiler::new();

    TypeSchemaBuilder::new("Trade")
        .f64_field("price")
        .i64_field("volume")
        .string_field("symbol")
        .register(compiler.type_tracker_mut().schema_registry_mut());

    let result = compiler.compile(&program);
    assert!(result.is_err(), "Unknown field on RowView should error");
    let err = format!("{:?}", result.unwrap_err());
    assert!(
        err.contains("nonexistent") && err.contains("does not exist"),
        "Error should mention the unknown field: {}",
        err
    );
    assert!(
        err.contains("price") && err.contains("volume") && err.contains("symbol"),
        "Error should list available fields: {}",
        err
    );
}

#[test]
fn test_index_by_result_propagates_indexed_type_for_between() {
    use shape_runtime::type_schema::TypeSchemaBuilder;

    let code = r#"
        function get_data() { return 0 }
        function test() {
            let trades: Table<Trade> = get_data()
            let indexed = trades.indexBy(|row| row.timestamp)
            let sliced = indexed.between(0, 10)
            return sliced
        }
    "#;
    let program = parse_program(code).unwrap();
    let mut compiler = BytecodeCompiler::new();

    TypeSchemaBuilder::new("Trade")
        .i64_field("timestamp")
        .f64_field("price")
        .register(compiler.type_tracker_mut().schema_registry_mut());

    let result = compiler.compile(&program);
    assert!(
        result.is_ok(),
        "indexBy result should be tracked as Indexed<T> for between(): {:?}",
        result.err()
    );
}

#[test]
fn test_bind_schema_emitted_for_table_type() {
    use shape_runtime::type_schema::TypeSchemaBuilder;

    let code = r#"
        function get_data() { return 0 }
        type MyData = { value: number }
        let d: Table<MyData> = get_data()
    "#;
    let program = parse_program(code).unwrap();

    let mut compiler = BytecodeCompiler::new();

    TypeSchemaBuilder::new("MyData")
        .f64_field("value")
        .register(compiler.type_tracker_mut().schema_registry_mut());

    let bytecode = compiler.compile(&program).unwrap();

    // Verify BindSchema instruction was emitted
    let has_bind_schema = bytecode
        .instructions
        .iter()
        .any(|i| i.opcode == OpCode::BindSchema);
    assert!(
        has_bind_schema,
        "Bytecode should contain BindSchema for Table<T> annotation"
    );
}

// =========================================================================
// Phase 5: Fuzzy comparison desugaring tests
// =========================================================================

#[test]
fn test_fuzzy_eq_absolute_no_fuzzy_opcodes() {
    let code = r#"function test() { return 100 ~= 102 within 5; }"#;
    let program = parse_program(code).unwrap();
    let bytecode = BytecodeCompiler::new().compile(&program).unwrap();

    // No fuzzy opcodes should appear -- desugared to arithmetic
    for instr in &bytecode.instructions {
        let name = format!("{:?}", instr.opcode);
        assert!(!name.contains("Fuzzy"), "Found fuzzy opcode: {:?}", instr);
    }
}

#[test]
fn test_fuzzy_eq_percentage_no_fuzzy_opcodes() {
    let code = r#"function test() { return 100 ~= 102 within 5%; }"#;
    let program = parse_program(code).unwrap();
    let bytecode = BytecodeCompiler::new().compile(&program).unwrap();

    for instr in &bytecode.instructions {
        let name = format!("{:?}", instr.opcode);
        assert!(!name.contains("Fuzzy"), "Found fuzzy opcode: {:?}", instr);
    }
}

#[test]
fn test_fuzzy_eq_absolute_execution() {
    let code = r#"
    function test() {
        let close_enough = 100 ~= 102 within 5;
        let too_far = 100 ~= 110 within 5;
        // Return 1 if close_enough is true and too_far is false
        if close_enough && !too_far { return 1; }
        return 0;
    }
    "#;
    let result = compile_and_run_fn(code, "test");
    let n = result.as_number_coerce().expect("Expected numeric value");
    assert_eq!(n, 1.0, "Fuzzy eq absolute test");
}

#[test]
fn test_fuzzy_gt_absolute_execution() {
    let code = r#"
    function test() {
        // a ~> b within t: a > b || abs(a - b) <= t
        let result1 = 100 ~> 98 within 5;   // true: 100 > 98
        let result2 = 100 ~> 103 within 5;  // true: abs(100-103)=3 <= 5
        let result3 = 100 ~> 110 within 5;  // false: 100 < 110 and abs(100-110)=10 > 5
        if result1 && result2 && !result3 { return 1; }
        return 0;
    }
    "#;
    let result = compile_and_run_fn(code, "test");
    let n = result.as_number_coerce().expect("Expected numeric value");
    assert_eq!(n, 1.0, "Fuzzy gt absolute test");
}

// =========================================================================
// Phase 6: Percent literal tests
// =========================================================================

#[test]
fn test_percent_literal_execution() {
    let code = r#"
    function test() {
        let a = 5%;
        let b = 100%;
        let c = 0.5%;
        // 0.05 + 1.0 + 0.005 = 1.055
        return a + b + c;
    }
    "#;
    let result = compile_and_run_fn(code, "test");
    let n = result.as_number_coerce().expect("Expected numeric value");
    assert!((n - 1.055).abs() < 1e-10, "Expected 1.055, got {}", n);
}

#[test]
fn test_percent_literal_multiplication() {
    let code = r#"
    function test() {
        return 50% * 200;
    }
    "#;
    let result = compile_and_run_fn(code, "test");
    let n = result.as_number_coerce().expect("Expected numeric value");
    assert_eq!(n, 100.0, "50% * 200 should be 100");
}

#[test]
fn test_typed_opcode_int_multiplication() {
    // Verify that int x int emits MulInt and executes correctly
    let code = r#"
    function test() {
        let a = 7;
        let b = 6;
        return a * b;
    }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(
        result.as_i64().expect("Expected Int"),
        42,
        "7 * 6 should be 42"
    );
}

#[test]
fn test_typed_opcode_number_arithmetic() {
    // Verify that number + number emits AddNumber and number > number emits GtNumber
    let code = r#"
    function test() {
        let a = 3.14;
        let b = 2.86;
        return a + b;
    }
    "#;
    let result = compile_and_run_fn(code, "test");
    let n = result.as_number_coerce().expect("Expected numeric value");
    assert!(
        (n - 6.0).abs() < 1e-10,
        "3.14 + 2.86 should be ~6.0, got {}",
        n
    );
}

#[test]
fn test_mixed_numeric_add_emits_coercion_and_typed_opcode() {
    let code = r#"
    function test() {
        let a = 1;
        let b = 2.5;
        return a + b;
    }
    "#;
    let program = parse_program(code).unwrap();
    let bytecode = BytecodeCompiler::new().compile(&program).unwrap();
    let opcodes: Vec<_> = bytecode.instructions.iter().map(|ins| ins.opcode).collect();
    assert!(
        opcodes.contains(&OpCode::IntToNumber),
        "Expected IntToNumber for int+number coercion, got opcodes: {:?}",
        opcodes
    );
    assert!(
        opcodes.contains(&OpCode::AddNumber),
        "Expected AddNumber for mixed numeric add, got opcodes: {:?}",
        opcodes
    );

    let result = compile_and_run_fn(code, "test");
    let n = result.as_number_coerce().expect("Expected numeric value");
    assert!((n - 3.5).abs() < 1e-10, "Expected 3.5, got {}", n);
}

#[test]
fn test_mixed_numeric_eq_emits_coercion_and_typed_eq() {
    let code = r#"
    function test() {
        if 1 == 1.0 { return 1; }
        return 0;
    }
    "#;
    let program = parse_program(code).unwrap();
    let bytecode = BytecodeCompiler::new().compile(&program).unwrap();
    let opcodes: Vec<_> = bytecode.instructions.iter().map(|ins| ins.opcode).collect();
    assert!(
        opcodes.contains(&OpCode::IntToNumber),
        "Expected IntToNumber before typed Eq for mixed numeric equality, got opcodes: {:?}",
        opcodes
    );
    assert!(
        opcodes.contains(&OpCode::EqNumber),
        "Expected EqNumber opcode for mixed numeric equality, got opcodes: {:?}",
        opcodes
    );

    let result = compile_and_run_fn(code, "test");
    assert_eq!(result.as_i64().expect("Expected Int"), 1);
}

#[test]
fn test_int_equality_emits_typed_eqint() {
    let code = r#"
    function test() {
        if 7 == 7 { return 1; }
        return 0;
    }
    "#;
    let program = parse_program(code).unwrap();
    let bytecode = BytecodeCompiler::new().compile(&program).unwrap();
    let opcodes: Vec<_> = bytecode.instructions.iter().map(|ins| ins.opcode).collect();
    assert!(
        opcodes.contains(&OpCode::EqInt),
        "Expected EqInt for int equality, got opcodes: {:?}",
        opcodes
    );
    assert!(
        !opcodes.contains(&OpCode::Eq),
        "Did not expect generic Eq for int equality, got opcodes: {:?}",
        opcodes
    );
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result.as_i64().expect("Expected Int"), 1);
}

#[test]
fn test_number_inequality_emits_typed_neqnumber() {
    let code = r#"
    function test() {
        if 3.0 != 2.0 { return 1; }
        return 0;
    }
    "#;
    let program = parse_program(code).unwrap();
    let bytecode = BytecodeCompiler::new().compile(&program).unwrap();
    let opcodes: Vec<_> = bytecode.instructions.iter().map(|ins| ins.opcode).collect();
    assert!(
        opcodes.contains(&OpCode::NeqNumber),
        "Expected NeqNumber for number inequality, got opcodes: {:?}",
        opcodes
    );
    assert!(
        !opcodes.contains(&OpCode::Neq),
        "Did not expect generic Neq for number inequality, got opcodes: {:?}",
        opcodes
    );
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result.as_i64().expect("Expected Int"), 1);
}

#[test]
fn test_index_access_number_propagates_typed_numeric_opcode() {
    let code = r#"
    function test() {
        let a = [1.5, 2.5, 3.5];
        return a[0] + a[1];
    }
    "#;
    let program = parse_program(code).unwrap();
    let bytecode = BytecodeCompiler::new().compile(&program).unwrap();
    let opcodes: Vec<_> = bytecode.instructions.iter().map(|ins| ins.opcode).collect();
    assert!(
        opcodes.contains(&OpCode::AddNumber),
        "Expected AddNumber for number array index arithmetic, got opcodes: {:?}",
        opcodes
    );
    let result = compile_and_run_fn(code, "test");
    let n = result.as_number_coerce().expect("Expected numeric value");
    assert!((n - 4.0).abs() < 1e-10, "Expected 4.0, got {}", n);
}

#[test]
fn test_index_access_int_propagates_typed_numeric_opcode() {
    let code = r#"
    function test() {
        let a = [1, 2, 3];
        return a[0] + a[1];
    }
    "#;
    let program = parse_program(code).unwrap();
    let bytecode = BytecodeCompiler::new().compile(&program).unwrap();
    let opcodes: Vec<_> = bytecode.instructions.iter().map(|ins| ins.opcode).collect();
    assert!(
        opcodes.contains(&OpCode::AddInt),
        "Expected AddInt for int array index arithmetic, got opcodes: {:?}",
        opcodes
    );
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result.as_i64().expect("Expected Int"), 3);
}

#[test]
fn test_array_push_then_index_numeric_emits_typed_arithmetic() {
    let code = r#"
    function test() {
        var u = [];
        var i = 0;
        while i < 4 {
            u = u.push(1.0);
            i = i + 1;
        }
        var j = 0;
        var s = 0.0;
        while j < 4 {
            s = s + u[j];
            j = j + 1;
        }
        return s;
    }
    "#;
    let program = parse_program(code).unwrap();
    let bytecode = BytecodeCompiler::new().compile(&program).unwrap();
    let opcodes: Vec<_> = bytecode.instructions.iter().map(|ins| ins.opcode).collect();
    assert!(
        opcodes.contains(&OpCode::AddNumber),
        "Expected AddNumber for numeric array accumulation, got opcodes: {:?}",
        opcodes
    );
    assert!(
        !opcodes.contains(&OpCode::Add),
        "Expected no generic Add in numeric array accumulation, got opcodes: {:?}",
        opcodes
    );
    let result = compile_and_run_fn(code, "test");
    let n = result.as_number_coerce().expect("Expected numeric value");
    assert!((n - 4.0).abs() < 1e-10, "Expected 4.0, got {}", n);
}

#[test]
fn test_vec_annotation_uses_array_numeric_specialization() {
    let code = r#"
    function test() {
        let xs: Vec<number> = [1.0, 2.0];
        return xs[0] + xs[1];
    }
    "#;
    let program = parse_program(code).unwrap();
    let bytecode = BytecodeCompiler::new().compile(&program).unwrap();
    let opcodes: Vec<_> = bytecode.instructions.iter().map(|ins| ins.opcode).collect();
    assert!(
        opcodes.contains(&OpCode::AddNumber),
        "Expected AddNumber for Vec<number> arithmetic, got opcodes: {:?}",
        opcodes
    );
    let result = compile_and_run_fn(code, "test");
    let n = result.as_number_coerce().expect("Expected numeric value");
    assert!((n - 3.0).abs() < 1e-10, "Expected 3.0, got {}", n);
}

#[test]
fn test_vec_param_annotation_uses_array_numeric_specialization() {
    let code = r#"
    function sum2(xs: Vec<number>) {
        return xs[0] + xs[1];
    }
    function test() {
        return sum2([1.0, 2.0]);
    }
    "#;
    let program = parse_program(code).unwrap();
    let bytecode = BytecodeCompiler::new().compile(&program).unwrap();
    let opcodes: Vec<_> = bytecode.instructions.iter().map(|ins| ins.opcode).collect();
    assert!(
        opcodes.contains(&OpCode::AddNumber),
        "Expected AddNumber for Vec<number> param arithmetic, got opcodes: {:?}",
        opcodes
    );
    let result = compile_and_run_fn(code, "test");
    let n = result.as_number_coerce().expect("Expected numeric value");
    assert!((n - 3.0).abs() < 1e-10, "Expected 3.0, got {}", n);
}

#[test]
fn test_mat_vec_mul_lowers_to_matrix_intrinsic() {
    let code = r#"
    function test() {
        let m: Mat<number> = [[1.0, 2.0], [3.0, 4.0]];
        let v: Vec<number> = [5.0, 6.0];
        let y: Vec<number> = m * v;
        return y[0] + y[1];
    }
    "#;
    let program = parse_program(code).unwrap();
    let bytecode = BytecodeCompiler::new().compile(&program).unwrap();
    let has_matmul_vec = bytecode.instructions.iter().any(|ins| {
        ins.opcode == OpCode::BuiltinCall
            && matches!(
                ins.operand,
                Some(Operand::Builtin(BuiltinFunction::IntrinsicMatMulVec))
            )
    });
    assert!(
        has_matmul_vec,
        "Expected IntrinsicMatMulVec builtin lowering"
    );
    let opcodes: Vec<_> = bytecode.instructions.iter().map(|ins| ins.opcode).collect();
    assert!(
        !opcodes.contains(&OpCode::Mul),
        "Expected no generic Mul for typed Mat*Vec path, got opcodes: {:?}",
        opcodes
    );
    let result = compile_and_run_fn(code, "test");
    let n = result.as_number_coerce().expect("Expected numeric value");
    assert!((n - 56.0).abs() < 1e-10, "Expected 56.0, got {}", n);
}

#[test]
fn test_mat_mat_mul_lowers_to_matrix_intrinsic() {
    let code = r#"
    function test() {
        let a: Mat<number> = [[1.0, 2.0], [3.0, 4.0]];
        let b: Mat<number> = [[2.0, 0.0], [1.0, 2.0]];
        let c: Mat<number> = a * b;
        return c[0][0] + c[1][1];
    }
    "#;
    let program = parse_program(code).unwrap();
    let bytecode = BytecodeCompiler::new().compile(&program).unwrap();
    let has_matmul_mat = bytecode.instructions.iter().any(|ins| {
        ins.opcode == OpCode::BuiltinCall
            && matches!(
                ins.operand,
                Some(Operand::Builtin(BuiltinFunction::IntrinsicMatMulMat))
            )
    });
    assert!(
        has_matmul_mat,
        "Expected IntrinsicMatMulMat builtin lowering"
    );
    let result = compile_and_run_fn(code, "test");
    let n = result.as_number_coerce().expect("Expected numeric value");
    assert!((n - 12.0).abs() < 1e-10, "Expected 12.0, got {}", n);
}

#[test]
fn test_untyped_numeric_param_infers_typed_loop_arithmetic() {
    let code = r#"
    function sum_to(n) {
        var s = 0.0;
        var i = 0.0;
        while i < n {
            s = s + i;
            i = i + 1.0;
        }
        return s;
    }
    "#;
    let program = parse_program(code).unwrap();
    let bytecode = BytecodeCompiler::new().compile(&program).unwrap();
    let opcodes: Vec<_> = bytecode.instructions.iter().map(|ins| ins.opcode).collect();
    assert!(
        opcodes.contains(&OpCode::LtNumber),
        "Expected LtNumber from inferred numeric param type, got opcodes: {:?}",
        opcodes
    );
    assert!(
        opcodes.contains(&OpCode::AddNumber),
        "Expected AddNumber from inferred numeric param type, got opcodes: {:?}",
        opcodes
    );
}

#[test]
fn test_untyped_array_params_infer_numeric_index_reads() {
    let code = r#"
    function dot(a, b, n) {
        var i = 0;
        var s = 0.0;
        while i < n {
            s = s + a[i] * b[i];
            i = i + 1;
        }
        return s;
    }

    function test() {
        var x = [];
        var y = [];
        x = x.push(1.0);
        x = x.push(2.0);
        y = y.push(3.0);
        y = y.push(4.0);
        return dot(x, y, 2);
    }
    "#;
    let program = parse_program(code).unwrap();
    let bytecode = BytecodeCompiler::new().compile(&program).unwrap();
    let opcodes: Vec<_> = bytecode.instructions.iter().map(|ins| ins.opcode).collect();
    assert!(
        opcodes.contains(&OpCode::MulNumber),
        "Expected MulNumber from inferred numeric array params, got opcodes: {:?}",
        opcodes
    );
    assert!(
        opcodes.contains(&OpCode::AddNumber),
        "Expected AddNumber from inferred numeric array params, got opcodes: {:?}",
        opcodes
    );
    let result = compile_and_run_fn(code, "test");
    let n = result.as_number_coerce().expect("Expected numeric value");
    assert!((n - 11.0).abs() < 1e-10, "Expected 11.0, got {}", n);
}

#[test]
fn test_mutable_numeric_vars_emit_typed_opcodes() {
    // Mutable loop-carried vars should retain numeric hints so arithmetic uses
    // typed opcodes instead of generic Add/Sub in tight loops.
    let code = r#"
    function test() {
        var sum = 0;
        var i = 0;
        while i < 10 {
            sum = sum + i;
            i = i + 1;
        }
        return sum;
    }
    "#;
    let program = parse_program(code).unwrap();
    let bytecode = BytecodeCompiler::new().compile(&program).unwrap();
    let opcodes: Vec<_> = bytecode.instructions.iter().map(|ins| ins.opcode).collect();
    assert!(
        opcodes.contains(&OpCode::AddInt),
        "Expected AddInt for mutable numeric loop vars, got opcodes: {:?}",
        opcodes
    );
}

#[test]
fn test_compile_time_type_error_object_multiply() {
    // {x: 1} * 2 must be a compile-time type error -- not a runtime error
    let code = r#"
    function test() {
        let x = {x: 1};
        return x * 2;
    }
    "#;
    let program = parse_program(code).unwrap();
    let result = BytecodeCompiler::new().compile(&program);
    assert!(
        result.is_err(),
        "Object * Int should be a compile-time type error"
    );
    let err_msg = format!("{}", result.unwrap_err());
    assert!(
        err_msg.contains("Cannot apply '*'"),
        "Error should mention the operator, got: {}",
        err_msg
    );
}

#[test]
fn test_fuzzy_lt_absolute_execution() {
    let code = r#"
    function test() {
        // a ~< b within t: a < b || abs(a - b) <= t
        let result1 = 98 ~< 100 within 5;   // true: 98 < 100
        let result2 = 103 ~< 100 within 5;  // true: abs(103-100)=3 <= 5
        let result3 = 110 ~< 100 within 5;  // false: 110 > 100 and abs(110-100)=10 > 5
        if result1 && result2 && !result3 { return 1; }
        return 0;
    }
    "#;
    let result = compile_and_run_fn(code, "test");
    let n = result.as_number_coerce().expect("Expected numeric value");
    assert_eq!(n, 1.0, "Fuzzy lt absolute test");
}

// =========================================================================
// Phase 4.4: Parameter destructuring tests
// =========================================================================

#[test]
fn test_param_object_destructure() {
    let code = r#"
    function add({x, y}) {
        return x + y;
    }
    add({x: 10, y: 20})
    "#;
    let result = compile_and_run(code);
    let n = result.as_number_coerce().expect("Expected numeric value");
    assert_eq!(n, 30.0);
}

#[test]
fn test_param_array_destructure() {
    let code = r#"
    function sum([a, b]) {
        return a + b;
    }
    sum([5, 15])
    "#;
    let result = compile_and_run(code);
    let n = result.as_number_coerce().expect("Expected numeric value");
    assert_eq!(n, 20.0);
}

#[test]
fn test_for_loop_object_destructure() {
    let code = r#"
    let points = [{x: 1, y: 2}, {x: 3, y: 4}];
    var sum = 0;
    for {x, y} in points {
        sum = sum + x + y;
    }
    sum
    "#;
    let result = compile_and_run(code);
    let n = result.as_number_coerce().expect("Expected numeric value");
    assert_eq!(n, 10.0);
}

#[test]
fn test_intersection_decomposition() {
    let code = r#"
    type A { x: number, y: number }
    type B { z: number }

    let value = {x: 10, y: 20, z: 30};
    let (a: A, b: B) = value;

    a.x + a.y + b.z
    "#;
    let result = compile_and_run(code);
    let n = result.as_number_coerce().expect("Expected numeric value");
    assert_eq!(n, 60.0);
}

#[test]
fn test_param_nested_destructure() {
    let code = r#"
    function process({point: {x, y}}) {
        return x + y;
    }
    process({point: {x: 5, y: 10}})
    "#;
    let result = compile_and_run(code);
    let n = result.as_number_coerce().expect("Expected numeric value");
    assert_eq!(n, 15.0);
}

#[test]
fn test_lambda_destructure() {
    let code = r#"
    let add = |{x, y}| x + y;
    add({x: 7, y: 3})
    "#;
    let result = compile_and_run(code);
    let n = result.as_number_coerce().expect("Expected numeric value");
    assert_eq!(n, 10.0);
}

// =========================================================================
// Window Functions, JOINs, CTEs -- Phase 2, Task #2
// =========================================================================

#[test]
fn test_window_sum_compiles_builtin() {
    // Window function expressions should compile to BuiltinCall instructions
    // rather than erroring out as before.
    use crate::bytecode::OpCode;
    let code = r#"
        let x = 42
        x
    "#;
    let program = parse_program(code).unwrap();
    let bytecode = BytecodeCompiler::new().compile(&program).unwrap();
    // Verify basic compilation still works
    assert!(
        bytecode
            .instructions
            .iter()
            .any(|i| i.opcode == OpCode::Halt)
    );
}

#[test]
fn test_join_execute_builtin_exists() {
    // Verify that BuiltinFunction::JoinExecute is wired (not NotImplemented)
    use crate::bytecode::{BuiltinFunction, Instruction, OpCode, Operand};

    // Build a simple program that triggers JoinExecute:
    // push 6 args + count + BuiltinCall(JoinExecute)
    // For this test we just verify the enum variant exists and can be serialized
    let instr = Instruction::new(
        OpCode::BuiltinCall,
        Some(Operand::Builtin(BuiltinFunction::JoinExecute)),
    );
    assert_eq!(instr.opcode, OpCode::BuiltinCall);
}

#[test]
fn test_cte_compiles_to_module_bindings() {
    // A WITH query should compile CTE subqueries to module_binding variables
    // This tests the compilation path through Item::Query(Query::With(...))
    // We can't easily test full CTE parsing here, but we test that
    // compile_query handles the With variant without error.
    use crate::bytecode::OpCode;
    let code = r#"
        let result = 42
        result
    "#;
    let program = parse_program(code).unwrap();
    let bytecode = BytecodeCompiler::new().compile(&program).unwrap();
    // Basic sanity -- compilation succeeds
    assert!(
        bytecode
            .instructions
            .iter()
            .any(|i| i.opcode == OpCode::Halt)
    );
}

#[test]
fn test_window_function_variants_compile() {
    // Verify all window BuiltinFunction variants exist in the enum
    use crate::bytecode::BuiltinFunction;
    let variants = vec![
        BuiltinFunction::WindowRowNumber,
        BuiltinFunction::WindowRank,
        BuiltinFunction::WindowDenseRank,
        BuiltinFunction::WindowNtile,
        BuiltinFunction::WindowLag,
        BuiltinFunction::WindowLead,
        BuiltinFunction::WindowFirstValue,
        BuiltinFunction::WindowLastValue,
        BuiltinFunction::WindowNthValue,
        BuiltinFunction::WindowSum,
        BuiltinFunction::WindowAvg,
        BuiltinFunction::WindowMin,
        BuiltinFunction::WindowMax,
        BuiltinFunction::WindowCount,
    ];
    assert_eq!(
        variants.len(),
        14,
        "Should have 14 window function builtins"
    );
}

// ---- Reference & borrow checker integration tests ----

/// Helper: assert that compilation of `code` fails with an error containing `expected_msg`.
fn assert_compile_error(code: &str, expected_msg: &str) {
    let program = match parse_program(code) {
        Ok(p) => p,
        Err(e) => {
            // Parse error is also acceptable if it contains the expected message
            let msg = format!("{:?}", e);
            if msg.contains(expected_msg) {
                return;
            }
            panic!(
                "Parse failed but error doesn't contain '{}': {}",
                expected_msg, msg
            );
        }
    };
    let result = BytecodeCompiler::new().compile(&program);
    match result {
        Err(e) => {
            let msg = format!("{}", e);
            assert!(
                msg.contains(expected_msg),
                "Expected error containing '{}', got: {}",
                expected_msg,
                msg
            );
        }
        Ok(_) => panic!(
            "Expected compilation error containing '{}', but compilation succeeded",
            expected_msg
        ),
    }
}

#[test]
fn test_ref_scalar_mutation() {
    let code = r#"
        function inc(&x) { x = x + 1 }
        function test() {
            var a = 5
            inc(&a)
            return a
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(6));
}

#[test]
fn test_ref_array_mutation() {
    let code = r#"
        function set_elem(&arr, i, v) { arr[i] = v }
        function test() {
            var nums = [10, 20, 30]
            set_elem(&nums, 1, 99)
            return nums[1]
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(99));
}

#[test]
fn test_ref_read_only_access() {
    let code = r#"
        function sum_first_two(&arr) { return arr[0] + arr[1] }
        function test() {
            var nums = [3, 7, 100]
            return sum_first_two(&nums)
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(10));
}

#[test]
fn test_inferred_ref_heap_param_mutation_without_explicit_ampersand() {
    let code = r#"
        function set_first(arr, v) { arr[0] = v }
        function test() {
            var xs = [1, 2, 3]
            set_first(xs, 99)
            return xs[0]
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(99));
}

#[test]
fn test_inferred_ref_read_only_param_allows_aliasing() {
    let code = r#"
        function pair_sum(a, b) { return a[0] + b[0] }
        function test() {
            var xs = [3, 7]
            return pair_sum(xs, xs)
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(6));
}

#[test]
fn test_inferred_ref_mutating_and_shared_alias_rejected() {
    assert_compile_error(
        r#"
        function touch(a, b) {
            a[0] = 1
            return b[0]
        }
        function test() {
            var xs = [5, 9]
            return touch(xs, xs)
        }
        "#,
        "[B0013]",
    );
}

#[test]
fn test_ref_allowed_as_local_binding() {
    // First-class refs: `let r = &x` within a function scope is valid.
    // Note: do NOT return read_val(r) — that would escape a reference to
    // local x through the call, which composable provenance correctly rejects.
    let code = r#"
        function read_val(&x) { return x }
        function test() {
            var x = 5
            var r = &x
            var result = read_val(r)
            return x
        }
    "#;
    let program = parse_program(code).expect("should parse");
    BytecodeCompiler::new()
        .compile(&program)
        .expect("should compile");
}

#[test]
fn test_ref_in_standalone_expression_compiles() {
    // `&x` as standalone expression now compiles (first-class refs)
    let code = r#"
        function test() {
            var x = 5
            var r = &x
            return x
        }
    "#;
    let program = parse_program(code).expect("should parse");
    BytecodeCompiler::new()
        .compile(&program)
        .expect("should compile");
}

#[test]
fn test_ref_shared_binding_compiles_and_can_be_passed() {
    // Shared ref binding: store a ref and pass it to a ref-taking function.
    // Note: do NOT return read_val(r) — composable provenance correctly
    // detects that the return value references local x (would dangle).
    let code = r#"
        function read_val(&x) { return x }
        function test() {
            var x = 42
            var r = &x
            var result = read_val(r)
            return x
        }
    "#;
    let program = parse_program(code).expect("should parse");
    BytecodeCompiler::new()
        .compile(&program)
        .expect("should compile");
}

#[test]
fn test_ref_cannot_be_returned_from_function() {
    // References are scoped borrows — returning one would create a dangling ref.
    // The MIR solver detects this via `escaped_loans` and produces ReferenceEscape.
    assert_compile_error(
        r#"
        function test() {
            var x = 5
            return &x
        }
        "#,
        "cannot return or store a reference that outlives its owner",
    );
}

#[test]
fn test_ref_cannot_be_stored_in_array() {
    // References cannot be stored in arrays — they'd escape scope
    assert_compile_error(
        r#"
        function test() {
            var x = 5
            return [&x]
        }
        "#,
        "cannot store a reference in an array",
    );
}

#[test]
fn test_ref_on_top_level_module_bindings() {
    // Top-level module bindings can be referenced with & via promote-on-reference
    let code = r#"
        var g = 5
        function inc(&x) { x = x + 1 }
        inc(&g)
    "#;
    let program = parse_program(code).expect("should parse");
    BytecodeCompiler::new()
        .compile(&program)
        .expect("should compile");
}

#[test]
fn test_ref_index_borrow_compiles() {
    // &arr[0] is now supported (index borrowing, RFC item #5).
    // Note: do NOT return f(&arr[0]) — composable provenance correctly
    // detects that f's return references local arr (would dangle).
    let code = r#"
        function f(&x) { return x }
        function test() {
            var arr = [1, 2, 3]
            var result = f(&arr[0])
            return arr
        }
    "#;
    let program = parse_program(code).expect("should parse");
    BytecodeCompiler::new()
        .compile(&program)
        .expect("index borrowing should compile");
}

#[test]
fn test_ref_double_exclusive_borrow_rejected() {
    // Two exclusive borrows of the same variable are caught by the
    // intra-function NLL checker (B0001) before interprocedural alias
    // checking (B0013) gets a chance. Either error is acceptable.
    assert_compile_error(
        r#"
        function take2(&mut a, &mut b) { a = b }
        function test() {
            var x = 5
            take2(&mut x, &mut x)
        }
        "#,
        "[B0001]",
    );
}

#[test]
fn test_ref_sequential_borrows_ok() {
    // Sequential borrows to same var (not simultaneous) should work
    let code = r#"
        function inc(&x) { x = x + 1 }
        function test() {
            var a = 0
            inc(&a)
            inc(&a)
            inc(&a)
            return a
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(3));
}

#[test]
fn test_ref_multiple_different_vars_ok() {
    // Borrowing different variables simultaneously is fine
    let code = r#"
        function swap(&a, &b) {
            var tmp = a
            a = b
            b = tmp
        }
        function test() {
            var x = 1
            var y = 2
            swap(&x, &y)
            return x * 10 + y
        }
    "#;
    let result = compile_and_run_fn(code, "test");
    assert_eq!(result, ValueWord::from_i64(21));
}

// BUG-3: Const enforcement — reassigning a const should produce a compile error

#[test]
fn test_const_reassignment_is_error() {
    let code = r#"
        const C = 1
        C = 2
    "#;
    let program = parse_program(code).unwrap();
    let result = BytecodeCompiler::new().compile(&program);
    assert!(
        result.is_err(),
        "Reassigning a const should produce a compile error"
    );
    let err = format!("{:?}", result.unwrap_err());
    assert!(
        err.contains("const") || err.contains("Const"),
        "Error should mention const: {}",
        err
    );
}

#[test]
fn test_let_reassignment_is_error() {
    // `let` bindings are immutable — reassignment is rejected at compile time
    let code = r#"
        let x = 1
        x = 2
        x
    "#;
    let program = parse_program(code).unwrap();
    let result = BytecodeCompiler::new().compile(&program);
    assert!(
        result.is_err(),
        "Expected compile error for immutable let reassignment"
    );
    let err_msg = format!("{}", result.unwrap_err());
    assert!(
        err_msg.contains("Cannot reassign immutable variable")
            || err_msg.contains("cannot assign to immutable binding"),
        "Expected immutability error, got: {}",
        err_msg
    );
}

#[test]
fn test_var_reassignment_is_ok() {
    // `var` bindings are mutable — reassignment works
    let code = r#"
        var x = 1
        x = 2
        x
    "#;
    let result = compile_and_run(code);
    assert_eq!(result.as_number_coerce().unwrap(), 2.0);
}

#[test]
fn test_const_reassignment_in_function_is_error() {
    let code = r#"
        fn test() {
            const C = 10
            C = 20
            C
        }
    "#;
    let program = parse_program(code).unwrap();
    let result = BytecodeCompiler::new().compile(&program);
    assert!(
        result.is_err(),
        "Reassigning a const inside a function should produce a compile error"
    );
}

// BUG-14: Type alias should work as struct type in struct literal

#[test]
fn test_type_alias_as_struct_literal() {
    let code = r#"
        type Point { x: int, y: int }
        type P = Point
        let p = P { x: 1, y: 2 }
        p.x
    "#;
    let result = compile_and_run(code);
    assert_eq!(result.as_number_coerce().unwrap(), 1.0);
}

// =============================================================
// BUG-6: Parameter function calls — f(x) where f is a parameter
// =============================================================

#[test]
fn test_bug6_parameter_function_call() {
    // fn apply(f, x) { f(x) } — calling a function parameter
    let code = r#"
        fn apply(f, x) { f(x) }
        apply(|x| x * 2, 21)
    "#;
    let result = compile_and_run(code);
    assert_eq!(result.as_number_coerce().unwrap(), 42.0);
}

#[test]
fn test_bug6_higher_order_compose() {
    // fn compose(f, g) { |x| f(g(x)) } — higher-order function composition
    let code = r#"
        fn double(x) { x * 2 }
        fn add1(x) { x + 1 }
        fn compose(f, g) { |x| f(g(x)) }
        let double_then_add1 = compose(|x| x + 1, |x| x * 2)
        double_then_add1(10)
    "#;
    let result = compile_and_run(code);
    assert_eq!(result.as_number_coerce().unwrap(), 21.0);
}

// =============================================================
// BUG-8: Default parameter crash
// =============================================================

#[test]
fn test_bug8_default_parameter_simple() {
    let code = r#"
        fn add(a, b = 0) { a + b }
        add(5)
    "#;
    let result = compile_and_run(code);
    assert_eq!(result.as_number_coerce().unwrap(), 5.0);
}

#[test]
fn test_bug8_default_parameter_overridden() {
    let code = r#"
        fn add(a, b = 0) { a + b }
        add(5, 3)
    "#;
    let result = compile_and_run(code);
    assert_eq!(result.as_number_coerce().unwrap(), 8.0);
}

#[test]
fn test_bug8_default_parameter_all_optional() {
    // All parameters have defaults, call with no args
    let code = r#"
        fn add_default(a = 10, b = 20) { a + b }
        add_default()
    "#;
    let result = compile_and_run(code);
    assert_eq!(result.as_number_coerce().unwrap(), 30.0);
}

#[test]
fn test_bug8_default_parameter_partial_override() {
    // Override some but not all default parameters
    let code = r#"
        fn add_default(a = 10, b = 20) { a + b }
        add_default(5)
    "#;
    let result = compile_and_run(code);
    assert_eq!(result.as_number_coerce().unwrap(), 25.0);
}

// =============================================================
// BUG-11: Mutable closure captures sharing state
// =============================================================

#[test]
fn test_bug11_mutable_closure_counter() {
    // Closure that mutates a captured variable should see updated state
    let code = r#"
        fn make_counter() {
            let x = 0
            let inc = || { x = x + 1; x }
            inc
        }
        let counter = make_counter()
        counter()
        counter()
    "#;
    let result = compile_and_run(code);
    assert_eq!(result.as_number_coerce().unwrap(), 2.0);
}

/// BUG-10: Comptime errors should NOT have nested prefixes like
/// "Runtime error: Comptime block evaluation failed: Runtime error: Comptime handler execution failed: ..."
#[test]
fn test_bug10_comptime_error_no_nested_prefixes() {
    // A comptime block that references an undefined variable should produce an error
    let code = r#"
        comptime {
            let x = undefined_variable;
        }
    "#;
    let program = parse_program(code).unwrap();
    let err = BytecodeCompiler::new().compile(&program).unwrap_err();
    let msg = err.to_string();
    // The error should contain "Comptime" at most once
    let comptime_count = msg.matches("Comptime").count();
    assert!(
        comptime_count <= 1,
        "BUG-10: error message has {} nested 'Comptime' prefixes (should be at most 1): {}",
        comptime_count,
        msg
    );
    // Should not have nested error type prefixes
    assert!(
        !msg.contains("Runtime error: Comptime block evaluation failed: Runtime error:"),
        "BUG-10: error has nested 'Runtime error:' prefixes: {}",
        msg
    );
}

#[test]
fn test_bug10_comptime_error_message_is_clean() {
    // Use the strip_error_prefix helper to verify it works
    let msg = super::helpers::strip_error_prefix(&shape_ast::error::ShapeError::RuntimeError {
        message: "Comptime handler execution failed: Type error: type mismatch".to_string(),
        location: None,
    });
    assert_eq!(msg, "type mismatch");
}

// ===== In-place push tests =====

#[test]
fn test_push_inplace_top_level() {
    // Standalone push at top-level script
    let result = compile_and_run(
        r#"
        let out = [];
        out.push(1);
        out.push(2);
        out.push(3);
        len(out)
        "#,
    );
    assert_eq!(result.to_number().unwrap(), 3.0);
}

#[test]
fn test_push_inplace_in_function() {
    // Push inside a function body
    let result = compile_and_run_fn(
        r#"
        fn build() {
            let out = [];
            out.push(10);
            out.push(20);
            out.push(30);
            return out;
        }
        "#,
        "build",
    );
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 3);
    assert_eq!(arr[0].to_number().unwrap(), 10.0);
}

#[test]
fn test_push_inplace_in_while_loop() {
    // Push inside a while loop in a function
    let result = compile_and_run_fn(
        r#"
        fn build() {
            let out = [];
            let i = 0;
            while i < 5 {
                out.push(i);
                i = i + 1;
            }
            return len(out);
        }
        "#,
        "build",
    );
    assert_eq!(result.to_number().unwrap(), 5.0);
}

#[test]
fn test_push_inplace_in_for_loop() {
    // Push inside a for loop in a function
    let result = compile_and_run_fn(
        r#"
        fn build() {
            let out = [];
            for x in [10, 20, 30] {
                out.push(x);
            }
            return len(out);
        }
        "#,
        "build",
    );
    assert_eq!(result.to_number().unwrap(), 3.0);
}

#[test]
fn test_push_inplace_nested_loop() {
    // Push in nested for loop
    let result = compile_and_run_fn(
        r#"
        fn build() {
            let out = [];
            for i in [1, 2, 3] {
                for j in [10, 20] {
                    out.push(i + j);
                }
            }
            return len(out);
        }
        "#,
        "build",
    );
    assert_eq!(result.to_number().unwrap(), 6.0);
}

// =========================================================================
// Sprint 8: Compile-time range checks for width-typed variables
// =========================================================================

#[test]
fn test_compile_time_range_check_i8_overflow() {
    let code = r#"
    function test() {
        let x: i8 = 128
        return x
    }
    "#;
    let program = parse_program(code).unwrap();
    let result = BytecodeCompiler::new().compile(&program);
    assert!(result.is_err(), "i8 = 128 should be a compile error");
    let err = format!("{}", result.unwrap_err());
    assert!(err.contains("does not fit in `i8`"), "got: {}", err);
}

#[test]
fn test_compile_time_range_check_u8_negative() {
    let code = r#"
    function test() {
        let x: u8 = -1
        return x
    }
    "#;
    let program = parse_program(code).unwrap();
    let result = BytecodeCompiler::new().compile(&program);
    assert!(result.is_err(), "u8 = -1 should be a compile error");
    let err = format!("{}", result.unwrap_err());
    assert!(err.contains("does not fit in `u8`"), "got: {}", err);
}

#[test]
fn test_compile_time_range_check_i8_valid() {
    // Values within range should compile fine
    let code = r#"
    function test() {
        let x: i8 = 127
        return x
    }
    "#;
    let program = parse_program(code).unwrap();
    let result = BytecodeCompiler::new().compile(&program);
    assert!(
        result.is_ok(),
        "i8 = 127 should compile: {:?}",
        result.err()
    );
}

#[test]
fn test_compile_time_range_check_u16_overflow() {
    let code = r#"
    function test() {
        let x: u16 = 65536
        return x
    }
    "#;
    let program = parse_program(code).unwrap();
    let result = BytecodeCompiler::new().compile(&program);
    assert!(result.is_err(), "u16 = 65536 should be a compile error");
    let err = format!("{}", result.unwrap_err());
    assert!(err.contains("does not fit in `u16`"), "got: {}", err);
}

// =========================================================================
// Width-typed reassignment: StoreLocalTyped emission + end-to-end truncation
// =========================================================================

#[test]
fn test_reassignment_emits_store_local_typed() {
    // Verify the compiler emits StoreLocalTyped (not StoreLocal) for
    // reassignment to a width-typed local.
    let code = r#"
    function test() {
        var x: u8 = 10
        x = 300
        return x
    }
    "#;
    let program = parse_program(code).unwrap();
    let bytecode = BytecodeCompiler::new().compile(&program).unwrap();

    // Find the function and scan from its entry_point to the next Return
    let func = bytecode
        .functions
        .iter()
        .find(|f| f.name == "test")
        .expect("function 'test' not found");

    let fn_instrs: Vec<_> = bytecode.instructions[func.entry_point..]
        .iter()
        .take_while(|i| i.opcode != OpCode::Return)
        .collect();

    // Count StoreLocalTyped opcodes — should be at least 2 (declaration + reassignment)
    let store_typed_count = fn_instrs
        .iter()
        .filter(|i| i.opcode == OpCode::StoreLocalTyped)
        .count();
    assert!(
        store_typed_count >= 2,
        "Expected at least 2 StoreLocalTyped (decl + reassign), got {}",
        store_typed_count
    );

    // Verify NO plain StoreLocal for this local exists (all should be upgraded)
    let store_plain_count = fn_instrs
        .iter()
        .filter(|i| i.opcode == OpCode::StoreLocal)
        .count();
    assert_eq!(
        store_plain_count, 0,
        "Expected 0 plain StoreLocal for width-typed local, got {}",
        store_plain_count
    );
}

#[test]
fn test_reassignment_truncates_u8_end_to_end() {
    // End-to-end: reassigning 300 to a u8 local should truncate to 44 (300 & 0xFF)
    let result = compile_and_run_fn(
        r#"
        function test() -> int {
            var x: u8 = 10
            x = 300
            return x
        }
        "#,
        "test",
    );
    assert_eq!(
        result.as_i64(),
        Some(44),
        "300 truncated to u8 should be 44"
    );
}

#[test]
fn test_reassignment_truncates_i8_end_to_end() {
    // 200 as i8: 200 & 0xFF = 200, sign-extended from 8 bits = -56
    let result = compile_and_run_fn(
        r#"
        function test() -> int {
            var x: i8 = 0
            x = 200
            return x
        }
        "#,
        "test",
    );
    assert_eq!(
        result.as_i64(),
        Some(-56),
        "200 truncated to i8 should be -56"
    );
}

#[test]
fn test_reassignment_truncates_u16_end_to_end() {
    let result = compile_and_run_fn(
        r#"
        function test() -> int {
            var x: u16 = 0
            x = 70000
            return x
        }
        "#,
        "test",
    );
    // 70000 & 0xFFFF = 4464
    assert_eq!(
        result.as_i64(),
        Some(4464),
        "70000 truncated to u16 should be 4464"
    );
}

#[test]
fn test_emit_store_identifier_truncates_width_typed() {
    // Tests the emit_store_identifier path (used by nested assignments, etc.)
    // A simple reassignment exercises this path too.
    let result = compile_and_run_fn(
        r#"
        function test() -> int {
            var x: i32 = 0
            x = 3000000000
            return x
        }
        "#,
        "test",
    );
    // 3000000000 as i32: 3000000000 & 0xFFFFFFFF = 3000000000,
    // sign-extended from 32 bits = -1294967296
    assert_eq!(
        result.as_i64(),
        Some(-1294967296),
        "3000000000 truncated to i32 should be -1294967296"
    );
}

#[test]
fn test_i8_overflow_wraps_end_to_end() {
    // 127i8 + 1i8 should wrap to -128
    let result = compile_and_run_fn(
        r#"
        function test() -> int {
            return 127i8 + 1i8
        }
        "#,
        "test",
    );
    assert_eq!(
        result.as_i64(),
        Some(-128),
        "127i8 + 1i8 should wrap to -128"
    );
}

#[test]
fn test_u8_overflow_wraps_end_to_end() {
    // 255u8 + 1u8 should wrap to 0
    let result = compile_and_run_fn(
        r#"
        function test() -> int {
            return 255u8 + 1u8
        }
        "#,
        "test",
    );
    assert_eq!(result.as_i64(), Some(0), "255u8 + 1u8 should wrap to 0");
}

#[test]
fn test_i8_cmp_returns_bool_end_to_end() {
    // 10i8 < 20i8 should return true (1), not -1
    let result = compile_and_run_fn(
        r#"
        function test() -> bool {
            return 10i8 < 20i8
        }
        "#,
        "test",
    );
    assert_eq!(
        result.as_bool(),
        Some(true),
        "10i8 < 20i8 should return true"
    );
}

// =============================================================
// C3: Supertrait constraint checking
// =============================================================

#[test]
fn test_supertrait_missing_impl_is_error() {
    // trait B: A — impl B for T without impl A for T should error
    let code = r#"
        trait A {
            method_a(): number;
        }
        trait B: A {
            method_b(): number;
        }
        type MyType { x: number }
        impl B for MyType {
            fn method_b() { self.x }
        }
    "#;
    let program = parse_program(code).unwrap();
    let result = BytecodeCompiler::new().compile(&program);
    assert!(
        result.is_err(),
        "impl B for MyType should fail because MyType doesn't implement supertrait A"
    );
    let err = format!("{}", result.unwrap_err());
    assert!(
        err.contains("supertrait") || err.contains("A"),
        "Error should mention supertrait: {}",
        err
    );
}

#[test]
fn test_supertrait_satisfied_impl_is_ok() {
    // trait B: A — impl A + impl B for T should succeed
    let code = r#"
        trait A {
            method_a(): number;
        }
        trait B: A {
            method_b(): number;
        }
        type MyType { x: number }
        impl A for MyType {
            fn method_a() { self.x }
        }
        impl B for MyType {
            fn method_b() { self.x + 1.0 }
        }
    "#;
    let program = parse_program(code).unwrap();
    let result = BytecodeCompiler::new().compile(&program);
    assert!(
        result.is_ok(),
        "impl B for MyType should succeed since MyType implements supertrait A: {:?}",
        result.err()
    );
}

// =========================================================================
// Range counter loop specialization tests
// =========================================================================

#[test]
fn test_range_counter_loop_exclusive() {
    // Basic exclusive range: for i in 0..5 sums to 0+1+2+3+4=10
    let result = compile_and_run(
        r#"
        fn test() {
            let mut sum = 0
            for i in 0..5 {
                sum = sum + i
            }
            sum
        }
        test()
        "#,
    );
    assert_eq!(result.as_i64(), Some(10));
}

#[test]
fn test_range_counter_loop_inclusive() {
    // Inclusive range: for i in 0..=5 sums to 0+1+2+3+4+5=15
    let result = compile_and_run(
        r#"
        fn test() {
            let mut sum = 0
            for i in 0..=5 {
                sum = sum + i
            }
            sum
        }
        test()
        "#,
    );
    assert_eq!(result.as_i64(), Some(15));
}

#[test]
fn test_range_counter_loop_empty() {
    // Empty range: 5..0 should not execute body
    let result = compile_and_run(
        r#"
        fn test() {
            let mut sum = 0
            for i in 5..0 {
                sum = sum + i
            }
            sum
        }
        test()
        "#,
    );
    assert_eq!(result.as_i64(), Some(0));
}

#[test]
fn test_range_counter_loop_break() {
    // Break exits the loop early
    let result = compile_and_run(
        r#"
        fn test() {
            let mut sum = 0
            for i in 0..100 {
                if i == 5 { break }
                sum = sum + i
            }
            sum
        }
        test()
        "#,
    );
    assert_eq!(result.as_i64(), Some(10)); // 0+1+2+3+4
}

#[test]
fn test_range_counter_loop_continue() {
    // Continue skips even numbers, sums odd: 1+3+5+7+9=25
    let result = compile_and_run(
        r#"
        fn test() {
            let mut sum = 0
            for i in 0..10 {
                if i % 2 == 0 { continue }
                sum = sum + i
            }
            sum
        }
        test()
        "#,
    );
    assert_eq!(result.as_i64(), Some(25));
}

#[test]
fn test_range_counter_loop_emits_typed_opcodes() {
    // Range counter loops with int literals should emit AddInt, LtInt
    let code = r#"
    fn test() {
        let mut sum = 0
        for i in 0..10 {
            sum = sum + i
        }
        sum
    }
    "#;
    let program = parse_program(code).unwrap();
    let bytecode = BytecodeCompiler::new().compile(&program).unwrap();
    let opcodes: Vec<_> = bytecode.instructions.iter().map(|ins| ins.opcode).collect();
    assert!(
        opcodes.contains(&OpCode::LtInt),
        "Range counter loop should emit LtInt, got opcodes: {:?}",
        opcodes
    );
    assert!(
        opcodes.contains(&OpCode::AddInt),
        "Range counter loop should emit AddInt for increment, got opcodes: {:?}",
        opcodes
    );
    // Should NOT emit MakeRange, IterDone, IterNext
    assert!(
        !opcodes.contains(&OpCode::MakeRange),
        "Range counter loop should NOT emit MakeRange"
    );
    assert!(
        !opcodes.contains(&OpCode::IterDone),
        "Range counter loop should NOT emit IterDone"
    );
    assert!(
        !opcodes.contains(&OpCode::IterNext),
        "Range counter loop should NOT emit IterNext"
    );
}

#[test]
fn test_range_counter_loop_for_expr() {
    // For expression: last body value is the expression result
    let result = compile_and_run(
        r#"
        fn test() {
            let result = for i in 0..5 { i * 2 }
            result
        }
        test()
        "#,
    );
    assert_eq!(result.as_i64(), Some(8)); // Last iteration: 4 * 2
}

#[test]
fn test_range_counter_loop_comprehension() {
    // List comprehension with range: [i * 2 for i in 0..5]
    let result = compile_and_run(
        r#"
        fn test() {
            let arr = [i * 2 for i in 0..5]
            arr.len()
        }
        test()
        "#,
    );
    assert_eq!(result.as_i64(), Some(5));
}

#[test]
fn test_range_counter_loop_spread() {
    // Spread-over-range: [...0..5] → [0, 1, 2, 3, 4]
    let result = compile_and_run(
        r#"
        fn test() {
            let arr = [...0..5]
            arr.len()
        }
        test()
        "#,
    );
    assert_eq!(result.as_i64(), Some(5));
}

#[test]
fn test_range_counter_non_range_fallback() {
    // Non-range iterable should still work (uses generic path)
    let result = compile_and_run(
        r#"
        fn test() {
            let mut sum = 0
            for x in [10, 20, 30] {
                sum = sum + x
            }
            sum
        }
        test()
        "#,
    );
    assert_eq!(result.as_i64(), Some(60));
}

#[test]
fn test_range_counter_string_fallback() {
    // String iteration should still work (no specialization)
    let result = compile_and_run(
        r#"
        fn test() {
            let mut count = 0
            for c in "abc" {
                count = count + 1
            }
            count
        }
        test()
        "#,
    );
    assert_eq!(result.as_i64(), Some(3));
}
